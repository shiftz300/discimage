//! Minimal deterministic ISO9660 level 2 + Joliet writer. Extents are absolute
//! disc LBAs; the output stream starts at the second session's INDEX 01.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    fs::File,
    io::{self, BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

pub const BLOCK: u32 = crate::codec::COOKED as u32;

/// A cooked-payload view of either a 2048-byte ISO or 2352-byte Mode 1 RAW.
pub(crate) struct Image {
    input: BufReader<File>,
    stride: u64,
    header: u64,
    position: u64,
    physical: u64,
    length: u64,
}

impl Image {
    pub(crate) fn open(path: &Path, raw: bool, sectors: u32) -> Result<Self> {
        let input = BufReader::with_capacity(1024 * 1024, File::open(path)?);
        let stride = if raw {
            crate::codec::RAW as u64
        } else {
            u64::from(BLOCK)
        };
        ensure!(
            input.get_ref().metadata()?.len() == u64::from(sectors) * stride,
            "incorrect session image length"
        );
        Ok(Self {
            input,
            stride,
            header: if raw { 16 } else { 0 },
            position: 0,
            physical: 0,
            length: u64::from(sectors) * u64::from(BLOCK),
        })
    }
}

impl Read for Image {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() || self.position >= self.length {
            return Ok(0);
        }
        let within = self.position % u64::from(BLOCK);
        let physical = self.position / u64::from(BLOCK) * self.stride + self.header + within;
        self.input
            .seek_relative(physical as i64 - self.physical as i64)?;
        let count = buffer.len().min((u64::from(BLOCK) - within) as usize);
        let read = self.input.read(&mut buffer[..count])?;
        self.physical = physical + read as u64;
        self.position += read as u64;
        Ok(read)
    }
}

impl Seek for Image {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let position = match from {
            SeekFrom::Start(value) => i128::from(value),
            SeekFrom::Current(value) => i128::from(self.position) + i128::from(value),
            SeekFrom::End(value) => i128::from(self.length) + i128::from(value),
        };
        self.position = u64::try_from(position)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid image offset"))?;
        Ok(self.position)
    }
}

/// Convert an absolute LBA to a file offset if the entire extent fits the session.
/// An empty extent may begin at EOF.
pub(crate) fn extent_offset(base: u32, sectors: u32, lba: u32, bytes: u64) -> Option<u64> {
    let offset = u64::from(lba.checked_sub(base)?) * u64::from(BLOCK);
    let remaining = (u64::from(sectors) * u64::from(BLOCK)).checked_sub(offset)?;
    (bytes <= remaining).then_some(offset)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub path: String,
    pub lba: u32,
    pub bytes: u32,
    pub sha256: String,
}

struct Node {
    name: String,
    primary: Vec<u8>,
    joliet: Vec<u8>,
    parent: usize,
    children: Vec<usize>,
    directory: bool,
    source: Option<PathBuf>,
    bytes: u32,
    extent: [u32; 2],
    sizes: [u32; 2],
}

impl Node {
    fn new(name: String, parent: usize, directory: bool, index: usize) -> Result<Self> {
        ensure!(
            name.encode_utf16().count() <= 64
                && !name.chars().any(|c| c as u32 > 0xffff || c.is_control()),
            "Joliet names must use at most 64 BMP characters: {name}"
        );
        let upper = name.to_ascii_uppercase();
        let valid = upper.len() <= 29
            && !upper.is_empty()
            && upper
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_' || b == b'.')
            && if directory {
                !upper.contains('.')
            } else {
                !upper.starts_with('.') && upper.bytes().filter(|&b| b == b'.').count() <= 1
            };
        let mut primary = if valid {
            upper
        } else {
            format!("{}{:06}", if directory { 'D' } else { 'F' }, index)
        };
        if !directory && !primary.contains('.') {
            primary.push('.');
        }
        Ok(Self {
            name: name.clone(),
            primary: primary.into_bytes(),
            joliet: ucs2(&name),
            parent,
            children: Vec::new(),
            directory,
            source: None,
            bytes: 0,
            extent: [0; 2],
            sizes: [0; 2],
        })
    }

    fn identifier(&self, joliet: usize) -> Vec<u8> {
        let mut name = if joliet == 1 {
            self.joliet.clone()
        } else {
            self.primary.clone()
        };
        if !self.directory {
            name.extend(if joliet == 1 {
                ucs2(";1")
            } else {
                b";1".to_vec()
            });
        }
        name
    }
}

pub struct Volume {
    pub base: u32,
    pub sectors: u32,
    pub drawing: u32,
    pub used: u32,
    nodes: Vec<Node>,
    headers: BTreeMap<u32, [u8; 2048]>,
}

