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

把可编辑的专辑配置、音频和图案放在 `projects/<album>/`。模板、预览、离线镜像和刻录作业放在该专辑的 `output/` 下。每次刻录使用新的作业目录，保存当次输入的快照和实际写入的会话。Git 忽略本地专辑、生成物和原始标定证据。

Keep editable album inputs in `projects/<album>/`. Store templates, previews, offline images and recording jobs in that album's `output/` directory. Use a fresh directory for each burn. A job stores its own input snapshot and recorded session. Git ignores private albums, generated files and raw calibration evidence.

| 制品 / Artifact | 如何识别 / What to look for |
| --- | --- |
| 模板 / Template | `layout.json`、PNG 和 SVG，供图案编辑使用 / Geometry plus artwork canvases |
| 离线镜像 / Offline bundle | `manifest.json`、`audio/`、`session2.iso`、`session2.bin` 和预览 / Checksummed files for local inspection |
| 刻录作业 / Burn job | `journal.json` 保存阶段，`planned/` 是预检，`actual-N/` 对应实测写入地址 / State, planned image and measured-session bundle |
| 标靶 / Calibration target | `calibration.iso` 和封盘后生成的 `single.json` / Single-session image and finalized receipt |
| 读回 / Readback | 数据或音频校验报告以及可选的 `cdtext.txt` / Digital verification and optional CD‑Text output |
