# Recording on macOS

`devices` shows the drive ID, write capabilities and the current disc state.
Use `devices --json` for the same data plus per-track details. A new job
requires a blank CD-R or CD-RW and a drive that supports writing that medium
with both SAO and TAO. The default requested speed is 8×. Choose a speed
supported by your drive and disc. A used CD-RW needs a separate erase.
See the [calibration walkthrough](calibration.md#reusing-cd-rw).

```sh
discimage devices
discimage prepare album.toml --output record --speed 8
open record/planned/preview.svg
discimage resume record
```

`prepare` selects the only eligible drive, snapshots inputs, and builds and
verifies the provisional bundle without reserving or writing the disc. Use
`--device 'DRIVE_ID'` if several drives qualify. `resume` uses the saved drive
ID and speed. An optional `--device` must match that saved ID.

For a single command that prepares and writes immediately, use
`discimage burn album.toml --device 'DRIVE_ID' --output record`.
`burn --dry-run` is equivalent to `prepare`.

For a matching CD-R preset, `burn --blank-cdr CAPACITY_SECTORS` selects and
reserves the only eligible blank drive at that exact reported capacity. Use the
capacity from the preset as the expected value. With multiple eligible drives,
also pass `--device`. The disc is checked again before writing audio.

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

| Stage | `resume` behavior |
| --- | --- |
| `prepared` | Revalidate inputs, acquire the selected drive, and start music on a blank CD-R or CD-RW |
| `music_written` | Confirm the music session, verify its PCM, and generate data at the measured NWA |
| `data_ready` | Reverify music and check NWA and available range. Reuse verified assets if they still fit, otherwise reauthor into a new attempt directory |
| `music_writing`, `data_writing` | Uncertain write; refuse automatic continuation |
| `preparing`, `complete` | No automatic continuation |

Ctrl+C requests DiscRecording abort during a write. During music readback and
offline preparation it stops before the next write. If a write was started,
inspect the media and the journal before any further operation. There is no
erase or overburn path.
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
`build.rs` links DiscRecording/Foundation only on macOS. Linux/Windows retain
read-only discovery and the same offline tools, with no recording backend yet.

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
