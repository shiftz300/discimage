//! Read-only CD timing acquisition and validation shared by macOS and Windows.
mod cache;
pub(crate) mod fit;
mod period;
pub(crate) mod transport;

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufWriter, Write},
    ops::Range,
    path::Path,
    time::Duration,
};

use crate::{
    author, console, device,
    spiral::{READ_TAIL_FRAMES, SECTOR_BITS, read_end_bits},
};
use cache::Cache;
use fit::Sample;
use period::Period;
use transport::{Command, Device, Link, READ_CHUNK, SECTOR_BYTES};

const PERIOD_TOL: f64 = 0.002;
const REPS: usize = 32;

/// Discovery reads only INQUIRY and GET CONFIGURATION metadata. It does not reserve media, set speed,
/// run the timing probe or bypass a failed permissions check.
#[cfg(target_os = "macos")]
pub(crate) fn hardware_identity(id: &str) -> Option<crate::device::Identity> {
    let mut link = Device::for_identity(id).ok()?;
    let standard = link
        .command(Command::Inquiry { serial: false })
        .ok()?
        .good()
        .ok()?;
    if let Some(reply) = link
        .command(Command::DriveSerial)
        .ok()
        .and_then(|reply| reply.good().ok())
        && let Some(identity) =
            crate::device::Identity::from_configuration(&standard.data, &reply.data)
    {
        return Some(identity);
    }
    let serial = link
        .command(Command::Inquiry { serial: true })
        .ok()?
        .good()
        .ok()?;
    crate::device::Identity::from_inquiry(&standard.data, &serial.data)
}

pub fn inspect(id: &str) -> Result<Range<u32>> {
    let id = device::probe_id(id)?;
    device::install_cancel_handler()?;
    disc_bounds(&mut Device::open(&id)?, true)
}

pub fn acquire(id: &str, requested: Option<Range<u32>>, output: &Path) -> Result<()> {
    acquire_inner(id, requested, output, None, false)
}

/// Measure a specific recorded calibration target, checking it under the probe reservation.
pub fn acquire_target(id: &str, output: &Path, target: &Path) -> Result<()> {
    acquire_inner(id, None, output, Some(target), false)
}

/// Measure the readable prefix of an interrupted target without closing the disc.
pub fn acquire_partial(id: &str, output: &Path, target: &Path) -> Result<()> {
    acquire_inner(id, None, output, Some(target), true)
}

