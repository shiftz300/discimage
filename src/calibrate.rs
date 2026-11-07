//! Photo-based radial calibration of the Mode 1 artwork track.
//! The wide-band measurement workflow is inspired by DiscImageStudio.
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::Path,
};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::{
    audio, author, console,
    layout::Plan,
    project::{Disc, Project, Track},
    target::Target,
};

const PHOTO_RADIUS: f64 = 60.0;
const HOLE_RADIUS: f64 = 7.5;

// Preserve the original public type paths for library callers.
pub use crate::project::Profile;
pub use crate::target::{PhaseBand, PhaseTarget};

const SINGLE_FORMAT: &str = "discimage-calibration-single-v1";
const DRAWING_START: u32 = 300;
const END_MARGIN: u32 = 150;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SingleTarget {
    format: String,
    reference_sectors: u32,
    written_sectors: u32,
    iso_sha256: String,
}

pub fn record_single(
    directory: &Path,
    device: Option<&str>,
    speed: Option<u32>,
    dry_run: bool,
) -> Result<()> {
    ensure!(
        speed.is_none_or(|speed| (1..=64).contains(&speed)),
        "speed must be 1–64x"
    );
    #[cfg(target_os = "macos")]
    {
        let device = device.map(crate::device::resolve_id).transpose()?;
        crate::macos::record_calibration(directory, device.as_deref(), speed, dry_run)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (directory, device, speed, dry_run);
        anyhow::bail!("Calibration recording requires macOS; photo fitting works offline")
    }
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn prepare_single(
    directory: &Path,
    free_sectors: u32,
    cancelled: impl Fn() -> bool,
) -> Result<u32> {
    use crate::iso::Volume;
    use std::io::BufWriter;
    // Use the measured capacity up to the encoder's supported range.
    let reference = free_sectors.min(360_000);
    let written = reference.saturating_sub(END_MARGIN);
    ensure!(
        written > DRAWING_START + 1000,
        "blank CD reports too little usable capacity"
    );
    let volume = Volume::new(None, 0, written, DRAWING_START, "CD radial calibration")?;
    std::fs::create_dir(directory).with_context(|| {
        format!(
            "cannot create new calibration directory {}",
            directory.display()
        )
    })?;
    let path = directory.join("calibration.iso");
    let mut iso = BufWriter::with_capacity(
        1024 * 1024,
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?,
    );
    volume.prefix(&mut iso)?;
    let extent = DRAWING_START..written;
    let target = Target::for_range(extent.clone());
    author::encode_drawing(&mut iso, extent, |p| target.palette(p), cancelled)?;
    iso.into_inner()?.sync_all()?;
    let receipt = SingleTarget {
        format: SINGLE_FORMAT.into(),
        reference_sectors: reference,
        written_sectors: written,
        iso_sha256: author::digest(&path)?,
    };
    write(
        &directory.join("pending.json"),
        &serde_json::to_vec_pretty(&receipt)?,
    )?;
    Ok(written)
}

/// Create a standalone project using the same multisession path as an album.
pub fn create(directory: &Path, capacity: u32) -> Result<()> {
    let project = Project {
        title: "CD radial calibration".into(),
        cd_text: None,
        tracks: vec![Track {
            path: "silence.wav".into(),
        }],
        disc: Disc {
            capacity_sectors: capacity,
            data_reserve_sectors: 300,
            calibration_target: true,
            ..Disc::default()
        },
        artwork: None,
        files: None,
    };
    // Reject a bad capacity before touching the destination.
    Plan::new(
        project.title.clone(),
        project.disc.clone(),
        vec![audio::AudioTrack {
            path: project.tracks[0].path.clone(),
            sample_frames: 300 * audio::SAMPLES_PER_SECTOR,
            sectors: 300,
            padding_sample_frames: 0,
            data_offset: 44,
        }],
    )?;
    std::fs::create_dir(directory)
        .with_context(|| format!("cannot create new target directory {}", directory.display()))?;
    audio::silence(&directory.join("silence.wav"))?;
    write(
        &directory.join("target.toml"),
        toml::to_string_pretty(&project)?.as_bytes(),
    )
}

fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("cannot create {}", path.display()))?
        .write_all(bytes)?;
    Ok(())
}

