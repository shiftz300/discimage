# Repository layout · 目录与文件说明

| 路径 / Path | 用途 / Purpose |
| --- | --- |
| [README.md](../README.md) · [zhcn.md](zhcn.md) | 快速上手 / Quick start |
| [docs/](.) | [刻录](recording.md)、[标定](calibration.md)、[时序](timing.md)、[预览](preview.md)、[验证](validation.md)和[架构](architecture.md) / Guides, hardware results and architecture |
| [examples/](../examples/) | [完整参数示例](../examples/album.toml) / Annotated configuration reference |
| [projects/](../projects/) | [专辑模板](../projects/template/album.toml)、本地专辑素材和各专辑的 `output/` / Starter template, private albums and their own `output/` directories |
| [calibrations/](../calibrations/) | 每种盘一份标定配置，可附本地证据 / One calibration file per medium, with local evidence |
| [src/](../src/) | Rust 命令、镜像制作、标定和刻录流程 / CLI, authoring, calibration and recording logic |
| [native/](../native/) · [build.rs](../build.rs) | macOS 光驱桥接与编译入口 / macOS bridge and build setup |
| [tests/](../tests/) | 离线测试、模拟光驱测试和固定对照数据 / Offline tests, recorder tests and fixtures |
| [Cargo.toml](../Cargo.toml) · [Cargo.lock](../Cargo.lock) | crate 配置和依赖锁定 / Crate configuration and locked dependencies |
| [LICENSE](../LICENSE) · [NOTICE.md](../NOTICE.md) | 许可证和来源归属 / License and attribution |
| [.github/workflows/](../.github/workflows/) | CI 检查 / Continuous integration |
| `target/` | Cargo 编译产物 / Cargo build products |

把可编辑的配置、原始音频和图案放在 `projects/<album>/`。每种盘型保留独立 TOML，
共享原始素材。`build` 默认输出到该项目的 `output/<TOML文件名>/`，成品内部所有文件平铺。
Git 忽略个人专辑、生成物和原始标定证据。

Editable albums live in `projects/<album>/`. Separate medium configurations share
original inputs. Each build produces one flat `output/<TOML name>/` package.

| 文件 / File | 用途 / Purpose |
| --- | --- |
| `01.wav`, `02.wav`, … | 唯一一套归一化 PCM / One normalized audio set |
| `artwork.png` | 冻结的图案输入 / Frozen artwork |
| `manifest.toml` | 参数、输入校验和、数据镜像信息和刻录状态 / Settings, hashes and recording state |
| `preview.svg` | 当前设计或实际写入区域的完整预览 / Current self-contained preview |
| `session2.iso` | 请求导出或读取实际 NWA 后生成 / Requested export or measured data-session image |
| `session2.bin` | 仅显式 `--raw` 时生成 / Explicit RAW export only |
| `file-0001.bin`, … | 可选附加文件，原路径存在清单中 / Optional companion data with original ISO paths preserved |
| `target.json` | 标靶才有 / Calibration targets only |

`.work/` 只用于生成数据时的临时文件，成功后删除；不保留 `source/planned/actual-N`。
旧格式读写兼容仅用于恢复和迁移，见[刻录说明](recording.md)。
