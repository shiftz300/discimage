//! CD read-timing import. The converted candidate uses the existing
//! constant-pitch encoder, not a second rendering model.
//! u/v and read-end conventions: Ashley-QwQ/disc-probe, CD_PHYSICAL_MODEL.md;
//! macOS transport and numeric CSV: NaiveTomcat/discprobe-macos.
use std::{fs, path::Path};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    audio::SECTORS_PER_SECOND,
    author,
    codec::CIRC_ORIGIN_FRAMES,
    console,
    layout::Plan,
    preview,
    project::Project,
    spiral::{
        ChannelSpiral, FRAME_BITS, FRAMES_PER_SECTOR, READ_TAIL_FRAMES, SECTOR_BITS, Spiral,
        read_end_bits,
    },
};

/// This metadata follows the candidate into manifests and recording snapshots.
/// Bounds are encoding coordinates, end exclusive, not readable track bounds.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub first_sector: u32,
    pub end_sector: u32,
    pub summary_sha256: String,
    pub phase_sha256: String,
}

impl Evidence {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.first_sector < self.end_sector && self.end_sector <= 360_000,
            "invalid timing range"
        );
        for hash in [&self.summary_sha256, &self.phase_sha256] {
            ensure!(
                hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()),
                "invalid timing evidence hash"
            );
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct Fit {
    media: String,
    sector_bits: u32,
    read_tail_frames: u32,
    #[serde(flatten)]
    geometry: ChannelSpiral,
    residual_rms_deg: f64,
    angle_sigma_measured_span_deg: f64,
    reference_lba: u32,
}

pub(crate) fn read(path: &Path) -> Result<Value> {
    ensure!(
        fs::metadata(path)?.len() <= 8 * 1024 * 1024,
        "report is too large"
    );
    serde_json::from_slice(&fs::read(path)?)
        .with_context(|| format!("invalid JSON: {}", path.display()))
}

struct Measurement {
    summary: Value,
    fit: Fit,
    targets: std::path::PathBuf,
    bounds: [u32; 2],
}

fn measurement(path: &Path) -> Result<Measurement> {
    let summary = read(path)?;
    let fit: Fit =
        serde_json::from_value(summary["fit"].clone()).context("missing CD geometry fit")?;
    ensure!(
        fit.media == "cd"
            && fit.sector_bits == SECTOR_BITS
            && fit.read_tail_frames == READ_TAIL_FRAMES,
        "unsupported media or read-end convention; expected CD/57624 bits/108 frames"
    );
    ensure!(
        fit.geometry.u.is_finite()
            && fit.geometry.u > 0.0
            && fit.geometry.v.is_finite()
            && fit.geometry.v > 0.0
            && fit.reference_lba < 360_000,
        "invalid CD timing geometry"
    );
    let targets = path.with_file_name("targets.csv");
    let status = path.with_file_name("status.json");
    if status.try_exists()? {
        let receipt = read(&status)?;
        ensure!(receipt["completed"] == true, "measurement is incomplete");
        if let Some(hash) = receipt["summary_sha256"].as_str() {
            ensure!(
                author::digest(path)? == hash,
                "timing summary changed after acquisition"
            );
        }
        if let Some(hash) = receipt["targets_sha256"].as_str() {
            ensure!(
                author::digest(&targets)? == hash,
                "timing targets changed after acquisition"
            );
        }
    }
    let bounds = coverage(&targets, &fit, &summary)?;
    Ok(Measurement {
        summary,
        fit,
        targets,
        bounds,
    })
}

pub(crate) fn phase_geometry(path: &Path) -> Result<(ChannelSpiral, [u32; 2])> {
    let m = measurement(path)?;
    Ok((m.fit.geometry, m.bounds))
}

fn number(value: &Value) -> Result<f64> {
    value
        .as_f64()
        .filter(|n| n.is_finite())
        .context("expected finite numeric report field")
}

