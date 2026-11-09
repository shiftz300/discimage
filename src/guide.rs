//! Single-target calibration: record once, measure read-only, export a media preset.
use std::{
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use clap::Args;
use discimage::{calibrate, device, probe, timing};

#[derive(Args)]
pub struct Options {
    /// Working directory. Reuse it to continue from the last step.
    #[arg(short, long, required = true)]
    output: Option<PathBuf>,
    /// Drive ID or unique model; overrides the default saved by select.
    #[arg(long)]
    device: Option<String>,
    /// Preset name; defaults to the working directory name.
    #[arg(long)]
    name: Option<String>,
    /// Reuse a completed target-bound summary; repeat for 2–8 independent rounds.
    #[arg(long)]
    timing: Vec<PathBuf>,
    /// Repeat only the read-only measurement, preserving previous reports.
    #[arg(long, conflicts_with = "timing")]
    remeasure: bool,
    /// Requested CD speed multiplier; omit for the drive-selected maximum.
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..=64))]
    speed: Option<u32>,
    /// Proceed with recording without the interactive confirmation.
    #[arg(long, conflicts_with = "bundle")]
    write: bool,
    /// Use the verified target bundle corresponding to an externally recorded disc.
    #[arg(long, conflicts_with = "device")]
    bundle: Option<PathBuf>,
    /// Photo path, or place photo.jpg, photo.jpeg or photo.png in the working directory.
    #[arg(long)]
    photo: Option<PathBuf>,
    /// Exported measurements, or place points.json in the working directory.
    #[arg(long)]
    points: Option<PathBuf>,
}

pub fn run(options: Options) -> Result<()> {
    if options
        .bundle
        .as_ref()
        .is_some_and(|p| p.join("manifest.toml").exists() || p.join("manifest.json").exists())
    {
        return radial(options);
    }
    let output = options
        .output
        .as_deref()
        .context("calibrate requires --output DIR")?;
    std::fs::create_dir_all(output)?;
    let directory = std::fs::canonicalize(output)?;
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join(".calibrate.lock"))?;
    lock.try_lock()
        .context("another calibration is using this directory")?;
    let name = options
        .name
        .clone()
        .unwrap_or_else(|| directory.file_name().unwrap().to_string_lossy().into());
    ensure!(
        !name.trim().is_empty() && name.len() <= 128,
        "invalid medium name"
    );
    let bundle = options
        .bundle
        .clone()
        .unwrap_or_else(|| directory.join("record"));
    let selected = if !options.timing.is_empty() {
        None
    } else {
        crate::selection::preferred(options.device.as_deref())?
    };
    if !bundle.try_exists()? {
        ensure!(
            options.bundle.is_none() && options.timing.is_empty(),
            "supplied target does not exist"
        );
        let drives = device::list()?;
        if drives.is_empty() {
            println!(
                "[1/3] Waiting for a drive and blank CD-R/CD-RW. Connect it and rerun the same command."
            );
            return Ok(());
        }
        let drive = if let Some(id) = &selected {
            device::resolve(&drives, id)?
        } else {
            ensure!(
                drives.len() == 1,
                "Multiple drives found; run discimage select ID or pass --device ID"
            );
            &drives[0]
        };
        calibrate::record_confirmed(&bundle, Some(&drive.id), options.speed, false, |reserved| {
            let kind = reserved
                .media
                .as_ref()
                .context("missing reserved medium")?
                .kind
                .as_str();
            println!(
                "[1/3] Drive {} [{}], blank {}. One data target will be written and finalized.",
                reserved.name,
                reserved.selector(),
                kind
            );
            if options.write {
                return Ok(true);
            }
            if !io::stdin().is_terminal() {
                println!(
                    "Rerun in an interactive terminal to confirm, or explicitly pass --write."
                );
                return Ok(false);
            }
            print!("Write to this disc? Enter y to continue [y/N] ");
            io::stdout().flush()?;
            let mut answer = String::new();
            io::stdin().read_line(&mut answer)?;
            Ok(answer.trim().eq_ignore_ascii_case("y"))
        })?;
        if !bundle.try_exists()? {
            return Ok(());
        }
    }
    ensure!(
        bundle.join("single.json").try_exists()?,
        "Target write is not confirmed complete; no erase or rewrite attempted. Inspect the disc and {}",
        bundle.display()
    );
    let recording = calibrate::single(&bundle)?;
    let saved = recording
        .drive
        .as_ref()
        .context("Legacy target has no media identity; preserve its evidence and record a target in a new directory")?;
    let timing = if !options.timing.is_empty() {
        options.timing.clone()
    } else {
        let measurement = directory.join("timing");
        let mut force = options.remeasure;
        loop {
            let reports = measurements(&directory)?;
            if reports.len() >= 2 && !force {
                break reports;
            }
            let selector = selected.clone().unwrap_or_else(|| saved.selector());
            let drives = device::list()?;
            let drive = device::resolve(&drives, &selector)?;
            if measurement.try_exists()? {
                archive(&measurement)?;
            }
            let round = if force {
                "Additional".to_owned()
            } else {
                format!("Round {}/2:", reports.len() + 1)
            };
            println!(
                "[2/3] {round} independent read-only measurement on {}; no target rewrite.",
                drive.name
            );
            probe::acquire_target(&drive.id, &measurement, &bundle)?;
            force = false;
        }
    };
    let points = options.points.clone().or_else(|| {
        let p = directory.join("points.json");
        p.is_file().then_some(p)
    });
    let profile = directory.join("disc.toml");
    timing::preset(&timing, &bundle, &name, points.as_deref(), &profile)?;
    println!(
        "[3/3] Preset saved to {}, name {:?}.",
        profile.display(),
        name
    );
    println!(
        "preset = {{ file = {:?}, name = {:?} }}",
        profile.to_string_lossy(),
        name
    );
    println!(
        "Independent timing fits agree within their propagated uncertainty. The read origin assumes 108 frames; check physical artwork on the first disc."
    );
    let photo = options.photo.clone().or(find_photo(&directory)?);
    let mark = directory.join("mark.json");
    if !mark.try_exists()? {
        calibrate::mark(&bundle, &mark)?;
    }
    if points.is_none() {
        println!(
            "Optional refinement: import {} and {} in the website photo point picker, export points.json here, then rerun this command.",
            mark.display(),
            photo
                .as_ref()
                .map_or_else(|| "a target photo".into(), |p| p.display().to_string())
        );
        println!(
            "Without photo points, the length scale assumes a 58 mm outer recording radius. Photo refinement preserves the timing-derived angular mapping."
        );
    }
    Ok(())
}

