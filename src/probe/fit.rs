// SPDX-License-Identifier: GPL-3.0-or-later
//! CD geometry ladder from Ashley-QwQ/disc-probe and NaiveTomcat/discprobe-macos.
//! Fit dimensionless radius/pitch to progressively unwrapped spindle phases.
use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeSet,
    f64::consts::{PI, TAU},
    ops::Range,
};

use crate::{
    console,
    spiral::{ChannelSpiral, FRAME_BITS, READ_TAIL_FRAMES, SECTOR_BITS, read_end_bits},
};

const BIT_MM: f64 = 16.0 / SECTOR_BITS as f64;
const U: f64 = 25.0 / BIT_MM;
const V: f64 = 0.0016 / BIT_MM;
const SIG_U: f64 = 0.08 * U;
const SIG_V: f64 = 0.15 * V;
const SAFE_DEG: f64 = 25.0;

#[derive(Clone, Copy)]
pub(super) struct Sample {
    pub lba: u32,
    pub phase: f64,
    pub sigma: f64,
    pub spread: f64,
    pub used: usize,
    pub period: f64,
}

#[derive(Clone)]
pub(super) struct Point {
    pub sample: Sample,
    pub predicted: f64,
    pub unwrapped: f64,
}

type Vector = [f64; 3];
type Matrix = [Vector; 3];

#[derive(Clone)]
pub(super) struct Fit {
    pub geometry: ChannelSpiral,
    pub offset: f64,
    reference: f64,
    covariance: Matrix,
    pub points: Vec<Point>,
    pub rms_deg: f64,
    pub chi2_per_dof: f64,
    pub rejected: Vec<u32>,
}

impl Fit {
    fn new(reference: u32) -> Self {
        Self {
            geometry: ChannelSpiral { u: U, v: V },
            offset: 0.0,
            reference: read_end_bits(reference) as f64,
            covariance: [[0.0; 3]; 3],
            points: Vec::new(),
            rms_deg: 0.0,
            chi2_per_dof: 0.0,
            rejected: Vec::new(),
        }
    }

    pub fn predict(&self, lba: u32) -> f64 {
        self.geometry
            .angle_between(self.reference, read_end_bits(lba) as f64)
            + self.offset
    }

    fn gradient(&self, lba: u32) -> Vector {
        let [a, b] = self
            .geometry
            .gradient(self.reference, read_end_bits(lba) as f64);
        [a, b, 1.0]
    }

    fn lba_at_turns(&self, turns: f64) -> u32 {
        let r = self.geometry.radius_squared(self.reference).sqrt();
        let v = self.geometry.v;
        if r + v * turns <= 0.0 {
            return 0;
        }
        let n = self.reference + PI * turns * (2.0 * r + v * turns);
        ((n - f64::from(READ_TAIL_FRAMES * FRAME_BITS)) / f64::from(SECTOR_BITS) - 1.0).floor()
            as u32
    }

    pub fn span_sigma_deg(&self, first: u32, last: u32) -> f64 {
        let lo = self.gradient(first);
        let hi = self.gradient(last);
        self.sigma(std::array::from_fn(|i| hi[i] - lo[i]))
            .to_degrees()
    }

    fn sigma(&self, row: Vector) -> f64 {
        dot(row, self.covariance.map(|r| dot(r, row)))
            .max(0.0)
            .sqrt()
    }

    fn variance_after(&self, span: Vector, lba: u32, variance: f64) -> f64 {
        let row = self.gradient(lba);
        let column = self.covariance.map(|r| dot(r, row));
        self.sigma(span).powi(2) - dot(span, column).powi(2) / (variance + dot(row, column))
    }

    fn add(&mut self, sample: Sample) -> Result<()> {
        if self.points.is_empty() {
            self.offset = wrap(sample.phase - self.predict(sample.lba));
        }
        let predicted = self.predict(sample.lba);
        let difference = wrap(sample.phase - predicted);
        ensure!(
            difference.abs() <= PI / 2.0,
            "ambiguous whole-turn count at LBA {}",
            sample.lba
        );
        self.points.push(Point {
            sample,
            predicted,
            unwrapped: predicted + difference,
        });
        self.refit()
    }

    fn try_add(&mut self, sample: Sample) -> Result<()> {
        let mut trial = self.clone();
        trial.add(sample)?;
        *self = trial;
        Ok(())
    }

