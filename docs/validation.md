# Validation — 2026-11-05

Validation was performed locally on macOS. Remote three-OS CI, physical burning,
consumer CD players, Blue Book conformance and photographed artwork were not tested.

## 0.4.0 recording workflow

The actual macOS DiscRecording bridge was compiled and linked with warnings
denied. Its hardware-free test constructs a system audio track from a 300-sector
PCM WAV and reads two distinct ISO blocks through the native producer using a
nonzero relative address. Device discovery was run on this Mac and returned no
attached optical drive.

Mock-backend tests run the real offline authoring/verification pipeline around
both write calls. They cover a no-write dry run, immutable snapshots and empty
directories, job locking, an observed NWA different from the planned address,
reduced remaining space, NWA disappearance/change, resuming without replaying
music, write interruption, final data address mismatch and rejection of modified
snapshots/unsupported media. The job journal is checked at the moment each
backend write is called. These tests do not exercise a laser or certify a drive.

## 0.3.0 template and strip extension

The suite additionally checks actual WAV length changes updating template
radii, an optional supplied session start, transparent protected regions,
recommended strip aspect ratio, escaped SVG titles, hidden guides and existing
directory rejection. Templates can be exported while the artwork path still
does not exist.

An asymmetric strip with different quadrants and radial rows establishes that
left-to-right goes clockwise from 12 o'clock and top-to-bottom goes outer to
inner. Rotation and design-preview pixels are checked against those known
regions. A complete strip-mode build is verified, including its preserved
snapshot aspect ratio and descrambled payload values at multiple LBAs in both
black and white portions of a half-black/half-white strip. Existing disc-mode
rotation, clipping, reproducibility and integrity tests remain in place.

A full-size 30-minute strip example was also built at session start 146475.
Its exported 1600-pixel disc template recommends a 4359×212 strip for the
42.61–57.99 mm annulus. Four asymmetric text labels and a line near the strip's
bottom were projected around the circumference, then checked in the actual ISO
payload readback. All 12 assets, two ISO files and 213375 Mode 1 sectors passed
`verify`. SVG templates were parsed, and a headless-browser rendering of the
disc SVG matched the PNG mask at the hole, outside and paintable sample points.

## Automated checks

`cargo fmt --all -- --check`, Clippy with warnings denied, and `cargo test --locked`
cover WAV bounds, odd RIFF chunks, sector alignment, unchanged asymmetric PCM,
capacity rejection, protected preview regions, rotation, tone, filesystem extent
rebasing and directory records spanning several sectors. Complete builds are
compared for byte-identical asset hashes. Existing output directories are
rejected. A RAW parity byte is changed and its asset hash updated; verification
must still reject the invalid EDC/ECC sector.

## Independent Mode 1 reference

The golden unit vector is LBA 12345 with payload byte `i mod 256` for `i=0..2047`.
EDC was computed with a separate, bit-at-a-time reflected polynomial recurrence,
not the Rust lookup table. P/Q ECC was computed using the explicit row-offset
and GF tables from [libchdr's cdrom implementation](https://github.com/rtissera/libchdr/blob/master/src/libchdr_cdrom.c).

Reference source SHA-256:

```text
7fc4515eb3598d88797b2786a560bd497720eb82f9b8c6391ba4ab7a74f5e351
```

The resulting full 2352-byte sector has SHA-256:

```text
891347f572c1920d48b495668d21e0123beefeb2ad454a83e3aa9e9c691da11f
```

The same independent tables verified six diverse sectors from a generated
session: zero/system area, primary and Joliet descriptors, metadata/file area,
first drawing sector and last sector. The external source/tables are not a
runtime dependency and are not copied into this repository.

## Independent ISO/Joliet reader

[PyCdlib 1.14.0](https://github.com/clalancette/pycdlib) was installed only in a
`/tmp` validation environment. It successfully opened the authored volume and
extracted English content, Chinese content in a nested directory, and an empty
file, matching the originals byte for byte.

Because PyCdlib opens a single volume rather than an audio/data multisession
disc, the validation view exposes the second-session descriptors at byte offset
`16*2048`, while all directory/file accesses keep their absolute disc LBAs.
Other reads at or beyond `session_start*2048` map to the ISO stream after
subtracting that start. This is a read adapter, not a rebased exported image.

## Full-size offline run

A 30-minute synthetic silent album (three ten-minute tracks) was authored using
the nominal 360000-sector budget and a supplied session start of 146475, 75
sectors beyond the default planned start. A Chinese companion text file was
included. Parameters and resulting bounds:

| Item | Value |
| --- | --- |
| Music | 135000 sectors |
| Session 2 INDEX 01 | LBA 146475, source `provided` |
| Data reserve | 10000 sectors |
| Drawing extent | LBA 156475–359850, end exclusive |
| Second-session length | 213375 sectors |
| ISO length | 436992000 bytes |
| RAW length | 501858000 bytes |
| Bundle assets / ISO files | 12 / 2, including `ARTWORK.BIN` |

CLI `build`, `verify` and a denser `readback --samples 128` were exercised. The
independent reader extracted the companion file and confirmed both the drawing
extent and its byte length. Design, payload readback and difference images were
visually inspected. The run does not establish the correctness of physical
placement, final lead-out behavior or a real drive's NWA.

## Physical-source boundary

The position convention follows the pinned
[DiscImageStudio CD data encoder](https://github.com/JiaFeiMiao-K-Cat/DiscImageStudio/blob/3a5cde635c182d6a7e01ff99c2cc1be2ac3ab668/src/DiscImageStudio.Cd/CdDataImageGenerator.cs)
and its CIRC offset mapping. Its
[physical model audit](https://github.com/JiaFeiMiao-K-Cat/DiscImageStudio/blob/3a5cde635c182d6a7e01ff99c2cc1be2ac3ab668/docs/PHYSICAL_MODEL_AUDIT.md)
identifies unresolved physical timing and calibration questions. Passing these
software checks must not be promoted to a claim of full CIRC/EFM simulation,
accurate reflectivity or a successfully burned CD Extra.
