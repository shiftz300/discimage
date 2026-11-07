use std::{
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

use discimage::{
    audio, author,
    layout::Plan,
    preview,
    project::{Artwork, Disc, Mode, Project},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn timing_import_roundtrips_candidate_and_rejects_changed_evidence() {
    use serde_json::json;
    let temp = Temp::new();
    wav(&temp.0.join("track.wav"), 300 * 588, 44100);
    let image = image::RgbaImage::from_fn(160, 80, |x, y| {
        let value = if (x / 20 + y / 10) % 2 == 0 { 0 } else { 255 };
        image::Rgba([value, value, value, 255])
    });
    image.save(temp.0.join("art.png")).unwrap();
    let settings = "title='timing test'\n[[tracks]]\npath='track.wav'\n[disc]\ncapacity_sectors=14000\ninner_radius_mm=25\nouter_radius_mm=25.5\ndata_reserve_sectors=300\n[artwork]\npath='art.png'\nmode='strip'\n";
    let project = temp.0.join("project.toml");
    std::fs::write(&project, settings).unwrap();
    let bundle = temp.0.join("bundle");
    author::build(&project, &bundle, None).unwrap();
    let bit: f64 = 1200.0 / (75.0 * 57624.0);
    let v = (25.5f64.powi(2) - 25.0f64.powi(2)) * std::f64::consts::PI
        / (bit * bit * 57624.0 * 14000.0);
    let shift = 92.0;
    let u = (25.0f64.powi(2) / (bit * bit) - v * shift * 588.0 / std::f64::consts::PI).sqrt();
    let summary = json!({"fit": {"u":u,"v":v,"media":"cd","sector_bits":57624,
        "read_tail_frames":108,"residual_rms_deg":0.05,"angle_sigma_measured_span_deg":0.1,
        "reference_lba":12500},"cache":{"strategy":"fua","fua_reads_media":true},
        "setup":{"readable_lba_range":[0,14000]}});
    let timing = temp.0.join("summary.json");
    std::fs::write(&timing, summary.to_string()).unwrap();
    let targets = temp.0.join("targets.csv");
    let mut csv = "lba,n_bits,used,period_ms\n".to_string();
    for s in [11800u32, 12000, 12300, 12800, 13200, 13500, 13800, 13900] {
        csv.push_str(&format!(
            "{s},{},32,29\n",
            (u64::from(s) + 1) * 57624 + 108 * 588
        ));
    }
    std::fs::write(&targets, &csv).unwrap();
    let comparison = temp.0.join("comparison");
    let phase = comparison.join("report.json");
    let photo = temp.0.join("rectified.bin");
    let offset = temp.0.join("offset.json");
    let points = temp.0.join("points.json");
    std::fs::write(&photo, b"synthetic rectified evidence").unwrap();
    std::fs::write(&offset, br#"{"forward_offset_frames":19}"#).unwrap();
    let manifest = author::load(&bundle).unwrap();
    let first = f64::from(manifest.plan.estimated_art_start_sector.max(12000));
    let last = f64::from((manifest.plan.estimated_art_end_sector - 1).min(13900));
    assert!(last - first >= 1000.0);
    // At the known shift the synthetic timing model is identical to the
    // generating model, leaving only independent rotations of the two curves.
    let samples: Vec<_> = [("a", 3.0), ("b", -25.0)].into_iter().flat_map(|(curve, rotation)| {
        (0..16).map(move |i| json!({
            "curve": curve, "sector": first + f64::from(i) * (last - first) / 15.0, "phase_deg": rotation,
        }))
    }).collect();
    std::fs::write(
        &points,
        json!({
            "format":"discimage-phase-v1", "convention":"photo-minus-design-clockwise",
            "bundle_sha256":author::digest(&bundle.join("manifest.json")).unwrap(),
            "evidence_sha256":author::digest(&photo).unwrap(), "samples":samples,
        })
        .to_string(),
    )
    .unwrap();
    let run = Command::new(env!("CARGO_BIN_EXE_discimage"))
        .args(["calibrate", "phase", "--timing"])
        .arg(&timing)
        .arg("--bundle")
        .arg(&bundle)
        .arg("--points")
        .arg(&points)
        .arg("--evidence")
        .arg(&photo)
        .args(["--offset-frames", "19", "--offset-evidence"])
        .arg(&offset)
        .arg("--output")
        .arg(&comparison)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(comparison.join("index.html").is_file());
    let output = temp.0.join("candidate");
    let run = Command::new(env!("CARGO_BIN_EXE_discimage"))
        .args(["calibrate", "import"])
        .arg(&project)
        .arg("--timing")
        .arg(&timing)
        .arg("--phase")
        .arg(&phase)
        .arg("--bundle")
        .arg(&bundle)
        .arg("--output")
        .arg(&output)
        .args(["--size", "256"])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let candidate = Project::load(&output.join("candidate.toml")).unwrap();
    for file in [
        "report.json",
        "design.png",
        "before.png",
        "after.png",
        "difference.png",
    ] {
        assert!(output.join(file).is_file(), "{file}");
    }
    assert!(!output.join("index.html").exists());
    assert!(candidate.tracks[0].path.is_absolute());
    assert_eq!(std::fs::read_to_string(&project).unwrap(), settings);
    assert!(!output.join("incomplete").exists());
    assert!(candidate.disc.timing.is_some());
    assert!((candidate.disc.inner_radius_mm - 25.0).abs() < 1e-10);
    assert_eq!(candidate.disc.timing.as_ref().unwrap().end_sector, 13901);
    let plan = Plan::from_project(&candidate, None).unwrap();
    let evidence = candidate.disc.timing.as_ref().unwrap();
    let codec = discimage::codec::Codec::default();
    for sector in [
        plan.estimated_art_start_sector,
        plan.estimated_art_end_sector - 1,
    ] {
        for offset in 0..2048 {
            let physical = codec.physical(sector, offset) as f64 / 2352.0;
            assert!(
                physical >= f64::from(evidence.first_sector)
                    && physical < f64::from(evidence.end_sector)
            );
        }
    }
    let converted = temp.0.join("converted");
    author::build(&output.join("candidate.toml"), &converted, None).unwrap();
    author::verify(&converted).unwrap();
    // Independent bundle hashes prove that the same geometry reaches encoding.
    assert_eq!(
        author::digest(&bundle.join("session2.iso")).unwrap(),
        author::digest(&converted.join("session2.iso")).unwrap()
    );
    for mode in [Mode::Disc, Mode::Strip] {
        let mut p = Project::load(&output.join("candidate.toml")).unwrap();
        p.artwork.as_mut().unwrap().mode = mode;
        let plan = Plan::from_project(&p, None).unwrap();
        preview::save(
            &plan,
            p.artwork.as_ref(),
            &temp.0.join(format!("{mode:?}.png")),
            256,
        )
        .unwrap();
    }
    let hash = author::digest(&output.join("candidate.toml")).unwrap();
    assert!(discimage::timing::import(&project, &timing, &phase, &bundle, &output, 256).is_err());
    assert_eq!(
        hash,
        author::digest(&output.join("candidate.toml")).unwrap()
    );
    // A changed targets file cannot silently broaden trusted coverage.
    std::fs::write(&targets, csv.replace("13900,", "13950,")).unwrap();
    let rejected = temp.0.join("rejected");
    assert!(discimage::timing::import(&project, &timing, &phase, &bundle, &rejected, 256).is_err());
    assert!(!rejected.exists());
    std::fs::write(&targets, &csv).unwrap();
    for (key, value) in [
        ("media", json!("dvd")),
        ("read_tail_frames", json!(111)),
        ("residual_rms_deg", json!(0.5)),
    ] {
        let mut invalid = summary.clone();
        invalid["fit"][key] = value;
        std::fs::write(&timing, invalid.to_string()).unwrap();
        assert!(
            discimage::timing::import(&project, &timing, &phase, &bundle, &rejected, 256).is_err()
        );
        assert!(!rejected.exists());
    }
    std::fs::write(&timing, summary.to_string()).unwrap();
    std::fs::write(temp.0.join("status.json"), r#"{"completed":false}"#).unwrap();
    assert!(discimage::timing::import(&project, &timing, &phase, &bundle, &rejected, 256).is_err());
    // A later NWA/layout change cannot move drawing outside the measured span.
    let mut p = Project::load(&output.join("candidate.toml")).unwrap();
    p.disc.end_margin_sectors = 0;
    assert!(Plan::from_project(&p, None).is_err());
    p.disc.end_margin_sectors = 150;
    p.disc.timing.as_mut().unwrap().first_sector = 12500;
    assert!(Plan::from_project(&p, None).is_err());
}

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "discimage-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

/// Independent RIFF fixture, with an odd-length unknown chunk and its pad byte.
fn wav(path: &Path, frames: u32, rate: u32) {
    let data_len = frames * 4;
    let mut out = std::fs::File::create(path).unwrap();
    out.write_all(b"RIFF").unwrap();
    out.write_all(&(46 + data_len).to_le_bytes()).unwrap();
    out.write_all(b"WAVEJUNK\x01\0\0\0x\0fmt \x10\0\0\0\x01\0\x02\0")
        .unwrap();
    out.write_all(&rate.to_le_bytes()).unwrap();
    out.write_all(&(rate * 4).to_le_bytes()).unwrap();
    out.write_all(b"\x04\0\x10\0data").unwrap();
    out.write_all(&data_len.to_le_bytes()).unwrap();
    out.set_len(u64::from(54 + data_len)).unwrap();
}

#[test]
fn sector_rounding_and_unknown_riff_chunks() {
    let temp = Temp::new();
    let path = temp.0.join("track.wav");
    wav(&path, 176_401, 44_100);
    let track = audio::inspect(&path).unwrap();
    assert_eq!(track.sectors, 301);
    assert_eq!(track.sample_frames, 176_401);
    assert_eq!(track.padding_sample_frames, 587);
}

#[test]
fn rejects_truncation_wrong_rate_and_short_tracks() {
    let temp = Temp::new();
    let path = temp.0.join("bad.wav");
    wav(&path, 176_400, 48_000);
    assert!(audio::inspect(&path).is_err());
    wav(&path, 1, 44_100);
    assert!(audio::inspect(&path).is_err());
    wav(&path, 176_400, 44_100);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(100)
        .unwrap();
    assert!(audio::inspect(&path).is_err());
}

#[test]
fn cli_resolves_project_paths_and_refuses_svg_overwrite() {
    let temp = Temp::new();
    let wav_path = temp.0.join("音乐.wav");
    wav(&wav_path, 176_400, 44_100);
    let manifest = temp.0.join("album.toml");
    std::fs::write(
        &manifest,
        "title = 'EP <&>'\n[[tracks]]\npath = '音乐.wav'\n",
    )
    .unwrap();
    let svg = temp.0.join("layout.svg");
    let output = Command::new(env!("CARGO_BIN_EXE_discimage"))
        .arg("plan")
        .arg(&manifest)
        .arg("--json")
        .arg("--svg")
        .arg(&svg)
        .current_dir(std::env::temp_dir())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(plan["music_sectors"], 300);
    assert_eq!(plan["burn_ready"], false);
    let diagram = std::fs::read_to_string(&svg).unwrap();
    assert!(diagram.contains("EP &lt;&amp;&gt;"));
    let output = Command::new(env!("CARGO_BIN_EXE_discimage"))
        .arg("plan")
        .arg(&manifest)
        .arg("--svg")
        .arg(&svg)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(std::fs::read_to_string(svg).unwrap(), diagram);
}

#[test]
fn rejects_unknown_settings_and_empty_track_list() {
    let temp = Temp::new();
    let path = temp.0.join("album.toml");
    std::fs::write(&path, "title = 'EP'\ntracks = []\n").unwrap();
    assert!(Project::load(&path).is_err());
    std::fs::write(
        &path,
        "title = 'EP'\ntracks = [{ path = 'a.wav' }]\n[disc]\ncapcity_sectors = 42\n",
    )
    .unwrap();
    assert!(Project::load(&path).is_err());
}

#[test]
fn artwork_rotation_and_clipping_preserve_protected_regions() {
    let temp = Temp::new();
    let path = temp.0.join("art.png");
    let source = image::RgbaImage::from_fn(32, 32, |x, _| {
        if x < 16 {
            image::Rgba([0, 0, 0, 255])
        } else {
            image::Rgba([255; 4])
        }
    });
    source.save(&path).unwrap();
    let audio_path = temp.0.join("audio.wav");
    wav(&audio_path, 176_400, 44_100);
    let plan = Plan::new(
        "EP".into(),
        Disc::default(),
        vec![audio::inspect(&audio_path).unwrap()],
    )
    .unwrap();
    let mut art = Artwork {
        path,
        ..Artwork::default()
    };
    let a = preview::render(&plan, Some(&art), 256).unwrap();
    art.rotation = 180.0;
    let b = preview::render(&plan, Some(&art), 256).unwrap();
    assert_eq!(a.get_pixel(20, 128).0, [0, 0, 0, 255]);
    assert_eq!(a.get_pixel(236, 128).0, [255; 4]);
    assert_eq!(b.get_pixel(20, 128).0, [255; 4]);
    assert_eq!(b.get_pixel(236, 128).0, [0, 0, 0, 255]);
    // Rotation cannot affect the hole, outside the disc, or music at ~26 mm.
    for (x, y) in [(128, 128), (0, 0), (180, 128)] {
        assert_eq!(a.get_pixel(x, y), b.get_pixel(x, y));
    }
    assert_eq!(a.get_pixel(128, 128)[3], 0);
    let png = temp.0.join("preview.png");
    preview::save(&plan, Some(&art), &png, 256).unwrap();
    assert_eq!(image::image_dimensions(png).unwrap(), (256, 256));
    assert!(preview::save(&plan, Some(&art), &temp.0.join("bad.png"), 100_000).is_err());
    assert!(!temp.0.join("bad.png").exists());
}

#[test]
fn complete_bundle_is_reproducible_preserves_pcm_and_detects_corruption() {
    let temp = Temp::new();
    let audio = temp.0.join("song.wav");
    wav(&audio, 176_401, 44_100);
    let mut input = std::fs::OpenOptions::new()
        .write(true)
        .open(&audio)
        .unwrap();
    input.seek(SeekFrom::Start(54)).unwrap();
    let pattern: Vec<u8> = (0..256).map(|i| i as u8).collect();
    input.write_all(&pattern).unwrap();
    drop(input);
    image::RgbaImage::from_fn(32, 32, |x, y| {
        let c = if x < y { 0 } else { 255 };
        image::Rgba([c, c, c, 255])
    })
    .save(temp.0.join("art.png"))
    .unwrap();
    std::fs::create_dir(temp.0.join("files")).unwrap();
    std::fs::write(temp.0.join("files/歌词.txt"), "保留随盘内容").unwrap();
    let manifest = temp.0.join("album.toml");
    std::fs::write(&manifest,"title='EP'\nfiles='files'\ntracks=[{path='song.wav'}]\n[disc]\ncapacity_sectors=13000\ndata_reserve_sectors=300\n[artwork]\npath='art.png'\ntone='gray'\n").unwrap();
    let original = author::digest(&audio).unwrap();
    let a = temp.0.join("a");
    let b = temp.0.join("b");
    let built = author::build(&manifest, &a, None).unwrap();
    assert_eq!(built.session_start, 11701);
    assert_eq!(built.session_start_source, "planned");
    assert_eq!(built.data_sectors, 1149);
    author::verify(&a).unwrap();
    assert_eq!(original, author::digest(&audio).unwrap());
    let exported = std::fs::read(a.join("audio/01.wav")).unwrap();
    assert_eq!(&exported[44..44 + 256], pattern);
    assert!(exported[44 + 176_401 * 4..].iter().all(|b| *b == 0));
    let copied = audio::inspect(&a.join("audio/01.wav")).unwrap();
    assert_eq!(copied.sample_frames, 301 * 588);
    assert!(a.join("readback.png").exists() && a.join("difference.png").exists());
    let projected = temp.0.join("verified.png");
    let result = Command::new(env!("CARGO_BIN_EXE_discimage"))
        .arg("verify")
        .arg(&a)
        .arg("--output")
        .arg(&projected)
        .args(["--size", "256", "--samples", "64"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(image::image_dimensions(&projected).unwrap(), (256, 256));
    assert!(String::from_utf8_lossy(&result.stdout).contains("local ISO"));
    assert!(!a.join("incomplete").exists());
    assert!(author::build(&manifest, &a, None).is_err());
    let rebuilt = author::build(&manifest, &b, None).unwrap();
    for (left, right) in built.assets.iter().zip(&rebuilt.assets) {
        assert_eq!((&left.path, &left.sha256), (&right.path, &right.sha256));
    }
    let file = built.files.iter().find(|f| f.path == "歌词.txt").unwrap();
    let mut iso = std::fs::File::open(a.join("session2.iso")).unwrap();
    iso.seek(SeekFrom::Start(
        u64::from(file.lba - built.session_start) * 2048,
    ))
    .unwrap();
    let mut text = vec![0; file.bytes as usize];
    iso.read_exact(&mut text).unwrap();
    assert_eq!(text, "保留随盘内容".as_bytes());
    // Even after updating the asset checksum, parity validation must detect a changed raw sector.
    let raw = a.join("session2.bin");
    let mut f = std::fs::OpenOptions::new().write(true).open(&raw).unwrap();
    f.seek(SeekFrom::Start(2076)).unwrap();
    f.write_all(&[1]).unwrap();
    drop(f);
    let mut changed = author::load(&a).unwrap();
    changed
        .assets
        .iter_mut()
        .find(|a| a.path == "session2.bin")
        .unwrap()
        .sha256 = author::digest(&raw).unwrap();
    std::fs::write(
        a.join("manifest.json"),
        serde_json::to_vec(&changed).unwrap(),
    )
    .unwrap();
    assert!(format!("{:#}", author::verify(&a).unwrap_err()).contains("EDC/ECC"));
}

#[test]
fn iso_metadata_spans_sectors_and_rebases_absolute_extents() {
    let temp = Temp::new();
    let files = temp.0.join("files");
    std::fs::create_dir(&files).unwrap();
    for i in 0..120 {
        std::fs::write(files.join(format!("中文{i:03}.txt")), [i as u8; 9]).unwrap();
    }
    for name in [".hidden", "short.tar.gz"] {
        std::fs::write(files.join(name), [1; 9]).unwrap();
    }
    std::fs::create_dir(files.join("dotted.dir")).unwrap();
    std::fs::write(files.join("dotted.dir/content"), [2; 9]).unwrap();
    let zero = discimage::iso::Volume::new(Some(&files), 0, 500, 400, "TEST").unwrap();
    let shifted = discimage::iso::Volume::new(Some(&files), 12000, 500, 400, "TEST").unwrap();
    let mut a = Vec::new();
    let mut b = Vec::new();
    let entries = zero.prefix(&mut a).unwrap();
    let shifted_entries = shifted.prefix(&mut b).unwrap();
    assert_eq!(a.len(), 400 * 2048);
    assert_eq!(entries.len(), 123);
    for (left, right) in entries.iter().zip(&shifted_entries) {
        assert_eq!(left.lba + 12000, right.lba);
        assert_eq!(left.sha256, right.sha256);
        assert_eq!(
            &a[left.lba as usize * 2048..left.lba as usize * 2048 + 9],
            &b[(right.lba - 12000) as usize * 2048..(right.lba - 12000) as usize * 2048 + 9]
        );
    }
    assert_eq!(&a[16 * 2048 + 1..16 * 2048 + 6], b"CD001");
    assert_eq!(&a[17 * 2048 + 88..17 * 2048 + 91], b"%/E");
    let descriptor = 16 * 2048;
    let root = u32::from_le_bytes(a[descriptor + 158..descriptor + 162].try_into().unwrap())
        as usize
        * 2048;
    let length =
        u32::from_le_bytes(a[descriptor + 166..descriptor + 170].try_into().unwrap()) as usize;
    let mut offset = root;
    while offset < root + length {
        let size = a[offset] as usize;
        if size == 0 {
            offset = (offset / 2048 + 1) * 2048;
            continue;
        }
        let id = &a[offset + 33..offset + 33 + a[offset + 32] as usize];
        if id.len() > 1 {
            assert_eq!(
                id.iter().filter(|&&b| b == b'.').count(),
                usize::from(a[offset + 25] & 2 == 0)
            );
        }
        offset += size;
    }
    assert!(discimage::iso::Volume::new(Some(&files), 0, 500, 20, "TEST").is_err());
    std::fs::write(files.join("ARTWORK.BIN"), b"reserved").unwrap();
    assert!(discimage::iso::Volume::new(Some(&files), 0, 500, 400, "TEST").is_err());
}

#[test]
fn strip_orientation_radial_mapping_and_rotation_match_preview() {
    let temp = Temp::new();
    let path = temp.0.join("strip.png");
    image::RgbaImage::from_fn(400, 40, |x, y| {
        let c = (x / 100 * 50 + if y < 20 { 0 } else { 25 }) as u8;
        image::Rgba([c, c, c, 255])
    })
    .save(&path)
    .unwrap();
    let audio = temp.0.join("track.wav");
    wav(&audio, 176_400, 44_100);
    let plan = Plan::new(
        "EP".into(),
        Disc::default(),
        vec![audio::inspect(&audio).unwrap()],
    )
    .unwrap();
    let mut art = Artwork {
        path,
        mode: Mode::Strip,
        ..Artwork::default()
    };
    let sampler = discimage::art::Sampler::new(&art, &plan).unwrap();
    let point = |u: f64, v: f64| {
        let r = plan.artwork_outer_radius_mm
            - v * (plan.artwork_outer_radius_mm - plan.artwork_inner_radius_mm);
        let angle = u * std::f64::consts::TAU - std::f64::consts::FRAC_PI_2;
        (r * angle.cos(), r * angle.sin())
    };
    for quarter in 0..4 {
        for (v, extra) in [(0.25, 0), (0.75, 25)] {
            let (x, y) = point((f64::from(quarter) + 0.5) / 4.0, v);
            assert_eq!(sampler.sample(x, y), quarter * 50 + extra);
        }
    }
    let (x, y) = point(0.0, 0.25);
    assert_eq!(sampler.sample(x, y), 0);
    let (x, y) = point(1.0 - 1e-6, 0.25);
    assert_eq!(sampler.sample(x, y), 150);
    art.rotation = 90.0;
    let rotated = discimage::art::Sampler::new(&art, &plan).unwrap();
    let (x, y) = point(0.375, 0.25);
    assert_eq!(rotated.sample(x, y), 0);
    assert_eq!(rotated.sample(0.0, 0.0), 255);
    art.rotation = 0.0;
    let preview = preview::render(&plan, Some(&art), 512).unwrap();
    for quarter in 0..4 {
        let (x, y) = point((f64::from(quarter) + 0.5) / 4.0, 0.25);
        let px = (256.0 + x * 512.0 / 126.0).floor() as u32;
        let py = (256.0 + y * 512.0 / 126.0).floor() as u32;
        assert_eq!(
            preview.get_pixel(px, py).0,
            [quarter * 50, quarter * 50, quarter * 50, 255]
        );
    }
}

#[test]
fn templates_use_actual_music_lengths_without_requiring_an_existing_picture() {
    let temp = Temp::new();
    let audio = temp.0.join("track.wav");
    wav(&audio, 176_401, 44_100);
    let project = temp.0.join("album.toml");
    std::fs::write(
        &project,
        "title='EP <&>'\ntracks=[{path='track.wav'}]\n[artwork]\npath='not-drawn-yet.png'\n",
    )
    .unwrap();
    let output = temp.0.join("templates");
    let command = Command::new(env!("CARGO_BIN_EXE_discimage"))
        .arg("template")
        .arg(&project)
        .arg("--output")
        .arg(&output)
        .arg("--size")
        .arg("512")
        .output()
        .unwrap();
    assert!(
        command.status.success(),
        "{}",
        String::from_utf8_lossy(&command.stderr)
    );
    for name in [
        "disc.svg",
        "disc.png",
        "strip.svg",
        "strip.png",
        "guide.svg",
        "layout.json",
    ] {
        assert!(output.join(name).is_file());
    }
    let layout: serde_json::Value =
        serde_json::from_slice(&std::fs::read(output.join("layout.json")).unwrap()).unwrap();
    assert_eq!(layout["plan"]["music_sectors"], 301);
    assert_eq!(layout["disc"]["width"], 512);
    let disc = image::open(output.join("disc.png")).unwrap().to_rgba8();
    assert_eq!(disc.get_pixel(256, 256)[3], 0);
    assert_eq!(disc.get_pixel(0, 0)[3], 0);
    let radius = (layout["plan"]["artwork_inner_radius_mm"].as_f64().unwrap()
        + layout["plan"]["artwork_outer_radius_mm"].as_f64().unwrap())
        / 2.0;
    let x = (256.0 + radius * 512.0 / 116.0).floor() as u32;
    assert_eq!(disc.get_pixel(x, 256).0, [255; 4]);
    let w = layout["strip"]["width"].as_f64().unwrap();
    let h = layout["strip"]["height"].as_f64().unwrap();
    let ratio = layout["strip"]["middle_circumference_mm"].as_f64().unwrap()
        / layout["strip"]["thickness_mm"].as_f64().unwrap();
    assert!((w / h / ratio - 1.0).abs() < 0.02);
    let svg = std::fs::read_to_string(output.join("disc.svg")).unwrap();
    assert!(
        svg.contains("EP &lt;&amp;&gt;")
            && svg.contains("style=\"display:none\"")
            && svg.contains("clip-path")
    );
    assert!(
        discimage::template::save(
            &Plan::from_project(&Project::load(&project).unwrap(), None).unwrap(),
            &output,
            512
        )
        .is_err()
    );
    assert!(!output.join("incomplete").exists());
    wav(&audio, 44_100 * 60, 44_100);
    let longer = Plan::from_project(&Project::load(&project).unwrap(), Some(16000)).unwrap();
    assert!(
        longer.artwork_inner_radius_mm
            > layout["plan"]["artwork_inner_radius_mm"].as_f64().unwrap()
    );
    assert_eq!(longer.estimated_art_start_sector, 26000);
    let next = temp.0.join("longer");
    discimage::template::save(&longer, &next, 512).unwrap();
    let second: serde_json::Value =
        serde_json::from_slice(&std::fs::read(next.join("layout.json")).unwrap()).unwrap();
    assert_eq!(second["plan"]["music_sectors"], 4500);
    assert_eq!(second["plan"]["estimated_art_start_sector"], 26000);
}

#[test]
fn strip_build_encodes_the_same_clockwise_mapping_and_keeps_its_aspect_ratio() {
    let temp = Temp::new();
    let audio = temp.0.join("song.wav");
    wav(&audio, 176_400, 44_100);
    image::RgbaImage::from_fn(512, 32, |x, _| {
        let c = if x < 256 { 0 } else { 255 };
        image::Rgba([c, c, c, 255])
    })
    .save(temp.0.join("strip.png"))
    .unwrap();
    let project = temp.0.join("album.toml");
    std::fs::write(&project,"title='Strip EP'\ntracks=[{path='song.wav'}]\n[disc]\ncapacity_sectors=13000\ndata_reserve_sectors=300\n[artwork]\npath='strip.png'\nmode='strip'\ntone='threshold'\n").unwrap();
    let directory = temp.0.join("bundle");
    let manifest = author::build(&project, &directory, Some(11800)).unwrap();
    let verified = author::verify(&directory).unwrap();
    assert_eq!(verified.artwork.as_ref().unwrap().mode, Mode::Strip);
    assert_eq!(
        image::image_dimensions(directory.join("source.png")).unwrap(),
        (512, 32)
    );
    let mut file = std::fs::File::open(directory.join("session2.iso")).unwrap();
    let codec = discimage::codec::Codec::default();
    let (mut checked, mut black, mut white) = (0, false, false);
    for lba in (manifest.plan.estimated_art_start_sector + 10
        ..manifest.plan.estimated_art_end_sector - 10)
        .step_by(53)
    {
        file.seek(SeekFrom::Start(
            u64::from(lba - manifest.session_start) * 2048,
        ))
        .unwrap();
        let mut sector = [0; 2048];
        file.read_exact(&mut sector).unwrap();
        for offset in (0..2048).step_by(31) {
            let byte = codec.physical(lba, offset);
            let (x, y) = discimage::codec::point(&manifest.plan.disc, byte);
            let angle =
                (y.atan2(x) + std::f64::consts::FRAC_PI_2).rem_euclid(std::f64::consts::TAU);
            if (angle - std::f64::consts::PI).abs() < 0.02 {
                continue;
            }
            let expected = if angle < std::f64::consts::PI {
                black = true;
                0x0d
            } else {
                white = true;
                0xaa
            };
            assert_eq!(sector[offset] ^ codec.scramble[offset + 4], expected);
            checked += 1;
        }
    }
    assert!(checked > 100 && black && white);
    preview::readback(
        &directory,
        &manifest,
        &temp.0.join("readback.png"),
        256,
        128,
    )
    .unwrap();
}

#[test]
fn calibration_cli_binds_photo_points_to_the_written_target() {
    let temp = Temp::new();
    let cli = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_discimage"))
            .current_dir(&temp.0)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    cli(&[
        "calibrate",
        "target",
        "--output",
        "target",
        "--capacity-sectors",
        "16500",
    ]);
    cli(&[
        "build",
        "target/target.toml",
        "--output",
        "written",
        "--session-start",
        "11775",
    ]);
    let directory = temp.0.join("written");
    let target: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("target.json")).unwrap()).unwrap();
    let bands: [[u32; 2]; 3] = serde_json::from_value(target["bands"].clone()).unwrap();
    let manifest = author::verify(&directory).unwrap();
    assert_eq!(manifest.session_start, 11775);
    let codec = discimage::codec::Codec::default();
    let mut data = std::fs::File::open(directory.join("session2.iso")).unwrap();
    let [start, end] = bands[0];
    for (lba, value) in [(start - 5, 0xaa), (start + 5, 0x0d), (end + 5, 0xaa)] {
        data.seek(SeekFrom::Start(
            u64::from(lba - manifest.session_start) * 2048,
        ))
        .unwrap();
        let mut byte = [0];
        data.read_exact(&mut byte).unwrap();
        assert_eq!(byte[0] ^ codec.scramble[4], value);
    }

    // Synthetic front-on photo coordinates with known radii, different from the target defaults.
    let (inner, outer) = (24.6_f64, 57.2_f64);
    let circle = |radius: f64| {
        let d = radius * 8.0;
        [
            [512.0, 512.0 - d],
            [512.0 + d, 512.0],
            [512.0, 512.0 + d],
            [512.0 - d, 512.0],
        ]
    };
    let edges = bands
        .into_iter()
        .flatten()
        .map(|lba| {
            let radius = (inner.powi(2)
                + f64::from(lba) / 16_500.0 * (outer.powi(2) - inner.powi(2)))
            .sqrt();
            [512.0 + 8.0 * radius, 512.0]
        })
        .collect::<Vec<_>>();
    let digest = author::digest(&directory.join("manifest.json")).unwrap();
    let points = serde_json::json!({
        "width": 1024, "height": 1024, "rim": circle(60.0), "hole": circle(7.5),
        "edges": edges, "bundle_sha256": digest,
    });
    std::fs::write(
        temp.0.join("points.json"),
        serde_json::to_vec(&points).unwrap(),
    )
    .unwrap();
    image::GrayImage::new(1024, 1024)
        .save(temp.0.join("photo.png"))
        .unwrap();
    cli(&["calibrate", "mark", "written", "--output", "mark.json"]);
    let descriptor: serde_json::Value =
        serde_json::from_slice(&std::fs::read(temp.0.join("mark.json")).unwrap()).unwrap();
    assert_eq!(descriptor["format"], "discimage-mark-v1");
    assert_eq!(descriptor["bundle_sha256"], digest);
    cli(&[
        "calibrate",
        "fit",
        "written",
        "points.json",
        "--output",
        "profile.toml",
    ]);
    let profile = std::fs::read_to_string(temp.0.join("profile.toml")).unwrap();
    let guided = [
        "calibrate",
        "guide",
        "--output",
        "guide",
        "--bundle",
        "written",
    ];
    cli(&[guided.as_slice(), &["--photo", "photo.png"]].concat());
    let workspace = temp.0.join("guide");
    assert_eq!(std::fs::read_dir(&workspace).unwrap().count(), 1);
    assert!(
        workspace
            .join(format!("mark-{}.json", &digest[..12]))
            .is_file()
    );
    assert!(!workspace.join("record").exists());
    std::fs::copy(temp.0.join("points.json"), workspace.join("points.json")).unwrap();
    cli(&guided);
    assert_eq!(
        std::fs::read_to_string(workspace.join("calibration.toml")).unwrap(),
        profile
    );
    cli(&guided);
    let mut mismatched = points.clone();
    mismatched["bundle_sha256"] = "wrong target".into();
    std::fs::write(
        temp.0.join("other.json"),
        serde_json::to_vec(&mismatched).unwrap(),
    )
    .unwrap();
    assert!(
        discimage::calibrate::fit(
            &directory,
            &temp.0.join("other.json"),
            &temp.0.join("bad.toml")
        )
        .is_err()
    );
    mismatched = points;
    mismatched["rim"][0][1] = 55.0.into();
    std::fs::write(
        temp.0.join("other.json"),
        serde_json::to_vec(&mismatched).unwrap(),
    )
    .unwrap();
    assert!(
        discimage::calibrate::fit(
            &directory,
            &temp.0.join("other.json"),
            &temp.0.join("bad.toml")
        )
        .is_err()
    );

    image::RgbaImage::from_fn(128, 32, |x, _| {
        image::Rgba(if x < 64 { [0, 0, 0, 255] } else { [255; 4] })
    })
    .save(temp.0.join("art.png"))
    .unwrap();
    for mode in ["strip", "disc"] {
        let config = temp.0.join("album.toml");
        std::fs::write(&config, format!(
            "title='EP'\ntracks=[{{path='target/silence.wav'}}]\n[disc]\ndata_reserve_sectors=300\n[artwork]\npath='art.png'\nmode='{mode}'\n{profile}"
        )).unwrap();
        let output = temp.0.join(mode);
        let built = author::build(&config, &output, Some(11775)).unwrap();
        let measured = built.plan.disc.calibration.as_ref().unwrap();
        assert!((measured.inner_radius_mm - inner).abs() < 1e-9);
        assert!((measured.outer_radius_mm - outer).abs() < 1e-9);
        assert_eq!(built.plan.disc.capacity_sectors, 16500);
        assert_eq!(built.plan.model, "constant-pitch-radial-fit-v1");
        assert!(
            std::fs::read_to_string(output.join("preview.svg"))
                .unwrap()
                .contains("Radially calibrated")
        );
        author::verify(&output).unwrap();
    }
}

#[test]
fn encoding_and_previews_match_the_pre_refactor_fixtures() {
    // Captured from the working encoder before the 0.3.1 reorganization.
    // This detects geometry/palette drift that self-consistent readback misses.
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/encoding.json")).unwrap();
    let temp = Temp::new();
    wav(&temp.0.join("audio.wav"), 300 * 588, 44100);
    image::GrayImage::from_fn(16, 8, |x, y| image::Luma([((x * 37 + y * 19) % 256) as u8]))
        .save(temp.0.join("source.png"))
        .unwrap();
    for (name, capacity, settings) in [
        (
            "disc",
            14000,
            "[artwork]\npath='source.png'\nmode='disc'\ntone='ordered'\nrotation=37.0\n",
        ),
        (
            "strip",
            14000,
            "[artwork]\npath='source.png'\nmode='strip'\ntone='threshold'\nrotation=180.0\n",
        ),
        ("radial", 20000, "calibration_target=true\n"),
        (
            "phase",
            14000,
            "[disc.phase_target]\ncontrols=[[12000,12080],[13750,13830]]\nbands=[{start=12200,check=13000,end=13500,velocity=1183.177343317657}]\n",
        ),
    ] {
        if name == "strip" {
            // The legacy encoder enlarged strips before sampling. Supply that
            // same grid explicitly so this remains an independent geometry and
            // palette regression, while new jobs retain their source pixels.
            image::open(temp.0.join("source.png"))
                .unwrap()
                .resize(4096, 4096, image::imageops::FilterType::Lanczos3)
                .save(temp.0.join("source.png"))
                .unwrap();
        }
        let project = temp.0.join(format!("{name}.toml"));
        std::fs::write(&project, format!(
            "title='Regression'\ntracks=[{{path='audio.wav'}}]\n[disc]\ncapacity_sectors={capacity}\ndata_reserve_sectors=300\n{settings}"
        )).unwrap();
        let output = temp.0.join(name);
        author::build(&project, &output, None).unwrap();
        for (file, hash) in expected[name].as_object().unwrap() {
            assert_eq!(
                author::digest(&output.join(file)).unwrap(),
                hash.as_str().unwrap(),
                "{name}/{file}"
            );
        }
    }
}
