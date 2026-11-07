// SPDX-License-Identifier: GPL-3.0-or-later
//! Offline comparison of photo phase curves against measured CD timing.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fmt::Write, fs, path::Path};

use crate::{
    author,
    codec::CIRC_ORIGIN_FRAMES,
    preview::escape,
    spiral::{ChannelSpiral, FRAME_BITS, SECTOR_BITS, Spiral},
    timing,
};

const ORIGIN: f64 = CIRC_ORIGIN_FRAMES as f64;

#[derive(Deserialize)]
struct Points {
    format: String,
    bundle_sha256: String,
    evidence_sha256: String,
    convention: String,
    samples: Vec<Point>,
    #[serde(flatten)]
    evidence: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
struct Point {
    curve: String,
    sector: f64,
    phase_deg: f64,
}

type Curves = BTreeMap<String, Vec<(f64, f64)>>;

struct Model {
    geometry: ChannelSpiral,
    spiral: Spiral,
    measured: [f64; 2],
    artwork: [f64; 2],
}

impl Model {
    fn drift(&self, sector: f64, shift: f64) -> f64 {
        let anchor = (self.measured[0] + self.measured[1]) / 2.0;
        let bits = |s| s * f64::from(SECTOR_BITS) + shift * f64::from(FRAME_BITS);
        let observed = self.geometry.angle_between(bits(anchor), bits(sector));
        let generated = self.spiral.angle_between(anchor, sector);
        (observed - generated).to_degrees()
    }
}

pub fn compare(
    summary: &Path,
    bundle: &Path,
    points_path: &Path,
    evidence: Option<&Path>,
    offset: Option<(f64, &Path)>,
    output: &Path,
) -> Result<()> {
    let manifest_path = author::manifest_path(bundle);
    let manifest = author::load(bundle)?;
    let manifest_hash = author::digest(&manifest_path)?;
    let (geometry, bounds) = timing::phase_geometry(summary)?;
    let model = Model {
        geometry,
        spiral: Spiral::new(&manifest.plan.disc),
        measured: [f64::from(bounds[0]), f64::from(bounds[1] + 1)],
        artwork: [
            f64::from(manifest.plan.estimated_art_start_sector),
            f64::from(manifest.plan.estimated_art_end_sector),
        ],
    };
    ensure!(
        model
            .geometry
            .radius_squared(-1024.0 * f64::from(FRAME_BITS))
            > 0.0,
        "geometry does not cover the photo phase search range"
    );
    let points: Points =
        serde_json::from_value(timing::read(points_path)?).context("invalid phase points")?;
    ensure!(
        points.format == "discimage-phase-v1"
            && points.convention == "photo-minus-design-clockwise",
        "unsupported photo phase points or angle convention"
    );
    ensure!(
        points.bundle_sha256 == manifest_hash,
        "points belong to a different bundle"
    );
    ensure!(
        points.evidence_sha256.len() == 64
            && points
                .evidence_sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit()),
        "missing photo evidence hash"
    );
    if let Some(evidence) = evidence {
        ensure!(
            author::digest(evidence)? == points.evidence_sha256,
            "photo evidence checksum mismatch"
        );
    }
    let groups = curves(points.samples, &model)?;
    let offset_value = offset.map(|(value, _)| value);
    let (mut report, html) = analyze(&model, &groups, offset_value)?;
    report["sources"] = json!({
        "timing_sha256": author::digest(summary)?,
        "targets_sha256": author::digest(&summary.with_file_name("targets.csv"))?,
        "manifest_sha256": manifest_hash,
        "points_sha256": author::digest(points_path)?,
        "evidence_sha256": points.evidence_sha256,
        "evidence_hash_verified": evidence.is_some(),
    });
    if let Some((_, path)) = offset {
        report["sources"]["offset_evidence_sha256"] = json!(author::digest(path)?);
    }
    let mut metadata = points.evidence;
    metadata.retain(|k, _| {
        matches!(
            k.as_str(),
            "evidence_kind" | "original_photo" | "rectification"
        )
    });
    report["evidence"] = json!(metadata);
    fs::create_dir(output).context("use a new phase report output directory")?;
    fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    fs::write(output.join("index.html"), html)?;
    println!(
        "Photo phase report: {}",
        output.join("index.html").display()
    );
    Ok(())
}

