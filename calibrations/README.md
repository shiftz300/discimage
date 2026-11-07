# Calibrations · 标定配置

每种盘一份 TOML，集中保存该盘的预设和标定记录。 Each medium has one TOML for its presets and calibration records.

| 配置 / File | 预设 / Preset | 实盘结果 / Result |
| --- | --- | --- |
| [Verbatim SW80EU1 CD‑RW](sw80eu1/disc.toml) | `sw80eu1` | 盘面目视确认 / Artwork visually checked |
| [Verbatim SW80EU1 CD‑RW](sw80eu1/disc.toml) | `sw80eu1-timing` | 时序试验，数据读回通过 / Experimental, digital readback passed |
| [AZO Gold Vinyl](azo-gold-vinyl/disc.toml) | `azo-gold-vinyl` | 盘面目视对齐，数据读回通过 / Alignment confirmed, digital readback passed |

在 `projects/my-album/album.toml` 顶层添加一行。路径相对专辑文件，`preset` 和 `[disc]` 二选一。已准备的刻录任务会保存解析后的参数。

Add this line at the top level of `projects/my-album/album.toml`. The path is relative to the album file. Use either `preset` or `[disc]`. Prepared recording jobs store the resolved settings.

```toml
preset = { file = "../../calibrations/azo-gold-vinyl/disc.toml", name = "azo-gold-vinyl" }
```

`[profiles]` 是专辑使用的参数。`[verification]` 记录实测光驱、固件和请求速度，`[measurements]` 保留照片拟合等观测值。选定预设后，刻录仍使用当前光驱报告的可写空间和第二会话起点。换光驱或速度时，同批盘可以沿用预设，先用一张检查盘面对齐和读回。

`[profiles]` supplies the album settings. `[verification]` records the tested drive, firmware, and requested speed. `[measurements]` holds observations such as the photo fit. Recording uses the current drive's writable capacity and measured second-session start. When changing drive or speed, use the same-batch preset as a starting point and check artwork alignment and readback on the first disc.

原始报告留在 Git 忽略的各介质 `evidence/` 目录。 Raw reports stay in each medium's Git-ignored `evidence/` directory.
