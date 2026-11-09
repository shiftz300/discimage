# Architecture

[Guides](README.md) · [Validation](validation.md)

One Rust crate. Offline authoring, recording and read-only calibration share the
same geometry and encoding code; native device access stays behind platform adapters.

| Area | Modules | Responsibility |
| --- | --- | --- |
| Inputs and layout | `project`, `audio`, `layout` | TOML, PCM validation, sector budgets and measured coverage |
| Drawing | `spiral`, `art`, `target`, `codec` | Radius/angle mapping, image sampling, CIRC positions and Mode 1 framing |
| Outputs | `iso`, `author`, `package`, `preview`, `template` | Filesystem, frozen packages, hashes and graphical exports |
| Recording | `burn`, `device`, `macos`, `windows`, `mmc` | Reservation, two sessions, NWA, verification and finalization |
| Calibration | `guide`, `calibrate`, `probe`, `timing`, `phase` | Target recording, timing acquisition, uncertainty and photo fitting |
| CLI | `main`, `selection`, `console` | Commands, drive defaults and terminal output |

## Authoring and recording

```text
TOML + WAV + artwork → Plan → frozen package
                                  ↓ burn
              audio → PCM verification → actual NWA
                                  ↓
                data encoding → write → verification → close
```

`build` freezes one flat input set without accessing a drive. ISO/RAW export is
optional. `burn` regenerates the data image after the drive reports NWA;
filesystem extents and artwork positions use absolute disc LBAs.

The journal records write intent before native calls. Package locking prevents
concurrent modification. Uncertain writes stop; confirmed intermediate states
can continue with the same command. See [recovery](recording.md#recovery).

## Shared physical model

`Spiral` maps sectors to radius and angle. `ChannelSpiral` expresses the same
constant-pitch geometry in channel bits. Capacity and radii define the reference
spiral; a smaller writable extent limits coverage instead of rescaling it.

The encoder maps controllable payload bytes through CIRC positions, chooses
`0D`/`AA` and compensates the Mode 1 scrambler. The drive supplies physical
scrambling, parity/channel coding and its own write strategy. Payload projection
is not a complete EFM or reflectivity simulation.

## Calibration and native boundaries

The guide records one target and acquires two independent rounds. `timing`
reconstructs data-only covariance, compares fits and exports an ordinary disc
preset. `layout` enforces its measured coverage during planning and NWA rebuilds.
Photo radial fitting scales radii and velocity together; phase fitting exports
a separate candidate. [Criteria](timing.md).

macOS uses DiscRecording for recording and an exclusive optical MMC user client
for timing. Windows uses exclusive IMAPI2 sessions and synchronous MMC commands,
with QPC timestamps for timing. Probe commands are bounded and cannot write media.
Linux supports offline authoring and discovery; recording is not implemented.
