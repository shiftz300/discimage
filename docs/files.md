# Repository layout · 目录与文件说明

| 路径 / Path | 用途 / Purpose |
| --- | --- |
| [README.md](../README.md) · [zhcn.md](zhcn.md) | 快速上手 / Quick start |
| [docs/](.) | [刻录](recording.md)、[标定](calibration.md)、[时序](timing.md)、[预览](preview.md)、[验证](validation.md)和[架构](architecture.md) / Guides, hardware results and architecture |
| [examples/](../examples/) | [完整参数示例](../examples/album.toml) / Annotated configuration reference |
| [projects/](../projects/) | [专辑模板](../projects/template/album.toml)和本地专辑素材 / Starter template and private album inputs |
| [calibrations/](../calibrations/) | 每种盘一份标定配置，可附本地证据 / One calibration file per medium, with local evidence |
| `output/` | 本地预览、镜像、刻录作业和读回记录 / Local previews, images, burn jobs and readback |
| [src/](../src/) | Rust 命令、镜像制作、标定和刻录流程 / CLI, authoring, calibration and recording logic |
| [native/](../native/) · [build.rs](../build.rs) | macOS 光驱桥接与编译入口 / macOS bridge and build setup |
| [assets/](../assets/) | CLI 内嵌的[取点页](../assets/mark.html)和[时序对比页](../assets/timing.html) / Embedded photo picker and timing comparison |
| [tools/](../tools/) | 独立的照片相位分析脚本 / Standalone photo phase analysis |
| [tests/](../tests/) | 离线测试、模拟光驱测试和固定对照数据 / Offline tests, recorder tests and fixtures |
| [Cargo.toml](../Cargo.toml) · [Cargo.lock](../Cargo.lock) | crate 配置和依赖锁定 / Crate configuration and locked dependencies |
| [LICENSE](../LICENSE) · [NOTICE.md](../NOTICE.md) | 许可证和来源归属 / License and attribution |
| [.github/workflows/](../.github/workflows/) | CI 检查 / Continuous integration |
| `target/` | Cargo 编译产物 / Cargo build products |

在 `projects/<album>/` 放 `album.toml`、音频和图案，在 `output/<album>/` 下为每次制作新建输出目录。专辑素材、生成物和标定原始证据由 Git 忽略。 / Keep inputs in `projects/<album>/` and give each generated run a new destination under `output/<album>/`. Git ignores private albums, generated files and raw calibration evidence.

| 制品 / Artifact | 如何识别 / What to look for |
| --- | --- |
| 模板 / Template | `layout.json`、PNG 和 SVG，供图案编辑使用 / Geometry plus artwork canvases |
| 离线镜像 / Offline bundle | `manifest.json`、`audio/`、`session2.iso`、`session2.bin` 和预览 / Checksummed files for local inspection |
| 刻录作业 / Burn job | `journal.json` 保存阶段，`planned/` 是预检，`actual-N/` 对应实测写入地址 / State, planned image and measured-session bundle |
| 标靶 / Calibration target | `calibration.iso` 和封盘后生成的 `single.json` / Single-session image and finalized receipt |
| 读回 / Readback | 数据或音频校验报告以及可选的 `cdtext.txt` / Digital verification and optional CD‑Text output |
