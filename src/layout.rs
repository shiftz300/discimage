use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::{
    audio::{self, AudioTrack},
    project::{Disc, Project},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub title: String,
    pub model: String,
    pub burn_ready: bool,
    pub disc: Disc,
    pub tracks: Vec<AudioTrack>,
    pub music_sectors: u32,
    pub gap_sectors: u32,
    pub estimated_art_start_sector: u32,
    pub estimated_art_end_sector: u32,
    pub artwork_sectors: u32,
    pub artwork_inner_radius_mm: f64,
    pub artwork_outer_radius_mm: f64,
}

impl Plan {
    /// One preparation path for templates, previews and authored sessions.
    pub fn from_project(project: &Project, session_start: Option<u32>) -> Result<Self> {
        ensure!(
            !(project.disc.calibration_target || project.disc.phase_target.is_some())
                || project.artwork.is_none(),
            "a calibration target cannot also contain artwork"
        );
        ensure!(
            (1..=98).contains(&project.tracks.len()),
            "expected 1–98 music tracks"
        );
        let tracks = project
            .tracks
            .iter()
            .map(|t| audio::inspect(&t.path))
            .collect::<Result<Vec<_>>>()?;
        let mut disc = project.disc.clone();
        if let Some(start) = session_start {
            let (music, gaps) = music_budget(&tracks, disc.intertrack_gap_sectors);
            disc.session_reserve_sectors = u32::try_from(
                u64::from(start)
                    .checked_sub(music + gaps)
                    .context("session start precedes music end")?,
            )?;
        }
        Self::new(project.title.clone(), disc, tracks)
    }

    pub fn new(title: String, mut disc: Disc, tracks: Vec<AudioTrack>) -> Result<Self> {
        ensure!(
            (1..=98).contains(&tracks.len()),
            "expected 1–98 music tracks"
        );
        ensure!(
            disc.capacity_sectors > 0 && disc.capacity_sectors <= 360_000,
            "capacity must be 1–360000 sectors; overburn is not supported"
        );
        if let Some(profile) = &disc.calibration {
            profile.validate()?;
            disc.capacity_sectors = disc.capacity_sectors.min(profile.reference_sectors);
        }
        ensure!(
            tracks.iter().all(|t| t.sectors >= 300
                && t.sectors <= disc.capacity_sectors
                && t.sectors == t.sample_frames.div_ceil(audio::SAMPLES_PER_SECTOR)
                && t.padding_sample_frames
                    == t.sectors * audio::SAMPLES_PER_SECTOR - t.sample_frames),
            "invalid audio sector length or padding"
        );
        ensure!(
            disc.inner_radius_mm.is_finite()
                && disc.outer_radius_mm.is_finite()
                && disc.inner_radius_mm > 0.0
                && disc.outer_radius_mm > disc.inner_radius_mm
                && disc.outer_radius_mm <= 60.0,
            "invalid disc radii"
        );
        ensure!(
            disc.velocity.is_finite()
                && (1000.0..=1500.0).contains(&disc.velocity)
                && disc.angle.is_finite(),
            "invalid geometry: velocity must be 1000–1500 mm/s and angle finite"
        );
        ensure!(
            disc.session_reserve_sectors >= 11_400,
            "reserve at least 11400 sectors for the session transition (estimate only)"
        );
        ensure!(
            disc.data_reserve_sectors >= 300,
            "reserve at least 300 sectors for filesystem/content (estimate only)"
        );
        let (music, gaps) = music_budget(&tracks, disc.intertrack_gap_sectors);
        let start = music
            + gaps
            + u64::from(disc.session_reserve_sectors)
            + u64::from(disc.data_reserve_sectors);
        let tail = u64::from(disc.end_margin_sectors);
        let capacity = u64::from(disc.capacity_sectors);
        ensure!(
            start + tail < capacity,
            "no artwork space: music + gaps + session/data reserves + margin = {} sectors, capacity = {}",
            start + tail,
            capacity
        );
        let end = capacity - tail;
        if let Some(timing) = &disc.timing {
            timing.validate()?;
            ensure!(
                start >= u64::from(timing.first_sector) + 2 && end <= u64::from(timing.end_sector),
                "artwork exceeds the measured timing range (including CIRC guard); adjust reserves or measure a wider range"
            );
        }
        if let Some(target) = &disc.phase_target {
            ensure!(
                !disc.calibration_target,
                "choose one calibration target type"
            );
            target.validate(start as u32..end as u32)?;
        }
        let inner = radius_at_sector(&disc, start as u32);
        let outer = radius_at_sector(&disc, end as u32);
        ensure!(
            !disc.calibration_target || outer - inner >= 4.0,
            "the calibration target needs at least 4 mm of radial drawing space"
        );
        let model = if disc.timing.is_some() {
            "constant-pitch-timing-candidate-v1"
        } else if disc.calibration.is_some() {
            "constant-pitch-radial-fit-v1"
        } else {
            "constant-pitch-area-estimate-v1"
        };
        Ok(Self {
            title,
            model: model.into(),
            burn_ready: false,
            disc,
            tracks,
            music_sectors: music as u32,
            gap_sectors: gaps as u32,
            estimated_art_start_sector: start as u32,
            estimated_art_end_sector: end as u32,
            artwork_sectors: (end - start) as u32,
            artwork_inner_radius_mm: inner,
            artwork_outer_radius_mm: outer,
        })
    }

