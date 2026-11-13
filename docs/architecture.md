# Architecture

[Guides](README.md) · [Validation](validation.md)

One Rust crate. Offline authoring, recording and read-only calibration share the
same geometry and encoding code; native device access stays behind platform adapters.

| Area | Modules | Responsibility |
| --- | --- | --- |
| Inputs and layout | `project`, `audio`, `layout` | TOML, PCM validation, sector budgets and measured coverage |
| Drawing | `spiral`, `art`, `target`, `codec` | Radius/angle mapping, image sampling, CIRC positions and Mode 1 framing |
| Outputs | `iso`, `author`, `package`, `preview`, `template` | Filesystem, frozen packages, hashes and graphical exports |
| Recording | `burn`, `device`, `recorder`, `mmc` | Reservation, two sessions, NWA, verification and finalization |
| Calibration | `guide`, `calibrate`, `probe`, `timing`, `phase` | Target recording, timing acquisition, uncertainty and photo fitting |
| CLI | `main`, `selection`, `console` | Commands, drive defaults and terminal output |

## Authoring and recording

```text
TOML + WAV + artwork → Plan → frozen package
                                  ↓ burn
           audio SAO → data TAO → actual NWA → audio readback
                                  ↓
              data encoding → write → close → data and audio readback
```

`build` freezes one flat input set without accessing a drive. ISO/RAW export is
optional. The recorder inspects the disc without changing its write mode. After
checking the recorded audio layout, `burn` explicitly selects data TAO and reads
the actual next writable address (NWA). It rebuilds the data image using
absolute disc LBAs and checks the address and capacity again before writing.

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

The same MMC recorder handles burn, erase and calibration targets on macOS and
Windows. macOS uses IOKit exclusive optical tasks and obtains discovery metadata
from DiscRecording. Windows transports the same commands through IMAPI2. The
timing probe has a separate bounded read interface on each platform.
Linux supports offline authoring and discovery; recording is not implemented.
