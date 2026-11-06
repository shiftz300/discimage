//! Device descriptions and the recording boundary shared by platform backends.
use crate::project::CdText;
#[cfg(not(target_os = "macos"))]
use anyhow::bail;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

static CANCELLED: AtomicBool = AtomicBool::new(false);
pub fn cancelled() -> bool {
    CANCELLED.load(Ordering::Relaxed)
}
pub fn install_cancel_handler() -> Result<()> {
    ctrlc::set_handler(|| CANCELLED.store(true, Ordering::Relaxed))
        .context("cannot install recording cancellation handler")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capabilities {
    pub cdr: bool,
    #[serde(default)]
    pub cdrw: bool,
    pub sao: bool,
    pub tao: bool,
    #[serde(default)]
    pub cd_text: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Media {
    pub kind: String,
    pub blank: bool,
    pub appendable: bool,
    pub reserved: bool,
    pub free_sectors: Option<u32>,
    pub nwa: Option<u32>,
    pub sessions: u32,
    pub tracks: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub number: u32,
    pub session: u32,
    pub start: Option<i64>,
    pub sectors: Option<u32>,
    pub audio: bool,
    pub closed: bool,
    pub empty: bool,
    pub next_writable: Option<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub backend: String,
    pub capabilities: Option<Capabilities>,
    pub media: Option<Media>,
    pub tracks: Vec<Track>,
}
#[derive(Debug, Clone, Serialize)]
pub struct Music {
    pub path: PathBuf,
    pub sectors: u32,
    pub gap: u32,
}

/// `music` closes the audio session and leaves the disc appendable.
/// `verify_music` confirms its content before `data` writes, verifies and finalizes.
/// The recorder owns its media reservation until dropped.
pub trait Recorder {
    fn inspect(&mut self) -> Result<Device>;
    fn music_gap(&self, _drive: &Device, requested: u32) -> u32 {
        requested
    }
    fn audio_sectors(&mut self, _path: &Path) -> Result<Option<u32>> {
        Ok(None)
    }
    fn lock(&mut self) -> Result<()>;
    fn preflight_cd_text(&mut self, _title: &str, _text: &CdText) -> Result<()> {
        anyhow::bail!("this recording backend does not support CD-Text")
    }
    fn music(
        &mut self,
        tracks: &[Music],
        speed: u32,
        cd_text: Option<(&str, &CdText)>,
    ) -> Result<()>;
    fn verify_music(&mut self, tracks: &[Music]) -> Result<()>;
    fn data(&mut self, path: &Path, start: u32, sectors: u32, speed: u32) -> Result<()>;
    fn verify_data_tail(&mut self, _path: &Path, _start: u32, _sectors: u32) -> Result<()> {
        anyhow::bail!("the recorder cannot verify unreported data tail sectors")
    }
    fn cancelled(&self) -> bool {
        cancelled()
    }
}

pub fn blank_cdr_recorder(id: Option<&str>, capacity: u32) -> Result<Box<dyn Recorder>> {
    #[cfg(target_os = "macos")]
    {
        Ok(Box::new(crate::macos::Recorder::reserve_blank_cdr(
            id, capacity,
        )?))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (id, capacity);
        bail!("Recording is implemented for macOS only")
    }
}

pub fn recorder(id: &str) -> Result<Box<dyn Recorder>> {
    #[cfg(target_os = "macos")]
    {
        Ok(Box::new(crate::macos::Recorder::new(id)))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = id;
        bail!("Recording is implemented for macOS only; offline authoring remains cross-platform")
    }
}

pub fn list() -> Result<Vec<Device>> {
    #[cfg(target_os = "macos")]
    {
        crate::macos::devices()
    }
    #[cfg(target_os = "linux")]
    {
        let mut devices = Vec::new();
        for entry in
            std::fs::read_dir("/sys/class/block").context("cannot read block-device sysfs")?
        {
            let entry = entry?;
            if std::fs::read_to_string(entry.path().join("device/type"))
                .is_ok_and(|s| s.trim() == "5")
            {
                let name =
                    std::fs::read_to_string(entry.path().join("device/model")).unwrap_or_default();
                devices.push(discovery(
                    format!("/dev/{}", entry.file_name().to_string_lossy()),
                    name.trim().into(),
                    "linux-sysfs",
                ));
            }
        }
        devices.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(devices)
    }
    #[cfg(target_os = "windows")]
    {
        let output=std::process::Command::new("powershell.exe").args(["-NoProfile","-NonInteractive","-Command",
            "$ErrorActionPreference='Stop'; [Console]::OutputEncoding=[System.Text.UTF8Encoding]::new($false); @(Get-CimInstance Win32_CDROMDrive | Select-Object DeviceID,Name) | ConvertTo-Json -Compress"]).output()?;
        anyhow::ensure!(
            output.status.success(),
            "device query failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        if output.stdout.iter().all(u8::is_ascii_whitespace) {
            return Ok(Vec::new());
        }
        let value: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        let rows = match value {
            serde_json::Value::Array(v) => v,
            serde_json::Value::Null => Vec::new(),
            v => vec![v],
        };
        Ok(rows
            .iter()
            .map(|r| {
                discovery(
                    r["DeviceID"].as_str().unwrap_or("").into(),
                    r["Name"].as_str().unwrap_or("").into(),
                    "windows-cim",
                )
            })
            .collect())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        bail!("Device discovery supports macOS, Windows and Linux")
    }
}

#[cfg(not(target_os = "macos"))]
fn discovery(id: String, name: String, backend: &str) -> Device {
    Device {
        id,
        name,
        backend: backend.into(),
        capabilities: None,
        media: None,
        tracks: Vec::new(),
    }
}
