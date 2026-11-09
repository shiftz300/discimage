# Read timing and photo phase (experimental)

Read timing estimates a CD spiral from a recorded data track. The probe runs
inside DiscImage on macOS and Windows. Phase comparison and candidate import
run offline on macOS, Windows and Linux. A drive must deliver real media
reads at a stable rotation rate. Caching, changing speed and USB latency
can make a drive unsuitable.

## Measure and compare

Use a finalized, single-session, single-track Mode 1 CD. Run `discimage devices`
and use the same drive ID or unique model as other commands, on macOS
or Windows. After `discimage select ID`, the `--device` flag is optional;
without a default, exactly one connected optical drive is required:

```sh
discimage devices
discimage calibrate probe --device GX40N --check
discimage calibrate probe --device GX40N --output measurement
```

When several drives share a model, replace `GX40N` with the exact selector shown by `devices`
(the 12-character hardware ID, or `local-xxxxxxxxxx` without a reliable drive serial). Legacy macOS whole-disc names (`diskN`) and Windows drive letters
(`E:`) remain accepted. If macOS exclusive access fails, identify and unmount
only that optical volume. Windows uses an exclusive IMAPI2 session and prevents
media removal during measurement; access failures do not change permissions.
It timestamps synchronous native commands with QueryPerformanceCounter and
releases the reservation and tray prevention on exit, including errors.

`--check` reports the recorded range using only metadata commands. Measurement
defaults to 3–85% of that range, matching DiscProbe. Supply both `--first` and
`--end` (exclusive) to choose another range. The probe rejects audio, mixed,
multisession and unfinalized media. Existing CD Extra experiments can still be
imported from saved reports.

Both platforms use the same conservative 1411 kB/s read-speed request and
32 reference/target pairs per point. The experimental fast/slow modes and
configurable sampling options have been removed. The probe records the read-speed
policy in the receipt and tests 40 ordinary reads and
40 FUA reads at the middle of the track. If ordinary reads look cached and
FUA does not help, it tries a zero-length READ(12) with FUA before each read.
The last fallback reads 256 sectors elsewhere in the confirmed track in
chunks of up to 32 sectors before each timed read. Indirect methods discard
timed reads under 3 ms. On slow USB bridges, FUA reads at least three times slower
than ordinary reads also identify ordinary reads as cached. READ(10) fallback
is limited to refused READ(12) commands; medium and hardware errors stop the run.
An inner/outer rotation check rejects clear CLV behavior before the geometry
ladder. Settled sampling jitter spans at least one spindle revolution. Phase
estimation subtracts the local drift
interpolated from adjacent reference reads. The probe issues no writes to
the disc. Ctrl+C stops before the next command. Windows scopes DiscProbe-style
process/thread scheduling priorities to acquisition and restores them on exit.

Keep `measurement/summary.json`, `measurement/targets.csv` and
`measurement/status.json` together. An interrupted acquisition has
`completed=false`. The receipt records the chosen cache strategy as soon as
the cache test completes. Compatible reports from earlier probes still work.

Use phase points extracted from the photo and the exact bundle used to
record the photographed artwork:

```sh
discimage calibrate phase --timing measurement/summary.json \
  --bundle record --points points.json \
  --evidence photo.jpg --output comparison

discimage calibrate import album.toml \
  --timing measurement/summary.json --phase comparison/report.json \
  --bundle record --output candidate
```

