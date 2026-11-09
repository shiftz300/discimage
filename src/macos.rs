//! DiscRecording bridge with owned UTF-8 JSON and one opaque session.
use crate::{
    audio,
    codec::{COOKED, MODE1_PAYLOAD, RAW},
    console,
    device::{self, Device, Music},
    project::CdText,
};
use anyhow::{Context, Result, ensure};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::{
    ffi::{CStr, CString, c_char, c_void},
    fs::File,
    io::{Read, Seek, SeekFrom},
    os::unix::fs::FileExt,
    path::Path,
    ptr,
    time::{Duration, Instant},
};

unsafe extern "C" {
    fn discimage_call(
        handle: *mut *mut c_void,
        json: *const c_char,
        cancelled: extern "C" fn() -> i32,
        progress: extern "C" fn(*const c_char),
    ) -> *mut c_char;
    fn discimage_free(value: *mut c_char);
}
extern "C" fn cancelled() -> i32 {
    i32::from(device::cancelled())
}

extern "C" fn progress(message: *const c_char) {
    // The bridge supplies a NUL-terminated string valid until this callback returns.
    if !message.is_null() {
        let message = unsafe { CStr::from_ptr(message) }.to_string_lossy();
        console::phase("drive", format_args!("{message}"));
    }
}

pub struct Recorder {
    handle: *mut c_void,
    id: String,
}
impl Recorder {
    pub fn new(id: &str) -> Self {
        Self {
            handle: ptr::null_mut(),
            id: id.into(),
        }
    }
    fn request<T: DeserializeOwned>(&mut self, value: Value) -> Result<T> {
        let input = CString::new(serde_json::to_vec(&value)?)?;
        // The bridge borrows the request during this call. It retains the opaque
        // session until close, and returns a malloc-owned string, even on errors.
        let output =
            unsafe { discimage_call(&mut self.handle, input.as_ptr(), cancelled, progress) };
        ensure!(!output.is_null(), "native recorder returned no response");
        let bytes = unsafe { CStr::from_ptr(output).to_bytes().to_vec() };
        unsafe { discimage_free(output) };
        let result: Value =
            serde_json::from_slice(&bytes).context("invalid native recording response")?;
        if let Some(error) = result.get("error").and_then(Value::as_str) {
            anyhow::bail!("{error}");
        }
        serde_json::from_value(result).context("native response schema mismatch")
    }
    pub fn reserve_blank_cdr(id: Option<&str>, capacity: u32) -> Result<Self> {
        let mut recorder = Self::new("");
        let mut request = json!({"op":"blank_cdr_open","capacity":capacity});
        if let Some(id) = id {
            request["device"] = json!(id);
        }
        let _: Value = recorder.request(request)?;
        Ok(recorder)
    }
    fn raw_media(&mut self, finalized: bool) -> Result<File> {
        #[derive(serde::Deserialize)]
        struct MediaNode {
            bsd_name: String,
        }
        let op = if finalized {
            "closed_media_node"
        } else {
            "media_node"
        };
        let media: MediaNode = self.request(json!({"op":op}))?;
        let name = device::whole_bsd_name(&media.bsd_name)
            .with_context(|| format!("invalid CD device name: {}", media.bsd_name))?;
        let path = format!("/dev/r{name}");
        File::open(&path).with_context(|| format!("cannot open {path} for CD readback"))
    }
    fn verify_mode1_tail(&mut self, path: &Path, start: u32, sectors: u32) -> Result<()> {
        let disc = self.raw_media(true)?;
        let iso = File::open(path)?;
        for relative in [16, sectors - 2, sectors - 1] {
            let lba = start + relative;
            let mut raw = [0; RAW];
            let mut expected = [0; COOKED];
            disc.read_exact_at(&mut raw, u64::from(lba) * RAW as u64)?;
            iso.read_exact_at(&mut expected, u64::from(relative) * COOKED as u64)?;
            ensure!(
                raw[15] == 1 && raw[MODE1_PAYLOAD] == expected,
                "Mode 1 readback mismatch at LBA {lba}"
            );
        }
        Ok(())
    }
}
impl Drop for Recorder {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            let _: Result<Value> = self.request(json!({"op":"close"}));
        }
    }
}

