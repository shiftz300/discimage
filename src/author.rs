//! Offline two-session authoring. No device calls or burn commands live here.
// SPDX-License-Identifier: GPL-3.0-or-later
// Payload compensation adapted from DiscImageStudio/CdDataImageGenerator.cs at
// 3a5cde635c182d6a7e01ff99c2cc1be2ac3ab668, Copyright (c) 2026 Disc Image Studio contributors.
// Rust adaptation adds absolute session LBAs, allocated artwork extents and checksummed bundles.
use crate::{
    art::{self, Drawing},
    audio,
    codec::{COOKED, Codec, RAW},
    console,
    iso::{Entry, Volume, extent_offset},
    layout::Plan,
    preview,
    project::{Artwork, Project},
    target::Target,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::{Component, Path},
    time::{Duration, Instant},
};

#[derive(Debug, Serialize, Deserialize)]
pub struct Asset {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub generator: String,
    pub plan: Plan,
    pub session_start: u32,
    pub session_start_source: String,
    pub data_sectors: u32,
    pub artwork: Option<Artwork>,
    pub files: Vec<Entry>,
    pub assets: Vec<Asset>,
}

pub fn build(project: &Path, output: &Path, session_start: Option<u32>) -> Result<Manifest> {
    build_project(Project::load(project)?, output, session_start)
}

