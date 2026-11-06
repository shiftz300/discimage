# discimage

> [!WARNING]
> **SAFETY WARNING · 刻录前必读**
> - **先用 `prepare` 预演，并确认选中的光驱和盘片。**
> - **`burn`、`resume` 和标定刻录都会写盘。CD‑R 刻后无法擦除，擦除 CD‑RW 会清空盘内数据。**
> - **只用光驱和盘片都支持的速度。请勿以管理员权限运行，写入期间请勿拔掉光驱。写入失败先看作业日志，再决定是否重试。**

[English](../README.md)

制作外圈带图案的音乐 CD。macOS、Windows 和 Linux 都可规划、预览和制作离线制品，目前可在 macOS 直接刻录。

## 安装

从 [GitHub Releases](https://github.com/shiftz300/discimage/releases/latest) 下载适合自己系统的预编译包。
解压前对照 `SHA256SUMS.txt` 核对压缩包。macOS 用 `shasum -a 256 <压缩包>`，Linux 用 `sha256sum <压缩包>`，Windows PowerShell 用 `Get-FileHash <压缩包>`。把 `<压缩包>` 换成下载的文件名。

| 系统 | 压缩包后缀 |
| --- | --- |
| macOS，Apple 芯片 | `macos-arm64.tar.gz` |
| macOS，Intel 芯片 | `macos-x64.tar.gz` |
| Linux，x86-64 | `linux-x64.tar.gz` |
| Windows，x86-64 | `windows-x64.zip` |

macOS 或 Linux 用户在解压目录打开终端，运行：

```sh
mkdir -p "$HOME/.local/bin"
cp -p ./discimage "$HOME/.local/bin/discimage"
export PATH="$HOME/.local/bin:$PATH"
discimage --version
```

Windows 用户在解压目录打开 PowerShell，运行：

```powershell
$env:Path = "$((Get-Location).Path);$env:Path"
discimage --version
```

上述命令把可执行文件加入当前终端的 PATH。主要功能直接运行即可。可选的 `calibrate probe` 需要外置 DiscProbe 和 Python。macOS 发布包未经 Apple 公证。如果系统拦截已核对的下载，请按 [Apple 的说明打开受信任的程序](https://support.apple.com/zh-cn/102445)。

## 规划与预览

每首歌准备一个 44.1 kHz、16 位、双声道 PCM WAV，至少四秒。其他格式可用 XLD 或 FFmpeg 转换，例如 `ffmpeg -i song.flac -ar 44100 -ac 2 -c:a pcm_s16le 01.wav`。

把 `album.toml` 放在 WAV 旁。文件路径相对此配置，数组顺序就是播放顺序。

```toml
title = "My album"
tracks = [{ path = "01.wav" }, { path = "02.wav" }]

[artwork]
path = "artwork.png"
mode = "strip"
```

```sh
mkdir output
discimage plan album.toml
discimage template album.toml --output output/templates
discimage preview album.toml --output output/preview.svg
```

输出的父目录只需创建一次，每次导出使用新路径。图片还没做完时也可以先生成模板。用浏览器打开 SVG。`strip` 从 12 点顺时针环绕，长条上边对应外缘。`mode = "disc"` 将图片居中并裁到可绘制的圆环。支持 PNG 和 JPEG。已有盘片标定时，先在专辑中引用对应的 `preset`，再规划和预览。见[盘片预设](../calibrations/README.md)、[图案选项](preview.md)及[完整专辑示例](../examples/album.toml)。

## 在 macOS 刻录

预留几 GB 硬盘空间，放入空白 CD-R 或 CD-RW，在系统终端中运行：

```sh
discimage devices
discimage prepare album.toml --output output/record --speed 8
open output/record/planned/preview.svg
# 确认预览后再开始写盘
discimage resume output/record
```

`prepare` 会选择唯一符合条件的空白盘，保存光驱和请求速度并生成预览，不写盘。有多个光驱符合条件时，添加 `--device 'DRIVE_ID'`，填入 `devices` 显示的完整 ID。选择光驱和盘片都支持的速度。用过的 CD-RW 需单独擦除。

`resume` 先写音乐会话并逐字节读回 PCM，再按光驱报告的下一可写地址重建、写入并校验数据会话。完成后，`output/record/journal.json` 应显示 `complete`，再次运行 `discimage devices` 可确认盘片已封盘且显示两个会话。盘面图案需要实物观察。中断恢复、CD-Text 和直接 `burn` 见[刻录说明](recording.md)。

日志显示当前阶段。写盘时还会显示光驱报告的进度、速度和用时。macOS 自带终端使用基础 ANSI 配色，设置 `NO_COLOR=1` 可关闭颜色。重定向的日志使用纯文本，`plan --json` 和 `devices --json` 将 JSON 写到标准输出。

## 离线制作

```sh
discimage build album.toml --output output/album
discimage verify output/album
discimage readback output/album --output output/readback.png
```

制品包含音频、带会话偏移的 ISO、RAW 数据镜像、预览和校验清单。`verify` 和 `readback` 检查本地文件。实际刻录按实测地址使用 ISO。布局为音乐在前、Mode 1 数据在后。完整 Blue Book CD Extra/XA 合规性及消费级播放器兼容性仍待验证。

[专辑模板](../projects/README.md) · [盘片标定配置](../calibrations/README.md) · [校准指南](calibration.md) · [读取计时实验](timing.md) · [实机结果](validation.md) · [目录结构](files.md)

项目受 JiaFeiMiao-K-Cat 及贡献者的 [DiscImageStudio](https://github.com/JiaFeiMiao-K-Cat/DiscImageStudio) 启发，绘图的 CIRC 映射继承了 CDImage 的工作。见[来源说明](../NOTICE.md)和 [GPL-3.0-or-later 许可](../LICENSE)。