fn acquire_inner(
    id: &str,
    requested: Option<Range<u32>>,
    output: &Path,
    target: Option<&Path>,
    partial: bool,
) -> Result<()> {
    if let Some(range) = &requested {
        ensure!(
            range.end <= 360_000 && range.end.saturating_sub(range.start) > 1000,
            "expected an absolute LBA range with more than 1000 CD sectors (end exclusive)"
        );
    }
    ensure!(!output.try_exists()?, "use a new probe output directory");
    let resolved = device::probe_id(id)?;
    device::install_cancel_handler()?;
    let pending = partial
        .then(|| crate::calibrate::pending(target.context("missing partial target")?))
        .transpose()?;
    #[cfg(target_os = "macos")]
    if let Some(target) = target {
        if !partial {
            crate::calibrate::single(target)?;
        }
        crate::macos::prepare_probe(&resolved, partial)?;
    }
    let mut link = Device::open(&resolved)?;
    let bounds = |link: &mut dyn Link, initial| match &pending {
        Some(receipt) => partial_bounds(link, receipt.written_sectors, initial),
        None => disc_bounds(link, initial),
    };
    let verify = |link: &mut dyn Link, readable: &Range<u32>, target: &Path| {
        if let Some(receipt) = &pending {
            verify_target_samples(link, readable, target)?;
            Ok(receipt.image_digest().to_owned())
        } else {
            verify_target(link, readable, target)
        }
    };
    let reported = bounds(&mut link, true)?;
    let readable = if partial {
        readable_prefix(&mut link, reported.clone(), |link| bounds(link, true))?
    } else {
        reported.clone()
    };
    let target_hash = target
        .map(|target| verify(&mut link, &readable, target))
        .transpose()?;
    #[cfg(windows)]
    let _priority = transport::timing_priority();
    let Range { start: first, end } = requested.unwrap_or_else(|| {
        if target.is_some() && !partial {
            return readable.start + 2..readable.end.saturating_sub(2);
        }
        let span = readable.end - readable.start - 1;
        let outer_percent = if partial { 95 } else { 85 };
        readable.start + span * 3 / 100..readable.start + span * outer_percent / 100
    });
    ensure!(
        readable.start <= first && end <= readable.end,
        "requested LBAs fall outside the confirmed data track {}..{}",
        readable.start,
        readable.end
    );
    fs::create_dir(output).context("cannot create probe output directory")?;
    let mut receipt = json!({
        "format": "discimage-probe-v1", "device": id, "native_device": resolved,
        "requested_range": [first, end], "completed": false,
    });
    #[cfg(windows)]
    {
        receipt["transport"] = json!({
            "backend": "windows-imapi2", "exclusive": true,
            "clock": "QueryPerformanceCounter", "timing": "synchronous-command",
        });
    }
    #[cfg(target_os = "macos")]
    {
        receipt["transport"] = json!({
            "backend": "macos-iokit", "exclusive": true,
            "clock": "CLOCK_UPTIME_RAW", "timing": "synchronous-command",
        });
    }
    save(&output.join("status.json"), &receipt)?;
    let reference = first + (end - first) / 2;
    let mut probe = Probe {
        link: &mut link,
        bounds: readable.clone(),
        reference,
        read12: true,
        cache: Cache::Plain,
        evict_cursor: 0,
        period: None,
        random: 1,
    };
    let speed = probe.setup_reads()?;
    let cache = probe.check_cache()?;
    receipt["set_cd_speed"] = speed.clone();
    receipt["cache"] = cache.clone();
    save(&output.join("status.json"), &receipt)?;
    console::phase(
        "probe",
        format_args!(
            "Verified Mode 1 range {}..{} | reference LBA {reference} | cache {}",
            readable.start, readable.end, cache["strategy"]
        ),
    );
    let rotation = probe.check_rotation(first..end)?;
    receipt["rotation_check"] = rotation.clone();
    save(&output.join("status.json"), &receipt)?;
    ensure!(
        rotation["looks_clv"] != true,
        "drive appears to use constant linear velocity across this range; geometry timing needs stable constant rotation"
    );
    probe.settle()?;
    let fit = fit::ladder(first..end, reference, partial, |lba| probe.measure(lba))?;
    let method = if probe.read12 { "read12" } else { "read10" };
    let period_ms = probe.period.map(|seconds| seconds * 1000.0);
    ensure!(
        bounds(&mut link, false)? == reported,
        "disc layout changed during measurement"
    );
    if let Some(target) = target {
        ensure!(
            Some(verify(&mut link, &readable, target)?) == target_hash,
            "calibration target changed during measurement"
        );
    }
    check_cancel()?;
    let measured = fit.points.iter().map(|p| p.sample.lba).chain([reference]);
    let lo = measured.clone().min().unwrap_or(reference);
    let hi = measured.max().unwrap_or(reference);
    let mut summary = json!({
        "fit": {
            "media": "cd", "sector_bits": SECTOR_BITS, "read_tail_frames": READ_TAIL_FRAMES,
            "read_completion_model_measured": false, "u": fit.geometry.u, "v": fit.geometry.v,
            "reference_lba": reference, "c_deg": fit.offset.to_degrees(),
            "period_ms": period_ms,
            "residual_rms_deg": fit.rms_deg, "chi2_per_dof": fit.chi2_per_dof,
            "angle_sigma_measured_span_deg": fit.span_sigma_deg(lo, hi),
        },
        "setup": {
            "family": "cd", "read_method": method,
            "readable_lba_range": [readable.start, readable.end],
            "experiment_range": [first, end],
            "rejected_lbas": fit.rejected,
        },
        "cache": cache,
        "rotation_check": rotation,
        "set_cd_speed": speed,
        "calibration_sha256": target_hash,
    });
    if partial {
        summary["partial_target"] = json!(true);
    }
    let targets_path = output.join("targets.csv");
    let mut csv = BufWriter::new(fs::File::create(&targets_path)?);
    writeln!(
        csv,
        "lba,n_bits,phase_deg,sigma_deg,std_deg,used,predicted_deg,unwrapped_deg,model_deg,residual_deg,period_ms"
    )?;
    for point in &fit.points {
        let s = point.sample;
        let model = fit.predict(s.lba);
        writeln!(
            csv,
            "{},{},{:.6},{:.6},{:.6},{},{:.6},{:.6},{:.6},{:.6},{:.6}",
            s.lba,
            read_end_bits(s.lba),
            s.phase.to_degrees(),
            s.sigma.to_degrees(),
            s.spread.to_degrees(),
            s.used,
            point.predicted.to_degrees(),
            point.unwrapped.to_degrees(),
            model.to_degrees(),
            (point.unwrapped - model).to_degrees(),
            s.period * 1000.0
        )?;
    }
    csv.flush()?;
    drop(csv);
    let summary_path = output.join("summary.json");
    save(&summary_path, &summary)?;
    receipt["completed"] = json!(true);
    receipt["summary_sha256"] = json!(author::digest(&summary_path)?);
    receipt["targets_sha256"] = json!(author::digest(&targets_path)?);
    let finished = output.join("status.tmp");
    save(&finished, &receipt)?;
    fs::rename(finished, output.join("status.json"))?;
    console::success(format_args!(
        "{} timing rungs | RMS {:.3}° | measured LBA {lo}..={hi}",
        fit.points.len(),
        fit.rms_deg
    ));
    println!("Timing summary: {}", summary_path.display());
    Ok(())
}

