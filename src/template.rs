//! Editable artwork canvases derived from the same album plan as authoring.
use crate::{layout::Plan, preview};
use anyhow::{Context, Result, ensure};
use image::{Rgba, RgbaImage};
use std::path::Path;

pub fn save(plan: &Plan, directory: &Path, size: u32) -> Result<()> {
    ensure!(
        (256..=4096).contains(&size),
        "template size must be 256–4096 pixels"
    );
    let inner = plan.artwork_inner_radius_mm;
    let outer = plan.artwork_outer_radius_mm;
    let circumference = std::f64::consts::TAU * (inner + outer) / 2.0;
    let thickness = outer - inner;
    // Match disc pixel density at the middle of the annulus, bounded by the input decoder.
    let density = (f64::from(size) / (2.0 * plan.disc.outer_radius_mm)).min(8192.0 / circumference);
    let width = (circumference * density).round().clamp(1.0, 8192.0) as u32;
    let height = (thickness * density).round().max(1.0) as u32;
    let scale = f64::from(size) / (2.0 * plan.disc.outer_radius_mm);
    let center = f64::from(size) / 2.0;
    let disc = RgbaImage::from_fn(size, size, |x, y| {
        let r = (f64::from(x) + 0.5 - center).hypot(f64::from(y) + 0.5 - center) / scale;
        if r >= inner && r <= outer {
            Rgba([255; 4])
        } else {
            Rgba([0; 4])
        }
    });
    std::fs::create_dir(directory).with_context(|| {
        format!(
            "cannot create new template directory {}",
            directory.display()
        )
    })?;
    std::fs::write(
        directory.join("incomplete"),
        "Template export incomplete.\n",
    )?;
    preview::png(&disc, &directory.join("disc.png"))?;
    preview::png(
        &RgbaImage::from_pixel(width, height, Rgba([255; 4])),
        &directory.join("strip.png"),
    )?;
    std::fs::write(directory.join("disc.svg"), disc_svg(plan, size))?;
    std::fs::write(directory.join("strip.svg"), strip_svg(plan, width, height))?;
    preview::save(plan, None, &directory.join("guide.svg"), 1024)?;
    let metadata = serde_json::json!({
        "format":"discimage-template-v1",
        "plan":plan,
        "disc":{"width":size,"height":size,"recording_radius_mm":plan.disc.outer_radius_mm},
        "strip":{"width":width,"height":height,"middle_circumference_mm":circumference,
            "thickness_mm":thickness,"origin":"12 o'clock before artwork.rotation",
            "direction":"clockwise","top":"outer radius","bottom":"inner radius"},
        "notes":["Uncalibrated geometry estimate. Regenerate when album layout changes.",
            "Paint the artwork layer; SVG guides are hidden by default and must remain hidden on export.",
            "Export SVG artwork as PNG at the declared canvas size; SVG is not an encoder input.",
            "Strip width and height are recommended pixel dimensions, rounded to whole pixels.",
            "Use matching session_start for template, preview and build."]
    });
    std::fs::write(
        directory.join("layout.json"),
        serde_json::to_vec_pretty(&metadata)?,
    )?;
    std::fs::remove_file(directory.join("incomplete"))?;
    Ok(())
}

fn disc_svg(plan: &Plan, size: u32) -> String {
    let title = preview::escape(&plan.title);
    let c = f64::from(size) / 2.0;
    let inner = c * plan.artwork_inner_radius_mm / plan.disc.outer_radius_mm;
    let outer = c * plan.artwork_outer_radius_mm / plan.disc.outer_radius_mm;
    let left = c - outer;
    let right = c + outer;
    let hole_left = c - inner;
    let hole_right = c + inner;
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 {size} {size}">
<title>{title} — disc artwork template</title>
<desc>Paint inside artwork. Protected areas are clipped. Guides are hidden; keep them hidden when exporting PNG.</desc>
<defs><clipPath id="ring"><path clip-rule="evenodd" d="M {right} {c} A {outer} {outer} 0 1 0 {left} {c} A {outer} {outer} 0 1 0 {right} {c} Z M {hole_right} {c} A {inner} {inner} 0 1 0 {hole_left} {c} A {inner} {inner} 0 1 0 {hole_right} {c} Z"/></clipPath></defs>
<g id="artwork" clip-path="url(#ring)"><rect width="{size}" height="{size}" fill="white"/></g>
<g id="guides" style="display:none" fill="none" stroke="#e78f34" stroke-width="1">
<circle cx="{c}" cy="{c}" r="{outer}"/><circle cx="{c}" cy="{c}" r="{inner}"/>
<path d="M {c} 0 V {size} M 0 {c} H {size}" stroke-dasharray="6 6"/>
</g></svg>
"##
    )
}

fn strip_svg(plan: &Plan, width: u32, height: u32) -> String {
    let title = preview::escape(&plan.title);
    let mut guides = String::new();
    for quarter in 0..=4 {
        let x = f64::from(width) * f64::from(quarter) / 4.0;
        guides.push_str(&format!(
            "<path d=\"M {x} 0 V {height}\"/><text x=\"{x}\" y=\"14\">{}°</text>",
            quarter * 90
        ));
    }
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">
<title>{title} — strip artwork template</title>
<desc>Left to right: one clockwise revolution, seam at 12 o'clock before rotation. Top is outer radius; bottom is inner radius. Hide guides when exporting PNG.</desc>
<g id="artwork"><rect width="{width}" height="{height}" fill="white"/></g>
<g id="guides" style="display:none" stroke="#e78f34" stroke-width="1" font-family="sans-serif" font-size="12" fill="#e78f34">{guides}</g>
</svg>
"##
    )
}
