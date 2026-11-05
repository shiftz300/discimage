use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub title: String,
    pub tracks: Vec<Track>,
    #[serde(default)]
    pub disc: Disc,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artwork: Option<Artwork>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<PathBuf>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Track {
    pub path: PathBuf,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Artwork {
    pub path: PathBuf,
    pub mode: Mode,
    pub rotation: f64,
    pub fit: Fit,
    pub tone: Tone,
    pub threshold: u8,
    pub gamma: f64,
    pub brightness: i16,
    pub invert: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Disc,
    Strip,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Fit {
    #[default]
    Contain,
    Cover,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tone {
    #[default]
    Gray,
    Threshold,
    Ordered,
}

impl Default for Artwork {
    fn default() -> Self {
        Self {
            path: PathBuf::new(),
            mode: Mode::Disc,
            rotation: 0.0,
            fit: Fit::Contain,
            tone: Tone::Gray,
            threshold: 128,
            gamma: 1.0,
            brightness: 0,
            invert: false,
        }
    }
}

impl Artwork {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.path.as_os_str().is_empty(),
            "artwork.path must not be empty"
        );
        ensure!(self.rotation.is_finite(), "artwork.rotation must be finite");
        ensure!(
            self.gamma.is_finite() && (0.1..=4.0).contains(&self.gamma),
            "artwork.gamma must be between 0.1 and 4"
        );
        ensure!(
            (-255..=255).contains(&self.brightness),
            "artwork.brightness must be -255–255"
        );
        Ok(())
    }
}

/// Planning inputs, never a replacement for the drive's reported capacity/NWA.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Disc {
    pub capacity_sectors: u32,
    pub inner_radius_mm: f64,
    pub outer_radius_mm: f64,
    pub intertrack_gap_sectors: u32,
    pub session_reserve_sectors: u32,
    pub data_reserve_sectors: u32,
    pub end_margin_sectors: u32,
    pub velocity: f64,
    pub angle: f64,
}

impl Default for Disc {
    fn default() -> Self {
        Self {
            capacity_sectors: 360_000,
            inner_radius_mm: 25.0,
            outer_radius_mm: 58.0,
            intertrack_gap_sectors: 0,
            session_reserve_sectors: 11_400,
            data_reserve_sectors: 10_000,
            end_margin_sectors: 150,
            velocity: 1200.0,
            angle: 0.0,
        }
    }
}

impl Project {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read project {}", path.display()))?;
        let mut project: Self = toml::from_str(&text).context("invalid project TOML")?;
        ensure!(!project.title.trim().is_empty(), "title must not be empty");
        // 99 total tracks, reserving one for the data session.
        ensure!(
            (1..=98).contains(&project.tracks.len()),
            "expected 1–98 music tracks"
        );
        let base = path.parent().unwrap_or(Path::new("."));
        if let Some(files) = &mut project.files {
            *files = base.join(&files);
        }
        for track in &mut project.tracks {
            track.path = base.join(&track.path);
        }
        if let Some(artwork) = &mut project.artwork {
            artwork.validate()?;
            artwork.path = base.join(&artwork.path);
        }
        Ok(project)
    }
}
