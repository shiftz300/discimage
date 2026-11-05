# Recording on macOS

`devices --json` reports a stable IORegistry path as `id`, write capabilities,
media state and available track details. No optical hardware returns `[]`.
Use the exact `id` with `burn` or `resume`. Blank CD-R and both SAO/TAO support
are required for a new job. The default requested speed is 8×.

```sh
discimage burn album.toml --device 'IORegistry id' --output record --dry-run
discimage resume record --device 'IORegistry id'
```

`burn` without `--dry-run` proceeds to recording after preparation. Dry run
reads drive status, snapshots inputs, builds and verifies the provisional bundle,
then stops. It neither reserves nor sends a write command to the drive.

## Transaction

- `source/`: normalized PCM WAVs, a normalized artwork snapshot, companion
  files/directories, and TOML. The snapshot includes empty directories and is
  hashed before use. Changing the original project later does not alter the job.
- `planned/`: full authored bundle using the lesser of requested capacity and
  reported blank-media capacity. Nothing from its provisional data session is
  written to the drive.
- Music: DiscRecording audio tracks, exact requested pregaps, required SAO,
  appendable flag on, audio checksum verification requested. First pregap is
  150 frames; subsequent gaps come from the project.
- NWA: read `DRNextWritableAddressKey` from the open empty/invisible track
  exposed by `DRDeviceTrackRefsKey` / `DRDeviceTrackInfoKey`. An ambiguous or
  missing address is an error. The recorded closed music track numbers, starts
  and session must match. There is no fallback to an estimated NWA and no
  arbitrary `+150` adjustment. The reported address must be in the expected
  second-session range; unexpected drive conventions stop the job.
- `actual-N/`: rebuilt volume, cooked ISO, RAW, checksums and previews using
  the observed NWA. If the writable range is shorter than the planned extent,
  increase the end margin while preserving the recording geometry. The same
  disc/strip sampler remains in use. Before writing, recheck source hashes,
  session layout, reservation, NWA and remaining capacity.
- Data: a custom `DRTrack` producer reads only the cooked ISO in 2048-byte
  blocks. The SDK documents the producer address as track-relative, so it is
  multiplied by the block size, not offset by the NWA a second time. The
  filesystem extents already contain absolute LBAs. The native prepare callback
  checks the media status supplied by DiscRecording and any assigned track
  start it exposes. Required TAO, appendable off, `ProduceAgain` verification,
  no overwrite and no eject on failure. DiscRecording writes Mode 1 framing
  and finalizes the disc. The RAW artifact is not sent to the drive.
- Completion: require DiscRecording's terminal success, finalized media with
  two sessions, matching original music layout, and a session-2 data track at
  the measured start with sufficient length. Successful writes request mount;
  the app never requests ejection.

The bridge holds the application's media reservation across both writes and
reauthoring. The framework manages write-time device access. Rechecks reject
an unexpected disc layout; a CD-R does not expose a portable unique serial via
these APIs. Keep the same disc in the drive throughout the job.

## Journal and recovery

`journal.json` is replaced atomically and synced before each write intent.
An OS file lock prevents concurrent processes from using the same job. Final
snapshots report the state last observed by the backend, not a prediction.

| Stage | `resume` behavior |
| --- | --- |
| `prepared` | Revalidate inputs, acquire the selected drive, and start music on a blank CD-R |
| `music_written` | Confirm the music session, read NWA and generate data; never rewrite music |
| `data_ready` | Recheck NWA and available range; reuse verified assets only if they still fit, otherwise reauthor into a new attempt directory |
| `music_writing`, `data_writing` | Uncertain write; refuse automatic continuation |
| `preparing`, `complete` | No automatic continuation |

Ctrl+C requests DiscRecording abort during a write. During offline preparation
it prevents the next write. If a write was started, inspect the media and the
journal before any further operation. There is no erase or overburn path.

## Backend and acceptance

`device::Recorder` abstracts status, reservation, appendable music write and
finalizing data write. `burn` uses only this interface. The Rust macOS adapter
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

The native framework can construct the audio and data producers without a
drive; orchestration and failure cases are exercised with a fake backend. At
implementation time no optical drive was connected. Physical SAO appendability,
NWA behavior on specific drives, TAO finalization, player playback, independent
mounted-file readback and photographed artwork are not yet validated. The current
data track is Mode 1; CD-ROM XA/Blue Book content constraints are separate work.
