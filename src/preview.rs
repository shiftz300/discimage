use std::{io::Cursor, path::Path};

use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};

use crate::{
    art::Drawing,
    layout::{Plan, radius_at_sector},
    project::Artwork,
    spiral::Spiral,
};

/// Read the authored ISO bytes, undo the per-sector scrambler, then project
/// payload samples using the CIRC position model. Unsampled pixels stay transparent.
/// Parity/EFM brightness and the drive's optical response are not simulated.
pub fn readback(
    directory: &Path,
    manifest: &crate::author::Manifest,
    path: &Path,
    size: u32,
    samples: u32,
) -> Result<()> {
    use std::io::{Read, Seek, SeekFrom};
    ensure!(
        (256..=4096).contains(&size) && (1..=2048).contains(&samples),
        "readback size must be 256–4096, samples per sector 1–2048"
    );
    let plan = &manifest.plan;
    let codec = crate::codec::Codec::default();
    let mut input = std::io::BufReader::with_capacity(
        1024 * 1024,
        std::fs::File::open(directory.join("session2.iso"))?,
    );
    ensure!(
        input.get_ref().metadata()?.len() == u64::from(manifest.data_sectors) * 2048,
        "incorrect ISO length"
    );
    input.seek(SeekFrom::Start(
        u64::from(plan.estimated_art_start_sector - manifest.session_start) * 2048,
    ))?;
    let mut sums = vec![0u64; (size * size) as usize];
    let mut counts = vec![0u32; (size * size) as usize];
    let scale = f64::from(size) / 126.0;
    for lba in plan.estimated_art_start_sector..plan.estimated_art_end_sector {
        let mut sector = [0; 2048];
        input.read_exact(&mut sector)?;
        for sample in 0..samples {
            let offset = (sample * 2048 / samples) as usize;
            let physical = codec.physical(lba, offset);
            if physical < i64::from(plan.estimated_art_start_sector) * 2352 {
                continue;
            }
            let (x, y) = crate::codec::point(&plan.disc, physical);
            let px = (f64::from(size) / 2.0 + x * scale).floor() as i32;
            let py = (f64::from(size) / 2.0 + y * scale).floor() as i32;
            if !(0..size as i32).contains(&px) || !(0..size as i32).contains(&py) {
                continue;
            }
            let palette = sector[offset] ^ codec.scramble[offset + 4];
            ensure!(
                matches!(palette, 0x0d | 0xaa),
                "unexpected drawing palette at LBA {lba}"
            );
            let index = py as usize * size as usize + px as usize;
            sums[index] += if palette == 0xaa { 255 } else { 0 };
            counts[index] += 1;
        }
    }
    let mut image = render(plan, None, size)?;
    let inner = radius_at_sector(&plan.disc, 0);
    let outer = plan.recording_outer_radius();
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let radius = (f64::from(x) + 0.5 - f64::from(size) / 2.0)
            .hypot(f64::from(y) + 0.5 - f64::from(size) / 2.0)
            / scale;
        if radius >= plan.artwork_inner_radius_mm && radius <= plan.artwork_outer_radius_mm {
            let i = (y * size + x) as usize;
            if counts[i] > 0 {
                let v = (sums[i] / u64::from(counts[i])) as u8;
                *pixel = Rgba([v, v, v, 255]);
            } else {
                *pixel = Rgba([0, 0, 0, 0]);
            }
        } else if radius >= inner && radius < outer {
            *pixel = Rgba([49, 57, 69, 255]);
        }
    }
    png(&image, path)
}

/// Orange intensity is grayscale design/readback difference; missing samples are transparent.
pub fn difference(design: &Path, readback: &Path, path: &Path) -> Result<()> {
    let a = image::open(design)?.to_rgba8();
    let b = image::open(readback)?.to_rgba8();
    ensure!(
        a.dimensions() == b.dimensions(),
        "preview dimensions differ"
    );
    let result = RgbaImage::from_fn(a.width(), a.height(), |x, y| {
        let p = a.get_pixel(x, y);
        let q = b.get_pixel(x, y);
        let d = p[0].abs_diff(q[0]);
        Rgba([d, d / 3, 0, if p[3] == 0 || q[3] == 0 { 0 } else { 255 }])
    });
    png(&result, path)
}

