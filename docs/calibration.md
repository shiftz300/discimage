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

Use a blank 120 mm CD-R or CD-RW and repeat the same command to continue:

```sh
discimage calibrate guide --output calibration
```

1. The guide creates the target, detects a drive, prepares previews and asks
   before recording. Review the target before confirming.
2. After recording, photograph the entire recording side straight on, including
   the disc rim and center hole. Save it as `calibration/photo.jpg` and rerun.
3. Open the generated local HTML page, mark four rim points, four hole points
   and six band edges along the same ray. Save the downloaded `points.json`
   into `calibration/` and rerun.
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
discimage burn target/target.toml --device 'DRIVE_ID' --output record --dry-run
discimage resume record --device 'DRIVE_ID'
discimage calibrate mark record/actual-1 photo.jpg --output points.html
# Open points.html and save points.json.
discimage calibrate fit record/actual-1 points.json --output calibration.toml
```

For offline authoring use `build target/target.toml --output bundle
--session-start LBA`, with the actual recording start. The current CLI has no
standalone physical-disc readback command; `readback` renders a local ISO.

## Reusing CD-RW

Preserve the last job, photos and measurements first. The following macOS
commands **erase the selected disc completely**. Identify the correct drive
with `drutil list`; replace `1` as needed. Wait for completion, then confirm
`blank: true` using `discimage devices --json` before starting a new job.

```sh
drutil list
drutil -drive 1 erase full
discimage devices --json
```

## Saved angular settings

[examples/sw80eu1.toml](../examples/sw80eu1.toml) records the visually accepted
Verbatim SW80EU1 / LG GX40N RQ00 combination, with requested speed 10×. Merge
its keys into the album's `[disc]` table, keeping capacity and both radii
together. The `velocity` value describes the modeled spiral at 1×; it is not
`--speed` and does not command the spindle. `angle` sets model orientation;
it does not align the picture to the printed label.

This setup reduced a measured inner-to-outer drift from about 14° to about
0.55° at one visible reference pair. The latter is a photo-based observation,
not a whole-disc accuracy bound. The preset is not a radial least-squares fit
and supplies no `[disc.calibration]` RMS. An existing radial profile takes
precedence over the preset's radii and capacity reference. Keep the matched
settings together and validate any combination before using it as a new preset.

`[disc.phase_target]` remains available for diagnostic pulse bands. Its
`controls` are solid intervals of absolute LBAs; each band specifies `start`,
`check`, `end` and `velocity`. The holdout after `check` shifts pulse phase by
0.24 turns. It is an experimental diagnostic, not an automatic angular fitter.
See [validation](validation.md) for the distinction between digital readback
and physical appearance.
