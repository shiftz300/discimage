//! Native CD recorder using IMAPI2 reservation and aligned MMC command transport.
//! MMC SAO preserves the project's exact audio gaps; TAO appends cooked Mode 1.
use crate::{
    audio, author,
    codec::{COOKED, RAW},
    console,
    device::{self, Device, Identity, Music},
    mmc,
    probe::transport::Reply,
    project::CdText,
};
use anyhow::{Context, Result, ensure};
use std::{
    ffi::c_void,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
    time::{Duration, Instant},
};

const CHUNK: u32 = 24; // 56 KiB raw; below the transport's 64 KiB aligned buffer.

trait Transport {
    fn execute(&mut self, cdb: &[u8], bytes: &mut [u8], write: bool, timeout: u32)
    -> Result<Reply>;
    fn reserve(&mut self) -> Result<()>;
    fn write_speeds(&mut self) -> Result<Vec<u32>> {
        Ok(Vec::new())
    }
}

unsafe extern "C" {
    fn discimage_windows_open(drive: *const u16, output: *mut *mut c_void) -> i32;
    fn discimage_windows_reserve(handle: *mut c_void) -> i32;
    fn discimage_windows_command(
        handle: *mut c_void,
        cdb: *mut u8,
        length: u32,
        data: *mut u8,
        capacity: u32,
        write: bool,
        timeout: u32,
        used: *mut u32,
        status: *mut u8,
        sense: *mut u8,
        times: *mut f64,
    ) -> i32;
    fn discimage_windows_close(handle: *mut c_void);
}

struct Imapi {
    handle: *mut c_void,
}
impl Imapi {
    fn new(id: &str) -> Result<Self> {
        ensure!(
            id.len() == 2 && id.as_bytes()[0].is_ascii_alphabetic() && id.ends_with(':'),
            "expected a Windows optical drive letter"
        );
        let drive = id.encode_utf16().chain([0]).collect::<Vec<_>>();
        let mut handle = std::ptr::null_mut();
        let code = unsafe { discimage_windows_open(drive.as_ptr(), &mut handle) };
        ensure!(
            code >= 0 && !handle.is_null(),
            "cannot open {id} through Windows IMAPI2 (HRESULT 0x{:08x})",
            code as u32
        );
        Ok(Self { handle })
    }
}
impl Drop for Imapi {
    fn drop(&mut self) {
        unsafe { discimage_windows_close(self.handle) };
    }
}

