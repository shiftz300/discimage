//! Bounded CD MMC encodings shared by the native recorders and offline tests.
//! SAO/CUE and CD-Text follow MMC and the cdrecord reference (see NOTICE.md).
use crate::{
    audio,
    device::{Capabilities, Media, Music, Track},
    project::CdText,
};
use anyhow::{Context, Result, ensure};

/// Apple's uninitialized USB bridge identity, before the real optical mechanism
/// is powered. It must not be saved as the mechanism's hardware identity.
pub(crate) fn apple_superdrive_proxy(inquiry: &[u8]) -> bool {
    let field = |bytes: &[u8], expected: &str| {
        std::str::from_utf8(bytes).is_ok_and(|s| {
            s.trim_matches(|c| c == ' ' || c == '\0')
                .eq_ignore_ascii_case(expected)
        })
    };
    inquiry.len() >= 36
        && inquiry[0] & 31 == 5
        && field(&inquiry[8..16], "Apple")
        && field(&inquiry[16..32], "SuperDrive")
}

pub(crate) fn be32(bytes: &[u8], offset: usize) -> Result<u32> {
    Ok(u32::from_be_bytes(
        bytes
            .get(offset..offset + 4)
            .context("short MMC response")?
            .try_into()?,
    ))
}

pub(crate) fn declared(bytes: &[u8], minimum: usize) -> Result<&[u8]> {
    ensure!(bytes.len() >= 2, "short MMC response");
    let length = usize::from(u16::from_be_bytes([bytes[0], bytes[1]])) + 2;
    ensure!(
        length >= minimum && bytes.len() >= length,
        "truncated MMC response"
    );
    Ok(&bytes[..length])
}

pub(crate) fn configuration(bytes: &[u8]) -> Result<(u16, Capabilities)> {
    let end = (be32(bytes, 0)? as usize)
        .checked_add(4)
        .context("invalid configuration length")?;
    ensure!(
        bytes.len() >= end && end >= 8,
        "truncated GET CONFIGURATION response"
    );
    let mut caps = Capabilities {
        cdr: false,
        cdrw: false,
        sao: false,
        tao: false,
        cd_text: false,
    };
    let mut offset = 8;
    while offset < end {
        ensure!(offset + 4 <= end, "truncated feature header");
        let code = u16::from_be_bytes([bytes[offset], bytes[offset + 1]]);
        let next = offset + 4 + usize::from(bytes[offset + 3]);
        ensure!(next <= end, "truncated feature descriptor");
        let data = &bytes[offset + 4..next];
        match code {
            0 => {
                ensure!(data.len().is_multiple_of(4), "invalid profile list");
                for profile in data.as_chunks::<4>().0 {
                    match u16::from_be_bytes([profile[0], profile[1]]) {
                        9 => caps.cdr = true,
                        10 => caps.cdrw = true,
                        _ => {}
                    }
                }
            }
            0x2d if data.len() >= 4 => caps.tao = true,
            0x2e if data.len() >= 4 => {
                caps.sao = data[0] & 0x20 != 0;
                caps.cd_text = data[0] & 1 != 0;
            }
            _ => {}
        }
        offset = next;
    }
    Ok((u16::from_be_bytes([bytes[6], bytes[7]]), caps))
}

pub(crate) fn write_speeds(bytes: &[u8]) -> Result<Vec<u32>> {
    let end = (be32(bytes, 0)? as usize)
        .checked_add(4)
        .context("invalid performance length")?;
    ensure!(
        end >= 8 && end <= bytes.len() && (end - 8).is_multiple_of(16),
        "truncated write-speed descriptors"
    );
    let mut speeds = Vec::new();
    for descriptor in bytes[8..end].as_chunks::<16>().0 {
        let kbps = be32(descriptor, 12)?;
        let sectors = (u64::from(kbps) * 1000 + 1176) / 2352;
        if (75..=4800).contains(&sectors) {
            speeds.push(sectors as u32);
        }
    }
    speeds.sort_unstable();
    speeds.dedup();
    Ok(speeds)
}

