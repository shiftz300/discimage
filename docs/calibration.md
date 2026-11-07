# Calibration

The offline tools work on macOS, Windows and Linux. The guided recorder uses
macOS. Interactive prompts currently use Chinese; individual subcommands use
English. Calibration has two separate parts:

- **Radial:** three solid concentric bands locate known LBAs in millimeters.
  These bands are expected to look like rings; they contain no angular dots.
- **Angular:** `disc.velocity` and `disc.angle` control the spiral mapping.
  A radial fit alone cannot straighten bent lettering or guarantee contrast.

For a readable disc with visible artwork, try the experimental
[read-timing/photo comparison](timing.md) before recording another target.
It reuses existing evidence and does not change the saved preset.

## Guided radial fit

On macOS, a single-session Mode 1 target can serve both the radial photo fit
and the read-timing probe. With a blank CD-R inserted:

```sh
mkdir -p calibrations/work
discimage calibrate record --output calibrations/work/record
discimage calibrate guide --output calibrations/work --bundle calibrations/work/record
```

The record command selects the only eligible drive, reserves the disc and
builds the target from its reported free capacity. If several blank CD-R
drives qualify, use `discimage devices` and pass an explicit `--device`.
The target is written at the fastest speed the drive permits unless
`--speed` is supplied. A lost reservation or replaced disc stops the job
before the write. The destination must be new.
Use `--dry-run` to check selection and reservation without creating files or writing.

The guide can also author and record the legacy two-session radial target.

Use a blank 120 mm CD-R or CD-RW and repeat the same command to continue:

```sh
discimage calibrate guide --output calibration
```

1. The guide creates the target, detects a drive, prepares previews and asks
   before recording. Review the target before confirming.
2. After recording, photograph the entire recording side straight on, including
   the disc rim and center hole. Save it as `calibration/photo.jpg` and rerun.
3. Open **标定取点** in [discimage-website](https://discimage.kasetry.cn/).
   Import the target JSON and photo. Mark four rim points, four hole points
   and six band edges along the same ray. Save `points.json` in the working
   directory and rerun the guide.
4. Copy the generated `[disc.calibration]` section into `album.toml`. Rebuild
   the template, preview and recording job in new directories.

Use `--photo PATH` and `--points PATH` to select files explicitly. With an
externally recorded target, `--bundle PATH` skips the recorder and works offline.
Use the exact bundle written to the disc, normally `record/actual-1/`; its hash
binds the photo measurements to the target. `planned/` is only a prediction.

The fit reports six residuals and radial RMS. It rejects excessive perspective,
invalid points and mismatched bundles. It does not fit angular phase, optical
contrast or a general perspective correction. If band edges are not visible,
do not guess points or repeatedly reburn the same unmeasurable target.

## Individual steps

```sh
discimage calibrate target --output target
discimage preview target/target.toml --output target.svg
discimage burn target/target.toml --device GX40N --output record --dry-run
discimage burn record
discimage calibrate mark record/actual-1 --output mark.json
# Import mark.json and photo.jpg in the website's photo point picker.
discimage calibrate fit record/actual-1 points.json --output calibration.toml
```

For offline authoring use `build target/target.toml --output bundle
--session-start LBA`, with the actual recording start. The current CLI has no
standalone physical-disc readback command; `verify -o` renders a local ISO.

## Reusing CD-RW

Preserve the last job, photos and measurements first. `discimage erase` selects
and confirms the CD-RW, completely erases it, and verifies the blank media state.
If more than one drive qualifies, pass a short `--device` selector from `discimage devices`.

```sh
discimage erase --dry-run
discimage erase
discimage devices
```

## Saved angular settings

The [preset catalog](../calibrations/README.md) lists the tested media and
drive combinations. Use `sw80eu1` for the visually checked CD‑RW profile.
Its `velocity` models the spiral at 1×. Select the drive and write speed when
starting the recording. The `angle` sets artwork orientation in the model.

On the tested GX40N at requested 10×, the correction reduced observed
inner-to-outer drift from about 14° to about 0.55° at one visible reference pair.
This is a local measurement. The separate AZO Gold Vinyl photo fit is kept
with the catalog's measurements and is available for comparison.

`[disc.phase_target]` remains available for diagnostic pulse bands. Its
`controls` are solid intervals of absolute LBAs; each band specifies `start`,
`check`, `end` and `velocity`. The holdout after `check` shifts pulse phase by
0.24 turns. It is an experimental diagnostic, not an automatic angular fitter.
See [validation](validation.md) for the distinction between digital readback
and physical appearance.
