//! Shared MMC recorder. Platform transports own native access and reservation.
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
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
    time::{Duration, Instant},
};

const CHUNK: u32 = 24; // Below the platform transports' 64 KiB limit.

pub(crate) trait Transport {
    fn execute(&mut self, cdb: &[u8], bytes: &mut [u8], write: bool, timeout: u32)
    -> Result<Reply>;
    fn reserve(&mut self) -> Result<()>;
    fn confirm_reservation(&mut self, _actual: &Device) -> Result<()> {
        Ok(())
    }
    fn snapshot(&self) -> Result<Option<Device>> {
        Ok(None)
    }
    fn requires_reservation_for_reads(&self) -> bool {
        false
    }
    fn write_speeds(&mut self) -> Result<Vec<u32>> {
        let reply = self.execute(
            &[0xac, 0, 0, 0, 0, 0, 0, 0, 0, 128, 3, 0],
            &mut [0; 2056],
            false,
            30,
        )?;
        if reply.status != 0 && reply.key() == 5 && matches!(reply.asc().0, 0x20 | 0x24) {
            return Ok(Vec::new());
        }
        mmc::write_speeds(&reply.good()?.data)
    }
}

fn native_id(id: &str) -> String {
    if cfg!(windows) {
        id.to_ascii_uppercase()
    } else {
        id.to_owned()
    }
}

fn backend() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos-iokit-mmc"
    } else {
        "windows-imapi2-mmc"
    }
}

#[cfg(target_os = "macos")]
fn open_transport(id: &str) -> Result<Box<dyn Transport>> {
    Ok(Box::new(crate::macos::RecordingLink::new(id)?))
}

#[cfg(windows)]
fn open_transport(id: &str) -> Result<Box<dyn Transport>> {
    Ok(Box::new(crate::windows::Imapi::new(id)?))
}

pub(crate) struct Recorder {
    id: String,
    link: Box<dyn Transport>,
    locked: bool,
}

