# Validation

Validation was performed locally on macOS. Consumer CD players and full
Blue Book conformance remain unverified. CI covers three operating systems;
physical recording results below apply only to the tested combinations.

## 0.3.1 core review

Rendering now shares one drawing-source selector and one spiral geometry
model. Pure target patterns are separate from photo/file workflows. Preparation
limits writable extents without changing the configured reference geometry;
mock recording verifies that the same absolute LBA retains its position after
capacity restriction and NWA rebuild.

Four fixtures captured before the refactor pin ISO, RAW, design, payload
readback and difference hashes (20 assets). The fixtures cover disc artwork,
strip artwork, radial bands and angular pulses. All match byte-for-byte after
reorganization. This checks preserved software behavior, not optical accuracy.

Local release checks passed: rustfmt, Clippy with warnings denied, 34 tests
(including the new smaller-media rejection), and an optimized 0.3.1 build.
The new binary also verified the previously burned album bundle: 15 assets,
one ISO file extent and all 239177 Mode 1 sectors. No new physical write was
performed for this code review.

## Accepted SW80EU1 album

On 2026-11-05, an LG GX40N (RQ00) wrote a Verbatim SW80EU1 CD-RW at requested
10×, following a full erase. Independent physical readback matched all 108370
audio sectors across six tracks and all 239177 Mode 1 data sectors. The data
track started at measured NWA 120520; the final disc had two closed sessions
and seven tracks, with no unreadable or mismatching data LBAs.

Data ISO and physical-read SHA-256:

```text
eb9c4169b9068eff9c75e9335bf931e260e5403d9b3ad123c1efa39ec69dcf12
```

The user confirmed legible lettering. A photo-based estimate at one visible
inner/outer reference pair found about 0.55° residual drift after angular
correction, compared with roughly 14° before it. This does not establish
uniform accuracy over the disc. All 38 data-write progress samples that
reported speed showed 10.1×; discrete samples cannot exclude transients.
The matched geometry is saved in the `sw80eu1` [preset](../calibrations/sw80eu1/disc.toml).
The 0.3.1 reorganization is verified offline and with mocks, without another
physical erase or burn.

## CD radial calibration and CD-RW

An offline CLI fixture authors a calibration target at a supplied NWA, checks
its on/off payload and fits known radii from synthetic photo coordinates. It
checks malformed rim geometry and a mismatched bundle, then authors verified
bundles in both strip and disc mode using the fitted profile. The recording
fixture checks that a used CD-RW is rejected, and a blank CD-RW with SAO and TAO
reaches two-session completion while retaining the frozen profile through the
measured-NWA rebuild.

On a Pioneer BDR-XD05 (firmware 3.11) with CD-RW, DiscRecording verified the
first session, and both DiscRecording and MMC track information reported LBA
11850 for the next writable sector. The initial data producer failed. With
DiscRecording's ISO image loader, the second attempt entered track writing but
the CLI stayed at 0% with a pending device write until cancellation. After
cancellation the closed Mode 1 track contained 10432 of the planned 347847
sectors. The first marked band started beyond the written extent.

An Apple SuperDrive (firmware 2.00) finalized a complete CD-RW target with
two sessions at NWA 11850. All 347847 Mode 1 data sectors matched the authored
ISO on raw readback. The photo fit remains unverified.

## CD-RW angular artwork

An LG GX40N (firmware RQ00) finalized a three-band CD-RW target. All six
audio tracks matched their WAV files, and all 118630 Mode 1 data sectors
matched the authored ISO. The observer saw only arcs across all three bands.
The bands sampled different radii with local prewarps of −120, 0 and +120
degrees per millimeter. This run produced no usable angular calibration.

## AZO Gold Vinyl

On 2026-11-07, an Apple SuperDrive (firmware 2.00) recorded and verified a
single-session AZO Gold Vinyl CD-R target. The 36-point read-timing
fit had a residual RMS of 0.185°. Four photos yielded a radial fit of
25.05–57.97 mm at 359,846 sectors, with a residual RMS of 0.077 mm.

A second disc from the batch was recorded at requested 24× with six audio tracks
and 238,887 Mode 1 data sectors starting at LBA 120520. PCM, data and CD-Text
passed digital verification. The user confirmed the artwork alignment on the
finished disc. The geometry used for that burn and the photo fit are stored
in separate tables in the [AZO Gold Vinyl configuration](../calibrations/azo-gold-vinyl/disc.toml).

## Recording workflow

The macOS DiscRecording bridge was compiled and linked with warnings denied.
Its hardware-free test constructs a system audio track from a 10732-sector PCM
WAV, opens an ISO with DiscRecording's image loader, and exercises the track
metadata shape observed on the Pioneer BDR-XD05.

On the Apple SuperDrive, DiscRecording reported an audio checksum error
at the second track of a six-track album. Independent raw readback matched
all six normalized WAVs byte-for-byte. The data session was appended at LBA
120520, finalized, and all 239177 Mode 1 data sectors matched the authored
ISO. The revised workflow makes independent PCM readback a condition of
the data append and moves decoder length checks ahead of writing.

Mock-backend tests run the real offline authoring/verification pipeline around
both write calls. They cover a no-write dry run, immutable snapshots and empty
directories, job locking, an observed NWA different from the planned address,
reduced remaining space, NWA disappearance/change, resuming without replaying
music, write interruption, final data address mismatch and rejection of modified
snapshots/unsupported media. They also check decoder rounding and resume after a
PCM mismatch. The job journal is checked at the moment each
backend write is called. These tests do not exercise a laser or certify a drive.

## Templates and strip mapping

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