pub(crate) fn media(profile: u16, bytes: &[u8], reserved: bool) -> Result<Media> {
    let bytes = declared(bytes, 24)?;
    let blank = bytes[2] & 3 == 0;
    let sessions = u32::from(bytes[4]) | (u32::from(bytes[9]) << 8);
    let tracks = u32::from(bytes[6]) | (u32::from(bytes[11]) << 8);
    ensure!(
        tracks <= 99 && sessions <= 99,
        "invalid CD session or track count"
    );
    Ok(Media {
        kind: match profile {
            8 => "cd-rom",
            9 => "cd-r",
            10 => "cd-rw",
            _ => "other",
        }
        .into(),
        blank,
        appendable: bytes[2] & 3 == 1,
        reserved,
        free_sectors: None,
        nwa: None,
        sessions: if blank { 0 } else { sessions },
        tracks: if blank { 0 } else { tracks },
    })
}

pub(crate) fn track(bytes: &[u8]) -> Result<Track> {
    let bytes = declared(bytes, 32)?;
    let high = |offset| bytes.get(offset).copied().unwrap_or(0);
    let empty = bytes[6] & 0x40 != 0; // Blank bit (not the Packet bit).
    // SAO can expose its signed first pregap (-150) here. It is not a data
    // append address; never display it as 4,294,967,146 or use it for ISO LBAs.
    let nwa = (bytes[7] & 1 != 0)
        .then(|| be32(bytes, 12))
        .transpose()?
        .filter(|lba| *lba <= 360_000);
    let start = be32(bytes, 8)? as i32;
    Ok(Track {
        number: u32::from(bytes[2]) | (u32::from(high(32)) << 8),
        session: u32::from(bytes[3]) | (u32::from(high(33)) << 8),
        start: Some(i64::from(start)),
        sectors: Some(be32(bytes, 24)?).filter(|sectors| *sectors <= 360_000),
        audio: bytes[5] & 4 == 0,
        closed: !empty && bytes[7] & 1 == 0,
        empty,
        next_writable: nwa,
    })
}

/// Preserve the returned page length and vendor extension; discard block descriptors.
pub(crate) fn write_parameters(bytes: &[u8], sao: bool, appendable: bool) -> Result<Vec<u8>> {
    let bytes = declared(bytes, 8)?;
    let block_len = usize::from(u16::from_be_bytes([bytes[6], bytes[7]]));
    let offset = 8 + block_len;
    let page = bytes
        .get(offset..)
        .context("missing write parameters page")?;
    let page_length = page.get(1).map(|n| usize::from(*n) + 2).unwrap_or(0);
    ensure!(
        (52..=page.len()).contains(&page_length) && page[0] & 0x3f == 5,
        "invalid write parameters page"
    );
    let mut out = vec![0; 8];
    out.extend_from_slice(&page[..page_length]);
    let page = &mut out[8..];
    page[0] = 5; // Clear PS; MODE SELECT uses PF, not SP.
    page[2] = (page[2] & 0x40) | if sao { 2 } else { 1 }; // Keep supported BUFE; clear test-write.
    page[3] = if appendable { 0xc0 } else { 0 } | if sao { 0 } else { 4 };
    page[4] = if sao { 0 } else { 8 }; // Raw audio / cooked Mode 1.
    page[7] = 0;
    page[8] = 0; // CD-DA/CD-ROM session.
    page[10..14].fill(0);
    page[14..16].copy_from_slice(&150u16.to_be_bytes());
    page[16..50].fill(0); // No stale MCN, ISRC or subheaders; preserve vendor bytes.
    Ok(out)
}

#[cfg(any(target_os = "macos", windows, test))]
pub(crate) fn leadin(msf: &[u8]) -> Option<i32> {
    let &[m, s, f] = msf else { return None };
    if !(90..100).contains(&m) || s >= 60 || f >= 75 {
        return None;
    }
    let lba = (i32::from(m) * 60 + i32::from(s)) * 75 + i32::from(f) - 450_150;
    (-45_000..=-4500).contains(&lba).then_some(lba)
}