fn verify_target(link: &mut dyn Link, readable: &Range<u32>, directory: &Path) -> Result<String> {
    let receipt = crate::calibrate::single(directory)?;
    ensure!(
        readable.start == 0
            && (readable.end == receipt.written_sectors
                || readable.end.checked_add(2) == Some(receipt.written_sectors)),
        "insert the recorded single-session calibration target"
    );
    verify_target_samples(link, readable, directory)?;
    crate::calibrate::digest(directory)
}

fn verify_target_samples(
    link: &mut dyn Link,
    readable: &Range<u32>,
    directory: &Path,
) -> Result<()> {
    use std::io::{Read, Seek, SeekFrom};
    let mut image = fs::File::open(directory.join("calibration.iso"))?;
    for lba in [16, 300, readable.end - 1]
        .into_iter()
        .chain((1..8).map(|n| readable.end * n / 8))
    {
        check_cancel()?;
        let reply = link
            .command(Command::Read {
                lba,
                count: 1,
                fua: false,
                read12: false,
            })?
            .good()?;
        let mut expected = [0; SECTOR_BYTES];
        image.seek(SeekFrom::Start(u64::from(lba) * SECTOR_BYTES as u64))?;
        image.read_exact(&mut expected)?;
        ensure!(
            reply.data == expected,
            "insert the matching calibration target; mismatch at LBA {lba}"
        );
    }
    Ok(())
}

fn save(path: &Path, data: &Value) -> Result<()> {
    fs::write(path, serde_json::to_vec_pretty(data)?)
        .with_context(|| format!("cannot write {}", path.display()))
}

fn check_cancel() -> Result<()> {
    ensure!(!device::cancelled(), "CD timing acquisition cancelled");
    Ok(())
}

fn ready(link: &mut dyn Link, initial: bool) -> Result<()> {
    check_cancel()?;
    let ready = link.command(Command::Ready)?;
    if initial && ready.status != 0 && ready.key() == 6 {
        // The initial open may report one unit-attention event for inserted media.
        link.command(Command::Ready)?.good()?;
    } else {
        ready.good()?;
    }
    Ok(())
}

fn partial_bounds(link: &mut dyn Link, planned: u32, initial: bool) -> Result<Range<u32>> {
    ready(link, initial)?;
    let config = link.command(Command::Configuration)?.good()?.data;
    ensure!(
        matches!(config.get(6..8), Some([0, 9..=10])),
        "expected CD-R or CD-RW"
    );
    let disc = declared(link.command(Command::Disc)?.good()?.data)?;
    let track = declared(link.command(Command::TrackNumber(1))?.good()?.data)?;
    if disc.get(6) == Some(&2) {
        let next = declared(link.command(Command::TrackNumber(2))?.good()?.data)?;
        ensure!(
            next.len() >= 32 && next[2..4] == [2, 1] && next[6] & 0x40 != 0,
            "recovery requires only one recorded track"
        );
    }
    partial_track_bounds(&disc, &track, planned)
}

fn partial_track_bounds(disc: &[u8], track: &[u8], planned: u32) -> Result<Range<u32>> {
    ensure!(
        disc.len() >= 24
            && disc[2] & 3 == 1
            && disc[3] == 1
            && disc[4] == 1
            && disc[5] == 1
            && matches!(disc[6], 1 | 2)
            && disc[9..12] == [0; 3],
        "recovery requires an unfinalized first CD session"
    );
    ensure!(
        track.len() >= 32
            && track[2..4] == [1, 1]
            && (track.len() < 34 || track[32..34] == [0, 0])
            && track[8..12] == [0; 4]
            && track[7] & 1 == 0
            && track[5] & 4 != 0
            && track[6] & 0x4f == 1,
        "recovery requires a closed Mode 1 data track starting at LBA 0"
    );
    let mut end = u32::from_be_bytes(track[24..28].try_into()?);
    ensure!(end < planned, "expected a partially recorded target");
    if track[7] & 2 != 0 {
        let last = u32::from_be_bytes(track[28..32].try_into()?);
        end = end.min(last.saturating_add(1));
    }
    ensure!(end > 1300, "too little recorded data for recovery");
    Ok(0..end)
}

