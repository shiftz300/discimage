//! Flat, portable recording inputs. Optical I/O belongs to `burn`.
use crate::{
    art, audio,
    author::{self, Asset, Images, Manifest},
    burn::{Journal, Stage},
    layout::Plan,
    preview,
    project::{Project, Track},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::Write,
    path::{Component, Path, PathBuf},
};

const FORMAT: &str = "discimage-package-v1";

#[derive(Debug, Serialize, Deserialize)]
pub struct Attachment {
    path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    asset: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Package {
    pub format: String,
    pub project: Project,
    pub inputs: Vec<Asset>,
    #[serde(skip_serializing_if = "Option::is_none")]
    files: Option<Vec<Attachment>>,
    fingerprint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<Manifest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub journal: Option<Journal>,
}

pub fn audio_path(index: usize) -> String {
    format!("{:02}.wav", index + 1)
}

fn flat(path: &Path) -> bool {
    path.components().count() == 1 && matches!(path.components().next(), Some(Component::Normal(_)))
}

impl Package {
    pub fn load(directory: &Path) -> Result<Self> {
        let path = directory.join("manifest.toml");
        ensure!(
            fs::metadata(&path)?.len() <= 16 * 1024 * 1024,
            "package manifest is too large"
        );
        let value: Self = toml::from_str(&fs::read_to_string(path)?)?;
        ensure!(value.format == FORMAT, "unsupported package format");
        ensure!(value.inputs.len() <= 10110, "too many package inputs");
        ensure!(
            value.inputs.iter().all(|a| flat(Path::new(&a.path))),
            "package inputs must be flat files"
        );
        ensure!(
            value.project.files.is_none(),
            "package file tree must use frozen attachments"
        );
        for path in value
            .project
            .tracks
            .iter()
            .map(|t| &t.path)
            .chain(value.project.artwork.iter().map(|a| &a.path))
        {
            ensure!(
                flat(path) && value.inputs.iter().any(|a| Path::new(&a.path) == path),
                "project references an unfrozen input"
            );
        }
        if let Some(files) = &value.files {
            ensure!(files.len() <= 10000, "too many attachments");
            for file in files {
                ensure!(
                    !file.path.as_os_str().is_empty()
                        && file
                            .path
                            .components()
                            .all(|c| matches!(c, Component::Normal(_))),
                    "invalid attachment path"
                );
                ensure!(
                    file.asset.as_ref().is_none_or(|name| flat(Path::new(name))
                        && value.inputs.iter().any(|a| &a.path == name)),
                    "invalid attachment asset"
                );
            }
        }
        ensure!(
            value.fingerprint == value.digest()?,
            "package settings changed; rebuild from the album"
        );
        Ok(value)
    }

    fn digest(&self) -> Result<String> {
        Ok(crate::hex(&Sha256::digest(serde_json::to_vec(&(
            &self.project,
            &self.inputs,
            &self.files,
        ))?)))
    }

    pub(crate) fn refresh(&mut self, directory: &Path) -> Result<()> {
        self.inputs = self
            .inputs
            .iter()
            .map(|a| author::asset(directory, &a.path))
            .collect::<Result<_>>()?;
        self.fingerprint = self.digest()?;
        self.save(directory)
    }