fn curves(points: Vec<Point>, model: &Model) -> Result<Curves> {
    ensure!(
        !points.is_empty() && points.len() <= 4096,
        "expected 1–4096 photo phase samples"
    );
    let mut groups = Curves::new();
    for p in points {
        ensure!(
            !p.curve.trim().is_empty() && p.phase_deg.is_finite(),
            "invalid photo phase sample"
        );
        ensure!(
            (model.measured[0]..=model.measured[1]).contains(&p.sector)
                && (model.artwork[0]..model.artwork[1]).contains(&p.sector),
            "point outside the measured artwork range"
        );
        groups
            .entry(p.curve)
            .or_default()
            .push((p.sector, p.phase_deg));
    }
    ensure!(
        groups.len() <= 16,
        "expected at most 16 distinct photo curves"
    );
    for samples in groups.values_mut() {
        samples.sort_by(|a, b| a.0.total_cmp(&b.0));
        ensure!(
            samples.len() >= 12 && samples[samples.len() - 1].0 - samples[0].0 >= 1000.0,
            "each curve needs at least 12 points spanning 1000 sectors"
        );
        ensure!(
            samples
                .windows(2)
                .all(|p| p[0].0 < p[1].0 && (p[1].1 - p[0].1).abs() < 180.0),
            "duplicate sectors or discontinuous photo phase; unwrap the same edge first"
        );
    }
    Ok(groups)
}

#[derive(Serialize)]
struct Evaluation {
    rms_deg: f64,
    max_deg: f64,
    rotation_deg: f64,
    predicted_deg: Vec<f64>,
}

fn evaluate(model: &Model, samples: &[(f64, f64)], shift: f64) -> Evaluation {
    let mut predicted: Vec<_> = samples
        .iter()
        .map(|&(s, _)| model.drift(s, shift))
        .collect();
    let count = samples.len() as f64;
    let rotation = samples
        .iter()
        .zip(&predicted)
        .map(|((_, a), b)| a - b)
        .sum::<f64>()
        / count;
    let (sum2, max) =
        samples
            .iter()
            .zip(&mut predicted)
            .fold((0.0, 0.0f64), |(sum, max), ((_, a), b)| {
                *b += rotation;
                let error = *b - a;
                (sum + error * error, max.max(error.abs()))
            });
    Evaluation {
        rms_deg: (sum2 / count).sqrt(),
        max_deg: max,
        rotation_deg: rotation,
        predicted_deg: predicted,
    }
}

#[derive(Clone, Copy, Serialize)]
struct Shift {
    shift_frames: f64,
    forward_offset_frames: f64,
    rms_deg: f64,
}

fn cost(model: &Model, curves: &[&[(f64, f64)]], shift: f64) -> f64 {
    curves
        .iter()
        .map(|s| evaluate(model, s, shift).rms_deg.powi(2))
        .sum::<f64>()
        / curves.len() as f64
}

fn fit(model: &Model, curves: &[&[(f64, f64)]]) -> Result<Shift> {
    let grid = |i: usize| -1024.0 + i as f64 * 32.0;
    let costs: Vec<_> = (0..65).map(|i| cost(model, curves, grid(i))).collect();
    let minima: Vec<_> = (1..64)
        .filter(|&i| costs[i] < costs[i - 1] && costs[i] < costs[i + 1])
        .collect();
    let [index] = minima.as_slice() else {
        anyhow::bail!("no unique interior shift fit; inspect photo points and angle conventions");
    };
    ensure!(
        costs[*index] < costs[0].min(costs[64]),
        "best phase shift is outside the search range"
    );
    let (mut lo, mut hi) = (grid(index - 1), grid(index + 1));
    let ratio = (5f64.sqrt() - 1.0) / 2.0;
    for _ in 0..70 {
        let a = hi - ratio * (hi - lo);
        let b = lo + ratio * (hi - lo);
        if cost(model, curves, a) < cost(model, curves, b) {
            hi = b;
        } else {
            lo = a;
        }
    }
    let shift = (lo + hi) / 2.0;
    Ok(Shift {
        shift_frames: shift,
        forward_offset_frames: ORIGIN - shift,
        rms_deg: cost(model, curves, shift).sqrt(),
    })
}

