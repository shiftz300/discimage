//! Exercise the shared MMC recorder through the package/journal policy.
use super::*;
use crate::{
    burn::{self, Journal, Stage},
    package,
};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    io::Write,
    rc::Rc,
    sync::atomic::{AtomicUsize, Ordering},
};

#[test]
fn failed_tray_prevention_is_released_without_sending_a_media_write() {
    struct Tray {
        commands: Rc<RefCell<Vec<u8>>>,
        transport_error: bool,
    }
    impl Transport for Tray {
        fn reserve(&mut self) -> Result<()> {
            Ok(())
        }
        fn execute(&mut self, cdb: &[u8], data: &mut [u8], _: bool, _: u32) -> Result<Reply> {
            assert!(data.is_empty());
            assert_eq!(cdb[0], 0x1e);
            self.commands.borrow_mut().push(cdb[4]);
            if self.transport_error && cdb[4] == 1 {
                anyhow::bail!("transport failed after command submission");
            }
            let mut sense = [0; 32];
            sense[0] = 0x70;
            sense[2] = 5;
            sense[12] = 0x24;
            Ok(Reply {
                data: vec![],
                status: if cdb[4] == 1 { 2 } else { 0 },
                sense,
                end: 0.0,
                duration: 0.0,
            })
        }
    }
    for transport_error in [false, true] {
        let commands = Rc::new(RefCell::new(Vec::new()));
        let mut recorder = Recorder {
            id: "D:".into(),
            link: Box::new(Tray {
                commands: commands.clone(),
                transport_error,
            }),
            locked: false,
        };
        assert!(device::Recorder::lock(&mut recorder).is_err());
        assert!(!recorder.locked);
        drop(recorder);
        assert_eq!(*commands.borrow(), [1, 0]);
    }
}

#[test]
fn automatic_speed_uses_highest_descriptor_or_mmc_maximum() {
    struct Speeds(Vec<u32>);
    impl Transport for Speeds {
        fn execute(&mut self, cdb: &[u8], _: &mut [u8], write: bool, _: u32) -> Result<Reply> {
            assert!(write);
            assert_eq!(cdb[0], 0xbb);
            let expected: u16 = if self.0.is_empty() { u16::MAX } else { 1764 };
            assert_eq!(&cdb[4..6], expected.to_be_bytes());
            Ok(Reply {
                data: vec![],
                status: 0,
                sense: [0; 32],
                end: 0.0,
                duration: 0.0,
            })
        }
        fn reserve(&mut self) -> Result<()> {
            Ok(())
        }
        fn write_speeds(&mut self) -> Result<Vec<u32>> {
            Ok(self.0.clone())
        }
    }
    for speeds in [vec![600, 750, 300], vec![]] {
        let mut recorder = Recorder {
            id: "D:".into(),
            link: Box::new(Speeds(speeds)),
            locked: false,
        };
        recorder.speed(None).unwrap();
    }
}

#[derive(Default)]
struct Disc {
    phase: u8,
    sao: bool,
    reserved: bool,
    tracks: Vec<(u32, u32)>,
    audio: BTreeMap<u32, Vec<u8>>,
    data: BTreeMap<u32, Vec<u8>>,
    writes: usize,
    bad_pcm: bool,
    bad_q: bool,
    q_formatted_only: bool,
    lead_out: u32,
    uncertain_write: bool,
    power_failure: bool,
    text_sectors: u32,
}

impl Disc {
    fn nwa(&self) -> u32 {
        if self.tracks.is_empty() {
            0
        } else {
            self.lead_out + 11_400
        }
    }