    pub fn recording_outer_radius(&self) -> f64 {
        radius_at_sector(&self.disc, self.disc.capacity_sectors)
    }

    pub fn session_start(&self) -> u32 {
        self.estimated_art_start_sector - self.disc.data_reserve_sectors
    }

    pub fn data_sectors(&self) -> u32 {
        self.estimated_art_end_sector - self.session_start()
    }

    pub(crate) fn music_ranges(&self) -> impl Iterator<Item = std::ops::Range<u32>> + '_ {
        let gap = self.disc.intertrack_gap_sectors;
        audio::track_ranges(self.tracks.iter().map(move |track| (track.sectors, gap)))
    }

    pub fn geometry_label(&self) -> &'static str {
        if self.disc.timing.is_some() {
            "Timing candidate · verify radial scale and physical alignment"
        } else if self.disc.calibration.is_some() {
            "Radially calibrated · constant-pitch model"
        } else {
            "Uncalibrated geometry estimate"
        }
    }
}

fn music_budget(tracks: &[AudioTrack], gap: u32) -> (u64, u64) {
    let music = tracks.iter().map(|track| u64::from(track.sectors)).sum();
    let gaps = (tracks.len() as u64 - 1) * u64::from(gap);
    (music, gaps)
}

/// A constant-pitch spiral sweeps area approximately linearly with sector count.
pub fn radius_at_sector(disc: &Disc, sector: u32) -> f64 {
    crate::spiral::Spiral::new(disc).radius(f64::from(sector.min(disc.capacity_sectors)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(sectors: u32) -> AudioTrack {
        AudioTrack {
            path: "test.wav".into(),
            sample_frames: sectors * 588,
            sectors,
            padding_sample_frames: 0,
            data_offset: 44,
        }
    }

    #[test]
    fn half_an_hour_reserves_transition_and_filesystem() {
        let p = Plan::new("EP".into(), Disc::default(), vec![track(135_000)]).unwrap();
        assert_eq!(p.estimated_art_start_sector, 156_400);
        assert_eq!(p.artwork_sectors, 203_450);
        assert!((p.artwork_inner_radius_mm - 42.6).abs() < 0.1);
        assert!(!p.burn_ready);
    }

    #[test]
    fn boundaries_follow_area_instead_of_linear_radius() {
        let d = Disc::default();
        assert_eq!(radius_at_sector(&d, 0), 25.0);
        assert_eq!(radius_at_sector(&d, d.capacity_sectors), 58.0);
        assert!(radius_at_sector(&d, d.capacity_sectors / 2) > (25.0 + 58.0) / 2.0);
    }

    #[test]
    fn counts_only_gaps_between_tracks() {
        let d = Disc {
            intertrack_gap_sectors: 150,
            ..Disc::default()
        };
        let p = Plan::new("EP".into(), d, vec![track(300), track(300)]).unwrap();
        assert_eq!(p.gap_sectors, 150);
        assert_eq!(p.estimated_art_start_sector, 22_150);
    }

    #[test]
    fn rejects_full_disc_and_arithmetic_extremes() {
        assert!(Plan::new("full".into(), Disc::default(), vec![track(359_000)]).is_err());
        let d = Disc {
            data_reserve_sectors: u32::MAX,
            end_margin_sectors: u32::MAX,
            ..Disc::default()
        };
        assert!(Plan::new("overflow".into(), d, vec![track(300)]).is_err());
    }

    #[test]
    fn rejects_nonfinite_geometry() {
        let d = Disc {
            inner_radius_mm: f64::NAN,
            ..Disc::default()
        };
        assert!(Plan::new("bad".into(), d, vec![track(300)]).is_err());
    }
}