/// Read the numeric targets.csv emitted by the supported probe. `used` is a
/// count of accepted read pairs, not a Boolean. Never infer coverage from the
/// readable track bounds or from the requested experiment range.
fn coverage(path: &Path, fit: &Fit, summary: &Value) -> Result<[u32; 2]> {
    ensure!(
        fs::metadata(path)?.len() <= 8 * 1024 * 1024,
        "targets.csv is too large"
    );
    let text = fs::read_to_string(path)?;
    let mut lines = text.lines();
    let header: Vec<_> = lines
        .next()
        .context("empty targets.csv")?
        .split(',')
        .collect();
    let columns = ["lba", "n_bits", "used", "period_ms"].map(|key| {
        header
            .iter()
            .position(|s| *s == key)
            .with_context(|| format!("missing CSV column {key}"))
    });
    let [lba, bits, used, period] = columns;
    let (lba, bits, used, period) = (lba?, bits?, used?, period?);
    let mut points = vec![fit.reference_lba];
    let mut periods = Vec::new();
    for line in lines.filter(|s| !s.trim().is_empty()) {
        let row: Vec<_> = line.split(',').collect();
        ensure!(row.len() == header.len(), "invalid numeric targets.csv row");
        let sector: u32 = row[lba].parse()?;
        let n: f64 = row[bits].parse()?;
        let count: u32 = row[used].parse()?;
        let ms: f64 = row[period].parse()?;
        ensure!(
            sector < 360_000 && count > 0 && ms.is_finite() && ms > 0.0,
            "invalid timing sample"
        );
        ensure!(
            (n - read_end_bits(sector) as f64).abs() < 0.5,
            "targets use a different CD read-end coordinate convention"
        );
        points.push(sector);
        periods.push(ms);
    }
    points.sort_unstable();
    points.dedup();
    ensure!(
        points.len() >= 9,
        "need at least eight distinct timing targets plus a reference"
    );
    let bounds = [points[0], *points.last().unwrap()];
    ensure!(bounds[1] - bounds[0] >= 1000, "insufficient measured span");
    let low = periods.iter().copied().fold(f64::INFINITY, f64::min);
    let high = periods.iter().copied().fold(0.0, f64::max);
    ensure!(
        (high - low) / low <= 0.005,
        "rotation periods vary by more than 0.5%; remeasure"
    );
    for key in ["readable_lba_range", "experiment_range"] {
        if let Some(range) = summary["setup"][key].as_array() {
            ensure!(
                range.len() == 2
                    && f64::from(bounds[0]) >= number(&range[0])?
                    && f64::from(bounds[1]) < number(&range[1])?,
                "targets lie outside {key}"
            );
        }
    }
    Ok(bounds)
}

/// Translate a read-timing angle law into the existing physical model, using
/// the project's linear velocity only as a length-scale assumption. At encoder
/// sector s, S² = u² + v*(s*57624 + K*588)/π. With b=velocity/(75*57624),
/// r=b*S and span=b²*v*57624*N/π reproduce Theta differences exactly.
fn convert(old: &crate::project::Disc, fit: &Fit, shift: f64, anchor: f64) -> crate::project::Disc {
    let mut disc = old.clone();
    let bit = old.velocity / f64::from(SECTORS_PER_SECOND * SECTOR_BITS);
    let inner2 = bit.powi(2) * fit.geometry.radius_squared(shift * f64::from(FRAME_BITS));
    let span =
        bit.powi(2) * fit.geometry.v * f64::from(SECTOR_BITS) * f64::from(old.capacity_sectors)
            / std::f64::consts::PI;
    disc.inner_radius_mm = inner2.sqrt();
    disc.outer_radius_mm = (inner2 + span).sqrt();
    disc.calibration = None; // The inherited radial fit's residual is not this fit's error.
    disc.timing = None;
    disc.angle = 0.0;
    let source = Spiral::new(old);
    let candidate = Spiral::new(&disc);
    disc.angle = (source.angle(source.radius(anchor)) - candidate.angle(candidate.radius(anchor)))
        .to_degrees()
        .rem_euclid(360.0);
    disc
}