pub(crate) fn png(image: &RgbaImage, path: &Path) -> Result<()> {
    use std::io::Write;
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    let mut output = std::io::BufWriter::new(file);
    DynamicImage::ImageRgba8(image.clone()).write_to(&mut output, ImageFormat::Png)?;
    output.flush()?;
    Ok(())
}

/// Predict the spatial placement of a design encoded by `source` when its
/// sectors lie on `measured`. Both models share an angle anchor. This compares
/// geometric mappings only; it is not a simulated readback or optical response.
pub(crate) fn projected(
    source: &Plan,
    artwork: Option<&Artwork>,
    measured: &Plan,
    size: u32,
) -> Result<RgbaImage> {
    let drawing = Drawing::new(source, artwork)?;
    let from = Spiral::new(&source.disc);
    let to = Spiral::new(&measured.disc);
    let mut image = render(measured, None, size)?;
    let half = f64::from(size) / 2.0;
    let scale = 126.0 / f64::from(size);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let dx = (f64::from(x) + 0.5 - half) * scale;
        let dy = (f64::from(y) + 0.5 - half) * scale;
        let radius = dx.hypot(dy);
        let sector = to.sector(radius);
        if sector >= f64::from(measured.estimated_art_start_sector)
            && sector < f64::from(measured.estimated_art_end_sector)
            && sector >= f64::from(source.estimated_art_start_sector)
            && sector < f64::from(source.estimated_art_end_sector)
        {
            let r = from.radius(sector);
            let angle = dy.atan2(dx) + from.angle(r) - to.angle(radius);
            if let Some(v) = drawing.design(r * angle.cos(), r * angle.sin(), sector as u32, (x, y))
            {
                *pixel = Rgba([v, v, v, 255]);
            }
        }
    }
    Ok(image)
}

const MUSIC: [u8; 4] = [105, 157, 211, 255];
const GAP: [u8; 4] = [226, 117, 127, 255];
const SESSION: [u8; 4] = [216, 164, 92, 255];
const FILES: [u8; 4] = [173, 147, 213, 255];
const ART: [u8; 4] = [129, 199, 187, 255];
const MARGIN: [u8; 4] = [230, 212, 141, 255];

struct Band {
    end: u32,
    color: [u8; 4],
}

/// Shared sector boundaries keep the vector and raster views consistent.
fn bands(plan: &Plan) -> Vec<Band> {
    let mut result = Vec::new();
    let mut end = 0;
    for (i, track) in plan.tracks.iter().enumerate() {
        end += track.sectors;
        let color = if i % 2 == 0 {
            MUSIC
        } else {
            [83, 132, 184, 255]
        };
        result.push(Band { end, color });
        if i + 1 < plan.tracks.len() {
            end += plan.disc.intertrack_gap_sectors;
            result.push(Band { end, color: GAP });
        }
    }
    end += plan.disc.session_reserve_sectors;
    result.push(Band {
        end,
        color: SESSION,
    });
    result.push(Band {
        end: plan.estimated_art_start_sector,
        color: FILES,
    });
    result.push(Band {
        end: plan.estimated_art_end_sector,
        color: ART,
    });
    result.push(Band {
        end: plan.disc.capacity_sectors,
        color: MARGIN,
    });
    result
}

/// Generate before opening the destination, so invalid inputs leave no file.
/// PNG is a composited disc; SVG is a self-contained, labeled two-view report.
pub fn save(plan: &Plan, artwork: Option<&Artwork>, path: &Path, size: u32) -> Result<()> {
    ensure!(
        (256..=4096).contains(&size),
        "preview size must be 256–4096 pixels"
    );
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    ensure!(
        matches!(extension.as_str(), "svg" | "png"),
        "preview output must end in .svg or .png"
    );
    let disc = render(plan, artwork, size)?;
    let mut png = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(disc).write_to(&mut png, ImageFormat::Png)?;
    let bytes = if extension == "svg" {
        report(plan, artwork, &png.into_inner()).into_bytes()
    } else {
        png.into_inner()
    };
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("cannot create {}", path.display()))?;
    std::io::Write::write_all(&mut output, &bytes)?;
    Ok(())
}

