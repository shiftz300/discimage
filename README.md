# discimage

> [!WARNING]
> **READ BEFORE RECORDING.**
> - **START WITH `build` AND CONFIRM THE DRIVE AND DISC.**
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
After updating the executable, run `source <(discimage completions zsh)` again.
Reloading replaces DiscImage's old completion functions; hidden compatibility
commands and options are not suggested.

## Design

Use one 44.1 kHz, 16-bit stereo PCM WAV per track (at least four seconds).
The TOML track array sets the order; paths are relative to the TOML.

```toml
title = "My album"
tracks = [{ path = "01.wav" }, { path = "02.wav" }]

[artwork]
path = "artwork.png"
mode = "strip"
```

```sh
discimage preview album.toml
discimage preview album.toml -o preview.svg
discimage template album.toml -o templates
```

`preview` shows music length and available artwork space; `--json` exports the
layout data. `strip` wraps a panoramic PNG/JPEG around the ring; `disc` clips a
centered image to it. Use the website to design artwork with exported templates.
See [artwork options](docs/preview.md) and [media presets](calibrations/README.md).

## Build and record

```sh
discimage build album.toml
open output/album/preview.svg
# Insert a blank CD-R or CD-RW, then write:
discimage burn output/album
```

`build` works without a drive on every platform. It freezes the audio, artwork,
settings and checksums into one flat directory, defaulting to `output/<TOML name>`
beside the project. `-o DIR` overrides that path. No provisional ISO is generated.
Rebuilding updates an unstarted or completed package; unfinished recordings and
unmanaged files are protected.

`burn` is implemented on macOS. It checks the drive and media, writes and reads
back the audio, reads the actual NWA, generates the data session once, verifies
it and finalizes the disc. The package retains one set of inputs and the actual
image; it has no `source/`, `planned/` or `actual-N/` directories. Parameters,
checksums and recording state share `manifest.toml`. `burn` accepts no `-o` or
`--dry-run`. Optional `--speed N` is frozen when recording starts; omit it for
automatic speed. Do not change drives or speed while continuing a recording.

### Drive selection and CD-RW

```sh
discimage devices
discimage select DRIVE_ID
discimage erase             # Used CD-RW only; type ERASE after checking the target.
discimage burn output/album
```

Or pass `--device DRIVE_ID` to a single operation. `select` alone shows the
default; `select --clear` removes it. A missing or ambiguous selection stops the
operation. Hardware IDs use the drive's standard vendor/model/serial fields,
without a prefix; `local-...` is an OS-connection fallback. Display numbers are
not selectors. Selection never erases or writes a disc.

Erasing defaults to a full erase. `erase --dry-run` only checks the target;
`erase --quick` clears metadata and may leave old artwork visible. CD-R cannot
be erased. After a successful burn, `devices` should show a closed disc with
two sessions. Check the physical artwork and audition the music.

## Optional image export and verification

```sh
discimage build album.toml --iso
discimage build album.toml --raw
discimage verify output/album
discimage verify output/album -o encoded.png
```

Choose `--iso`, `--raw`, or both. ISO is 2048-byte data; RAW adds 2352-byte
Mode 1 framing and EDC/ECC. These are data-session images; audio remains separate.
Without an observed `--session-start LBA`, exports are estimates. Recording
always regenerates data at the drive's actual NWA and uses cooked ISO.
`verify` checks local files, not the physical disc. Encoded previews require
an exported or recorded image; they remain optional.

Old completed jobs can be verified and flattened once with
`discimage verify OLD_JOB --compact`; calibration evidence stays in its original
layout. Old unfinished jobs remain readable for safe recovery. The former
`plan`, `prepare`, `resume` and `readback` commands have been removed.

Logs support macOS Terminal ANSI colors, `NO_COLOR=1` and plain redirected output.
`preview --json` and `devices --json` keep stdout machine-readable. CD-Text is
optional and currently supports printable ASCII. Full Blue Book CD Extra/XA
compliance and consumer-player compatibility remain unverified.

[Album template](projects/README.md) · [Calibration](docs/calibration.md) · [Timing](docs/timing.md) · [Recording and recovery](docs/recording.md) · [Hardware results](docs/validation.md) · [Files](docs/files.md)

Inspired by [DiscImageStudio](https://github.com/JiaFeiMiao-K-Cat/DiscImageStudio)
by JiaFeiMiao-K-Cat and contributors, including its CDImage-derived CIRC mapping.
See [credits](NOTICE.md) and the [GPL-3.0-or-later license](LICENSE).