fn analyze(model: &Model, groups: &Curves, offset: Option<f64>) -> Result<(Value, String)> {
    let all: Vec<_> = groups.values().map(Vec::as_slice).collect();
    let joint = if let Some(offset) = offset {
        let shift = ORIGIN - offset;
        ensure!(
            (-1024.0..=1024.0).contains(&shift),
            "external offset is outside the supported range"
        );
        Shift {
            shift_frames: shift,
            forward_offset_frames: offset,
            rms_deg: cost(model, &all, shift).sqrt(),
        }
    } else {
        fit(model, &all)?
    };
    let mut independent = BTreeMap::new();
    let mut curves = BTreeMap::new();
    let mut holdouts = Vec::new();
    let mut charts = String::new();
    for (name, samples) in groups {
        independent.insert(name, fit(model, &[samples])?);
        if groups.len() > 1 {
            let checked = if offset.is_some() {
                joint
            } else {
                let other: Vec<_> = groups
                    .iter()
                    .filter(|(k, _)| *k != name)
                    .map(|(_, v)| v.as_slice())
                    .collect();
                fit(model, &other)?
            };
            let test = evaluate(model, samples, checked.shift_frames);
            holdouts.push(json!({
                "held_out_curve": name, "shift_frames": checked.shift_frames,
                "rms_deg": test.rms_deg, "max_deg": test.max_deg,
                "photo_rotation_refitted": true, "shift_refitted": false,
            }));
        }
        let forward = evaluate(model, samples, ORIGIN);
        let fitted = evaluate(model, samples, joint.shift_frames);
        chart(&mut charts, name, samples, &forward, &fitted);
        curves.insert(
            name,
            json!({
                "samples": samples.len(), "lba_range": [samples[0].0, samples[samples.len()-1].0],
                "naive_origin": evaluate(model, samples, 0.0),
                "forward_origin": forward, "fitted": fitted,
            }),
        );
    }
    let report = json!({
        "format": "discimage-phase-report-v1", "joint": joint,
        "offset_source": if offset.is_some() { "external-measurement" } else { "curve-fit" },
        "individual": independent, "holdouts": holdouts, "origin_frames": CIRC_ORIGIN_FRAMES,
        "measured_lba_range": model.measured, "profile_applied": false,
        "physical_validation": false, "curves": curves,
    });
    let page = format!(
        r#"<!doctype html><html lang="en"><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1"><title>CD timing and photo phase</title>
<style>body{{margin:2rem auto;padding:0 1rem;max-width:900px;font:16px/1.5 system-ui;background:#111723;color:#e8edf6}}section{{background:#1b2434;border-radius:8px;padding:1rem;margin:1rem 0}}svg{{width:100%;font-size:13px}}text{{fill:#bac6da}}</style>
<h1>CD timing and photo phase</h1><p>Effective shift {:.3} frames · forward offset {:.3} frames · joint RMS {:.3}°</p>
<p>White dots show the photograph. Orange shows the forward origin and green shows the fitted shift. Each curve has its own photo rotation. See <a href="report.json">report.json</a> for source hashes and held-out errors.</p>
{charts}<p>The fit estimates an effective coordinate offset. The curves share one photo, and physical validation requires a recorded disc.</p></html>"#,
        joint.shift_frames, joint.forward_offset_frames, joint.rms_deg
    );
    Ok((report, page))
}

fn chart(
    out: &mut String,
    name: &str,
    samples: &[(f64, f64)],
    forward: &Evaluation,
    fitted: &Evaluation,
) {
    let values = samples
        .iter()
        .map(|p| p.1)
        .chain(forward.predicted_deg.iter().copied())
        .chain(fitted.predicted_deg.iter().copied());
    let (low, high) = values.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), n| {
        (lo.min(n), hi.max(n))
    });
    let (low, high) = (low - 0.1, high + 0.1);
    let (first, last) = (samples[0].0, samples[samples.len() - 1].0);
    let x = |s| 60.0 + (s - first) / (last - first) * 680.0;
    let y = |n| 235.0 - (n - low) / (high - low) * 195.0;
    let _ = write!(
        out,
        r#"<section><h2>{}</h2><p>Forward RMS {:.3}° · fitted RMS {:.3}°</p>
<svg viewBox="0 0 800 280" role="img" aria-label="Measured and fitted phase curves">
<path d="M60 30V235H750" fill="none" stroke="grey"/>
<text x="4" y="40">{high:.2}°</text><text x="4" y="235">{low:.2}°</text>
<text x="60" y="260">{first:.0}</text><text x="680" y="260">{last:.0}</text>"#,
        escape(name),
        forward.rms_deg,
        fitted.rms_deg
    );
    for (row, color) in [(forward, "#ffb454"), (fitted, "#66d9c3")] {
        out.push_str("<polyline points=\"");
        for (&(s, _), &n) in samples.iter().zip(&row.predicted_deg) {
            let _ = write!(out, "{:.2},{:.2} ", x(s), y(n));
        }
        let _ = write!(
            out,
            "\" fill=\"none\" stroke=\"{color}\" stroke-width=\"2\"/>"
        );
    }
    for &(s, n) in samples {
        let _ = write!(
            out,
            r#"<circle cx="{:.2}" cy="{:.2}" r="2.5" fill="white"/>"#,
            x(s),
            y(n)
        );
    }
    out.push_str("</svg></section>");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Disc;
    use std::f64::consts::PI;

    fn model() -> Model {
        let disc = Disc {
            capacity_sectors: 359_847,
            inner_radius_mm: 24.5,
            outer_radius_mm: 57.2275,
            velocity: 1183.1773,
            ..Disc::default()
        };
        Model {
            geometry: ChannelSpiral {
                u: 89_491.8,
                v: 5.4067,
            },
            spiral: Spiral::new(&disc),
            measured: [122_000.0, 356_000.0],
            artwork: [130_520.0, 359_697.0],
        }
    }

    fn samples(model: &Model, shift: f64, rotation: f64) -> Vec<(f64, f64)> {
        // Independent cumulative-angle form, with a different photo orientation.
        (0..40)
            .map(|i| {
                let sector = 150_000.0 + f64::from(i) * 4900.0;
                let n = sector * 57624.0 + shift * 588.0;
                let ChannelSpiral { u, v } = model.geometry;
                let measured = 2.0 * PI / v * ((u.powi(2) + v * n / PI).sqrt() - u);
                let generated = model.spiral.angle(model.spiral.radius(sector));
                (sector, (measured - generated).to_degrees() + rotation)
            })
            .collect()
    }

    #[test]
    fn fitted_and_fixed_phase_keep_independent_photo_rotations() {
        let model = model();
        let groups = Curves::from([
            ("a".into(), samples(&model, 92.0, 720.0)),
            ("<b>".into(), samples(&model, 92.0, -35.0)),
        ]);
        for offset in [None, Some(19.0)] {
            let (report, page) = analyze(&model, &groups, offset).unwrap();
            let joint = &report["joint"];
            assert!((joint["shift_frames"].as_f64().unwrap() - 92.0).abs() < 1e-4);
            assert!(joint["rms_deg"].as_f64().unwrap() < 1e-7);
            assert_eq!(report["physical_validation"], false);
            assert_eq!(report["profile_applied"], false);
            for check in report["holdouts"].as_array().unwrap() {
                assert!(check["max_deg"].as_f64().unwrap() < 1e-7);
                assert_eq!(check["shift_refitted"], false);
            }
            assert!(page.contains("&lt;b&gt;"));
        }
        let groups = Curves::from([
            ("a".into(), samples(&model, 80.0, 0.0)),
            ("b".into(), samples(&model, 150.0, 0.0)),
        ]);
        let (report, _) = analyze(&model, &groups, None).unwrap();
        assert!(report["holdouts"][0]["rms_deg"].as_f64().unwrap() > 1.0);
    }

    #[test]
    fn phase_points_reject_wrapping_and_out_of_range_samples() {
        let model = model();
        let points = || {
            samples(&model, 92.0, 0.0)
                .into_iter()
                .map(|(sector, phase_deg)| Point {
                    curve: "a".into(),
                    sector,
                    phase_deg,
                })
                .collect::<Vec<_>>()
        };
        assert!(curves(points(), &model).is_ok());
        let mut invalid = points();
        invalid[0].phase_deg += 360.0;
        assert!(curves(invalid, &model).is_err());
        let mut invalid = points();
        invalid[0].sector = 360_000.0;
        assert!(curves(invalid, &model).is_err());
    }
}
