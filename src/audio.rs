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
    let bytes = track.sectors * RAW as u32;
    header(&mut output, bytes)?;
    let expected = u64::from(track.sample_frames) * 4;
    ensure!(
        std::io::copy(&mut input.take(expected), &mut output)? == expected,
        "truncated audio source"
    );
    output.write_all(&vec![0; track.padding_sample_frames as usize * 4])?;
    output.flush()?;
    Ok(())
}

pub(crate) fn silence(path: &Path) -> Result<()> {
    let mut output = OpenOptions::new().write(true).create_new(true).open(path)?;
    let bytes = 300 * RAW as u32;
    header(&mut output, bytes)?;
    output.set_len(44 + u64::from(bytes))?;
    Ok(())
}

pub(crate) fn extend_normalized(path: &Path, sectors: u32) -> Result<()> {
    let track = inspect(path)?;
    let mut wav = OpenOptions::new().write(true).open(path)?;
    ensure!(
        track.data_offset == 44
            && track.padding_sample_frames == 0
            && wav.metadata()?.len() == 44 + u64::from(track.sectors) * RAW as u64
            && sectors == track.sectors + 1,
        "expected a normalized WAV and one additional sector"
    );
    let bytes = sectors
        .checked_mul(RAW as u32)
        .filter(|&size| size <= u32::MAX - 36)
        .context("padded WAV exceeds the RIFF size limit")?;
    wav.set_len(44 + u64::from(bytes))?;
    header(&mut wav, bytes)
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