fn bundle(directory: &Path) -> Result<(Target, u32, String)> {
    let single = directory.join("single.json");
    if single.try_exists()? {
        let file = File::open(&single)?;
        ensure!(
            file.metadata()?.len() <= 4096,
            "calibration receipt is too large"
        );
        let receipt: SingleTarget = serde_json::from_reader(file)?;
        ensure!(
            receipt.format == SINGLE_FORMAT
                && receipt.reference_sectors <= 360_000
                && receipt.reference_sectors.checked_sub(END_MARGIN)
                    == Some(receipt.written_sectors)
                && receipt.written_sectors > DRAWING_START + 1000,
            "invalid single-session calibration receipt"
        );
        return Ok((
            Target::for_range(DRAWING_START..receipt.written_sectors),
            receipt.reference_sectors,
            author::digest(&single)?,
        ));
    }
    let manifest = author::load(directory)?;
    ensure!(
        manifest.plan.disc.calibration_target,
        "bundle has no calibration target"
    );
    let asset = manifest
        .assets
        .iter()
        .find(|asset| asset.path == "target.json")
        .context("bundle is missing its target metadata")?;
    let path = directory.join("target.json");
    let file = File::open(&path)?;
    ensure!(
        asset.bytes <= 4096
            && file.metadata()?.len() == asset.bytes
            && author::digest(&path)? == asset.sha256,
        "calibration target metadata checksum mismatch"
    );
    let target: Target = serde_json::from_reader(file)?;
    let plan = &manifest.plan;
    let edges = target.edges();
    ensure!(
        target.format == "discimage-target-v1"
            && edges[0] > plan.estimated_art_start_sector
            && edges[5] < plan.estimated_art_end_sector
            && edges.windows(2).all(|edge| edge[0] < edge[1]),
        "invalid calibration target bands"
    );
    Ok((
        target,
        plan.disc.capacity_sectors,
        author::digest(&author::manifest_path(directory))?,
    ))
}

pub fn digest(directory: &Path) -> Result<String> {
    Ok(bundle(directory)?.2)
}

/// Export the identity of a verified target for the website's photo point picker.
pub fn mark(directory: &Path, output: &Path) -> Result<()> {
    let descriptor = serde_json::json!({
        "format": "discimage-mark-v1",
        "bundle_sha256": digest(directory)?,
    });
    write(output, &serde_json::to_vec_pretty(&descriptor)?)
}

type Point = [f64; 2];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Points {
    bundle_sha256: String,
    width: u32,
    height: u32,
    rim: [Point; 4],
    hole: [Point; 4],
    edges: [Point; 6],
}

struct Circle {
    center: Point,
    radius: f64,
    max_error: f64,
}

fn distance(a: Point, b: Point) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// Centered algebraic circle fit. Fit both circles independently to catch tilt
/// and clicks on the recording area instead of the physical boundaries.
fn circle(points: &[Point; 4]) -> Result<Circle> {
    let mean = [
        points.iter().map(|p| p[0]).sum::<f64>() / 4.0,
        points.iter().map(|p| p[1]).sum::<f64>() / 4.0,
    ];
    let (mut xx, mut xy, mut yy, mut xz, mut yz) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for &[x, y] in points {
        let x = x - mean[0];
        let y = y - mean[1];
        let z = (x * x + y * y) / 2.0;
        xx += x * x;
        xy += x * y;
        yy += y * y;
        xz += x * z;
        yz += y * z;
    }
    let determinant = xx * yy - xy * xy;
    ensure!(
        determinant > 0.08 * (xx + yy).powi(2),
        "circle points must cover all four sides"
    );
    let center = [
        mean[0] + (yy * xz - xy * yz) / determinant,
        mean[1] + (xx * yz - xy * xz) / determinant,
    ];
    let distances = points.map(|p| distance(p, center));
    let radius = distances.iter().sum::<f64>() / 4.0;
    ensure!(
        distance(mean, center) < 0.25 * radius,
        "circle points are unevenly spread"
    );
    let max_error = distances
        .iter()
        .map(|r| (r - radius).abs())
        .fold(0.0, f64::max);
    Ok(Circle {
        center,
        radius,
        max_error,
    })
}