pub(crate) fn cue(tracks: &[Music], text_leadin: Option<i32>) -> Result<Vec<u8>> {
    ensure!(
        !tracks.is_empty() && tracks.len() <= 98,
        "expected 1–98 audio tracks plus the data track"
    );
    ensure!(
        tracks[0].gap == 150 && tracks.iter().all(|t| t.sectors >= 300),
        "invalid audio track or first pregap"
    );
    let mut out = Vec::new();
    let mut entry = |number: u8, index: u8, form: u8, lba: i64| -> Result<()> {
        let frames = lba + 150;
        ensure!(
            (0..100 * 60 * 75).contains(&frames),
            "CUE address exceeds CD MSF range"
        );
        out.extend([
            1,
            number,
            index,
            form,
            0,
            (frames / 4500) as u8,
            (frames / 75 % 60) as u8,
            (frames % 75) as u8,
        ]);
        Ok(())
    };
    // Only the CD-Text lead-in uses a wrapped absolute MSF (e.g. 97:34:23).
    if let Some(start) = text_leadin {
        ensure!(
            (-45_000..=-4500).contains(&start),
            "invalid CD-Text lead-in"
        );
        entry(0, 0, 0x41, i64::from(start) + 450_000)?;
    } else {
        entry(0, 0, 1, -150)?;
    }
    let mut end = 0i64;
    for (i, track) in tracks.iter().enumerate() {
        let start = end + if i == 0 { 0 } else { i64::from(track.gap) };
        // A track without a pregap starts directly at INDEX 01. Some drives
        // get stuck at INDEX 00 when both entries have the same address.
        if track.gap > 0 {
            entry(i as u8 + 1, 0, 0, start - i64::from(track.gap))?;
        }
        entry(i as u8 + 1, 1, 0, start)?;
        end = start + i64::from(track.sectors) + i64::from(track.tail);
    }
    entry(0xaa, 1, 1, end)?;
    Ok(out)
}

pub(crate) fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &byte in bytes {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = (crc << 1) ^ if crc & 0x8000 != 0 { 0x1021 } else { 0 };
        }
    }
    !crc
}

#[cfg(any(target_os = "macos", windows, test))]
pub(crate) fn verify_q<'a>(
    frames: impl Iterator<Item = &'a [u8]>,
    first: u32,
    track: u8,
    index: u8,
    origin: u32,
) -> Result<()> {
    fn bcd(n: u32) -> u8 {
        ((n / 10) as u8) << 4 | (n % 10) as u8
    }
    fn msf(n: u32) -> [u8; 3] {
        [bcd(n / 4500), bcd(n / 75 % 60), bcd(n % 75)]
    }
    let mut total = 0u32;
    let mut matched = 0;
    for (offset, frame) in frames.enumerate() {
        let lba = first + offset as u32;
        total += 1;
        let mut raw = [0; 12];
        let q = if frame.len() == 96 {
            for (out, bits) in raw.iter_mut().zip(frame.chunks_exact(8)) {
                *out = bits.iter().fold(0, |byte, bit| byte << 1 | (bit >> 6 & 1));
            }
            &raw
        } else {
            ensure!(frame.len() == 16, "invalid Q subchannel transfer");
            &frame[..12]
        };
        let crc = u16::from_be_bytes([q[10], q[11]]);
        if (frame.len() == 96 || crc != 0) && crc16(&q[..10]) != crc {
            continue;
        }
        if q[0] & 15 != 1 {
            continue; // MCN/ISRC frames do not carry track position.
        }
        ensure!(
            q[0] & 0xd0 == 0
                && q[1..3] == [bcd(u32::from(track)), index]
                && q[3..6] == msf(lba.abs_diff(origin))
                && q[6] == 0
                && q[7..10] == msf(lba + 150),
            "Q subchannel mismatch at LBA {lba}: expected track {track:02} INDEX {index:02}, got {:02x}/{:02x} (relative {:02x?})",
            q[1],
            q[2],
            &q[3..6]
        );
        matched += 1;
    }
    ensure!(
        total > 0 && matched >= total.div_ceil(2),
        "insufficient valid Q position frames at LBA {first}: {matched}/{total}"
    );
    Ok(())
}

