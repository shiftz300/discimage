//! Repeatability of independently acquired timing fits, in their measured domain.
//! Covariance is reconstructed from read-pair uncertainties, without spiral priors.
use super::{Measurement, quality, read};
use crate::{
    author,
    probe::fit,
    spiral::{FRAME_BITS, READ_TAIL_FRAMES, SECTOR_BITS},
};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{collections::BTreeSet, fs, path::Path};

const ALPHA: f64 = 0.01;
const GRID: usize = 256;
type Covariance = [[f64; 2]; 2];

#[derive(Debug, Serialize)]
pub struct Round {
    pub summary_sha256: String,
    pub targets_sha256: String,
    pub residual_rms_deg: f64,
    pub chi2_per_dof: f64,
    pub measured_span_sigma_deg: f64,
    pub median_phase_scatter_deg: Option<f64>,
    pub median_completion_jitter_us: Option<f64>,
    pub covariance_uv: Covariance,
    pub max_angle_sigma_deg: f64,
    pub max_confidence_band_deg: f64,
}
#[derive(Debug, Serialize)]
pub struct Pair {
    pub first: usize,
    pub second: usize,
    pub mahalanobis_squared: f64,
    pub limit: f64,
    pub consistent: bool,
    pub curve_rms_deg: f64,
    pub curve_max_deg: f64,
}
#[derive(Debug, Serialize)]
pub struct Assessment {
    pub method: &'static str,
    pub confidence_level: f64,
    pub encoding_range: [u32; 2],
    pub anchor_sector: f64,
    pub selected: usize,
    pub rounds: Vec<Round>,
    pub pairs: Vec<Pair>,
    pub consistent: bool,
}

