use anyhow::{Result, bail};
use discimage::{
    author,
    burn::{self, Journal, Stage},
    device::{Capabilities, Device, Identity, Media, Music, Recorder, Track},
    project::CdText,
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
    AudioReadback,
    FinalAudioReadback,
    MissingNwa,
    MovingNwa,
    Close,
    ReadOnlyWithShortTail,
}
struct Drive {
    job: PathBuf,
    status: Device,
    calls: Vec<&'static str>,
    fault: Fault,
    expected_speed: Option<u32>,
    written_cd_text: Option<(String, String, Vec<String>)>,
}
impl Drive {
    fn new(work: &Work, fault: Fault) -> Self {
        Self {
            job: work.job.clone(),
            calls: Vec::new(),
            fault,
            expected_speed: Some(8),
            written_cd_text: None,
            status: Device {
                hardware: None,
                id: "fixture-drive".into(),
                name: "Fixture".into(),
                backend: "test".into(),
                capabilities: Some(Capabilities {
                    cdr: true,
                    cdrw: false,
                    sao: true,
                    tao: true,
                    cd_text: false,
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
    fn preflight_cd_text(&mut self, _title: &str, _text: &CdText) -> Result<()> {
        Ok(())
    }
    fn music(
        &mut self,
        tracks: &[Music],
        speed: Option<u32>,
        cd_text: Option<(&str, &CdText)>,
    ) -> Result<()> {
        self.written_cd_text = cd_text
            .map(|(title, text)| (title.to_owned(), text.artist.clone(), text.titles.clone()));
        assert_eq!(Journal::load(&self.job)?.stage, Stage::MusicWriting);
        assert_eq!(
            tracks
                .iter()
                .map(|t| (t.sectors, t.gap))
                .collect::<Vec<_>>(),
            [(300, 150), (310, 75)]
        );
        assert_eq!(speed, self.expected_speed);
        assert!(
            tracks
                .iter()
                .all(|t| (t.path.starts_with(self.job.join("planned/audio"))
                    || t.path.parent() == Some(self.job.as_path()))
                    && t.path.is_file())
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
        let kind = self.media().kind.clone();
        *self.media() = Media {
            kind,
            blank: false,
            appendable: true,
            reserved: true,
            free_sectors: Some(1100),
            nwa,
            sessions: 1,
            tracks: 2,
        };
        self.status.tracks = vec![track(1, 1, 0, 298, true), track(2, 1, 375, 308, true)];
        Ok(())
    }
    fn verify_music(&mut self, tracks: &[Music], finalized: bool) -> Result<()> {
        assert_eq!(tracks.len(), 2);
        let stage = Journal::load(&self.job)?.stage;
        if finalized {
            assert_eq!(stage, Stage::DataWriting);
            assert!(!self.media().appendable);
            self.calls.push("verify_final_music");
            if self.fault == Fault::FinalAudioReadback {
                bail!("simulated finalized PCM mismatch");
            }
        } else {
            assert!(matches!(stage, Stage::MusicWritten | Stage::DataReady));
            self.calls.push("verify_music");
            if self.fault == Fault::AudioReadback {
                bail!("simulated PCM mismatch");
            }
        }
        Ok(())
    }
    fn data(&mut self, path: &Path, start: u32, sectors: u32, speed: Option<u32>) -> Result<()> {
        let job = Journal::load(&self.job)?;
        assert_eq!(job.stage, Stage::DataWriting);
        assert_eq!(job.nwa, Some(start));
        assert_eq!(speed, self.expected_speed);
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
        let length = if self.fault == Fault::ReadOnlyWithShortTail {
            self.media().kind = "cd-rom".into();
            sectors - 2
        } else {
            sectors
        };
        let observed = if self.fault == Fault::Close {
            start + 150
        } else {
            start
        };
        self.status
            .tracks
            .push(track(3, 2, i64::from(observed), length, false));
        Ok(())
    }
    fn verify_data(&mut self, path: &Path, start: u32, sectors: u32) -> Result<()> {
        assert_eq!(Journal::load(&self.job)?.stage, Stage::DataWriting);
        assert!(!self.media().appendable);
        assert_eq!(
            self.status.tracks.last().unwrap().start,
            Some(i64::from(start))
        );
        assert_eq!(path.metadata()?.len(), u64::from(sectors) * 2048);
        self.calls.push("verify_data");
        Ok(())
    }
    fn verify_data_tail(&mut self, path: &Path, start: u32, sectors: u32) -> Result<()> {
        assert!(self.fault == Fault::ReadOnlyWithShortTail);
        let manifest = author::load(path.parent().unwrap())?;
        assert_eq!(
            (start, sectors),
            (manifest.session_start, manifest.data_sectors)
        );
        self.calls.push("verify_data_tail");
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
fn reused_native_path_cannot_resume_on_replaced_or_unidentified_hardware() {
    for replacement_serial in [Some("UNIT-B"), None] {
        let work = Work::new();
        let mut drive = Drive::new(&work, Fault::None);
        drive.status.id = "D:".into();
        drive.status.hardware = Some(discimage::device::Identity {
            vendor: "Fixture".into(),
            product: "Writer".into(),
            serial: "UNIT-A".into(),
        });
        assert!(drive.status.hardware_id().is_some());
        burn::start(&mut drive, &work.project, &work.job, Some(8), true).unwrap();
        drive.status.hardware = replacement_serial.map(|serial| discimage::device::Identity {
            vendor: "Fixture".into(),
            product: "Writer".into(),
            serial: serial.into(),
        });
        drive.calls.clear();
        let error = burn::resume(&mut drive, &work.job).unwrap_err();
        assert!(format!("{error:#}").contains("hardware identity"));
        assert!(drive.calls.is_empty());
        assert_eq!(Journal::load(&work.job).unwrap().stage, Stage::Prepared);
    }
}

#[test]
fn automatic_speed_survives_preparation_and_resume_for_both_sessions() {
    let work = Work::new();
    let mut drive = Drive::new(&work, Fault::None);
    drive.expected_speed = None;
    let prepared = burn::start(&mut drive, &work.project, &work.job, None, true).unwrap();
    assert_eq!(prepared.speed, None);
    assert_eq!(drive.writes(), 0);
    assert_eq!(Journal::load(&work.job).unwrap().speed, None);
    let complete = burn::resume(&mut drive, &work.job).unwrap();
    assert_eq!(complete.speed, None);
    assert_eq!(complete.stage, Stage::Complete);
    assert_eq!(drive.writes(), 2);
}

#[test]
fn invalid_explicit_speed_does_not_create_a_job_or_write() {
    let work = Work::new();
    let mut drive = Drive::new(&work, Fault::None);
    for speed in [0, 65, u32::MAX] {
        assert!(burn::start(&mut drive, &work.project, &work.job, Some(speed), false).is_err());
    }
    assert!(!work.job.exists());
    assert_eq!(drive.writes(), 0);
}

#[test]
fn finalized_cdr_can_be_reported_as_read_only_with_two_unreported_sectors() {
    let work = Work::new();
    let mut drive = Drive::new(&work, Fault::ReadOnlyWithShortTail);
    let complete = burn::start(&mut drive, &work.project, &work.job, Some(8), false).unwrap();
    assert_eq!(complete.stage, Stage::Complete);
    // Numeric speeds in existing v1 journals retain their meaning.
    assert_eq!(Journal::load(&work.job).unwrap().speed, Some(8));
    assert!(
        drive
            .calls
            .ends_with(&["verify_data_tail", "verify_final_music"])
    );
}

#[test]
fn completed_jobs_are_not_prepared_again_or_overwritten() {
    let work = Work::new();
    let mut drive = Drive::new(&work, Fault::None);
    burn::start(&mut drive, &work.project, &work.job, Some(8), false).unwrap();
    let before = std::fs::read(work.job.join("journal.json")).unwrap();
    let calls = drive.calls.len();
    let error = burn::start(&mut drive, &work.project, &work.job, None, true).unwrap_err();
    assert!(error.to_string().contains("already complete"));
    assert!(error.to_string().contains("new --output"));
    assert_eq!(calls, drive.calls.len());
    assert_eq!(
        before,
        std::fs::read(work.job.join("journal.json")).unwrap()
    );
}

#[test]
fn used_rewritable_discs_explain_the_explicit_erase_step() {
    let work = Work::new();
    let mut used = Drive::new(&work, Fault::None).status;
    let media = used.media.as_mut().unwrap();
    media.kind = "cd-rw".into();
    media.blank = false;
    media.appendable = false;
    media.sessions = 2;
    media.tracks = 5;
    used.capabilities.as_mut().unwrap().cdrw = true;
    let error = burn::select_blank(std::slice::from_ref(&used))
        .unwrap_err()
        .to_string();
    assert!(error.contains("2 sessions, 5 tracks"));
    assert!(error.contains("discimage erase"));
    used.media.as_mut().unwrap().kind = "cd-r".into();
    let error = burn::select_blank(&[used]).unwrap_err().to_string();
    assert!(error.contains("new blank CD-R"));
    assert!(!error.contains("'discimage erase'"));
}

#[test]
fn automatic_preparation_requires_one_eligible_blank_drive() {
    let work = Work::new();
    let blank = Drive::new(&work, Fault::None).status;
    let mut used = blank.clone();
    used.id = "used".into();
    used.media.as_mut().unwrap().blank = false;
    let mut incompatible = blank.clone();
    incompatible.id = "incompatible".into();
    incompatible.capabilities.as_mut().unwrap().tao = false;
    assert!(burn::select_blank(&[used.clone(), incompatible.clone()]).is_err());
    assert_eq!(
        burn::select_blank(&[used, incompatible, blank.clone()])
            .unwrap()
            .id,
        blank.id
    );
    let mut another = blank.clone();
    another.id = "another".into();
    assert!(burn::select_blank(&[blank, another]).is_err());
}

#[test]
fn dry_run_freezes_inputs_then_writes_two_sessions_using_the_measured_nwa() {
    let work = Work::new();
    let catalog = work.root.join("presets.toml");
    std::fs::write(
        &catalog,
        "[profiles.fixture]\ndata_reserve_sectors=300\nintertrack_gap_sectors=75\nangle=12.5\n\
         [verification.fixture]\ndrive='Another drive'\nrequested_speed=24\n",
    )
    .unwrap();
    let album = std::fs::read_to_string(&work.project).unwrap();
    let (album, _) = album.split_once("[disc]").unwrap();
    let album = format!("{album}preset={{file='presets.toml', name='fixture'}}\n");
    std::fs::write(&work.project, &album).unwrap();
    let mut drive = Drive::new(&work, Fault::None);
    let prepared = burn::start(&mut drive, &work.project, &work.job, Some(8), true).unwrap();
    assert_eq!(prepared.stage, Stage::Prepared);
    let wrong_drive = std::process::Command::new(env!("CARGO_BIN_EXE_discimage"))
        .arg("burn")
        .arg(&work.job)
        .args(["--device", "wrong-drive"])
        .output()
        .unwrap();
    assert!(!wrong_drive.status.success());
    let stderr = String::from_utf8(wrong_drive.stderr).unwrap();
    assert!(stderr.contains("does not match the recording job's saved drive"));
    assert!(!stderr.contains('\x1b'));
    assert_eq!(Journal::load(&work.job).unwrap().stage, Stage::Prepared);
    assert!(drive.calls.is_empty());
    assert!(work.job.join("source/files/empty").is_dir());
    let planned = author::load(&work.job.join("planned")).unwrap();
    // The default 360000-sector spiral survives a smaller writable range.
    assert_eq!(planned.plan.disc.capacity_sectors, 360000);
    assert_eq!(planned.plan.estimated_art_end_sector, 13350);
    let original = discimage::project::Project::load(&work.project).unwrap();
    assert_eq!(original.disc.angle, 12.5);
    assert_eq!(planned.plan.disc.geometry(), original.disc.geometry());
    assert_eq!(planned.session_start, 12085);
    // Existing source files may change without changing the already-prepared job.
    std::fs::remove_file(catalog).unwrap();
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
    assert_eq!(
        drive.calls,
        [
            "lock",
            "music",
            "verify_music",
            "data",
            "verify_final_music"
        ]
    );
    let actual = author::verify(&complete.actual(&work.job)).unwrap();
    assert_eq!(actual.session_start, 12150);
    assert_eq!(actual.data_sectors, 1100);
    assert_eq!(actual.plan.estimated_art_end_sector, 13250);
    assert_eq!(actual.plan.disc.geometry(), original.disc.geometry());
    assert_eq!(actual.plan.disc.angle, original.disc.angle);
    for lba in [0, 12085, 12385, 13249] {
        let byte = i64::from(lba) * 2352;
        assert_eq!(
            discimage::codec::point(&original.disc, byte),
            discimage::codec::point(&actual.plan.disc, byte)
        );
    }
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
fn cd_text_requires_drive_support_and_survives_preparation() {
    let work = Work::new();
    let mut drive = Drive::new(&work, Fault::None);
    let config = std::fs::read_to_string(&work.project).unwrap();
    std::fs::write(
        &work.project,
        format!("{config}\n[cd_text]\nartist='Artist'\ntitles=['First','Second']\n"),
    )
    .unwrap();
    let error = burn::start(&mut drive, &work.project, &work.job, Some(8), true).unwrap_err();
    assert!(format!("{error:#}").contains("does not report CD-Text writing support"));
    assert!(!work.job.exists());

    drive.status.capabilities.as_mut().unwrap().cd_text = true;
    let prepared = burn::start(&mut drive, &work.project, &work.job, Some(8), true).unwrap();
    assert_eq!(prepared.stage, Stage::Prepared);
    assert_eq!(drive.writes(), 0);
    std::fs::write(&work.project, config).unwrap();
    let complete = burn::resume(&mut drive, &work.job).unwrap();
    assert_eq!(complete.stage, Stage::Complete);
    assert_eq!(
        drive.written_cd_text,
        Some((
            "EP".into(),
            "Artist".into(),
            vec!["First".into(), "Second".into()]
        ))
    );
}

#[test]
fn smaller_replacement_media_is_rejected_before_music() {
    let work = Work::new();
    let mut drive = Drive::new(&work, Fault::None);
    burn::start(&mut drive, &work.project, &work.job, Some(8), true).unwrap();
    let planned = author::load(&work.job.join("planned")).unwrap();
    drive.media().free_sectors = Some(planned.plan.estimated_art_end_sector - 1);
    let error = burn::resume(&mut drive, &work.job).unwrap_err();
    assert!(format!("{error:#}").contains("blank CD is smaller"));
    assert_eq!(drive.writes(), 0);
    assert_eq!(Journal::load(&work.job).unwrap().stage, Stage::Prepared);
}

#[test]
fn missing_nwa_stops_after_music_and_resume_does_not_rewrite_music() {
    let work = Work::new();
    let mut drive = Drive::new(&work, Fault::MissingNwa);
    let error = burn::start(&mut drive, &work.project, &work.job, Some(8), false).unwrap_err();
    assert!(format!("{error:#}").contains("next writable LBA"));
    assert_eq!(Journal::load(&work.job).unwrap().stage, Stage::MusicWritten);
    assert_eq!(drive.calls, ["lock", "music"]);
    drive.media().nwa = Some(12150);
    let complete = burn::resume(&mut drive, &work.job).unwrap();
    assert_eq!(complete.stage, Stage::Complete);
    assert_eq!(
        drive.calls,
        [
            "lock",
            "music",
            "lock",
            "verify_music",
            "data",
            "verify_final_music"
        ]
    );
}

#[test]
fn pcm_mismatch_blocks_data_and_resume_rechecks_without_writing_music_again() {
    let work = Work::new();
    let mut drive = Drive::new(&work, Fault::AudioReadback);
    let error = burn::start(&mut drive, &work.project, &work.job, Some(8), false).unwrap_err();
    assert!(format!("{error:#}").contains("simulated PCM mismatch"));
    assert_eq!(Journal::load(&work.job).unwrap().stage, Stage::MusicWritten);
    assert_eq!(drive.calls, ["lock", "music", "verify_music"]);
    drive.fault = Fault::None;
    let complete = burn::resume(&mut drive, &work.job).unwrap();
    assert_eq!(complete.stage, Stage::Complete);
    assert_eq!(
        drive.calls,
        [
            "lock",
            "music",
            "verify_music",
            "lock",
            "verify_music",
            "data",
            "verify_final_music"
        ]
    );
}

#[test]
fn finalized_audio_mismatch_blocks_completion_and_does_not_retry_writes() {
    let work = Work::new();
    let mut drive = Drive::new(&work, Fault::FinalAudioReadback);
    let error = burn::start(&mut drive, &work.project, &work.job, Some(8), false).unwrap_err();
    assert!(format!("{error:#}").contains("music readback failed after disc finalization"));
    let journal = Journal::load(&work.job).unwrap();
    assert_eq!(journal.stage, Stage::DataWriting);
    assert_eq!(journal.observed.media.unwrap().sessions, 2);
    assert!(burn::resume(&mut drive, &work.job).is_err());
    assert_eq!(drive.writes(), 2);
}

#[test]
fn legacy_authoring_exports_tail_silence_without_shifting_the_data_layout() {
    let work = Work::new();
    let config = std::fs::read_to_string(&work.project).unwrap();
    std::fs::write(
        &work.project,
        format!("intertrack_gap_position = 'tail'\n{config}"),
    )
    .unwrap();
    let mut drive = Drive::new(&work, Fault::None);
    let prepared = burn::start(&mut drive, &work.project, &work.job, Some(8), true).unwrap();
    assert_eq!(prepared.stage, Stage::Prepared);
    assert!(drive.calls.is_empty());

    let original = std::fs::read(work.root.join("one.wav")).unwrap();
    let planned_dir = work.job.join("planned");
    let first = std::fs::read(planned_dir.join("audio/01.wav")).unwrap();
    assert_eq!(first.len(), original.len() + 75 * 2352);
    assert_eq!(first[44..original.len()], original[44..]);
    assert!(first[original.len()..].iter().all(|b| *b == 0));
    assert_eq!(
        std::fs::read(planned_dir.join("audio/02.wav")).unwrap(),
        std::fs::read(work.root.join("two.wav")).unwrap()
    );
    assert!(
        !std::fs::read_to_string(planned_dir.join("session1.cue"))
            .unwrap()
            .contains("PREGAP")
    );
    let planned = author::verify(&planned_dir).unwrap();
    assert_eq!(planned.plan.music_sectors, 610);
    assert_eq!(planned.plan.gap_sectors, 75);
    assert_eq!(planned.session_start, 12_085);
}

#[test]
fn track_lead_in_moves_the_index_without_moving_music_or_data() {
    let work = Work::new();
    let config = std::fs::read_to_string(&work.project)
        .unwrap()
        .replace("[disc]", "[disc]\ncapacity_sectors = 14000");
    std::fs::write(
        &work.project,
        format!("intertrack_gap_position = 'tail'\ntrack_lead_in_sectors = 10\n{config}"),
    )
    .unwrap();
    let second = work.root.join("two.wav");
    let mut pcm = std::fs::read(&second).unwrap();
    pcm[44..48].copy_from_slice(&[1, 2, 3, 4]);
    std::fs::write(&second, &pcm).unwrap();
    discimage::package::build(&work.project, &work.job, None, None).unwrap();
    let package = discimage::package::Package::load(&work.job).unwrap();
    let plan = package.verify_inputs(&work.job).unwrap();
    assert_eq!(plan.disc.track_lead_in_sectors, 0);
    assert_eq!(plan.gap_sectors, 65);
    assert_eq!(plan.music_sectors, 620);
    assert_eq!(plan.tracks[0].sectors + plan.gap_sectors, 365);
    assert_eq!(plan.session_start(), 12_085);
    let frozen = std::fs::read(work.job.join("02.wav")).unwrap();
    assert!(frozen[44..44 + 10 * 2352].iter().all(|&byte| byte == 0));
    assert_eq!(&frozen[44 + 10 * 2352..], &pcm[44..]);

    let legacy = work.root.join("legacy");
    author::build_images(&work.project, &legacy, None, author::Images::Iso).unwrap();
    let manifest = author::verify(&legacy).unwrap();
    assert_eq!(manifest.session_start, 12_085);
    let exported = std::fs::read(legacy.join("audio/02.wav")).unwrap();
    assert_eq!(exported, frozen);
    let first = std::fs::read(legacy.join("audio/01.wav")).unwrap();
    assert_eq!(first.len(), 44 + 365 * 2352);
}

#[test]
fn changing_nwa_blocks_data_and_resume_reauthors_at_the_new_address() {
    let work = Work::new();
    let mut drive = Drive::new(&work, Fault::MovingNwa);
    let error = burn::start(&mut drive, &work.project, &work.job, Some(8), false).unwrap_err();
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
        assert!(burn::start(&mut drive, &work.project, &work.job, Some(8), false).is_err());
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
    assert!(burn::start(&mut drive, &work.project, &work.job, Some(8), false).is_err());
    assert!(!work.job.exists());
    drive.media().kind = "cd-r".into();
    drive.media().blank = false;
    assert!(burn::start(&mut drive, &work.project, &work.job, Some(8), false).is_err());
    assert!(!work.job.exists());
    drive.media().blank = true;
    drive.status.capabilities.as_mut().unwrap().tao = false;
    assert!(burn::start(&mut drive, &work.project, &work.job, Some(8), false).is_err());
    assert!(!work.job.exists());
    drive.status.capabilities.as_mut().unwrap().tao = true;
    burn::start(&mut drive, &work.project, &work.job, Some(8), true).unwrap();
    std::fs::write(work.job.join("source/files/note.txt"), "changed").unwrap();
    assert!(burn::resume(&mut drive, &work.job).is_err());
    assert!(drive.calls.is_empty());
}

#[test]
fn blank_cdrw_uses_the_same_measured_nwa_and_frozen_calibration() {
    let work = Work::new();
    let mut config = std::fs::OpenOptions::new()
        .append(true)
        .open(&work.project)
        .unwrap();
    writeln!(config, "[disc.calibration]\nreference_sectors=14000\ninner_radius_mm=24.6\nouter_radius_mm=57.2\nrms_mm=0.1").unwrap();
    let mut drive = Drive::new(&work, Fault::None);
    let capabilities = drive.status.capabilities.as_mut().unwrap();
    capabilities.cdr = false;
    capabilities.cdrw = true;
    drive.media().kind = "cd-rw".into();
    drive.media().blank = false;
    assert!(burn::start(&mut drive, &work.project, &work.job, Some(8), false).is_err());
    assert!(!work.job.exists() && drive.calls.is_empty());
    drive.media().blank = true;

    let complete = burn::start(&mut drive, &work.project, &work.job, Some(8), false).unwrap();
    assert_eq!(complete.stage, Stage::Complete);
    assert_eq!(complete.observed.media.unwrap().kind, "cd-rw");
    assert_eq!(
        drive.calls,
        [
            "lock",
            "music",
            "verify_music",
            "data",
            "verify_final_music"
        ]
    );
    let planned = author::load(&work.job.join("planned")).unwrap();
    let actual = author::load(&work.job.join("actual-1")).unwrap();
    assert_eq!(actual.session_start, 12150);
    assert_eq!(actual.plan.disc.geometry(), (24.6, 57.2, 14000));
    assert_eq!(planned.plan.disc.geometry(), actual.plan.disc.geometry());
}

#[test]
fn flat_package_build_is_offline_and_defers_all_data_encoding() {
    use discimage::package::{self, Package};
    let work = Work::new();
    package::build(&work.project, &work.job, None, None).unwrap();
    let package = Package::load(&work.job).unwrap();
    package.verify(&work.job).unwrap();
    assert!(package.journal.is_none() && package.image.is_none());
    assert!(!work.job.join("session2.iso").exists());
    assert!(
        std::fs::read_dir(&work.job)
            .unwrap()
            .all(|e| e.unwrap().file_type().unwrap().is_file())
    );
    assert!(work.job.join("01.wav").is_file() && work.job.join("02.wav").is_file());
    let manifest = std::fs::read(work.job.join("manifest.toml")).unwrap();
    std::fs::write(work.job.join("notes.txt"), "private note").unwrap();
    assert!(package::build(&work.project, &work.job, None, None).is_err());
    assert_eq!(
        std::fs::read(work.job.join("manifest.toml")).unwrap(),
        manifest
    );
    std::fs::remove_file(work.job.join("notes.txt")).unwrap();
    package::build(&work.project, &work.job, None, None).unwrap();
    std::fs::write(work.job.join("01.wav"), "changed").unwrap();
    assert!(Package::load(&work.job).unwrap().verify(&work.job).is_err());
    let mut drive = Drive::new(&work, Fault::None);
    assert!(burn::record(&mut drive, &work.job, Some(8)).is_err());
    assert!(drive.calls.is_empty());
}

#[test]
fn selected_image_formats_are_real_and_raw_only_remains_verifiable() {
    use discimage::{
        author::Images,
        package::{self, Package},
    };
    for (images, iso, raw) in [
        (Images::Iso, true, false),
        (Images::Raw, false, true),
        (Images::Both, true, true),
    ] {
        let work = Work::new();
        let settings = std::fs::read_to_string(&work.project)
            .unwrap()
            .replace("[disc]\n", "[disc]\ncapacity_sectors=14000\n");
        std::fs::write(&work.project, settings).unwrap();
        package::build(&work.project, &work.job, Some(images), Some(12150)).unwrap();
        let package = Package::load(&work.job).unwrap();
        package.verify(&work.job).unwrap();
        assert_eq!(work.job.join("session2.iso").exists(), iso);
        assert_eq!(work.job.join("session2.bin").exists(), raw);
        assert!(!work.job.join(".work").exists());
        assert!(
            std::fs::read_dir(&work.job).unwrap().all(|e| e
                .unwrap()
                .file_type()
                .unwrap()
                .is_file())
        );
        let image = package.image.unwrap();
        discimage::preview::readback(&work.job, &image, &work.root.join("encoded.png"), 256, 16)
            .unwrap();
    }
}

#[test]
fn flat_recording_keeps_one_copy_and_recovers_after_audio_verification_failure() {
    use discimage::package::{self, Package};
    let work = Work::new();
    package::build(&work.project, &work.job, None, None).unwrap();
    let audio = author::digest(&work.job.join("01.wav")).unwrap();
    let mut drive = Drive::new(&work, Fault::AudioReadback);
    assert!(burn::record(&mut drive, &work.job, Some(8)).is_err());
    assert_eq!(Journal::load(&work.job).unwrap().stage, Stage::MusicWritten);
    assert!(!work.job.join("session2.iso").exists());
    assert!(package::build(&work.project, &work.job, None, None).is_err());
    assert!(burn::record(&mut drive, &work.job, Some(10)).is_err());
    drive.fault = Fault::None;
    let complete = burn::record(&mut drive, &work.job, None).unwrap();
    assert_eq!(complete.stage, Stage::Complete);
    assert_eq!(complete.actual(&work.job), work.job.canonicalize().unwrap());
    assert_eq!(drive.calls.iter().filter(|&&c| c == "music").count(), 1);
    assert_eq!(author::digest(&work.job.join("01.wav")).unwrap(), audio);
    Package::load(&work.job).unwrap().verify(&work.job).unwrap();
    assert!(work.job.join("session2.iso").is_file());
    assert!(!work.job.join("session2.bin").exists());
    assert!(
        std::fs::read_dir(&work.job)
            .unwrap()
            .all(|e| e.unwrap().file_type().unwrap().is_file())
    );
    let writes = drive.writes();
    assert!(burn::record(&mut drive, &work.job, None).is_err());
    assert_eq!(writes, drive.writes());
    package::build(&work.project, &work.job, None, None).unwrap();
    assert!(Package::load(&work.job).unwrap().journal.is_none());
}

#[test]
fn completed_legacy_recording_migrates_without_changing_the_written_iso() {
    use discimage::package::{self, Package};
    let work = Work::new();
    let mut drive = Drive::new(&work, Fault::None);
    let complete = burn::start(&mut drive, &work.project, &work.job, Some(8), false).unwrap();
    let hash = author::digest(&complete.actual(&work.job).join("session2.iso")).unwrap();
    let changed = work.job.join("source/unexpected.txt");
    std::fs::write(&changed, "changed after recording").unwrap();
    assert!(package::migrate(&work.job).is_err());
    assert!(work.job.join("journal.json").exists());
    assert_eq!(
        author::digest(&complete.actual(&work.job).join("session2.iso")).unwrap(),
        hash
    );
    std::fs::remove_file(changed).unwrap();
    package::migrate(&work.job).unwrap();
    let migrated = Package::load(&work.job).unwrap();
    migrated.verify(&work.job).unwrap();
    assert_eq!(migrated.journal.unwrap().stage, Stage::Complete);
    assert_eq!(
        author::digest(&work.job.join("session2.iso")).unwrap(),
        hash
    );
    assert!(
        std::fs::read_dir(&work.job)
            .unwrap()
            .all(|e| e.unwrap().file_type().unwrap().is_file())
    );
    assert!(!work.job.join("planned").exists() && !work.job.join("actual-1").exists());
}

#[test]
fn flat_package_requires_the_configured_medium_before_any_write() {
    use discimage::package;
    let work = Work::new();
    let settings = std::fs::read_to_string(&work.project)
        .unwrap()
        .replace("[disc]\n", "[disc]\nmedia='cd-rw'\n");
    std::fs::write(&work.project, settings).unwrap();
    package::build(&work.project, &work.job, None, None).unwrap();
    let mut drive = Drive::new(&work, Fault::None);
    let error = burn::record(&mut drive, &work.job, Some(8)).unwrap_err();
    assert!(format!("{error:#}").contains("requires cd-rw"));
    assert!(drive.calls.is_empty());
}

#[test]
fn flat_package_replaces_data_at_changed_nwa_without_keeping_attempt_folders() {
    use discimage::package::{self, Package};
    let work = Work::new();
    package::build(&work.project, &work.job, None, None).unwrap();
    let mut drive = Drive::new(&work, Fault::MovingNwa);
    let error = burn::record(&mut drive, &work.job, Some(8)).unwrap_err();
    assert!(format!("{error:#}").contains("NWA changed before data write"));
    assert_eq!(Journal::load(&work.job).unwrap().stage, Stage::DataReady);
    drive.fault = Fault::None;
    let complete = burn::record(&mut drive, &work.job, None).unwrap();
    assert_eq!(complete.nwa, Some(12151));
    assert_eq!(complete.attempt, 2);
    assert_eq!(drive.calls.iter().filter(|&&c| c == "music").count(), 1);
    Package::load(&work.job).unwrap().verify(&work.job).unwrap();
    assert!(
        std::fs::read_dir(&work.job)
            .unwrap()
            .all(|e| e.unwrap().file_type().unwrap().is_file())
    );
}

#[test]
fn flat_uncertain_writes_are_protected_from_retry_and_rebuild() {
    use discimage::package;
    for (fault, stage) in [
        (Fault::Music, Stage::MusicWriting),
        (Fault::Close, Stage::DataWriting),
    ] {
        let work = Work::new();
        package::build(&work.project, &work.job, None, None).unwrap();
        let mut drive = Drive::new(&work, fault);
        assert!(burn::record(&mut drive, &work.job, Some(8)).is_err());
        assert_eq!(Journal::load(&work.job).unwrap().stage, stage);
        let calls = drive.calls.len();
        assert!(burn::record(&mut drive, &work.job, None).is_err());
        assert!(package::build(&work.project, &work.job, None, None).is_err());
        assert_eq!(calls, drive.calls.len());
    }
}

#[test]
fn finalized_job_recovers_with_readback_after_the_same_drive_reconnects() {
    let work = Work::new();
    discimage::package::build(&work.project, &work.job, None, None).unwrap();
    let mut drive = Drive::new(&work, Fault::FinalAudioReadback);
    drive.status.hardware = Some(Identity {
        vendor: "Fixture".into(),
        product: "Optical drive".into(),
        serial: "XYZ012345".into(),
    });
    assert!(burn::record(&mut drive, &work.job, Some(8)).is_err());
    let saved = Journal::load(&work.job).unwrap();
    assert_eq!(saved.stage, Stage::DataWriting);
    let writes = drive.writes();
    drive.status.id = "reconnected-drive".into();
    drive.fault = Fault::None;
    let recovered = burn::record(&mut drive, &work.job, None).unwrap();
    assert_eq!(recovered.stage, Stage::Complete);
    assert_eq!(recovered.device, saved.device);
    assert_eq!(recovered.observed.id, drive.status.id);
    assert_eq!(drive.writes(), writes);
    assert!(
        drive
            .calls
            .ends_with(&["lock", "verify_data", "verify_final_music"])
    );
}
