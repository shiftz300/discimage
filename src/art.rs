use crate::{
    codec::{self, RAW},
    layout::Plan,
    project::{Artwork, Disc, Fit, Mode, Tone},
    target::{PhaseTarget, Target},
};
use anyhow::{Context, Result, ensure};
use image::{GrayImage, ImageFormat, ImageReader, Rgba, RgbaImage, imageops};

/// Select the drawing once. Encoding and design previews use the same source;
/// physical byte dithering and screen pixel dithering remain distinct.
pub(crate) enum Drawing<'a> {
    Blank,
    Image(Sampler, &'a Disc),
    Radial(Target),
    Phase(&'a PhaseTarget, &'a Disc),
}

impl<'a> Drawing<'a> {
    pub(crate) fn new(plan: &'a Plan, artwork: Option<&Artwork>) -> Result<Self> {
        let disc = &plan.disc;
        ensure!(
            !(disc.calibration_target || disc.phase_target.is_some()) || artwork.is_none(),
            "a calibration target cannot also contain artwork"
        );
        ensure!(
            !disc.calibration_target || disc.phase_target.is_none(),
            "choose one calibration target type"
        );
        Ok(if disc.calibration_target {
            Self::Radial(Target::new(plan))
        } else if let Some(target) = &disc.phase_target {
            Self::Phase(target, disc)
        } else if let Some(artwork) = artwork {
            Self::Image(Sampler::new(artwork, plan)?, disc)
        } else {
            Self::Blank
        })
    }

    pub(crate) fn active(&self) -> bool {
        !matches!(self, Self::Blank)
    }

    pub(crate) fn palette(&self, physical: i64) -> u8 {
        match self {
            Self::Blank => 0xaa,
            Self::Image(source, disc) => {
                let (x, y) = codec::point(disc, physical);
                source.palette(x, y, physical)
            }
            Self::Radial(target) => target.palette(physical),
            Self::Phase(target, disc) => target.palette(disc, physical as f64 / RAW as f64),
        }
    }

    pub(crate) fn design(&self, x: f64, y: f64, sector: u32, pixel: (u32, u32)) -> Option<u8> {
        Some(match self {
            Self::Blank => return None,
            Self::Image(source, _) => source.design(x, y, pixel.0, pixel.1),
            Self::Radial(target) => {
                if target.dark(sector) {
                    0
                } else {
                    255
                }
            }
            Self::Phase(target, disc) => target.preview(disc, x.hypot(y), y.atan2(x)),
        })
    }
}

/// A fixed source grid shared by every physical sample in an authoring job.
pub struct Sampler {
    image: GrayImage,
    settings: Artwork,
    radius: f64,
    inner: f64,
    outer: f64,
    rotation: (f64, f64),
}

impl Sampler {
    pub fn new(settings: &Artwork, plan: &Plan) -> Result<Self> {
        let source = load(settings)?;
        let mut gray = settings.clone();
        gray.tone = Tone::Gray;
        let image = GrayImage::from_fn(source.width(), source.height(), |x, y| {
            image::Luma([tone(source.get_pixel(x, y).0, &gray, x, y)])
        });
        Ok(Self {
            image,
            settings: settings.clone(),
            radius: plan.recording_outer_radius(),
            inner: plan.artwork_inner_radius_mm,
            outer: plan.artwork_outer_radius_mm,
            rotation: settings.rotation.to_radians().sin_cos(),
        })
    }

    pub fn sample(&self, x: f64, y: f64) -> u8 {
        let (sin, cos) = self.rotation;
        let (u, v) = match self.settings.mode {
            Mode::Disc => (
                (cos * x + sin * y) / (2.0 * self.radius) + 0.5,
                (-sin * x + cos * y) / (2.0 * self.radius) + 0.5,
            ),
            Mode::Strip => {
                let radius = x.hypot(y);
                if radius < self.inner || radius > self.outer {
                    return 255;
                }
                // Left edge starts at 12 o'clock; top edge is the outer radius.
                let angle =
                    y.atan2(x) + std::f64::consts::FRAC_PI_2 - self.settings.rotation.to_radians();
                (
                    angle.rem_euclid(std::f64::consts::TAU) / std::f64::consts::TAU,
                    ((self.outer - radius) / (self.outer - self.inner))
                        .clamp(0.0, 1.0 - f64::EPSILON),
                )
            }
        };
        let px = (u * f64::from(self.image.width())).floor() as i32;
        let py = (v * f64::from(self.image.height())).floor() as i32;
        if (0..self.image.width() as i32).contains(&px)
            && (0..self.image.height() as i32).contains(&py)
        {
            self.image.get_pixel(px as u32, py as u32)[0]
        } else {
            255
        }
    }