/// One English ASCII block, title and performer, followed by three size packs.
pub(crate) fn text_packs(title: &str, text: &CdText) -> Result<Vec<u8>> {
    ensure!(
        !text.titles.is_empty() && text.titles.len() <= 98,
        "invalid CD-Text track count"
    );
    let titles = std::iter::once(title)
        .chain(text.titles.iter().map(String::as_str))
        .collect::<Vec<_>>();
    let performers = vec![text.artist.as_str(); titles.len()];
    let mut packs: Vec<[u8; 18]> = Vec::new();
    let mut counts = [0u8; 16];
    for (kind, strings) in [(0x80u8, titles), (0x81, performers)] {
        let mut current: Option<[u8; 18]> = None;
        let mut used = 0;
        for (track, string) in strings.iter().enumerate() {
            ensure!(
                string.bytes().all(|b| (0x20..=0x7e).contains(&b)),
                "CD-Text requires printable ASCII"
            );
            for (pos, byte) in string.bytes().chain([0]).enumerate() {
                if current.is_none() {
                    ensure!(packs.len() < 253, "CD-Text exceeds one 256-pack block");
                    let mut pack = [0; 18];
                    pack[..4].copy_from_slice(&[
                        kind,
                        track as u8,
                        packs.len() as u8,
                        pos.min(15) as u8,
                    ]);
                    current = Some(pack);
                    used = 0;
                    counts[usize::from(kind & 15)] = counts[usize::from(kind & 15)]
                        .checked_add(1)
                        .context("too many CD-Text packs")?;
                }
                current.as_mut().unwrap()[4 + used] = byte;
                used += 1;
                if used == 12 {
                    packs.push(current.take().unwrap());
                }
            }
        }
        if let Some(pack) = current {
            packs.push(pack);
        }
    }
    ensure!(packs.len() <= 253, "CD-Text exceeds one 256-pack block");
    let mut size = [0; 36];
    size[..4].copy_from_slice(&[1, 1, text.titles.len() as u8, 0]); // ISO 646 ASCII.
    counts[15] = 3;
    size[4..20].copy_from_slice(&counts);
    size[20] = (packs.len() + 2) as u8;
    size[28] = 9; // English.
    for (i, data) in size.as_chunks::<12>().0.iter().enumerate() {
        let mut pack = [0; 18];
        pack[..4].copy_from_slice(&[0x8f, i as u8, packs.len() as u8, 0]);
        pack[4..16].copy_from_slice(data);
        packs.push(pack);
    }
    let mut out = Vec::new();
    for mut pack in packs {
        let crc = crc16(&pack[..16]);
        pack[16..].copy_from_slice(&crc.to_be_bytes());
        out.extend(pack);
    }
    Ok(out)
}

/// Four packs / 96 R-W symbols. Repeat to the least common multiple, never pad packs.
pub(crate) fn text_symbols(packs: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        !packs.is_empty() && packs.len().is_multiple_of(18),
        "invalid CD-Text packs"
    );
    let count = packs.len() / 18;
    let repeats = if count.is_multiple_of(4) {
        1
    } else if count.is_multiple_of(2) {
        2
    } else {
        4
    };
    let bytes = packs.repeat(repeats);
    let mut out = Vec::new();
    for b in bytes.as_chunks::<3>().0 {
        out.extend([
            b[0] >> 2,
            ((b[0] & 3) << 4) | (b[1] >> 4),
            ((b[1] & 15) << 2) | (b[2] >> 6),
            b[2] & 63,
        ]);
    }
    Ok(out)
}

