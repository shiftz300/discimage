//! Device descriptions and the recording boundary shared by platform backends.
mod identity;
use crate::project::CdText;
#[cfg(not(any(target_os = "macos", windows)))]
use anyhow::bail;
use anyhow::{Context, Result};
pub use identity::Identity;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

static CANCELLED: AtomicBool = AtomicBool::new(false);
pub fn cancelled() -> bool {
    CANCELLED.load(Ordering::Relaxed)
}
pub fn install_cancel_handler() -> Result<()> {
    static HANDLER: std::sync::OnceLock<std::result::Result<(), String>> =
        std::sync::OnceLock::new();
    HANDLER
        .get_or_init(|| {
            ctrlc::set_handler(|| CANCELLED.store(true, Ordering::Relaxed))
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|error| {
            anyhow::anyhow!("cannot install optical operation cancellation handler: {error}")
        })?;
    Ok(())
}

#[cfg(target_os = "macos")]
pub(crate) fn whole_bsd_name(id: &str) -> Option<&str> {
    let name = id.strip_prefix("/dev/").unwrap_or(id);
    name.strip_prefix("disk")
        .filter(|n| !n.is_empty() && n.len() <= 10 && n.bytes().all(|b| b.is_ascii_digit()))
        .map(|_| name)
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hardware: Option<Identity>,
    pub id: String,
    pub name: String,
    pub backend: String,
    pub capabilities: Option<Capabilities>,
    pub media: Option<Media>,
    pub tracks: Vec<Track>,
}

impl Device {
    pub fn hardware_id(&self) -> Option<String> {
        self.hardware.as_ref()?.id()
    }
    /// Native-path hashes are explicitly local, never portable hardware IDs.
    pub fn connection_selector(&self) -> String {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(self.id.as_bytes());
        format!("local-{}", crate::hex(&digest[..5]))
    }
    pub fn selector(&self) -> String {
        self.hardware_id()
            .unwrap_or_else(|| self.connection_selector())
    }
}

/// Resolve before filtering by media eligibility: a model shared by two drives
/// remains ambiguous even if only one currently contains writable media.
pub fn resolve<'a>(drives: &'a [Device], selector: &str) -> Result<&'a Device> {
    let selector = selector.trim();
    anyhow::ensure!(!selector.is_empty(), "device selector must not be empty");
    let short = selector.to_ascii_lowercase();
    // Keep accepting the earlier spelling; only the bare hardware ID is displayed.
    let hardware = short.strip_prefix("cd-").unwrap_or(&short);
    let is_hardware = hardware.len() == 12 && hardware.bytes().all(|c| c.is_ascii_hexdigit());
    anyhow::ensure!(
        is_hardware || !selector.bytes().all(|c| c.is_ascii_digit()),
        "display numbers are not device selectors; use a selector or unique model from devices"
    );
    let exact: Vec<_> = drives
        .iter()
        .filter(|d| d.id == selector || (cfg!(windows) && d.id.eq_ignore_ascii_case(selector)))
        .collect();
    if let [drive] = exact.as_slice() {
        return Ok(drive);
    }
    anyhow::ensure!(
        exact.is_empty(),
        "duplicate native device IDs; cannot safely select a drive"
    );
    let name = |value: &str| {
        value
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|s| !s.is_empty())
            .map(str::to_ascii_lowercase)
            .collect::<Vec<_>>()
            .join(" ")
    };
    let requested = format!(" {} ", name(selector));
    let matches: Vec<_> = drives
        .iter()
        .filter(|drive| {
            if is_hardware || short.starts_with("cd-") {
                drive.hardware_id().as_deref() == Some(hardware)
            } else if short.starts_with("local-") {
                drive.connection_selector() == short
            } else {
                !requested.trim().is_empty()
                    && format!(" {} ", name(&drive.name)).contains(&requested)
            }
        })
        .collect();
    match matches.as_slice() {
        [drive] => Ok(drive),
        [] => anyhow::bail!(
            "no connected optical drive matches {selector:?}; run 'discimage devices' for current selectors"
        ),
        _ => anyhow::bail!(
            "device selector {selector:?} is ambiguous: {}. Use an exact selector from devices or native ID",
            matches
                .iter()
                .map(|d| format!("{} ({})", d.selector(), d.name))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

pub fn resolve_id(selector: &str) -> Result<String> {
    Ok(resolve(&list()?, selector)?.id.clone())
}

/// Preserve explicitly supplied legacy probe addresses, which the native
/// transports independently validate as whole optical devices.
pub(crate) fn probe_id(selector: &str) -> Result<String> {
    #[cfg(target_os = "macos")]
    if let Some(bsd) = whole_bsd_name(selector) {
        return Ok(bsd.into());
    }
    #[cfg(windows)]
    {
        let bytes = selector.as_bytes();
        if bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
            return resolve_id(&selector.to_ascii_uppercase());
        }
    }
    resolve_id(selector)
}
#[derive(Debug, Clone, Serialize)]
pub struct Music {
    pub path: PathBuf,
    pub sectors: u32,
    pub gap: u32,
    pub tail: u32,
}

/// `music` closes the audio session and leaves the disc appendable.
/// `verify_music` checks PCM, silence and Q subchannel indexes before and after `data`
/// writes, verifies and finalizes the disc.
/// The recorder owns its media reservation until dropped.
pub trait Recorder {
    /// Observe the drive and media without changing the write mode.
    fn inspect(&mut self) -> Result<Device>;
    /// Configure the reserved appendable disc for data writing and report its NWA.
    fn prepare_append(&mut self) -> Result<Device> {
        self.inspect()
    }
    fn preflight_speed(&mut self, _speed: Option<u32>) -> Result<()> {
        Ok(())
    }
    fn lock(&mut self) -> Result<()>;
    fn preflight_cd_text(&mut self, _title: &str, _text: &CdText) -> Result<()> {
        anyhow::bail!("this recording backend does not support CD-Text")
    }
    fn music(
        &mut self,
        tracks: &[Music],
        speed: Option<u32>,
        cd_text: Option<(&str, &CdText)>,
    ) -> Result<()>;
    fn verify_music(&mut self, tracks: &[Music], finalized: bool) -> Result<()>;
    fn data(&mut self, path: &Path, start: u32, sectors: u32, speed: Option<u32>) -> Result<()>;
    fn verify_data_tail(&mut self, _path: &Path, _start: u32, _sectors: u32) -> Result<()> {
        anyhow::bail!("the recorder cannot verify unreported data tail sectors")
    }
    fn cancelled(&self) -> bool {
        cancelled()
    }
}

pub fn recorder(id: &str) -> Result<Box<dyn Recorder>> {
    #[cfg(any(target_os = "macos", windows))]
    {
        Ok(Box::new(crate::recorder::Recorder::new(id)?))
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = id;
        bail!(
            "Recording is implemented for macOS and Windows; offline authoring remains cross-platform"
        )
    }
}

/// Erase selection must never broaden to other media or silently pick one drive.
pub fn select_erasable<'a>(drives: &'a [Device], id: Option<&str>) -> Result<&'a Device> {
    let selected = id.map(|id| resolve(drives, id)).transpose()?;
    let eligible = |d: &&Device| {
        selected.is_none_or(|selected| d.id == selected.id)
            && d.capabilities.as_ref().is_some_and(|c| c.cdrw)
            && d.media
                .as_ref()
                .is_some_and(|m| m.kind == "cd-rw" && !m.reserved)
    };
    let mut candidates = drives.iter().filter(eligible);
    let selected = candidates.next().context(
        "no available CD-RW in the selected drive; CD-R and other media cannot be erased",
    )?;
    anyhow::ensure!(
        candidates.next().is_none(),
        "multiple CD-RW drives; specify --device with a selector from devices"
    );
    Ok(selected)
}

