# discimage

> [!WARNING]
> **READ BEFORE RECORDING.**
> - **START WITH `burn --dry-run` AND CONFIRM THE DRIVE AND DISC.**
> - **`burn` AND CALIBRATION RECORDING WRITE TO THE DISC. A WRITTEN CD-R CANNOT BE ERASED. ERASING A CD-RW DESTROYS ITS CONTENTS.**
> - **USE A SPEED SUPPORTED BY BOTH THE DRIVE AND DISC. DO NOT RUN AS ROOT OR UNPLUG THE DRIVE DURING A WRITE. IF A WRITE FAILS, CHECK THE JOB JOURNAL BEFORE RETRYING.**

[简体中文](docs/zhcn.md)

Make a music CD with artwork in the outer ring. Plan, design and build on macOS, Windows or Linux. Record directly on macOS.

## Install

Download the prebuilt archive for your system from [GitHub Releases](https://github.com/shiftz300/discimage/releases/latest).
Before extracting, compare the archive's hash with `SHA256SUMS.txt`. Use `shasum -a 256 <archive>` on macOS, `sha256sum <archive>` on Linux, or `Get-FileHash <archive>` in PowerShell. Replace `<archive>` with the downloaded filename.

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

These commands add the executable to the current terminal's PATH. The executable includes CD timing acquisition on macOS and Windows and offline photo phase comparison. macOS packages are not notarized. If Gatekeeper blocks a verified download, follow [Apple's instructions for opening a trusted app](https://support.apple.com/en-us/102445).

Optional shell completion (current zsh session):

```sh
autoload -Uz compinit && compinit
source <(discimage completions zsh)
```

Add these lines after PATH/alias setup in `~/.zshrc` to load completion in new
sessions. `completions` also supports `bash`, `fish`, `powershell` and `elvish`.
It prints a script without accessing the drive.

## Plan and preview

Prepare one 44.1 kHz, 16-bit stereo PCM WAV per track (at least four seconds). Convert other formats with XLD or FFmpeg, for example `ffmpeg -i song.flac -ar 44100 -ac 2 -c:a pcm_s16le 01.wav`.

Save `album.toml` beside the WAVs and run the commands below from that album directory. Paths are relative to this file, and the array sets the play order.

```toml
title = "My album"
tracks = [{ path = "01.wav" }, { path = "02.wav" }]

[artwork]
path = "artwork.png"
mode = "strip"
```

```sh
discimage plan album.toml
mkdir -p output
discimage template album.toml --output output/templates
discimage preview album.toml --output output/preview.svg
```

Create the output parent directory once and use a new destination for each export. `template` works before the artwork file exists. Open the SVG in a browser. In `strip` mode the image wraps clockwise from 12 o'clock, with its top edge at the outer rim. Use `mode = "disc"` for a centered image clipped to the available ring. Images can be PNG or JPEG. For a calibrated disc, add the matching `preset` to the album before planning. See [media presets](calibrations/README.md), [artwork options](docs/preview.md) and the [full album example](examples/album.toml).

`discimage devices` shows a 12-character hexadecimal hardware ID, without a prefix, derived from the
standard drive vendor, model and serial number. It stays the same across macOS,
Windows and Linux when the drive exposes that serial; ports, drive letters and
firmware versions are excluded. Discovery uses read-only metadata commands.
Without a reliable serial, the selector is `local-xxxxxxxxxx` and is valid only
for that OS connection. `devices --json` includes the hardware fields,
`hardware_id`, `connection_selector`, preferred `selector` and full native `id`.
Use `--device GX40N` for a unique model, or copy the displayed selector.
Display numbers cannot select a drive. Missing or ambiguous selectors stop the
operation, including duplicate hardware IDs. Saved jobs keep their original
native ID and must be prepared again after moving a drive to another connection.

Choose either a saved default or an explicit device for each command:

```sh
# Save the default, then prepare a job.
discimage select GX40N
discimage burn album.toml -o record --dry-run
```

```sh
# Alternatively, specify the drive in one command.
discimage burn album.toml -o record --dry-run --device GX40N
```

`select` alone shows the saved default; `select --clear` removes it.
`--device` overrides the default for that operation without changing it.
The default applies to `erase`, new recording jobs and live calibration commands.
A missing or ambiguous default stops the operation. Existing jobs always retain
their saved drive; an explicit `--device` must match it. Offline commands ignore
the default. Selection only saves an ID and never reserves or writes a disc.
The setting is `device = "ID"` in the user-level `discimage/config.toml`:
`~/Library/Application Support` on macOS, `%APPDATA%` on Windows, or
`$XDG_CONFIG_HOME` (default `~/.config`) on Linux.

## Record on macOS

Allow a few GB of free disk space, insert a blank CD-R or CD-RW, then run these commands in Terminal:

```sh
discimage devices
mkdir -p output
discimage burn album.toml --output output/record --dry-run
open output/record/planned/preview.svg
# After checking the preview, start writing:
discimage burn output/record
```

`burn --dry-run` uses the only eligible blank disc, saves the drive and requested speed with the job, and builds a preview without writing to the disc. If several drives qualify, pass `--device` with a short selector or unique model from `devices`. By default, the drive chooses its supported maximum speed. Use `--speed N` to request a specific multiplier. Existing jobs keep their saved speed when resumed. Each recording needs a new output directory; a completed job cannot be prepared again. For a previously written CD-RW, run `discimage erase` first. It selects the only available CD-RW, asks for confirmation, performs a full erase and confirms blank media. `erase --dry-run` checks selection without erasing; `erase --quick` only resets disc metadata.

`burn JOB` writes the audio session, compares the recorded PCM against the prepared WAVs, then uses the drive's measured next writable address to build, write and verify the data session. Check `output/record/journal.json` for stage `complete` and run `discimage devices` again to confirm the disc is closed and reports two sessions. Inspect the disc surface for artwork quality. To prepare and record immediately, use `discimage burn album.toml -o output/new-job`. For interruptions and CD-Text, see [recording and recovery](docs/recording.md).

The former `prepare`, `resume` and `readback` commands remain hidden compatibility entry points. Use `burn PROJECT --dry-run`, `burn JOB` and `verify BUNDLE -o FILE` in new workflows.

Logs show the active stage and, during drive writes, reported progress, speed and elapsed time. The macOS Terminal uses basic ANSI colors. Set `NO_COLOR=1` for plain logs. Redirected logs are plain text, and `plan --json` and `devices --json` write JSON to stdout.

## Build offline

```sh
mkdir -p output
discimage build album.toml --output output/album
discimage verify output/album
discimage verify output/album --output output/readback.png
```

The bundle contains audio, a session-offset ISO, a RAW data image, previews and a checksummed manifest. `verify` uses local files. The recording job uses the ISO at the measured address. The layout is music first, then Mode 1 data. Full Blue Book CD Extra/XA compliance and consumer player compatibility remain unverified.

[Album template](projects/README.md) · [Media presets](calibrations/README.md) · [Calibration guide](docs/calibration.md) · [Experimental read-timing calibration](docs/timing.md) · [Hardware results](docs/validation.md) · [Repository layout](docs/files.md)

Inspired by [DiscImageStudio](https://github.com/JiaFeiMiao-K-Cat/DiscImageStudio) by JiaFeiMiao-K-Cat and contributors, including its CDImage-derived CIRC mapping. See [credits](NOTICE.md) and the [GPL-3.0-or-later license](LICENSE).