fn readable_prefix(
    link: &mut dyn Link,
    reported: Range<u32>,
    recheck: impl Fn(&mut dyn Link) -> Result<Range<u32>>,
) -> Result<Range<u32>> {
    // A failed write can leave the last reported sectors unreadable. Keep
    // timing reads away from that area and validate the chosen bound.
    for margin in [1024, 4096, 8192, 16384, 32768, 49152, 65536, 98304] {
        check_cancel()?;
        let end = reported.end.saturating_sub(margin);
        if end <= 1300 {
            break;
        }
        let reply = link.command(Command::Read {
            lba: end - 1,
            count: 1,
            fua: false,
            read12: false,
        })?;
        if reply.status == 0 && reply.data.len() == SECTOR_BYTES {
            console::phase(
                "probe",
                format_args!(
                    "Reported track ends at {}; using LBA 0..{end}",
                    reported.end
                ),
            );
            return Ok(0..end);
        }
        if reply.key() == 6 && reply.asc() == (0x29, 0) {
            ensure!(
                recheck(link)? == reported,
                "partial target changed after drive reset"
            );
        } else if matches!(reply.key(), 2 | 4 | 6) {
            reply.good()?;
        }
        console::phase(
            "probe",
            format_args!("LBA {end} boundary could not be read; moving inward"),
        );
    }
    anyhow::bail!("could not establish a readable partial target boundary")
}

fn disc_bounds(link: &mut dyn Link, initial: bool) -> Result<Range<u32>> {
    ready(link, initial)?;
    let config = link.command(Command::Configuration)?.good()?.data;
    ensure!(
        matches!(config.get(6..8), Some([0, 8..=10])),
        "probe requires an inserted CD-ROM, CD-R or CD-RW"
    );
    let cap = link.command(Command::Capacity)?.good()?.data;
    let toc = declared(link.command(Command::Toc)?.good()?.data)?;
    let disc = declared(link.command(Command::Disc)?.good()?.data)?;
    let track = declared(link.command(Command::TrackNumber(1))?.good()?.data)?;
    bounds(&cap, &toc, &disc, &track)
}

fn declared(mut bytes: Vec<u8>) -> Result<Vec<u8>> {
    let header = bytes.first_chunk::<2>().context("short MMC response")?;
    let length = usize::from(u16::from_be_bytes(*header)) + 2;
    ensure!(bytes.len() >= length, "truncated MMC response");
    bytes.truncate(length);
    Ok(bytes)
}

fn bounds(capacity: &[u8], toc: &[u8], disc: &[u8], track: &[u8]) -> Result<Range<u32>> {
    ensure!(
        toc.len() == 20
            && toc[2..4] == [1, 1]
            && toc[5] & 4 != 0
            && toc[6] == 1
            && toc[14] == 0xaa
            && disc.len() >= 24
            && disc[2] & 3 == 2
            && disc[4] == 1
            && disc[9] == 0,
        "probe requires a finalized single-session CD with one data track"
    );
    let be = |bytes: &[u8], i| u32::from_be_bytes(bytes[i..][..4].try_into().unwrap());
    let start = be(toc, 8);
    ensure!(
        capacity.len() == 8
            && be(capacity, 4) == SECTOR_BYTES as u32
            && track.len() >= 32
            && track[2..4] == [1, 1]
            && track.get(32).copied().unwrap_or(0) == 0
            && track.get(33).copied().unwrap_or(0) == 0
            && track[6] & 0x4f == 1
            && be(track, 8) == start,
        "could not confirm a recorded Mode 1 CD track with 2048-byte sectors"
    );
    let mut end = start
        .saturating_add(be(track, 24))
        .min(be(toc, 16))
        .min(be(capacity, 0).saturating_add(1))
        .min(360_000);
    if track[7] & 2 != 0 {
        end = end.min(be(track, 28).saturating_add(1));
    }
    ensure!(
        end.saturating_sub(start) > 1000,
        "no usable recorded Mode 1 track extent"
    );
    Ok(start..end)
}

struct Probe<'a> {
    link: &'a mut dyn Link,
    bounds: Range<u32>,
    reference: u32,
    read12: bool,
    cache: Cache,
    evict_cursor: u32,
    period: Option<f64>,
    random: u32,
}