fn measurements(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut previous = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        if let Some(index) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.strip_prefix("timing-previous-"))
            .and_then(|n| n.parse::<u32>().ok())
        {
            previous.push((index, entry.path()));
        }
    }
    previous.sort_by_key(|(n, _)| *n);
    let mut complete = Vec::new();
    for path in previous
        .into_iter()
        .map(|(_, p)| p)
        .chain([directory.join("timing")])
    {
        let status = path.join("status.json");
        if status.try_exists()? {
            ensure!(
                std::fs::metadata(&status)?.len() <= 8 * 1024 * 1024,
                "measurement status is too large"
            );
            let value: serde_json::Value = serde_json::from_slice(&std::fs::read(status)?)?;
            if value["completed"] == true {
                complete.push(path.join("summary.json"));
            }
        }
    }
    // Use the latest two complete rounds, never search history for passing pairs.
    if complete.len() > 2 {
        complete.drain(..complete.len() - 2);
    }
    Ok(complete)
}

fn archive(path: &Path) -> Result<()> {
    for attempt in 1..=1000 {
        let destination = path.with_file_name(format!("timing-previous-{attempt:03}"));
        if !destination.try_exists()? {
            std::fs::rename(path, destination)?;
            return Ok(());
        }
    }
    anyhow::bail!("too many previous measurements; archive this batch directory")
}

fn radial(options: Options) -> Result<()> {
    let directory = options
        .output
        .as_ref()
        .context("missing photo output directory")?;
    std::fs::create_dir_all(directory)
        .with_context(|| format!("Cannot create working directory {}", directory.display()))?;
    let directory = &std::fs::canonicalize(directory)?;
    let profile = directory.join("calibration.toml");
    if profile.try_exists()? {
        println!(
            "Radial fit complete. Add the [disc.calibration] section from {} to the album configuration.",
            profile.display()
        );
        return Ok(());
    }
    let bundle = options
        .bundle
        .as_ref()
        .context("missing legacy photo bundle")?
        .clone();
    let points = options
        .points
        .as_ref()
        .cloned()
        .unwrap_or_else(|| directory.join("points.json"));
    if options.points.is_some() || points.try_exists()? {
        let result = calibrate::fit(&bundle, &points, &profile)?;
        println!(
            "[3/3] Radial fit complete. Radii {:.3}–{:.3} mm, RMS {:.3} mm.",
            result.inner_radius_mm, result.outer_radius_mm, result.rms_mm
        );
        println!(
            "Add the [disc.calibration] section from {} to the album configuration.",
            profile.display()
        );
        return Ok(());
    }
    let photo = match options.photo {
        Some(path) => Some(path),
        None => find_photo(directory)?,
    };
    let Some(photo) = photo else {
        println!(
            "[2/3] Photograph the complete recording side straight on. Save photo.jpg or photo.png in {}, then rerun the same command.",
            directory.display()
        );
        return Ok(());
    };
    let target_digest = calibrate::digest(&bundle)?;
    let target = directory.join(format!("mark-{}.json", &target_digest[..12]));
    if !target.try_exists()? {
        calibrate::mark(&bundle, &target)?;
    }
    println!(
        "[2/3] Open the website photo point picker, import {} and {}, mark the points and export points.json.",
        target.display(),
        photo.display()
    );
    println!(
        "Save points.json in {}, then rerun the same command.",
        directory.display()
    );
    Ok(())
}

fn find_photo(directory: &Path) -> Result<Option<PathBuf>> {
    let mut photo = None;
    for name in ["photo.jpg", "photo.jpeg", "photo.png"] {
        let path = directory.join(name);
        if path.try_exists()? {
            ensure!(
                photo.is_none(),
                "Multiple photos found; use --photo to select one"
            );
            photo = Some(path);
        }
    }
    Ok(photo)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resume_keeps_the_latest_pair_without_cherry_picking() {
        let dir = std::env::temp_dir().join(format!("discimage-rounds-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, completed) in [
            ("timing-previous-001", true),
            ("timing-previous-003", true),
            ("timing-previous-002", false),
            ("timing", true),
        ] {
            std::fs::create_dir_all(dir.join(name)).unwrap();
            std::fs::write(
                dir.join(name).join("status.json"),
                serde_json::json!({"completed":completed}).to_string(),
            )
            .unwrap();
        }
        let result = measurements(&dir).unwrap();
        assert_eq!(
            result,
            vec![
                dir.join("timing-previous-003/summary.json"),
                dir.join("timing/summary.json")
            ]
        );
        std::fs::write(dir.join("timing/status.json"), "{\"completed\":false}").unwrap();
        let result = measurements(&dir).unwrap();
        assert_eq!(
            result,
            vec![
                dir.join("timing-previous-001/summary.json"),
                dir.join("timing-previous-003/summary.json")
            ]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
