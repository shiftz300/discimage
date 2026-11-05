use anyhow::{Result, bail};
use discimage::{
    author,
    burn::{self, Journal, Stage},
    device::{Capabilities, Device, Media, Music, Recorder, Track},
};
use std::{
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

struct Work {
    root: PathBuf,
    project: PathBuf,
    job: PathBuf,
}
impl Work {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "discimage-recording-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        wav(&root.join("one.wav"), 300);
        wav(&root.join("two.wav"), 310);
        std::fs::create_dir_all(root.join("extras/empty")).unwrap();
        std::fs::write(root.join("extras/note.txt"), "unchanged companion file").unwrap();
        let project = root.join("album.toml");
        std::fs::write(&project,"title='EP'\nfiles='extras'\ntracks=[{path='one.wav'},{path='two.wav'}]\n[disc]\ndata_reserve_sectors=300\nintertrack_gap_sectors=75\n").unwrap();
        let job = root.join("job");
        Self { root, project, job }
    }
}
impl Drop for Work {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}
fn wav(path: &Path, sectors: u32) {
    let mut out = File::create(path).unwrap();
    let bytes = sectors * 2352;
    out.write_all(b"RIFF").unwrap();
    out.write_all(&(36 + bytes).to_le_bytes()).unwrap();
    out.write_all(b"WAVEfmt \x10\0\0\0\x01\0\x02\0\x44\xac\0\0\x10\xb1\x02\0\x04\0\x10\0data")
        .unwrap();
    out.write_all(&bytes.to_le_bytes()).unwrap();
    out.set_len(u64::from(44 + bytes)).unwrap();
}

#[derive(PartialEq, Eq)]
enum Fault {
    None,
    Music,
    MissingNwa,
    MovingNwa,
    Close,
}
struct Drive {
    job: PathBuf,
    status: Device,
    calls: Vec<&'static str>,
    fault: Fault,
}
impl Drive {
    fn new(work: &Work, fault: Fault) -> Self {
        Self {
            job: work.job.clone(),
            calls: Vec::new(),
            fault,
            status: Device {
                id: "fixture-drive".into(),
                name: "Fixture".into(),
                backend: "test".into(),
                capabilities: Some(Capabilities {
                    cdr: true,
                    sao: true,
                    tao: true,
                }),
                media: Some(Media {
                    kind: "cd-r".into(),
                    blank: true,
                    appendable: false,
                    reserved: false,
                    free_sectors: Some(13_500),
                    nwa: Some(0),
                    sessions: 0,
                    tracks: 0,
                }),
                tracks: Vec::new(),
            },
        }
    }
    fn media(&mut self) -> &mut Media {
        self.status.media.as_mut().unwrap()
    }
    fn writes(&self) -> usize {
        self.calls
            .iter()
            .filter(|&&c| c == "music" || c == "data")
            .count()
    }
}
impl Recorder for Drive {
    fn inspect(&mut self) -> Result<Device> {
        if self.fault == Fault::MovingNwa
            && Journal::load(&self.job).is_ok_and(|j| j.stage == Stage::DataReady)
        {
            self.media().nwa = Some(12151);
        }
        Ok(self.status.clone())
    }
    fn lock(&mut self) -> Result<()> {
        self.calls.push("lock");
        self.media().reserved = true;
        Ok(())
    }
    fn music(&mut self, tracks: &[Music], speed: u32) -> Result<()> {
        assert_eq!(Journal::load(&self.job)?.stage, Stage::MusicWriting);
        assert_eq!(
            tracks
                .iter()
                .map(|t| (t.sectors, t.gap))
                .collect::<Vec<_>>(),
            [(300, 150), (310, 75)]
        );
        assert_eq!(speed, 8);
        assert!(
            tracks
                .iter()
                .all(|t| t.path.starts_with(self.job.join("planned/audio")) && t.path.is_file())
        );
        self.calls.push("music");
        if self.fault == Fault::Music {
            bail!("simulated write interruption");
        }
        let nwa = if self.fault == Fault::MissingNwa {
            None
        } else {
            Some(12150)
        };
        *self.media() = Media {
            kind: "cd-r".into(),
            blank: false,
            appendable: true,
            reserved: true,
            free_sectors: Some(1100),
            nwa,
            sessions: 1,
            tracks: 2,
        };
        self.status.tracks = vec![track(1, 1, 0, 300, true), track(2, 1, 375, 310, true)];
        Ok(())
    }
    fn data(&mut self, path: &Path, start: u32, sectors: u32, speed: u32) -> Result<()> {
        let job = Journal::load(&self.job)?;
        assert_eq!(job.stage, Stage::DataWriting);
        assert_eq!(job.nwa, Some(start));
        assert_eq!(speed, 8);
        let actual = author::load(path.parent().unwrap())?;
        assert_eq!(actual.session_start, start);
        assert_eq!(actual.data_sectors, sectors);
        assert_eq!(path.metadata()?.len(), u64::from(sectors) * 2048);
        assert_eq!(actual.plan.music_sectors, 610);
        assert_eq!(
            actual
                .files
                .iter()
                .find(|f| f.path == "note.txt")
                .unwrap()
                .bytes,
            24
        );
        self.calls.push("data");
        let media = self.media();
        media.appendable = false;
        media.nwa = None;
        media.sessions = 2;
        media.tracks = 3;
        let observed = if self.fault == Fault::Close {
            start + 150
        } else {
            start
        };
        self.status
            .tracks
            .push(track(3, 2, i64::from(observed), sectors, false));
        Ok(())
    }
}
fn track(number: u32, session: u32, start: i64, sectors: u32, audio: bool) -> Track {
    Track {
        number,
        session,
        start: Some(start),
        sectors: Some(sectors),
        audio,
        closed: true,
        empty: false,
        next_writable: None,
    }
}

