# discimage

> [!WARNING]
> **SAFETY WARNING · 刻录前必读**
> - **写入前确认光驱和盘片。CD-R 不能擦除，擦除 CD-RW 会清空内容。**
> - **不要以管理员权限运行或在写入中拔掉光驱。失败先保留作业并检查日志，再决定是否重试。**

[English](../README.md) · [使用指南](README.md)

制作外圈带图案的音乐 CD。macOS、Windows、Linux 都可设计和制作离线成品。
macOS 可直接刻录，Windows 刻录仍属实验性。

## 安装

下载 [Releases](https://github.com/shiftz300/discimage/releases/latest) 中对应系统的包，
对照 `SHA256SUMS.txt` 校验并解压。不需要 Rust 或 Python。

在解压目录打开终端，macOS/Linux：

```sh
mkdir -p "$HOME/.local/bin"
cp -p ./discimage "$HOME/.local/bin/discimage"
export PATH="$HOME/.local/bin:$PATH"
discimage --version
```

Windows PowerShell：

```powershell
$env:Path = "$((Get-Location).Path);$env:Path"
discimage --version
```

PATH 修改对当前窗口生效。macOS 发布包未经公证；核对后的文件若被拦截，
按 [Apple 说明](https://support.apple.com/zh-cn/102445)打开。

## 1. 准备预设与音频

已有对应盘型的[预设](../calibrations/README.md)可直接使用；新批次先按[标定指南](zhcn/calibration.md)制作预设。
将 44.1 kHz、16 位、双声道 PCM WAV 放进 `audio/`，创建 `album.toml`：

```toml
title = "我的专辑"
preset = { file = "calibrations/my-batch/disc.toml", name = "my-batch" }
tracks = [{ path = "audio/01.wav" }, { path = "audio/02.wav" }]
```

替换实际路径与预设名，数组顺序就是播放顺序。

## 2. 设计图案

```sh
discimage preview album.toml
discimage template album.toml -o templates
```

在[网页编辑器](https://discimage.kasetry.cn/)导入 `templates/layout.json`，设计并导出 ZIP。
把 `artwork.png` 放在配置旁，将导出的 `[artwork]` 段加入专辑，保留原曲目和预设。
[图案操作](zhcn/preview.md)。

## 3. 制作和刻录

```sh
discimage build album.toml
discimage verify output/album
```

用浏览器查看 `output/album/preview.svg`。放入对应空白盘，再运行：

```sh
discimage devices
discimage select DRIVE_ID
discimage select
discimage burn output/album
```

`DRIVE_ID` 换成列表里的硬件 ID。写过的 CD-RW 先[显式擦除](zhcn/recording.md#复用-cd-rw)，CD-R 换空白盘。
速度默认自动选择。完成后 `devices` 应显示封盘、两个会话，日志包含音频和数据回读通过；再检查盘面并试听。
[完整流程与失败处理](zhcn/recording.md)。

按需使用[拍照精修与中文 AI 提示词](zhcn/calibration.md#3-可选拍照精修)。
zsh 补全：`autoload -Uz compinit && compinit`，再运行 `source <(discimage completions zsh)`。
其他参数见 `discimage COMMAND --help`。

CD-Text 当前支持可打印 ASCII；完整 Blue Book 合规性和消费级播放器兼容性仍待验证。
项目受 [DiscImageStudio](https://github.com/JiaFeiMiao-K-Cat/DiscImageStudio) 启发。
[来源](../NOTICE.md) · [GPL-3.0-or-later](../LICENSE)。
