# discimage

> [!WARNING]
> **READ BEFORE RECORDING.**
> - **START WITH `prepare` AND CONFIRM THE DRIVE AND DISC.**
> - **`burn`, `resume` AND CALIBRATION RECORDING WRITE TO THE DISC. A WRITTEN CD-R CANNOT BE ERASED. ERASING A CD-RW DESTROYS ITS CONTENTS.**
> - **USE A SPEED SUPPORTED BY BOTH THE DRIVE AND DISC. DO NOT RUN AS ROOT OR UNPLUG THE DRIVE DURING A WRITE. IF A WRITE FAILS, CHECK THE JOB JOURNAL BEFORE RETRYING.**

[简体中文](docs/zhcn.md)

Make a music CD with artwork in the outer ring. Plan, design and build on macOS, Windows or Linux. Record directly on macOS.

## Install

Download the prebuilt archive for your system from [GitHub Releases](https://github.com/shiftz300/discimage/releases/latest).
Compare its SHA-256 with `SHA256SUMS.txt` before extracting. Use the executable inside the archive.

| System | Archive suffix |
| --- | --- |
| macOS, Apple Silicon | `macos-arm64.tar.gz` |
| macOS, Intel | `macos-x64.tar.gz` |
| Linux, x86-64 | `linux-x64.tar.gz` |
| Windows, x86-64 | `windows-x64.zip` |

On macOS or Linux, open a terminal in the extracted folder and run:

```sh
mkdir -p "$HOME/.local/bin"
cp -p ./discimage "$HOME/.local/bin/discimage"
export PATH="$HOME/.local/bin:$PATH"
discimage --version
```

On Windows, open PowerShell in the extracted folder and run:

```powershell
$env:Path = "$((Get-Location).Path);$env:Path"
discimage --version
```

These commands add the executable to the current terminal's PATH. The main workflow runs directly from the executable. The optional `calibrate probe` command requires an external DiscProbe checkout and Python. macOS packages are not notarized. If Gatekeeper blocks a verified download, follow [Apple's instructions for opening a trusted app](https://support.apple.com/en-us/102445).

## Plan and preview

Prepare one 44.1 kHz, 16-bit stereo PCM WAV per track (at least four seconds). Convert other formats with XLD or FFmpeg, for example `ffmpeg -i song.flac -ar 44100 -ac 2 -c:a pcm_s16le 01.wav`.

Save `album.toml` beside the WAVs. Paths are relative to this file, and the array sets the play order.

```toml
title = "My album"
tracks = [{ path = "01.wav" }, { path = "02.wav" }]

[artwork]
path = "artwork.png"
mode = "strip"
```

```sh
mkdir output
discimage plan album.toml
discimage template album.toml --output output/templates
discimage preview album.toml --output output/preview.svg
```

Create the output parent directory once and use a new destination for each export. `template` works before the artwork file exists. Open the SVG in a browser. In `strip` mode the image wraps clockwise from 12 o'clock, with its top edge at the outer rim. Use `mode = "disc"` for a centered image clipped to the available ring. Images can be PNG or JPEG. For a calibrated disc, add the matching `preset` to the album before planning. See [media presets](calibrations/README.md), [artwork options](docs/preview.md) and the [full album example](examples/album.toml).

## Record on macOS

Allow a few GB of free disk space, insert a blank CD-R or CD-RW, then run these commands in Terminal:

```sh
discimage devices
discimage prepare album.toml --output output/record --speed 8
open output/record/planned/preview.svg
# After checking the preview, start writing:
discimage resume output/record
```

`prepare` uses the only eligible blank disc, saves the drive and requested speed with the job, and builds a preview without writing to the disc. If several drives qualify, pass `--device 'DRIVE_ID'` with the exact ID from `devices`. Choose a speed supported by the drive and disc. A previously written CD-RW needs a separate erase.

`resume` writes the audio session, compares the recorded PCM against the prepared WAVs, then uses the drive's measured next writable address to build, write and verify the data session. Check `output/record/journal.json` for stage `complete` and run `discimage devices` again to confirm the disc is closed and reports two sessions. Inspect the disc surface for artwork quality. For interruptions, CD-Text and direct `burn`, see [recording and recovery](docs/recording.md).

Logs show the active stage and, during drive writes, reported progress, speed and elapsed time. The macOS Terminal uses basic ANSI colors. Set `NO_COLOR=1` for plain logs. Redirected logs are plain text, and `plan --json` and `devices --json` write JSON to stdout.

## Build offline

```sh
discimage build album.toml --output output/album
discimage verify output/album
discimage readback output/album --output output/readback.png
```

The bundle contains audio, a session-offset ISO, a RAW data image, previews and a checksummed manifest. `verify` and `readback` use local files. The recording job uses the ISO at the measured address. The layout is music first, then Mode 1 data. Full Blue Book CD Extra/XA compliance and consumer player compatibility remain unverified.

[Album template](projects/README.md) · [Media presets](calibrations/README.md) · [Calibration guide](docs/calibration.md) · [Experimental read-timing calibration](docs/timing.md) · [Hardware results](docs/validation.md) · [Repository layout](docs/files.md)

Inspired by [DiscImageStudio](https://github.com/JiaFeiMiao-K-Cat/DiscImageStudio) by JiaFeiMiao-K-Cat and contributors, including its CDImage-derived CIRC mapping. See [credits](NOTICE.md) and the [GPL-3.0-or-later license](LICENSE).