pub(crate) fn ranges(tracks: &[Music]) -> Vec<std::ops::Range<u32>> {
    audio::track_ranges(tracks.iter().map(|t| (t.sectors + t.tail, t.gap))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uninitialized_bridge_identity_gate_rejects_other_drives_and_short_inquiry() {
        let mut inquiry = [b' '; 36];
        inquiry[0] = 5;
        inquiry[8..16].copy_from_slice(b"Apple   ");
        inquiry[16..32].copy_from_slice(b"SuperDrive      ");
        assert!(apple_superdrive_proxy(&inquiry));
        assert!(!apple_superdrive_proxy(&inquiry[..35]));
        inquiry[0] = 0;
        assert!(!apple_superdrive_proxy(&inquiry));
        inquiry[0] = 5;
        inquiry[8..16].copy_from_slice(b"PIONEER ");
        assert!(!apple_superdrive_proxy(&inquiry));
    }

    #[test]
    fn write_speed_descriptors_use_2352_bytes_per_cd_sector() {
        let mut bytes = vec![0; 40];
        bytes[3] = 36;
        bytes[20..24].copy_from_slice(&1764u32.to_be_bytes());
        bytes[36..40].copy_from_slice(&706u32.to_be_bytes());
        assert_eq!(write_speeds(&bytes).unwrap(), [300, 750]);
        assert!(write_speeds(&bytes[..39]).is_err());
    }

    #[test]
    fn configuration_and_media_use_reported_profiles_and_reject_truncated_descriptors() {
        let mut config = vec![0; 8];
        config[6..8].copy_from_slice(&10u16.to_be_bytes());
        config.extend([0, 0, 1, 8, 0, 9, 0, 0, 0, 10, 1, 0]);
        config.extend([0, 0x2d, 1, 4, 0, 0, 0, 0, 0, 0x2e, 1, 4, 0x21, 0, 0, 0]);
        let len = config.len() as u32 - 4;
        config[..4].copy_from_slice(&len.to_be_bytes());
        let (profile, caps) = configuration(&config).unwrap();
        assert_eq!(profile, 10);
        assert!(caps.cdr && caps.cdrw && caps.tao && caps.sao && caps.cd_text);
        assert!(configuration(&config[..config.len() - 1]).is_err());
        let mut disc = [0; 34];
        disc[..2].copy_from_slice(&32u16.to_be_bytes());
        disc[2] = 0x10;
        disc[4] = 1;
        disc[6] = 1;
        let blank = media(10, &disc, true).unwrap();
        assert!(blank.blank && blank.reserved && !blank.appendable);
        assert_eq!((blank.sessions, blank.tracks), (0, 0));
        disc[2] = 0x11;
        disc[4] = 2;
        disc[6] = 3;
        let used = media(10, &disc, false).unwrap();
        assert!(!used.blank && used.appendable && !used.reserved);
        assert_eq!((used.sessions, used.tracks), (2, 3));
    }

    #[test]
    fn cue_omits_empty_pregaps_and_preserves_track_addresses() {
        let tracks = [150, 0, 75].map(|gap| Music {
            path: "x.wav".into(),
            sectors: 300,
            gap,
            tail: 0,
        });
        let sheet = cue(&tracks, leadin(&[97, 34, 23])).unwrap();
        let entries = sheet.as_chunks::<8>().0.iter().collect::<Vec<_>>();
        assert_eq!(entries[0].as_slice(), [1, 0, 0, 0x41, 0, 97, 34, 23]);
        assert_eq!(entries[1].as_slice(), [1, 1, 0, 0, 0, 0, 0, 0]);
        assert_eq!(entries[2].as_slice(), [1, 1, 1, 0, 0, 0, 2, 0]);
        assert_eq!(entries[3].as_slice(), [1, 2, 1, 0, 0, 0, 6, 0]);
        assert_eq!(entries[4].as_slice(), [1, 3, 0, 0, 0, 0, 10, 0]);
        assert_eq!(entries[5].as_slice(), [1, 3, 1, 0, 0, 0, 11, 0]);
        assert_eq!(ranges(&tracks), [0..300, 300..600, 675..975]);
        assert_eq!(entries.last().unwrap()[5..], [0, 15, 0]);
    }

    #[test]
    fn q_readback_rejects_the_real_stuck_index_even_with_a_valid_crc() {
        let correct = [
            0x01, 0x02, 0x01, 0x00, 0x00, 0x00, 0x00, 0x03, 0x01, 0x25, 0x0f, 0xa8, 0, 0, 0, 0,
        ];
        verify_q([correct.as_slice()].into_iter(), 13_450, 2, 1, 13_450).unwrap();
        let mut wrong = correct;
        wrong[2] = 0;
        let crc = crc16(&wrong[..10]);
        wrong[10..12].copy_from_slice(&crc.to_be_bytes());
        assert!(
            verify_q([wrong.as_slice()].into_iter(), 13_450, 2, 1, 13_450)
                .unwrap_err()
                .to_string()
                .contains("Q subchannel mismatch")
        );
    }

    #[test]
    fn text_has_valid_crc_counts_sequence_and_roundtrips_six_bit_symbols() {
        assert_eq!(crc16(b"123456789"), 0xce3c);
        let text = CdText {
            artist: "Artist".into(),
            titles: vec![
                "A long track title for several packs".into(),
                "Track two".into(),
            ],
        };
        let packs = text_packs("Album", &text).unwrap();
        let rows = packs.as_chunks::<18>().0.iter().collect::<Vec<_>>();
        for (i, pack) in rows.iter().enumerate() {
            assert_eq!(pack[2], i as u8);
            assert_eq!(crc16(&pack[..16]).to_be_bytes(), pack[16..]);
        }
        let size = rows[rows.len() - 3..]
            .iter()
            .flat_map(|p| p[4..16].iter().copied())
            .collect::<Vec<_>>();
        assert_eq!(size[..4], [1, 1, 2, 0]);
        assert_eq!(
            size[4] as usize,
            rows.iter().filter(|p| p[0] == 0x80).count()
        );
        assert_eq!(
            size[5] as usize,
            rows.iter().filter(|p| p[0] == 0x81).count()
        );
        assert_eq!(size[19], 3);
        assert_eq!(size[20] as usize, rows.len() - 1);
        let symbols = text_symbols(&packs).unwrap();
        assert!(symbols.len().is_multiple_of(96));
        assert!(symbols.iter().all(|s| *s < 64));
        let decoded = symbols
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|b| {
                [
                    (b[0] << 2) | (b[1] >> 4),
                    (b[1] << 4) | (b[2] >> 2),
                    (b[2] << 6) | b[3],
                ]
            })
            .collect::<Vec<_>>();
        assert!(decoded.chunks_exact(packs.len()).all(|b| b == packs));
        assert!(
            text_packs(
                "x",
                &CdText {
                    artist: "x".into(),
                    titles: vec!["x".repeat(4096)]
                }
            )
            .is_err()
        );
    }

    #[test]
    fn track_parser_distinguishes_blank_packet_closed_and_signed_pregap() {
        let mut b = [0; 36];
        b[..2].copy_from_slice(&34u16.to_be_bytes());
        b[2] = 2;
        b[3] = 2;
        b[5] = 4;
        b[6] = 1;
        b[8..12].copy_from_slice(&(-150i32).to_be_bytes());
        b[24..28].copy_from_slice(&1000u32.to_be_bytes());
        let t = track(&b).unwrap();
        assert!(t.closed && !t.empty && !t.audio);
        assert_eq!(t.start, Some(-150));
        b[6] |= 0x20;
        assert!(!track(&b).unwrap().empty); // Packet is not Blank.
        b[6] |= 0x40;
        b[7] = 1;
        assert!(track(&b).unwrap().empty);
        assert!(!track(&b).unwrap().closed);
        b[12..16].copy_from_slice(&(-150i32).to_be_bytes());
        assert_eq!(track(&b).unwrap().next_writable, None);
        assert!(!track(&b).unwrap().closed);
        assert!(track(&b[..31]).is_err());
    }

    #[test]
    fn parameters_clear_test_write_and_stale_metadata_preserving_bufe() {
        let mut b = [0xff; 68];
        b[..2].copy_from_slice(&66u16.to_be_bytes());
        b[6..8].copy_from_slice(&8u16.to_be_bytes());
        b[16] = 0x85;
        b[17] = 50;
        let p = write_parameters(&b, true, true).unwrap();
        assert_eq!(&p[..8], [0; 8]);
        assert_eq!(&p[8..13], [5, 50, 0x42, 0xc0, 0]);
        assert!(p[24..58].iter().all(|b| *b == 0));
        assert!(p[58..].iter().all(|b| *b == 0xff));
        let p = write_parameters(&b, false, false).unwrap();
        assert_eq!(&p[10..13], [0x41, 4, 8]);
        assert!(write_parameters(&b[..50], true, true).is_err());
    }
}