pub(super) fn assess(measurements: &[Measurement]) -> Result<Assessment> {
    ensure!(
        (2..=8).contains(&measurements.len()),
        "need 2–8 independent completed timing rounds"
    );
    let target = measurements[0].summary["calibration_sha256"]
        .as_str()
        .context("timing report is not bound to a calibration target")?;
    ensure!(target.len() == 64, "invalid calibration target digest");
    let mut hashes = BTreeSet::new();
    let mut sample_hashes = BTreeSet::new();
    let mut rounds = Vec::new();
    let mut models = Vec::new();
    let first = measurements.iter().map(|m| m.bounds[0] + 1).max().unwrap();
    let end = measurements.iter().map(|m| m.bounds[1] + 1).min().unwrap();
    ensure!(
        end.saturating_sub(first) >= 1000,
        "insufficient common measured span for repeat comparison"
    );
    let anchor = (f64::from(first) + f64::from(end)) / 2.0;
    let pair_count = measurements.len() * (measurements.len() - 1) / 2;
    // chi-square with exactly two degrees of freedom has survival exp(-x/2).
    // Bonferroni bounds family-wise false rejection over every pair at 1%.
    let limit = -2.0 * (ALPHA / pair_count as f64).ln();
    let radius = (-2.0 * ALPHA.ln()).sqrt();
    for m in measurements {
        quality(&m.summary, &m.fit)?;
        ensure!(
            m.summary["calibration_sha256"].as_str() == Some(target),
            "repeat measurements belong to different targets"
        );
        let summary_hash = author::digest(&m.path)?;
        let targets_hash = author::digest(&m.targets)?;
        let status = read(&m.path.with_file_name("status.json"))?;
        ensure!(
            status["completed"] == true
                && status["summary_sha256"] == summary_hash
                && status["targets_sha256"] == targets_hash,
            "repeat measurement is incomplete or its receipts do not match"
        );
        ensure!(
            hashes.insert(summary_hash.clone()) && sample_hashes.insert(targets_hash.clone()),
            "copied or identical reports are not independent repeat measurements"
        );
        let data = samples(&m.targets)?;
        let samples = &data.values;
        let rms = (samples.iter().map(|(_, _, r)| r * r).sum::<f64>() / samples.len() as f64)
            .sqrt()
            .to_degrees();
        ensure!(
            (rms - m.fit.residual_rms_deg).abs() <= 1e-5,
            "reported RMS does not match the recorded residuals"
        );
        let covariance = fit::covariance(m.fit.geometry, m.fit.reference_lba, samples)?;
        validate_covariance(covariance)?;
        ensure!(
            m.fit.geometry.u > radius * covariance[0][0].sqrt()
                && m.fit.geometry.v > radius * covariance[1][1].sqrt(),
            "timing geometry is not identifiable without its priors; measure a wider span"
        );
        let max_sigma = (0..=GRID)
            .map(|i| {
                let sector = f64::from(first) + f64::from(end - first) * i as f64 / GRID as f64;
                let g = m.fit.geometry.gradient(bits(anchor), bits(sector));
                variance(g, covariance).sqrt().to_degrees()
            })
            .fold(0.0, f64::max);
        ensure!(
            max_sigma.is_finite() && radius * max_sigma < 180.0,
            "timing uncertainty leaves the angular winding ambiguous; acquire more information"
        );
        models.push(m.fit.geometry);
        rounds.push(Round {
            summary_sha256: summary_hash,
            targets_sha256: targets_hash,
            residual_rms_deg: rms,
            chi2_per_dof: samples.iter().map(|(_, s, r)| (r / s).powi(2)).sum::<f64>()
                / (samples.len() - 3) as f64,
            measured_span_sigma_deg: variance(
                m.fit
                    .geometry
                    .gradient(bits(f64::from(first)), bits(f64::from(end))),
                covariance,
            )
            .sqrt()
            .to_degrees(),
            median_phase_scatter_deg: data.scatter_deg,
            median_completion_jitter_us: data.jitter_us,
            covariance_uv: covariance,
            max_angle_sigma_deg: max_sigma,
            max_confidence_band_deg: radius * max_sigma,
        });
    }
    let mut pairs = Vec::new();
    for a in 0..rounds.len() {
        for b in a + 1..rounds.len() {
            let cov = std::array::from_fn(|i| {
                std::array::from_fn(|j| {
                    rounds[a].covariance_uv[i][j] + rounds[b].covariance_uv[i][j]
                })
            });
            let delta = [models[b].u - models[a].u, models[b].v - models[a].v];
            let score = mahalanobis(delta, cov)?;
            let mut sum = 0.0;
            let mut max: f64 = 0.0;
            for i in 0..=GRID {
                let s = f64::from(first) + f64::from(end - first) * i as f64 / GRID as f64;
                let d = (models[b].angle_between(bits(anchor), bits(s))
                    - models[a].angle_between(bits(anchor), bits(s)))
                .to_degrees();
                sum += d * d;
                max = max.max(d.abs());
            }
            pairs.push(Pair {
                first: a,
                second: b,
                mahalanobis_squared: score,
                limit,
                consistent: score <= limit,
                curve_rms_deg: (sum / (GRID + 1) as f64).sqrt(),
                curve_max_deg: max,
            });
        }
    }
    let selected = (0..rounds.len())
        .min_by(|a, b| {
            rounds[*a]
                .max_confidence_band_deg
                .total_cmp(&rounds[*b].max_confidence_band_deg)
        })
        .unwrap();
    Ok(Assessment {
        method: "data-only covariance; variance inflated for overdispersion; pairwise chi-square(2), Bonferroni 1% family-wise test; linearized model",
        confidence_level: 1.0 - ALPHA,
        encoding_range: [first, end],
        anchor_sector: anchor,
        selected,
        consistent: pairs.iter().all(|p| p.consistent),
        rounds,
        pairs,
    })
}

