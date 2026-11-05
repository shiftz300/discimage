//! Photo-based radial calibration of the Mode 1 artwork track.
//! The wide-band measurement workflow is inspired by DiscImageStudio.
use std::{
    fs::{File, OpenOptions},
    io::{Cursor, Write},
    path::Path,
};

use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use image::{ImageFormat, ImageReader};
use serde::Deserialize;

use crate::{
    audio, author,
    layout::Plan,
    project::{Disc, Project, Track},
    target::Target,
};

const PHOTO_RADIUS: f64 = 60.0;
const HOLE_RADIUS: f64 = 7.5;

// Preserve the original public type paths for library callers.
pub use crate::project::Profile;
pub use crate::target::{PhaseBand, PhaseTarget};

/// Create a standalone project using the same multisession path as an album.
pub fn create(directory: &Path, capacity: u32) -> Result<()> {
    let project = Project {
        title: "CD radial calibration".into(),
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
        author::digest(&directory.join("manifest.json"))?,
    ))
}

/// Produce a self-contained local page for taking points without a GUI dependency.
pub fn mark(directory: &Path, photo: &Path, output: &Path) -> Result<()> {
    let (_, _, digest) = bundle(directory)?;
    let mut reader = ImageReader::open(photo)?.with_guessed_format()?;
    ensure!(
        matches!(reader.format(), Some(ImageFormat::Png | ImageFormat::Jpeg)),
        "photo must be PNG or JPEG"
    );
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let photo = reader.decode()?.thumbnail(3072, 3072);
    let mut bytes = Cursor::new(Vec::new());
    photo.write_to(&mut bytes, ImageFormat::Png)?;
    let page = include_str!("mark.html")
        .replace("@BUNDLE@", &digest)
        .replace("@PHOTO@", &STANDARD.encode(bytes.into_inner()));
    write(output, page.as_bytes())
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
    eprintln!("Band residuals (fitted − observed, mm): {errors:+.3?}");
    Ok(profile)
}
