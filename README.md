# discimage

> [!WARNING]
> **READ BEFORE RECORDING.**
> - **CONFIRM THE DRIVE AND DISC BEFORE WRITING.**
> - **CD-R CANNOT BE ERASED. ERASING CD-RW DESTROYS ITS CONTENTS.**
> - **DO NOT RUN AS ROOT OR DISCONNECT THE DRIVE DURING WRITING. IF WRITING FAILS, KEEP THE JOB AND CHECK ITS LOG BEFORE RETRYING.**

[简体中文](docs/zhcn.md) · [Guides](docs/README.md)

Make a music CD with artwork in the outer ring. Design and build on macOS,
Windows or Linux. Record on macOS; Windows recording is experimental.

## Install

Download your system's archive from [Releases](https://github.com/shiftz300/discimage/releases/latest)
and verify it against `SHA256SUMS.txt`. No Rust or Python installation is needed.

| System | Archive suffix |
| --- | --- |
| macOS Apple Silicon / Intel | `macos-arm64.tar.gz` / `macos-x64.tar.gz` |
| Linux x86-64 | `linux-x64.tar.gz` |
| Windows x86-64 | `windows-x64.zip` |

In the extracted directory, on macOS/Linux:

```sh
mkdir -p "$HOME/.local/bin"
cp -p ./discimage "$HOME/.local/bin/discimage"
export PATH="$HOME/.local/bin:$PATH"
discimage --version
```

On Windows, in PowerShell:

```powershell
$env:Path = "$((Get-Location).Path);$env:Path"
discimage --version
```

These PATH changes apply to the current terminal. macOS packages are not notarized;
if blocked, follow [Apple's instructions](https://support.apple.com/en-us/102445) for a verified download.

## 1. Choose a media preset

Use a matching [tested preset](calibrations/README.md), or calibrate your own batch:

```sh
discimage devices
discimage select DRIVE_ID
discimage select
discimage calibrate -o calibrations/my-batch
```

Replace `DRIVE_ID` with the hardware ID shown by `devices`. Calibration writes one
blank CD-R/CD-RW target, reads it twice and exports `disc.toml`. Continue with the
same command if interrupted. [Calibration and optional photo refinement](docs/calibration.md).

## 2. Prepare and design

Use one 44.1 kHz, 16-bit stereo PCM WAV per track, at least four seconds.
Save `album.toml` in your working directory; the array sets the playing order:

```toml
title = "My album"
preset = { file = "calibrations/my-batch/disc.toml", name = "my-batch" }
tracks = [{ path = "audio/01.wav" }, { path = "audio/02.wav" }]
```

Replace the preset with your own path/name, then export the album's template:

```sh
discimage preview album.toml
discimage template album.toml -o templates
```

Import `templates/layout.json` into the [artwork editor](https://discimage.kasetry.cn/).
Design, export the artwork ZIP, put `artwork.png` beside `album.toml`, and add the
exported `artwork.toml` section to your album. Keep the tracks and preset.
[Images, text and previews](docs/preview.md).

## 3. Build and burn

```sh
discimage build album.toml
discimage verify output/album
# Open output/album/preview.svg in a browser and check the design.
discimage devices
discimage select DRIVE_ID
discimage select
# Insert a matching blank disc, then write:
discimage burn output/album
```

Use a new blank CD-R, or [explicitly erase a used CD-RW](docs/recording.md#reusing-cd-rw).
The calibration target is already written. Speed is automatic unless you supply `--speed N`.
`burn` writes and checks audio, reads the actual NWA, then encodes, writes,
checks and closes the data session. Afterwards `devices` should show two sessions;
inspect the artwork and audition the music. [Recording and recovery](docs/recording.md).

## More

- [Manual photo refinement](docs/calibration.md#3-optional-photo-refinement) · [Copyable AI prompt](docs/calibration.md#copyable-ai-prompt)
- [ISO/RAW export and local verification](docs/recording.md#image-export-local-checks-and-cd-text)
- [Windows notes](docs/windows.md) · [Hardware results](docs/validation.md) · [Files](docs/files.md)

Current zsh completion: `autoload -Uz compinit && compinit`, then
`source <(discimage completions zsh)`. Also supports bash, fish, PowerShell and elvish.
Run `discimage COMMAND --help` for options. Logs support macOS Terminal colors and `NO_COLOR=1`.

CD-Text currently supports printable ASCII. Full Blue Book CD Extra/XA compliance
and consumer-player compatibility remain unverified.

Inspired by [DiscImageStudio](https://github.com/JiaFeiMiao-K-Cat/DiscImageStudio)
by JiaFeiMiao-K-Cat and contributors, including its CDImage-derived CIRC mapping.
[Credits](NOTICE.md) · [GPL-3.0-or-later](LICENSE).