Import the `candidate/` folder in **时序对比** on
[discimage-website](https://discimage.kasetry.cn/). The tab shows the four
comparison images and the drift curve from `report.json`. Export `candidate.toml`
to use with `preview` and `build`, then record the resulting package with `burn`. It references the
original assets by absolute path; it is not a portable bundle. No preset is
automatically applied. Use a new output directory on each run.

The original project must use the same geometry as the photographed bundle.
The importer verifies report linkage, reads actual `targets.csv` coverage, and
rounds inward after translating read-end coordinates to encoding coordinates.
It limits the candidate's outer extent and rejects a drawing start outside that
range, including a two-sector CIRC guard. Readable capacity and requested ranges
are never treated as measured coverage. The same checks run after an actual NWA
rebuild. Original projects without timing metadata retain their prior behavior.

Current candidate screening requires at least eight distinct targets plus a
reference, verified FUA or uncached plain reads, period spread ≤0.5%, timing RMS
≤0.3°, span sigma ≤0.5°, and at least two photo curves with validation RMS ≤0.3°
and maximum error ≤1°. Flush and eviction runs can be analyzed for phase but
do not qualify for candidate import. These are conservative software gates, **not universal
accuracy guarantees**. Interrupted experimental runs with `completed=false`
are rejected. The hashes identify evidence; they are not independent proof that
the photo extraction or physical assumptions were correct.

### Conversion without a second encoder

Let `B=57624`, `F=588`, `K=111−D`, and `b=velocity/(75 B)` in mm per channel bit.
For a reference extent of `N` sectors, the ordinary constant-pitch parameters are:

```text
inner² = b² (u² + v K F / π)
outer² = inner² + b² v B N / π
```

The global angle is aligned with the old model at the center of the measured
interval. The project's velocity is retained only as a length-scale assumption;
read timing measures the dimensionless `u/v` geometry, not absolute millimeters.
Any old radial-fit residual is removed rather than relabeled as a timing residual.
The existing `Spiral`, image sampler, CIRC mapping and encoder then use the candidate
unchanged. The predicted candidate view is regular by construction, not an
independent success test. Strip images are refitted to each plan's artwork annulus.

## What has been checked

On 2026-11-06, the existing two-session disc was tested only within its Mode 1
track, preserving absolute LBAs. There were 3,716 successful reads and no write
or erase commands. Two timing rounds agreed within 0.106°. A constant-pitch
fit used 36 locations; ten earlier measurements at five other locations gave
0.129° RMS and 0.209° maximum disagreement, without refitting their offset.
These measurements check timing consistency, not whole-disc visual accuracy.

The experiment combined the read-only macOS transport from
[discprobe-macos](https://github.com/NaiveTomcat/discprobe-macos/tree/61ea3498dd6139bc92328bb3c1b9afae099f5394)
and the CD solver from
[disc-probe](https://github.com/Ashley-QwQ/disc-probe/tree/934bb784202dfadc420aa39355a0f31ac08e7614).
Both are GPL-3.0-or-later. This experiment predates the built-in probe. A mixed audio/data disc needs data-track bounds;
do not use a whole-disc data-read sweep across its audio and session gaps.

## Compare an existing photograph offline

`discimage calibrate phase` compares the timing summary, the exact written
bundle and phase points already extracted from a photo. It works offline
and does not open a drive or modify project settings.

Open `comparison/index.html` in a browser. `report.json` includes input hashes,
individual fits, a shared fit and leave-one-curve-out checks. The output
directory must be new. `--evidence` verifies the supplied photo or rectified
array against the recorded hash; omit it only when that evidence is unavailable.
It does not verify all image/audio assets in the bundle: use `discimage verify`
for that separate check.

The timing summary must use the disc-probe CD `u`/`v` convention, 57,624 channel
bits per sector and the assumed 108-frame read tail. Keep `targets.csv` next to
`summary.json`: the importer derives coverage from accepted targets and the fit
reference. It accepts both the Mac branch's `readable_lba_range` and the earlier
adapter's `experiment_range` as bounds, not as proof of coverage. An inferred
read-completion location is not a measured hardware delay.

If a separate fiducial measurement supplied the forward offset `D`, add
`--offset-frames D --offset-evidence measurement.json`. The comparison records that
file's hash and checks the supplied offset against the photo curves without
refitting it. This does not make shared-photo observations independent. Omitting
these flags fits the offset from the provided curves.

The point file has this structure (at least 12 samples per curve):

```json
{
  "format": "discimage-phase-v1",
  "bundle_sha256": "SHA256 of the exact manifest.toml (legacy: manifest.json)",
  "evidence_sha256": "SHA256 of the photograph or rectified array",
  "evidence_kind": "photo",
  "convention": "photo-minus-design-clockwise",
  "samples": [
    {"curve": "edge-a", "sector": 180000.5, "phase_deg": 12.8}
  ]
}
```

`sector` is the fractional **encoding coordinate** (`Codec::physical / 2352`),
not a session-relative file offset. For a known generating-model radius `r`,
it is `(r² − inner²) / (outer² − inner²) × reference_sectors`. A radius measured
in a photo cannot be substituted directly: first establish its correspondence
to the generating image, for example using known radial features. The script
does not rectify photographs or extract these correspondences for you.

`phase_deg` is the continuously unwrapped photo angle minus its corresponding
design angle, clockwise in the same disc view. Follow the same edge or matched
image feature within a curve. Do not jump between arms or independently reduce
each angle modulo 360°. An arbitrary overall rotation per curve is allowed.
Use multiple separated curves to test whether a shared shift explains them.

## Why the origin matters

discimage's inverse CIRC coordinates and a standard forward CIRC encoder differ
by **111 channel frames** at the frame level. `src/codec.rs` tests this against
the forward data-slot permutation for every payload byte at several LBAs.
Intra-frame symbol positions are not identical to uniformly spaced payload
bytes; this is a frame-origin comparison, not a complete EFM simulator.

The photo comparison evaluates `theta_timing(n + K × 588) − theta_generated(n)`
and fits one `K`, plus a separate constant rotation for each observed curve.
The upstream forward-coordinate offset is `D = 111 − K`. A residual `D` may
include the drive's read-completion origin and other modeling effects; it is
not an audio sample offset or a different burn speed. See
[CD along-track discussion #143](https://github.com/JiaFeiMiao-K-Cat/DiscImageStudio/discussions/143).

For the saved SW80EU1 photograph, one rectification gave `K ≈ 107.99` frames
(`D ≈ 3.01`), 0.065° joint RMS and 0.072°/0.087° held-out curve RMS. Only the
held-out curve's photo rotation was refitted. However, three rectification
choices with ±0.25 mm radial sampling perturbations gave `D ≈ −3.09…13.13`.
That is a sensitivity range, **not a statistical confidence interval**. The
original photo is no longer at its old path; these checks reused the retained
rectified arrays. Neither this fit nor the timing result replaces the visually
accepted preset. No new production configuration was exported or applied.

A subsequent paired-fiducial measurement on the same retained photo supplied
`D ≈ 2.31785` frames (`K ≈ 108.68215`). Its tested extraction settings gave
2.255–3.000 frames; this is sensitivity to those settings, not a confidence
interval. Checking that supplied offset against the two existing lettering
curves gave 0.0651° joint RMS. The importer generated a separate candidate and
limited encoding coverage to 122001–355999 (end exclusive). A full offline
bundle passed hash, Mode 1
EDC/ECC validation and payload projection. The accepted preset remains unchanged
and this candidate has not been physically recorded or visually validated.

## Reducing calibration burns

[Upstream issue #49](https://github.com/JiaFeiMiao-K-Cat/DiscImageStudio/issues/49)
describes a **DVD** coarse/fine workflow: use a whole curved boundary, retain
the exact generating parameters, and check the predicted curve between marked
points. Dense rings can carry calibration information; ring count alone cannot
determine the two radii or correction direction. Its two-disc target is
conditional, not a guarantee, and its DVD formulas cannot replace CD encoding.

For this CD setup, the useful workflow is:

1. Keep the current readable disc and its actual bundle. Use read timing for
   the initial model, with repeats and positions excluded from fitting.
2. Unify the encoding/read origins. Reuse visible image features on that disc
   for the remaining phase check; no new target is needed when they suffice.
3. Check multiple curves and sensitivity to photo rectification. If they
   disagree, improve the photograph or analysis before changing the disc.
4. Save a candidate separately only when supported. Its first actual artwork
   burn still provides the physical validation that a fitted report cannot.

The offline comparison and candidate import stop before automatic preset
application. A complete front-on photo of the same disc, including rim and
true center hole, would improve the remaining check without another burn.

```sh
cargo test --locked --lib phase::tests
cargo test --locked --lib probe::tests
cargo test --locked --lib codec::tests
```
