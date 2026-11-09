// SPDX-License-Identifier: GPL-3.0-or-later
//! Periodogram and robust phase extraction adapted from disc-probe analysis.py.
use std::f64::consts::TAU;

use super::fit::{median, wrap};

const HARMONIC_ACCEPT: f64 = 0.9;

pub(super) struct Period {
    pub seconds: f64,
    pub origin: f64,
    pub coherence: f64,
    residuals: Vec<(f64, f64)>,
}

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

fn circular(angles: impl Iterator<Item = f64>) -> (f64, f64) {
    let (sin, cos, count) = angles
        .map(f64::sin_cos)
        .fold((0.0, 0.0, 0), |(y, x, n), (s, c)| (y + s, x + c, n + 1));
    (sin.atan2(cos), (sin.hypot(cos) / f64::from(count)).min(1.0))
}

fn peak(times: &[f64], lo: f64, hi: f64, step: f64) -> (f64, f64) {
    let lo = lo.clamp(2.5, 1.0 / 0.003);
    let hi = hi.clamp(2.5, 1.0 / 0.003);
    let count = (((hi - lo) / step).ceil() as u32).clamp(1, 200_000);
    let mut best = (lo, -1.0);
    for i in 0..=count {
        let f = lo + (hi - lo) * f64::from(i) / f64::from(count);
        let coherence = circular(times.iter().map(|&t| t * f * TAU)).1;
        if coherence > best.1 {
            best = (f, coherence);
        }
    }
    best
}

fn refine(times: &[f64], mut f: f64, mut step: f64) -> (f64, f64) {
    let mut best = (f, 0.0);
    for _ in 0..3 {
        best = peak(times, f - step, f + step, step / 20.0);
        f = best.0;
        step *= 0.1;
    }
    best
}

pub(super) fn estimate(times: &[f64], prior: Option<f64>) -> Option<Period> {
    if times.len() < 8 {
        return None;
    }
    let base = times[0];
    let times: Vec<_> = times.iter().map(|&t| t - base).collect();
    let span = *times.last()?;
    if span <= 0.0 {
        return None;
    }
    let step = 1.0 / (8.0 * span);
    let mut candidates = Vec::new();
    if let Some(period) = prior {
        for k in 1..=3 {
            let center = f64::from(k) / period;
            let (f, _) = peak(&times, 0.98 * center, 1.02 * center, step);
            candidates.push(refine(&times, f, step));
        }
    } else {
        let (f, _) = peak(&times, 2.5, 1.0 / 0.003, step);
        let (f, best) = refine(&times, f, step);
        candidates.push((f, best));
        candidates.extend(
            (2..=12)
                .map(|k| f / f64::from(k))
                .take_while(|&candidate| candidate >= 2.5)
                .map(|candidate| refine(&times, candidate, step)),
        );
    }
    // Multiples of the true frequency can score just as well as the fundamental.
    let best = candidates
        .iter()
        .map(|&(_, score)| score)
        .reduce(f64::max)?;
    let (frequency, coherence) = candidates
        .into_iter()
        .filter(|&(_, score)| score >= HARMONIC_ACCEPT * best)
        .min_by(|&(a, _), &(b, _)| a.total_cmp(&b))?;
    let mut period = 1.0 / frequency;
    let mut origin = 0.0;
    let mut kept = times.clone();
    for _ in 0..3 {
        let revs: Vec<_> = kept
            .iter()
            .map(|&t| ((t - origin) / period).round())
            .collect();
        let t_center = mean(&kept);
        let rev_center = mean(&revs);
        let variance = revs.iter().map(|r| (r - rev_center).powi(2)).sum::<f64>();
        if variance <= 0.0 {
            return None;
        }
        period = revs
            .iter()
            .zip(&kept)
            .map(|(r, t)| (r - rev_center) * (t - t_center))
            .sum::<f64>()
            / variance;
        if !(0.003..=0.4).contains(&period) {
            return None;
        }
        origin = t_center - period * rev_center;
        let residual = |t: f64| (t - origin) - ((t - origin) / period).round() * period;
        let center = median(kept.iter().map(|&t| residual(t)).collect());
        let limit = (7.413 * median(kept.iter().map(|&t| (residual(t) - center).abs()).collect()))
            .max(20e-6);
        kept = times
            .iter()
            .copied()
            .filter(|&t| (residual(t) - center).abs() <= limit)
            .collect();
        if kept.len() < 8 {
            return None;
        }
    }
    let residuals = kept
        .into_iter()
        .map(|time| {
            let elapsed = time - origin;
            let residual = elapsed - (elapsed / period).round() * period;
            (base + time, residual)
        })
        .collect();
    Some(Period {
        seconds: period,
        origin: base + origin,
        coherence,
        residuals,
    })
}

pub(super) fn phase(reference: &Period, times: &[f64]) -> Option<(f64, f64, usize)> {
    let angles: Vec<_> = times
        .iter()
        .map(|&t| {
            // The residual at the neighboring reference reads tracks local spindle drift.
            let samples = &reference.residuals;
            let next = samples.partition_point(|&(at, _)| at < t);
            let drift = if next == 0 {
                samples[0].1
            } else if next == samples.len() {
                samples[next - 1].1
            } else {
                let (a, ra) = samples[next - 1];
                let (b, rb) = samples[next];
                ra + (rb - ra) * (t - a) / (b - a)
            };
            (t - reference.origin - drift) / reference.seconds * TAU
        })
        .collect();
    if angles.len() < 8 {
        return None;
    }
    let center = circular(angles.iter().copied()).0;
    let delta = |a| wrap(a - center).abs();
    let limit = (7.413 * median(angles.iter().map(|&a| delta(a)).collect())).max(1f64.to_radians());
    let kept: Vec<_> = angles.into_iter().filter(|&a| delta(a) <= limit).collect();
    if kept.len() < 8 {
        return None;
    }
    let (center, resultant) = circular(kept.iter().copied());
    let spread = (-2.0 * resultant.ln()).sqrt();
    (spread <= 30f64.to_radians()).then_some((center, spread, kept.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_residuals_remove_local_spindle_drift() {
        let reference = Period {
            seconds: 0.025,
            origin: 0.0,
            coherence: 1.0,
            residuals: vec![(0.0, 0.0), (0.201, 0.001), (0.4, 0.0)],
        };
        let times: Vec<_> = (0..16)
            .map(|i| {
                let nominal = (f64::from(i) + 0.25) * reference.seconds;
                // Solve t = nominal + interpolated residual(t) on each side.
                if nominal < 0.2 {
                    nominal * 0.201 / 0.2
                } else {
                    (nominal * 0.199 + 0.0004) / 0.2
                }
            })
            .collect();
        let (angle, spread, count) = phase(&reference, &times).unwrap();
        assert!((angle - TAU / 4.0).abs() < 1e-10);
        assert!(spread < 1e-6);
        assert_eq!(count, times.len());
    }
}
