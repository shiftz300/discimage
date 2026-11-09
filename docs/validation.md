# Validation

Earlier physical validation was performed locally on macOS. Consumer CD players and full
Blue Book conformance remain unverified. CI covers three operating systems;
physical recording results below apply only to the tested combinations.

## Windows XD05 bring-up, 2026-11-09

Windows discovery, hardware identity, MMC metadata, exclusive IMAPI2 reservation
and release were exercised on PIONEER BDR-XD05 firmware 3.11. The inserted CD-RW
reported ATIP 97:34:23, 359,847 free sectors and 4x/10x write-speed descriptors.
This same disc had previously worked in an Apple optical drive on macOS; this
session does not establish that the disc is faulty.

An explicit OPC attempt returned fixed sense `70/03/73/03` before audio transfer.
A later 4x attempt without standalone OPC accepted the CUE and buffered audio
transfers, then returned deferred sense `71/03/73/03` after SYNCHRONIZE CACHE.
The job remained `music_writing`; no data append or successful recording was
claimed. Full and quick MMC erase attempts also failed. A separate Windows
IDiscFormat2Erase comparison reported a media-speed mismatch. Reloading the same
disc restored its original blank/capacity metadata. These observations do not
isolate the cause to the disc, the drive, or their Windows write strategy.

The local, ignored `calibrations/work/` directory retains the original job
manifests and `windows-xd05-*.log`/JSON evidence, including
`windows-xd05-smoke-auto-opc.log` with actual CDBs and sense data. All uncertain
jobs and their original inputs remain protected from automatic retry.
Successful XD05 recording and erasing remain unverified. Subsequent Apple-drive
control results are recorded below.

Software tests cover the actual Windows MMC recorder through simulated command
responses: exact gaps, CD-Text, little-endian PCM, NWA, full data comparison,
corrupt readback, uncertain writes, power-calibration errors, and unsupported
speed rejection before creating a recording journal. Pure MMC encodings are
tested on all platforms. Windows measurement uses DiscProbe's period/phase and
geometry algorithms with slow-USB cache screening, CLV screening, opcode-error
handling and scoped scheduling priority. Software coverage is separate from
physical results.

Local Windows validation passed 86 tests (46 library, 4 CLI, 13 offline and
23 recording tests), formatting and Clippy with warnings denied. The final
flat-package recovery-message fix also passed both Windows recorder tests.
The Release executable was rebuilt after those checks.

## Windows Apple SuperDrive control, 2026-11-09

The same CD-RW was moved to an Apple USB SuperDrive (USB 05ac:1500). Windows
initially exposed the unpowered bridge as `Apple SuperDrive (2.00)` and the slot
would not accept a disc. A standalone, identity-checked SPTI helper sent the
non-data Apple wake CDB `EA 00 00 00 00 00 01`; status was good and the user
confirmed normal disc loading. No Apple driver was installed. After loading,
INQUIRY exposed the actual HL-DT-ST DVDRW GX40N (RQ00), with hardware selector
`26302db3b7be`. The temporary bridge selector must not be saved as a hardware ID.

With identical short-target inputs and the same Windows backend, the drive
rejected 4x during preflight and reported only 10x. At supported 10x it completed
the full two-session job:

- Audio: 300 sectors of silence, compared byte-for-byte. MMC reported 298 sectors;
  the two additional sectors were included in the actual PCM comparison.
- Actual data NWA: 11700. Data: 4650 cooked Mode 1 sectors, all read back and matched.
- Final medium: closed CD-RW, two sessions, two closed tracks; package stage `complete`.
- Local package verification passed again after recording.

Evidence is retained in `calibrations/work/windows-apple-smoke/`,
`windows-apple-smoke-burn.log`, `windows-apple-after-smoke.json` and
`windows-apple-wake.log`. This is a digital control result; physical artwork,
consumer-player behavior and numerical read-timing acquisition remain separate
validation steps. Erase and single-session results follow below.
The control narrows investigation to XD05-specific initialization/write-strategy
behavior and does not establish that the XD05 hardware is defective. The user
chose to retire that XD05 from this CD workflow; Apple GX40N is now the saved
default drive. Blu-ray behavior was not tested.

The standalone wake helper was used only for local bring-up. The application
does not include Apple-specific wake commands or install drivers; device
initialization is delegated to the separately installed driver. Discovery
excludes the uninitialized Apple bridge identity from portable hardware IDs.

## Windows calibration parity, 2026-11-09

The same Apple GX40N and CD-RW completed a full MMC erase in approximately
500 seconds. The backend confirmed blank state and writable capacity before
preparing the same single-session target generator used by macOS. Automatic
write-speed selection requested the maximum reported 10x (750 sectors/s).
All 359,697 cooked Mode 1 sectors matched on physical readback, and the final
layout was confirmed as one closed session with one data track at LBA 0.
Only then was `pending.json` promoted to `single.json`.

