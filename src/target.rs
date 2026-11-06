//! Pure radial bands and angular pulse patterns, independent of photo and file workflows.
use std::ops::Range;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use crate::{codec::RAW, layout::Plan, project::Disc, spiral::Spiral};

/// Known pulse trains, specified in absolute CD sectors. Their observed
/// winding is the measurement, including when none of the pulses line up.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhaseTarget {
    pub controls: Vec<[u32; 2]>,
    pub bands: Vec<PhaseBand>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhaseBand {
    pub start: u32,
    pub check: u32,
    pub end: u32,
    pub velocity: f64,
}

impl PhaseTarget {
    pub(crate) const CHECK_TURNS: f64 = 0.24;
    pub(crate) const DUTY: f64 = 0.22;

    pub(crate) fn validate(&self, drawing: Range<u32>) -> Result<()> {
        ensure!(
            (2..=24).contains(&self.controls.len()) && (1..=24).contains(&self.bands.len()),
            "expected 2–24 contrast rings and 1–24 pulse bands"
        );
        let mut ranges = Vec::new();
        for &[start, end] in &self.controls {
            ensure!(
                end > start && end - start >= 64,
                "contrast ring is too short"
            );
            ranges.push((start, end));
        }
        for band in &self.bands {
            ensure!(
                band.start < band.check
                    && band.check < band.end
                    && band.check - band.start >= 400
                    && band.end - band.check >= 180
                    && (1000.0..=1500.0).contains(&band.velocity),
                "invalid phase band"
            );
            ranges.push((band.start, band.end));
        }
        ranges.sort_unstable();
        ensure!(
            ranges[0].0 >= drawing.start
                && ranges.last().unwrap().1 <= drawing.end
                && ranges.windows(2).all(|pair| pair[0].1 <= pair[1].0),
            "phase target ranges must be disjoint and inside the drawing extent"
        );
        Ok(())
    }

    pub(crate) fn palette(&self, disc: &Disc, sector: f64) -> u8 {
        let mut dark = self
            .controls
            .iter()
            .any(|&[lo, hi]| (f64::from(lo)..f64::from(hi)).contains(&sector));
        if let Some(band) = self
            .bands
            .iter()
            .find(|b| (f64::from(b.start)..f64::from(b.end)).contains(&sector))
        {
            let (inner, outer, count) = disc.geometry();
            let span = (outer * outer - inner * inner) / f64::from(count);
            let radius = (inner * inner + span * sector).sqrt();
            let first = (inner * inner + span * f64::from(band.start)).sqrt();
            let turns = band.velocity * (sector - f64::from(band.start))
                / (75.0 * std::f64::consts::PI * (radius + first));
            let offset = if sector < f64::from(band.check) {
                0.0
            } else {
                Self::CHECK_TURNS
            };
            dark = (turns - offset).rem_euclid(1.0) < Self::DUTY;
        }
        if dark { 0x0d } else { 0xaa }
    }

    pub(crate) fn preview(&self, disc: &Disc, radius: f64, angle: f64) -> u8 {
        let spiral = Spiral::new(disc);
        let sector = spiral.sector(radius);
        let nominal = spiral.angle(radius);
        let pi = std::f64::consts::PI;
        let delta = (angle - nominal + pi).rem_euclid(2.0 * pi) - pi;
        let closest = sector + delta * radius * 75.0 / disc.velocity;
        if self.palette(disc, closest) == 0x0d {
            0
        } else {
            255
        }
    }
}

#[cfg(test)]
mod phase_tests {
    use super::*;

    #[test]
    fn phase_target_separates_controls_training_and_validation() {
        let mut target = PhaseTarget {
            controls: vec![[132000, 132600], [147000, 147600]],
            bands: vec![PhaseBand {
                start: 135000,
                check: 139000,
                end: 142000,
                velocity: 1200.0,
            }],
        };
        target.validate(130000..150000).unwrap();
        let disc = Disc::default();
        assert_eq!(target.palette(&disc, 132250.0), 0x0d);
        assert_eq!(target.palette(&disc, 133500.0), 0xaa);
        let (inner, outer, count) = disc.geometry();
        let scale = (outer.powi(2) - inner.powi(2)) / f64::from(count);
        let band = &target.bands[0];
        let radius = (inner.powi(2) + scale * f64::from(band.start)).sqrt();
        let sector_at_turn = |turns: f64| {
            let radius = radius + turns * 75.0 * std::f64::consts::PI * scale / band.velocity;
            (radius.powi(2) - inner.powi(2)) / scale
        };
        assert_eq!(target.palette(&disc, sector_at_turn(1.10)), 0x0d);
        assert_eq!(target.palette(&disc, sector_at_turn(300.10)), 0xaa);
        assert_eq!(target.palette(&disc, sector_at_turn(300.34)), 0x0d);
        target.controls.push([138000, 139000]);
        assert!(target.validate(130000..150000).is_err());
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Target {
    pub(crate) format: String,
    pub(crate) bands: [[u32; 2]; 3],
}

impl Target {
    pub fn new(plan: &Plan) -> Self {
        Self::for_range(plan.estimated_art_start_sector..plan.estimated_art_end_sector)
    }

    pub(crate) fn for_range(range: Range<u32>) -> Self {
        // Keep clear space before, between and after the marks. Integer LBAs
        // bind each boundary to the bytes actually sent to the drive.
        let edge = |percent| range.start + (range.end - range.start) * percent / 100;
        Self {
            format: "discimage-target-v1".into(),
            bands: [
                [edge(5), edge(17)],
                [edge(32), edge(51)],
                [edge(69), edge(94)],
            ],
        }
    }

    pub(crate) fn edges(&self) -> [u32; 6] {
        let [[a, b], [c, d], [e, f]] = self.bands;
        [a, b, c, d, e, f]
    }

    pub fn dark(&self, sector: u32) -> bool {
        self.bands
            .iter()
            .any(|&[start, end]| (start..end).contains(&sector))
    }

    pub fn palette(&self, physical: i64) -> u8 {
        if self.dark((physical.max(0) / RAW as i64) as u32) {
            0x0d
        } else {
            0xaa
        }
    }
}