/// Release only the selected optical filesystem before the target's read-only probe.
/// diskutil is a macOS system tool; no forced unmount, eject or general disk scan.
pub(crate) fn prepare_probe(id: &str) -> Result<()> {
    use std::process::{Command, Stdio};
    #[derive(serde::Deserialize, PartialEq)]
    struct Node {
        bsd_name: String,
        media_id: u64,
    }
    let mut recorder = Recorder::new("");
    let before: Node = recorder.request(json!({"op":"probe_node", "device":id}))?;
    let bsd =
        device::whole_bsd_name(&before.bsd_name).context("invalid optical whole-disc BSD name")?;
    console::phase(
        "probe",
        format_args!(
            "Unmounting only the selected optical volume ({bsd}) for exclusive read access"
        ),
    );
    let mut child = Command::new("/usr/sbin/diskutil")
        .args(["unmountDisk", bsd])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if device::cancelled() || started.elapsed() > Duration::from_secs(20) {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("optical unmount cancelled or timed out; no measurement started");
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    ensure!(
        status.success(),
        "could not unmount selected optical volume {bsd}; close applications using that disc"
    );
    let after: Node = recorder.request(json!({"op":"probe_node", "device":id}))?;
    ensure!(
        before == after,
        "selected optical medium changed while unmounting"
    );
    Ok(())
}

pub fn erase(id: &str, quick: bool, confirm: impl FnOnce(&Device) -> Result<bool>) -> Result<()> {
    let mut recorder = Recorder::new(id);
    // Reserve and identify this physical media before asking for confirmation.
    let selected: Device = recorder.request(json!({"op":"erase_open", "device":id}))?;
    if !confirm(&selected)? {
        println!("Erase cancelled. No disc contents changed.");
        return Ok(());
    }
    ensure!(!device::cancelled(), "erase cancelled before starting");
    console::phase(
        "erase",
        format_args!(
            "{} erase of {}",
            if quick { "Quick" } else { "Full" },
            selected.name
        ),
    );
    let after: Device = recorder.request(json!({"op":"erase", "quick":quick}))?;
    ensure!(
        after.id == id
            && after.media.as_ref().is_some_and(|m| m.kind == "cd-rw"
                && m.blank
                && m.sessions == 0
                && m.tracks == 0
                && m.free_sectors.is_some_and(|n| n > 0))
            && after.tracks.iter().all(|t| t.empty),
        "erase finished but blank CD-RW state was not confirmed; inspect devices before recording"
    );
    console::success(format_args!(
        "CD-RW erased; blank media and writable capacity confirmed. Ready for prepare."
    ));
    Ok(())
}

pub fn record_calibration(
    directory: &Path,
    id: Option<&str>,
    speed: Option<u32>,
    dry_run: bool,
    confirm: impl FnOnce(&Device) -> Result<bool>,
) -> Result<()> {
    #[derive(serde::Deserialize)]
    struct Reserved {
        capacity: u32,
        drive: Device,
    }
    device::install_cancel_handler()?;
    let mut recorder = Recorder::new("");
    let mut request = json!({"op":"calibration_open"});
    if let Some(id) = id {
        request["device"] = json!(id);
    }
    let reserved: Reserved = recorder.request(request)?;
    console::phase(
        "calibration",
        format_args!(
            "Reserved {} in {} ({} available sectors)",
            reserved
                .drive
                .media
                .as_ref()
                .map_or("CD", |m| m.kind.as_str()),
            reserved.drive.name,
            reserved.capacity
        ),
    );
    if dry_run {
        println!("Calibration preflight passed. No disc write or output files.");
        return Ok(());
    }
    if !confirm(&reserved.drive)? {
        println!("Calibration paused. No files created or disc write attempted.");
        return Ok(());
    }
    let count = crate::calibrate::prepare_single(directory, reserved.capacity, device::cancelled)?;
    let path = std::fs::canonicalize(directory.join("calibration.iso"))?;
    let mut request = json!({"op":"calibration_write","path":&path,"sectors":count});
    if let Some(speed) = speed {
        request["speed"] = json!(speed);
    }
    console::phase(
        "calibration",
        format_args!("Writing and verifying the single-session Mode 1 target"),
    );
    let _: Value = recorder.request(request)?;
    let start = Instant::now();
    let short = loop {
        let observed = device::Recorder::inspect(&mut recorder)?;
        ensure!(
            observed.id == reserved.drive.id,
            "recorded drive identity changed"
        );
        if observed.media.as_ref().is_some_and(|m| {
            matches!(m.kind.as_str(), "cd-r" | "cd-rw" | "cd-rom")
                && !m.blank
                && !m.appendable
                && m.sessions == 1
                && m.tracks == 1
        }) && let [track] = observed.tracks.as_slice()
            && track.number == 1
            && track.session == 1
            && !track.audio
            && track.closed
            && !track.empty
            && track.start == Some(0)
            && let Some(reported) = track.sectors
            && (reported == count || reported.checked_add(2) == Some(count))
        {
            break reported != count;
        }
        ensure!(
            start.elapsed() < Duration::from_secs(20),
            "could not confirm the finalized calibration track at LBA 0; inspect the disc before retrying"
        );
        std::thread::sleep(Duration::from_millis(250));
    };
    if short {
        // Some drives exclude two readable sectors from the reported track size.
        recorder.verify_mode1_tail(&path, 0, count)?;
    }
    crate::calibrate::complete_single(directory, &reserved.drive, speed)?;
    println!(
        "Verified one finalized Mode 1 track at LBA 0..{count}. Target: {}",
        directory.display()
    );
    Ok(())
}

impl device::Recorder for Recorder {
    fn inspect(&mut self) -> Result<Device> {
        self.request(if self.handle.is_null() {
            json!({"op":"probe","device":self.id})
        } else {
            json!({"op":"status"})
        })
    }
    fn lock(&mut self) -> Result<()> {
        if self.handle.is_null() {
            let _: Value = self.request(json!({"op":"open","device":self.id}))?;
        }
        Ok(())
    }
    fn music_gap(&self, drive: &Device, requested: u32) -> u32 {
        if drive.name.starts_with("Apple SuperDrive ")
            && drive.media.as_ref().is_some_and(|m| m.kind == "cd-rw")
        {
            requested.max(150)
        } else {
            requested
        }
    }
    fn audio_sectors(&mut self, path: &Path) -> Result<Option<u32>> {
        #[derive(serde::Deserialize)]
        struct Length {
            sectors: u32,
        }
        let length: Length = self
            .request(json!({"op":"audio_length","path":path}))
            .with_context(|| format!("cannot preflight WAV {}", path.display()))?;
        Ok(Some(length.sectors))
    }
    fn preflight_cd_text(&mut self, title: &str, text: &CdText) -> Result<()> {
        let _: Value = self.request(json!({"op":"cd_text_check","title":title,"cd_text":text}))?;
        Ok(())
    }
    fn music(
        &mut self,
        tracks: &[Music],
        speed: Option<u32>,
        cd_text: Option<(&str, &CdText)>,
    ) -> Result<()> {
        let mut request = json!({"op":"audio","tracks":tracks,"speed":speed});
        if let Some((title, text)) = cd_text {
            request["title"] = json!(title);
            request["cd_text"] = json!(text);
        }
        let _: Value = self.request(request)?;
        Ok(())
    }
    fn verify_music(&mut self, tracks: &[Music]) -> Result<()> {
        verify_pcm(&self.raw_media(false)?, tracks)
    }
    fn verify_data_tail(&mut self, path: &Path, start: u32, sectors: u32) -> Result<()> {
        self.verify_mode1_tail(path, start, sectors)
    }
    fn data(&mut self, path: &Path, start: u32, sectors: u32, speed: Option<u32>) -> Result<()> {
        let _: Value = self.request(
            json!({"op":"data","path":path,"start":start,"sectors":sectors,"speed":speed}),
        )?;
        Ok(())
    }
}

fn verify_pcm(disc: &File, tracks: &[Music]) -> Result<()> {
    const CHUNK_SECTORS: u32 = 128;
    let mut actual = vec![0; CHUNK_SECTORS as usize * RAW];
    let mut expected = actual.clone();
    let start = Instant::now();
    console::phase(
        "readback",
        format_args!("Comparing the music session with the prepared WAVs"),
    );
    let ranges = audio::track_ranges(tracks.iter().map(|track| (track.sectors, track.gap)));
    for (i, (track, range)) in tracks.iter().zip(ranges).enumerate() {
        let mut lba = range.start;
        let number = i + 1;
        let source = audio::inspect(&track.path)?;
        ensure!(
            source.sectors == track.sectors && source.padding_sample_frames == 0,
            "normalized WAV for track {number} has changed"
        );
        let mut wav = File::open(&track.path)?;
        wav.seek(SeekFrom::Start(source.data_offset))?;
        let mut reported = Instant::now();
        console::phase(
            "readback",
            format_args!(
                "Track {number}/{} | {} sectors",
                tracks.len(),
                track.sectors
            ),
        );
        let mut left = track.sectors;
        while left > 0 {
            ensure!(!device::cancelled(), "audio readback cancelled");
            let count = left.min(CHUNK_SECTORS);
            let len = count as usize * RAW;
            wav.read_exact(&mut expected[..len])?;
            disc.read_exact_at(&mut actual[..len], u64::from(lba) * RAW as u64)
                .with_context(|| format!("cannot read track {number} at LBA {lba}"))?;
            let (actual_sectors, _) = actual[..len].as_chunks::<RAW>();
            let (expected_sectors, _) = expected[..len].as_chunks::<RAW>();
            if let Some(offset) = actual_sectors
                .iter()
                .zip(expected_sectors)
                .position(|(a, b)| a != b)
            {
                anyhow::bail!(
                    "PCM differs from the WAV in track {number} at LBA {}",
                    lba + offset as u32
                );
            }
            left -= count;
            lba += count;
            if reported.elapsed() >= Duration::from_secs(15) {
                console::phase(
                    "readback",
                    format_args!(
                        "Track {number}/{} | {}/{} sectors | {}s elapsed",
                        tracks.len(),
                        track.sectors - left,
                        track.sectors,
                        start.elapsed().as_secs()
                    ),
                );
                reported = Instant::now();
            }
        }
    }
    console::success(format_args!(
        "All {} audio tracks matched byte-for-byte",
        tracks.len()
    ));
    Ok(())
}

pub fn devices() -> Result<Vec<Device>> {
    let value: Value = Recorder::new("").request(json!({"op":"list"}))?;
    let mut drives: Vec<Device> = serde_json::from_value(value["devices"].clone())?;
    for drive in &mut drives {
        if let Some(identity) = crate::probe::hardware_identity(&drive.id) {
            drive.hardware = Some(identity);
        }
    }
    Ok(drives)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_readback_skips_pregaps_and_reports_the_first_mismatching_sector() {
        let root = std::env::temp_dir().join(format!("discimage-pcm-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("track.wav");
        audio::silence(&path).unwrap();
        let track = |gap| Music {
            path: path.clone(),
            sectors: 300,
            gap,
        };
        let tracks = [track(150), track(75)];
        let disc = File::create(root.join("disc.raw")).unwrap();
        disc.set_len(675 * 2352).unwrap();
        disc.write_all_at(&[1; 75 * 2352], 300 * 2352).unwrap();
        let raw = File::open(root.join("disc.raw")).unwrap();
        verify_pcm(&raw, &tracks).unwrap();
        disc.write_all_at(&[1], 674 * 2352).unwrap();
        let error = verify_pcm(&raw, &tracks).unwrap_err();
        assert!(error.to_string().contains("track 2 at LBA 674"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn real_framework_opens_iso_and_audio_and_reads_nested_track_metadata() {
        let root = std::env::temp_dir().join(format!("discimage-native-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("data.iso");
        let mut bytes = Vec::new();
        for i in 0..4 {
            bytes.extend([i as u8; 2048]);
        }
        std::fs::write(&path, bytes).unwrap();
        let wav = root.join("audio.wav");
        let mut audio = Vec::new();
        let sectors = 10_732u32;
        let length = sectors * 2352;
        audio.extend(b"RIFF");
        audio.extend((length + 36).to_le_bytes());
        audio.extend(b"WAVEfmt ");
        audio.extend(16u32.to_le_bytes());
        audio.extend(1u16.to_le_bytes());
        audio.extend(2u16.to_le_bytes());
        audio.extend(44100u32.to_le_bytes());
        audio.extend(176400u32.to_le_bytes());
        audio.extend(4u16.to_le_bytes());
        audio.extend(16u16.to_le_bytes());
        audio.extend(b"data");
        audio.extend(length.to_le_bytes());
        std::fs::write(&wav, audio).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&wav)
            .unwrap()
            .set_len(44 + u64::from(length))
            .unwrap();
        let blank = json!({
            "media": {"kind":"cd-r","blank":true,"sessions":0,"free_sectors":333_000},
            "capabilities": {"cdr":true,"sao":true},
            "tracks": []
        });
        let mut used = blank.clone();
        used["media"]["blank"] = json!(false);
        let mut rewritable = blank.clone();
        rewritable["media"]["kind"] = json!("cd-rw");
        rewritable["capabilities"]["cdrw"] = json!(true);
        let mut unsupported_rw = rewritable.clone();
        unsupported_rw["capabilities"]["cdrw"] = json!(false);
        let mut unknown_capacity = blank.clone();
        unknown_capacity["media"]["free_sectors"] = Value::Null;
        let mut recorded_track = blank.clone();
        recorded_track["tracks"] = json!([{"empty":false}]);
        let response: Value = Recorder::new("")
            .request(json!({
                "op":"selftest","path":path,"audio":wav,"audio_sectors":sectors,
                "erase_candidates":[
                    {"media":{"kind":"cd-rw"},"capabilities":{"cdrw":true}},
                    {"media":{"kind":"cd-r"},"capabilities":{"cdrw":true}},
                    {"media":null,"capabilities":{"cdrw":true}},
                    {"media":{"kind":"cd-rw"},"capabilities":{"cdrw":false}}
                ],
                "calibration_candidates":[blank,used,rewritable,unknown_capacity,recorded_track,unsupported_rw]
            }))
            .unwrap();
        assert_eq!(
            response["erase_candidates"],
            json!([true, false, false, false])
        );
        assert_eq!(response["automatic_speed"], true);
        assert_eq!(response["manual_speed"], true);
        assert_eq!(response["image_sectors"], 4);
        assert_eq!(response["image_mode1"], true);
        assert_eq!(
            response["calibration_capacities"],
            json!([333_000, null, null, null, null, null])
        );
        assert_eq!(
            response["single_capacities"],
            json!([333_000, null, 333_000, null, null, null])
        );
        assert_eq!(response["audio_sectors"], sectors);
        assert_eq!(response["nwa"], 11850);
        assert_eq!(response["audio_track"], true);
        assert_eq!(response.get("closed_track_nwa"), Some(&Value::Null));
        assert_eq!(response["status_percent"], json!([-1, -1, 37]));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn disc_recording_accepts_cd_text_without_truncation() {
        let mut recorder = Recorder::new("");
        let response: Value = recorder
            .request(json!({
                "op": "cd_text_check",
                "title": "Album",
                "cd_text": {"artist": "Artist", "titles": ["Track one", "Track two"]}
            }))
            .unwrap();
        assert_eq!(response["entries"], 3);
        let error = recorder
            .request::<Value>(json!({
                "op": "cd_text_check",
                "title": "Album",
                "cd_text": {"artist": "Artist", "titles": ["x".repeat(4096)]}
            }))
            .unwrap_err();
        assert!(error.to_string().contains("too long"));
    }
}
