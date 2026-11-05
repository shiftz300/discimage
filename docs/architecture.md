# Architecture

One crate, with platform-independent authoring and a separate recording workflow.
The native boundary is `device::Recorder`. Additional crates are unnecessary
until another backend introduces substantial native dependencies.

| Module | Owns |
| --- | --- |
| `project` | Strict TOML settings, radial profiles and relative paths |
| `audio` | RIFF validation, sector alignment and unchanged PCM export |
| `layout` | Track budgets, session extents and available artwork radii |
| `spiral` | Shared sector/radius/angle mapping, independent of writable limits |
| `target` | Pure radial bands and angular pulse patterns |
| `art` | Disc/strip image sampling, tone and selection of the drawing source |
| `codec` | Inverse CIRC positions, scrambler and Mode 1 EDC/ECC framing |
| `iso` | ISO9660/Joliet allocation, absolute extents and file snapshots |
| `author` | Bundle construction, manifest, hashes and integrity verification |
| `preview` | Design report, ISO payload projection and difference image |
| `template` | Editable canvases and masks derived from the album layout |
| `calibrate` | Target export, local photo picker and radial fitting |
| `burn` | Journaled two-session state machine, snapshots and recovery |
| `device` | Device descriptions, discovery and the backend interface |
| `macos` | Owned JSON FFI adapter and PCM readback |
| `main`, `guide` | CLI commands and interactive calibration prompts |

`native/macos.m` wraps DiscRecording and is built only on macOS.
`mark.html` is the self-contained photo point picker embedded by `calibrate`.
Pure rendering and configuration do not depend on the calibration workflow;
`calibrate` re-exports its former public configuration types for compatibility.

## Offline pipeline

```text
Project + WAVs → Plan → Volume + Drawing → ISO / RAW / manifest
                   ├→ templates
                   └→ design preview
                              ISO → payload readback → difference image
```

`Plan::from_project` applies the same optional session start everywhere.
`Plan::session_start` and `data_sectors` supply shared session boundaries.
`Drawing` selects blank fill, an image, radial bands or angular pulse bands
once. Both encoding and design previews use it. Pixel dithering in design
previews remains distinct from byte-phased physical dithering.

`Spiral` owns the constant-pitch area model and its inverse. The capacity and
radii define a **reference geometry**; available media space only limits the
write extent by increasing the end margin. Neither preparation nor a later
NWA rebuild should silently change the calibrated spiral.

Builds snapshot inputs, allocate `ARTWORK.BIN` separately from user files,
write the ISO and RAW, export previews and hash all assets. An `incomplete`
marker remains until the manifest is complete. Existing output directories
are rejected. Paths in a project resolve relative to its TOML file.

## Addressing and encoding

Music INDEX 01 starts at disc LBA 0. Session 2 starts after music, intertrack
gaps and the session-transition reserve, unless an actual NWA is supplied.
The ISO stream begins at session 2 INDEX 01, but its extents contain absolute
LBAs:

```text
stream offset = (file LBA - session start) × 2048
```

Changing NWA requires rebuilding the filesystem, framing and artwork phase.
The recorder never substitutes a guessed address. Separate CUE/TOC files are
per-session descriptions, not a portable whole-disc image.

The encoder maps each artwork payload byte through the upstream CIRC position
model and spiral, selects `0D` or `AA`, then compensates the Mode 1 scrambler.
File and directory bytes remain unchanged. RAW sectors include sync, MSF,
EDC and P/Q ECC. The drive still supplies physical scrambling, CIRC and EFM;
this is not a full channel encoder or reflectivity simulator.

## Recording boundary

`burn` freezes source files and verifies them before each write. A job progresses
through prepared → music writing → music written → data ready → data writing →
complete. Write intent is persisted before a native call; uncertain writes
cannot resume automatically. The backend must verify PCM before appending,
use the measured NWA, verify data and report final two-session layout.

Device quirks such as system audio-decoder rounding and SuperDrive CD-RW gap
requirements live behind `Recorder`. Offline rendering has no device branches.
Windows CIM and Linux sysfs currently provide discovery only; implementing a
new recorder must preserve reservation, NWA and finalization semantics.

## Validation boundaries

`tests/offline.rs` checks complete bundles, image orientation, calibration and
unchanged media bytes. `tests/encoding.json` pins pre-refactor ISO/RAW and
preview hashes for four drawing modes. `tests/recording.rs` exercises the real
authoring pipeline with a mock recorder, including capacity limits, changed
NWA, interruption and immutable snapshots. macOS tests exercise framework
producers and PCM comparison without writing a physical disc.

Digital integrity, photographed appearance and player compatibility are
separate acceptance checks. See [validation](validation.md),
[recording](recording.md) and [previews](preview.md).
