//! Two-session recording policy. Native APIs stay behind `Recorder`.
use crate::{
    art, audio,
    author::{self, Manifest},
    console,
    device::{Device, Media, Music, Recorder, Track},
    iso::Volume,
    layout::Plan,
    project::{Project, Track as Input},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Preparing,
    Prepared,
    MusicWriting,
    MusicWritten,
    DataReady,
    DataWriting,
    Complete,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Journal {
    pub format: String,
    pub stage: Stage,
    pub device: String,
    pub speed: u32,
    pub source_sha256: Option<String>,
    pub planned_sha256: Option<String>,
    pub actual_sha256: Option<String>,
    pub attempt: u32,
    pub nwa: Option<u32>,
    pub observed: Device,
    pub error: Option<String>,
}
impl Journal {
    pub fn load(directory: &Path) -> Result<Self> {
        let file = File::open(directory.join("journal.json"))?;
        ensure!(
            file.metadata()?.len() <= 1024 * 1024,
            "recording journal is too large"
        );
        let value: Self = serde_json::from_reader(file)?;
        ensure!(
            value.format == "discimage-burn-v1",
            "unsupported recording journal"
        );
        Ok(value)
    }
    fn save(&self, directory: &Path) -> Result<()> {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let temp = directory.join(format!(
            "journal-{}-{}.tmp",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(&serde_json::to_vec_pretty(self)?)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(temp, directory.join("journal.json"))?;
        #[cfg(unix)]
        File::open(directory)?.sync_all()?;
        Ok(())
    }
    fn advance(&mut self, directory: &Path, stage: Stage) -> Result<()> {
        self.stage = stage;
        self.error = None;
        self.save(directory)
    }
    pub fn actual(&self, directory: &Path) -> PathBuf {
        directory.join(format!("actual-{}", self.attempt))
    }
}

pub fn start(
    backend: &mut dyn Recorder,
    project: &Path,
    directory: &Path,
    speed: u32,
    dry_run: bool,
) -> Result<Journal> {
    ensure!((1..=64).contains(&speed), "speed must be 1–64x");
    let observed = backend.inspect()?;
    let available = blank(&observed, false)?;
    console::phase(
        "prepare",
        format_args!(
            "{} | {} | {available} writable sectors | requested {speed}x",
            observed.name,
            writable_cd(&observed)?.kind,
        ),
    );
    let mut project = Project::load(project)?;
    check_cd_text(backend, &observed, &project)?;
    project
        .disc
        .limit_end(available.saturating_sub(project.disc.end_margin_sectors));
    if project.tracks.len() > 1 {
        let requested = project.disc.intertrack_gap_sectors;
        let gap = backend.music_gap(&observed, requested);
        if gap != requested {
            console::warning(format_args!(
                "Adjusted track gaps from {requested} to {gap} sectors for {}",
                observed.name,
            ));
            project.disc.intertrack_gap_sectors = gap;
        }
    }
    let plan = Plan::from_project(&project, None)?;
    project.disc.capacity_sectors = plan.disc.capacity_sectors;
    let volume = volume(&project, &plan)?;
    let source = project.artwork.as_ref().map(art::load).transpose()?;
    check_cancel(backend)?;
    std::fs::create_dir(directory)
        .with_context(|| format!("cannot create new recording job {}", directory.display()))?;
    let directory = std::fs::canonicalize(directory)?;
    let _lock = job_lock(&directory)?;
    let mut journal = Journal {
        format: "discimage-burn-v1".into(),
        stage: Stage::Preparing,
        device: observed.id.clone(),
        speed,
        source_sha256: None,
        planned_sha256: None,
        actual_sha256: None,
        attempt: 0,
        nwa: None,
        observed,
        error: None,
    };
    journal.save(&directory)?;
    let result = (|| {
        let snapshot = directory.join("source");
        std::fs::create_dir(&snapshot)?;
        std::fs::create_dir(snapshot.join("audio"))?;
        project.tracks.clear();
        for (i, track) in plan.tracks.iter().enumerate() {
            check_cancel(backend)?;
            let path = PathBuf::from(format!("audio/{:02}.wav", i + 1));
            let frozen = snapshot.join(&path);
            audio::export(track, &frozen)?;
            normalize_length(backend, &frozen, track.sectors)?;
            project.tracks.push(Input { path });
        }
        if project.files.is_some() {
            volume.snapshot(&snapshot.join("files"))?;
            project.files = Some("files".into());
        }
        if let Some(image) = source {
            image.save(snapshot.join("artwork.png"))?;
            project.artwork.as_mut().unwrap().path = "artwork.png".into();
        }
        let config = snapshot.join("project.toml");
        std::fs::write(&config, toml::to_string_pretty(&project)?)?;
        journal.source_sha256 = Some(fingerprint(&snapshot)?);
        check_cancel(backend)?;
        console::phase(
            "prepare",
            format_args!("Building the provisional audio and data bundle"),
        );
        let planned = directory.join("planned");
        author::build(&config, &planned, None)?;
        author::verify(&planned)?;
        journal.planned_sha256 = Some(author::digest(&planned.join("manifest.json"))?);
        journal.advance(&directory, Stage::Prepared)?;
        if !dry_run {
            run(backend, &directory, &mut journal)?;
        }
        Ok(())
    })();
    finish(&directory, journal, result)
}

fn normalize_length(backend: &mut dyn Recorder, path: &Path, mut sectors: u32) -> Result<()> {
    // Some decoders round an exact CD frame boundary up by one sector.
    let original = sectors;
    while let Some(reported) = backend.audio_sectors(path)? {
        if reported == sectors {
            break;
        }
        ensure!(
            reported == sectors + 1 && sectors - original < 4,
            "{}: system decoder reports {reported} sectors for a {sectors}-sector WAV",
            path.display()
        );
        audio::extend_normalized(path, reported)?;
        sectors = reported;
    }
    if sectors != original {
        console::warning(format_args!(
            "{}: added {} silent tail sector(s) to match the system decoder",
            path.display(),
            sectors - original
        ));
    }
    Ok(())
}

pub fn resume(backend: &mut dyn Recorder, directory: &Path) -> Result<Journal> {
    let directory = std::fs::canonicalize(directory)?;
    let _lock = job_lock(&directory)?;
    let mut journal = Journal::load(&directory)?;
    ensure!(
        matches!(
            journal.stage,
            Stage::Prepared | Stage::MusicWritten | Stage::DataReady
        ),
        "stage {:?} cannot be resumed automatically; no write attempted",
        journal.stage
    );
    let result = run(backend, &directory, &mut journal);
    finish(&directory, journal, result)
}

fn job_lock(directory: &Path) -> Result<File> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join("lock"))?;
    file.try_lock()
        .context("another process is using this recording job")?;
    Ok(file)
}

fn finish(directory: &Path, mut journal: Journal, result: Result<()>) -> Result<Journal> {
    if let Err(error) = result {
        journal.error = Some(format!("{error:#}"));
        if let Err(save) = journal.save(directory) {
            console::warning(format_args!("Could not update recording journal: {save:#}"));
        }
        return Err(error).with_context(|| {
            format!(
                "recording stopped at {:?}; see {}",
                journal.stage,
                directory.join("journal.json").display()
            )
        });
    }
    Ok(journal)
}

fn run(backend: &mut dyn Recorder, directory: &Path, journal: &mut Journal) -> Result<()> {
    ensure!(
        (1..=64).contains(&journal.speed),
        "invalid saved recording speed"
    );
    console::phase(
        "check",
        format_args!("Checking the saved inputs and recording bundle"),
    );
    let snapshot = directory.join("source");
    check_source(&snapshot, journal)?;
    let planned = directory.join("planned");
    let manifest = checked_bundle(&planned, journal.planned_sha256.as_deref())?;
    let observed = backend.inspect()?;
    check_device(&observed, journal)?;
    check_cancel(backend)?;
    backend.lock()?;
    let observed = backend.inspect()?;
    check_device(&observed, journal)?;
    journal.observed = observed;
    let gap = manifest.plan.disc.intertrack_gap_sectors;
    let tracks = manifest
        .plan
        .tracks
        .iter()
        .enumerate()
        .map(|(i, t)| Music {
            path: planned.join(format!("audio/{:02}.wav", i + 1)),
            sectors: t.sectors,
            gap: if i == 0 { 150 } else { gap },
        })
        .collect::<Vec<_>>();
    if journal.stage == Stage::Prepared {
        let available = blank(&journal.observed, true)?;
        let project = Project::load(&snapshot.join("project.toml"))?;
        check_cd_text(backend, &journal.observed, &project)?;
        ensure!(
            available >= manifest.plan.estimated_art_end_sector,
            "blank CD is smaller than the prepared plan; create a new job"
        );
        ensure!(
            tracks.len() == 1 || backend.music_gap(&journal.observed, gap) == gap,
            "prepared track gaps are incompatible with this drive; create a new job"
        );
        for track in &tracks {
            ensure!(
                backend
                    .audio_sectors(&track.path)?
                    .is_none_or(|reported| reported == track.sectors),
                "{}: system decoder changed the prepared track length; create a new job",
                track.path.display()
            );
        }
        check_cancel(backend)?;
        journal.advance(directory, Stage::MusicWriting)?;
        console::phase(
            "session 1",
            format_args!(
                "Writing {} audio tracks (SAO, disc stays appendable)",
                tracks.len()
            ),
        );
        backend.music(
            &tracks,
            journal.speed,
            project
                .cd_text
                .as_ref()
                .map(|text| (project.title.as_str(), text)),
        )?;
        journal.advance(directory, Stage::MusicWritten)?;
    }
    check_cancel(backend)?;
    let observed = backend.inspect()?;
    check_device(&observed, journal)?;
    let (nwa, free) = next(&observed, &manifest.plan)?;
    journal.observed = observed;
    console::phase(
        "session 1",
        format_args!("Drive reports next writable LBA {nwa} and {free} free sectors"),
    );
    backend
        .verify_music(&tracks)
        .context("music readback failed before data append")?;
    let cached = if journal.stage == Stage::DataReady && journal.nwa == Some(nwa) {
        Some(checked_bundle(
            &journal.actual(directory),
            journal.actual_sha256.as_deref(),
        )?)
    } else {
        None
    };
    let actual = if let Some(actual) = cached.filter(|a| a.data_sectors <= free) {
        actual
    } else {
        let mut project = Project::load(&snapshot.join("project.toml"))?;
        let end = nwa
            .checked_add(free)
            .context("drive reported an invalid writable range")?;
        project.disc.limit_end(end);
        journal.attempt = journal
            .attempt
            .checked_add(1)
            .context("recording attempt counter overflow")?;
        journal.nwa = Some(nwa);
        journal.actual_sha256 = None;
        journal.advance(directory, Stage::MusicWritten)?;
        let path = journal.actual(directory);
        console::phase(
            "session 2",
            format_args!("Rebuilding the data bundle at LBA {nwa}"),
        );
        author::build_project(project, &path, Some(nwa))?;
        let actual = author::verify(&path)?;
        journal.actual_sha256 = Some(author::digest(&path.join("manifest.json"))?);
        journal.advance(directory, Stage::DataReady)?;
        actual
    };
    ensure!(
        actual.session_start == nwa
            && actual.plan.music_sectors == manifest.plan.music_sectors
            && actual.plan.gap_sectors == manifest.plan.gap_sectors,
        "reauthored session does not match the recorded music layout"
    );
    check_source(&snapshot, journal)?;
    check_cancel(backend)?;
    let observed = backend.inspect()?;
    check_device(&observed, journal)?;
    let (current, remaining) = next(&observed, &manifest.plan)?;
    ensure!(
        current == actual.session_start,
        "NWA changed before data write; no data was written"
    );
    ensure!(
        remaining >= actual.data_sectors,
        "remaining CD space is smaller than the reauthored data track"
    );
    journal.observed = observed;
    journal.advance(directory, Stage::DataWriting)?;
    console::phase(
        "session 2",
        format_args!(
            "Writing {} Mode 1 sectors, then verifying and finalizing",
            actual.data_sectors
        ),
    );
    backend.data(
        &journal.actual(directory).join("session2.iso"),
        actual.session_start,
        actual.data_sectors,
        journal.speed,
    )?;
    let observed = backend.inspect()?;
    if observed.media.as_ref().is_some_and(|m| m.kind == "cd-rom") {
        ensure!(
            observed.id == journal.device
                && journal
                    .observed
                    .media
                    .as_ref()
                    .is_some_and(|m| m.kind == "cd-r"),
            "unexpected read-only medium after finalization"
        );
    } else {
        check_device(&observed, journal)?;
    }
    let short = closed(
        &observed,
        &manifest.plan,
        actual.session_start,
        actual.data_sectors,
    )?;
    journal.observed = observed;
    if short {
        backend.verify_data_tail(
            &journal.actual(directory).join("session2.iso"),
            actual.session_start,
            actual.data_sectors,
        )?;
    }
    journal.advance(directory, Stage::Complete)
}

fn volume(project: &Project, plan: &Plan) -> Result<Volume> {
    Volume::new(
        project.files.as_deref(),
        plan.session_start(),
        plan.data_sectors(),
        plan.disc.data_reserve_sectors,
        &plan.title,
    )
}
fn checked_bundle(path: &Path, expected: Option<&str>) -> Result<Manifest> {
    ensure!(
        expected == Some(author::digest(&path.join("manifest.json"))?.as_str()),
        "saved bundle manifest changed"
    );
    author::verify(path)
}
fn check_source(path: &Path, journal: &Journal) -> Result<()> {
    ensure!(
        journal.source_sha256.as_deref() == Some(fingerprint(path)?.as_str()),
        "frozen recording inputs changed"
    );
    Ok(())
}
fn check_cancel(backend: &dyn Recorder) -> Result<()> {
    ensure!(
        !backend.cancelled(),
        "recording cancelled before the next write"
    );
    Ok(())
}
fn check_device(device: &Device, journal: &Journal) -> Result<()> {
    ensure!(device.id == journal.device, "recording drive changed");
    let media = writable_cd(device)?;
    ensure!(
        journal
            .observed
            .media
            .as_ref()
            .is_some_and(|m| m.kind == media.kind),
        "CD media type changed during the recording job"
    );
    Ok(())
}
fn writable_cd(device: &Device) -> Result<&Media> {
    ensure!(!device.id.is_empty(), "drive has no stable identifier");
    let media = device
        .media
        .as_ref()
        .context("insert a CD-R or CD-RW; no ready media was reported")?;
    ensure!(
        matches!(media.kind.as_str(), "cd-r" | "cd-rw"),
        "only CD-R and CD-RW media are supported, found {}",
        media.kind
    );
    ensure!(
        device.capabilities.as_ref().is_some_and(|c| {
            c.sao && c.tao && if media.kind == "cd-rw" { c.cdrw } else { c.cdr }
        }),
        "drive must support writing {}, audio SAO and data TAO",
        media.kind
    );
    Ok(media)
}
fn blank(device: &Device, reserved: bool) -> Result<u32> {
    let media = writable_cd(device)?;
    ensure!(
        media.blank && !device.tracks.iter().any(|t| !t.empty),
        "music recording requires a blank CD-R or CD-RW"
    );
    ensure!(!reserved || media.reserved, "CD reservation was lost");
    media
        .free_sectors
        .filter(|&v| v > 0)
        .context("drive did not report blank media capacity")
}

/// Pick a drive for preparation only. The write path rechecks the saved drive
/// and media after taking the reservation.
pub fn select_blank(drives: &[Device]) -> Result<&Device> {
    let mut candidates = drives.iter().filter(|drive| blank(drive, false).is_ok());
    let selected = candidates.next().context(
        "no drive with a writable blank CD-R or CD-RW; run 'discimage devices' for details",
    )?;
    ensure!(
        candidates.next().is_none(),
        "multiple drives have writable blank discs; use 'discimage devices' and pass --device"
    );
    Ok(selected)
}
fn check_cd_text(backend: &mut dyn Recorder, device: &Device, project: &Project) -> Result<()> {
    if let Some(text) = &project.cd_text {
        ensure!(
            device
                .capabilities
                .as_ref()
                .is_some_and(|caps| caps.cd_text),
            "{} does not report CD-Text writing support",
            device.name
        );
        backend.preflight_cd_text(&project.title, text)?;
    }
    Ok(())
}
fn recorded(device: &Device) -> Vec<&Track> {
    let mut tracks = device
        .tracks
        .iter()
        .filter(|t| !t.empty && t.closed)
        .collect::<Vec<_>>();
    tracks.sort_by_key(|t| t.number);
    tracks
}
fn music_layout(tracks: &[&Track], plan: &Plan) -> Result<()> {
    ensure!(
        tracks.len() >= plan.tracks.len(),
        "recorded music track count is incomplete"
    );
    let mut start = 0u32;
    for (i, (recorded, expected)) in tracks.iter().zip(&plan.tracks).enumerate() {
        // The SuperDrive's MMC track size excludes two readable trailing sectors.
        ensure!(
            recorded.number == i as u32 + 1
                && recorded.session == 1
                && recorded.audio
                && recorded.start == Some(i64::from(start))
                && recorded
                    .sectors
                    .is_none_or(|s| s.saturating_add(2) >= expected.sectors),
            "recorded music layout does not match track {}",
            i + 1
        );
        start += expected.sectors;
        if i + 1 < plan.tracks.len() {
            start += plan.disc.intertrack_gap_sectors;
        }
    }
    Ok(())
}
fn next(device: &Device, plan: &Plan) -> Result<(u32, u32)> {
    let media = writable_cd(device)?;
    ensure!(
        !media.blank && media.appendable && media.reserved,
        "music must be on a reserved, appendable CD"
    );
    ensure!(
        (1..=2).contains(&media.sessions),
        "unexpected session count before data append"
    );
    let tracks = recorded(device);
    ensure!(
        tracks.len() == plan.tracks.len(),
        "expected only the original closed music tracks before append"
    );
    music_layout(&tracks, plan)?;
    ensure!(
        !device.tracks.iter().any(|t| !t.closed && !t.empty),
        "partially written track present; will not append"
    );
    let nwa = media
        .nwa
        .context("drive did not expose an unambiguous next writable LBA")?;
    ensure!(
        nwa >= plan.music_sectors + plan.gap_sectors + 11_400 && nwa < plan.disc.capacity_sectors,
        "reported NWA is outside the valid second-session range: {nwa}"
    );
    let free = media
        .free_sectors
        .filter(|&v| v > 0)
        .context("drive did not report remaining writable sectors")?;
    Ok((nwa, free))
}
fn closed(device: &Device, plan: &Plan, nwa: u32, sectors: u32) -> Result<bool> {
    let media = device
        .media
        .as_ref()
        .context("no finalized medium reported")?;
    ensure!(
        !media.blank && !media.appendable && media.sessions == 2,
        "drive did not report a finalized two-session disc"
    );
    let tracks = recorded(device);
    ensure!(
        tracks.len() == plan.tracks.len() + 1,
        "final disc does not contain the expected track count"
    );
    music_layout(&tracks, plan)?;
    let data = tracks.last().unwrap();
    ensure!(
        data.session == 2
            && !data.audio
            && data.number == tracks.len() as u32
            && data.start == Some(i64::from(nwa))
            && data
                .sectors
                .is_some_and(|length| length >= sectors || length.checked_add(2) == Some(sectors)),
        "final data track does not match the measured LBA and length"
    );
    Ok(data.sectors.is_some_and(|length| length < sectors))
}

/// Hash names, directory markers and file contents, independent of absolute job path.
fn fingerprint(root: &Path) -> Result<String> {
    fn visit(root: &Path, path: &Path, hash: &mut Sha256) -> Result<()> {
        let meta = std::fs::symlink_metadata(path)?;
        ensure!(
            meta.is_dir() || meta.is_file(),
            "recording snapshot contains a symlink or special file"
        );
        let name = path
            .strip_prefix(root)?
            .to_str()
            .context("non-Unicode snapshot path")?;
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        hash.update([u8::from(meta.is_dir())]);
        if meta.is_dir() {
            let mut entries = std::fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                visit(root, &entry.path(), hash)?;
            }
        } else {
            hash.update(meta.len().to_le_bytes());
            let mut file = File::open(path)?;
            let mut buf = [0; 65536];
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hash.update(&buf[..n]);
            }
        }
        Ok(())
    }
    let mut hash = Sha256::new();
    visit(root, root, &mut hash)?;
    Ok(crate::hex(&hash.finalize()))
}
