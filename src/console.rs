//! Line-oriented diagnostics. Command results and JSON stay on stdout.
use std::{
    fmt::Arguments,
    io::{self, IsTerminal, Write},
};

/// Human-readable discovery results. IDs are wrapped without dropping characters;
/// JSON remains the copyable machine-readable representation.
pub fn devices(devices: &[crate::device::Device]) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let yes = |value| if value { "yes" } else { "no" };
    let number = |value: Option<u32>| value.map_or_else(|| "unknown".into(), |n| n.to_string());
    if devices.is_empty() {
        return "No optical drives reported.\n".into();
    }
    for (index, drive) in devices.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        let _ = writeln!(out, "Drive {}: {}", index + 1, drive.name);
        let _ = writeln!(
            out,
            "  ID            {}",
            drive
                .hardware_id()
                .as_deref()
                .unwrap_or("unavailable (no reliable drive serial)")
        );
        let _ = writeln!(
            out,
            "  Connection    {} (this OS only)",
            drive.connection_selector()
        );
        let _ = writeln!(out, "  Backend       {}", drive.backend);
        if let Some(media) = &drive.media {
            let state = if media.blank {
                "blank"
            } else if media.appendable {
                "appendable"
            } else {
                "closed (no further sessions)"
            };
            let kind = match media.kind.as_str() {
                "cd-r" => "CD-R",
                "cd-rw" => "CD-RW",
                other => other,
            };
            let _ = writeln!(out, "  Disc          {kind} / {state}");
            let _ = writeln!(
                out,
                "  Contents      {} sessions / {} tracks",
                media.sessions, media.tracks
            );
            let _ = writeln!(out, "  Blank         {}", yes(media.blank));
            let _ = writeln!(out, "  Appendable    {}", yes(media.appendable));
            let _ = writeln!(out, "  Reserved      {}", yes(media.reserved));
            let _ = writeln!(
                out,
                "  Free space    {} sectors",
                number(media.free_sectors)
            );
            let _ = writeln!(out, "  Next write    {} (LBA)", number(media.nwa));
            if !media.blank && !media.appendable && media.kind == "cd-rw" {
                out.push_str(
                    "  Reuse         Run 'discimage erase' only to replace this disc's contents.\n",
                );
            }
        } else {
            let status = if drive.backend == "macos-discrecording" {
                "no ready media"
            } else {
                "not reported by this backend"
            };
            let _ = writeln!(out, "  Disc          {status}");
        }
        out.push_str("\n  Write capabilities\n");
        if let Some(c) = &drive.capabilities {
            for (name, supported) in [
                ("CD-R", c.cdr),
                ("CD-RW", c.cdrw),
                ("SAO", c.sao),
                ("TAO", c.tao),
                ("CD-Text", c.cd_text),
            ] {
                let _ = writeln!(
                    out,
                    "    {name:<10} {}",
                    if supported {
                        "supported"
                    } else {
                        "not supported"
                    }
                );
            }
        } else {
            out.push_str("    Not reported by this backend\n");
        }
        if !drive.tracks.is_empty() {
            out.push_str("\n  Tracks (addresses in LBA, lengths in sectors)\n");
            out.push_str("    Track Session Type     Start     Length Closed Empty Next write\n");
            for track in &drive.tracks {
                let start = track
                    .start
                    .map_or_else(|| "unknown".into(), |n| n.to_string());
                let _ = writeln!(
                    out,
                    "    {:>5} {:>7} {:<5} {:>9} {:>10} {:>6} {:>5} {}",
                    track.number,
                    track.session,
                    if track.audio { "audio" } else { "data" },
                    start,
                    number(track.sectors),
                    yes(track.closed),
                    yes(track.empty),
                    number(track.next_writable)
                );
            }
        } else {
            out.push_str("\n  Track details not reported.\n");
        }
        out.push_str("\n  Device ID (wrapped; use devices --json to copy the exact value)\n");
        // Count characters instead of bytes so non-ASCII device names stay valid.
        for chunk in drive.id.chars().collect::<Vec<_>>().chunks(76) {
            out.push_str("    ");
            out.extend(chunk);
            out.push('\n');
        }
    }
    out
}