/// Geometric composition only; no spiral, CIRC/EFM or optical model is implied.
pub fn render(plan: &Plan, artwork: Option<&Artwork>, size: u32) -> Result<RgbaImage> {
    ensure!(
        (256..=4096).contains(&size),
        "preview size must be 256–4096 pixels"
    );
    let drawing = Drawing::new(plan, artwork)?;
    let mut output = RgbaImage::new(size, size);
    let bands = bands(plan);
    let half = f64::from(size) / 2.0;
    let scale = 63.0 / half;
    let inner = radius_at_sector(&plan.disc, 0);
    let outer = plan.recording_outer_radius();
    let spiral = Spiral::new(&plan.disc);
    for (x, y, pixel) in output.enumerate_pixels_mut() {
        let dx = f64::from(x) + 0.5 - half;
        let dy = f64::from(y) + 0.5 - half;
        let radius = dx.hypot(dy) * scale;
        if !(7.5..=60.0).contains(&radius) {
            continue;
        }
        let color = if radius < inner || radius >= outer {
            [63, 71, 84, 255]
        } else {
            let sector = (spiral.sector(radius) as u32).min(plan.disc.capacity_sectors - 1);
            let band = &bands[bands.partition_point(|b| b.end <= sector)];
            if band.color == ART {
                if let Some(value) = drawing.design(dx * scale, dy * scale, sector, (x, y)) {
                    [value, value, value, 255]
                } else {
                    ART
                }
            } else if drawing.active() {
                [49, 57, 69, 255]
            } else {
                band.color
            }
        };
        *pixel = Rgba(color);
    }
    Ok(output)
}

fn report(plan: &Plan, artwork: Option<&Artwork>, png: &[u8]) -> String {
    let title = escape(&plan.title);
    let mut rings = String::new();
    let mut start = 0;
    for band in bands(plan) {
        let inner = radius_at_sector(&plan.disc, start);
        let outer = radius_at_sector(&plan.disc, band.end);
        rings.push_str(&format!("<circle r=\"{:.5}\" fill=\"none\" stroke=\"#{:02x}{:02x}{:02x}\" stroke-width=\"{:.5}\"/>\n",
            (inner + outer) / 2.0, band.color[0], band.color[1], band.color[2], outer - inner));
        start = band.end;
    }
    let mut legend = String::new();
    for (i, (color, label)) in [
        (MUSIC, "Music (alternating tracks)"),
        (GAP, "Intertrack gaps"),
        (SESSION, "Session transition reserve"),
        (FILES, "Filesystem + files reserve"),
        (ART, "Available artwork annulus"),
        (MARGIN, "End margin"),
    ]
    .iter()
    .enumerate()
    {
        let x = 32 + (i % 3) * 302;
        let y = 510 + (i / 3) * 30;
        legend.push_str(&format!("<rect x=\"{x}\" y=\"{y}\" width=\"12\" height=\"12\" fill=\"#{:02x}{:02x}{:02x}\"/><text x=\"{}\" y=\"{}\">{label}</text>",
            color[0], color[1], color[2], x + 22, y + 11));
    }
    let geometry = plan.geometry_label();
    let settings = if plan.disc.calibration_target {
        "Calibration target · three bands with six measurable edges".into()
    } else if plan.disc.phase_target.is_some() {
        "Phase target · contrast rings, pulse trains and reserved phase steps".into()
    } else if let Some(a) = artwork {
        format!(
            "Mode: {:?} · tone: {:?} · fit (disc): {:?} · rotation: {:.1}° · gamma: {:.2} · brightness: {} · threshold: {} · invert: {}",
            a.mode, a.tone, a.fit, a.rotation, a.gamma, a.brightness, a.threshold, a.invert
        )
    } else {
        "No artwork supplied — colored annulus shows the available budget".into()
    };
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 960 670" width="960" height="670">
<title>{title} — layout and artwork preview</title>
<rect width="960" height="670" rx="16" fill="#11151c"/>
<g fill="#edf2fa" font-family="sans-serif">
<text x="32" y="42" font-size="24">{title}</text>
<text x="32" y="70" font-size="13" fill="#9baabd">{geometry} · no optical simulation</text>
<text x="48" y="109" font-size="16">Sector layout</text>
<text x="520" y="109" font-size="16">Artwork composition</text>
<g transform="translate(248 300) scale(2.9)"><circle r="60" fill="#3f4754"/>{rings}<circle r="7.5" fill="#11151c"/></g>
<image x="526" y="126" width="348" height="348" href="data:image/png;base64,{}"/>
<g font-size="13">{legend}</g>
<text x="32" y="592" font-size="14">{} tracks · music {:.2} min · artwork {:.2}–{:.2} mm · {} sectors</text>
<text x="32" y="620" font-size="11">{settings}</text>
<text x="32" y="647" font-size="11" fill="#9baabd">Design preview. Bundle readback.png projects encoded payload samples; no optical simulation.</text>
</g></svg>
"##,
        STANDARD.encode(png),
        plan.tracks.len(),
        f64::from(plan.music_sectors) / 4500.0,
        plan.artwork_inner_radius_mm,
        plan.artwork_outer_radius_mm,
        plan.artwork_sectors
    )
}

