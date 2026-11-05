# discimage

面向 **macOS、Windows、Linux** 的音乐光盘与外圈绘图 CLI。按音乐的实际长度保留 CD-DA 音乐区，把剩余空间用于随盘文件和可见图案。

**0.4.0 接入 macOS 双会话刻录流程**：探测 CD-R、写音乐会话、读取实际 NWA、重建并写入绘图数据会话、封盘。保留三平台离线制作、两种图片模式、模板和预览。macOS 原生接口及模拟写入流程已验证，实体光盘与播放器验收仍待完成。

## 来源与许可

核心概念和 CD 绘图技术参考 **[DiscImageStudio](https://github.com/JiaFeiMiao-K-Cat/DiscImageStudio)**，由 **JiaFeiMiao-K-Cat 及其贡献者**开发：把图像映射到光盘螺旋，通过编码后的标记分布形成肉眼可见图案，并通过介质标定改善位置精度。

参考版本：[`3a5cde635c182d6a7e01ff99c2cc1be2ac3ab668`](https://github.com/JiaFeiMiao-K-Cat/DiscImageStudio/tree/3a5cde635c182d6a7e01ff99c2cc1be2ac3ab668)。CIRC 位置映射还继承其上游 [CDImage](https://github.com/arduinocelentano/cdimage) 的工作。Rust 移植与修改说明保留在 `codec.rs`、`author.rs` 和 [NOTICE.md](NOTICE.md)。本项目采用 **GPL-3.0-or-later**，全文见 [LICENSE](LICENSE)。

## 制作一次专辑

安装近期稳定版 Rust，准备 44.1 kHz / 16-bit / 双声道、小端 PCM format-tag 1 WAV。需要转换时先使用外部 FFmpeg：

```sh
ffmpeg -i song.flac -ar 44100 -ac 2 -c:a pcm_s16le song.wav
cargo build --release --locked
```

创建 `album.toml`；路径相对项目文件，曲目顺序以数组为准：

```toml
title = "My short album"
files = "extras"                       # 可选；真实目录，不接受符号链接
tracks = [{ path = "01.wav" }, { path = "02.wav" }]

[artwork]                             # 可选；省略时绘图区为均匀白色负载
path = "artwork.png"                  # PNG / JPEG
mode = "disc"                         # disc 圆盘 / strip 长条环绕
fit = "contain"                       # contain / cover，仅用于 disc
rotation = 0.0                        # 顺时针角度
tone = "gray"                         # gray / threshold / ordered
threshold = 128
gamma = 1.0
brightness = 0
invert = false
```

```sh
./target/release/discimage plan album.toml --json
./target/release/discimage template album.toml --output templates
./target/release/discimage preview album.toml --output preview.svg
./target/release/discimage build album.toml --output album
./target/release/discimage verify album
./target/release/discimage readback album --output detailed.png --samples 128 --size 1600
```

`build` 必须使用尚不存在的目录，父目录须已存在。参数或容量检查失败会报错；写入过程中失败则保留 `incomplete` 标记，校验器拒绝将其视为成品。不覆盖已有目录或预览文件，不写盘、不擦除、不弹出光驱。

完整参数例子见 [examples/album.toml](examples/album.toml)。输入目前不接受 RF64、WAVE_FORMAT_EXTENSIBLE 或浮点 WAV，每轨至少占 300 扇区（4 秒）。音乐采样不归一化、不混入绘图数据，仅在最后一个扇区末尾补零。

## 在 macOS 上写盘

连接刻录机并插入空白 CD-R，然后读取 `id`、介质和能力：

```sh
./target/release/discimage devices --json
```

用返回的完整设备 `id` 替换下面的占位符。可以直接写入，也可以先 `--dry-run` 生成可审阅的作业。预演不占用或写入光盘；会在硬盘上保存固定的输入快照和完整预制镜像。

```sh
./target/release/discimage burn album.toml --device '设备 id' --output record --dry-run
./target/release/discimage resume record --device '设备 id'
```

省略 `--dry-run` 时，`burn` 会从准备直接进入刻录。`--speed` 默认请求 8×，实际速度由设备与系统决定。作业目录必须尚不存在；两份会话布局和镜像可能占用数 GB 磁盘空间。

1. 检查空白 CD-R、容量和驱动器的 CD-R / SAO / TAO 能力。保存 `source/` 快照和 `planned/`，校验素材与镜像后才开始写盘。
2. 使用 SAO 写入第一会话，按项目间隔布置音乐，并保持光盘可追加。
3. 确认读回的音乐轨号、位置和会话吻合。只使用驱动器报告的 **NWA** 和剩余扇区；不使用估算起点代替。以该地址重建 `actual-1/` 的文件系统、绘图负载与预览。
4. 写入前复查 NWA 与容量，以 TAO 写入 **2048 字节扇区的 Mode 1 数据流**，校验数据并关闭追加。最终检查两会话、轨号、数据起点和长度。

`journal.json` 保存当前阶段、最后观测到的介质状态、NWA 和错误。`prepared`、`music_written`、`data_ready` 可用 `resume`；后两者只继续数据会话。`music_writing` / `data_writing` 表示写入结果不确定，不会自动重试。Ctrl+C 在写入阶段请求系统中止；在离线生成阶段会阻止下一次写入。当前不擦除、不超刻，也不自动弹盘。

目前仍是 **Mode 1 的音乐在前、数据在后**布局；完整 Blue Book / XA、实体盘效果和播放器兼容性未验收。接口行为和验证范围见 [recording.md](docs/recording.md)。

## 根据专辑导出模板

`template` 读取实际 WAV 长度，使用与预览、制作相同的布局计算；图片尚不存在时也能先导出模板。输出目录必须不存在：

```text
templates/
  disc.svg / disc.png     圆盘正方形画布，仅可绘图环带为不透明白色
  strip.svg / strip.png   长条展开画布，整个矩形对应一圈环带
  guide.svg              带说明的布局参考图
  layout.json            专辑布局、半径、画布尺寸与环绕约定
```

默认圆盘画布 1600×1600，可用 `--size 256..4096` 修改。长条尺寸按环带中间圆的周长与径向宽度计算，匹配圆盘像素密度，宽度最多 8192，尺寸取整。模板比例可减少中间半径处的横向拉伸；圆环内外两侧仍有自然的弯曲比例差。

SVG 含 `artwork` 组和默认隐藏的 `guides` 组。在 artwork 内作图，导出时保持 guides 隐藏。PNG 模板没有文字或彩色辅助线。SVG 用于编辑，完成后按原画布尺寸导出 PNG 作为图片输入。

| `artwork.mode` | 映射规则 | 专辑时长变化时 |
| --- | --- | --- |
| `disc`，默认 | 圆盘构图，图片中心对应盘心，剩余环带可见 | 完整记录直径上的图片比例不变，裁切窗口改变 |
| `strip` | 从左到右顺时针绕一圈，接缝默认在 12 点；上边对应外缘，下边对应内缘 | 整张长条铺满新的环带 |

长条模式配置：

```toml
[artwork]
path = "my-strip.png"
mode = "strip"
rotation = 0.0
tone = "gray"
```

`rotation` 在两种模式下都是顺时针角度，长条模式下会移动接缝。`fit` 只影响圆盘模式；长条直接把完整宽度映射为 360°、完整高度映射为外缘到内缘，不先压成正方形。接缝是否连续取决于图片左右边缘的内容。

模板、预览和实际编码共用采样坐标。曲目、间隔、容量、文件预留或实际会话起点改变后应重新导出模板。若使用 `--session-start`，模板、预览与制作命令应提供相同值。

## 输出内容

```text
album/
  audio/01.wav …       按扇区对齐的音乐，PCM 保持原样
  session1.cue         第一会话的曲目与 pregap 描述
  session2.iso         第二会话的 2048 字节 Mode 1 负载，目录使用绝对盘 LBA
  session2.bin         对应的 2352 字节 RAW；含 MSF、EDC、P/Q ECC，未做物理扰码
  session2.toc         只描述第二会话的数据轨
  source.png           可选；圆盘为 1600×1600，长条保留宽高比、长边归一化至 4096
  preview.svg          独立可查看的布局与图片合成双视图
  design.png           设计合成图
  readback.png         从实际 ISO 读取绘图字节后的采样投影
  difference.png       设计/读回的灰度差异热图
  manifest.json        会话位置、参数、文件范围与所有制品的 SHA-256
```

这是**双会话制品包**，不是一个可交给任意软件烧录的单文件整盘镜像。普通 CUE 不能作为跨平台双会话描述；`session1.cue` 与 `session2.toc` 分别只描述一个会话。RAW 不含子通道、lead-in、lead-out 或 EFM 通道位。

`session2.iso` 是**带会话偏移的 ISO 数据流**，不是零起点的独立挂载镜像。文件和路径表中的 extent 是绝对盘 LBA，流偏移从第二会话 INDEX 01 开始。后端必须使用清单中的 `session_start`，不能把它直接作为一张零起点数据盘写入。

## CD Extra 布局与绘图

```text
盘内圈 → Session 1：CD-DA 音乐音轨
       → 第一会话 lead-out / 下一会话 lead-in / pregap 预算
       → Session 2：Mode 1 数据轨
          [ISO9660/Joliet 元数据 + 随盘文件 + 预留 | ARTWORK.BIN 绘图区]
       → 最终 lead-out（macOS 刻录由系统封盘时生成）
```

当前实现的是**音乐在前、数据在后的双会话 enhanced CD 离线布局**。尚未实现完整 Blue Book / CD Extra 的 CD-ROM XA、CDPLUS 内容约束及合规验证，也未做消费者播放器验收，不宣称完整标准合规。

绘图区作为 ISO 中真实分配的 `ARTWORK.BIN` 文件存在，避免图案依赖可能被工具清空的 padding。这个文件是编码负载，不是音乐，不应当作音轨播放。用户文件和目录的字节保持原样，图片只控制专用绘图区。

编码采用上游的 `0x0D / 0xAA` 二阶调制方案，按绝对 LBA 补偿 Mode 1 扰码及 CIRC 字节位置。RAW 输出补齐同步头、BCD MSF、EDC 和 P/Q ECC；物理扰码、完整 CIRC 和 EFM 仍由刻录机完成。灰度通过绝对逻辑字节上的确定性抖动表达；设计像素抖动与物理字节抖动明确分开。

## 容量、几何和会话起点

默认值：

| 参数 | 默认值 | 含义 |
| --- | --- | --- |
| `capacity_sectors` | 360000 | 标称 80 分钟的预算，不是介质探测值 |
| `inner_radius_mm` / `outer_radius_mm` | 25 / 58 | 未标定的记录区域半径 |
| `session_reserve_sectors` | 11400 | 首次会话转换的规划预留 |
| `data_reserve_sectors` | 10000 | 文件系统和随盘文件的固定预算，约 19.5 MiB 负载 |
| `intertrack_gap_sectors` | 0 | 仅相邻音乐轨之间的 gap |
| `end_margin_sectors` | 150 | 数据轨结束后的余量预算 |
| `velocity` / `angle` | 1200 / 0 | mm/s 线速度与顺时针起始角 |

这些字段放在 TOML 的 `[disc]` 中。文件与元数据超出数据预算时明确报错；不会截断文件或缩短图案。CD-DA 每秒 75 扇区，每扇区 2352 字节、588 个双声道采样帧；Mode 1 每扇区只有 2048 字节用户负载，不能直接按 WAV 文件字节数减“700 MB”。

默认第二会话起点为“音乐扇区 + 轨间间隔 + 会话预留”。如有设备实际给出的 NWA，可重新生成：

```sh
./target/release/discimage build album.toml --output measured --session-start 146475
```

清单区分 `planned` 与 `provided`，后者仅表示手动提供，不能证明已经过硬件探测。起点变化必须重新生成 ISO/RAW 和预览，不能只改清单里的数字。

几何采用恒螺距面积近似，`r²` 随绝对扇区数线性变化；角度结合线速度计算。未标定介质的定位和反射率没有保证，有限 CIRC 流的时序原点和边界仍需实盘验证。

## 预览、校验和维护

保留旋转、contain/cover、灰度/阈值/有序抖动、伽马、亮度、反相和 PNG/SVG 输出。设计图、编码负载读回、差异图分别输出；孔洞、盘外和未采到的像素透明。读回默认每扇区采样 32 字节，可用 `--samples 1..2048` 提高密度；它不模拟 parity、EFM 明暗或光学效果。详见 [预览说明](docs/preview.md)。

`verify` 校验每个制品的大小和 SHA-256、导出音频长度、ISO 内文件的范围与哈希，以及每个 RAW 扇区的地址头、EDC 和 ECC。另使用独立 ISO 读取器和独立 ECC 表进行交叉验证，记录见 [validation.md](docs/validation.md)。

保持一个 crate，使用职责明确的单词模块，不为每个类型拆文件。`art` 管图片采样，`codec` 管扇区编码，`iso` 管文件系统，`author` 串联制品；`preview` 负责图形输出。完整结构见 [architecture.md](docs/architecture.md)。

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

三平台 CI 已配置，尚未在远程执行。`devices` 分别使用 macOS DiscRecording、Windows CIM 和 Linux sysfs。Windows/Linux 当前只做设备发现与离线制作；刻录通过 `Recorder` 接口扩展。后续重点是实机 NWA、刻录、读回和盘面效果验收。

仓库：[shiftz300/discimage](https://github.com/shiftz300/discimage)。