#[test]
fn dry_run_freezes_inputs_then_writes_two_sessions_using_the_measured_nwa() {
    let work = Work::new();
    let mut drive = Drive::new(&work, Fault::None);
    let prepared = burn::start(&mut drive, &work.project, &work.job, 8, true).unwrap();
    assert_eq!(prepared.stage, Stage::Prepared);
    assert!(drive.calls.is_empty());
    assert!(work.job.join("source/files/empty").is_dir());
    let planned = author::load(&work.job.join("planned")).unwrap();
    assert_eq!(planned.plan.disc.capacity_sectors, 13500);
    assert_eq!(planned.session_start, 12085);
    // Existing source files may change without changing the already-prepared job.
    std::fs::write(work.root.join("one.wav"), b"original later edited").unwrap();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(work.job.join("lock"))
        .unwrap();
    lock.try_lock().unwrap();
    assert!(burn::resume(&mut drive, &work.job).is_err());
    assert!(drive.calls.is_empty());
    drop(lock);
    let complete = burn::resume(&mut drive, &work.job).unwrap();
    assert_eq!(complete.stage, Stage::Complete);
    assert_eq!(drive.calls, ["lock", "music", "data"]);
    let actual = author::verify(&complete.actual(&work.job)).unwrap();
    assert_eq!(actual.session_start, 12150);
    assert_eq!(actual.data_sectors, 1100);
    assert_eq!(actual.plan.disc.end_margin_sectors, 250);
    assert_ne!(
        planned
            .assets
            .iter()
            .find(|a| a.path == "session2.iso")
            .unwrap()
            .sha256,
        actual
            .assets
            .iter()
            .find(|a| a.path == "session2.iso")
            .unwrap()
            .sha256
    );
    assert!(burn::resume(&mut drive, &work.job).is_err());
    assert_eq!(drive.writes(), 2);
}

#[test]
fn missing_nwa_stops_after_music_and_resume_does_not_rewrite_music() {
    let work = Work::new();
    let mut drive = Drive::new(&work, Fault::MissingNwa);
    let error = burn::start(&mut drive, &work.project, &work.job, 8, false).unwrap_err();
    assert!(format!("{error:#}").contains("next writable LBA"));
    assert_eq!(Journal::load(&work.job).unwrap().stage, Stage::MusicWritten);
    assert_eq!(drive.calls, ["lock", "music"]);
    drive.media().nwa = Some(12150);
    let complete = burn::resume(&mut drive, &work.job).unwrap();
    assert_eq!(complete.stage, Stage::Complete);
    assert_eq!(drive.calls, ["lock", "music", "lock", "data"]);
}

#[test]
fn changing_nwa_blocks_data_and_resume_reauthors_at_the_new_address() {
    let work = Work::new();
    let mut drive = Drive::new(&work, Fault::MovingNwa);
    let error = burn::start(&mut drive, &work.project, &work.job, 8, false).unwrap_err();
    assert!(format!("{error:#}").contains("NWA changed before data write"));
    assert_eq!(Journal::load(&work.job).unwrap().stage, Stage::DataReady);
    assert_eq!(drive.writes(), 1);
    drive.fault = Fault::None;
    let complete = burn::resume(&mut drive, &work.job).unwrap();
    assert_eq!(complete.stage, Stage::Complete);
    assert_eq!(complete.nwa, Some(12151));
    assert_eq!(complete.attempt, 2);
    assert!(work.job.join("actual-1/manifest.json").exists());
    assert_eq!(drive.writes(), 2);
}

#[test]
fn interrupted_write_and_incorrect_final_track_are_never_retried() {
    for (fault, stage, writes) in [
        (Fault::Music, Stage::MusicWriting, 1),
        (Fault::Close, Stage::DataWriting, 2),
    ] {
        let work = Work::new();
        let mut drive = Drive::new(&work, fault);
        assert!(burn::start(&mut drive, &work.project, &work.job, 8, false).is_err());
        let journal = Journal::load(&work.job).unwrap();
        assert_eq!(journal.stage, stage);
        assert!(journal.error.is_some());
        assert!(burn::resume(&mut drive, &work.job).is_err());
        assert_eq!(drive.writes(), writes);
    }
}

#[test]
fn rejects_unsafe_media_and_modified_snapshot_before_writing() {
    let work = Work::new();
    let mut drive = Drive::new(&work, Fault::None);
    drive.media().kind = "cd-rw".into();
    assert!(burn::start(&mut drive, &work.project, &work.job, 8, false).is_err());
    assert!(!work.job.exists());
    drive.media().kind = "cd-r".into();
    drive.media().blank = false;
    assert!(burn::start(&mut drive, &work.project, &work.job, 8, false).is_err());
    assert!(!work.job.exists());
    drive.media().blank = true;
    drive.status.capabilities.as_mut().unwrap().tao = false;
    assert!(burn::start(&mut drive, &work.project, &work.job, 8, false).is_err());
    assert!(!work.job.exists());
    drive.status.capabilities.as_mut().unwrap().tao = true;
    burn::start(&mut drive, &work.project, &work.job, 8, true).unwrap();
    std::fs::write(work.job.join("source/files/note.txt"), "changed").unwrap();
    assert!(burn::resume(&mut drive, &work.job).is_err());
    assert!(drive.calls.is_empty());
}
