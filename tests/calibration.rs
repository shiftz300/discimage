//! Offline acceptance of automatic calibration, with explicit model assumptions.
use discimage::{author, project::Project, timing};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture {
    root: PathBuf,
    target: PathBuf,
    summary: PathBuf,
    repeat: PathBuf,
}
impl Fixture {
    fn new(kind: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "discimage-auto-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let target = root.join("record");
        let measurement = root.join("timing");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir(&measurement).unwrap();
        let iso = target.join("calibration.iso");
        // A small synthetic extent keeps the tests independent of optical hardware.
        fs::File::create(&iso)
            .unwrap()
            .set_len(23850 * 2048)
            .unwrap();
        let receipt = json!({"format":"discimage-calibration-single-v1","reference_sectors":24000,
            "written_sectors":23850,"iso_sha256":author::digest(&iso).unwrap(),"speed":10,
            "drive":{"id":"mock-drive","name":"Mock recorder","backend":"mock","capabilities":null,
            "media":{"kind":kind,"blank":true,"appendable":false,"reserved":true,"free_sectors":24000,"nwa":0,"sessions":0,"tracks":0},"tracks":[]}});
        fs::write(target.join("single.json"), receipt.to_string()).unwrap();
        let payload = 300u32 * 2352;
        let mut wav = fs::File::create(root.join("unused.wav")).unwrap();
        wav.write_all(b"RIFF").unwrap();
        wav.write_all(&(payload + 36).to_le_bytes()).unwrap();
        wav.write_all(b"WAVEfmt \x10\0\0\0\x01\0\x02\0").unwrap();
        wav.write_all(&44100u32.to_le_bytes()).unwrap();
        wav.write_all(&176400u32.to_le_bytes()).unwrap();
        wav.write_all(b"\x04\0\x10\0data").unwrap();
        wav.write_all(&payload.to_le_bytes()).unwrap();
        wav.set_len(u64::from(payload) + 44).unwrap();
        let mut csv = String::from("lba,n_bits,used,period_ms,sigma_deg,residual_deg\n");
        for lba in [100u32, 3000, 6000, 9000, 12000, 15000, 18000, 21000, 23000] {
            csv.push_str(&format!(
                "{lba},{},32,29,0.2,0.1\n",
                (u64::from(lba) + 1) * 57624 + 108 * 588
            ));
        }
        fs::write(measurement.join("targets.csv"), csv).unwrap();
        let summary = measurement.join("summary.json");
        let repeat = root.join("repeat/summary.json");
        let f = Self {
            root,
            target,
            summary,
            repeat,
        };
        f.save_summary(
            json!({"fit":{"media":"cd","sector_bits":57624,"read_tail_frames":108,
            "u":89491.85907555123,"v":81.10106071239952,"reference_lba":12000,
            "residual_rms_deg":0.1,"angle_sigma_measured_span_deg":0.1},
            "cache":{"strategy":"fua","fua_reads_media":true},
            "rotation_check":{"checked":true,"looks_clv":false},
            "setup":{"readable_lba_range":[0,23850],"experiment_range":[2,23848]},
            "calibration_sha256":author::digest(&f.target.join("single.json")).unwrap()}),
        );
        fs::create_dir(f.repeat.parent().unwrap()).unwrap();
        fs::write(
            f.repeat.with_file_name("targets.csv"),
            fs::read_to_string(f.summary.with_file_name("targets.csv"))
                .unwrap()
                .replace(",0.2,0.1", ",0.2,-0.1"),
        )
        .unwrap();
        let mut other: Value =
            serde_json::from_str(&fs::read_to_string(&f.summary).unwrap()).unwrap();
        other["fit"]["reference_lba"] = json!(9000);
        f.save_at(&f.repeat, other);
        f
    }
    fn save_summary(&self, summary: Value) {
        self.save_at(&self.summary, summary);
    }
    fn save_at(&self, path: &Path, summary: Value) {
        fs::write(path, summary.to_string()).unwrap();
        fs::write(
            path.with_file_name("status.json"),
            json!({"completed":true,
            "summary_sha256":author::digest(path).unwrap(),
            "targets_sha256":author::digest(&path.with_file_name("targets.csv")).unwrap()})
            .to_string(),
        )
        .unwrap();
    }
    fn export(&self, output: &Path, points: Option<&Path>) -> anyhow::Result<()> {
        timing::preset(
            &[self.summary.clone(), self.repeat.clone()],
            &self.target,
            "batch",
            points,
            output,
        )
    }
    fn disc(&self, preset: &Path) -> discimage::project::Disc {
        let project = self.root.join("album.toml");
        fs::write(&project,format!("title='album'\ntracks=[{{path='unused.wav'}}]\npreset={{file={:?},name='batch'}}\n",preset.to_string_lossy())).unwrap();
        Project::load(&project).unwrap().disc
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn angle(d: &discimage::project::Disc, s: f64) -> f64 {
    let span = d.outer_radius_mm.powi(2) - d.inner_radius_mm.powi(2);
    let r = (d.inner_radius_mm.powi(2) + s / f64::from(d.capacity_sectors) * span).sqrt();
    2.0 * d.velocity * (f64::from(d.capacity_sectors) / 75.0) / span * (r - d.inner_radius_mm)
}
#[test]
fn automatic_preset_is_loadable_preserves_timing_angles_and_refines_without_reburn() {
    for kind in ["cd-r", "cd-rw"] {
        let f = Fixture::new(kind);
        let output = f.root.join("disc.toml");
        f.export(&output, None).unwrap();
        let original = f.disc(&output);
        assert_eq!(original.media.unwrap().kind(), kind);
        let evidence = original.timing.as_ref().unwrap();
        assert_eq!((evidence.first_sector, evidence.end_sector), (101, 23001));
        assert_eq!(evidence.phase_sha256, None);
        assert_eq!(evidence.assumed_shift_frames, Some(108.0));
        assert_eq!(evidence.repeat_sha256.len(), 1);
        evidence.validate().unwrap();
        let expected = |s: f64| {
            std::f64::consts::TAU / 81.10106071239952
                * (89491.85907555123_f64.powi(2)
                    + 81.10106071239952 * (s * 57624.0 + 108.0 * 588.0) / std::f64::consts::PI)
                    .sqrt()
        };
        for s in [101.0, 12000.25, 23000.0] {
            assert!(
                ((angle(&original, s) - angle(&original, 12000.0))
                    - (expected(s) - expected(12000.0)))
                .abs()
                    < 1e-8
            );
        }
        let hash = author::digest(&output).unwrap();
        f.export(&output, None).unwrap();
        assert_eq!(author::digest(&output).unwrap(), hash);
        let edges: Vec<_> = [5u32, 17, 32, 51, 69, 94]
            .map(|p| 300 + (23850 - 300) * p / 100)
            .iter()
            .map(|s| {
                let span = original.outer_radius_mm.powi(2) - original.inner_radius_mm.powi(2);
                let r = (original.inner_radius_mm.powi(2) + f64::from(*s) / 24000.0 * span).sqrt()
                    * 0.995;
                [500.0 + r * 7.5, 500.0]
            })
            .collect();
        let points = f.root.join("points.json");
        fs::write(
            &points,
            json!({"bundle_sha256":author::digest(&f.target.join("single.json")).unwrap(),
            "width":1000,"height":1000,"rim":[[950,500],[500,950],[50,500],[500,50]],
            "hole":[[556.25,500.0],[500.0,556.25],[443.75,500.0],[500.0,443.75]],"edges":edges})
            .to_string(),
        )
        .unwrap();
        f.export(&output, Some(&points)).unwrap();
        let refined = f.disc(&output);
        let refined_hash = author::digest(&output).unwrap();
        assert!(f.export(&output, None).is_err());
        assert_eq!(author::digest(&output).unwrap(), refined_hash);
        assert!((refined.outer_radius_mm - original.outer_radius_mm * 0.995).abs() < 1e-6);
        for s in [101.0, 12000.25, 23000.0] {
            assert!((angle(&original, s) - angle(&refined, s)).abs() < 1e-8);
        }
        let value: toml::Table = toml::from_str(&fs::read_to_string(&output).unwrap()).unwrap();
        assert_eq!(
            value["verification"]["batch"]["phase_measured"].as_bool(),
            Some(false)
        );
        assert!(value["measurements"]["batch"].is_table());
        let package = f.root.join("album-output");
        discimage::package::build(&f.root.join("album.toml"), &package, None, None).unwrap();
        let frozen = discimage::package::Package::load(&package).unwrap();
        frozen.verify(&package).unwrap();
        assert_eq!(
            frozen.project.disc.timing.unwrap().assumed_shift_frames,
            Some(108.0)
        );
    }
}
#[test]
fn automatic_preset_rejects_bad_or_mismatched_evidence_and_preserves_previous_file() {
    let f = Fixture::new("cd-r");
    let output = f.root.join("disc.toml");
    f.export(&output, None).unwrap();
    let good: Value = serde_json::from_str(&fs::read_to_string(&f.summary).unwrap()).unwrap();
    let hash = author::digest(&output).unwrap();
    for (field, value) in [
        ("quality", json!(5.0)),
        ("cache", json!("evict")),
        ("target", json!("wrong")),
        ("rotation", json!(true)),
        ("unchecked", json!(false)),
    ] {
        let mut bad = good.clone();
        match field {
            "quality" => bad["fit"]["residual_rms_deg"] = value,
            "cache" => bad["cache"]["strategy"] = value,
            "target" => bad["calibration_sha256"] = value,
            "rotation" => bad["rotation_check"]["looks_clv"] = value,
            _ => bad["rotation_check"]["checked"] = value,
        }
        f.save_summary(bad);
        assert!(f.export(&output, None).is_err(), "accepted {field}");
        assert_eq!(author::digest(&output).unwrap(), hash);
    }
    f.save_summary(good);
    let current = fs::read_to_string(&output).unwrap();
    fs::write(
        &output,
        current.replace("physical validation pending", "visually accepted"),
    )
    .unwrap();
    let manual = fs::read(&output).unwrap();
    assert!(f.export(&output, None).is_err());
    assert_eq!(fs::read(&output).unwrap(), manual);
    fs::write(&output, current).unwrap();
    fs::write(
        f.summary.with_file_name("status.json"),
        "{\"completed\":false}",
    )
    .unwrap();
    assert!(f.export(&output, None).is_err());
    assert_eq!(author::digest(&output).unwrap(), hash);
}
#[test]
fn guided_cli_finishes_offline_and_reruns_without_accessing_a_drive() {
    let f = Fixture::new("cd-rw");
    let output = f.root.join("batch");
    for _ in 0..2 {
        let result = Command::new(env!("CARGO_BIN_EXE_discimage"))
            .args(["calibrate", "--output"])
            .arg(&output)
            .arg("--bundle")
            .arg(&f.target)
            .arg("--timing")
            .arg(&f.summary)
            .arg("--timing")
            .arg(&f.repeat)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(output.join("disc.toml").is_file());
        assert!(!output.join("record").exists());
        assert!(!output.join("timing").exists());
    }
    let mut unfinished = f.root.join("unfinished");
    fs::create_dir_all(unfinished.join("record")).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_discimage"))
        .args(["calibrate", "-o"])
        .arg(&unfinished)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("no erase or rewrite attempted"));
    unfinished.push("record");
    assert_eq!(fs::read_dir(unfinished).unwrap().count(), 0);
}

#[test]
fn repeatability_uses_uncertainty_instead_of_an_rms_cutoff() {
    let f = Fixture::new("cd-rw");
    let paths = [f.summary.clone(), f.repeat.clone()];
    // Equal geometry, different anchors and large measured noise remain repeatable.
    for (i, path) in paths.iter().enumerate() {
        let csv = fs::read_to_string(path.with_file_name("targets.csv"))
            .unwrap()
            .replace(",0.2,0.1", ",2,1.2")
            .replace(",0.2,-0.1", ",2,-1.5");
        fs::write(path.with_file_name("targets.csv"), csv).unwrap();
        let mut summary: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        summary["fit"]["residual_rms_deg"] = json!(if i == 0 { 1.2 } else { 1.5 });
        f.save_at(path, summary);
    }
    let result = timing::assess(&paths).unwrap();
    assert!(result.consistent);
    assert!(result.rounds[0].residual_rms_deg > 0.3);
    f.export(&f.root.join("disc.toml"), None).unwrap();
    // A quieter residual alone cannot make two incompatible geometries agree.
    for path in &paths {
        let csv = fs::read_to_string(path.with_file_name("targets.csv"))
            .unwrap()
            .replace(",2,1.2", ",0.05,0.01")
            .replace(",2,-1.5", ",0.05,-0.01");
        fs::write(path.with_file_name("targets.csv"), csv).unwrap();
        let mut summary: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        summary["fit"]["residual_rms_deg"] = json!(0.01);
        f.save_at(path, summary);
    }
    let mut changed: Value = serde_json::from_str(&fs::read_to_string(&f.repeat).unwrap()).unwrap();
    changed["fit"]["u"] = json!(89591.85907555123);
    f.save_at(&f.repeat, changed);
    let result = timing::assess(&paths).unwrap();
    assert!(!result.consistent);
    let previous = fs::read(f.root.join("disc.toml")).unwrap();
    assert!(f.export(&f.root.join("disc.toml"), None).is_err());
    assert_eq!(fs::read(f.root.join("disc.toml")).unwrap(), previous);
}

#[test]
fn repeat_assessment_rejects_copies_and_invalid_uncertainties() {
    let f = Fixture::new("cd-r");
    assert!(timing::assess(std::slice::from_ref(&f.summary)).is_err());
    assert!(timing::assess(&[f.summary.clone(), f.summary.clone()]).is_err());
    let paths = [f.summary.clone(), f.repeat.clone()];
    let original = fs::read_to_string(f.repeat.with_file_name("targets.csv")).unwrap();
    let summary: Value = serde_json::from_str(&fs::read_to_string(&f.repeat).unwrap()).unwrap();
    for csv in [
        original.replace(",0.2,", ",0,"),
        original.replace(",0.2,", ",NaN,"),
        original.clone() + original.lines().nth(1).unwrap() + "\n",
        original.replace("sigma_deg", "unknown"),
    ] {
        fs::write(f.repeat.with_file_name("targets.csv"), csv).unwrap();
        f.save_at(&f.repeat, summary.clone());
        assert!(timing::assess(&paths).is_err());
    }
}

#[test]
fn most_precise_round_wins_even_with_larger_residual_rms() {
    let f = Fixture::new("cd-r");
    let path = &f.repeat;
    fs::write(
        path.with_file_name("targets.csv"),
        fs::read_to_string(path.with_file_name("targets.csv"))
            .unwrap()
            .replace(",0.2,-0.1", ",0.05,-0.11"),
    )
    .unwrap();
    let mut summary: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    summary["fit"]["residual_rms_deg"] = json!(0.11);
    f.save_at(path, summary);
    let a = timing::assess(&[f.summary.clone(), f.repeat.clone()]).unwrap();
    assert!(a.consistent);
    assert_eq!(a.selected, 1);
    assert!(a.rounds[1].residual_rms_deg > a.rounds[0].residual_rms_deg);
    assert!(a.rounds[1].max_confidence_band_deg < a.rounds[0].max_confidence_band_deg);
}

#[test]
fn uncertainty_rejects_unidentifiable_data_and_intersects_real_coverage() {
    let f = Fixture::new("cd-r");
    let paths = [f.summary.clone(), f.repeat.clone()];
    let original = fs::read_to_string(f.repeat.with_file_name("targets.csv")).unwrap();
    let summary: Value = serde_json::from_str(&fs::read_to_string(&f.repeat).unwrap()).unwrap();
    let mut csv = original.lines().next().unwrap().to_owned() + "\n";
    for (i, line) in original.lines().skip(1).enumerate() {
        let mut cols: Vec<String> = line.split(',').map(str::to_owned).collect();
        let lba = 12000 + i * 150;
        cols[0] = lba.to_string();
        cols[1] = ((lba + 1) * 57624 + 108 * 588).to_string();
        cols[4] = "1000000".into();
        csv += &(cols.join(",") + "\n");
    }
    let mut narrow = summary.clone();
    narrow["fit"]["reference_lba"] = json!(12600);
    fs::write(f.repeat.with_file_name("targets.csv"), csv).unwrap();
    f.save_at(&f.repeat, narrow);
    let error = timing::assess(&paths).unwrap_err().to_string();
    assert!(error.contains("identifiable"), "{error}");
    // Adjust both the LBA and its independent bit coordinate, preserving valid data.
    let mut csv = original.lines().next().unwrap().to_owned() + "\n";
    for line in original.lines().skip(1) {
        let mut cols: Vec<String> = line.split(',').map(str::to_owned).collect();
        let lba = match cols[0].as_str() {
            "100" => 1100,
            "23000" => 22000,
            _ => cols[0].parse::<usize>().unwrap(),
        };
        cols[0] = lba.to_string();
        cols[1] = ((lba + 1) * 57624 + 108 * 588).to_string();
        csv += &(cols.join(",") + "\n");
    }
    fs::write(f.repeat.with_file_name("targets.csv"), csv).unwrap();
    f.save_at(&f.repeat, summary);
    let a = timing::assess(&paths).unwrap();
    assert!(a.consistent);
    assert_eq!(a.encoding_range, [1101, 22001]);
}

#[test]
fn windows_completion_jitter_is_normalized_by_period_and_sample_count() {
    let f = Fixture::new("cd-rw");
    let paths = [f.summary.clone(), f.repeat.clone()];
    // Derived from the documented Windows GX40N controls, not an OS exception:
    // the same ~315 us completion scatter is ~9.4 deg at 12 ms, ~3.95 at 28.7 ms.
    for (path, period_ms, rms) in [
        (&f.summary, 12.1095_f64, 1.770_f64),
        (&f.repeat, 28.7028, 0.7325),
    ] {
        let spread = 314.7 * 360.0 / (period_ms * 1000.0);
        let sigma = (spread / 32.0_f64.sqrt()).max(0.2);
        let mut csv = "lba,n_bits,used,period_ms,sigma_deg,residual_deg,std_deg\n".to_owned();
        for i in 0..36 {
            let lba = 100 + 22900 * i / 35;
            let residual = if i % 2 == 0 { rms } else { -rms };
            csv += &format!(
                "{lba},{},32,{period_ms},{sigma},{residual},{spread}\n",
                (lba + 1) * 57624 + 108 * 588
            );
        }
        fs::write(path.with_file_name("targets.csv"), csv).unwrap();
        let mut summary: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        summary["fit"]["residual_rms_deg"] = json!(rms);
        f.save_at(path, summary);
    }
    let a = timing::assess(&paths).unwrap();
    assert!(a.consistent);
    assert!(a.rounds.iter().all(|r| r.residual_rms_deg > 0.3));
    assert!(
        a.rounds
            .iter()
            .all(|r| (1.0..1.5).contains(&r.chi2_per_dof))
    );
    for r in &a.rounds {
        assert!((r.median_completion_jitter_us.unwrap() - 314.7).abs() < 1e-6);
    }
    assert!(
        a.rounds[0].median_phase_scatter_deg.unwrap()
            > 2.0 * a.rounds[1].median_phase_scatter_deg.unwrap()
    );
    assert_eq!(a.selected, 1);
    // The same normalized data reaches the real preset exporter on every OS.
    f.export(&f.root.join("disc.toml"), None).unwrap();
}