impl Volume {
    pub fn new(
        files: Option<&Path>,
        base: u32,
        sectors: u32,
        drawing: u32,
        title: &str,
    ) -> Result<Self> {
        ensure!(
            base.checked_add(sectors).is_some() && drawing < sectors && sectors <= 360_000,
            "invalid ISO sector range"
        );
        let mut root = Node::new("ROOT".into(), 0, true, 0)?;
        root.primary = vec![0];
        root.joliet = vec![0];
        let mut volume = Self {
            base,
            sectors,
            drawing,
            used: 0,
            nodes: vec![root],
            headers: BTreeMap::new(),
        };
        if let Some(path) = files {
            ensure!(
                std::fs::symlink_metadata(path)?.is_dir(),
                "files must name a real directory"
            );
            volume.scan(path, 0, 0)?;
        }
        ensure!(
            !volume.nodes[0]
                .children
                .iter()
                .any(|&i| volume.nodes[i].name.eq_ignore_ascii_case("ARTWORK.BIN")),
            "ARTWORK.BIN is reserved for the generated drawing"
        );
        let art = volume.nodes.len();
        let mut node = Node::new("ARTWORK.BIN".into(), 0, false, art)?;
        node.bytes = (sectors - drawing) * BLOCK;
        node.extent = [base + drawing; 2];
        volume.nodes.push(node);
        volume.nodes[0].children.push(art);
        volume.layout(title)?;
        Ok(volume)
    }

