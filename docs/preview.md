# Artwork and previews

[简体中文](zhcn/preview.md) · [Guides](README.md)

## 1. Export the album template

```sh
discimage template album.toml -o templates
```

The actual WAV lengths and media preset determine the drawable annulus.
The artwork file need not exist yet. The destination must not already exist.
Import `templates/layout.json` into the [editor](https://discimage.kasetry.cn/).

## 2. Choose the image mapping

| Mode | Mapping |
| --- | --- |
| `disc` | Square canvas across the recording diameter; protected areas are masked |
| `strip` | One clockwise revolution; top is outer radius, bottom is inner radius, seam starts at 12 o'clock |

Input is PNG/JPEG, up to 8192 pixels per side. `rotation` is clockwise.
`fit = "contain"` or `"cover"` applies to disc mode; strip mode uses the whole image.
Resolve strip seams in the design. Changing album length or media requires a new
template and another composition check.

Export the website ZIP. Put **`artwork.png`** beside the album TOML and copy its
`artwork.toml` section into the album, replacing the existing `[artwork]` section.
Keep the track list and preset. `preview.png` is a reference image, not the input.
Save `discimage-design.json` if you want to edit the website design again.

## 3. Inspect before writing

```sh
discimage preview album.toml
discimage preview album.toml -o design.png --size 4096
discimage build album.toml
```

The first command shows track timing and artwork space. The PNG destination
must be new. Open the package's `preview.svg` to inspect layout and composition.
Preview output supports 256–4096 pixels. Increasing it does not increase the
physical recording density.

| Tone | Effect |
| --- | --- |
| `gray` | Grayscale design, deterministic halftoning during encoding |
| `threshold` | Binary edges, adjustable cutoff; useful for crisp signatures |
| `ordered` | Ordered design dithering and deterministic physical sampling |

Use a sharp source rather than a scaled screenshot. Tone settings do not predict
physical color or contrast.

## Optional encoded-payload view

```sh
discimage build album.toml --iso
discimage verify output/album -o encoded.png --size 1600 --samples 128
```

`verify -o` projects controllable drawing data from the local ISO/RAW. It does
not read the physical disc. Sampling density is 1–2048 bytes per sector; higher
values take longer. Transparent gaps can be unsampled areas. This view does not
simulate all parity/channel symbols, reflectivity or drive write strategy.
