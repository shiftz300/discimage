# 时序与照片相位

[English](../timing.md) · [文档入口](../README.md) · [标定步骤](calibration.md)

## 每种测量能确定什么

| 测量 | 能精修什么 | 不能单独证明什么 |
| --- | --- | --- |
| 读取时序 | 螺旋几何、编码位置间的角度关系 | CD 相位原点、相对标签的朝向 |
| 三环照片 | 实际半径尺度，保持时序角度关系 | 角向相位 |
| 照片中的已知角向特征 | 剩余沿轨相位偏移 | 低残差不等于绝对实物精度 |

所以 DiscProbe 能修正角度映射。自动预设仍假定 108 个通道帧的读取原点和 58 mm 外缘半径；
照片精修分别补充尺度或相位测量。

## 指标与判断

每点按相位标准误加权。程序从实际样本重建几何协方差，不借几何先验制造精度，
并按超额残差扩大不确定度。RMS 只提示质量，不单独决定导出。

两轮几何差异按合并协方差检验，名义 99% 的二维卡方界限为 9.21034；
多轮时修正多重比较。全部配对一致才导出，选择传播置信带更小的一轮，范围限制在共同实际测量区间。
判据依赖线性化模型和近似独立噪声，不是物理准确度保证。

终端给出 RMS、chi2/dof、跨度标准不确定度、最大名义 99% 角度带、50 mm 处位移、
度/微秒抖动和复测一致性。相同时间抖动在高转速下对应更大角度，两个系统使用同一套计算。
[实测结果与手动预设比较](../validation.md#automatic-versus-accepted-manual-preset)。

回执不完整或改变、目标不一致、复制报告、无效误差、缓存读、转速异常、覆盖不足、
不可识别的协方差和整圈歧义仍会拒绝。共同模型偏差和盘面效果需要另外检查。
数学细节见[英文判据](../timing.md#uncertainty-and-repeatability)。

## 离线复用报告

```sh
discimage calibrate -o calibrations/review --name my-batch \
  --bundle calibrations/my-batch/record \
  --timing calibrations/my-batch/timing-previous-001/summary.json \
  --timing calibrations/my-batch/timing/summary.json
```

每份摘要旁都要保留对应 `targets.csv` 和完整 `status.json`。
新采集要求已封盘、单会话、单 Mode 1 数据轨；默认请求 1411 kB/s，每点 32 对读取。
`calibrate probe --check` 只检查元数据。

## 照片相位精修

准备可匹配的角向特征照片、生成那张盘面的准确刻录包，以及外部提取的相位对应数据。
每条曲线至少 12 个样本，使用多条曲线做留出验证。这不是三环径向 `points.json`。

```sh
discimage calibrate phase --timing measurement/summary.json \
  --bundle record --points phase.json --evidence photo.jpg --output comparison

discimage calibrate import album.toml \
  --timing measurement/summary.json --phase comparison/report.json \
  --bundle record --output candidate
```

输出目录必须尚不存在，专辑使用照片生成时的原始几何。
查看 `comparison/index.html`，再用 `candidate/candidate.toml` 做 preview/build；网页 **时序对比** 页也可查看结果。
照片身份和留出检查必须通过。结果是独立候选，不自动验收或覆盖生产预设。

<details>
<summary>相位数据格式</summary>

```json
{
  "format": "discimage-phase-v1",
  "bundle_sha256": "准确 manifest.toml 的 SHA256",
  "evidence_sha256": "照片或校正图像的 SHA256",
  "convention": "photo-minus-design-clockwise",
  "samples": [{"curve": "edge-a", "sector": 180000.5, "phase_deg": 12.8}]
}
```

每条曲线补足样本。`sector` 是可带小数的编码坐标，不是文件偏移或照片半径。
`phase_deg` 是顺时针、连续展开的“照片减设计”角度；同一曲线跟踪同一特征。
允许不同曲线各有整体旋转。检查特征对应和图像校正敏感性，AI 重复估计不等于独立照片。

</details>