fn radial_fit(target: &Target, reference: u32, points: &Points) -> Result<(Profile, [f64; 6])> {
    ensure!(
        (1..=8192).contains(&points.width)
            && (1..=8192).contains(&points.height)
            && points
                .rim
                .iter()
                .chain(&points.hole)
                .chain(&points.edges)
                .all(|p| (0.0..=f64::from(points.width)).contains(&p[0])
                    && (0.0..=f64::from(points.height)).contains(&p[1])),
        "photo points fall outside the image"
    );
    let rim = circle(&points.rim).context("outer rim")?;
    let hole = circle(&points.hole).context("center hole")?;
    ensure!(
        rim.radius >= 180.0,
        "use a larger, sharp photo of the entire disc"
    );
    let scale = PHOTO_RADIUS / rim.radius;
    ensure!(
        rim.max_error * scale <= 0.40
            && hole.max_error * scale <= 0.30
            && distance(rim.center, hole.center) * scale <= 0.65
            && (hole.radius * scale - HOLE_RADIUS).abs() <= 0.55,
        "physical boundaries do not match a front-on 120 mm CD; recheck points or retake photo"
    );

    let radii = points.edges.map(|p| distance(p, rim.center) * scale);
    ensure!(
        radii.windows(2).all(|r| r[0] < r[1]) && radii[0] > HOLE_RADIUS && radii[5] < PHOTO_RADIUS,
        "mark the six band edges in order from the center outward"
    );
    let angle = |p: Point| (p[1] - rim.center[1]).atan2(p[0] - rim.center[0]);
    let first = angle(points.edges[0]);
    ensure!(
        points
            .edges
            .iter()
            .all(|&p| (angle(p) - first).cos() >= 6f64.to_radians().cos()),
        "take the six band edges along a single ray"
    );

    let x = target
        .edges()
        .map(|lba| f64::from(lba) / f64::from(reference));
    let y = radii.map(|r| r * r);
    let x_mean = x.iter().sum::<f64>() / 6.0;
    let y_mean = y.iter().sum::<f64>() / 6.0;
    let slope = x
        .iter()
        .zip(y)
        .map(|(x, y)| (x - x_mean) * (y - y_mean))
        .sum::<f64>()
        / x.iter().map(|x| (x - x_mean).powi(2)).sum::<f64>();
    let intercept = y_mean - slope * x_mean;
    let errors = std::array::from_fn(|i| (intercept + slope * x[i]).sqrt() - radii[i]);
    ensure!(
        errors.iter().all(|err| err.abs() <= 0.45),
        "band edges do not fit the radial model within 0.45 mm; check the points and bundle"
    );
    let profile = Profile {
        reference_sectors: reference,
        inner_radius_mm: intercept.sqrt(),
        outer_radius_mm: (intercept + slope).sqrt(),
        rms_mm: (errors.iter().map(|e| e * e).sum::<f64>() / 6.0).sqrt(),
    };
    profile
        .validate()
        .context("measured radii are inconsistent with the CD model")?;
    Ok((profile, errors))
}

pub fn fit(directory: &Path, points: &Path, output: &Path) -> Result<Profile> {
    let (target, reference, digest) = bundle(directory)?;
    let points = File::open(points)?;
    ensure!(
        points.metadata()?.len() <= 64 * 1024,
        "points file is too large"
    );
    let points: Points = serde_json::from_reader(points).context("invalid photo points file")?;
    ensure!(
        points.bundle_sha256 == digest,
        "points belong to a different target bundle"
    );
    let (profile, errors) = radial_fit(&target, reference, &points)?;
    let mut settings = String::from("[disc.calibration]\n");
    settings.push_str(&toml::to_string_pretty(&profile)?);
    write(output, settings.as_bytes())?;
    console::phase(
        "fit",
        format_args!("Band residuals (fitted minus observed, mm): {errors:+.3?}"),
    );
    Ok(profile)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Seek, SeekFrom};

    #[test]
    fn single_target_uses_reported_capacity_and_requires_a_completed_receipt() {
        let root = std::env::temp_dir().join(format!("discimage-single-{}", std::process::id()));
        let result = (|| -> Result<()> {
            let sectors = prepare_single(&root, 2000, || false)?;
            assert_eq!(sectors, 1850);
            let mut iso = File::open(root.join("calibration.iso"))?;
            assert_eq!(iso.metadata()?.len(), u64::from(sectors) * 2048);
            iso.seek(SeekFrom::Start(16 * 2048))?;
            let mut header = [0; 84];
            iso.read_exact(&mut header)?;
            assert_eq!(&header[..7], b"\x01CD001\x01");
            assert_eq!(u32::from_le_bytes(header[80..84].try_into()?), sectors);
            assert!(bundle(&root).is_err());
            std::fs::rename(root.join("pending.json"), root.join("single.json"))?;
            let (target, reference, _) = bundle(&root)?;
            assert_eq!(reference, 2000);
            assert_eq!(target.edges(), [377, 563, 796, 1090, 1369, 1757]);
            Ok(())
        })();
        std::fs::remove_dir_all(root).unwrap();
        result.unwrap();
    }
}
