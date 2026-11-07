# Preview and graphical output

A CLI still needs inspectable graphics. Preview/export is a first-class part
of the offline pipeline. Source composition, encoded-payload projection and
optical behavior are different things and must keep distinct captions.

## Available views

| Output | Meaning |
| --- | --- |
| `preview.svg` | Self-contained report: vector layout on the left, artwork composition on the right |
| `design.png` | Actual fitted/rotated/tone-processed source clipped to the available annulus |
| `readback.png` | Drawing samples read from the generated ISO, descrambled and projected through the CIRC byte-position convention |
| `difference.png` | Orange grayscale difference between design and readback; transparent where either has no sample |

Music tracks alternate layout colors. Gaps, session transition, filesystem/files,
artwork and final margin have separate legend entries. Thin or zero-width zones
may be invisible at normal scale; the numeric sector budget remains authoritative.
Artwork views shade protected regions neutrally and leave the hole/outer space
transparent. Layout colors are annotations, not predicted disc colors.

## Commands

```sh
discimage template album.toml --output templates
discimage preview album.toml --output preview.svg
discimage preview album.toml --output disc.png --size 1600
discimage build album.toml --iso --output album
discimage verify album --output detail.png --samples 128 --size 1600
```

`preview PROJECT` prints track timing and artwork space. Add `--json` for the layout data, or `-o FILE` to export PNG/SVG. `build --iso` exports the selected data format and a single SVG preview; encoded PNG and difference views remain optional through the preview/verification APIs; `verify -o`
first verifies the bundle then reads its ISO. Output resolution is 256–4096,
default 1024. Input PNG/JPEG dimensions are limited to 8192 per side. SVG embeds
its raster image and has no external source links or executable content. Preview exports
refuse overwrite. There is no interactive GUI dependency.

## Shared composition and tone

`artwork.mode` defaults to `disc` for existing projects and manifests. In disc
mode the square canvas maps to the complete recording diameter, with a mask
protecting music, session transition, files and margin. Changing music duration
changes the annular window, not the image scale. `contain` uses white
letterboxing and `cover` crops to a square.

In `strip` mode the complete image width maps to one clockwise revolution,
starting at 12 o'clock; its top edge is the outer artwork radius and bottom edge
the inner artwork radius. The strip is not fitted into a square; `fit` has no
effect. Rotation is clockwise in both modes and moves the strip seam. Left/right
content discontinuities remain visible at the seam. Changing the available
annulus changes the radial stretch, so regenerate templates when layout changes.

Preview and physical encoding both call `Sampler` for the same coordinate
mapping. Physical placement still uses the plan's absolute LBA/spiral model.

Processing order is weighted sRGB-byte grayscale, gamma (`0.1–4`, exponent
`1/gamma`), brightness (`-255–255`), optional inversion and finally tone.
Transparent pixels composite over white. This is an artistic tone pipeline,
not radiometric calibration. Disc sources use a square canvas sized to the longest source side, bounded
to 1600–4096 pixels. Strips retain their original pixels, up to the 8192-pixel
decoder limit, without resizing. The same normalized grid is used by design
preview and authoring, retained in the package as `artwork.png`.

- **gray:** continuous in design; deterministic stochastic `0D/AA` halftoning
  at each absolute logical byte in authoring.
- **threshold:** hard cutoff at `threshold`, default 128.
- **ordered:** 4×4 Bayer pixels in design; a deterministic 16-phase sequence
  indexed by absolute logical byte in authoring. Threshold shifts that sequence.

Changing preview resolution changes pixel dithering. It does not change the
physical byte phase or authored ISO. Do not use a screen bitmap as a substitute
for physical-domain tone sampling.

## Album-specific templates

`template` reads the actual sector-aligned music lengths and uses the same plan
as preview/build. It works before the artwork file exists. It exports both
`disc.svg/png` and `strip.svg/png`, a `guide.svg` layout report, and `layout.json`
containing plan, dimensions, physical radii and mapping conventions.

Disc PNG is an opaque white paintable annulus on transparent protected areas.
Its canvas width maps to the recording diameter, not the full 120 mm disc.
Strip PNG is a blank white rectangle. Strip dimensions use the middle-radius
circumference divided by the radial thickness, with matched pixel density,
integer rounding and width capped at 8192. Extremely thin annuli can yield a
one-pixel-high strip; the exact physical dimensions remain in the JSON.

SVG templates have an editable `artwork` group and hidden `guides`. Disc artwork
is clipped to the annulus. Keep guides hidden and export PNG at the declared
canvas size before using it as encoder input; SVG is not an input image format.
The separate guide report is for reference and must not be burned as artwork.

Output directories must not exist; interrupted template exports keep an
`incomplete` marker. `--size` selects disc canvas size (default 1600, 256–4096),
with strip dimensions derived automatically. Template and preview accept
`--session-start`, matching build; use the same value for all operations.

## Readback fidelity

The reader samples the actual `ARTWORK.BIN` ISO extent, reverses the Mode 1
scramble mask, accepts only known palette bytes and projects the samples with
absolute LBA, pair swap and inverse CIRC position mapping. Samples landing in
one display pixel are averaged, not overwritten.

Default density is 32 payload bytes per sector. `--samples` accepts 1–2048;
higher density reduces sampling noise and holes at a higher runtime cost.
Unsampled drawing pixels remain transparent; they are not assumed white.
At a small synthetic sector count or large output resolution, increase density.
The difference map is a sampling/design comparison, not an optical error metric.

Readback does not model parity, all channel symbols, EFM merging, a drive's write
strategy, measured reflectivity, ATIP timing or complete finite-stream CIRC
latency. Mode 1 headers/parity and boundary contributions are not projected as
artwork. The file/RAW hashes and EDC/ECC checks are separate from this picture.

## Remaining fine previews

| View | Requirement |
| --- | --- |
| Extent labels/overlay | Exact metadata/file/allocated-drawing LBAs, not just a total reserved band |
| Spiral and local detail | Radius/angle/LBA labels, direction, zoom and crop; distinguish sparse sampling |
| Calibration comparison | Generation vs measured geometry, anchors, residuals, units and uncertainty |
| Independent channel comparison | Reference physical timing, byte order, finite tails and parity effects |
| Hardware report | Read TOC/sessions, data hashes, audio verification and photographed artwork separately |

Retain stable coordinates, exportable masks and source/reference views, explicit
calibration IDs and deterministic settings. A full physical encoder preview
requires independent timing evidence; a self-roundtrip cannot establish it.
