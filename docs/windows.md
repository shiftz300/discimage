# Windows 光驱联调

Windows 通过 IMAPI2 管理刻录保留并发送 MMC 命令，完成设备识别、CD-R/CD-RW 写入、
CD-RW 擦除和单会话标靶；discprobe 同样保留独占 IMAPI2 会话，并用 QPC 在原生同步命令边界计时。音频使用 SAO，保留工程中的零间隔或自定义间隔，
可写 ASCII CD-Text；数据使用 2048 字节 Mode 1 TAO，按实际 NWA 重新生成。

Windows 刻录目前处于实机联调阶段。苹果 SuperDrive / GX40N 已在同一张 CD-RW 上完成
六轨双会话刻录、PCM 与全量数据回读、CD-Text 内容/CRC 和封盘验证；XD05 的写入问题尚未解决，本次 CD 流程
不再使用它。完整擦除及单会话标靶写入、封盘和全量读回已通过。低速默认下的完整时序采集
得到 0.733° RMS、0.456° 跨范围不确定度，优于最高档两轮的 1.770°/2.750° RMS；
按上游规则可用但有噪声，仍超过本项目 0.3° RMS 导入门槛，未应用为预设。见 [validation.md](validation.md)。
源码构建使用 Rust MSVC 工具链和包含 C++ 编译器的 Visual Studio / Windows SDK。

## 识别和预检

在 PowerShell 中运行本地构建（发布版将路径替换为 `discimage.exe`）：

```powershell
cargo build --locked
New-Item -ItemType Directory -Force calibrations/work | Out-Null
.\target\debug\discimage.exe devices
.\target\debug\discimage.exe devices --json
```

使用 `devices` 中的硬件 ID、唯一型号或盘符选择设备。XD05 示例：

```powershell
.\target\debug\discimage.exe select BDR-XD05
.\target\debug\discimage.exe erase --dry-run
```

先确认序列号、介质类型、空白状态和容量。排他锁失败时，关闭使用该盘的播放器、
资源管理器窗口和其他刻录程序，再重试。设备必须分配 Windows 盘符。
访问失败会保留错误，不会更改系统权限。无盘时 `media` 为 `null`。
原装 Apple USB SuperDrive 在未初始化时可能不吸盘。请由单独安装的设备驱动处理
加载初始化；程序不内置 Apple 专用唤醒命令或安装驱动。加载后可能显示真实的
`HL-DT-ST GX40N` 机芯；应使用此时的硬件 ID 保存默认值，临时桥接身份不生成硬件 ID。
显式 `--speed N` 会在写入意图保存前核对实际介质的支持速度；自动速度使用查询到的
最高速度。擦除也先恢复 TAO 参数并选择最高支持速度，与 macOS 默认行为对齐。
Windows 与 macOS 测量统一请求 1411 kB/s，每点固定 32 对采样，并检查缓存与转速稳定性。
实验性的快慢档、自动切速和可配置采样已撤销。刻录和擦除仍默认最高支持速度。

## 复用 CD-RW

```powershell
.\target\debug\discimage.exe erase
```

核对设备后输入 `ERASE`，将清空盘片。默认完整擦除；`--quick` 只重置元数据，
旧图案可能仍可见。程序等待光驱完成并确认空白状态与容量；期间保持连接。
CD-R 不可擦除。

## 单会话标靶与 discprobe 测量

使用空白 CD-R 或 CD-RW，每次使用新的输出目录：

```powershell
.\target\debug\discimage.exe calibrate record -o calibrations/work/windows-target --dry-run
.\target\debug\discimage.exe calibrate record -o calibrations/work/windows-target
.\target\debug\discimage.exe calibrate probe --check
.\target\debug\discimage.exe calibrate probe -o calibrations/work/windows-measurement
```

`record` 会按真实容量制作三环标靶、写入单个 Mode 1 数据轨、封盘并全量读回校验。
失败时保留 `pending.json`；只有最终轨道状态、范围和内容校验通过，才生成
`single.json`。`--dry-run` 只验证选择和保留，不生成文件或写盘。

`probe` 使用 discprobe 的缓存/FUA 探测、主轴周期估计、逐级几何拟合和参考漂移修正。
它仅接受已封盘的单会话、单数据轨 CD。读取范围和每次传输均受已确认轨道边界约束。
保留测量目录的 `status.json`、`summary.json` 和 `targets.csv`。
缓存、USB 延迟或 CLV 转速可能导致测量失败；不得把失败或中断结果当作有效标定。

单会话标靶也可用于照片径向拟合：

```powershell
.\target\debug\discimage.exe calibrate guide -o calibrations/work/windows-fit --bundle calibrations/work/windows-target
```

按引导拍摄照片和取点。时序估计、照片拟合与物理图案验证各自提供证据，
测量不会直接覆盖已有预设。相位比较与候选导入见 [timing.md](timing.md)。

## 双会话刻录

复用同一张 CD-RW 前先保存标靶、测量和照片，再显式擦除。

```powershell
.\target\debug\discimage.exe calibrate target -o calibrations/work/windows-extra-target
.\target\debug\discimage.exe build calibrations/work/windows-extra-target/target.toml -o calibrations/work/windows-extra
.\target\debug\discimage.exe burn calibrations/work/windows-extra
.\target\debug\discimage.exe devices
```

音频写入后逐字节读回。通过后读取实际 NWA，编码并写入第二会话、封盘、全量校验。
最终必须得到两会话及正确音轨/数据轨布局。中断写入的 `music_writing` 或
`data_writing` 状态不会自动重试；检查 `manifest.toml` 中的状态和错误。
软件测试通过与实盘通过分别记录；仍需观察盘面并试听音频。

## 原始命令诊断

仅在联调时开启跟踪；日志会包含 CDB、少量传输数据、HRESULT 和原始 sense：

```powershell
$env:DISCIMAGE_TRACE_MMC = '1'
.\target\debug\discimage.exe devices --json
Remove-Item Env:DISCIMAGE_TRACE_MMC
```

开启时设备检查还会读取当前/默认写参数、ATIP 和写速描述符，不改变写模式。
测量入口只接收预定义的有界命令，不发送盘面写入；读数、读速设置及托盘防移除均在独占会话内完成。
退出或错误时恢复托盘状态、媒体通知并释放保留。时间戳不包含缓冲区分配及向 Rust 复制读数的耗时。
