# 标定一批盘

[English](../calibration.md) · [文档入口](../README.md)

准备一张同批空白 CD-R 或 CD-RW。CD-R 标靶占用这张盘；CD-RW 保存证据后可擦除复用。
不同盘型、不同批次分别保存预设。macOS/Windows 可直接刻靶和测量，照片拟合可以离线完成。

## 1. 运行标定

```sh
discimage devices
discimage select DRIVE_ID
discimage select
discimage calibrate -o calibrations/my-batch
```

`DRIVE_ID` 换成列表里的硬件 ID。核对保留的光驱和空白盘，输入 `y`。
已写过的 CD-RW 先[显式擦除](recording.md#复用-cd-rw)。省略速度参数即可自动选速。

程序只刻一次单会话标靶并回读，然后独立读取两轮，检查不确定度和一致性，导出 `disc.toml`。
等终端显示 `Preset saved to .../disc.toml`。实测 GX40N 两轮约 15 分钟，其他光驱可能不同。
RMS 只提示质量，不是固定成功线。

中断后重跑同一条命令继续。完成的标靶不重刻；不确定的写入会停止。
复测不一致会保留旧预设和全部报告。主动增加一轮只读测量时才加 `--remeasure`。

## 2. 用到专辑里

在 `projects/my-album/cdrw.toml` 最上方、任何表之前添加：

```toml
preset = { file = "../../calibrations/my-batch/disc.toml", name = "my-batch" }
```

路径相对专辑 TOML，名称默认等于批次目录名。`preset` 与 `[disc]` 二选一。
改预设后重新导出模板并 build。首张成品检查盘面对齐；
[复测一致还不等于已达到手动预设的实物效果](../validation.md#automatic-versus-accepted-manual-preset)。

## 3. 可选拍照精修

时序标定已经估计螺旋角度映射。三环照片补充实际半径尺度，保持该角度映射。
使用同一张标靶，保留标靶文件与两轮完整报告。

1. 正对整张 120 mm CD 的记录面拍照，包含实体外缘和真实中心孔，避开遮住带边缘的反光。
   保存为批次目录的 `photo.jpg`。JPEG/PNG，最长边 ≤8192 像素，整盘直径 ≥360 像素。
2. 在网页 **标定取点** 页导入同批 `mark.json` 和照片，按顺序点 14 点：

   | 编号 | 点哪里 |
   | --- | --- |
   | 1–4 | 实体外缘：上、右、下、左 |
   | 5–8 | 真实中心孔边缘：上、右、下、左 |
   | 9–14 | 从内向外，第 1 带内/外缘、第 2 带内/外缘、第 3 带内/外缘 |

   最后六点沿**同一条射线**。放大看清，点错用撤销。
3. 导出 `points.json` 到批次目录，再跑同一条 `calibrate` 命令。
   原文件齐全时不需要光驱、重测或重刻。

成功后更新 `disc.toml`。外缘/孔检查失败就重拍或改点；顺序/残差失败先核对带边缘和标靶身份。
CLI 不自动矫正倾斜或镜头畸变。拟合同时缩放半径和速度，保持角度关系。
照片与时序模型的径向不一致超过 0.3 mm RMS 时停止更新；这是径向检查，不是时序 RMS 门槛。

### 中文 AI 提示词

<details>
<summary>展开提示词，辅助定位取点</summary>

提供原照片和 `mark.json`，把 WIDTH、HEIGHT 换成测量图像当前朝向下的像素尺寸。

```text
请测量这张 DiscImage 三带 CD 标靶照片，尺寸为 WIDTH × HEIGHT 像素。
在原照片上取点，左上为原点，x 向右、y 向下。
rim：实体外缘的上、右、下、左，各一点。
hole：真实中心孔边缘的上、右、下、左，各一点。
edges：沿一条清晰射线从内向外，三条带各取内缘和外缘，共六点。
不要把印刷圈、夹持区边界或反光当作实体外缘或孔边缘。
输出 points.json，只包含 bundle_sha256（原样复制 mark.json）、
width、height、rim（4 对 [x,y]）、hole（4 对）、edges（6 对）。
另给带编号的叠加图供核对，不改变原照片内容。
不确定的说明放在 JSON 外。边界不清就指出，不要猜坐标或哈希。
若看到的是缩略图或裁切图，先说明到测量图像的坐标变换；
变换未知时不要声称给出了原图坐标。
```

核对每个点，将文件放进批次目录，再跑标定命令。也可参考 AI 的定位自己在网页点完再导出。
网页自动换算显示缩放；AI 缩略图坐标需要另行换算。程序不内置 AI 服务。
最终预设是 TOML，JSON 只用于交换测量点。

</details>

## 仍有角向弯曲时

三环没有角向参照。带有已知文字或其他角向特征的照片可做单独的
[照片相位精修](timing.md#照片相位精修)，先导出独立候选，不自动覆盖引导式预设。
相对印刷标签的绝对朝向也没有被自动测量。

## 要保留什么

`disc.toml` 是专辑预设。后续精修还需要 `record/`、`timing/`、另一轮
`timing-previous-NNN/`，以及可选的 `photo.jpg`、`points.json`。
`mark.json` 是取点时的标靶身份。[文件结构](../files.md)。