pub fn erase(
    id: Option<&str>,
    quick: bool,
    dry_run: bool,
    confirm: impl FnOnce(&Device) -> Result<bool>,
) -> Result<()> {
    #[cfg(any(target_os = "macos", windows))]
    {
        let drives = list()?;
        let selected = select_erasable(&drives, id)?;
        if dry_run {
            println!(
                "{}",
                crate::console::devices(std::slice::from_ref(selected))
            );
            println!("Erase preflight passed. No media reservation or erase performed.");
            return Ok(());
        }
        install_cancel_handler()?;
        crate::recorder::erase(selected, quick, confirm)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = (id, quick, dry_run, confirm);
        bail!("CD-RW erasing is implemented for macOS and Windows")
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
                let mut drive = discovery(
                    format!("/dev/{}", entry.file_name().to_string_lossy()),
                    name.trim().into(),
                    "linux-sysfs",
                );
                drive.hardware = identity::linux(&entry.path().join("device"));
                devices.push(drive);
            }
        }
        devices.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(devices)
    }
    #[cfg(target_os = "windows")]
    {
        crate::windows::devices()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        bail!("Device discovery supports macOS, Windows and Linux")
    }
}

#[cfg(target_os = "linux")]
fn discovery(id: String, name: String, backend: &str) -> Device {
    Device {
        hardware: None,
        id,
        name,
        backend: backend.into(),
        capabilities: None,
        media: None,
        tracks: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disc(id: &str, kind: &str) -> Device {
        Device {
            hardware: None,
            id: id.into(),
            name: id.into(),
            backend: "test".into(),
            capabilities: Some(Capabilities {
                cdr: true,
                cdrw: true,
                sao: false,
                tao: false,
                cd_text: false,
            }),
            media: Some(Media {
                kind: kind.into(),
                blank: false,
                appendable: false,
                reserved: false,
                free_sectors: Some(0),
                nwa: None,
                sessions: 2,
                tracks: 7,
            }),
            tracks: Vec::new(),
        }
    }

    #[test]
    fn selectors_have_one_format_across_backends_and_accept_unique_models() {
        for (backend, id) in [
            ("macos-iokit-mmc", "IOService:/USB/Optical"),
            ("linux-sysfs", "/dev/sr0"),
            ("windows-imapi2-mmc", "E:"),
        ] {
            let mut drive = disc(id, "cd-rw");
            drive.backend = backend.into();
            drive.name = "HL-DT-ST DVDRW GX40N (RQ00)".into();
            let selector = drive.selector();
            assert!(selector.starts_with("local-"));
            assert_eq!(selector.len(), 16);
            let drives = [drive];
            for input in [selector.as_str(), "GX40N", "dvdrw gx40n", id] {
                assert_eq!(resolve(&drives, input).unwrap().id, id);
            }
            for input in ["1", "GX40", "", "cd-0000000000"] {
                assert!(resolve(&drives, input).is_err());
            }
        }
    }

    #[test]
    fn duplicate_models_and_stale_connections_never_fall_back_to_another_drive() {
        let mut a = disc("/port/a", "cd-rw");
        let mut b = disc("/port/b", "cd-r");
        a.name = "HL-DT-ST DVDRW GX40N (RQ00)".into();
        b.name = a.name.clone();
        let key = a.selector();
        let drives = [b.clone(), a.clone()];
        assert!(resolve(&drives, "gx40n").is_err());
        assert!(select_erasable(&drives, Some("gx40n")).is_err());
        assert_eq!(resolve(&drives, &key).unwrap().id, a.id);
        assert_eq!(resolve(&[a.clone(), b.clone()], &key).unwrap().id, a.id);
        assert!(resolve(&[b], &key).is_err());
        assert!(resolve(&[a.clone(), a], &key).is_err());
    }

    #[test]
    fn same_hardware_gets_the_same_id_across_os_paths_and_rejects_duplicate_serials() {
        let identity = Identity {
            vendor: "HL-DT-ST".into(),
            product: "DVDRW GX40N".into(),
            serial: "KABX128713".into(),
        };
        let expected = identity.id().unwrap();
        for id in [
            "IOService:/USB/Port1/DVD",
            "IOService:/USB/Port2/DVD",
            "/dev/sr0",
            "/dev/sr3",
            "E:",
            "F:",
        ] {
            let mut drive = disc(id, "cd-rw");
            drive.hardware = Some(identity.clone());
            assert_eq!(drive.selector(), expected);
            let drives = [drive];
            for key in [
                expected.clone(),
                expected.to_ascii_uppercase(),
                format!("cd-{expected}"),
            ] {
                assert_eq!(resolve(&drives, &key).unwrap().id, id);
            }
        }
        let mut a = disc("port-a", "cd-rw");
        let mut b = disc("port-b", "cd-rw");
        a.hardware = Some(identity.clone());
        b.hardware = Some(identity);
        assert!(resolve(&[a, b], &expected).is_err());
        // A hardware digest can contain only decimal digits; it is not a display index.
        let mut numeric = disc("/dev/sr0", "cd-rw");
        numeric.hardware = Some(Identity {
            vendor: "HL-DT-ST".into(),
            product: "DVDRW GX40N".into(),
            serial: "K000003".into(),
        });
        assert_eq!(numeric.hardware_id().as_deref(), Some("022886750101"));
        assert_eq!(resolve(&[numeric], "022886750101").unwrap().id, "/dev/sr0");
    }

    #[test]
    fn erase_selection_rejects_non_rewritable_missing_and_reserved_media() {
        let mut candidates = vec![disc("a", "cd-r")];
        for kind in ["cd-r", "cd-rom", "dvd-rw", "unknown"] {
            candidates[0].media.as_mut().unwrap().kind = kind.into();
            assert!(select_erasable(&candidates, None).is_err());
        }
        candidates[0] = disc("a", "cd-rw");
        candidates[0].media.as_mut().unwrap().reserved = true;
        assert!(select_erasable(&candidates, None).is_err());
        candidates[0].media = None;
        assert!(select_erasable(&candidates, None).is_err());
        candidates[0] = disc("a", "cd-rw");
        candidates[0].capabilities.as_mut().unwrap().cdrw = false;
        assert!(select_erasable(&candidates, None).is_err());
        candidates[0].capabilities = None;
        assert!(select_erasable(&candidates, None).is_err());
        assert!(select_erasable(&[], None).is_err());
    }

    #[test]
    fn erase_selection_requires_unique_target_and_exact_requested_id() {
        let candidates = vec![disc("a", "cd-rw"), disc("b", "cd-rw"), disc("c", "cd-r")];
        assert!(select_erasable(&candidates, None).is_err());
        assert_eq!(select_erasable(&candidates, Some("b")).unwrap().id, "b");
        assert!(select_erasable(&candidates, Some("c")).is_err());
        assert!(select_erasable(&candidates, Some("missing")).is_err());
        assert_eq!(select_erasable(&candidates[1..], None).unwrap().id, "b");
    }
}