    fn scan(&mut self, path: &Path, parent: usize, depth: usize) -> Result<()> {
        ensure!(depth < 8, "ISO directory nesting exceeds 8 levels");
        let mut entries = std::fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|e| e.file_name());
        let mut names = HashSet::new();
        for entry in entries {
            ensure!(
                self.nodes.len() < 10_000,
                "at most 10000 ISO entries are supported"
            );
            let kind = entry.file_type()?;
            ensure!(
                kind.is_dir() || kind.is_file(),
                "symlinks and special files are unsupported: {}",
                entry.path().display()
            );
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("non-Unicode filename"))?;
            let index = self.nodes.len();
            let mut node = Node::new(name, parent, kind.is_dir(), index)?;
            if !names.insert(node.primary.clone()) {
                node.primary =
                    format!("N{index:06}{}", if node.directory { "" } else { "." }).into_bytes();
                ensure!(names.insert(node.primary.clone()), "ISO alias collision");
            }
            if kind.is_file() {
                node.bytes = u32::try_from(entry.metadata()?.len())
                    .context("file exceeds ISO size limit")?;
                node.source = Some(entry.path());
            }
            self.nodes.push(node);
            self.nodes[parent].children.push(index);
            if kind.is_dir() {
                self.scan(&entry.path(), index, depth + 1)?;
            }
        }
        Ok(())
    }

    fn layout(&mut self, title: &str) -> Result<()> {
        let mut dirs = vec![0];
        let mut next = 0;
        while next < dirs.len() {
            let parent = dirs[next];
            dirs.extend(
                self.nodes[parent]
                    .children
                    .iter()
                    .filter(|&&i| self.nodes[i].directory)
                    .copied(),
            );
            next += 1;
        }
        let mut tables = [[0u32; 2]; 2];
        let mut lengths = [0; 2];
        let mut cursor = 19;
        for j in 0..2 {
            lengths[j] = dirs
                .iter()
                .map(|&i| {
                    let n = self.nodes[i].identifier(j).len() as u32;
                    8 + n + n % 2
                })
                .sum();
            for location in &mut tables[j] {
                *location = cursor;
                cursor += blocks(lengths[j]);
            }
        }
        for j in 0..2 {
            for &dir in &dirs {
                let mut size = 68u32;
                let mut children = self.nodes[dir].children.clone();
                children.sort_by_key(|&i| self.nodes[i].identifier(j));
                for child in children {
                    let len = record_size(self.nodes[child].identifier(j).len()) as u32;
                    if size % BLOCK + len > BLOCK {
                        size = blocks(size) * BLOCK;
                    }
                    size += len;
                }
                self.nodes[dir].extent[j] = self.base + cursor;
                self.nodes[dir].sizes[j] = blocks(size) * BLOCK;
                cursor += blocks(size);
            }
        }
        for node in &mut self.nodes {
            if node.source.is_some() {
                node.extent = [self.base + cursor; 2];
                cursor = cursor
                    .checked_add(blocks(node.bytes))
                    .context("ISO capacity overflow")?;
            }
        }
        self.used = cursor;
        ensure!(
            cursor <= self.drawing,
            "files + ISO metadata need {cursor} sectors, data_reserve_sectors is {}",
            self.drawing
        );
        for j in 0..2 {
            for (endian, &location) in tables[j].iter().enumerate() {
                let mut table = Vec::new();
                for &id in &dirs {
                    let name = self.nodes[id].identifier(j);
                    table.extend([name.len() as u8, 0]);
                    let lba = self.nodes[id].extent[j];
                    table.extend(if endian == 0 {
                        lba.to_le_bytes()
                    } else {
                        lba.to_be_bytes()
                    });
                    let parent = (dirs
                        .iter()
                        .position(|&n| n == self.nodes[id].parent)
                        .unwrap()
                        + 1) as u16;
                    table.extend(if endian == 0 {
                        parent.to_le_bytes()
                    } else {
                        parent.to_be_bytes()
                    });
                    table.extend(&name);
                    if name.len() % 2 == 1 {
                        table.push(0);
                    }
                }
                self.insert(location, &table);
            }
            for &dir in &dirs {
                let node = &self.nodes[dir];
                let mut data = vec![0; node.sizes[j] as usize];
                let dot = record(node.extent[j], node.sizes[j], true, &[0]);
                data[..dot.len()].copy_from_slice(&dot);
                let parent = &self.nodes[node.parent];
                let dotdot = record(parent.extent[j], parent.sizes[j], true, &[1]);
                data[34..68].copy_from_slice(&dotdot);
                let mut offset = 68;
                let mut children = node.children.clone();
                children.sort_by_key(|&i| self.nodes[i].identifier(j));
                for child in children {
                    let child = &self.nodes[child];
                    let rec = record(
                        child.extent[j],
                        if child.directory {
                            child.sizes[j]
                        } else {
                            child.bytes
                        },
                        child.directory,
                        &child.identifier(j),
                    );
                    if offset % 2048 + rec.len() > 2048 {
                        offset = offset.div_ceil(2048) * 2048;
                    }
                    data[offset..offset + rec.len()].copy_from_slice(&rec);
                    offset += rec.len();
                }
                let sector = node.extent[j] - self.base;
                self.insert(sector, &data);
            }
            let mut descriptor = [0u8; 2048];
            descriptor[0] = if j == 0 { 1 } else { 2 };
            descriptor[1..6].copy_from_slice(b"CD001");
            descriptor[6] = 1;
            descriptor[8..72].fill(b' ');
            let label: String = title
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() {
                        c.to_ascii_uppercase()
                    } else {
                        '_'
                    }
                })
                .take(32)
                .collect();
            if j == 0 {
                descriptor[40..40 + label.len()].copy_from_slice(label.as_bytes());
            } else {
                descriptor[40..72].fill(0);
                let label = ucs2(&label.chars().take(16).collect::<String>());
                descriptor[40..40 + label.len()].copy_from_slice(&label);
                descriptor[88..91].copy_from_slice(b"%/E");
            }
            dual32(&mut descriptor[80..88], self.base + self.sectors);
            dual16(&mut descriptor[120..124], 1);
            dual16(&mut descriptor[124..128], 1);
            dual16(&mut descriptor[128..132], 2048);
            dual32(&mut descriptor[132..140], lengths[j]);
            descriptor[140..144].copy_from_slice(&(self.base + tables[j][0]).to_le_bytes());
            descriptor[148..152].copy_from_slice(&(self.base + tables[j][1]).to_be_bytes());
            descriptor[156..190].copy_from_slice(&record(
                self.nodes[0].extent[j],
                self.nodes[0].sizes[j],
                true,
                &[0],
            ));
            descriptor[190..813].fill(b' ');
            for start in [813, 830, 864] {
                descriptor[start..start + 16].copy_from_slice(b"2000010100000000");
            }
            descriptor[847..863].copy_from_slice(b"0000000000000000");
            descriptor[881] = 1;
            self.headers.insert(16 + j as u32, descriptor);
        }
        let mut end = [0; 2048];
        end[0] = 255;
        end[1..6].copy_from_slice(b"CD001");
        end[6] = 1;
        self.headers.insert(18, end);
        Ok(())
    }

    fn insert(&mut self, sector: u32, bytes: &[u8]) {
        for (i, chunk) in bytes.chunks(2048).enumerate() {
            let mut block = [0; 2048];
            block[..chunk.len()].copy_from_slice(chunk);
            self.headers.insert(sector + i as u32, block);
        }
    }

    fn path(&self, mut id: usize) -> String {
        let mut parts = Vec::new();
        while id != 0 {
            parts.push(self.nodes[id].name.as_str());
            id = self.nodes[id].parent;
        }
        parts.reverse();
        parts.join("/")
    }

    /// Freeze the scanned tree, including empty directories, before any recording.
    pub fn snapshot(&self, directory: &Path) -> Result<()> {
        std::fs::create_dir(directory)?;
        for (id, node) in self.nodes.iter().enumerate().skip(1) {
            let destination = directory.join(self.path(id));
            if node.directory {
                std::fs::create_dir_all(&destination)?;
            } else if let Some(source) = &node.source {
                ensure!(
                    std::fs::symlink_metadata(source)?.is_file(),
                    "source is no longer a regular file"
                );
                let mut input = File::open(source)?;
                ensure!(
                    input.metadata()?.len() == u64::from(node.bytes),
                    "source size changed before snapshot"
                );
                let mut output = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(destination)?;
                let count =
                    std::io::copy(&mut (&mut input).take(u64::from(node.bytes)), &mut output)?;
                ensure!(
                    count == u64::from(node.bytes) && input.metadata()?.len() == count,
                    "source size changed during snapshot"
                );
                output.sync_all()?;
            }
        }
        Ok(())
    }

    /// Writes metadata, snapshots user files, then pads up to the drawing extent.
    pub fn prefix(&self, output: &mut impl Write) -> Result<Vec<Entry>> {
        let first_file = self
            .nodes
            .iter()
            .filter(|n| n.source.is_some())
            .map(|n| n.extent[0] - self.base)
            .min()
            .unwrap_or(self.used);
        let zero = [0; 2048];
        for sector in 0..first_file {
            output.write_all(self.headers.get(&sector).unwrap_or(&zero))?;
        }
        let mut cursor = first_file;
        let mut records = Vec::new();
        let mut buffer = [0; 65536];
        for (id, node) in self
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.source.is_some())
        {
            let mut file = File::open(node.source.as_ref().unwrap())?;
            ensure!(
                file.metadata()?.len() == u64::from(node.bytes),
                "source size changed: {}",
                self.path(id)
            );
            let mut left = node.bytes as usize;
            let mut hash = Sha256::new();
            while left > 0 {
                let count = left.min(buffer.len());
                file.read_exact(&mut buffer[..count])?;
                output.write_all(&buffer[..count])?;
                hash.update(&buffer[..count]);
                left -= count;
            }
            ensure!(
                file.metadata()?.len() == u64::from(node.bytes),
                "source size changed during copy"
            );
            let padding = (BLOCK - node.bytes % BLOCK) % BLOCK;
            output.write_all(&zero[..padding as usize])?;
            cursor += blocks(node.bytes);
            records.push(Entry {
                path: self.path(id),
                lba: node.extent[0],
                bytes: node.bytes,
                sha256: crate::hex(&hash.finalize()),
            });
        }
        for _ in cursor..self.drawing {
            output.write_all(&zero)?;
        }
        Ok(records)
    }
}