pub fn import(
    project: &Path,
    summary: &Path,
    phase: &Path,
    bundle: &Path,
    output: &Path,
    size: u32,
) -> Result<()> {
    ensure!(
        (256..=4096).contains(&size),
        "preview size must be 256–4096"
    );
    let mut project = Project::load(&fs::canonicalize(project)?)?;
    ensure!(
        project.disc.timing.is_none(),
        "start from the original project, not another timing candidate"
    );
    let original = Plan::from_project(&project, None)?;
    let Measurement {
        summary: measured,
        fit,
        targets,
        bounds,
    } = measurement(summary)?;
    ensure!(
        (0.0..=0.3).contains(&fit.residual_rms_deg)
            && (0.0..=0.5).contains(&fit.angle_sigma_measured_span_deg),
        "timing fit exceeds candidate quality limits (RMS 0.3°, span sigma 0.5°)"
    );
    let cache = &measured["cache"];
    ensure!(
        (cache["strategy"] == "fua" && cache["fua_reads_media"] == true)
            || (cache["strategy"] == "plain" && cache["plain_served_from_cache"] == false),
        "media reads not established: require verified FUA or uncached plain reads"
    );
    let report = read(phase)?;
    ensure!(
        report["format"] == "discimage-phase-report-v1"
            && report["origin_frames"] == CIRC_ORIGIN_FRAMES,
        "unsupported photo phase report"
    );
    let summary_hash = author::digest(summary)?;
    let targets_hash = author::digest(&targets)?;
    let manifest_hash = author::digest(&author::manifest_path(bundle))?;
    ensure!(
        report["sources"]["timing_sha256"] == summary_hash
            && report["sources"]["targets_sha256"] == targets_hash
            && report["sources"]["manifest_sha256"] == manifest_hash
            && report["sources"]["evidence_hash_verified"] == true,
        "phase report must match this timing/bundle and have verified photo evidence"
    );
    let generating = author::load(bundle)?;
    let a = &original.disc;
    let b = &generating.plan.disc;
    ensure!(
        a.geometry() == b.geometry() && a.velocity == b.velocity && a.angle == b.angle,
        "project geometry differs from the photo's generating bundle"
    );
    let shift = number(&report["joint"]["shift_frames"])?;
    let offset = number(&report["joint"]["forward_offset_frames"])?;
    ensure!(
        (-1024.0..=1024.0).contains(&shift)
            && (f64::from(CIRC_ORIGIN_FRAMES) - shift - offset).abs() < 1e-7
            && (0.0..=0.3).contains(&number(&report["joint"]["rms_deg"])?),
        "inconsistent or poor phase fit"
    );
    let curves = report["curves"]
        .as_object()
        .context("missing photo curves")?;
    let holdouts = report["holdouts"]
        .as_array()
        .context("missing phase validation")?;
    ensure!(
        curves.len() >= 2 && holdouts.len() >= 2,
        "need at least two photo curves and validation checks"
    );
    for check in holdouts {
        ensure!(
            (0.0..=0.3).contains(&number(&check["rms_deg"])?)
                && (0.0..=1.0).contains(&number(&check["max_deg"])?),
            "phase validation exceeds limits"
        );
    }
    // Convert read-end coverage into encoder coordinates, then round inward.
    let sector_shift = (f64::from(READ_TAIL_FRAMES) - shift) / f64::from(FRAMES_PER_SECTOR);
    let start = (f64::from(bounds[0]) + 1.0 + sector_shift).ceil() as u32;
    let end = (f64::from(bounds[1]) + 1.0 + sector_shift).floor() as u32;
    let anchor = (f64::from(start) + f64::from(end)) / 2.0;
    project.disc = convert(a, &fit, shift, anchor);
    let phase_hash = author::digest(phase)?;
    project.disc.timing = Some(Evidence {
        first_sector: start,
        end_sector: end,
        summary_sha256: summary_hash.clone(),
        phase_sha256: phase_hash.clone(),
    });
    project.disc.limit_end(end);
    let candidate = Plan::from_project(&project, None)?;
    // Decode all images and generate previews before creating the destination.
    console::phase(
        "preview",
        format_args!("Building the design and measured-model comparison"),
    );
    let design = preview::render(&candidate, project.artwork.as_ref(), size)?;
    let before = preview::projected(&original, project.artwork.as_ref(), &candidate, size)?;
    let after = preview::projected(&candidate, project.artwork.as_ref(), &candidate, size)?;
    let old_spiral = Spiral::new(a);
    let new_spiral = Spiral::new(&candidate.disc);
    let drift = |sector: f64| {
        // Anchor-relative differences, avoiding an arbitrary whole-disc rotation.
        let delta = |s: &Spiral| s.angle_between(anchor, sector);
        (delta(&new_spiral) - delta(&old_spiral)).to_degrees()
    };
    let changes: Vec<_> = (0..=100)
        .map(|i| {
            let sector = f64::from(candidate.estimated_art_start_sector)
                + f64::from(candidate.artwork_sectors) * f64::from(i) / 100.0;
            json!({"sector": sector, "old_drift_deg": drift(sector)})
        })
        .collect();
    let output_report = json!({
        "format": "discimage-timing-candidate-v1", "physical_validation": false,
        "profile_applied": false, "length_scale": "inherited project velocity; not measured by timing",
        "shift_frames": shift, "forward_offset_frames": offset, "anchor_sector": anchor,
        "measured_target_lbas": bounds, "encoding_range": [start, end],
        "original": original, "candidate": candidate, "drift": changes,
        "sources": {"summary_sha256": summary_hash, "targets_sha256": targets_hash,
            "phase_sha256": phase_hash, "manifest_sha256": manifest_hash},
        "timing": measured, "phase": report,
    });
    let settings = toml::to_string_pretty(&project)?;
    fs::create_dir(output)
        .with_context(|| format!("use a new output directory: {}", output.display()))?;
    fs::write(
        output.join("incomplete"),
        "Candidate generation incomplete\n",
    )?;
    preview::png(&design, &output.join("design.png"))?;
    preview::png(&before, &output.join("before.png"))?;
    preview::png(&after, &output.join("after.png"))?;
    preview::difference(
        &output.join("before.png"),
        &output.join("after.png"),
        &output.join("difference.png"),
    )?;
    fs::write(output.join("candidate.toml"), settings)?;
    fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&output_report)?,
    )?;
    fs::copy(summary, output.join("summary.json"))?;
    fs::copy(targets, output.join("targets.csv"))?;
    fs::copy(phase, output.join("phase.json"))?;
    fs::remove_file(output.join("incomplete"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{Disc, Profile};

    #[test]
    fn conversion_matches_independent_channel_angle_and_preserves_anchor() {
        let fit = Fit {
            media: "cd".into(),
            sector_bits: 57624,
            read_tail_frames: 108,
            geometry: ChannelSpiral {
                u: 89491.85907555123,
                v: 5.406737380826635,
            },
            residual_rms_deg: 0.1,
            angle_sigma_measured_span_deg: 0.11,
            reference_lba: 239000,
        };
        let old = Disc {
            velocity: 1183.177343317657,
            angle: 12.91,
            calibration: Some(Profile {
                reference_sectors: 360000,
                inner_radius_mm: 24.5,
                outer_radius_mm: 57.2275,
                rms_mm: 0.01,
            }),
            ..Disc::default()
        };
        let anchor = 239000.0;
        let previous = Spiral::new(&old);
        for shift in [0.0, 92.0, 108.68214925180564, 111.0, 200.0] {
            let mut disc = convert(&old, &fit, shift, anchor);
            let converted = Spiral::new(&disc);
            let theta = |s: f64| {
                let ChannelSpiral { u, v } = fit.geometry;
                std::f64::consts::TAU / v
                    * ((u * u + v * (s * 57624.0 + shift * 588.0) / std::f64::consts::PI).sqrt()
                        - u)
            };
            let rotation =
                converted.angle(converted.radius(anchor)) - previous.angle(previous.radius(anchor));
            assert!(rotation.sin().abs() < 1e-9 && rotation.cos() > 0.99999);
            for s in [122001.0, 150000.125, 239000.0, 355999.0] {
                let difference = converted.angle(converted.radius(s))
                    - converted.angle(converted.radius(anchor));
                assert!((difference - (theta(s) - theta(anchor))).abs() < 1e-9);
            }
            let before = converted.point(130521 * 2352 + 138);
            disc.limit_end(350000);
            assert_eq!(Spiral::new(&disc).point(130521 * 2352 + 138), before);
            assert!(disc.calibration.is_none());
        }
    }
}
