# Calibrate a media batch

[简体中文](zhcn/calibration.md) · [Guides](README.md)

Use one blank CD-R or CD-RW from the batch. A CD-R target consumes that disc;
CD-RW can be erased after its evidence is saved. Different media/batches need
separate presets. Recording/measurement runs on macOS and Windows; photo fitting is offline.

## 1. Run calibration

```sh
discimage devices
discimage select DRIVE_ID
discimage select
discimage calibrate -o calibrations/my-batch
```

Replace `DRIVE_ID` with the displayed hardware ID. Confirm the reserved drive and
blank disc, then enter `y`. A used CD-RW needs [explicit erasure](recording.md#reusing-cd-rw) first.
Speed is automatic unless specified.

The guide writes and verifies one single-session target, takes two independent
read-only rounds, checks uncertainty/repeatability and exports `disc.toml`.
Wait for `Preset saved to .../disc.toml`. The tested GX40N took about 15 minutes
for two fresh rounds; other drives vary. RMS is diagnostic, not a fixed pass line.

Rerun the same command to continue. Completed targets are not rewritten.
Uncertain writes stop. Disagreement preserves the previous preset and reports.
Use `--remeasure` only to request one additional read-only round.

## 2. Use the preset

At the top of `projects/my-album/cdrw.toml`, before any table:

```toml
preset = { file = "../../calibrations/my-batch/disc.toml", name = "my-batch" }
```

Paths are relative to the album TOML. The name defaults to the batch directory
name. Use `preset` instead of a `[disc]` block. Rebuild templates and packages
after changing calibration. Check the first physical artwork before broad use;
[repeatability alone does not establish manual-preset equivalence](validation.md).

## 3. Optional photo refinement

Timing already estimates the spiral's angular mapping. A three-band photo adds
physical radius scale. Keep the same target and both completed timing rounds.

1. Photograph the complete 120 mm disc straight on, including the physical rim
   and actual center hole. Avoid glare over the band edges. Save `photo.jpg`
   in the batch directory; JPEG/PNG, longest side ≤8192 pixels, disc diameter ≥360 pixels.
2. Open the editor's **标定取点** tab, import this batch's `mark.json` and photo.
   Follow the 14 prompts:

   | Points | Order |
   | --- | --- |
   | 1–4 | Physical rim: top, right, bottom, left |
   | 5–8 | Actual hole edge: top, right, bottom, left |
   | 9–14 | Bands from inside outward: band 1 inner/outer, band 2 inner/outer, band 3 inner/outer |

   Take the last six points along **one ray**. Zoom to see boundaries; undo mistaken clicks.
3. Export `points.json` into the batch directory and rerun the same `calibrate` command.
   With the target files and both reports present, no drive, rewrite or remeasurement is needed.

Success updates `disc.toml`. Rim/hole failures mean retake or correct the photo;
edge-order/residual failures mean review points and target identity. The fitter
does not rectify camera tilt or lens distortion. It scales radii and velocity
together, preserving timing angles. Photo/model mismatch above 0.3 mm RMS rejects
that update; this radial check is distinct from the timing RMS diagnostic.

### Copyable AI prompt

<details>
<summary>Use AI to assist point selection</summary>

Attach the photo and `mark.json`. Replace WIDTH/HEIGHT with that image's pixel dimensions.

```text
Measure this DiscImage three-band CD target photo, WIDTH by HEIGHT pixels.
Use the existing photo, with x rightward, y downward, origin top-left.
Locate rim and actual hole edges at top/right/bottom/left (four points each).
Locate six band edges on one clear ray, inside to outside: band 1 inner/outer,
band 2 inner/outer, band 3 inner/outer. Do not use label rings or reflections.
Return points.json containing only bundle_sha256 (copied exactly from mark.json),
width, height, rim (4 [x,y] pairs), hole (4 pairs), edges (6 pairs).
Provide a numbered overlay for review without changing the photo.
Put uncertainty notes outside JSON. Do not invent obscured edges or hashes.
If viewing a resized/cropped image, state the coordinate transform first;
if it is unknown, do not claim original-pixel coordinates.
```

Review the points visually, save the file in the batch directory and rerun calibration.
Alternatively use the suggestions to click manually in the website. Display
zoom is mapped by the website; AI thumbnail coordinates need their own transform.
There is no built-in AI service. The preset is TOML; JSON is measurement exchange.

</details>

## Remaining angular drift

Concentric rings cannot identify angle phase. Known lettering or other angular
features can support a separate [photo phase fit](timing.md#photo-phase-refinement).
This exports a candidate and does not overwrite the guided preset automatically.
Absolute orientation relative to the printed label remains unmeasured.

## Keep these files

`disc.toml` is the album preset. Keep `record/`, `timing/`, the independent
`timing-previous-NNN/` report, and optional `photo.jpg`/`points.json` for refinement.
`mark.json` identifies the target in the point picker. [Directory layout](files.md).
