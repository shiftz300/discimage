# discimage

> [!WARNING]
> **SAFETY WARNING · 刻录前必读**
> - **先用 `build` 预演，并确认选中的光驱和盘片。**
> - **`burn` 和标定刻录都会写盘。CD‑R 刻后无法擦除，擦除 CD‑RW 会清空盘内数据。**
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

上述命令把可执行文件加入当前终端的 PATH。可执行文件包含 macOS 和 Windows 的 CD 读取计时探测，以及离线照片相位比较。macOS 发布包未经 Apple 公证。如果系统拦截已核对的下载，请按 [Apple 的说明打开受信任的程序](https://support.apple.com/zh-cn/102445)。

可选：在当前 zsh 窗口启用 Tab 补全。

```sh
autoload -Uz compinit && compinit
source <(discimage completions zsh)
```

把这两行放到 `~/.zshrc` 中 PATH／别名设置之后，可在新窗口自动加载。
也支持 `bash`、`fish`、`powershell` 和 `elvish`；生成脚本不访问光驱。
更新可执行文件后，再执行 `source <(discimage completions zsh)` 即可重载。
重载会替换旧的 DiscImage 补全函数，不再提示隐藏的兼容命令或参数。

## 设计

每首歌使用 44.1 kHz、16 位、双声道 PCM WAV，至少四秒。
TOML 的音轨数组决定播放顺序，路径相对 TOML 文件。

```toml
title = "我的专辑"
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

`preview` 显示专辑时长和可绘区域，`--json` 输出布局数据。
`strip` 将长条 PNG/JPEG 环绕一圈，`disc` 将居中图片裁进圆环。
将模板导入网页编辑器设计。见[图案参数](preview.md)和[盘片预设](../calibrations/README.md)。

## 制作和刻录

```sh
discimage build album.toml
open output/album/preview.svg
# 插入空白 CD-R 或 CD-RW，检查后刻录：
discimage burn output/album
```

`build` 在所有平台均不需要光驱。它将音频、图案、参数快照和校验清单整理成一份平铺的成品，
默认放在 TOML 旁的 `output/<TOML文件名>/`，用 `-o DIR` 可指定位置。不预先编码估算 ISO。
重建会更新尚未开始或已完成的成品；未完成的刻录和目录内额外的私人文件受到保护。

`burn` 目前在 macOS 实现。它检查光驱和盘片，写音乐并读回比对，读取实际 NWA，
只生成一次正确起点的数据会话，再写入、校验和封盘。
成品内只保留一套输入和实际镜像，没有 `source/`、`planned/`、`actual-N/` 多套目录。
参数、校验和、刻录状态统一保存在 `manifest.toml`。
`burn` 不接受 `-o` 或 `--dry-run`；省略速度自动选择，也可以用 `--speed N`。
开始写入后速度和光驱会固定，恢复时不能更换。

### 光驱选择和 CD-RW

```sh
discimage devices
discimage select 光驱ID
discimage erase             # 仅用于旧 CD-RW；核对后输入 ERASE。
discimage burn output/album
```

也可以每次用 `--device ID` 指定。`select` 查看默认值，`select --clear` 清除默认值。
指定光驱断开、匹配不唯一时停止，不改选其他设备。
硬件 ID 由标准厂商、型号和序列号生成，不带前缀；无法读取可靠序列号时显示 `local-...`，
它只代表当前系统连接。列表序号不能用于选择。`select` 不擦除或写盘。

`erase` 默认完整擦除；`erase --dry-run` 只检查，`erase --quick` 仅清除目录，可能残留旧图案。
CD-R 不能擦除。刻录成功后，`devices` 应显示盘片已封盘、具有两个会话，再检查盘面并试听。

## 可选镜像导出与检查

```sh
discimage build album.toml --iso
discimage build album.toml --raw
discimage verify output/album
discimage verify output/album -o encoded.png
```

`--iso` 导出 2048 字节数据扇区，`--raw` 导出带 Mode 1 帧结构和 EDC/ECC 的 2352 字节扇区，可同时指定。
两者都是数据会话镜像，音乐音轨单独保存。没有实测 `--session-start LBA` 时，导出明确属于估算。
实际刻录仍按光驱报告的 NWA 重建数据并写入 ISO。
`verify` 检查本地文件，不读取实盘；编码投影需已有导出或实际刻录的镜像，按需生成。

旧的已完成作业可用 `discimage verify OLD_JOB --compact` 验证后一次性迁移为平铺结构，
标定证据保持原样。旧的未完成作业仍支持安全恢复。
`plan`、`prepare`、`resume`、`readback` 命令已移除。

日志兼容 macOS 自带终端 ANSI 配色，`NO_COLOR=1` 可关闭颜色，重定向自动使用纯文本。
`preview --json`、`devices --json` 保持标准输出为 JSON。
CD-Text 可选，当前仅支持可打印 ASCII。完整 Blue Book CD Extra/XA 合规性及消费级播放器兼容性仍待验证。

[专辑模板](../projects/README.md) · [校准](calibration.md) · [读取计时](timing.md) · [刻录与恢复](recording.md) · [实机结果](validation.md) · [文件结构](files.md)

项目受 JiaFeiMiao-K-Cat 及贡献者的 [DiscImageStudio](https://github.com/JiaFeiMiao-K-Cat/DiscImageStudio) 启发。
见[来源说明](../NOTICE.md)和 [GPL-3.0-or-later 许可](../LICENSE)。