pub(crate) fn build_project(
    project: Project,
    output: &Path,
    session_start: Option<u32>,
) -> Result<Manifest> {
    let plan = Plan::from_project(&project, session_start)?;
    let start = plan.session_start();
    let sectors = plan.data_sectors();
    let volume = volume(&project, &plan)?;
    // Input image is decoded before creating any output directory.
    let source = project.artwork.as_ref().map(art::load).transpose()?;
    std::fs::create_dir(output)
        .with_context(|| format!("cannot create new output directory {}", output.display()))?;
    std::fs::write(
        output.join("incomplete"),
        b"Build incomplete. Do not use these assets.\n",
    )?;
    std::fs::create_dir(output.join("audio"))?;
    let mut artwork = project.artwork;
    if let Some(source) = source {
        source.save(output.join("source.png"))?;
        artwork.as_mut().unwrap().path = output.join("source.png");
    }
    let drawing = Drawing::new(&plan, artwork.as_ref())?;
    let mut assets = music(&plan, output)?;
    let files = data(&plan, &volume, &drawing, output)?;
    assets.extend(["session2.iso", "session2.bin", "session2.toc"].map(str::to_owned));
    if plan.disc.calibration_target {
        std::fs::write(
            output.join("target.json"),
            serde_json::to_vec_pretty(&Target::new(&plan))?,
        )?;
        assets.push("target.json".into());
    }
    preview::save_bundle(&plan, artwork.as_ref(), output)?;
    assets.extend(["preview.svg", "design.png"].map(str::to_owned));
    if artwork.is_some() {
        assets.push("source.png".into());
    }
    if let Some(a) = &mut artwork {
        a.path = "source.png".into();
    }
    let mut manifest = Manifest {
        format: "discimage-extra-v1".into(),
        generator: env!("CARGO_PKG_VERSION").into(),
        plan,
        session_start: start,
        session_start_source: if session_start.is_some() {
            "provided"
        } else {
            "planned"
        }
        .into(),
        data_sectors: sectors,
        artwork,
        files,
        assets: Vec::new(),
    };
    preview::readback(output, &manifest, &output.join("readback.png"), 1024, 32)?;
    preview::difference(
        &output.join("design.png"),
        &output.join("readback.png"),
        &output.join("difference.png"),
    )?;
    assets.extend(["readback.png", "difference.png"].map(str::to_owned));
    for name in assets {
        let path = output.join(&name);
        manifest.assets.push(Asset {
            path: name,
            bytes: std::fs::metadata(&path)?.len(),
            sha256: digest(&path)?,
        });
    }
    std::fs::write(
        output.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    std::fs::remove_file(output.join("incomplete"))?;
    Ok(manifest)
}

pub(crate) fn volume(project: &Project, plan: &Plan) -> Result<Volume> {
    Volume::new(
        project.files.as_deref(),
        plan.session_start(),
        plan.data_sectors(),
        plan.disc.data_reserve_sectors,
        &plan.title,
    )
}

fn music(plan: &Plan, output: &Path) -> Result<Vec<String>> {
    let mut assets = Vec::new();
    let mut cue =
        String::from("REM SESSION 1 ONLY - write appendable; see manifest.json for session 2\n");
    for (i, track) in plan.tracks.iter().enumerate() {
        let name = audio::track_path(i);
        audio::export(track, &output.join(&name))?;
        assets.push(name.clone());
        cue.push_str(&format!(
            "FILE \"{name}\" WAVE\n  TRACK {:02} AUDIO\n",
            i + 1
        ));
        if i > 0 && plan.disc.intertrack_gap_sectors > 0 {
            cue.push_str(&format!(
                "    PREGAP {}\n",
                msf(plan.disc.intertrack_gap_sectors)
            ));
        }
        cue.push_str("    INDEX 01 00:00:00\n");
    }
    std::fs::write(output.join("session1.cue"), cue)?;
    assets.push("session1.cue".into());
    Ok(assets)
}

fn data(plan: &Plan, volume: &Volume, drawing: &Drawing<'_>, output: &Path) -> Result<Vec<Entry>> {
    let start = volume.base;
    let sectors = volume.sectors;
    let codec = Codec::default();
    let mut iso = BufWriter::with_capacity(1024 * 1024, File::create(output.join("session2.iso"))?);
    let mut files = volume.prefix(&mut iso)?;
    let drawing_hash = encode_drawing(
        &mut iso,
        start + volume.drawing..start + sectors,
        |physical| drawing.palette(physical),
        || false,
    )?;
    iso.flush()?;
    drop(iso);
    files.push(Entry {
        path: "ARTWORK.BIN".into(),
        lba: plan.estimated_art_start_sector,
        bytes: plan.artwork_sectors * 2048,
        sha256: drawing_hash,
    });
    let mut iso = BufReader::with_capacity(1024 * 1024, File::open(output.join("session2.iso"))?);
    let mut raw = BufWriter::with_capacity(1024 * 1024, File::create(output.join("session2.bin"))?);
    console::phase(
        "encode",
        format_args!("Generating Mode 1 framing for {sectors} sectors"),
    );
    let mut reported = Instant::now();
    for (index, lba) in (start..start + sectors).enumerate() {
        let mut payload = [0; COOKED];
        iso.read_exact(&mut payload)?;
        raw.write_all(&codec.sector(lba, &payload))?;
        sector_progress("encode", index as u32 + 1, sectors, &mut reported);
    }
    raw.flush()?;
    drop(raw);
    // This TOC describes session 2 only; its required absolute start is in the manifest.
    std::fs::write(
        output.join("session2.toc"),
        "// SESSION 2 ONLY - requires manifest.json session_start\nCD_ROM\nTRACK MODE1\nDATAFILE \"session2.iso\"\n",
    )?;
    Ok(files)
}

pub(crate) fn encode_drawing(
    output: &mut impl Write,
    extent: std::ops::Range<u32>,
    palette: impl Fn(i64) -> u8,
    cancelled: impl Fn() -> bool,
) -> Result<String> {
    let codec = Codec::default();
    let count = extent.end - extent.start;
    let progress = count.div_ceil(10).max(1);
    let mut hash = Sha256::new();
    console::phase("drawing", format_args!("Encoding {count} sectors"));
    let mut reported = Instant::now();
    for (index, lba) in extent.enumerate() {
        let mut sector = [0; COOKED];
        for (offset, byte) in sector.iter_mut().enumerate() {
            *byte = palette(codec.physical(lba, offset)) ^ codec.scramble[offset + 4];
        }
        output.write_all(&sector)?;
        hash.update(sector);
        let done = index as u32 + 1;
        if done.is_multiple_of(256) || done.is_multiple_of(progress) || done == count {
            ensure!(!cancelled(), "target generation cancelled");
        }
        if done.is_multiple_of(progress) || done == count {
            show_sectors("drawing", done, count);
            reported = Instant::now();
        } else {
            sector_progress("drawing", done, count, &mut reported);
        }
    }
    Ok(crate::hex(&hash.finalize()))
}

pub fn load(directory: &Path) -> Result<Manifest> {
    ensure!(
        !directory.join("incomplete").try_exists()?,
        "bundle is incomplete"
    );
    let file = File::open(directory.join("manifest.json"))?;
    ensure!(
        file.metadata()?.len() <= 16 * 1024 * 1024,
        "manifest is too large"
    );
    let manifest: Manifest = serde_json::from_reader(file)?;
    ensure!(
        manifest.format == "discimage-extra-v1",
        "unsupported bundle format"
    );
    let p = &manifest.plan;
    ensure!(
        !(p.disc.calibration_target || p.disc.phase_target.is_some()) || manifest.artwork.is_none(),
        "a calibration target cannot also contain artwork"
    );
    let expected = Plan::new(p.title.clone(), p.disc.clone(), p.tracks.clone())?;
    ensure!(
        p.model == expected.model
            && !p.burn_ready
            && p.disc.capacity_sectors == expected.disc.capacity_sectors
            && p.music_sectors == expected.music_sectors
            && p.gap_sectors == expected.gap_sectors
            && p.estimated_art_start_sector == expected.estimated_art_start_sector
            && p.estimated_art_end_sector == expected.estimated_art_end_sector
            && p.artwork_sectors == expected.artwork_sectors
            && (p.artwork_inner_radius_mm - expected.artwork_inner_radius_mm).abs() < 1e-9
            && (p.artwork_outer_radius_mm - expected.artwork_outer_radius_mm).abs() < 1e-9,
        "manifest layout does not match its inputs"
    );
    ensure!(
        manifest.session_start == p.session_start() && manifest.data_sectors == p.data_sectors(),
        "inconsistent session layout"
    );
    ensure!(
        manifest.assets.len() <= 110 && manifest.files.len() <= 10_001,
        "manifest entry limit exceeded"
    );
    Ok(manifest)
}

pub fn verify(directory: &Path) -> Result<Manifest> {
    let manifest = load(directory)?;
    console::phase(
        "verify",
        format_args!(
            "Checking local files and {} Mode 1 sectors",
            manifest.data_sectors
        ),
    );
    let mut seen = std::collections::HashSet::new();
    for asset in &manifest.assets {
        ensure!(
            !asset.path.is_empty()
                && Path::new(&asset.path)
                    .components()
                    .all(|c| matches!(c, Component::Normal(_))),
            "invalid asset path"
        );
        ensure!(seen.insert(asset.path.as_str()), "duplicate asset path");
        let path = directory.join(&asset.path);
        ensure!(
            std::fs::symlink_metadata(&path)?.is_file(),
            "asset must be a regular file"
        );
        ensure!(
            std::fs::metadata(&path)?.len() == asset.bytes && digest(&path)? == asset.sha256,
            "asset checksum mismatch: {}",
            asset.path
        );
    }
    for required in ["session2.iso", "session2.bin", "session1.cue"] {
        ensure!(seen.contains(required), "missing required asset {required}");
    }
    ensure!(
        !manifest.plan.disc.calibration_target || seen.contains("target.json"),
        "missing calibration target metadata"
    );
    for (i, track) in manifest.plan.tracks.iter().enumerate() {
        let name = format!("audio/{:02}.wav", i + 1);
        ensure!(seen.contains(name.as_str()), "missing audio asset {name}");
        let exported = audio::inspect(&directory.join(name))?;
        ensure!(
            exported.sample_frames == track.sectors * 588 && exported.sectors == track.sectors,
            "exported music duration does not match the session layout"
        );
    }
    let mut iso =
        BufReader::with_capacity(1024 * 1024, File::open(directory.join("session2.iso"))?);
    let mut raw =
        BufReader::with_capacity(1024 * 1024, File::open(directory.join("session2.bin"))?);
    ensure!(
        iso.get_ref().metadata()?.len() == u64::from(manifest.data_sectors) * COOKED as u64
            && raw.get_ref().metadata()?.len() == u64::from(manifest.data_sectors) * RAW as u64,
        "incorrect session image length"
    );
    let codec = Codec::default();
    let mut reported = Instant::now();
    for (index, lba) in
        (manifest.session_start..manifest.session_start + manifest.data_sectors).enumerate()
    {
        let mut payload = [0; COOKED];
        let mut sector = [0; RAW];
        iso.read_exact(&mut payload)?;
        raw.read_exact(&mut sector)?;
        ensure!(
            sector == codec.sector(lba, &payload),
            "Mode 1 framing/EDC/ECC mismatch at LBA {lba}"
        );
        sector_progress(
            "verify",
            index as u32 + 1,
            manifest.data_sectors,
            &mut reported,
        );
    }
    for entry in &manifest.files {
        let offset = extent_offset(
            manifest.session_start,
            manifest.data_sectors,
            entry.lba,
            u64::from(entry.bytes),
        )
        .context("file extent outside session")?;
        iso.seek(SeekFrom::Start(offset))?;
        ensure!(
            hash_reader(&mut (&mut iso).take(u64::from(entry.bytes)))? == entry.sha256,
            "ISO file checksum mismatch: {}",
            entry.path
        );
    }
    Ok(manifest)
}

fn sector_progress(label: &str, done: u32, count: u32, reported: &mut Instant) {
    if done.is_multiple_of(256) && reported.elapsed() >= Duration::from_secs(15) {
        show_sectors(label, done, count);
        *reported = Instant::now();
    }
}

fn show_sectors(label: &str, done: u32, count: u32) {
    console::phase(
        label,
        format_args!("{:3}% | {done}/{count} sectors", done * 100 / count),
    );
}

pub fn digest(path: &Path) -> Result<String> {
    hash_reader(&mut BufReader::new(File::open(path)?))
}
fn hash_reader(reader: &mut impl Read) -> Result<String> {
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(crate::hex(&hash.finalize()))
}
fn msf(sectors: u32) -> String {
    format!(
        "{:02}:{:02}:{:02}",
        sectors / 4500,
        sectors / 75 % 60,
        sectors % 75
    )
}