    fn equations(&self) -> Result<(Vector, Matrix)> {
        let mut rows = Vec::with_capacity(self.points.len() + 2);
        let mut residuals = Vec::with_capacity(rows.capacity());
        for point in &self.points {
            let sigma = point.sample.sigma;
            rows.push(self.gradient(point.sample.lba).map(|d| d / sigma));
            residuals.push((point.unwrapped - self.predict(point.sample.lba)) / sigma);
        }
        rows.extend([[1.0 / SIG_U, 0.0, 0.0], [0.0, 1.0 / SIG_V, 0.0]]);
        residuals.extend([(U - self.geometry.u) / SIG_U, (V - self.geometry.v) / SIG_V]);
        let scales: Vector =
            std::array::from_fn(|i| rows.iter().map(|r| r[i] * r[i]).sum::<f64>().sqrt());
        let mut gram = [[0.0; 3]; 3];
        let mut rhs = [0.0; 3];
        for (row, y) in rows.into_iter().zip(residuals) {
            let row: Vector = std::array::from_fn(|i| row[i] / scales[i]);
            for (i, normal) in gram.iter_mut().enumerate() {
                rhs[i] += row[i] * y;
                for (j, cell) in normal.iter_mut().enumerate() {
                    *cell += row[i] * row[j];
                }
            }
        }
        let inv = inverse(gram).context("ill-conditioned CD timing fit")?;
        let step = std::array::from_fn(|i| dot(inv[i], rhs) / scales[i]);
        let covariance =
            std::array::from_fn(|i| std::array::from_fn(|j| inv[i][j] / (scales[i] * scales[j])));
        Ok((step, covariance))
    }

    fn refit(&mut self) -> Result<()> {
        for _ in 0..15 {
            let (step, _) = self.equations()?;
            self.geometry.u += step[0];
            self.geometry.v += step[1];
            self.offset += step[2];
            ensure!(
                self.geometry.u.is_finite()
                    && self.geometry.u > 0.0
                    && self.geometry.v.is_finite()
                    && self.geometry.v > 0.0
                    && self.offset.is_finite(),
                "CD timing fit left its physical range"
            );
            if step.iter().all(|s| s.abs() < 1e-9) {
                break;
            }
        }
        let mut chi2 = 0.0;
        let mut sum2 = 0.0;
        for point in &self.points {
            let residual = point.unwrapped - self.predict(point.sample.lba);
            ensure!(
                residual.abs() <= PI / 2.0,
                "whole-turn fit disagrees at LBA {}",
                point.sample.lba
            );
            sum2 += residual * residual;
            chi2 += (residual / point.sample.sigma).powi(2);
        }
        self.chi2_per_dof = chi2 / self.points.len().saturating_sub(3).max(1) as f64;
        ensure!(
            self.chi2_per_dof <= 25.0,
            "CD timing residuals are inconsistent with a single spiral"
        );
        self.rms_deg = (sum2 / self.points.len() as f64).sqrt().to_degrees();
        let (_, covariance) = self.equations()?;
        self.covariance = covariance.map(|row| row.map(|x| x * self.chi2_per_dof.max(1.0)));
        Ok(())
    }
}

/// Data-only covariance, marginalizing each run's arbitrary phase offset.
/// Do not let the spiral priors manufacture precision for weak measurements.
pub(crate) fn covariance(
    model: ChannelSpiral,
    reference: u32,
    samples: &[(u32, f64, f64)],
) -> Result<[[f64; 2]; 2]> {
    ensure!(samples.len() > 3, "too few samples for timing covariance");
    let rows: Vec<Vector> = samples
        .iter()
        .map(|(lba, sigma, _)| {
            let [u, v] =
                model.gradient(read_end_bits(reference) as f64, read_end_bits(*lba) as f64);
            [u / sigma, v / sigma, 1.0 / sigma]
        })
        .collect();
    let scales: Vector =
        std::array::from_fn(|i| rows.iter().map(|r| r[i] * r[i]).sum::<f64>().sqrt());
    ensure!(
        scales.iter().all(|s| s.is_finite() && *s > 0.0),
        "unidentifiable timing geometry"
    );
    let mut gram = [[0.0; 3]; 3];
    for row in rows {
        for i in 0..3 {
            for j in 0..3 {
                gram[i][j] += row[i] * row[j] / (scales[i] * scales[j]);
            }
        }
    }
    let inverse =
        inverse(gram).context("timing geometry is not identifiable from the measured span")?;
    let chi2 =
        samples.iter().map(|(_, s, r)| (r / s).powi(2)).sum::<f64>() / (samples.len() - 3) as f64;
    // Overdispersion expands uncertainty; unusually quiet data never shrinks it.
    let inflation = chi2.max(1.0);
    Ok(std::array::from_fn(|i| {
        std::array::from_fn(|j| inverse[i][j] * inflation / (scales[i] * scales[j]))
    }))
}

