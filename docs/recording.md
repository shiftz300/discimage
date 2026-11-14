# Make and record an album

[简体中文](zhcn/recording.md) · [Guides](README.md)

Install the release executable and confirm `discimage --version` works.
Paths below are relative to your working directory. macOS records directly;
Windows recording is experimental. Linux can prepare the same offline package.

## 1. Prepare the album

Use 44.1 kHz, 16-bit stereo PCM WAV, one file per track, at least four seconds.
Create `projects/my-album/cdrw.toml` and put the WAVs in its `audio/` directory:

```toml
title = "My album"
preset = { file = "../../calibrations/my-batch/disc.toml", name = "my-batch" }
intertrack_gap_position = "tail"
intertrack_gap_sectors = 150
track_lead_in_sectors = 10
tracks = [{ path = "audio/01.wav" }, { path = "audio/02.wav" }]
```

Use the preset from [calibration](calibration.md) for the actual medium/batch.
The track array controls playing order. Put top-level fields before any table.
Keep CD-R and CD-RW settings in separate TOMLs; they can share audio and artwork.
The example keeps 140 sectors of silence at the end of each track except the
last. The next track starts with 10 sectors (133 ms) of silence after INDEX 01.
The music starts and the planned data address stay at the same positions as
with a 150-sector tail. Build writes the 10 sectors into the package WAVs.
Use zero lead-in and zero gap for a continuous album. The first track has its
own pre-gap before LBA 0. [ITU-R BR.1385 §4](https://www.itu.int/rec/R-REC-BR.1385-0-199812-W/en).

## 2. Design the artwork

```sh
discimage preview projects/my-album/cdrw.toml
discimage template projects/my-album/cdrw.toml -o projects/my-album/output/templates
```

Import the exported `layout.json` into the [editor](https://discimage.kasetry.cn/).
Export its ZIP, place `artwork.png` beside the album TOML, and add the exported
`[artwork]` section without replacing tracks or preset. [Image options](preview.md).
Template output needs a new directory.

## 3. Build and inspect

```sh
discimage build projects/my-album/cdrw.toml
discimage verify projects/my-album/output/cdrw
```

Open `output/cdrw/preview.svg` beside the album in a browser. The package is flat
and includes frozen audio, artwork, checksums and recording state in `manifest.toml`.
Edit source files and rebuild when changing the design; do not edit the package.
These commands do not access the drive.

## 4. Select the drive

```sh
discimage devices
discimage select DRIVE_ID
discimage select
```

Replace `DRIVE_ID` with the hardware ID shown by `devices`. Confirm the default
and insert a matching blank disc. `--device ID` overrides a single operation.
Missing or ambiguous selections stop; display numbers are not drive IDs.

### Reusing CD-RW

If its contents may be cleared, run:

```sh
discimage erase --dry-run
discimage erase
```

Check the displayed target, enter `ERASE`, wait for completion and confirm blank
state with `devices`. Full erase is the default; quick erase may leave artwork.
CD-R requires another blank disc. Preserve calibration evidence before reusing a target.

## 5. Burn and confirm

```sh
discimage burn projects/my-album/output/cdrw
discimage devices
```

Use `--slowest` to request the lowest write speed reported for the inserted disc,
or `--speed N` for a known supported speed. With neither option, speed is automatic.
If the drive does not report supported speeds, `--slowest` stops before writing.
The program checks
the audio samples, silence and Q subchannel track indexes before appending data
at the actual NWA, then repeats the audio checks after finalization. The drive
must support raw P–W or formatted Q readback. Keep it connected throughout.

Success requires successful audio/data readback and a closed disc with two
sessions: N audio tracks and one data track. Inspect the artwork and audition
the music. `verify PACKAGE` checks local files, not physical-disc contents.
For another blank disc, rebuild the album before burning again.

## Recovery

The recorder retries commands explicitly rejected as busy or queue full, with
a bounded backoff. A medium write error or an unknown command outcome stops the
job and preserves its journal.

| Situation | Action |
| --- | --- |
| No blank writable disc | Replace CD-R, or explicitly erase CD-RW |
| Wrong media preset | Select the matching configuration; do not bypass the check |
| Confirmed intermediate state | Keep the same disc/drive/package and rerun `burn PACKAGE` |
| `data_writing` interrupted after finalization | Keep the same disc and rerun `burn PACKAGE` to verify all data and audio without writing |
| Interrupted write without a matching finalized disc | Preserve the journal and inspect the disc |
| Build refuses replacement | Check for an unfinished recording or extra files; preserve recovery evidence |

## Image export, local checks and CD-Text

`build PROJECT --iso` exports cooked data; `--raw` exports framed Mode 1 data.
Without measured `--session-start LBA`, these are estimates. `burn` uses actual NWA.
[Encoded-payload previews](preview.md#optional-encoded-payload-view) are optional.

`[cd_text]` accepts printable ASCII album/performer/track fields; WAV tags do not
become CD-Text automatically. On macOS, `drutil cdtext` can read it on a capable drive.
