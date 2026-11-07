//! CLI defaults. Recording jobs keep their own frozen device binding.
use std::{
    fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use discimage::device::{self, Device};

fn config() -> Result<PathBuf> {
    let env = |name| {
        std::env::var_os(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    #[cfg(windows)]
    let root = env("APPDATA").context("APPDATA is unavailable")?;
    #[cfg(target_os = "macos")]
    let root = env("HOME")
        .context("HOME is unavailable")?
        .join("Library/Application Support");
    #[cfg(not(any(windows, target_os = "macos")))]
    let root = env("XDG_CONFIG_HOME")
        .or_else(|| env("HOME").map(|home| home.join(".config")))
        .context("XDG_CONFIG_HOME and HOME are unavailable")?;
    ensure!(
        root.is_absolute(),
        "configuration directory must be absolute"
    );
    Ok(root.join("discimage/config.toml"))
}

fn read(path: &Path) -> Result<toml::Table> {
    match fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).with_context(|| format!("invalid {}", path.display())),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(toml::Table::new()),
        Err(error) => Err(error).with_context(|| format!("cannot read {}", path.display())),
    }
}

fn preferred_at(explicit: Option<&str>, path: &Path) -> Result<Option<String>> {
    if let Some(explicit) = explicit {
        return Ok(Some(explicit.into()));
    }
    read(path)?
        .get("device")
        .map(|value| {
            value
                .as_str()
                .filter(|value| !value.trim().is_empty())
                .map(String::from)
                .with_context(|| format!("device must be a nonempty string in {}", path.display()))
        })
        .transpose()
}

/// Explicit flags do not read or change the saved default.
pub fn preferred(explicit: Option<&str>) -> Result<Option<String>> {
    if let Some(explicit) = explicit {
        return Ok(Some(explicit.into()));
    }
    let selected = preferred_at(None, &config()?)?;
    if let Some(selected) = &selected {
        discimage::console::phase("device", format_args!("Using default drive {selected}"));
    }
    Ok(selected)
}

fn save(path: &Path, selector: Option<&str>) -> Result<()> {
    let mut values = read(path)?;
    match selector {
        Some(selector) => {
            values.insert("device".into(), selector.into());
        }
        None => {
            values.remove("device");
        }
    }
    if values.is_empty() {
        if path.try_exists()? {
            fs::remove_file(path)?;
        }
        return Ok(());
    }
    let parent = path
        .parent()
        .context("configuration has no parent directory")?;
    fs::create_dir_all(parent)?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let temporary = parent.join(format!(".config-{}-{nonce}.tmp", std::process::id()));
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(toml::to_string_pretty(&values)?.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.with_context(|| format!("cannot save {}", path.display()))
}

fn select_at(path: &Path, drives: &[Device], selector: &str) -> Result<String> {
    let selected = device::resolve(drives, selector)?.selector();
    save(path, Some(&selected))?;
    Ok(selected)
}

pub fn run(selector: Option<&str>, clear: bool) -> Result<()> {
    let path = config()?;
    if clear {
        save(&path, None)?;
        println!("Default drive cleared.");
    } else if let Some(selector) = selector {
        let selected = select_at(&path, &device::list()?, selector)?;
        println!("Default drive: {selected}");
    } else if let Some(selected) = preferred_at(None, &path)? {
        println!("Default drive: {selected}");
    } else {
        println!("No default drive selected.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use discimage::device::Identity;

    #[test]
    fn selection_persists_identity_and_explicit_flags_override_without_changing_it() {
        let root = std::env::temp_dir().join(format!("discimage-selection-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let path = root.join("config.toml");
        let mut drive = Device {
            hardware: Some(Identity {
                vendor: "HL-DT-ST".into(),
                product: "DVDRW GX40N".into(),
                serial: "K12345".into(),
            }),
            id: "native-port".into(),
            name: "HL-DT-ST DVDRW GX40N".into(),
            backend: "test".into(),
            capabilities: None,
            media: None,
            tracks: vec![],
        };
        let key = select_at(&path, std::slice::from_ref(&drive), "GX40N").unwrap();
        assert_eq!(key, "9c9be5068b6f");
        assert_eq!(
            preferred_at(None, &path).unwrap().as_deref(),
            Some(key.as_str())
        );
        assert_eq!(
            preferred_at(Some("other-drive"), &path).unwrap().as_deref(),
            Some("other-drive")
        );
        assert!(device::resolve(&[], &preferred_at(None, &path).unwrap().unwrap()).is_err());
        let mut other = drive.clone();
        other.hardware.as_mut().unwrap().serial = "K99999".into();
        assert!(device::resolve(&[other], &key).is_err());
        let before = fs::read(&path).unwrap();
        assert!(select_at(&path, &[drive.clone(), drive.clone()], "GX40N").is_err());
        assert!(select_at(&path, &[], "missing").is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        drive.hardware = None;
        assert!(
            select_at(&path, &[drive], "GX40N")
                .unwrap()
                .starts_with("local-")
        );
        fs::write(&path, "device = 'stale'\nother = true\n").unwrap();
        save(&path, None).unwrap();
        assert_eq!(preferred_at(None, &path).unwrap(), None);
        assert_eq!(read(&path).unwrap()["other"].as_bool(), Some(true));
        fs::write(&path, "invalid = [").unwrap();
        assert!(preferred_at(None, &path).is_err());
        assert_eq!(
            preferred_at(Some("override"), &path).unwrap().as_deref(),
            Some("override")
        );
        fs::write(&path, "device = 1").unwrap();
        assert!(preferred_at(None, &path).is_err());
        fs::write(&path, "device = 'old'").unwrap();
        save(&path, None).unwrap();
        assert!(!path.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