pub(super) fn ladder(
    range: Range<u32>,
    reference: u32,
    partial: bool,
    mut measure: impl FnMut(u32) -> Result<Option<Sample>>,
) -> Result<Fit> {
    let mut fit = Fit::new(reference);
    let first = range.start;
    let last = range.end - 1;
    let allowed = |lba: u32| lba.clamp(first, last);
    let mut tried = BTreeSet::from([reference]);
    for turns in [0.5, 1.5] {
        let lba = allowed(fit.lba_at_turns(turns));
        ensure!(tried.insert(lba), "cannot place initial CD timing rungs");
        fit.add(
            measure(lba)?.with_context(|| format!("unstable initial measurement at LBA {lba}"))?,
        )?;
    }
    while fit.points.len() < 36 && tried.len() <= 64 {
        let mut choices: BTreeSet<_> = (0..40).map(|i| first + (last - first) * i / 39).collect();
        for k in 0..18 {
            let turns = f64::from(1u32 << k);
            choices.extend([
                allowed(fit.lba_at_turns(turns)),
                allowed(fit.lba_at_turns(-turns)),
            ]);
        }
        choices.retain(|lba| tried.iter().all(|old| lba.abs_diff(*old) >= 64));
        let safe = |set: &BTreeSet<u32>| {
            set.iter()
                .copied()
                .filter(|&lba| fit.sigma(fit.gradient(lba)).to_degrees() < SAFE_DEG)
                .collect::<Vec<_>>()
        };
        let mut reachable = safe(&choices);
        if reachable.is_empty() && fit.points.len() < 8 {
            // Closely spaced rungs can establish the slope before larger steps are safe.
            let near = (0..9)
                .flat_map(|k| {
                    let turns = f64::from(1u32 << k);
                    [
                        allowed(fit.lba_at_turns(turns)),
                        allowed(fit.lba_at_turns(-turns)),
                    ]
                })
                .filter(|lba| !tried.contains(lba))
                .collect();
            reachable = safe(&near);
        }
        if reachable.is_empty() {
            break;
        }
        let distance = |lba: u32| {
            tried
                .iter()
                .map(|old| lba.abs_diff(*old))
                .min()
                .unwrap_or(0)
        };
        let lba = if reachable.len() != choices.len() {
            reachable
                .into_iter()
                .max_by_key(|lba| lba.abs_diff(reference))
        } else if fit.points.len().is_multiple_of(3) {
            reachable.into_iter().max_by_key(|&lba| distance(lba))
        } else {
            let lo = fit.gradient(first);
            let hi = fit.gradient(last);
            let span = std::array::from_fn(|i| hi[i] - lo[i]);
            let variance = median(fit.points.iter().map(|p| p.sample.sigma).collect()).powi(2);
            reachable.into_iter().min_by(|&a, &b| {
                fit.variance_after(span, a, variance)
                    .total_cmp(&fit.variance_after(span, b, variance))
            })
        }
        .context("no reachable CD timing rung")?;
        tried.insert(lba);
        let before = fit.points.len();
        for attempt in 1..=if partial { 2 } else { 1 } {
            if let Some(sample) = measure(lba)? {
                if !partial {
                    fit.add(sample)?;
                    break;
                }
                match fit.try_add(sample) {
                    Ok(()) => break,
                    Err(error) => console::phase(
                        "probe",
                        format_args!("LBA {lba}, attempt {attempt}: {error}"),
                    ),
                }
            }
        }
        if fit.points.len() > before {
            console::phase(
                "probe",
                format_args!(
                    "Rung {}/36 | LBA {lba} | RMS {:.3}°",
                    fit.points.len(),
                    fit.rms_deg
                ),
            );
        } else if partial {
            fit.rejected.push(lba);
            console::warning(format_args!("Excluding LBA {lba} after remeasurement"));
        }
    }
    ensure!(
        fit.points.len() >= 8,
        "could not unwrap eight reliable CD timing rungs"
    );
    Ok(fit)
}

fn dot(a: Vector, b: Vector) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}

