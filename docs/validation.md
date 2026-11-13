# Validation

[Guides](README.md) · [Calibration criteria](timing.md#uncertainty-and-repeatability)

The shared MMC writer and track-tail mode have software regression coverage.
On macOS, the GX40N wrote and finalized a six-track CD-RW with track-tail
silence. All audio and Mode 1 sectors matched on readback, and the Q indexes
matched before and after the data append. Windows hardware validation of the
shared writer is pending.

## Software checks

| Check | Result |
| --- | --- |
| Test suite | 110 passed on macOS: 57 library, 7 CLI, 8 calibration, 13 offline, 25 recording |
| Clippy, warnings denied | Passed |
| Release build | Passed |
| 2,400 synthetic nonlinear spiral fits | Confidence-band checks passed across heterogeneous and underestimated-noise cases |
| 40,000 correlated Gaussian comparisons | Nominal 1% false rejection measured at 0.925% |
| 20,000 four-round comparisons | Family false rejection 0.900% with multiple-comparison correction |

Tests cover Mode 1 framing, CIRC position conventions, image mapping, frozen
inputs, actual-NWA rebuilds, readback corruption, uncertain writes, target
binding and invalid calibration data. Windows high-noise regression reports
with RMS 1.770°/0.7325° pass the shared assessment when their measured uncertainty
supports it. That regression was run on macOS, not on Windows hardware.

Reproduce software checks:

```sh
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo build --release --locked
```

## Current macOS calibration run

Apple SuperDrive/GX40N read the verified single-session CD-RW target in two
fresh independent rounds, 36 points each, over target LBA 2–359692.
The run took about 15 minutes and issued no erase or disc writes.

| Metric | Round 1 | Round 2 |
| --- | --- | --- |
| Residual RMS, diagnostic | 0.3424° | 0.3008° |
| Reduced chi-square | 3.050 | 2.466 |
| Data-only measured-span sigma | 0.2110° | 0.1891° |
| Maximum nominal 99% angle band | ±0.7849° | ±0.7065° |
| Median completion scatter | 38.7 us | 38.9 us |

The pair statistic was 0.0413 against 9.2103. Curve difference was 0.0202° RMS,
0.0647° maximum. The guide selected round 2, exported the preset, and exited
successfully. Receipt hashes matched; the preset loaded through album preview;
the drive was released with its original single-session layout.

## Automatic versus accepted manual preset

This is a calculation between models, **not a new physical accuracy measurement**.
The interval is encoder sectors 151196–355999, end exclusive, sampled at 20,001
positions. Each comparison aligns the two models at the interval midpoint,
removing an arbitrary overall rotation without wrapping the angle differences.

| Reference profile | Maximum angular difference | Difference in inner-to-outer angular span |
| --- | --- | --- |
| Visually accepted `sw80eu1` | 1.134° | −0.410° |
| Photo-phase candidate `sw80eu1-timing` | 0.595° | −0.509° |

Across the full automatic measured range, the difference from `sw80eu1` reaches
18.391° near the inner region. The manual profile was visually checked in an
outer artwork region, so that difference does not establish which model is more
accurate there. Repeated timing agreement alone does not establish equivalence
to the accepted manual preset.

Raw radius differences over the outer interval are about 0.561–0.769 mm. The
automatic preset assumes a 58 mm outer radius; the manual preset uses 57.227 mm.
Uniformly rescaling the automatic model to that reference outer radius reduces
radial differences below 0.0001 mm while leaving its angular law unchanged.
This diagnoses a scale convention; it is not a measurement of the correct radius.
The subsequent automatic-preset trial below provides outer-artwork acceptance;
this calculation alone does not validate the entire disc or measure the radius scale.

## Automatic-preset artwork trial

On 2026-11-10, the user-authorized full CD-RW erase was followed by a four-track
album trial using the fresh automatic preset. All four PCM tracks matched on
readback. The data session started at actual LBA 141196, contained 218497 sectors,
and passed verification. The final disc had two closed sessions and five tracks.
The user confirmed accurate artwork alignment. The accepted automatic parameters
are saved as `sw80eu1-auto` in the [media catalog](../calibrations/README.md);
`sw80eu1` and `sw80eu1-timing` remain intact. Album inputs and generated media
stay local. The assumed 58 mm scale and unmeasured label orientation are retained
as explicit assumptions, not relabeled as photo measurements.

## Hardware scope

| Case | Established | Still limited |
| --- | --- | --- |
| macOS GX40N, CD-RW | Target write/readback, fresh automatic calibration, audio/data recording and finalization | Automatic trial artwork user-confirmed; full-disc absolute accuracy unmeasured |
| macOS tested CD-R/CD-RW presets | Catalogued artwork checks and digital readback | Applies to the recorded combinations in the [preset catalog](../calibrations/README.md) |
| Windows GX40N | Recorded erase, single-session target and audio/data readback results | Current automatic rule still needs a fresh Windows hardware run |
| Windows Pioneer XD05 | Discovery and device access | Successful write/erase workflow remains unresolved |
| Linux | Offline authoring | No native recording backend |

Digital integrity, photographed alignment and consumer-player behavior are
separate checks. Full Blue Book CD Extra/XA conformance, complete physical EFM
simulation and reliable prediction of reflectivity or artwork color remain
unverified. Sources and licensing are recorded in [NOTICE](../NOTICE.md).