The new exclusive IMAPI2 measurement transport passed a separate-process
ownership test: a second process could not reserve the drive while the probe
held it, and could reserve it after probe destruction. The Release
`calibrate probe --check` then confirmed the recorded range `0..359697`.
QPC timestamps surround synchronous native commands, excluding buffer
preparation and result copying. These initial parity runs requested MMC maximum
available read speed; accepting the request does not establish stable rotation
or an accurate geometry fit. A later low-speed default comparison is recorded
below.

Actual acquisition completed all 36 rungs over measured LBA 10790..=305740.
The maximum-speed request was accepted. Ordinary repeated reads were cached
(median 1.544 ms), while FUA reads reached media (median 24.219 ms); FUA was
selected. Inner/outer periods were 12.1095/12.1150 ms, so the CLV screen passed.
The final fit RMS was 1.770 degrees, exceeding the existing 0.3-degree candidate
import gate. The completed acquisition is retained as evidence, not applied as
a calibration preset. The disc was then ejected through IMAPI2 and no-media
state confirmed, allowing the user to install a separate device driver.

The full Windows suite passed 89 tests (48 library, 4 CLI, 13 offline and
24 recording), with the opt-in hardware reservation test run separately and
passing. Formatting and Clippy with warnings denied also passed. A further
focused regression covers best-effort tray release when PREVENT fails after
submission; it sends no media writes. Apple-specific automatic initialization
is absent from the application.

Ignored local evidence is retained under `calibrations/work/windows-parity/`
(`record.log`, `record/single.json`, `reservation.log`, `probe-check.log`),
plus `probe.log`, `measurement/` and `eject.log`, with the erase log at
`calibrations/work/windows-calibration-erase.log`.

After the user installed the separate optical-drive driver, discovery again
confirmed the same GX40N hardware identity and the same 359,697-sector finalized
single-track calibration disc. A second maximum-speed acquisition completed all
36 rungs. RMS was 2.7499 degrees and measured-span sigma 1.7664 degrees; both
timing candidate limits still failed. Both runs used verified FUA reads and
showed only 0.0175% / 0.0207% period spread. Median individual phase scatter was
9.394 / 9.737 degrees, equivalent to approximately 316 / 328 microseconds at
the measured 12.11-ms revolution period. Installing the driver did not establish
an improvement in timing precision in these runs.

A short read-only control using upstream DiscProbe's original Python IMAPI2
transport and period/phase analysis reproduced 8.199-degree reference jitter
(275.8 microseconds) and 10.904-degree target scatter with 64 read pairs.
The high noise is therefore observable outside this project's Rust phase and
geometry implementation. This does not isolate its source among USB delivery,
driver scheduling, mechanism completion behavior or media-dependent latency.
Upstream's measured-span noise cutoff is 0.6 degrees; both full runs exceed it,
as well as this project's stricter candidate gates. Acquisition success and
precision suitability must be reported separately. Existing photo-fit records
remain available independently; no new photo data was needed for these tests.

The SPTI control was skipped because a normal exclusive lock returned Windows
error 6 before any SPTI read was issued. No mounted-volume check was bypassed.
Evidence is retained in the ignored `measurement-driver/`, `comparison.json`,
`transport-control.json` and corresponding logs beside the first run.

A later speed control identified an important comparison confound: the previous
macOS measurement code requested 1411 kB/s, whereas the Windows runs above
requested maximum speed. Earlier macOS fit results cannot be used as a matched
OS comparison without controlling speed and medium. A same-disc Windows control
using upstream IMAPI2 and analysis waited for consecutive stable period fits
before collecting 64 reference/target pairs at each setting:

| Read request | Measured rpm | Period | Reference timing jitter | Reference phase jitter |
| --- | ---: | ---: | ---: | ---: |
| Maximum | 4954.48 | 12.1103 ms | 279.74 us | 8.316 degrees |
| 1411 kB/s | 2090.41 | 28.7025 ms | 286.92 us | 3.599 degrees |

The nearly unchanged timing jitter produces less angular error at the lower
rotation rate. Target phase scatter also decreased from 10.044 to 3.478 degrees
(mean uncertainty 1.255 to 0.435 degrees). This is a short sampling control,
not another full 36-rung fit or proof that candidate-import gates pass. It
demonstrates that speed accounts for a substantial part of the angular-noise
difference; remaining platform and media differences are not isolated. The
maximum read-speed request was restored on exit and the application default
was unchanged. See ignored `read-speed-control-settled.json` and its log. The
earlier un-settled maximum-speed short block was invalid and is not used here.