impl Transport for Imapi {
    fn write_speeds(&mut self) -> Result<Vec<u32>> {
        // Query MMC directly: constructing a format writer just to query speeds
        // can also initialize its write mode, clobbering the prepared SAO page.
        let reply = self.execute(
            &[0xac, 0, 0, 0, 0, 0, 0, 0, 0, 128, 3, 0],
            &mut vec![0; 2056],
            false,
            30,
        )?;
        if reply.status != 0 && reply.key() == 5 && matches!(reply.asc().0, 0x20 | 0x24) {
            return Ok(Vec::new());
        }
        mmc::write_speeds(&reply.good()?.data)
    }
    fn execute(
        &mut self,
        cdb: &[u8],
        bytes: &mut [u8],
        write: bool,
        timeout: u32,
    ) -> Result<Reply> {
        ensure!(
            (6..=16).contains(&cdb.len()) && bytes.len() <= 65536,
            "invalid bounded MMC command"
        );
        let mut command = cdb.to_vec();
        let (mut used, mut status, mut sense) = (0, 0, [0; 32]);
        let mut times = [0.0; 2];
        let trace = std::env::var_os("DISCIMAGE_TRACE_MMC").is_some();
        if trace {
            console::phase(
                "mmc",
                format_args!(
                    "CDB {} | {} bytes | {} | payload {}",
                    crate::hex(cdb),
                    bytes.len(),
                    if write { "out" } else { "in" },
                    if write {
                        crate::hex(&bytes[..bytes.len().min(128)])
                    } else {
                        String::new()
                    }
                ),
            );
        }
        let code = unsafe {
            discimage_windows_command(
                self.handle,
                command.as_mut_ptr(),
                command.len() as u32,
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
        if trace {
            console::phase(
                "mmc",
                format_args!(
                    "HRESULT {:08x} | status {status:02x} | used {used} | sense {} | data {}",
                    code as u32,
                    crate::hex(&sense),
                    if !write {
                        crate::hex(&bytes[..(used as usize).min(bytes.len()).min(256)])
                    } else {
                        String::new()
                    }
                ),
            );
        }
        ensure!(
            code >= 0,
            "Windows IMAPI2 command {:02x} failed (HRESULT 0x{:08x}, sense {:02x?}); command was not retried",
            cdb[0],
            code as u32,
            sense
        );
        ensure!(
            used as usize <= bytes.len(),
            "invalid IMAPI2 transfer length"
        );
        ensure!(
            times.iter().all(|v| v.is_finite() && *v >= 0.0),
            "invalid native command timestamps"
        );
        Ok(Reply {
            data: bytes[..used as usize].to_vec(),
            status,
            sense,
            end: times[0],
            duration: times[1],
        })
    }
    fn reserve(&mut self) -> Result<()> {
        let code = unsafe { discimage_windows_reserve(self.handle) };
        ensure!(
            code >= 0,
            "cannot reserve the Windows optical drive (IMAPI2 HRESULT 0x{:08x}); close other disc applications",
            code as u32
        );
        Ok(())
    }
}

pub(crate) struct Recorder {
    id: String,
    link: Box<dyn Transport>,
    locked: bool,
}

/// The timing API accepts only the bounded read-only Command enum. Reservation
/// and tray prevention have the same lifetime as macOS's exclusive probe handle.
pub(crate) struct ProbeDevice {
    link: Imapi,
}
impl ProbeDevice {
    pub(crate) fn open(id: &str) -> Result<Self> {
        let mut link = Imapi::new(id)?;
        link.reserve()?;
        if let Err(error) = link
            .execute(&[0x1e, 0, 0, 0, 1, 0], &mut [], false, 10)
            .and_then(Reply::good)
        {
            let _ = link.execute(&[0x1e, 0, 0, 0, 0, 0], &mut [], false, 10);
            return Err(error).context("cannot prevent media removal during CD timing");
        }
        Ok(Self { link })
    }
}
impl crate::probe::transport::Link for ProbeDevice {
    fn command(&mut self, command: crate::probe::transport::Command) -> Result<Reply> {
        let (packet, length, transfer) = command.packet();
        ensure!(
            transfer <= 32 * COOKED,
            "timing transfer exceeds read limit"
        );
        self.link.execute(
            &packet[..usize::from(length)],
            &mut vec![0; transfer],
            false,
            5,
        )
    }
}
impl Drop for ProbeDevice {
    fn drop(&mut self) {
        let _ = self
            .link
            .execute(&[0x1e, 0, 0, 0, 0, 0], &mut [], false, 10);
    }
}

impl Recorder {
    pub(crate) fn new(id: &str) -> Result<Self> {
        Ok(Self {
            id: id.to_ascii_uppercase(),
            link: Box::new(Imapi::new(id)?),
            locked: false,
        })
    }

    fn raw(&mut self, cdb: &[u8], capacity: usize, timeout: u32) -> Result<Reply> {
        self.link
            .execute(cdb, &mut vec![0; capacity], false, timeout)
    }

    fn read(&mut self, cdb: &[u8], capacity: usize) -> Result<Vec<u8>> {
        self.raw(cdb, capacity, 30)?
            .good()
            .with_context(|| format!("MMC command {:02x} on {}", cdb[0], self.id))
            .map(|r| r.data)
    }

    fn send(&mut self, cdb: &[u8], bytes: &mut [u8], timeout: u32) -> Result<()> {
        let reply = self.link.execute(cdb, bytes, true, timeout)?;
        let reply = reply.good().with_context(|| {
            format!(
                "MMC command {:02x} on {}; inspect the saved job before retrying",
                cdb[0], self.id
            )
        })?;
        ensure!(
            reply.data.len() == bytes.len(),
            "short MMC data-out transfer; write was not retried"
        );
        Ok(())
    }

    fn track_info(&mut self, number: u32) -> Result<Vec<u8>> {
        let mut cdb = [0; 10];
        cdb[0] = 0x52;
        cdb[1] = 1;
        cdb[2..6].copy_from_slice(&number.to_be_bytes());
        cdb[8] = 48;
        self.read(&cdb, 48)
    }

    fn disc_info(&mut self) -> Result<Vec<u8>> {
        self.read(&[0x51, 0, 0, 0, 0, 0, 0, 0, 34, 0], 34)
    }

    fn ready(&mut self) -> Result<bool> {
        for _ in 0..8 {
            let reply = self.raw(&[0; 6], 0, 10)?;
            if reply.status == 0 {
                return Ok(true);
            }
            if reply.key() == 6 {
                continue;
            } // Unit attention after insertion / close.
            if reply.key() == 2 && reply.asc().0 == 0x3a {
                return Ok(false);
            }
            reply.good()?;
        }
        anyhow::bail!("drive repeatedly reported unit attention; check the connection")
    }

    fn observe(&mut self) -> Result<Device> {
        let inquiry = self.read(&[0x12, 0, 0, 0, 36, 0], 36)?;
        ensure!(
            inquiry.len() >= 36 && inquiry[0] & 31 == 5,
            "selected device is not an MMC optical drive"
        );
        let field = |range| String::from_utf8_lossy(&inquiry[range]).trim().to_string();
        let name = format!("{} {} ({})", field(8..16), field(16..32), field(32..36));
        let serial_config = self.read(&[0x46, 2, 1, 8, 0, 0, 0, 2, 0, 0], 512).ok();
        let hardware = serial_config
            .as_ref()
            .and_then(|b| Identity::from_configuration(&inquiry, b))
            .or_else(|| {
                self.read(&[0x12, 1, 0x80, 0, 255, 0], 255)
                    .ok()
                    .and_then(|b| Identity::from_inquiry(&inquiry, &b))
            })
            .filter(|_| !mmc::apple_superdrive_proxy(&inquiry));
        // RT=0 includes supported profiles even if no medium is present.
        let ready = self.ready()?;
        let config = self.read(&[0x46, 0, 0, 0, 0, 0, 0, 0x10, 0, 0], 4096)?;
        let (profile, caps) = mmc::configuration(&config)?;
        if ready && std::env::var_os("DISCIMAGE_TRACE_MMC").is_some() {
            // Optional read-only bring-up evidence, never a mode change.
            for page in [5, 0x85, 0x2a] {
                let _ = self.read(&[0x5a, 8, page, 0, 0, 0, 0, 1, 0, 0], 256);
            }
            let _ = self.read(&[0x43, 0, 4, 0, 0, 0, 0, 0, 64, 0], 64);
            let _ = self.read(&[0xac, 0, 0, 0, 0, 0, 0, 0, 0, 16, 3, 0], 264);
        }
        let mut drive = Device {
            hardware,
            id: self.id.clone(),
            name,
            backend: "windows-imapi2-mmc".into(),
            capabilities: Some(caps),
            media: None,
            tracks: Vec::new(),
        };
        if !ready {
            return Ok(drive);
        }
        let disc = self.disc_info()?;
        let mut media = mmc::media(profile, &disc, self.locked)?;
        // Non-CD media remain visible, but never enter the CD write path.
        if !matches!(profile, 8..=10) {
            drive.media = Some(media);
            return Ok(drive);
        }
        for number in 1..=media.tracks {
            let track = mmc::track(&self.track_info(number)?)?;
            ensure!(
                track.number == number,
                "drive returned the wrong track descriptor"
            );
            drive.tracks.push(track);
        }
        if media.blank || media.appendable {
            // 0xff is MMC's invisible / incomplete track selector. Never infer NWA
            // from audio length, a TOC lead-out, or the estimated session offset.
            let info = self.track_info(0xff)?;
            let writable = mmc::track(&info)?;
            media.nwa = writable.next_writable;
            if info.get(7).is_some_and(|flags| flags & 1 != 0) {
                media.free_sectors = Some(mmc::be32(&info, 16)?).filter(|free| *free <= 360_000);
            }
        }
        drive.media = Some(media);
        Ok(drive)
    }

    fn wait_ready(&mut self, timeout: Duration) -> Result<()> {
        let start = Instant::now();
        let mut reported = Instant::now();
        loop {
            let reply = self.raw(&[0; 6], 0, 10)?;
            if reply.status == 0 {
                return Ok(());
            }
            ensure!(
                reply.key() == 6 || (reply.key() == 2 && reply.asc().0 == 4),
                "drive operation failed: {}",
                reply.good().err().context("missing SCSI failure")?
            );
            ensure!(
                start.elapsed() < timeout,
                "timed out waiting for the optical drive; inspect the disc before retrying"
            );
            // Once an erase / close has started, wait for it to settle even after
            // Ctrl-C. Stopping the wait cannot safely undo the drive's command.
            if reported.elapsed() >= Duration::from_secs(15) {
                console::phase(
                    "drive",
                    format_args!(
                        "Waiting for completion | {}s elapsed",
                        start.elapsed().as_secs()
                    ),
                );
                reported = Instant::now();
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    fn parameters(&mut self, sao: bool, appendable: bool) -> Result<()> {
        let page = self.read(&[0x5a, 8, 5, 0, 0, 0, 0, 1, 0, 0], 256)?;
        let mut parameters = mmc::write_parameters(&page, sao, appendable)?;
        let mut cdb = [0; 10];
        cdb[0] = 0x55;
        cdb[1] = 0x10;
        cdb[7..9].copy_from_slice(&(parameters.len() as u16).to_be_bytes());
        self.send(&cdb, &mut parameters, 30)
    }

    fn speed(&mut self, speed: Option<u32>) -> Result<()> {
        ensure!(
            speed.is_none_or(|s| (1..=64).contains(&s)),
            "speed must be 1–64x"
        );
        let mut cdb = [0; 12];
        cdb[0] = 0xbb;
        cdb[2..4].fill(0xff);
        let sectors = self.selected_speed(speed)?;
        let rate = sectors.map_or(u16::MAX, |s| {
            ((u64::from(s) * 2352 + 500) / 1000).min(u64::from(u16::MAX - 1)) as u16
        });
        if let Some(sectors) = sectors {
            console::phase(
                "speed",
                format_args!("Requested {sectors} sectors/s ({rate} kB/s)"),
            );
        } else {
            console::phase(
                "speed",
                format_args!("Requested maximum available speed (MMC 0xffff)"),
            );
        }
        cdb[4..6].copy_from_slice(&rate.to_be_bytes());
        self.send(&cdb, &mut [], 30)
    }

    fn selected_speed(&mut self, speed: Option<u32>) -> Result<Option<u32>> {
        let supported = self.link.write_speeds()?;
        if let Some(speed) = speed {
            ensure!(
                supported.is_empty() || supported.contains(&(speed * 75)),
                "requested {speed}x is not supported by this drive/media; supported sectors/s: {supported:?}"
            );
        }
        Ok(speed
            .map(|s| s * 75)
            .or_else(|| supported.iter().max().copied()))
    }

    fn write_blocks(&mut self, lba: i32, blocks: u32, bytes: &mut [u8]) -> Result<()> {
        check_cancel()?;
        ensure!(
            self.locked && (1..=CHUNK).contains(&blocks),
            "write requires a reserved drive and bounded transfer"
        );
        let mut cdb = [0; 10];
        cdb[0] = 0x2a;
        cdb[2..6].copy_from_slice(&lba.to_be_bytes());
        cdb[7..9].copy_from_slice(&(blocks as u16).to_be_bytes());
        // A failed / uncertain write is never resent, even if the driver reports
        // BUSY or unit attention. The journal remains in its writing stage.
        self.send(&cdb, bytes, 120)
            .with_context(|| format!("WRITE(10) at LBA {lba}, {blocks} sectors"))
    }

    fn silence(&mut self, mut lba: i32, mut sectors: u32) -> Result<()> {
        while sectors > 0 {
            let count = sectors.min(CHUNK);
            self.write_blocks(lba, count, &mut vec![0; count as usize * RAW])?;
            lba += count as i32;
            sectors -= count;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        self.send(&[0x35, 2, 0, 0, 0, 0, 0, 0, 0, 0], &mut [], 120)?;
        self.wait_ready(Duration::from_secs(600))
    }

    fn close_data(&mut self) -> Result<()> {
        self.flush()?;
        // TAO flush completes its track; close function 2 finalizes the session.
        // The write-parameters multisession flag is already zero.
        self.send(&[0x5b, 1, 2, 0, 0, 0, 0, 0, 0, 0], &mut [], 120)?;
        self.wait_ready(Duration::from_secs(600))
    }

    fn text_leadin(&mut self) -> Result<i32> {
        let info = self.disc_info()?;
        let info = mmc::declared(&info, 24)?;
        let (m, s, f) = (
            i32::from(info[17]),
            i32::from(info[18]),
            i32::from(info[19]),
        );
        ensure!(
            (90..100).contains(&m) && s < 60 && f < 75,
            "drive did not report a usable first lead-in address for CD-Text"
        );
        let lba = (m * 60 + s) * 75 + f - 150 - 450_000;
        ensure!(
            (-45_000..=-4500).contains(&lba),
            "invalid CD-Text lead-in address {lba}"
        );
        Ok(lba)
    }

    fn write_text(&mut self, start: i32, packs: &[u8]) -> Result<()> {
        let symbols = mmc::text_symbols(packs)?;
        let mut lba = start;
        let mut cursor = 0;
        while lba < -150 {
            let count = ((-150 - lba) as u32).min(CHUNK);
            let mut bytes = vec![0; count as usize * 96];
            for byte in &mut bytes {
                *byte = symbols[cursor];
                cursor = (cursor + 1) % symbols.len();
            }
            self.write_blocks(lba, count, &mut bytes)?;
            lba += count as i32;
        }
        Ok(())
    }

    fn cooked(&mut self, lba: u32, count: u32) -> Result<Vec<u8>> {
        let mut cdb = [0; 10];
        cdb[0] = 0x28;
        cdb[2..6].copy_from_slice(&lba.to_be_bytes());
        cdb[7..9].copy_from_slice(&(count as u16).to_be_bytes());
        let bytes = self.read(&cdb, count as usize * COOKED)?;
        ensure!(
            bytes.len() == count as usize * COOKED,
            "short data readback at LBA {lba}"
        );
        Ok(bytes)
    }

    fn verify_data(&mut self, path: &Path, start: u32, sectors: u32) -> Result<()> {
        console::phase(
            "readback",
            format_args!("Comparing all {sectors} Mode 1 sectors"),
        );
        let mut file = File::open(path)?;
        let mut relative = 0;
        let mut reported = Instant::now();
        while relative < sectors {
            check_cancel()?;
            let count = (sectors - relative).min(CHUNK);
            let actual = self.cooked(start + relative, count)?;
            let mut expected = vec![0; actual.len()];
            file.read_exact(&mut expected)?;
            if let Some(offset) = actual
                .as_chunks::<COOKED>()
                .0
                .iter()
                .zip(expected.as_chunks::<COOKED>().0)
                .position(|(a, b)| a != b)
            {
                anyhow::bail!(
                    "data readback mismatch at LBA {}",
                    start + relative + offset as u32
                );
            }
            relative += count;
            if reported.elapsed() >= Duration::from_secs(15) {
                console::phase(
                    "readback",
                    format_args!("{relative}/{sectors} data sectors matched"),
                );
                reported = Instant::now();
            }
        }
        ensure!(
            file.metadata()?.len() == u64::from(sectors) * COOKED as u64,
            "data image length changed"
        );
        Ok(())
    }

    fn write_data(
        &mut self,
        path: &Path,
        start: u32,
        sectors: u32,
        speed: Option<u32>,
    ) -> Result<()> {
        ensure!(self.locked, "drive must be reserved before data recording");
        let before = self.observe()?;
        let media = before.media.as_ref().context("no media")?;
        ensure!(
            matches!(media.kind.as_str(), "cd-r" | "cd-rw")
                && media.reserved
                && (media.blank || media.appendable)
                && ((media.blank && start == 0) || media.nwa == Some(start))
                && media.free_sectors.is_some_and(|n| n >= sectors),
            "data NWA, capacity or reservation changed; no write attempted"
        );
        let mut file = File::open(path)?;
        ensure!(
            sectors >= 300
                && start.checked_add(sectors).is_some_and(|n| n <= 360_000)
                && file.metadata()?.len() == u64::from(sectors) * COOKED as u64,
            "invalid cooked Mode 1 image extent"
        );
        self.parameters(false, false)?;
        self.speed(speed)?;
        let confirmed = self.track_info(0xff)?;
        ensure!(
            mmc::track(&confirmed)?.next_writable == Some(start),
            "NWA changed after data setup; no write attempted"
        );
        let mut relative = 0;
        let mut reported = Instant::now();
        while relative < sectors {
            let count = (sectors - relative).min(CHUNK);
            let mut bytes = vec![0; count as usize * COOKED];
            file.read_exact(&mut bytes)?;
            self.write_blocks((start + relative) as i32, count, &mut bytes)?;
            relative += count;
            if reported.elapsed() >= Duration::from_secs(15) {
                console::phase(
                    "write",
                    format_args!("{relative}/{sectors} data sectors accepted"),
                );
                reported = Instant::now();
            }
        }
        self.close_data()?;
        self.verify_data(path, start, sectors)
    }

    pub(crate) fn reserve_blank_cdr(id: Option<&str>, capacity: u32) -> Result<Self> {
        Self::reserve_blank_cd(id, capacity, false)
    }

    fn reserve_blank_cd(id: Option<&str>, capacity: u32, rewritable: bool) -> Result<Self> {
        use device::Recorder as _;
        let drives = devices()?;
        let selected = if let Some(id) = id {
            device::resolve(&drives, id)?
        } else {
            let eligible = drives.iter().filter(|d| blank_cd(d, capacity, rewritable));
            let candidates = eligible.collect::<Vec<_>>();
            ensure!(
                candidates.len() == 1,
                "expected one eligible blank CD; connect a drive or specify --device"
            );
            candidates[0]
        };
        ensure!(
            blank_cd(selected, capacity, rewritable),
            "selected drive needs a supported blank CD with sufficient writable capacity"
        );
        let mut recorder = Self::new(&selected.id)?;
        recorder.lock()?;
        ensure!(
            blank_cd(&recorder.observe()?, capacity, rewritable),
            "blank CD changed while reserving the drive"
        );
        Ok(recorder)
    }
}

impl device::Recorder for Recorder {
    fn inspect(&mut self) -> Result<Device> {
        let drive = self.observe()?;
        if self.locked
            && drive
                .media
                .as_ref()
                .is_some_and(|m| !m.blank && m.appendable)
        {
            // NWA is write-mode dependent. After audio SAO, select data TAO
            // before reporting the address used to author the second session.
            self.parameters(false, false)?;
            self.observe()
        } else {
            Ok(drive)
        }
    }
    fn preflight_speed(&mut self, speed: Option<u32>) -> Result<()> {
        self.selected_speed(speed)?;
        Ok(())
    }

    fn lock(&mut self) -> Result<()> {
        if !self.locked {
            self.link.reserve()?;
            if let Err(error) = self.send(&[0x1e, 0, 0, 0, 1, 0], &mut [], 10) {
                // A transport error can occur after PREVENT reached the drive.
                let _ = self.send(&[0x1e, 0, 0, 0, 0, 0], &mut [], 10);
                return Err(error).context("cannot prevent media removal during recording");
            }
            self.locked = true;
        }
        Ok(())
    }

    fn audio_sectors(&mut self, path: &Path) -> Result<Option<u32>> {
        Ok(Some(audio::inspect(path)?.sectors))
    }

    fn preflight_cd_text(&mut self, title: &str, text: &CdText) -> Result<()> {
        mmc::text_packs(title, text)?;
        self.text_leadin()?;
        Ok(())
    }

    fn music(
        &mut self,
        tracks: &[Music],
        speed: Option<u32>,
        text: Option<(&str, &CdText)>,
    ) -> Result<()> {
        ensure!(self.locked, "audio recording requires a reservation");
        let before = self.observe()?;
        let media = before.media.as_ref().context("no media")?;
        ensure!(
            media.blank
                && media.reserved
                && matches!(media.kind.as_str(), "cd-r" | "cd-rw")
                && before.capabilities.as_ref().is_some_and(|c| c.sao),
            "audio SAO requires blank CD-R/CD-RW and a capable reserved drive"
        );
        let ranges = mmc::ranges(tracks);
        let end = ranges.last().context("no audio tracks")?.end;
        ensure!(
            media
                .free_sectors
                .is_some_and(|free| free > end.saturating_add(11_400)),
            "insufficient space for two sessions"
        );
        let cue = mmc::cue(tracks, text.is_some())?;
        let text = text
            .map(|(title, text)| -> Result<_> {
                ensure!(
                    text.titles.len() == tracks.len()
                        && before.capabilities.as_ref().is_some_and(|c| c.cd_text),
                    "CD-Text track count or drive capability mismatch"
                );
                Ok((self.text_leadin()?, mmc::text_packs(title, text)?))
            })
            .transpose()?;
        // Validate and open every WAV before the first irreversible command.
        let mut files = tracks
            .iter()
            .map(|t| -> Result<_> {
                let info = audio::inspect(&t.path)?;
                ensure!(
                    info.sectors == t.sectors && info.padding_sample_frames == 0,
                    "prepared WAV length changed"
                );
                let mut file = File::open(&t.path)?;
                file.seek(SeekFrom::Start(info.data_offset))?;
                Ok(file)
            })
            .collect::<Result<Vec<_>>>()?;
        self.parameters(true, true)?;
        self.speed(speed)?;
        let mut command = [0; 10];
        command[0] = 0x5d;
        let length = (cue.len() as u32).to_be_bytes();
        command[6..9].copy_from_slice(&length[1..]);
        self.send(&command, &mut cue.clone(), 30)?;
        if let Some((start, packs)) = text {
            self.write_text(start, &packs)?;
        }
        // In SAO the first pregap is explicit at signed LBA -150, not part of WAV 1.
        self.silence(-150, 150)?;
        for (i, ((track, range), file)) in tracks.iter().zip(ranges).zip(&mut files).enumerate() {
            if i > 0 {
                self.silence((range.start - track.gap) as i32, track.gap)?;
            }
            console::phase(
                "write",
                format_args!(
                    "Audio track {}/{} | {} sectors",
                    i + 1,
                    tracks.len(),
                    track.sectors
                ),
            );
            let mut lba = range.start;
            let mut reported = Instant::now();
            while lba < range.end {
                let count = (range.end - lba).min(CHUNK);
                let mut bytes = vec![0; count as usize * RAW];
                file.read_exact(&mut bytes)?;
                // MMC CD-DA uses little-endian PCM, matching WAV. Do not apply
                // the network byte order used by the CDB's address fields.
                self.write_blocks(lba as i32, count, &mut bytes)?;
                lba += count;
                if reported.elapsed() >= Duration::from_secs(15) {
                    console::phase(
                        "write",
                        format_args!(
                            "Track {} | {}/{} sectors accepted",
                            i + 1,
                            lba - range.start,
                            track.sectors
                        ),
                    );
                    reported = Instant::now();
                }
            }
        }
        // For SAO SYNCHRONIZE CACHE closes this session according to the CUE.
        self.flush()
    }

    fn verify_music(&mut self, tracks: &[Music]) -> Result<()> {
        for (i, (track, range)) in tracks.iter().zip(mmc::ranges(tracks)).enumerate() {
            let info = audio::inspect(&track.path)?;
            ensure!(
                info.sectors == track.sectors && info.padding_sample_frames == 0,
                "prepared WAV changed before readback"
            );
            let mut file = File::open(&track.path)?;
            file.seek(SeekFrom::Start(info.data_offset))?;
            let mut lba = range.start;
            console::phase(
                "readback",
                format_args!("Comparing audio track {}/{}", i + 1, tracks.len()),
            );
            let mut reported = Instant::now();
            while lba < range.end {
                check_cancel()?;
                let count = (range.end - lba).min(CHUNK);
                let mut cdb = [0; 12];
                cdb[0] = 0xbe;
                cdb[1] = 4; // Expected sector type CD-DA.
                cdb[2..6].copy_from_slice(&lba.to_be_bytes());
                cdb[6..9].copy_from_slice(&count.to_be_bytes()[1..]);
                cdb[9] = 0x10; // User data only.
                let actual = self.read(&cdb, count as usize * RAW)?;
                ensure!(
                    actual.len() == count as usize * RAW,
                    "short audio readback at LBA {lba}"
                );
                let mut expected = vec![0; actual.len()];
                file.read_exact(&mut expected)?;
                if let Some(offset) = actual
                    .as_chunks::<RAW>()
                    .0
                    .iter()
                    .zip(expected.as_chunks::<RAW>().0)
                    .position(|(a, b)| a != b)
                {
                    anyhow::bail!(
                        "PCM differs from WAV in track {} at LBA {}",
                        i + 1,
                        lba + offset as u32
                    );
                }
                lba += count;
                if reported.elapsed() >= Duration::from_secs(15) {
                    console::phase(
                        "readback",
                        format_args!(
                            "Track {} | {}/{} sectors matched",
                            i + 1,
                            lba - range.start,
                            track.sectors
                        ),
                    );
                    reported = Instant::now();
                }
            }
        }
        console::success(format_args!(
            "All {} audio tracks matched byte-for-byte",
            tracks.len()
        ));
        Ok(())
    }

    fn data(&mut self, path: &Path, start: u32, sectors: u32, speed: Option<u32>) -> Result<()> {
        self.write_data(path, start, sectors, speed)
    }

    fn verify_data_tail(&mut self, path: &Path, start: u32, sectors: u32) -> Result<()> {
        ensure!(sectors >= 2, "invalid tail extent");
        let mut file = File::open(path)?;
        file.seek(SeekFrom::Start(u64::from(sectors - 2) * COOKED as u64))?;
        let mut expected = vec![0; 2 * COOKED];
        file.read_exact(&mut expected)?;
        ensure!(
            self.cooked(start + sectors - 2, 2)? == expected,
            "data tail readback mismatch"
        );
        Ok(())
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if self.locked {
            // Release tray prevention on this handle before its driver/volume locks.
            let _ = self.send(&[0x1e, 0, 0, 0, 0, 0], &mut [], 10);
        }
    }
}

fn check_cancel() -> Result<()> {
    ensure!(
        !device::cancelled(),
        "optical operation cancelled; inspect the saved job before retrying"
    );
    Ok(())
}

fn blank_cd(drive: &Device, capacity: u32, rewritable: bool) -> bool {
    let (Some(caps), Some(media)) = (&drive.capabilities, &drive.media) else {
        return false;
    };
    caps.tao
        && match media.kind.as_str() {
            "cd-r" => caps.cdr,
            "cd-rw" => rewritable && caps.cdrw,
            _ => false,
        }
        && media.blank
        && media.free_sectors.is_some_and(|free| free >= capacity)
        && drive.tracks.iter().all(|t| t.empty)
}

pub(crate) fn devices() -> Result<Vec<Device>> {
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
    let mut drives = Vec::new();
    for letter in b'A'..=b'Z' {
        let root = [u16::from(letter), b':' as u16, b'\\' as u16, 0];
        // GetDriveTypeW only examines this NUL-terminated root; no PowerShell/CIM.
        if unsafe { GetDriveTypeW(root.as_ptr()) } == 5 {
            let id = format!("{}:", char::from(letter));
            drives.push(Recorder::new(&id)?.observe().with_context(|| format!("cannot inspect optical drive {id}; close other disc applications and check access"))?);
        }
    }
    Ok(drives)
}

pub(crate) fn erase(
    id: &str,
    quick: bool,
    confirm: impl FnOnce(&Device) -> Result<bool>,
) -> Result<()> {
    use device::Recorder as _;
    let mut recorder = Recorder::new(id)?;
    recorder.lock()?;
    let before = recorder.observe()?;
    ensure!(
        before.media.as_ref().is_some_and(|m| m.kind == "cd-rw"),
        "only CD-RW can be erased"
    );
    if !confirm(&before)? {
        println!("Erase cancelled. No disc contents changed.");
        return Ok(());
    }
    check_cancel()?;
    console::phase(
        "erase",
        format_args!(
            "{} erase of {}",
            if quick { "Quick" } else { "Full" },
            before.name
        ),
    );
    // The preceding failed/new job may have left SAO and multisession selected.
    // Normalize to data TAO and the highest supported speed, like macOS.
    recorder.parameters(false, false)?;
    recorder.speed(None)?;
    let mut cdb = [0; 12];
    cdb[0] = 0xa1;
    cdb[1] = 0x10 | u8::from(quick);
    recorder.send(&cdb, &mut [], 120)?;
    recorder.wait_ready(Duration::from_secs(90 * 60))?;
    let after = recorder.observe()?;
    ensure!(
        after.id == before.id
            && after.media.as_ref().is_some_and(|m| m.kind == "cd-rw"
                && m.blank
                && m.tracks == 0
                && m.sessions == 0
                && m.free_sectors.is_some_and(|n| n > 0)),
        "erase returned without confirmed blank CD-RW state"
    );
    console::success(format_args!(
        "CD-RW erased; blank state and writable capacity confirmed"
    ));
    Ok(())
}

pub(crate) fn record_calibration(
    directory: &Path,
    id: Option<&str>,
    speed: Option<u32>,
    dry_run: bool,
) -> Result<()> {
    ensure!(
        !directory.try_exists()?,
        "use a new calibration output directory"
    );
    device::install_cancel_handler()?;
    let mut recorder = Recorder::reserve_blank_cd(id, 1451, true)?;
    let before = recorder.observe()?;
    let capacity = before
        .media
        .as_ref()
        .and_then(|m| m.free_sectors)
        .context("missing blank CD-R capacity")?;
    recorder.selected_speed(speed)?;
    console::phase(
        "calibration",
        format_args!("Reserved {} | {capacity} writable sectors", before.name),
    );
    if dry_run {
        println!("Calibration preflight passed. No files created or disc write attempted.");
        return Ok(());
    }
    let count = crate::calibrate::prepare_single(directory, capacity, device::cancelled)?;
    let path = directory.join("calibration.iso");
    let digest = author::digest(&path)?;
    recorder.write_data(&path, 0, count, speed)?;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let after = recorder.observe()?;
        ensure!(
            after.id == before.id
                && before
                    .hardware_id()
                    .is_none_or(|expected| after.hardware_id().as_ref() == Some(&expected)),
            "recorded calibration drive identity changed"
        );
        if after.media.as_ref().is_some_and(|media| {
            matches!(media.kind.as_str(), "cd-r" | "cd-rw" | "cd-rom")
                && !media.blank
                && !media.appendable
                && media.sessions == 1
                && media.tracks == 1
        }) && let [track] = after.tracks.as_slice()
            && track.number == 1
            && track.session == 1
            && !track.audio
            && track.closed
            && !track.empty
            && track.start == Some(0)
            && track
                .sectors
                .is_some_and(|n| n == count || n.checked_add(2) == Some(count))
        {
            break;
        }
        ensure!(
            Instant::now() < deadline,
            "could not confirm the finalized calibration track at LBA 0; inspect the disc before retrying"
        );
        std::thread::sleep(Duration::from_millis(250));
    }
    ensure!(
        author::digest(&path)? == digest,
        "calibration image changed during recording"
    );
    let pending = directory.join("pending.json");
    std::fs::hard_link(&pending, directory.join("single.json"))?;
    std::fs::remove_file(pending)?;
    console::success(format_args!(
        "Verified one finalized Mode 1 track at LBA 0..{count}. Target: {}",
        directory.display()
    ));
    Ok(())
}

#[cfg(test)]
mod tests;