impl Recorder {
    pub(crate) fn new(id: &str) -> Result<Self> {
        Ok(Self {
            id: native_id(id),
            link: open_transport(id)?,
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

    pub(crate) fn observe(&mut self) -> Result<Device> {
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
            backend: backend().into(),
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

    fn parameters(&mut self, sao: bool, appendable: bool, text: bool) -> Result<()> {
        let page = self.read(&[0x5a, 8, 5, 0, 0, 0, 0, 1, 0, 0], 256)?;
        let mut parameters = mmc::write_parameters(&page, sao, appendable)?;
        if text {
            parameters[12] = 3; // Raw P–W for the CD-Text lead-in.
        }
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
        if let Some(lba) = mmc::leadin(&info[17..20]) {
            return Ok(lba);
        }
        let atip = self.read(&[0x43, 0, 4, 0, 0, 0, 0, 0, 64, 0], 64)?;
        mmc::leadin(&mmc::declared(&atip, 12)?[8..11])
            .context("drive did not report a usable first lead-in address for CD-Text")
    }

    fn write_text(&mut self, start: i32, symbols: &[u8]) -> Result<()> {
        console::phase("write", format_args!("Writing CD-Text lead-in"));
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

    fn read_audio(&mut self, lba: u32, count: u32, subchannel: u8) -> Result<Vec<u8>> {
        let mut cdb = [0; 12];
        cdb[0] = 0xbe;
        cdb[1] = 4; // Expected sector type CD-DA.
        cdb[2..6].copy_from_slice(&lba.to_be_bytes());
        cdb[6..9].copy_from_slice(&count.to_be_bytes()[1..]);
        cdb[9] = 0x10;
        cdb[10] = subchannel;
        let stride = RAW
            + match subchannel {
                1 => 96,
                2 => 16,
                _ => 0,
            };
        let bytes = self.read(&cdb, count as usize * stride)?;
        ensure!(
            bytes.len() == count as usize * stride,
            "short audio readback at LBA {lba}"
        );
        Ok(bytes)
    }

    fn verify_indexes(&mut self, tracks: &[Music]) -> Result<()> {
        let ranges = mmc::ranges(tracks);
        let mut subchannel = 1;
        for (i, range) in ranges.iter().enumerate() {
            check_cancel()?;
            let start = range.start;
            let mut check =
                |first: u32, last: u32, number: usize, index: u8, origin: u32| -> Result<()> {
                    let count = last - first;
                    let mut result = self.read_audio(first, count, subchannel);
                    // The raw P–W and formatted Q forms are optional MMC commands.
                    // Try the other form on a transfer failure, never on a mismatch.
                    if subchannel == 1 && result.is_err() && !device::cancelled() {
                        result = self.read_audio(first, count, 2).or(result);
                        if result.is_ok() {
                            subchannel = 2;
                        }
                    }
                    let bytes = result.context("cannot read audio Q subchannel")?;
                    let size = if subchannel == 1 { 96 } else { 16 };
                    mmc::verify_q(
                        bytes.chunks_exact(RAW + size).map(|s| &s[RAW..]),
                        first,
                        number as u8,
                        index,
                        origin,
                    )
                };
            if i > 0 {
                let previous = &ranges[i - 1];
                let end = previous.end;
                check(end - 8, end, i, 1, previous.start)?;
                if tracks[i].gap > 0 {
                    check(end, start.min(end + 8), i + 1, 0, start)?;
                    if start > end + 8 {
                        check(start.saturating_sub(8).max(end + 8), start, i + 1, 0, start)?;
                    }
                }
            }
            check(start, start + 8, i + 1, 1, start)?;
        }
        console::success(format_args!(
            "Audio Q subchannel matches the planned track boundaries"
        ));
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
        let before = device::Recorder::inspect(self)?;
        let media = before.media.as_ref().context("no media")?;
        ensure!(
            matches!(media.kind.as_str(), "cd-r" | "cd-rw")
                && media.reserved
                && (media.blank || media.appendable)
                && ((media.blank && start == 0) || media.nwa == Some(start))
                && media.free_sectors.is_some_and(|n| n >= sectors)
                && before.capabilities.as_ref().is_some_and(|caps| {
                    caps.tao
                        && if media.kind == "cd-r" {
                            caps.cdr
                        } else {
                            caps.cdrw
                        }
                }),
            "data NWA, capacity or reservation changed; no write attempted"
        );
        let mut file = File::open(path)?;
        ensure!(
            sectors >= 300
                && start.checked_add(sectors).is_some_and(|n| n <= 360_000)
                && file.metadata()?.len() == u64::from(sectors) * COOKED as u64,
            "invalid cooked Mode 1 image extent"
        );
        self.parameters(false, false, false)?;
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
        let drives = device::list()?;
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
        let locked = recorder.observe()?;
        ensure!(
            same_drive(selected, &locked)
                && blank_cd(&locked, capacity, rewritable)
                && locked
                    .media
                    .as_ref()
                    .zip(selected.media.as_ref())
                    .is_some_and(|(a, b)| { a.kind == b.kind && a.free_sectors == b.free_sectors }),
            "blank CD changed while reserving the drive"
        );
        Ok(recorder)
    }
}

impl device::Recorder for Recorder {
    fn inspect(&mut self) -> Result<Device> {
        if !self.locked
            && let Some(drive) = self.link.snapshot()?
        {
            return Ok(drive);
        }
        let drive = self.observe()?;
        if self.locked
            && drive
                .media
                .as_ref()
                .is_some_and(|m| !m.blank && m.appendable)
        {
            // NWA is write-mode dependent. After audio SAO, select data TAO
            // before reporting the address used to author the second session.
            self.parameters(false, false, false)?;
            self.observe()
        } else {
            Ok(drive)
        }
    }
    fn preflight_speed(&mut self, speed: Option<u32>) -> Result<()> {
        if self.locked || !self.link.requires_reservation_for_reads() {
            self.selected_speed(speed)?;
        }
        Ok(())
    }

    fn lock(&mut self) -> Result<()> {
        if !self.locked {
            self.link.reserve()?;
            let result = self.send(&[0x1e, 0, 0, 0, 1, 0], &mut [], 10);
            let result = result.and_then(|()| {
                if self.link.requires_reservation_for_reads() {
                    self.raw(&[0; 6], 0, 10)?
                        .good()
                        .context("drive reported a media change or was not ready after locking")?;
                    let actual = self.observe()?;
                    self.link.confirm_reservation(&actual)?;
                }
                Ok(())
            });
            if let Err(error) = result {
                // A transport error can occur after PREVENT reached the drive.
                let _ = self.send(&[0x1e, 0, 0, 0, 0, 0], &mut [], 10);
                return Err(error)
                    .context("cannot confirm the selected medium is locked for recording");
            }
            self.locked = true;
        }
        Ok(())
    }

    fn preflight_cd_text(&mut self, title: &str, text: &CdText) -> Result<()> {
        mmc::text_packs(title, text)?;
        // The macOS metadata view is usable before the exclusive transport opens.
        if self.locked || !self.link.requires_reservation_for_reads() {
            self.text_leadin()?;
        }
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
                && before.capabilities.as_ref().is_some_and(|c| {
                    c.sao && c.tao && if media.kind == "cd-r" { c.cdr } else { c.cdrw }
                }),
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
        let text = text
            .map(|(title, text)| -> Result<_> {
                ensure!(
                    text.titles.len() == tracks.len()
                        && before.capabilities.as_ref().is_some_and(|c| c.cd_text),
                    "CD-Text track count or drive capability mismatch"
                );
                Ok((
                    self.text_leadin()?,
                    mmc::text_symbols(&mmc::text_packs(title, text)?)?,
                ))
            })
            .transpose()?;
        let mut cue = mmc::cue(tracks, text.as_ref().map(|(start, _)| *start))?;
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
        self.parameters(true, true, text.is_some())?;
        self.speed(speed)?;
        // Recheck the invisible track after mode selection and before CUE.
        // SAO can report -150 for the first pregap or leave NWA invalid until CUE.
        let info = self.track_info(0xff)?;
        let writable = mmc::track(&info)?;
        ensure!(
            writable.empty
                && (info[7] & 1 == 0 || matches!(mmc::be32(&info, 12)? as i32, -150 | 0)),
            "blank audio track or write address changed after SAO setup"
        );
        let mut command = [0; 10];
        command[0] = 0x5d;
        let length = (cue.len() as u32).to_be_bytes();
        command[6..9].copy_from_slice(&length[1..]);
        self.send(&command, &mut cue, 30)?;
        if let Some((start, packs)) = text {
            self.write_text(start, &packs)?;
        }
        // In SAO the first pregap is explicit at signed LBA -150, not part of WAV 1.
        self.silence(-150, 150)?;
        let mut bytes = vec![0; CHUNK as usize * RAW];
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
            let pcm_end = range.end - track.tail;
            while lba < range.end {
                let count = (range.end - lba).min(CHUNK);
                let bytes = &mut bytes[..count as usize * RAW];
                let pcm_len = pcm_end.saturating_sub(lba).min(count) as usize * RAW;
                file.read_exact(&mut bytes[..pcm_len])?;
                bytes[pcm_len..].fill(0);
                // MMC CD-DA uses little-endian PCM, matching WAV. Do not apply
                // the network byte order used by the CDB's address fields.
                self.write_blocks(lba as i32, count, bytes)?;
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

    fn verify_music(&mut self, tracks: &[Music], _finalized: bool) -> Result<()> {
        self.verify_indexes(tracks)?;
        audio::verify_pcm(tracks, CHUNK, |lba, bytes| {
            let count = (bytes.len() / RAW) as u32;
            bytes.copy_from_slice(&self.read_audio(lba, count, 0)?);
            Ok(())
        })
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
        && media.sessions == 0
        && media.tracks == 0
        && media.free_sectors.is_some_and(|free| free >= capacity)
        && drive.tracks.iter().all(|t| t.empty)
}

fn same_drive(before: &Device, after: &Device) -> bool {
    before.id == after.id
        && before
            .hardware_id()
            .is_none_or(|id| after.hardware_id().as_ref() == Some(&id))
}

pub(crate) fn erase(
    selected: &Device,
    quick: bool,
    confirm: impl FnOnce(&Device) -> Result<bool>,
) -> Result<()> {
    use device::Recorder as _;
    let mut recorder = Recorder::new(&selected.id)?;
    recorder.lock()?;
    let before = recorder.observe()?;
    ensure!(
        same_drive(selected, &before)
            && before.capabilities.as_ref().is_some_and(|c| c.cdrw)
            && before
                .media
                .as_ref()
                .is_some_and(|m| m.kind == "cd-rw" && m.reserved),
        "the selected drive must still contain a reserved CD-RW"
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
    // Normalize to data TAO and the highest supported speed.
    recorder.parameters(false, false, false)?;
    recorder.speed(None)?;
    let mut cdb = [0; 12];
    cdb[0] = 0xa1;
    cdb[1] = 0x10 | u8::from(quick);
    recorder.send(&cdb, &mut [], 120)?;
    recorder.wait_ready(Duration::from_secs(90 * 60))?;
    let after = recorder.observe()?;
    ensure!(
        same_drive(&before, &after)
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
    confirm: impl FnOnce(&Device) -> Result<bool>,
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
    if !confirm(&before)? {
        println!("Calibration paused. No files created or disc write attempted.");
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
            same_drive(&before, &after),
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
    crate::calibrate::complete_single(directory, &before, speed)?;
    console::success(format_args!(
        "Verified one finalized Mode 1 track at LBA 0..{count}. Target: {}",
        directory.display()
    ));
    Ok(())
}

#[cfg(test)]
mod tests;
