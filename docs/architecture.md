# Architecture

One crate contains a platform-independent offline pipeline and a thin CLI.
Authoring remains free of device calls; recording is a separate workflow.
Keep one-word filenames, direct domain functions and a small number of modules.

```text
src/
  main.rs       CLI parsing and orchestration
  project.rs    strict TOML settings, defaults and path resolution
  audio.rs      bounded RIFF inspection and unchanged PCM export
  layout.rs     sector budgets, geometry inputs and estimated radii
  art.rs        shared image fitting, tone processing and physical sampling
  codec.rs      CIRC position map, scrambler, Mode 1 framing/EDC/ECC
  iso.rs        ISO9660/Joliet tree, absolute extents and sequential prefix
  author.rs     audio/data bundle generation, manifest and verification
  preview.rs    design report, sampled payload readback and difference image
  template.rs   editable disc/strip canvases and album-specific dimensions
  device.rs     device status, discovery and Recorder interface
  burn.rs       journaled two-session recording policy and snapshot validation
  macos.rs      typed Rust adapter to the native recorder
  lib.rs        public modules and shared hexadecimal formatting
native/
  macos.m       DiscRecording bridge (built only for the macOS target)
tests/
  offline.rs    WAV, filesystem, CLI and complete-authoring integration tests
  recording.rs  mock-device recording, recovery and safety boundaries
docs/
  architecture.md
  preview.md
  recording.md
  validation.md
```

The offline pipeline shares project, geometry and image types and has no
platform dependencies. Recording uses a narrow trait; the small Objective-C
bridge is the only native implementation, linked conditionally by `build.rs`.
Keep one crate while that boundary is sufficient. A separate drive crate can
make sense once multiple platforms bring substantial native build dependencies.
Avoid a file per type or an empty crate per platform.

## Offline data flow

1. Parse the manifest and inspect normalized WAVs. Sector arithmetic is bounded
   before use; track tail padding is explicit. Relative paths resolve against
   the TOML file rather than the process's current directory.
2. Scan real file/directory entries, reject symlinks/special files, allocate
   primary and Joliet metadata and file extents. Use stable ordering, fixed ISO
   timestamps and bounded directory/name limits. Native primary identifiers get
   deterministic aliases where needed; Joliet preserves BMP names.
3. Create a new output directory with an `incomplete` marker. Snapshot normalized
   source artwork and copy music PCM without processing it. Failures preserve
   the marker and cannot masquerade as completed bundles.
4. Write ISO metadata, user file snapshots and reserved space, then generate
   the separately allocated `ARTWORK.BIN` extent. The drawing cannot overwrite
   source files, directories or music.
5. Generate the corresponding raw Mode 1 sectors: sync, BCD MSF, user data,
   EDC, zero reserved bytes and P/Q ECC. Image sectors are unscrambled; the
   recording device must perform physical scrambling, CIRC and EFM.
6. Export design, actual-payload readback and difference previews; hash assets
   and ISO file extents; write the versioned manifest last and remove the marker.

`author` orchestrates this flow through music and data stages. `iso` owns the
filesystem allocation, `codec` owns sector transformations, and `art` owns
source sampling. Graphical exports consume the same `Plan` as encoding; the
readback additionally reads the actual ISO rather than re-rendering its source.

## Session-relative streams and absolute LBAs

The first music INDEX 01 is disc LBA 0. Subsequent music gaps contribute to
session 1's program length. The first lead-out / next lead-in / data pregap are
represented by the configured session-transition budget. Session 2 starts at
`music_sectors + gap_sectors + session_reserve_sectors`, unless a supplied start
changes that budget before planning.

Both volume descriptors and directory/path-table extents refer to absolute
LBAs. The session ISO stream's offset zero is session 2 INDEX 01. For any file:

```text
stream offset = (absolute file LBA - session_start) * 2048
```

A separate-write recording backend must obtain the actual NWA and reauthor
session 2 at that start. Changing only JSON or only RAW headers is insufficient:
filesystem extents, framing and physical artwork phase all depend on it.
The manifest labels the start `planned` or `provided`; neither label proves
hardware acceptance. The per-session CUE/TOC cannot convey the complete disc.

The v1 bundle is an enhanced-CD layout with a Mode 1 second session, not a
CloneCD image and not full Blue Book compliance. CD-ROM XA / CDPLUS requirements
and hardware/player compatibility remain separate implementation milestones.

## Encoding model and limits

The CIRC inverse position table and pair-swap convention adapt the pinned
DiscImageStudio implementation. A drawing payload byte is selected from the
source at its modeled physical position, converted to `0D/AA`, then XORed with
its Mode 1 scramble byte. File and filesystem bytes are never compensated as
images. EDC/ECC covers the resulting valid logical sector.

The projection uses absolute disc position, a constant-pitch area approximation,
linear velocity and start angle. Gray tone is deterministic byte-phased
halftoning; threshold uses a hard cutoff; ordered tone uses a fixed 16-phase
sequence. Design-pixel dithering is deliberately separate. Disc sources normalize
to 1600 square pixels; strip sources preserve aspect ratio with their longest
side normalized to 4096. Both composite alpha over white. `Sampler` owns inverse
rotation, disc/strip coordinate mapping and tone, consumed by preview and
authoring alike. Strip width maps clockwise from 12 o'clock, and height maps
outer to inner artwork radius. Templates use these same plan radii.

`Plan::from_project` centralizes optional session-start handling for templates,
previews and authoring. Keep `--session-start` consistent across those operations.
Template export does not load an artwork image, so users can design after music
has established the available annulus. The template module owns only canvases,
masks and dimension metadata; it does not introduce another rendering engine.

This is not a full CIRC/EFM encoder or reflectivity simulator. Parity bytes,
sector framing and finite-stream boundary/latency effects can reduce contrast
or affect placement. The upstream timing audit's concerns remain relevant.
Do not equate self-readback consistency with correct on-disc physical phase.
See [preview.md](preview.md) and [validation.md](validation.md).

## Recording backend boundary

| OS | Current discovery | Recording route |
| --- | --- | --- |
| macOS | DiscRecording device and media status | Implemented Objective-C bridge; hardware acceptance pending |
| Windows | CIM `Win32_CDROMDrive` | IMAPI2 COM via Rust Windows bindings |
| Linux | SCSI peripheral type 5 in sysfs | Evaluate cdrdao/cdrtools or libburn |

Apple's SDK `DRBurn.h`, `DRDevice.h` and `DRTrack.h` define the burn,
media-reservation and track status APIs used in [recording.md](recording.md).
Microsoft documents distinct writers in
[IMAPI Interfaces](https://learn.microsoft.com/en-us/windows/win32/imapi/imapi-interfaces).
[cdrdao](https://github.com/cdrdao/cdrdao) and the
[cdrecord manual](https://cdrtools.sourceforge.net/private/man/cdrecord/cdrecord.1.html)
are references for Linux mode/capacity/append behavior.

Each backend must establish real blank CD-R capacity, supported writing modes,
actual session start, ownership and finalization semantics. Do not silently
substitute a single-session mixed-mode disc. Independently accept audio on
players, files by hash and photographed artwork; CI cannot establish those.