struct Samples {
    values: Vec<(u32, f64, f64)>,
    scatter_deg: Option<f64>,
    jitter_us: Option<f64>,
}
fn samples(path: &Path) -> Result<Samples> {
    let text = fs::read_to_string(path)?;
    let mut lines = text.lines();
    let header: Vec<_> = lines
        .next()
        .context("empty targets.csv")?
        .split(',')
        .collect();
    let column = |name: &str| {
        header
            .iter()
            .position(|s| *s == name)
            .with_context(|| format!("missing CSV column {name}; acquire a new measurement"))
    };
    let (lba, sigma, residual) = (
        column("lba")?,
        column("sigma_deg")?,
        column("residual_deg")?,
    );
    let scatter = header.iter().position(|s| *s == "std_deg");
    let period = column("period_ms")?;
    let mut result = Vec::new();
    let mut scatters = Vec::new();
    let mut jitters = Vec::new();
    let mut unique = BTreeSet::new();
    for line in lines.filter(|s| !s.is_empty()) {
        let row: Vec<_> = line.split(',').collect();
        ensure!(row.len() == header.len(), "invalid timing CSV row");
        let lba: u32 = row[lba].parse()?;
        let s: f64 = row[sigma].parse()?;
        let r: f64 = row[residual].parse()?;
        ensure!(
            unique.insert(lba) && s.is_finite() && s > 0.0 && r.is_finite() && r.abs() < 90.0,
            "invalid or duplicate timing uncertainty/residual sample"
        );
        result.push((lba, s.to_radians(), r.to_radians()));
        if let Some(column) = scatter {
            let spread: f64 = row[column].parse()?;
            let period_ms: f64 = row[period].parse()?;
            ensure!(
                spread.is_finite() && spread >= 0.0 && period_ms.is_finite() && period_ms > 0.0,
                "invalid read-completion scatter or period"
            );
            scatters.push(spread);
            // Identical host timing jitter spans more degrees at higher spindle speed.
            jitters.push(spread * period_ms * 1000.0 / 360.0);
        }
    }
    ensure!(result.len() >= 8, "too few independent targets");
    Ok(Samples {
        values: result,
        scatter_deg: (!scatters.is_empty()).then(|| fit::median(scatters)),
        jitter_us: (!jitters.is_empty()).then(|| fit::median(jitters)),
    })
}
fn bits(sector: f64) -> f64 {
    sector * f64::from(SECTOR_BITS) + f64::from(READ_TAIL_FRAMES * FRAME_BITS)
}
fn variance(g: [f64; 2], c: Covariance) -> f64 {
    (g[0] * g[0] * c[0][0] + 2.0 * g[0] * g[1] * c[0][1] + g[1] * g[1] * c[1][1]).max(0.0)
}
fn validate_covariance(c: Covariance) -> Result<()> {
    ensure!(
        c.iter().flatten().all(|v| v.is_finite())
            && c[0][0] > 0.0
            && c[1][1] > 0.0
            && (c[0][1] - c[1][0]).abs() <= 1e-8 * (c[0][0] * c[1][1]).sqrt(),
        "invalid geometry covariance"
    );
    let rho = c[0][1] / (c[0][0] * c[1][1]).sqrt();
    ensure!(rho.abs() < 1.0 - 1e-12, "singular geometry covariance");
    Ok(())
}
fn mahalanobis(d: [f64; 2], c: Covariance) -> Result<f64> {
    validate_covariance(c)?;
    let x = d[0] / c[0][0].sqrt();
    let y = d[1] / c[1][1].sqrt();
    let rho = c[0][1] / (c[0][0] * c[1][1]).sqrt();
    let score = (x * x - 2.0 * rho * x * y + y * y) / (1.0 - rho * rho);
    ensure!(score.is_finite(), "non-finite repeat consistency statistic");
    Ok(score.max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Noise(u64);
    impl Noise {
        fn uniform(&mut self) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            ((self.0 >> 11) as f64 + 0.5) / ((1u64 << 53) as f64)
        }
        fn normal(&mut self) -> f64 {
            (-2.0 * self.uniform().ln()).sqrt() * (std::f64::consts::TAU * self.uniform()).cos()
        }
    }
    #[test]
    fn chi_square_rule_has_the_declared_false_rejection_rate() {
        let mut rng = Noise(0xdea5b61);
        let trials = 40_000;
        let mut rejected = [0; 4];
        let alphas = [0.1_f64, 0.05, 0.01, 0.001];
        // Correlated, unequal parameter variances; not an identity-matrix shortcut.
        let covariance = [[4.0, 1.2], [1.2, 0.64]];
        for _ in 0..trials {
            let z = [rng.normal(), rng.normal()];
            let d = [2.0 * z[0], 0.6 * z[0] + 0.28_f64.sqrt() * z[1]];
            let score = mahalanobis(d, covariance).unwrap();
            for (i, alpha) in alphas.iter().enumerate() {
                rejected[i] += usize::from(score > -2.0 * alpha.ln());
            }
        }
        for (alpha, count) in alphas.into_iter().zip(rejected) {
            let rate = count as f64 / trials as f64;
            println!("alpha={alpha:.3} false_rejection={rate:.5} trials={trials}");
            assert!((rate - alpha).abs() < 0.004);
        }
    }
    #[test]
    fn familywise_rule_and_detectable_shifts() {
        let mut rng = Noise(9123874);
        let trials = 20_000;
        let mut family = 0;
        let mut detected = [0; 4];
        let shifts = [0.0, 2.0, 4.0, 8.0];
        for _ in 0..trials {
            let draws: Vec<_> = (0..4).map(|_| [rng.normal(), rng.normal()]).collect();
            let mut any = false;
            for i in 0..4 {
                for j in i + 1..4 {
                    let d = [draws[i][0] - draws[j][0], draws[i][1] - draws[j][1]];
                    any |= mahalanobis(d, [[2.0, 0.0], [0.0, 2.0]]).unwrap()
                        > -2.0 * (ALPHA / 6.0).ln();
                }
            }
            family += usize::from(any);
            for (i, shift) in shifts.iter().enumerate() {
                let d = [
                    (draws[0][0] - draws[1][0]) / 2.0_f64.sqrt() + shift,
                    (draws[0][1] - draws[1][1]) / 2.0_f64.sqrt(),
                ];
                detected[i] += usize::from(
                    mahalanobis(d, [[1.0, 0.0], [0.0, 1.0]]).unwrap() > -2.0 * ALPHA.ln(),
                );
            }
        }
        println!(
            "4-round family false_rejection={:.5}",
            family as f64 / trials as f64
        );
        assert!(family as f64 / trials as f64 <= 0.013);
        for (shift, count) in shifts.into_iter().zip(detected) {
            println!(
                "combined_sigma_shift={shift} detection={:.5}",
                count as f64 / trials as f64
            );
        }
        assert!(detected[0] < trials / 50);
        assert!(detected[2] > trials * 8 / 10);
        assert!(detected[3] > trials * 99 / 100);
    }
    #[test]
    fn covariance_and_shared_gradient_handle_curvature_and_weak_designs() {
        let model = crate::spiral::ChannelSpiral {
            u: 89491.5,
            v: 5.40675,
        };
        for s in [3.0, 70_000.0, 359_693.0] {
            let a = bits(180_000.0);
            let b = bits(s);
            let exact = model.gradient(a, b);
            for (i, step) in [0.01, 0.000001].into_iter().enumerate() {
                let mut lo = model;
                let mut hi = model;
                if i == 0 {
                    lo.u -= step;
                    hi.u += step;
                } else {
                    lo.v -= step;
                    hi.v += step;
                }
                let numeric = (hi.angle_between(a, b) - lo.angle_between(a, b)) / (2.0 * step);
                assert!((numeric - exact[i]).abs() < 1e-5 * exact[i].abs().max(1.0));
            }
        }
        let make = |first: u32, step: u32| {
            (0..36)
                .map(|i| (first + i * step, 0.2_f64.to_radians(), 0.1_f64.to_radians()))
                .collect::<Vec<_>>()
        };
        let broad = fit::covariance(model, 180_000, &make(2, 10_000)).unwrap();
        let narrow = fit::covariance(model, 180_000, &make(180_000, 40)).unwrap();
        assert!(narrow[0][0] > broad[0][0] * 1000.0);
        let repeated = vec![(180_000, 0.01, 0.0); 36];
        assert!(fit::covariance(model, 180_000, &repeated).is_err());
        assert!(validate_covariance([[1.0, 1.0], [1.0, 1.0]]).is_err());
    }
}
