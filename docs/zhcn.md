# discimage

[English](../README.md)

用 Rust 编写的音乐光盘与外圈绘图 CLI，当前版本 **0.3.2**。
按专辑实际长度分配音乐区和外圈图案，支持圆盘图、长条环绕、模板及完整预览。
macOS、Windows、Linux 均可离线制作，**目前只有 macOS 支持直接刻录**。

核心概念和绘图技术来自 **[DiscImageStudio](https://github.com/JiaFeiMiao-K-Cat/DiscImageStudio)**，
作者为 JiaFeiMiao-K-Cat 及其贡献者，CIRC 映射还继承了 CDImage 的工作。
参考版本与移植说明见 [NOTICE.md](../NOTICE.md)，采用 [GPL-3.0-or-later](../LICENSE) 许可。

## 安装

安装当前稳定版 Rust，在本仓库中运行：

```sh
cargo install --path . --locked
discimage --version
```

每首歌准备一个 **44.1 kHz、16-bit、双声道 PCM WAV**，至少 4 秒；图片支持 PNG / JPEG。
其他音频可用 XLD 或 FFmpeg 转换，例如：

```sh
ffmpeg -i song.flac -ar 44100 -ac 2 -c:a pcm_s16le 01.wav
```

## 从专辑到预览

在素材旁创建 `album.toml`。曲目顺序以数组为准，路径相对这个 TOML 文件。

```toml
title = "My album"
tracks = [{ path = "01.wav" }, { path = "02.wav" }]
# files = "extras"                   # 可选：随盘文件目录

[artwork]
path = "artwork.png"
mode = "strip"                       # strip 长条环绕 / disc 圆盘构图
tone = "threshold"                   # threshold 阈值 / gray 灰度 / ordered 有序抖动
threshold = 128
rotation = 0.0                       # 顺时针角度
```

```sh
discimage plan album.toml
discimage template album.toml --output templates
discimage preview album.toml --output preview.svg
```

用浏览器打开 `preview.svg`。模板根据音乐时长计算，包含透明 PNG 和可编辑 SVG；
图片尚不存在时也可以先运行 `template`。

- `strip`：长条从左向右顺时针绕一圈，默认接缝在 12 点，上边对应外缘。
- `disc`：图片居中，只显示可绘图圆环；`fit = "cover"` 先裁成正方形，默认 `contain` 保留全图。

制作完整离线制品并检查：

```sh
discimage build album.toml --output album
discimage verify album
discimage readback album --output detailed.png --samples 128 --size 1600
```

`album/` 包含音乐、ISO/RAW 数据、布局预览、设计图、编码负载读回图、差异图及哈希清单。
`verify` 和 `readback` 检查的是**本地制品文件**。输出文件及制品目录必须尚不存在；
一次完整刻录作业请预留数 GB 硬盘空间。

## 在 macOS 刻录

插入空白 CD-R 或 CD-RW，在终端运行以查看阶段、进度和速度。
把 `DRIVE_ID` 替换为 `devices` 返回的完整设备 ID。

```sh
discimage devices --json
discimage burn album.toml --device 'DRIVE_ID' --output record --dry-run
# 查看 record/planned/preview.svg，准备好后开始写盘：
discimage resume record --device 'DRIVE_ID'
```

程序会写音乐、读回并逐字节比较 WAV、读取实际下一可写地址 NWA、重建数据会话，
然后写入、校验并封盘。`record/journal.json` 保存阶段和错误；成功时为 `complete`。
再次运行 `devices --json`，应看到两个会话、不可继续追加，以及音乐轨之后的一条数据轨。
盘面图案是否清晰需要另行观察。

在 `burn` 后加 `--speed 10` 可请求 10×，默认 8×，须选择光驱和盘片支持的速度。
用过的 CD-RW 需要单独擦除；结果不确定的写入不会自动重试。
恢复步骤见 [刻录说明](recording.md)。

## 校准与范围

可运行 `discimage calibrate guide --output calibration` 进行引导式径向校准，
步骤见 [校准说明](calibration.md)。角向对齐还需要合适的螺旋参数。
[SW80EU1 / GX40N 配置](../examples/sw80eu1.toml) 保留了实测组合的参数，
其他盘片和设备需要单独验证。模板、预览和刻录使用相同模型。

实验性的 [读取计时校准](timing.md) 可复用现有数据轨和照片，减少标定重刻。
`calibrate probe` 调用外置 discprobe-macos，实时显示进度；`calibrate import`
在全平台离线生成独立候选配置及可缩放的对比页。原配置保留，候选仍需实物验证。

当前实现音乐在前、Mode 1 数据在后的双会话布局；完整 Blue Book CD Extra/XA 合规及
消费级播放器兼容性仍未验收。带会话偏移的 ISO 和分开的 CUE/TOC 不能直接当作通用整盘镜像。
实机验证记录见 [validation.md](validation.md)。

[全部参数](../examples/album.toml) · [预览细节](preview.md) · [代码结构](architecture.md)

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```
