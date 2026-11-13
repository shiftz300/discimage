use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    ops::Range,
    path::Path,
};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::codec::RAW;

pub const SAMPLES_PER_SECTOR: u32 = 588;
pub const SECTORS_PER_SECOND: u32 = 75;

pub(crate) fn track_path(index: usize) -> String {
    format!("audio/{:02}.wav", index + 1)
}

/// Compute INDEX 01 ranges from (track sectors, pregap sectors).
/// The first pregap is before disc LBA 0.
pub(crate) fn track_ranges(
    tracks: impl Iterator<Item = (u32, u32)>,
) -> impl Iterator<Item = Range<u32>> {
    tracks.enumerate().scan(0, |end, (index, (sectors, gap))| {
        let start = *end + if index == 0 { 0 } else { gap };
        *end = start + sectors;
        Some(start..*end)
    })
}

/// Compare CD-DA from LBA 0 through the end of the last track, including
/// inter-track silence. The first track's pregap precedes LBA 0.
#[cfg(any(target_os = "macos", windows, test))]
pub(crate) fn verify_pcm(
    tracks: &[crate::device::Music],
    chunk_sectors: u32,
    mut read: impl FnMut(u32, &mut [u8]) -> Result<()>,
) -> Result<()> {
    use crate::{console, device};
    use std::time::{Duration, Instant};

    let mut actual = vec![0; chunk_sectors as usize * RAW];
    let mut expected = actual.clone();
    let start = Instant::now();
    let ranges = track_ranges(
        tracks
            .iter()
            .map(|track| (track.sectors + track.tail, track.gap)),
    );
    for (i, (track, range)) in tracks.iter().zip(ranges).enumerate() {
        let number = i + 1;
        if i > 0 {
            let mut lba = range.start - track.gap;
            while lba < range.start {
                ensure!(!device::cancelled(), "audio readback cancelled");
                let count = (range.start - lba).min(chunk_sectors);
                let bytes = &mut actual[..count as usize * RAW];
                read(lba, bytes).with_context(|| {
                    format!("cannot read pregap before track {number} at LBA {lba}")
                })?;
                if let Some(offset) = bytes
                    .as_chunks::<RAW>()
                    .0
                    .iter()
                    .position(|sector| sector.iter().any(|&byte| byte != 0))
                {
                    anyhow::bail!(
                        "non-silent pregap before track {number} at LBA {}",
                        lba + offset as u32
                    );
                }
                lba += count;
            }
        }

        let source = inspect(&track.path)?;
        ensure!(
            source.sectors == track.sectors && source.padding_sample_frames == 0,
            "normalized WAV for track {number} has changed"
        );
        let mut wav = File::open(&track.path)?;
        wav.seek(SeekFrom::Start(source.data_offset))?;
        console::phase(
            "readback",
            format_args!(
                "Track {number}/{} | {} sectors",
                tracks.len(),
                track.sectors
            ),
        );
        let mut lba = range.start;
        let mut reported = Instant::now();
        let pcm_end = range.end - track.tail;
        while lba < range.end {
            ensure!(!device::cancelled(), "audio readback cancelled");
            let count = (range.end - lba).min(chunk_sectors);
            let len = count as usize * RAW;
            let pcm_len = pcm_end.saturating_sub(lba).min(count) as usize * RAW;
            wav.read_exact(&mut expected[..pcm_len])?;
            expected[pcm_len..len].fill(0);
            read(lba, &mut actual[..len])
                .with_context(|| format!("cannot read track {number} at LBA {lba}"))?;
            if let Some(offset) = actual[..len]
                .as_chunks::<RAW>()
                .0
                .iter()
                .zip(expected[..len].as_chunks::<RAW>().0)
                .position(|(a, b)| a != b)
            {
                anyhow::bail!(
                    "PCM differs from the WAV in track {number} at LBA {}",
                    lba + offset as u32
                );
            }
            lba += count;
            if reported.elapsed() >= Duration::from_secs(15) {
                console::phase(
                    "readback",
                    format_args!(
                        "Track {number}/{} | {}/{} sectors | {}s elapsed",
                        tracks.len(),
                        lba - range.start,
                        track.sectors,
                        start.elapsed().as_secs()
                    ),
                );
                reported = Instant::now();
            }
        }
    }
    console::success(format_args!(
        "All {} audio tracks and planned silence matched byte-for-byte",
        tracks.len()
    ));
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioTrack {
    pub path: std::path::PathBuf,
    /// Stereo sample frames, not individual channel samples.
    pub sample_frames: u32,
    pub sectors: u32,
    pub padding_sample_frames: u32,
    pub data_offset: u64,
}

pub fn inspect(path: &Path) -> Result<AudioTrack> {
    let (sample_frames, data_offset) =
        inspect_riff(path).with_context(|| format!("cannot inspect WAV {}", path.display()))?;
    ensure!(sample_frames > 0, "{}: empty audio", path.display());
    let sectors = sample_frames.div_ceil(SAMPLES_PER_SECTOR);
    // Reject rather than silently extend very short tracks to the CD minimum.
    ensure!(
        sectors >= 300,
        "{}: tracks must occupy at least 300 sectors (4 seconds)",
        path.display()
    );
    Ok(AudioTrack {
        path: path.to_path_buf(),
        sample_frames,
        sectors,
        padding_sample_frames: sectors * SAMPLES_PER_SECTOR - sample_frames,
        data_offset,
    })
}

/// Bounded RIFF walk: honor word padding, check declared extents against the
/// actual file, and never allocate based on an untrusted chunk length.
fn inspect_riff(path: &Path) -> Result<(u32, u64)> {
    let mut file = File::open(path)?;
    let file_len = file.metadata()?.len();
    let mut header = [0; 12];
    file.read_exact(&mut header)?;
    ensure!(
        &header[..4] == b"RIFF" && &header[8..] == b"WAVE",
        "expected a little-endian RIFF/WAVE file (RF64 is not supported)"
    );
    let riff_end = u64::from(u32::from_le_bytes(header[4..8].try_into()?)) + 8;
    ensure!(
        riff_end >= 12 && riff_end <= file_len,
        "invalid or truncated RIFF extent"
    );
    let mut offset = 12;
    let mut format_seen = false;
    let mut frames = None;
    while offset < riff_end {
        ensure!(riff_end - offset >= 8, "truncated RIFF chunk header");
        file.seek(SeekFrom::Start(offset))?;
        let mut chunk = [0; 8];
        file.read_exact(&mut chunk)?;
        let size = u32::from_le_bytes(chunk[4..].try_into()?);
        let end = offset + 8 + u64::from(size);
        let next = end + u64::from(size % 2);
        ensure!(
            next <= riff_end,
            "truncated RIFF chunk or missing word padding"
        );
        match &chunk[..4] {
            b"fmt " => {
                ensure!(!format_seen && size >= 16, "duplicate or invalid fmt chunk");
                let mut fmt = [0; 16];
                file.read_exact(&mut fmt)?;
                let word = |i| u16::from_le_bytes([fmt[i], fmt[i + 1]]);
                let dword = |i: usize| u32::from_le_bytes(fmt[i..i + 4].try_into().unwrap());
                ensure!(
                    word(0) == 1
                        && word(2) == 2
                        && dword(4) == 44_100
                        && dword(8) == 176_400
                        && word(12) == 4
                        && word(14) == 16,
                    "expected PCM format-tag 1 WAV, 44.1 kHz, stereo, 16-bit; convert the source first"
                );
                format_seen = true;
            }
            b"data" => {
                ensure!(format_seen, "fmt chunk must precede data");
                ensure!(
                    frames.is_none() && size.is_multiple_of(4),
                    "duplicate data chunk or incomplete stereo sample frame"
                );
                frames = Some((size / 4, offset + 8));
            }
            _ => {}
        }
        offset = next;
    }
    frames.context("missing WAV data chunk")
}

/// Copy PCM unchanged, normalize the RIFF container and explicitly zero-pad its tail.
pub fn export(track: &AudioTrack, path: &Path) -> Result<()> {
    export_with_tail(track, path, 0)
}

pub(crate) fn export_with_tail(track: &AudioTrack, path: &Path, tail: u32) -> Result<()> {
    let current = inspect(&track.path)?;
    ensure!(
        current.sample_frames == track.sample_frames && current.data_offset == track.data_offset,
        "audio input changed since planning: {}",
        track.path.display()
    );
    let mut input = File::open(&track.path)?;
    input.seek(SeekFrom::Start(track.data_offset))?;
    let mut output =
        std::io::BufWriter::new(OpenOptions::new().write(true).create_new(true).open(path)?);
    let bytes = track
        .sectors
        .checked_add(tail)
        .and_then(|n| n.checked_mul(RAW as u32))
        .filter(|&n| n <= u32::MAX - 36)
        .context("padded WAV exceeds the RIFF size limit")?;
    header(&mut output, bytes)?;
    let expected = u64::from(track.sample_frames) * 4;
    ensure!(
        std::io::copy(&mut input.take(expected), &mut output)? == expected,
        "truncated audio source"
    );
    output.write_all(&vec![0; track.padding_sample_frames as usize * 4])?;
    output.flush()?;
    if tail != 0 {
        output.get_ref().set_len(44 + u64::from(bytes))?;
    }
    Ok(())
}

pub(crate) fn silence(path: &Path) -> Result<()> {
    let mut output = OpenOptions::new().write(true).create_new(true).open(path)?;
    let bytes = 300 * RAW as u32;
    header(&mut output, bytes)?;
    output.set_len(44 + u64::from(bytes))?;
    Ok(())
}

fn header(output: &mut impl Write, bytes: u32) -> Result<()> {
    output.write_all(b"RIFF")?;
    output.write_all(&(36 + bytes).to_le_bytes())?;
    output.write_all(b"WAVEfmt ")?;
    output.write_all(&16u32.to_le_bytes())?;
    for value in [1u16, 2] {
        output.write_all(&value.to_le_bytes())?;
    }
    output.write_all(&44100u32.to_le_bytes())?;
    output.write_all(&176400u32.to_le_bytes())?;
    for value in [4u16, 16] {
        output.write_all(&value.to_le_bytes())?;
    }
    output.write_all(b"data")?;
    output.write_all(&bytes.to_le_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::Music;

    #[test]
    fn pcm_readback_rejects_corruption_in_pregap_and_track() {
        let path = std::env::temp_dir().join(format!("discimage-pcm-{}.wav", std::process::id()));
        silence(&path).unwrap();
        let track = |gap| Music {
            path: path.clone(),
            sectors: 300,
            gap,
            tail: 0,
        };
        let tracks = [track(150), track(75), track(0)];
        let verify = |corrupt_at: Option<u32>| {
            let mut end = 0;
            let result = verify_pcm(&tracks, 24, |lba, bytes| {
                assert_eq!(lba, end, "readback must cover the entire program area");
                let count = (bytes.len() / RAW) as u32;
                assert!((1..=24).contains(&count));
                end = lba + count;
                bytes.fill(0);
                if let Some(bad) = corrupt_at.filter(|bad| (lba..end).contains(bad)) {
                    bytes[(bad - lba) as usize * RAW + RAW - 1] = 1;
                }
                Ok(())
            });
            if result.is_ok() {
                assert_eq!(end, 975);
            }
            result
        };
        verify(None).unwrap();
        let error = verify(Some(374)).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("pregap before track 2 at LBA 374")
        );
        let error = verify(Some(674)).unwrap_err();
        assert!(error.to_string().contains("track 2 at LBA 674"));
        std::fs::remove_file(path).unwrap();
    }
}
