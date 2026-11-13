//! macOS optical discovery and exclusive IOKit MMC transport.
use crate::{
    console,
    device::{self, Device},
    probe::transport::Reply,
    recorder::Transport,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{
    ffi::{CStr, CString, c_char, c_void},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

unsafe extern "C" {
    fn discimage_macos_query(json: *const c_char, cancelled: extern "C" fn() -> i32)
    -> *mut c_char;
    fn discimage_free(value: *mut c_char);
    fn discimage_record_open(path: *const c_char, error: *mut i32) -> *mut c_void;
    fn discimage_probe_close(handle: *mut c_void);
    fn discimage_record_command(
        handle: *mut c_void,
        cdb: *mut u8,
        length: u8,
        data: *mut u8,
        capacity: u32,
        write: bool,
        timeout: u32,
        used: *mut u32,
        status: *mut u8,
        sense: *mut u8,
        times: *mut u64,
    ) -> i32;
}

extern "C" fn cancelled() -> i32 {
    i32::from(device::cancelled())
}

fn query<T: DeserializeOwned>(value: Value) -> Result<T> {
    let input = CString::new(serde_json::to_vec(&value)?)?;
    // The result is owned by this call and released before returning.
    let output = unsafe { discimage_macos_query(input.as_ptr(), cancelled) };
    ensure!(
        !output.is_null(),
        "native optical metadata query returned no response"
    );
    let result = unsafe { serde_json::from_slice::<Value>(CStr::from_ptr(output).to_bytes()) };
    unsafe { discimage_free(output) };
    let result = result.context("invalid native optical metadata")?;
    if let Some(error) = result.get("error").and_then(Value::as_str) {
        anyhow::bail!("{error}");
    }
    serde_json::from_value(result).context("native optical metadata schema mismatch")
}

fn identify(drive: &mut Device) {
    if let Some(identity) = crate::probe::hardware_identity(&drive.id) {
        drive.hardware = Some(identity);
    }
}

fn snapshot(id: &str) -> Result<Device> {
    let mut drive = query(json!({"op":"probe","device":id}))?;
    identify(&mut drive);
    Ok(drive)
}

pub(crate) fn devices() -> Result<Vec<Device>> {
    #[derive(Deserialize)]
    struct Devices {
        devices: Vec<Device>,
    }
    let mut drives = query::<Devices>(json!({"op":"list"}))?.devices;
    for drive in &mut drives {
        identify(drive);
    }
    Ok(drives)
}

#[derive(Deserialize, PartialEq)]
struct Node {
    bsd_name: Option<String>,
    media_id: u64,
}

// Unmount only the selected disc. Recheck its I/O Registry identity before
// opening the exclusive user client; never unmount a replacement volume.
fn unmount(id: &str, op: &str) -> Result<Node> {
    let request = json!({"op":op,"device":id});
    let before: Node = query(request.clone())?;
    if let Some(name) = &before.bsd_name {
        let bsd = device::whole_bsd_name(name).context("invalid optical whole-disc BSD name")?;
        console::phase(
            "drive",
            format_args!("Unmounting selected optical disc ({bsd})"),
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
                anyhow::bail!("optical unmount cancelled or timed out");
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        ensure!(
            status.success(),
            "could not unmount selected optical disc {bsd}; close applications using it"
        );
    }
    let after: Node = query(request)?;
    ensure!(
        before == after,
        "selected optical medium changed while unmounting"
    );
    Ok(after)
}

pub(crate) fn prepare_probe(id: &str) -> Result<()> {
    unmount(id, "probe_node")?;
    Ok(())
}

pub(crate) struct RecordingLink {
    id: CString,
    handle: *mut c_void,
    medium: Option<Device>,
}

impl RecordingLink {
    pub(crate) fn new(id: &str) -> Result<Self> {
        ensure!(
            id.starts_with("IOService:"),
            "expected a macOS optical registry path"
        );
        Ok(Self {
            id: CString::new(id)?,
            handle: std::ptr::null_mut(),
            medium: None,
        })
    }
}

impl Transport for RecordingLink {
    fn snapshot(&self) -> Result<Option<Device>> {
        snapshot(self.id.to_str()?).map(Some)
    }

    fn requires_reservation_for_reads(&self) -> bool {
        true
    }

    fn reserve(&mut self) -> Result<()> {
        if !self.handle.is_null() {
            return Ok(());
        }
        let id = self.id.to_str()?;
        let before = unmount(id, "media_node")?;
        let medium = snapshot(id)?;
        let after: Node = query(json!({"op":"media_node","device":id}))?;
        ensure!(
            before == after,
            "selected optical medium changed before reservation"
        );
        ensure!(
            !device::cancelled(),
            "recording cancelled before reservation"
        );
        let mut error = 0;
        // IOKit resolves the exact optical service and acquires exclusive access.
        let handle = unsafe { discimage_record_open(self.id.as_ptr(), &mut error) };
        ensure!(
            !handle.is_null(),
            "cannot reserve selected optical drive (IOKit 0x{:08x}); close other disc applications",
            error as u32
        );
        self.handle = handle;
        self.medium = Some(medium);
        Ok(())
    }

    fn confirm_reservation(&mut self, actual: &Device) -> Result<()> {
        // IOKit removes the IOCDMedia node while the device is exclusively open.
        // Compare its last snapshot with fresh MMC state under the tray lock.
        let before = self
            .medium
            .as_ref()
            .context("no optical medium before locking")?;
        let expected = before
            .media
            .as_ref()
            .context("no optical medium before locking")?;
        let media = actual
            .media
            .as_ref()
            .context("no optical medium after locking")?;
        ensure!(
            before.id == actual.id
                && before
                    .hardware_id()
                    .is_none_or(|id| actual.hardware_id().as_ref() == Some(&id))
                && media.kind == expected.kind
                && media.blank == expected.blank
                && if media.blank {
                    media.free_sectors == expected.free_sectors
                } else {
                    media.appendable == expected.appendable
                        && media.sessions == expected.sessions
                        && media.tracks == expected.tracks
                },
            "selected optical medium changed during reservation"
        );
        self.medium = None;
        Ok(())
    }

    fn execute(
        &mut self,
        cdb: &[u8],
        bytes: &mut [u8],
        write: bool,
        timeout: u32,
    ) -> Result<Reply> {
        ensure!(
            !self.handle.is_null() && (6..=16).contains(&cdb.len()) && bytes.len() <= 65_536,
            "invalid MMC transfer or no exclusive optical reservation"
        );
        let mut command = [0; 16];
        command[..cdb.len()].copy_from_slice(cdb);
        let (mut used, mut status, mut sense, mut times) = (0, 0, [0; 32], [0; 2]);
        // All buffers remain valid until the synchronous command returns.
        let error = unsafe {
            discimage_record_command(
                self.handle,
                command.as_mut_ptr(),
                cdb.len() as u8,
                bytes.as_mut_ptr(),
                bytes.len() as u32,
                write,
                timeout,
                &mut used,
                &mut status,
                sense.as_mut_ptr(),
                times.as_mut_ptr(),
            )
        };
        ensure!(
            error == 0,
            "macOS IOKit command {:02x} failed (0x{:08x}); command was not retried",
            cdb[0],
            error as u32
        );
        ensure!(
            used as usize <= bytes.len() && times[1] >= times[0],
            "invalid MMC response"
        );
        Ok(Reply {
            data: bytes[..used as usize].to_vec(),
            status,
            sense,
            end: times[1] as f64 * 1e-9,
            duration: (times[1] - times[0]) as f64 * 1e-9,
        })
    }
}

impl Drop for RecordingLink {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { discimage_probe_close(self.handle) };
        }
    }
}
