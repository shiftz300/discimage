# Recording on macOS

`devices` groups the disc state, write capabilities, per-track details,
selector and full device ID. Use `--device GX40N` for a unique model or
copy its 12-character hardware ID (`local-xxxxxxxxxx` when a reliable
drive serial is unavailable). Long native IDs remain available in
`devices --json`. Missing values are shown
as unknown, separately from zero or unsupported capabilities. A new job
requires a blank CD-R or CD-RW and a drive that supports writing that medium
with both SAO and TAO. By default, no fixed speed is requested: DiscRecording lets the drive use its
supported maximum for the inserted medium. Use `--speed N` to request a specific
CD speed multiplier (1–64×). A used CD-RW needs `discimage erase` before preparation.
See the [calibration walkthrough](calibration.md#reusing-cd-rw).

Use `discimage select ID` once to save a default, or pass `--device ID` to each
command. Explicit flags override the default without updating it. A saved job
keeps its own drive binding and ignores the default. `discimage select` shows
the setting; `discimage select --clear` removes it. An unavailable default stops
selection rather than falling back to another drive.

```sh
discimage devices
discimage burn album.toml --output record --dry-run
open record/planned/preview.svg
discimage burn record
```

`burn --dry-run` selects the only eligible drive, snapshots inputs, and builds and
verifies the provisional bundle without reserving or writing the disc. Use
`--device` with a short selector or unique model if several drives qualify. `burn JOB` uses the saved drive
ID and speed policy for both sessions. Automatic speed is saved as `null` in
`journal.json`; numeric speeds from older jobs remain fixed when resumed. An
optional `--device` must resolve to that saved native ID.

For a single command that prepares and writes immediately, use
`discimage burn album.toml --output record`. Add `--device` when more than one blank drive qualifies.
Pass an existing job directory to `burn` to start or continue its recording.

For a matching CD-R preset, `burn --blank-cdr CAPACITY_SECTORS` selects and
reserves the only eligible blank drive at that exact reported capacity. Use the
capacity from the preset as the expected value. With multiple eligible drives,
also pass `--device`. The disc is checked again before writing audio.

A closed disc with music tracks in session 1 and a data track in session 2 is
already recorded. `burn --dry-run` requires blank media and never erases it implicitly.
For another recording on that CD-RW, run `discimage erase` and prepare into a
new output directory. Existing job directories are rejected before selecting or
reserving a drive; use `burn JOB` only for a resumable existing job.

## Erasing CD-RW

```sh
discimage erase --dry-run
discimage erase
```

`erase` selects the only available CD-RW, reserves and identifies that media,
shows its contents and asks you to type `ERASE`. Multiple eligible drives require
a `--device` selector from `devices`. CD-R and other media are rejected.
The default is a complete erase, including the previous artwork; `--quick`
only resets the disc metadata and can leave previous marks visible. `--yes`
explicitly approves erasure without the prompt. `--dry-run` checks selection
without reserving or erasing the media.

The native DiscRecording erase reports state changes, available percentages and
elapsed time, with a heartbeat every 15 seconds. DRErase has no abort operation:
Ctrl+C before starting cancels; after starting, the CLI waits for completion and
keeps reporting progress. Keep the drive connected. Success requires terminal
framework success followed by a blank CD-RW with zero sessions/tracks and
positive writable capacity. This confirms the drive state, not a byte-for-byte
blank-surface scan. Erasing does not start recording automatically.

## CD-Text

Set the optional block below to write the project's `title`, the performer,
and the track titles in the first audio session.

```toml
[cd_text]
artist = "Artist"
titles = ["First track", "Second track"]
```

Use printable ASCII and list one title per WAV in track order. Preparation
checks drive support and rejects text that DiscRecording would alter or
truncate. After recording, use `drutil cdtext` with a drive that can read
CD-Text to confirm the fields on the disc.

## Transaction

- `source/`: normalized PCM WAVs, a normalized artwork snapshot, companion
  files/directories, and TOML. The snapshot includes empty directories and is
  hashed before use. The system decoder is checked before recording. If it
  rounds up by a sector, preparation adds silent tail padding to the WAV
  snapshot and checks the length again. Changing the original project later
  does not alter the job.
- `planned/`: full authored bundle limited to the reported writable space,
  with the configured reference geometry retained. Nothing from its provisional data session is
  written to the drive.
- Music: DiscRecording writes audio tracks in SAO and leaves the CD
  appendable. The first pregap is 150 sectors. For CD-RW on Apple SuperDrive,
  track gaps below 150 sectors are raised to 150 during preparation.
- NWA: read `DRNextWritableAddressKey` from the open empty/invisible track
  exposed by `DRDeviceTrackRefsKey` / `DRDeviceTrackInfoKey`. These keys can
  appear inside the media dictionary on macOS. The bridge waits briefly while
  the drive refreshes its media state after a write. An ambiguous or
  missing address is an error. The recorded closed music track numbers, starts
  and session must match. There is no fallback to an estimated NWA and no
  arbitrary `+150` adjustment. The reported address must be in the expected
  second-session range; unexpected drive conventions stop the job.
- Music verification: read each audio track from the drive and compare its PCM
  byte-for-byte against the normalized WAV. A read error or mismatch blocks
  the data write and keeps the job at a resumable stage. macOS's internal
  audio verifier can report a checksum error on correctly written tracks.
- `actual-N/`: rebuilt volume, cooked ISO, RAW, checksums and previews using
  the observed NWA. If the writable range is shorter than the planned extent,
  increase the end margin while preserving the recording geometry. The same
  disc/strip sampler remains in use. Before writing, recheck source hashes,
  session layout, reservation, NWA and remaining capacity.
- Data: DiscRecording opens the cooked ISO as one image track after the bridge
  checks the file size, track length and reserved media's current NWA. The
  filesystem extents already contain absolute LBAs. Required TAO, appendable
  off, `ProduceAgain` verification,
  no overwrite and no eject on failure. DiscRecording writes Mode 1 framing
  and finalizes the disc. The RAW artifact is not sent to the drive.
- Completion: require DiscRecording's terminal success, finalized media with
  two sessions, matching original music layout, and a session-2 data track at
  the measured start with sufficient length. Successful writes request mount;
  the app never requests ejection.

The bridge holds the application's media reservation across both writes and
reauthoring. The framework manages write-time device access. Rechecks reject
an unexpected disc layout; recordable CDs do not expose a portable unique serial via
these APIs. Keep the same disc in the drive throughout the job.

## Journal and recovery

`journal.json` is replaced atomically and synced before each write intent.
An OS file lock prevents concurrent processes from using the same job. Final
snapshots report the state last observed by the backend, not a prediction.

| Stage | `burn JOB` behavior |
| --- | --- |
| `prepared` | Revalidate inputs, acquire the selected drive, and start music on a blank CD-R or CD-RW |
| `music_written` | Confirm the music session, verify its PCM, and generate data at the measured NWA |
| `data_ready` | Reverify music and check NWA and available range. Reuse verified assets if they still fit, otherwise reauthor into a new attempt directory |
| `music_writing`, `data_writing` | Uncertain write; refuse automatic continuation |
| `preparing`, `complete` | No automatic continuation |

Ctrl+C requests DiscRecording abort during a write. During music readback and
offline preparation it stops before the next write. If a write was started,
inspect the media and the journal before any further operation. The recording transaction never erases or overwrites media. Erasing is a separate
explicit command; there is no overburn path.
The CLI writes diagnostics and progress to stderr. On Terminal.app, labels
use basic ANSI colors. Redirect stderr to save a plain-text log, or set
`NO_COLOR=1` to keep plain text in an interactive terminal. During writing,
the drive status updates on phase changes, on progress milestones and at
least every 15 seconds while DiscRecording continues reporting. Percentage
and speed are shown when available. Elapsed time and time since the last
change help distinguish a slow stage from one that is progressing.

## Backend and acceptance

`device::Recorder` abstracts status, reservation, appendable music write,
PCM readback and finalizing data write. `burn` uses only this interface. The Rust macOS adapter
uses owned JSON and one opaque session handle to call `native/macos.m`;
`build.rs` links DiscRecording/Foundation only on macOS. Linux and Windows
share the offline tools. Windows also has a separate read-only CD timing
probe. Disc writing uses the macOS backend.

Primary interface references are the installed macOS SDK headers:
`DRDevice.h` (media status/capabilities/reservation), `DRTrack.h` (NWA, pregap,
producer and verification), and `DRBurn.h` (required strategy, append/finalize,
abort, terminal status). For the distinction between pregap address and the
next user-accessible sector, see the original
[cdrtools multisession notes](https://sources.debian.org/src/cdrtools/4%3A2.01%2B01a01-2/README.multi)
and `cdrecord`'s write-mode comments. The backend consumes the reported address
as-is and checks the actual resulting data track; it does not invent an offset.

An LG GX40N (firmware RQ00) with Verbatim SW80EU1 CD-RW completed the
full six-track album and visible artwork workflow at requested 10×.

An Apple SuperDrive (firmware 2.00) completed SAO, measured-NWA append, TAO
and finalization on CD-RW. Independent raw readback confirmed the recorded
PCM and Mode 1 payload. See [validation](validation.md) for drive-specific
results and remaining acceptance checks.