    pub(crate) fn save(&self, directory: &Path) -> Result<()> {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = directory.join(format!(
            ".manifest-{}-{}.tmp",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.write_all(toml::to_string_pretty(self)?.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(path, directory.join("manifest.toml"))?;
        #[cfg(unix)]
        File::open(directory)?.sync_all()?;
        Ok(())
    }

    pub fn project(&self, directory: &Path) -> Project {
        let mut project = self.project.clone();
        for track in &mut project.tracks {
            track.path = directory.join(&track.path);
        }
        if let Some(art) = &mut project.artwork {
            art.path = directory.join(&art.path);
        }
        project
    }

    pub fn verify_inputs(&self, directory: &Path) -> Result<Plan> {
        author::verify_assets(directory, &self.inputs)?;
        Plan::from_project(&self.project(directory), None)
    }

    pub fn verify(&self, directory: &Path) -> Result<()> {
        let plan = self.verify_inputs(directory)?;
        if let Some(image) = &self.image {
            ensure!(
                image.plan.music_sectors == plan.music_sectors
                    && image.plan.gap_sectors == plan.gap_sectors
                    && image.plan.disc.geometry() == plan.disc.geometry(),
                "image does not match the frozen album"
            );
            author::verify_manifest(directory, image, false)?;
        }
        Ok(())
    }

    /// Only data and derived previews are regenerated. Audio and artwork stay put.
    pub(crate) fn images(
        &mut self,
        directory: &Path,
        session_start: Option<u32>,
        end: Option<u32>,
        images: Images,
    ) -> Result<()> {
        self.verify_inputs(directory)?;
        let work = directory.join(".work");
        if work.try_exists()? {
            fs::remove_dir_all(&work)?;
        }
        fs::create_dir(&work)?;
        let mut project = self.project(directory);
        if let Some(files) = &self.files {
            let root = work.join("files");
            fs::create_dir(&root)?;
            for item in files {
                let destination = root.join(&item.path);
                if let Some(asset) = &item.asset {
                    fs::create_dir_all(destination.parent().context("missing attachment parent")?)?;
                    if fs::hard_link(directory.join(asset), &destination).is_err() {
                        fs::copy(directory.join(asset), &destination)?;
                    }
                } else {
                    fs::create_dir_all(destination)?;
                }
            }
            project.files = Some(root);
        }
        if let Some(end) = end {
            project.disc.limit_end(end);
        }
        let output = work.join("image");
        let mut image = author::images(&project, &output, session_start, images)?;
        image.artwork = self.project.artwork.clone();
        for (index, track) in image.plan.tracks.iter_mut().enumerate() {
            track.path = audio_path(index).into();
        }
        author::verify_manifest(&output, &image, false)?;
        let old = self
            .image
            .as_ref()
            .map(|i| i.assets.iter().map(|a| a.path.clone()).collect::<Vec<_>>())
            .unwrap_or_default();
        for asset in &image.assets {
            ensure!(
                flat(Path::new(&asset.path)) && !self.inputs.iter().any(|a| a.path == asset.path),
                "invalid generated image asset"
            );
            fs::rename(output.join(&asset.path), directory.join(&asset.path))?;
        }
        self.image = Some(image);
        self.save(directory)?;
        for path in old {
            if !self
                .image
                .as_ref()
                .unwrap()
                .assets
                .iter()
                .any(|a| a.path == path)
                && directory.join(&path).try_exists()?
            {
                fs::remove_file(directory.join(path))?;
            }
        }
        fs::remove_dir_all(work)?;
        Ok(())
    }
}

pub(crate) fn lock(directory: &Path) -> Result<File> {
    let parent = directory.parent().context("missing package parent")?;
    fs::create_dir_all(parent)?;
    let name = directory
        .file_name()
        .context("missing package name")?
        .to_string_lossy();
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(parent.join(format!(".{name}.lock")))?;
    file.try_lock()
        .context("another process is using this package")?;
    Ok(file)
}

pub fn default_output(project: &Path) -> Result<PathBuf> {
    Ok(project
        .parent()
        .unwrap_or(Path::new("."))
        .join("output")
        .join(project.file_stem().context("project has no name")?))
}

fn files(root: &Path, directory: &Path, list: &mut Vec<(PathBuf, bool)>) -> Result<()> {
    ensure!(
        list.len() <= 10000
            && directory
                .components()
                .count()
                .saturating_sub(root.components().count())
                <= 64,
        "attachment tree is too large"
    );
    let mut entries = fs::read_dir(directory)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let kind = entry.file_type()?;
        ensure!(
            kind.is_file() || kind.is_dir(),
            "attachments cannot contain symlinks or special files"
        );
        list.push((path.strip_prefix(root)?.to_path_buf(), kind.is_dir()));
        if kind.is_dir() {
            files(root, &path, list)?;
        }
    }
    Ok(())
}

fn replace(temporary: &Path, output: &Path) -> Result<()> {
    let previous = output.with_file_name(format!(
        ".{}.previous",
        output
            .file_name()
            .context("missing output name")?
            .to_string_lossy()
    ));
    ensure!(
        !previous.try_exists()?,
        "an interrupted replacement is preserved at {}; recover it before rebuilding",
        previous.display()
    );
    let existed = output.try_exists()?;
    if existed {
        fs::rename(output, &previous)?;
    }
    if let Err(error) = fs::rename(temporary, output) {
        if existed {
            fs::rename(&previous, output)?;
        }
        return Err(error.into());
    }
    if existed {
        fs::remove_dir_all(previous)?;
    }
    Ok(())
}

pub fn build(
    source: &Path,
    output: &Path,
    images: Option<Images>,
    session_start: Option<u32>,
) -> Result<()> {
    let _lock = lock(output)?;
    if output.try_exists()? {
        let old = Package::load(output)
            .context("output is not a managed package; choose another destination")?;
        ensure!(
            old.journal
                .as_ref()
                .is_none_or(|j| matches!(j.stage, Stage::Prepared | Stage::Complete)),
            "unfinished recording is protected; resume it before rebuilding"
        );
        owned(output, &old)?;
    }
    let mut project = Project::load(source)?;
    let plan = Plan::from_project(&project, None)?;
    author::volume(&project, &plan)?;
    let mut attachments = Vec::new();
    if let Some(root) = &project.files {
        files(root, root, &mut attachments)?;
    }
    let temporary = output.with_file_name(format!(
        ".{}.building",
        output
            .file_name()
            .context("missing output name")?
            .to_string_lossy()
    ));
    ensure!(
        !temporary.try_exists()?,
        "unfinished build exists at {}",
        temporary.display()
    );
    fs::create_dir(&temporary)?;
    let result = (|| -> Result<()> {
        let mut inputs = Vec::new();
        project.tracks.clear();
        for (index, track) in plan.tracks.iter().enumerate() {
            let path = audio_path(index);
            audio::export(track, &temporary.join(&path))?;
            inputs.push(author::asset(&temporary, &path)?);
            project.tracks.push(Track { path: path.into() });
        }
        if let Some(artwork) = &mut project.artwork {
            art::load(artwork)?.save(temporary.join("artwork.png"))?;
            artwork.path = "artwork.png".into();
            inputs.push(author::asset(&temporary, "artwork.png")?);
        }
        let frozen = project
            .files
            .as_ref()
            .map(|root| {
                attachments
                    .iter()
                    .enumerate()
                    .map(|(index, (path, directory))| {
                        let asset = if *directory {
                            None
                        } else {
                            let name = format!("file-{:04}.bin", index + 1);
                            fs::copy(root.join(path), temporary.join(&name))?;
                            inputs.push(author::asset(&temporary, &name)?);
                            Some(name)
                        };
                        Ok(Attachment {
                            path: path.clone(),
                            asset,
                        })
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?;
        project.files = None;
        let mut package = Package {
            format: FORMAT.into(),
            project,
            inputs,
            files: frozen,
            fingerprint: String::new(),
            image: None,
            journal: None,
        };
        package.fingerprint = package.digest()?;
        let resolved = package.project(&temporary);
        let plan = Plan::from_project(&resolved, None)?;
        preview::save(
            &plan,
            resolved.artwork.as_ref(),
            &temporary.join("preview.svg"),
            1600,
        )?;
        package.save(&temporary)?;
        if let Some(images) = images {
            package.images(&temporary, session_start, None, images)?;
        }
        package.verify(&temporary)?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&temporary);
        return Err(error);
    }
    replace(&temporary, output)
}

fn owned(directory: &Path, package: &Package) -> Result<()> {
    let mut names = package
        .inputs
        .iter()
        .map(|a| a.path.as_str())
        .collect::<std::collections::HashSet<_>>();
    names.extend(["manifest.toml", "preview.svg"]);
    if let Some(image) = &package.image {
        names.extend(image.assets.iter().map(|a| a.path.as_str()));
    }
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        ensure!(
            entry.file_type()?.is_file()
                && names.contains(
                    entry
                        .file_name()
                        .to_str()
                        .context("invalid package filename")?
                ),
            "unmanaged files in output; choose another destination"
        );
    }
    Ok(())
}

/// Collapse a verified, completed legacy job; active jobs remain untouched.
pub fn migrate(directory: &Path) -> Result<()> {
    if directory.join("manifest.toml").try_exists()? {
        return Package::load(directory)?.verify(directory);
    }
    let _lock = lock(directory)?;
    let legacy = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.join("lock"))?;
    legacy.try_lock().context("legacy recording is in use")?;
    let mut journal = Journal::load(directory)?;
    ensure!(
        journal.stage == Stage::Complete,
        "only completed legacy recordings can be migrated"
    );
    ensure!(
        journal.source_sha256.as_deref()
            == Some(crate::burn::fingerprint(&directory.join("source"))?.as_str()),
        "legacy source snapshot changed"
    );
    let actual = journal.actual(directory);
    ensure!(
        journal.actual_sha256.as_deref()
            == Some(author::digest(&actual.join("manifest.json"))?.as_str()),
        "legacy image manifest changed"
    );
    let original = author::verify(&actual)?;
    ensure!(
        !original.plan.disc.calibration_target && original.plan.disc.phase_target.is_none(),
        "keep calibration evidence in its original layout"
    );
    let temporary = directory.with_file_name(format!(
        ".{}.migration",
        directory
            .file_name()
            .context("missing job name")?
            .to_string_lossy()
    ));
    build(
        &directory.join("source/project.toml"),
        &temporary,
        None,
        None,
    )?;
    let mut package = Package::load(&temporary)?;
    let result = (|| -> Result<()> {
        package.project.disc = original.plan.disc.clone();
        package.project.disc.media =
            journal
                .observed
                .media
                .as_ref()
                .and_then(|m| match m.kind.as_str() {
                    "cd-rw" => Some(crate::project::Medium::Cdrw),
                    "cd-r" | "cd-rom" => Some(crate::project::Medium::Cdr),
                    _ => None,
                });
        let mut image = original;
        image.assets.clear();
        fs::copy(actual.join("session2.iso"), temporary.join("session2.iso"))?;
        image
            .assets
            .push(author::asset(&temporary, "session2.iso")?);
        let project = package.project(&temporary);
        fs::remove_file(temporary.join("preview.svg"))?;
        preview::save(
            &image.plan,
            project.artwork.as_ref(),
            &temporary.join("preview.svg"),
            1600,
        )?;
        image.assets.push(author::asset(&temporary, "preview.svg")?);
        image.artwork = package.project.artwork.clone();
        for (index, track) in image.plan.tracks.iter_mut().enumerate() {
            track.path = audio_path(index).into();
        }
        journal.format = "discimage-burn-v2".into();
        journal.source_sha256 = None;
        journal.planned_sha256 = None;
        journal.actual_sha256 = None;
        package.journal = Some(journal);
        package.image = Some(image);
        package.refresh(&temporary)?;
        package.verify(&temporary)?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&temporary);
        return Err(error);
    }
    drop(legacy);
    replace(&temporary, directory)
}
