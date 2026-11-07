# discimage

> [!WARNING]
> **SAFETY WARNING · 刻录前必读**
> - **先用 `burn --dry-run` 预演，并确认选中的光驱和盘片。**
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

在 `album.toml` 所在的专辑目录运行。

```sh
discimage plan album.toml
mkdir -p output
discimage template album.toml --output output/templates
discimage preview album.toml --output output/preview.svg
```

输出的父目录只需创建一次，每次导出使用新路径。图片还没做完时也可以先生成模板。用浏览器打开 SVG。`strip` 从 12 点顺时针环绕，长条上边对应外缘。`mode = "disc"` 将图片居中并裁到可绘制的圆环。支持 PNG 和 JPEG。已有盘片标定时，先在专辑中引用对应的 `preset`，再规划和预览。见[盘片预设](../calibrations/README.md)、[图案选项](preview.md)及[完整专辑示例](../examples/album.toml)。

`discimage devices` 显示由标准光驱厂商、型号、序列号生成的 12 位十六进制硬件 ID，不带前缀。
只要光驱向系统提供相同序列号，macOS、Windows、Linux 得到的 ID 就相同；不包含端口、盘符或固件版本。
检测只读取设备元数据。读不到可靠序列号时显示 `local-xxxxxxxxxx`，它只代表当前系统连接。
`devices --json` 保留硬件字段、`hardware_id`、`connection_selector`、首选 `selector` 和完整系统 `id`。
用 `--device GX40N` 选择唯一型号，或复制显示的标识；列表序号不能用于选择。
标识不存在、匹配多台或硬件 ID 重复时停止操作。已有作业仍绑定原始系统 ID，换连接后应重新准备作业。

两种写法任选其一：

```sh
# 先保存默认光驱，再准备作业。
discimage select GX40N
discimage burn album.toml -o record --dry-run
```

```sh
# 或者一次写全，明确指定本次光驱。
discimage burn album.toml -o record --dry-run --device GX40N
```

`select` 单独运行可查看默认值，`select --clear` 清除默认值。
`--device` 只覆盖本次操作，不修改默认值。
默认选择用于 `erase`、新刻录作业和需要光驱的标定命令；指定光驱断开或匹配不唯一时停止，不改选其他设备。
已有作业继续使用保存的光驱，显式 `--device` 必须与其一致；离线命令不读取默认选择。
`select` 只保存 ID，不占用或写入盘片。
设置以 `device = "ID"` 存在用户目录下的 `discimage/config.toml` 中：macOS 位于
`~/Library/Application Support`，Windows 位于 `%APPDATA%`，Linux 位于 `$XDG_CONFIG_HOME`（默认 `~/.config`）。

## 在 macOS 刻录

预留几 GB 硬盘空间，放入空白 CD-R 或 CD-RW，在系统终端中运行：

```sh
discimage devices
mkdir -p output
discimage burn album.toml --output output/record --dry-run
open output/record/planned/preview.svg
# 确认预览后再开始写盘
discimage burn output/record
```

`burn --dry-run` 会选择唯一符合条件的空白盘，保存光驱和请求速度并生成预览，不写盘。有多个光驱符合条件时，用 `--device` 指定 `devices` 显示的短标识或唯一型号。省略 `--speed` 时，由光驱按盘片选择支持的最高速度；需要固定倍速时使用 `--speed N`。已有作业恢复时沿用保存的速度。每次重新刻录都使用新的输出目录，已完成的作业不能再次准备。用过的 CD-RW 先运行 `discimage erase`：自动选择唯一可用的 CD-RW，确认后完整擦除并检查空盘状态。`erase --dry-run` 只检查选择，`erase --quick` 仅快速清除盘片目录。

`burn JOB` 先写音乐会话并逐字节读回 PCM，再按光驱报告的下一可写地址重建、写入并校验数据会话。完成后，`output/record/journal.json` 应显示 `complete`，再次运行 `discimage devices` 可确认盘片已封盘且显示两个会话。盘面图案需要实物观察。若希望一次完成准备和刻录，使用 `discimage burn album.toml -o output/new-job`。中断恢复和 CD-Text 见[刻录说明](recording.md)。

旧的 `prepare`、`resume`、`readback` 保留为隐藏兼容入口；新流程分别使用 `burn PROJECT --dry-run`、`burn JOB`、`verify BUNDLE -o FILE`。

日志显示当前阶段。写盘时还会显示光驱报告的进度、速度和用时。macOS 自带终端使用基础 ANSI 配色，设置 `NO_COLOR=1` 可关闭颜色。重定向的日志使用纯文本，`plan --json` 和 `devices --json` 将 JSON 写到标准输出。

## 离线制作

```sh
mkdir -p output
discimage build album.toml --output output/album
discimage verify output/album
discimage verify output/album --output output/readback.png
```

制品包含音频、带会话偏移的 ISO、RAW 数据镜像、预览和校验清单。`verify` 检查本地文件。实际刻录按实测地址使用 ISO。布局为音乐在前、Mode 1 数据在后。完整 Blue Book CD Extra/XA 合规性及消费级播放器兼容性仍待验证。

[专辑模板](../projects/README.md) · [盘片标定配置](../calibrations/README.md) · [校准指南](calibration.md) · [读取计时实验](timing.md) · [实机结果](validation.md) · [目录结构](files.md)

项目受 JiaFeiMiao-K-Cat 及贡献者的 [DiscImageStudio](https://github.com/JiaFeiMiao-K-Cat/DiscImageStudio) 启发，绘图的 CIRC 映射继承了 CDImage 的工作。见[来源说明](../NOTICE.md)和 [GPL-3.0-or-later 许可](../LICENSE)。
