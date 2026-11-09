# 制作并刻录专辑

[English](../recording.md) · [文档入口](../README.md)

先安装发布版，确认 `discimage --version` 可用。下面路径相对你的工作目录，不要求下载源码。
macOS 可直接刻录，Windows 刻录仍属实验性，Linux 可制作离线成品。

## 1. 准备专辑

每首使用 44.1 kHz、16 位、双声道 PCM WAV，至少四秒。
把 WAV 放进 `projects/my-album/audio/`，创建 `projects/my-album/cdrw.toml`：

```toml
title = "我的专辑"
preset = { file = "../../calibrations/my-batch/disc.toml", name = "my-batch" }
tracks = [{ path = "audio/01.wav" }, { path = "audio/02.wav" }]
```

引用对应盘型、批次的[标定预设](calibration.md)。数组顺序就是播放顺序，顶层字段放在所有表之前。
CD-R 与 CD-RW 分别保存配置，可以共享音频和图案。

## 2. 设计图案

```sh
discimage preview projects/my-album/cdrw.toml
discimage template projects/my-album/cdrw.toml -o projects/my-album/output/templates
```

在[网页编辑器](https://discimage.kasetry.cn/)导入 `layout.json`，设计后导出 ZIP。
把 `artwork.png` 放到专辑 TOML 旁，加入导出的 `[artwork]` 段，保留曲目和预设。
模板输出目录必须尚不存在。[图片模式与清晰度](preview.md)。

## 3. 制作并检查

```sh
discimage build projects/my-album/cdrw.toml
discimage verify projects/my-album/output/cdrw
```

用浏览器打开专辑目录下的 `output/cdrw/preview.svg`。
成品目录平铺存放冻结后的音频、图案、校验信息和 `manifest.toml` 刻录状态。
改设计时修改原文件并重新 build，不手改成品。这两条命令不接触光驱。

## 4. 选光驱

```sh
discimage devices
discimage select DRIVE_ID
discimage select
```

`DRIVE_ID` 换成列表里的硬件 ID。确认默认光驱，放入匹配的空白盘。
`--device ID` 可覆盖一次操作；目标断开或不唯一会停止，列表序号不能当 ID。

### 复用 CD-RW

确认允许清除内容后：

```sh
discimage erase --dry-run
discimage erase
```

核对目标，输入 `ERASE`，等待完成，用 `devices` 确认空白。默认完整擦除，快速擦除可能残留图案。
CD-R 只能换空白盘。复用标靶前先保存标定证据。

## 5. 刻录并确认

```sh
discimage burn projects/my-album/output/cdrw
discimage devices
```

省略 `--speed` 自动选速。程序写音乐并回读，取得实际 NWA 后生成、写入、校验数据，再封盘。
全程保持光驱连接。成功应包含音频和数据回读通过，以及封盘的两个会话：N 首歌的 N 条音轨加一条数据轨。
查看盘面并试听；`verify` 只查本地文件，不代替实盘回读。刻下一张前重新 build 同一配置。

## 失败时怎么做

| 情况 | 操作 |
| --- | --- |
| 没有空白可写盘 | CD-R 换盘；CD-RW 显式擦除 |
| 预设盘型不符 | 选择匹配配置，不改字段绕过检查 |
| 已确认的中间状态 | 保留原盘、光驱和成品，重跑同一条 burn |
| `music_writing` / `data_writing` 不确定 | 保留日志和清单，检查盘片后再决定 |
| build 拒绝覆盖 | 检查未完成刻录或额外文件，保留恢复证据 |

## 可选导出与 CD-Text

`build 配置 --iso` 导出数据 ISO，`--raw` 导出带 Mode 1 帧的数据。
没有实测 `--session-start LBA` 时是估算；实际刻录仍使用光驱 NWA。
[编码负载预览](preview.md#可选编码负载预览)可按需生成。

`[cd_text]` 支持可打印 ASCII 的专辑、表演者和曲名；WAV tag 不会自动变成 CD-Text。
macOS 的 `drutil cdtext` 可在支持的光驱上读回。