impl Probe<'_> {
    fn check_read(&self, lba: u32, count: u8) -> Result<()> {
        check_cancel()?;
        ensure!(
            (1..=READ_CHUNK).contains(&count)
                && self.bounds.contains(&lba)
                && lba + u32::from(count) <= self.bounds.end,
            "read at LBA {lba} + {count} sectors lies outside the recorded track or transfer limit"
        );
        Ok(())
    }

    fn timed(&mut self, lba: u32) -> Result<Option<f64>> {
        check_cancel()?;
        // Jitter the command start so repeated reads resolve the spindle's fundamental.
        self.random ^= self.random << 13;
        self.random ^= self.random >> 17;
        self.random ^= self.random << 5;
        // A slow spindle needs jitter spanning a whole revolution to avoid
        // period aliases. The initial unknown-period window remains 50 ms.
        let window = (self.period.unwrap_or(0.04) * 1.25 * 1e6).max(50_000.0) as u64;
        std::thread::sleep(Duration::from_micros(u64::from(self.random) % window));
        self.sample(lba)
    }

    fn check_rotation(&mut self, range: Range<u32>) -> Result<Value> {
        let inner = range.start;
        let outer = range.end - 1;
        let radius_squared = |lba| {
            25.0f64.powi(2)
                + 0.0016 * (16.0 / f64::from(SECTOR_BITS)) * read_end_bits(lba) as f64
                    / std::f64::consts::PI
        };
        let expected = (radius_squared(outer) / radius_squared(inner)).sqrt();
        if expected < 1.1 {
            return Ok(
                json!({"checked":false,"reason":"range too narrow to distinguish CAV from CLV"}),
            );
        }
        console::phase(
            "probe",
            format_args!("Checking inner/outer rotation before the geometry ladder"),
        );
        let mut periods = Vec::new();
        for lba in [inner, outer] {
            let mut times = Vec::new();
            for _ in 0..2 {
                times.clear();
                for _ in 0..24 {
                    times.extend(self.timed(lba)?);
                }
            }
            let Some(fit) = period::estimate(&times, None).filter(|p| p.coherence >= 0.8) else {
                return Ok(json!({"checked":false,"reason":"inconclusive rotation period"}));
            };
            periods.push(fit.seconds);
        }
        let ratio = periods[1] / periods[0];
        Ok(json!({"checked":true,"inner_lba":inner,"outer_lba":outer,
            "period_ms":periods.iter().map(|p| p*1000.0).collect::<Vec<_>>(),
            "ratio":ratio,"expected_clv":expected,"looks_clv":looks_clv(ratio, expected)}))
    }

    fn settle(&mut self) -> Result<()> {
        self.period = None;
        let mut previous = None;
        for round in 1..=10 {
            console::phase(
                "probe",
                format_args!("Settling spindle at LBA {} ({round}/10)", self.reference),
            );
            let mut times = Vec::new();
            for _ in 0..16 {
                times.extend(self.timed(self.reference)?);
            }
            let current = period::estimate(&times, None).filter(|fit| fit.coherence >= 0.95);
            if let (Some(current), Some(last)) = (&current, previous)
                && stable(current, last)
            {
                self.period = Some(current.seconds);
                console::phase(
                    "probe",
                    format_args!("Stable period {:.4} ms", current.seconds * 1000.0),
                );
                return Ok(());
            }
            previous = current.map(|fit| fit.seconds);
        }
        anyhow::bail!(
            "spindle period did not stabilize; this drive or medium may not support constant-rotation timing"
        )
    }

    fn measure(&mut self, lba: u32) -> Result<Option<Sample>> {
        console::phase(
            "probe",
            format_args!("Measuring LBA {lba} ({REPS} read pairs)"),
        );
        for attempt in 0..2 {
            if attempt == 1 {
                self.settle()?;
            }
            let mut reference = Vec::new();
            let mut target = Vec::new();
            for _ in 0..REPS {
                reference.extend(self.timed(self.reference)?);
                target.extend(self.timed(lba)?);
            }
            if let Some(fit) = period::estimate(&reference, self.period)
                && fit.coherence >= 0.93
                && self.period.is_some_and(|last| stable(&fit, last))
                && let Some((phase, spread, used)) = period::phase(&fit, &target)
            {
                self.period = Some(fit.seconds);
                return Ok(Some(Sample {
                    lba,
                    phase,
                    spread,
                    used,
                    period: fit.seconds,
                    sigma: (spread / (used as f64).sqrt()).max(0.2f64.to_radians()),
                }));
            }
        }
        console::warning(format_args!("Skipping LBA {lba}: no stable timing sample"));
        Ok(None)
    }
}

fn stable(period: &Period, previous: f64) -> bool {
    (period.seconds / previous - 1.0).abs() <= PERIOD_TOL
}