fn inverse(matrix: Matrix) -> Option<Matrix> {
    let mut rows: [[f64; 6]; 3] = std::array::from_fn(|i| {
        std::array::from_fn(|j| {
            if j < 3 {
                matrix[i][j]
            } else {
                if j - 3 == i { 1.0 } else { 0.0 }
            }
        })
    });
    for i in 0..3 {
        let pivot = (i..3).max_by(|&a, &b| rows[a][i].abs().total_cmp(&rows[b][i].abs()))?;
        if !rows[pivot][i].is_finite() || rows[pivot][i].abs() < 1e-15 {
            return None;
        }
        rows.swap(i, pivot);
        let divisor = rows[i][i];
        let basis = rows[i].map(|x| x / divisor);
        rows[i] = basis;
        for (_, row) in rows.iter_mut().enumerate().filter(|(j, _)| *j != i) {
            let factor = row[i];
            *row = std::array::from_fn(|k| row[k] - factor * basis[k]);
        }
    }
    Some(rows.map(|r| [r[3], r[4], r[5]]))
}

pub(super) fn wrap(value: f64) -> f64 {
    (value + PI).rem_euclid(TAU) - PI
}

pub(crate) fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    let n = values.len();
    (values[n / 2] + values[(n - 1) / 2]) / 2.0
}

#[cfg(test)]
mod uncertainty_tests {
    use super::*;
    #[test]
    fn noisy_spiral_fits_have_conservative_data_only_confidence_bands() {
        let mut random = 0x18273u64;
        let mut normal = || {
            let mut uniform = || {
                random ^= random << 13;
                random ^= random >> 7;
                random ^= random << 17;
                ((random >> 11) as f64 + 0.5) / ((1u64 << 53) as f64)
            };
            (-2.0 * uniform().ln()).sqrt() * (TAU * uniform()).cos()
        };
        let truth = ChannelSpiral {
            u: U * 1.002,
            v: V * 0.997,
        };
        let reference = 180_000;
        let mut total = 0;
        // Independent, heteroscedastic data; also test underestimated read-pair noise.
        for (noise_deg, excess) in [(0.05_f64, 1.0), (0.2, 1.0), (1.0, 1.0), (0.2, 2.0)] {
            let mut outside = 0;
            let mut worst_rms: f64 = 0.0;
            let trials = 600;
            for _ in 0..trials {
                let mut f = Fit::new(reference);
                f.geometry = truth;
                for i in 0..36 {
                    let lba = 2 + i * 10_000;
                    let sigma = (noise_deg * (1.0 + f64::from(i % 4) * 0.3)).to_radians();
                    let angle = truth
                        .angle_between(read_end_bits(reference) as f64, read_end_bits(lba) as f64);
                    let value = angle + 0.17 + sigma * excess * normal();
                    f.points.push(Point {
                        sample: Sample {
                            lba,
                            phase: wrap(value),
                            sigma,
                            spread: sigma,
                            used: 32,
                            period: 0.029,
                        },
                        predicted: angle,
                        unwrapped: value,
                    });
                }
                f.refit().unwrap();
                let samples: Vec<_> = f
                    .points
                    .iter()
                    .map(|p| {
                        (
                            p.sample.lba,
                            p.sample.sigma,
                            p.unwrapped - f.predict(p.sample.lba),
                        )
                    })
                    .collect();
                let cov = covariance(f.geometry, reference, &samples).unwrap();
                // Independently evaluate the nonlinear angular error at both extent ends.
                let first = read_end_bits(reference) as f64;
                for lba in [2, 350_002] {
                    let last = read_end_bits(lba) as f64;
                    let g = f.geometry.gradient(first, last);
                    let variance = g[0] * g[0] * cov[0][0]
                        + 2.0 * g[0] * g[1] * cov[0][1]
                        + g[1] * g[1] * cov[1][1];
                    let error =
                        f.geometry.angle_between(first, last) - truth.angle_between(first, last);
                    outside += usize::from(error.abs() > (-2.0 * 0.01_f64.ln() * variance).sqrt());
                }
                worst_rms = worst_rms.max(f.rms_deg);
            }
            let rate = outside as f64 / (2 * trials) as f64;
            println!(
                "noise_deg={noise_deg} excess={excess} nonlinear_band_outside={rate:.5} max_rms_deg={worst_rms:.3} fits={trials}"
            );
            assert!(rate < 0.025, "uncertainty band undercovers: {rate}");
            total += trials;
        }
        assert_eq!(total, 2400);
    }
}