Following the speed control, the measurement default was changed on both macOS
and Windows to a conservative 1411 kB/s request. Recording and erase defaults
remain maximum supported speed. Eight probe regression tests, formatting,
strict all-target Clippy and the Release build passed after this change.

A complete 36-rung Windows run at the new default then used the same GX40N,
calibration disc, requested LBA range and 32 read pairs per target as the two
maximum-speed runs. Actual period was 28.7028 ms (approximately 2090 rpm), FUA
reads were verified and the CLV screen passed. The final RMS was 0.7325 degrees,
measured-span sigma 0.4560 degrees and chi2/dof 1.2258. Period spread was 0.0183%.
Median individual phase scatter was 3.948 degrees, equivalent to 314.7 us,
compared with 9.394/9.737 degrees (316/328 us) at maximum speed. The angular
improvement therefore occurred without a corresponding decrease in absolute
read-completion timing scatter.

The lower-speed run qualifies as upstream's "usable, but noisy" (chi2/dof <=2
and measured-span sigma between 0.25 and 0.6 degrees), whereas both maximum
runs were too noisy. It passes this project's 0.5-degree span-sigma limit but
still exceeds the stricter 0.3-degree RMS candidate gate; no preset was
automatically imported or applied. This establishes lower internal fit error
for the tested speed/disc/drive, not independently verified physical artwork
accuracy. All three completed receipts and their summary/target hashes were
checked. Evidence: ignored `measurement-low1411/`, `probe-low1411.log` and the
updated `comparison.json` under `calibrations/work/windows-parity/`.

For a user-selected budget of approximately 20 minutes, a temporary 96-pair
configuration completed the same 36-point range in 1231.5 seconds (20.5 min).
Default automatic fast screening measured 6.531-degree reference jitter and
fell back to 1411 kB/s; cache and rotation checks were repeated before sampling.
The completed slow run measured a 28.7017-ms period, verified FUA reads and
0.00172% period spread. RMS improved from 0.7325 degrees at 32 pairs to 0.5311
degrees at 96 pairs; measured-span sigma improved from 0.4560 to 0.3087 degrees.
Median individual scatter remained approximately 3.94 degrees / 314 us, while
the median uncertainty of a target mean decreased from 0.698 to 0.402 degrees.
Chi2/dof was 1.8471. The existing 0.3-degree RMS import gate still failed.

The experimental configuration used 36 targets, 96 pairs/target, conservative
1411 kB/s and verified FUA. It is retained only as a local comparison record;
the experimental modes and sampling options were subsequently reverted.
Normal sampling remains 32 pairs; no preset
was applied and the import limits were not relaxed. This single comparison
does not guarantee that further averaging reaches 0.3 degrees or establishes
an absolute physical accuracy. Among 28 shared targets, residual correlation
between the two fitted runs was -0.132; this does not establish a repeatable
fixed-position bias. Raw read-timestamp evidence is insufficient to identify
the remaining timing-noise mechanism.

Local evidence: `measurement-auto96/`, `probe-auto96.log`, `auto96-run.json`,
`comparison.json` and `residual-comparison.json` in the ignored work directory.
The receipt explicitly records requested auto mode, selected slow mode,
96 pairs/target, the rejected fast attempt and elapsed time. Its summary and
target checksums passed verification. At the user's request, production
measurement was restored to the same fixed 1411 kB/s and 32-pair behavior on
Windows and macOS, without automatic fast screening, speed fallback, `--slow`
or `--reps`. Windows native transport, exclusive ownership and safety boundaries
remain in place. Earlier experiment receipts describe the tested version only.

## Windows six-track CD-RW workflow, 2026-11-09

The same GX40N and authorized CD-RW completed a full album workflow using an
existing SW80EU1 timing profile. The package was built and its seven frozen
inputs verified before full erasure. Automatic write speed selected the
supported maximum 10x for erasure and both recording sessions.

All six audio tracks (108,370 source sectors) matched physical PCM readback
byte-for-byte. The observed second-session NWA was 120520. All 235,479 cooked
Mode 1 data sectors matched the authored image. Final discovery confirmed two
closed sessions, six audio tracks and one data track, all closed, with the
recording journal at `complete` and device reservation released.

A separate read-only READ TOC format 5 comparison fetched 14 CD-Text packs.
Every pack CRC matched; album/track titles and all performers matched the
frozen package. Final local verification passed all seven inputs and the data
image again. The private project, recording package, disc metadata and readback
evidence remain in Git-ignored local storage; their contents and hashes are
not included in this record. Consumer-player listening and fresh physical
artwork inspection are separate from these digital verification results.

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