fn looks_clv(ratio: f64, expected: f64) -> bool {
    (expected - expected.round()).abs() / expected >= 0.03
        && (ratio - expected).abs() / expected < 0.03
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mmc::Reply;
    use std::f64::consts::{PI, TAU};

    #[test]
    fn target_binding_checks_real_payload_and_never_reads_outside_the_data_track() {
        use std::io::{Read, Seek, SeekFrom};
        struct Reader {
            image: fs::File,
            corrupt: bool,
            reads: Vec<u32>,
        }
        impl Link for Reader {
            fn command(&mut self, command: Command) -> Result<Reply> {
                let Command::Read {
                    lba,
                    count: 1,
                    fua: false,
                    read12: false,
                } = command
                else {
                    panic!("target verification must only issue bounded READ(10)")
                };
                assert!(lba < 1850);
                self.reads.push(lba);
                let mut data = vec![0; SECTOR_BYTES];
                self.image
                    .seek(SeekFrom::Start(u64::from(lba) * SECTOR_BYTES as u64))?;
                self.image.read_exact(&mut data)?;
                if self.corrupt {
                    data[0] ^= 1;
                }
                Ok(Reply {
                    status: 0,
                    sense: [0; 32],
                    data,
                    end: 0.0,
                    duration: 0.01,
                })
            }
        }
        let root =
            std::env::temp_dir().join(format!("discimage-probe-target-{}", std::process::id()));
        let result = (|| -> Result<()> {
            crate::calibrate::prepare_single(&root, 2000, || false)?;
            fs::rename(root.join("pending.json"), root.join("single.json"))?;
            let mut reader = Reader {
                image: fs::File::open(root.join("calibration.iso"))?,
                corrupt: false,
                reads: Vec::new(),
            };
            let hash = verify_target(&mut reader, &(0..1850), &root)?;
            assert_eq!(hash, crate::calibrate::digest(&root)?);
            assert_eq!(reader.reads.len(), 10);
            assert!(verify_target(&mut reader, &(0..1853), &root).is_err());
            assert_eq!(reader.reads.len(), 10);
            reader.corrupt = true;
            assert!(verify_target(&mut reader, &(0..1850), &root).is_err());
            Ok(())
        })();
        fs::remove_dir_all(root).unwrap();
        result.unwrap();
    }

    #[test]
    fn mode1_read_bounds_require_a_finalized_single_data_track() {
        let mut capacity = [0; 8];
        capacity[..4].copy_from_slice(&359_847u32.to_be_bytes());
        capacity[4..].copy_from_slice(&2048u32.to_be_bytes());
        let mut toc = [0; 20];
        toc[2..4].copy_from_slice(&[1, 1]);
        toc[5] = 4;
        toc[6] = 1;
        toc[14] = 0xaa;
        toc[16..20].copy_from_slice(&359_848u32.to_be_bytes());
        let mut disc = [0; 34];
        disc[2] = 2;
        disc[4] = 1;
        let mut track = [0; 48];
        track[2..4].copy_from_slice(&[1, 1]);
        track[6] = 1;
        track[7] = 2;
        track[24..28].copy_from_slice(&359_848u32.to_be_bytes());
        track[28..32].copy_from_slice(&359_693u32.to_be_bytes());
        assert_eq!(bounds(&capacity, &toc, &disc, &track).unwrap(), 0..359_694);
        track[6] = 2;
        assert!(bounds(&capacity, &toc, &disc, &track).is_err());
        track[6] = 1;
        disc[4] = 2;
        assert!(bounds(&capacity, &toc, &disc, &track).is_err());
        disc[4] = 1;
        disc[2] = 1;
        assert!(bounds(&capacity, &toc, &disc, &track).is_err());
        disc[2] = 2;
        toc[5] = 0;
        assert!(bounds(&capacity, &toc, &disc, &track).is_err());
        assert!(declared(vec![0, 18, 1, 1]).is_err());
    }

    #[test]
    fn partial_bounds_use_the_recorded_track_and_reject_other_layouts() {
        let mut disc = [0; 34];
        disc[2..7].copy_from_slice(&[5, 1, 1, 1, 2]);
        let mut track = [0; 36];
        track[1] = 34;
        track[2..4].copy_from_slice(&[1, 1]);
        track[5] = 4;
        track[6] = 1;
        track[24..28].copy_from_slice(&335_232u32.to_be_bytes());
        assert_eq!(
            partial_track_bounds(&disc, &track, 359_697).unwrap(),
            0..335_232
        );
        assert!(partial_track_bounds(&disc, &track, 335_232).is_err());
        track[7] = 1;
        assert!(partial_track_bounds(&disc, &track, 359_697).is_err());
        track[7] = 0;
        disc[4] = 2;
        assert!(partial_track_bounds(&disc, &track, 359_697).is_err());
    }

    #[test]
    fn identity_commands_request_only_standard_read_only_metadata() {
        for (serial, expected) in [
            (false, [0x12, 0, 0, 0, 36, 0]),
            (true, [0x12, 1, 0x80, 0, 255, 0]),
        ] {
            let (cdb, length, transfer) = Command::Inquiry { serial }.packet();
            assert_eq!(&cdb[..6], &expected);
            assert_eq!((length, transfer), (6, usize::from(expected[4])));
            assert!(cdb[6..].iter().all(|&v| v == 0));
        }
        let (cdb, length, transfer) = Command::DriveSerial.packet();
        assert_eq!(&cdb[..10], &[0x46, 2, 1, 8, 0, 0, 0, 2, 0, 0]);
        assert_eq!((length, transfer), (10, 512));
        assert!(cdb[10..].iter().all(|&v| v == 0));
    }

    #[test]
    fn cache_commands_encode_reads_and_no_data_at_the_absolute_lba() {
        for (read12, op, length) in [(true, 0xa8, 12), (false, 0x28, 10)] {
            for count in [1, 32] {
                let (cdb, size, transfer) = Command::Read {
                    lba: 120_520,
                    count,
                    fua: true,
                    read12,
                }
                .packet();
                assert_eq!(
                    (cdb[0], cdb[1], size, transfer),
                    (op, 8, length, usize::from(count) * 2048)
                );
                assert_eq!(cdb[2..6], 120_520u32.to_be_bytes());
                assert_eq!(cdb[if read12 { 9 } else { 8 }], count);
            }
        }
        let (cdb, size, transfer) = Command::Flush(120_520).packet();
        assert_eq!((cdb[0], cdb[1], size, transfer), (0xa8, 8, 12, 0));
        assert_eq!(cdb[2..6], 120_520u32.to_be_bytes());
        assert_eq!(cdb[6..10], [0; 4]);
        let (cdb, size, transfer) = Command::Speed.packet();
        assert_eq!(
            (&cdb[..6], size, transfer),
            (&[0xbb, 0, 0x05, 0x83, 0xff, 0xff][..], 12, 0)
        );
    }

    #[test]
    fn cache_check_uses_legacy_order_and_bounds_eviction() {
        struct Reader {
            expected: Cache,
            flushed: bool,
            evicted: u32,
            reads: u32,
            next_evict: u32,
            cached_duration: f64,
        }
        impl Link for Reader {
            fn command(&mut self, command: Command) -> Result<Reply> {
                let (count, duration) = match command {
                    Command::Flush(lba) => {
                        assert!((100..1200).contains(&lba));
                        self.flushed = true;
                        (0, 0.0002)
                    }
                    Command::Read {
                        lba, count, fua, ..
                    } => {
                        assert!(100 <= lba && lba + u32::from(count) <= 1200);
                        self.reads += 1;
                        if count != 1 {
                            assert!(!fua && count <= 32);
                            assert_eq!(lba, self.next_evict);
                            self.evicted += u32::from(count);
                            self.next_evict = if lba + u32::from(count) == 1200 {
                                759
                            } else {
                                lba + u32::from(count)
                            };
                        }
                        let media = match self.expected {
                            Cache::Plain => true,
                            Cache::Fua => fua,
                            Cache::Flush => std::mem::take(&mut self.flushed),
                            Cache::Evict => self.evicted == 256,
                        };
                        (count, if media { 0.024 } else { self.cached_duration })
                    }
                    _ => panic!("unexpected command during cache test"),
                };
                Ok(Reply {
                    data: vec![0; usize::from(count) * 2048],
                    status: 0,
                    sense: [0; 32],
                    end: 1.0,
                    duration,
                })
            }
        }
        for (expected, cached_duration) in [
            (Cache::Plain, 0.0002),
            (Cache::Fua, 0.0002),
            (Cache::Fua, 0.008),
            (Cache::Flush, 0.0002),
            (Cache::Evict, 0.0002),
        ] {
            let mut reader = Reader {
                expected,
                flushed: false,
                evicted: 0,
                reads: 0,
                next_evict: 1179,
                cached_duration,
            };
            let mut probe = Probe {
                link: &mut reader,
                bounds: 100..1200,
                reference: 650,
                read12: true,
                cache: Cache::Plain,
                evict_cursor: 420,
                period: None,
                random: 1,
            };
            assert!(probe.check_read(1199, 2).is_err());
            assert!(probe.check_read(1200, 1).is_err());
            let report = probe.check_cache().unwrap();
            assert_eq!(probe.cache, expected);
            assert_eq!(report["plain"]["n"], 40);
            assert_eq!(report["fua"]["n"], 40);
            assert_eq!(probe.sample(650).unwrap(), Some(1.0));
            assert_eq!(
                reader.evicted,
                if expected == Cache::Evict { 256 } else { 0 }
            );
            let after_flush = matches!(expected, Cache::Flush | Cache::Evict);
            assert_eq!(report["after_flush"]["n"] == 40, after_flush);
            assert!(reader.reads >= if after_flush { 121 } else { 81 });
        }
    }

    #[test]
    fn read10_fallback_requires_a_refused_opcode_and_never_hides_media_errors() {
        struct Refused {
            key: u8,
            commands: Vec<u8>,
        }
        impl Link for Refused {
            fn command(&mut self, command: Command) -> Result<Reply> {
                let (cdb, _, length) = command.packet();
                self.commands.push(cdb[0]);
                let refused = self.commands.len() == 1;
                let mut sense = [0; 32];
                if refused {
                    sense[0] = 0x70;
                    sense[2] = self.key;
                    sense[12] = if self.key == 5 { 0x20 } else { 0x11 };
                }
                Ok(Reply {
                    data: vec![0; length],
                    status: if refused { 2 } else { 0 },
                    sense,
                    end: 1.0,
                    duration: 0.03,
                })
            }
        }
        for key in [3, 5] {
            let mut link = Refused {
                key,
                commands: Vec::new(),
            };
            let mut probe = Probe {
                link: &mut link,
                bounds: 0..10000,
                reference: 5000,
                read12: true,
                cache: Cache::Plain,
                evict_cursor: 0,
                period: None,
                random: 1,
            };
            let result = probe.setup_reads();
            assert_eq!(result.is_ok(), key == 5);
            assert_eq!(probe.read12, key != 5);
            if key == 3 {
                assert_eq!(link.commands, [0xa8]);
            } else {
                assert_eq!(&link.commands[..3], [0xa8, 0x28, 0xbb]);
            }
        }
    }

    #[test]
    fn clv_screen_distinguishes_constant_rotation_and_whole_turn_aliases() {
        assert!(looks_clv(1.82, 1.8));
        assert!(!looks_clv(1.0, 1.8));
        assert!(!looks_clv(2.0, 2.01));
    }

    #[test]
    fn synthetic_timestamps_recover_period_and_multi_turn_geometry() {
        let period = 0.02870227;
        let reference = 180_470;
        let (u, v) = (89_452.674_f64, 5.403173_f64);
        let n0 = read_end_bits(reference) as f64;
        let radius = |n: f64| (u * u + v * n / PI).sqrt();
        let period_samples: Vec<_> = [0, 2, 5, 8, 10, 11, 15, 17, 20, 22, 23, 26, 28, 31, 35, 38]
            .into_iter()
            .map(|turn| 2000.0 + f64::from(turn) * period)
            .collect();
        let rotation = period::estimate(&period_samples, None).unwrap();
        assert!((rotation.seconds - period).abs() < 1e-7);
        let measure = |lba| {
            let n = read_end_bits(lba) as f64;
            let phase = (2.0 * (n - n0) / (radius(n) + radius(n0)) + 0.08).rem_euclid(TAU);
            let times: Vec<_> = (0..32)
                .map(|i| {
                    rotation.origin
                        + (f64::from(i * 3) + phase / TAU) * period
                        + f64::from(i % 5 - 2) * 2e-6
                })
                .collect();
            let (phase, spread, used) = period::phase(&rotation, &times).unwrap();
            Ok(Some(Sample {
                lba,
                phase,
                spread,
                used,
                period,
                sigma: 0.2_f64.to_radians(),
            }))
        };
        let result = fit::ladder(1500..359_440, reference, false, measure).unwrap();
        assert_eq!(result.points.len(), 36);
        assert!((result.geometry.u - u).abs() < 0.1);
        assert!((result.geometry.v - v).abs() < 1e-4);
        assert!(result.rms_deg < 0.05);
        assert!(result.span_sigma_deg(1500, 359_439) < 0.5);

        let mut attempts = 0;
        let outlier = result.points[6].sample.lba;
        let recovered = fit::ladder(1500..359_440, reference, true, |lba| {
            let sample = measure(lba)?;
            Ok(sample.map(|mut sample| {
                if lba == outlier {
                    sample.phase += PI;
                    attempts += 1;
                }
                sample
            }))
        })
        .unwrap();
        assert_eq!(attempts, 2);
        assert_eq!(recovered.rejected, [outlier]);
        assert_eq!(recovered.points.len(), 36);
        assert!((recovered.geometry.u - u).abs() < 0.1);
        assert!((recovered.geometry.v - v).abs() < 1e-4);
    }
}