pub(crate) fn escape(text: &str) -> String {
    text.chars()
        .filter(|c| *c >= ' ' || matches!(c, '\n' | '\r' | '\t'))
        .collect::<String>()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_tone_preserves_extremes_and_half_density() {
        let a = Artwork {
            tone: crate::project::Tone::Ordered,
            ..Artwork::default()
        };
        let mut white = 0;
        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(crate::art::tone([0, 0, 0, 255], &a, x, y), 0);
                assert_eq!(crate::art::tone([255; 4], &a, x, y), 255);
                white += u32::from(crate::art::tone([128, 128, 128, 255], &a, x, y) == 255);
            }
        }
        assert_eq!(white, 8);
    }

    #[test]
    fn escapes_project_title() {
        assert_eq!(
            escape("<EP & \"friends\">"),
            "&lt;EP &amp; &quot;friends&quot;&gt;"
        );
    }

    #[test]
    fn measured_projection_has_the_correct_rotation_for_both_image_modes() {
        use crate::{
            audio::AudioTrack,
            project::{Disc, Mode},
        };
        let path =
            std::env::temp_dir().join(format!("discimage-projection-{}.png", std::process::id()));
        let source = RgbaImage::from_fn(80, 80, |x, _| {
            if x < 40 {
                Rgba([0, 0, 0, 255])
            } else {
                Rgba([255; 4])
            }
        });
        source.save(&path).unwrap();
        let plan = Plan::new(
            "rotation".into(),
            Disc::default(),
            vec![AudioTrack {
                path: "unused.wav".into(),
                sample_frames: 176400,
                sectors: 300,
                padding_sample_frames: 0,
                data_offset: 44,
            }],
        )
        .unwrap();
        let mut measured = plan.clone();
        measured.disc.angle = 90.0;
        for mode in [Mode::Disc, Mode::Strip] {
            let art = Artwork {
                path: path.clone(),
                mode,
                ..Artwork::default()
            };
            let reference = render(&plan, Some(&art), 256).unwrap();
            let rotated = projected(&plan, Some(&art), &measured, 256).unwrap();
            // Image coordinates increase clockwise. A +90° placement means
            // sampling design at (dy,-dx); this expectation is independent of
            // the projection's angle arithmetic.
            for (x, y) in [(210, 150), (150, 210), (45, 105), (105, 45)] {
                assert_eq!(rotated.get_pixel(x, y), reference.get_pixel(y, 255 - x));
            }
        }
        std::fs::remove_file(path).unwrap();
    }
}
