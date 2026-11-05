# discimage

[简体中文](docs/zhcn.md)

A Rust CLI for music CDs with visible artwork in the remaining outer ring.
Version **0.3.1**. Offline authoring, templates and previews work on macOS,
Windows and Linux. Drive recording is currently available on **macOS**.

Inspired by **[DiscImageStudio](https://github.com/JiaFeiMiao-K-Cat/DiscImageStudio)**
by JiaFeiMiao-K-Cat and contributors, including its CDImage-derived CIRC mapping.
Source revisions and adaptations are credited in [NOTICE.md](NOTICE.md).
Licensed under [GPL-3.0-or-later](LICENSE).

## Install

With current stable Rust, run from this repository:

```sh
cargo install --path . --locked
discimage --version
```

Prepare one **44.1 kHz, 16-bit, stereo PCM WAV** per track (at least four seconds),
plus a PNG or JPEG. Convert other audio with XLD or FFmpeg, for example:

```sh
ffmpeg -i song.flac -ar 44100 -ac 2 -c:a pcm_s16le 01.wav
```

## Preview an album

Create `album.toml` beside your files. Track order follows the array; paths are
relative to this TOML file.

```toml
title = "My album"
tracks = [{ path = "01.wav" }, { path = "02.wav" }]
# files = "extras"                   # optional companion directory

[artwork]
path = "artwork.png"
mode = "strip"                       # strip: wrap a rectangle; disc: centered disc image
tone = "threshold"                   # threshold / gray / ordered
threshold = 128
rotation = 0.0                       # clockwise degrees
```

```sh
discimage plan album.toml
discimage template album.toml --output templates
discimage preview album.toml --output preview.svg
```

Open `preview.svg` in a browser. Templates include transparent PNG and editable
SVG canvases sized from the album length. `template` works before the artwork
file exists. In `strip` mode, left to right wraps clockwise, with the seam at
12 o'clock and the top edge at the outer radius. In `disc` mode, the picture is
centered on the disc and clipped to the available annulus; `fit = "cover"` crops
to a square, while the default `contain` keeps the whole image.

Build and inspect a complete offline bundle:

```sh
discimage build album.toml --output album
discimage verify album
discimage readback album --output detailed.png --samples 128 --size 1600
```

`album/` contains music, ISO/RAW data, `preview.svg`, `design.png`,
`readback.png`, `difference.png` and a checksummed manifest. `verify` and
`readback` inspect **local files**, not the physical disc. Output files and
bundle directories must be new; allow several GB of free space for a burn job.

## Record on macOS

Insert a blank CD-R or CD-RW. Run in Terminal to see stage, progress and speed.
Replace `DRIVE_ID` with the exact ID from `devices`.

```sh
discimage devices --json
discimage burn album.toml --device 'DRIVE_ID' --output record --dry-run
# Review record/planned/preview.svg, then write the prepared job:
discimage resume record --device 'DRIVE_ID'
```

The workflow writes music, reads it back against the WAVs, obtains the actual
next writable address (NWA), rebuilds the data session, then writes, verifies
and finalizes it. `record/journal.json` records progress and errors. A successful
job ends at `complete` with two sessions. Use `devices --json` to check that the
disc has two sessions, is no longer appendable, and contains the audio tracks
followed by one data track. Inspect the disc separately for artwork quality.

`--speed 10` on `burn` requests 10×; the default is 8×. Choose a speed supported
by your drive and media. Used CD-RW media need a separate erase; uncertain
writes are never automatically retried. See [recording and recovery](docs/recording.md).

## Calibration and limits

The CLI provides a [guided radial calibration](docs/calibration.md). Angular
alignment also depends on the spiral model. The tested
[SW80EU1 / GX40N settings](examples/sw80eu1.toml) are an example for that setup,
not a universal preset. Templates, previews and recording share the same model.

This is a music-first, two-session Mode 1 layout. Full Blue Book CD Extra/XA
compliance and consumer-player compatibility remain unverified. A bundle's
session-offset ISO and separate CUE/TOC files cannot be treated as a generic
single-image disc. See [hardware results](docs/validation.md).

[All settings](examples/album.toml) · [Previews](docs/preview.md) ·
[Architecture](docs/architecture.md)

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```