pub fn blocks(bytes: u32) -> u32 {
    bytes.div_ceil(BLOCK)
}
fn ucs2(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_be_bytes).collect()
}
fn record_size(name: usize) -> usize {
    33 + name + usize::from(name.is_multiple_of(2))
}
fn dual16(out: &mut [u8], value: u16) {
    out[..2].copy_from_slice(&value.to_le_bytes());
    out[2..4].copy_from_slice(&value.to_be_bytes());
}
fn dual32(out: &mut [u8], value: u32) {
    out[..4].copy_from_slice(&value.to_le_bytes());
    out[4..8].copy_from_slice(&value.to_be_bytes());
}
fn record(lba: u32, bytes: u32, directory: bool, name: &[u8]) -> Vec<u8> {
    let mut result = vec![0; record_size(name.len())];
    result[0] = result.len() as u8;
    dual32(&mut result[2..10], lba);
    dual32(&mut result[10..18], bytes);
    result[18..25].copy_from_slice(&[100, 1, 1, 0, 0, 0, 0]);
    result[25] = if directory { 2 } else { 0 };
    dual16(&mut result[28..32], 1);
    result[32] = name.len() as u8;
    result[33..33 + name.len()].copy_from_slice(name);
    result
}

#[cfg(test)]
mod tests {
    use super::extent_offset;

    #[test]
    fn absolute_extents_respect_both_session_boundaries() {
        let offset = |lba, bytes| extent_offset(12_300, 8, lba, bytes);
        assert_eq!(offset(12_300, 16_384), Some(0));
        assert_eq!(offset(12_307, 2048), Some(14_336));
        assert_eq!(offset(12_308, 0), Some(16_384));
        assert_eq!(offset(12_299, 0), None);
        assert_eq!(offset(12_307, 2049), None);
        assert_eq!(offset(12_308, 1), None);
        assert_eq!(offset(12_309, 0), None);
        assert_eq!(offset(12_301, u64::MAX), None);
    }
}