    fn position_q(&self, address: u32) -> [u8; 12] {
        let (i, start) = self
            .tracks
            .iter()
            .enumerate()
            .find_map(|(i, &(start, length))| {
                let from = i.checked_sub(1).map_or(0, |previous| {
                    let (start, length) = self.tracks[previous];
                    start + length
                });
                (from..start + length)
                    .contains(&address)
                    .then_some((i, start))
            })
            .unwrap();
        let index = u8::from(address >= start && !(self.bad_q && i > 0));
        let bcd = |n: u32| (((n / 10) << 4) | (n % 10)) as u8;
        let msf = |n: u32| [bcd(n / 4500), bcd(n / 75 % 60), bcd(n % 75)];
        let mut q = [1, bcd(i as u32 + 1), index, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        q[3..6].copy_from_slice(&msf(address.abs_diff(start)));
        q[7..10].copy_from_slice(&msf(address + 150));
        let crc = mmc::crc16(&q[..10]);
        q[10..].copy_from_slice(&crc.to_be_bytes());
        q
    }

    fn command(&mut self, cdb: &[u8], bytes: &mut [u8], write: bool) -> Result<Vec<u8>> {
        let result = match cdb[0] {
            0 => Vec::new(),
            0x12 if cdb[1] == 0 => {
                let mut b = vec![b' '; 36];
                b[0] = 5;
                b[8..16].copy_from_slice(b"FIXTURE ");
                b[16..32].copy_from_slice(b"MMC CD WRITER   ");
                b
            }
            0x12 => vec![5, 0x80, 0, 4, b'A', b'B', b'C', b'D'],
            0x46 => {
                let mut b = vec![0; 8];
                b[7] = 9;
                b.extend([
                    0, 0, 1, 8, 0, 9, 1, 0, 0, 10, 0, 0, 0, 0x2d, 1, 4, 0, 0, 0, 0, 0, 0x2e, 1, 4,
                    0x21, 0, 0, 0,
                ]);
                let len = b.len() as u32 - 4;
                b[..4].copy_from_slice(&len.to_be_bytes());
                b
            }
            0x51 => {
                let mut b = vec![0; 34];
                b[1] = 32;
                b[2] = self.phase;
                b[4] = if self.phase < 2 || self.tracks.is_empty() {
                    1
                } else {
                    2
                };
                b[6] = if self.phase == 0 {
                    1
                } else {
                    self.tracks.len() as u8 + u8::from(self.phase == 2)
                };
                b[17..20].copy_from_slice(&[99, 2, 0]);
                b
            }
            0x52 => {
                let number = u32::from_be_bytes(cdb[2..6].try_into()?);
                let mut b = vec![0; 36];
                b[1] = 34;
                if number == 0xff {
                    b[2] = self.tracks.len() as u8 + 1;
                    b[3] = if self.phase == 0 { 1 } else { 2 };
                    b[6] = 0x41;
                    b[7] = 1;
                    b[8..12].copy_from_slice(&self.nwa().to_be_bytes());
                    b[12..16].copy_from_slice(&self.nwa().to_be_bytes());
                    b[16..20].copy_from_slice(&(16_300 - self.nwa()).to_be_bytes());
                } else {
                    let index = number as usize - 1;
                    b[2] = number as u8;
                    let (start, length) = if index < self.tracks.len() {
                        b[3] = 1;
                        self.tracks[index]
                    } else {
                        b[3] = if self.tracks.is_empty() { 1 } else { 2 };
                        b[5] = 4;
                        b[6] = 1;
                        (self.nwa(), self.data.len() as u32)
                    };
                    b[8..12].copy_from_slice(&start.to_be_bytes());
                    b[24..28].copy_from_slice(&length.to_be_bytes());
                }
                b
            }
            0x5a => {
                let mut b = vec![0; 60];
                b[1] = 58;
                b[8] = 5;
                b[9] = 50;
                b[10] = 0x40;
                b
            }
            0x55 => {
                assert!(write && self.reserved);
                self.sao = bytes[10] & 15 == 2;
                assert_eq!(bytes[12], if self.sao { 3 } else { 8 });
                Vec::new()
            }
            0xbb | 0x1e => Vec::new(),
            0x54 => panic!("do not require a separate optional OPC command"),
            0x5d => {
                assert!(write && self.reserved && self.sao);
                let entries = bytes.as_chunks::<8>().0;
                assert_eq!(entries[0], [1, 0, 0, 0x41, 0, 99, 2, 0]);
                let lba = |e: &[u8; 8]| {
                    (u32::from(e[5]) * 60 + u32::from(e[6])) * 75 + u32::from(e[7]) - 150
                };
                let starts = entries
                    .iter()
                    .filter(|e| e[1] != 0xaa && e[2] == 1)
                    .map(|e| (e[1], lba(e)))
                    .collect::<Vec<_>>();
                self.lead_out = lba(entries.last().unwrap());
                self.tracks = starts
                    .iter()
                    .enumerate()
                    .map(|(i, &(number, start))| {
                        let next = entries
                            .iter()
                            .find(|e| e[1] == number + 1)
                            .unwrap_or(entries.last().unwrap());
                        assert_eq!(i + 1, number as usize);
                        (start, lba(next) - start)
                    })
                    .collect();
                Vec::new()
            }
            0x2a => {
                assert!(write && self.reserved);
                self.writes += 1;
                let lba = i32::from_be_bytes(cdb[2..6].try_into()?);
                let count = usize::from(u16::from_be_bytes(cdb[7..9].try_into()?));
                if self.uncertain_write && lba == 0 {
                    anyhow::bail!("injected uncertain SPTI write");
                }
                if lba < -150 {
                    assert_eq!(bytes.len(), count * 96);
                    assert!(bytes.iter().all(|b| *b < 64));
                    self.text_sectors += count as u32;
                } else if lba >= 0 {
                    let sector = if self.sao { RAW } else { COOKED };
                    assert_eq!(bytes.len(), count * sector);
                    let map = if self.sao {
                        &mut self.audio
                    } else {
                        &mut self.data
                    };
                    for (i, b) in bytes.chunks_exact(sector).enumerate() {
                        map.insert(lba as u32 + i as u32, b.to_vec());
                    }
                }
                Vec::new()
            }
            0x35 => {
                if self.sao {
                    self.phase = 1;
                }
                Vec::new()
            }
            0x5b => {
                assert_eq!(cdb[2], 2);
                self.phase = 2;
                Vec::new()
            }
            0xbe | 0x28 => {
                let lba = u32::from_be_bytes(cdb[2..6].try_into()?);
                let count = if cdb[0] == 0xbe {
                    u32::from_be_bytes([0, cdb[6], cdb[7], cdb[8]])
                } else {
                    u32::from(u16::from_be_bytes(cdb[7..9].try_into()?))
                };
                let map = if cdb[0] == 0xbe {
                    &self.audio
                } else {
                    &self.data
                };
                let mut out = Vec::new();
                for i in 0..count {
                    let address = lba + i;
                    out.extend(map.get(&address).context("read beyond recorded sectors")?);
                    if cdb[0] == 0xbe && cdb[10] != 0 {
                        if self.q_formatted_only && cdb[10] == 1 {
                            anyhow::bail!("drive does not support raw P-W");
                        }
                        let q = self.position_q(address);
                        if cdb[10] == 1 {
                            out.extend(q.iter().flat_map(|byte| {
                                (0..8).rev().map(move |bit| (byte >> bit & 1) << 6)
                            }));
                        } else {
                            out.extend(q);
                            out.extend([0; 4]);
                        }
                    }
                }
                if self.bad_pcm && cdb[0] == 0xbe && cdb[10] == 0 {
                    out[0] ^= 1;
                }
                out
            }
            op => anyhow::bail!("unexpected MMC opcode {op:02x}"),
        };
        Ok(if write { bytes.to_vec() } else { result })
    }
}

struct Simulated(Rc<RefCell<Disc>>);

impl Transport for Simulated {
    fn write_speeds(&mut self) -> Result<Vec<u32>> {
        Ok(vec![300, 600, 750])
    }
    fn reserve(&mut self) -> Result<()> {
        self.0.borrow_mut().reserved = true;
        Ok(())
    }
    fn execute(
        &mut self,
        cdb: &[u8],
        bytes: &mut [u8],
        write: bool,
        _timeout: u32,
    ) -> Result<Reply> {
        if cdb[0] == 0x2a && self.0.borrow().power_failure {
            let mut sense = [0; 32];
            sense[0] = 0x70;
            sense[2] = 3;
            sense[12] = 0x73;
            sense[13] = 3;
            return Ok(Reply {
                data: Vec::new(),
                status: 2,
                sense,
                end: 0.0,
                duration: 0.01,
            });
        }
        let data = self.0.borrow_mut().command(cdb, bytes, write)?;
        Ok(Reply {
            data,
            status: 0,
            sense: [0; 32],
            end: 0.0,
            duration: 0.01,
        })
    }
}

#[test]
fn calibration_reuse_requires_the_matching_cd_media_write_capability() {
    let disk = Rc::new(RefCell::new(Disc::default()));
    let mut recorder = Recorder {
        id: "D:".into(),
        link: Box::new(Simulated(disk)),
        locked: false,
    };
    let mut drive = recorder.observe().unwrap();
    assert!(blank_cd(&drive, 14000, false));
    assert!(!blank_cd(&drive, 20000, true));
    drive.media.as_mut().unwrap().kind = "cd-rw".into();
    assert!(!blank_cd(&drive, 14000, false));
    assert!(blank_cd(&drive, 14000, true));
    drive.capabilities.as_mut().unwrap().cdrw = false;
    assert!(!blank_cd(&drive, 14000, true));
}

#[test]
fn mmc_calibration_writes_and_reads_one_mode1_track_at_lba_zero() {
    use device::Recorder as _;
    let root =
        std::env::temp_dir().join(format!("discimage-mmc-calibration-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("calibration.iso");
    let image = vec![0xa5; 300 * COOKED];
    std::fs::write(&path, &image).unwrap();
    let disk = Rc::new(RefCell::new(Disc::default()));
    let mut recorder = Recorder {
        id: "D:".into(),
        link: Box::new(Simulated(disk.clone())),
        locked: false,
    };
    recorder.lock().unwrap();
    recorder.write_data(&path, 0, 300, Some(8)).unwrap();
    let drive = recorder.observe().unwrap();
    let media = drive.media.unwrap();
    assert_eq!((media.sessions, media.tracks), (1, 1));
    assert!(!media.blank && !media.appendable);
    let [track] = drive.tracks.as_slice() else {
        panic!("expected one data track")
    };
    assert_eq!((track.start, track.sectors), (Some(0), Some(300)));
    assert!(!track.audio && track.closed);
    assert_eq!(
        disk.borrow()
            .data
            .values()
            .flatten()
            .copied()
            .collect::<Vec<_>>(),
        image
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn mmc_checks_cd_text_gaps_q_pcm_nwa_and_uncertain_job_state() {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    for fault in 0..8 {
        let root = std::env::temp_dir().join(format!(
            "discimage-mmc-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let source = root.join("one.wav");
        audio::silence(&source).unwrap();
        let mut wav = std::fs::OpenOptions::new()
            .write(true)
            .open(&source)
            .unwrap();
        wav.seek(SeekFrom::Start(44)).unwrap();
        wav.write_all(&[0x12, 0x34, 0x56, 0x78]).unwrap();
        drop(wav);
        let project = root.join("album.toml");
        let mut config = "title='Album'\ntracks=[{path='one.wav'},{path='one.wav'}]\n[disc]\ncapacity_sectors=16300\ndata_reserve_sectors=300\nintertrack_gap_sectors=75\n[cd_text]\nartist='Artist'\ntitles=['One','Two']\n".to_owned();
        if fault >= 5 {
            config = config.replace(
                "intertrack_gap_sectors=75",
                "intertrack_gap_sectors=75\nintertrack_gap_position='tail'",
            );
        }
        std::fs::write(&project, config).unwrap();
        let job = root.join("job");
        package::build(&project, &job, None, None).unwrap();
        let disk = Rc::new(RefCell::new(Disc {
            bad_pcm: fault == 1,
            bad_q: fault == 6,
            q_formatted_only: fault == 7,
            uncertain_write: fault == 2,
            power_failure: fault == 3,
            ..Disc::default()
        }));
        let mut recorder = Recorder {
            id: "D:".into(),
            link: Box::new(Simulated(disk.clone())),
            locked: false,
        };
        let result = burn::record(&mut recorder, &job, Some(if fault == 4 { 5 } else { 8 }));
        if fault == 4 {
            assert!(result.is_err());
            assert!(package::Package::load(&job).unwrap().journal.is_none());
            assert_eq!(disk.borrow().writes, 0);
            drop(recorder);
            std::fs::remove_dir_all(root).unwrap();
            continue;
        }
        let journal = Journal::load(&job).unwrap();
        if !matches!(fault, 0 | 5 | 7) {
            assert!(
                result
                    .as_ref()
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("manifest.toml")
            );
        }
        match fault {
            0 | 5 | 7 => {
                assert_eq!(result.unwrap().stage, Stage::Complete);
                assert_eq!(journal.nwa, Some(12_075));
                let first = if fault == 0 { 300 } else { 375 };
                assert_eq!(disk.borrow().tracks, [(0, first), (375, 300)]);
                assert!(disk.borrow().audio[&374].iter().all(|b| *b == 0));
                // MMC's wire format is WAV/Intel order, independently pinned
                // against cdrecord's CDR_SWABAUDIO behavior for MMC writers.
                assert_eq!(&disk.borrow().audio[&0][..4], [0x12, 0x34, 0x56, 0x78]);
                assert!(!disk.borrow().data.is_empty());
                assert_eq!(disk.borrow().text_sectors, 4350);
            }
            1 | 6 => {
                assert!(result.is_err());
                assert_eq!(journal.stage, Stage::MusicWritten);
                assert!(disk.borrow().data.is_empty());
                if fault == 6 {
                    assert!(
                        journal
                            .error
                            .as_deref()
                            .unwrap()
                            .contains("Q subchannel mismatch at LBA 375")
                    );
                }
            }
            _ => {
                assert!(result.is_err());
                assert_eq!(journal.stage, Stage::MusicWriting);
                if fault == 3 {
                    assert_eq!(disk.borrow().writes, 0);
                    assert!(
                        journal
                            .error
                            .as_ref()
                            .unwrap()
                            .contains("power calibration")
                    );
                }
                let writes = disk.borrow().writes;
                assert!(burn::record(&mut recorder, &job, None).is_err());
                assert_eq!(disk.borrow().writes, writes);
            }
        }
        drop(recorder);
        std::fs::remove_dir_all(root).unwrap();
    }
}