    pub fn design(&self, x: f64, y: f64, px: u32, py: u32) -> u8 {
        levels(f64::from(self.sample(x, y)), &self.settings, px, py)
    }

    /// Physical dithering is phased by absolute logical byte, never output pixels.
    pub fn palette(&self, x: f64, y: f64, byte: i64) -> u8 {
        let gray = f64::from(self.sample(x, y));
        let white = match self.settings.tone {
            Tone::Threshold => gray >= f64::from(self.settings.threshold),
            Tone::Ordered => {
                const PHASE: [u8; 16] = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];
                gray + 128.0 - f64::from(self.settings.threshold)
                    >= (f64::from(PHASE[byte.rem_euclid(16) as usize]) + 0.5) * 16.0
            }
            Tone::Gray => {
                let mut hash = (byte as u64).wrapping_add(0x9e3779b97f4a7c15);
                hash = (hash ^ (hash >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
                hash = (hash ^ (hash >> 27)).wrapping_mul(0x94d049bb133111eb);
                hash ^= hash >> 31;
                gray / 255.0 > (hash >> 11) as f64 / ((1u64 << 53) as f64)
            }
        };
        if white { 0xaa } else { 0x0d }
    }
}

pub(crate) fn load(art: &Artwork) -> Result<RgbaImage> {
    art.validate()?;
    let mut reader = ImageReader::open(&art.path)?.with_guessed_format()?;
    ensure!(
        matches!(reader.format(), Some(ImageFormat::Png | ImageFormat::Jpeg)),
        "artwork must be PNG or JPEG"
    );
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let source = reader.decode().context("cannot decode artwork")?;
    if art.mode == Mode::Strip {
        // Preserve the original pixel grid and edges, including in job snapshots.
        // Decoder limits already bound the input to 8192 pixels on each side.
        let fitted = source.to_rgba8();
        let mut canvas = RgbaImage::from_pixel(fitted.width(), fitted.height(), Rgba([255; 4]));
        imageops::overlay(&mut canvas, &fitted, 0, 0);
        return Ok(canvas);
    }
    let size = source.width().max(source.height()).clamp(1600, 4096);
    let fitted = match art.fit {
        Fit::Contain => source.resize(size, size, imageops::FilterType::Lanczos3),
        Fit::Cover => source.resize_to_fill(size, size, imageops::FilterType::Lanczos3),
    }
    .to_rgba8();
    let mut canvas = RgbaImage::from_pixel(size, size, Rgba([255; 4]));
    imageops::overlay(
        &mut canvas,
        &fitted,
        i64::from((size - fitted.width()) / 2),
        i64::from((size - fitted.height()) / 2),
    );
    Ok(canvas)
}

pub(crate) fn tone(pixel: [u8; 4], art: &Artwork, x: u32, y: u32) -> u8 {
    let gray = (0.2126 * f64::from(pixel[0])
        + 0.7152 * f64::from(pixel[1])
        + 0.0722 * f64::from(pixel[2]))
        / 255.0;
    let mut value =
        (gray.powf(1.0 / art.gamma) * 255.0 + f64::from(art.brightness)).clamp(0.0, 255.0);
    if art.invert {
        value = 255.0 - value;
    }
    levels(value, art, x, y)
}

fn levels(value: f64, art: &Artwork, x: u32, y: u32) -> u8 {
    match art.tone {
        Tone::Gray => value.round() as u8,
        Tone::Threshold => {
            if value >= f64::from(art.threshold) {
                255
            } else {
                0
            }
        }
        Tone::Ordered => {
            const BAYER: [[u8; 4]; 4] =
                [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
            let threshold = (f64::from(BAYER[y as usize % 4][x as usize % 4]) + 0.5) * 16.0;
            if value + 128.0 - f64::from(art.threshold) >= threshold {
                255
            } else {
                0
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrow_ink_edges_survive_full_resolution_strip_loading_and_snapshots() {
        let path = std::env::temp_dir().join(format!("discimage-edges-{}.png", std::process::id()));
        let source = RgbaImage::from_fn(8192, 8, |x, _| {
            if x % 5 == 0 {
                Rgba([0, 0, 0, 255])
            } else {
                Rgba([255; 4])
            }
        });
        source.save(&path).unwrap();
        let art = Artwork {
            path: path.clone(),
            mode: Mode::Strip,
            ..Artwork::default()
        };
        let first = load(&art).unwrap();
        assert_eq!(first, source);
        first.save(&path).unwrap();
        assert_eq!(load(&art).unwrap(), source);
        std::fs::remove_file(path).unwrap();
    }
}