pub fn phase(label: &str, message: Arguments<'_>) {
    line(label, 36, message);
}

pub fn speed(speed: Option<u32>) -> String {
    speed.map_or_else(
        || "auto (drive-selected maximum)".into(),
        |n| format!("requested {n}x"),
    )
}

pub fn success(message: Arguments<'_>) {
    line("done", 32, message);
}

pub fn warning(message: Arguments<'_>) {
    line("warning", 33, message);
}

pub fn error(message: Arguments<'_>) {
    line("error", 31, message);
}

fn line(label: &str, color: u8, message: Arguments<'_>) {
    let mut stderr = io::stderr().lock();
    // Basic ANSI colors are supported by Terminal.app. Keep redirected logs
    // and consoles without advertised ANSI support readable as plain text.
    let use_color = cfg!(unix)
        && stderr.is_terminal()
        && std::env::var_os("NO_COLOR").is_none_or(|value| value.is_empty())
        && std::env::var_os("TERM").is_some_and(|value| !value.is_empty() && value != "dumb");
    if use_color {
        let _ = writeln!(stderr, "\x1b[{color}m[{label}]\x1b[0m {message}");
    } else {
        let _ = writeln!(stderr, "[{label}] {message}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{Capabilities, Device, Media, Track};

    #[test]
    fn discovery_keeps_full_ids_and_distinguishes_unknown_from_zero() {
        let id = format!("IOService:/{}", "光驱/segment/".repeat(30));
        let mut drive = Device {
            hardware: None,
            id: id.clone(),
            name: "Test drive".into(),
            backend: "macos-discrecording".into(),
            capabilities: Some(Capabilities {
                cdr: true,
                cdrw: true,
                sao: true,
                tao: false,
                cd_text: false,
            }),
            media: Some(Media {
                kind: "cd-rw".into(),
                blank: false,
                appendable: false,
                reserved: true,
                free_sectors: Some(0),
                nwa: None,
                sessions: 2,
                tracks: 7,
            }),
            tracks: vec![Track {
                number: 7,
                session: 2,
                start: Some(-150),
                sectors: Some(12345),
                audio: false,
                closed: true,
                empty: false,
                next_writable: Some(0),
            }],
        };
        let output = devices(std::slice::from_ref(&drive));
        let (_, wrapped) = output.split_once("exact value)\n").unwrap();
        assert_eq!(
            wrapped
                .lines()
                .map(|line| line.strip_prefix("    ").unwrap())
                .collect::<String>(),
            id
        );
        for expected in [
            "CD-RW / closed",
            "2 sessions / 7 tracks",
            "Reserved      yes",
            "Free space    0 sectors",
            "Next write    unknown",
            "Run 'discimage erase'",
            "not supported",
            "-150",
            "12345",
        ] {
            assert!(output.contains(expected), "missing {expected}");
        }
        assert!(
            output
                .lines()
                .any(|line| line.split_whitespace().collect::<Vec<_>>()
                    == ["7", "2", "data", "-150", "12345", "yes", "no", "0"])
        );
        drive.media.as_mut().unwrap().free_sectors = None;
        assert!(devices(&[drive]).contains("Free space    unknown sectors"));
    }

    #[test]
    fn discovery_does_not_invent_missing_backend_information() {
        let drive = Device {
            hardware: None,
            id: "/dev/sr0".into(),
            name: "Drive".into(),
            backend: "linux-sysfs".into(),
            capabilities: None,
            media: None,
            tracks: Vec::new(),
        };
        let output = devices(std::slice::from_ref(&drive));
        assert!(output.contains("ID            unavailable"));
        assert!(output.contains(&format!("Connection    {}", drive.connection_selector())));
        let mut identified = drive;
        identified.hardware = Some(crate::device::Identity {
            vendor: "HL-DT-ST".into(),
            product: "DVDRW GX40N".into(),
            serial: "K12345".into(),
        });
        let identified = devices(&[identified]);
        assert!(identified.contains("ID            9c9be5068b6f"));
        assert!(!identified.contains("cd-9c9be5068b6f"));
        assert!(output.contains("not reported by this backend"));
        assert!(!output.contains("no ready media"));
        assert_eq!(devices(&[]), "No optical drives reported.\n");
    }
}
