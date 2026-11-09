# 图案与预览

[English](../preview.md) · [文档入口](../README.md)

## 1. 导出专辑模板

```sh
discimage template album.toml -o templates
```

实际 WAV 时长和盘片预设决定可绘圆环，导出时还不需要图案文件。
输出目录必须尚不存在。将 `templates/layout.json` 导入[网页编辑器](https://discimage.kasetry.cn/)。

## 2. 选择图片模式

| 模式 | 映射方式 |
| --- | --- |
| `disc` | 方形画布覆盖记录区直径，不可绘区域由遮罩保护 |
| `strip` | 宽度顺时针绕一圈，上边是外缘、下边是内缘，接缝默认在 12 点 |

输入为 PNG/JPEG，每边最多 8192 像素。`rotation` 顺时针旋转。
`fit = "contain"` / `"cover"` 只影响圆盘模式；长条使用整张图片，首尾接缝需在设计中处理。
更换曲目或盘片后重新导出模板并检查构图。

网页导出 ZIP 后，把 **`artwork.png`** 放到专辑 TOML 旁，将 `artwork.toml` 中的 `[artwork]` 段替换回专辑，
保留曲目和预设。`preview.png` 只是参考图。需要再次编辑网页设计时，另存 `discimage-design.json`。

## 3. 放大检查

```sh
discimage preview album.toml
discimage preview album.toml -o design.png --size 4096
discimage build album.toml
```

第一条显示曲目时间和可绘空间；PNG 输出文件必须尚不存在。
用浏览器打开成品的 `preview.svg` 检查分区和构图。
预览分辨率支持 256–4096；调大不会增加光盘物理编码密度。

| 灰度处理 | 效果 |
| --- | --- |
| `gray` | 保留灰度，编码时确定性半色调 |
| `threshold` | 可调阈值的二值边缘，适合明确的签名轮廓 |
| `ordered` | 有序抖动和确定性物理采样 |

优先使用清晰原图，避免放大截图。灰度设置不预测实物颜色或对比度。

## 可选编码负载预览

```sh
discimage build album.toml --iso
discimage verify output/album -o encoded.png --size 1600 --samples 128
```

它投影本地 ISO/RAW 中可控的绘图负载，不读取实盘。
`--samples` 为每扇区 1–2048 字节，越高越慢；透明空洞可能是未采样区域。
该预览不模拟全部校验/通道符号、反射率或光驱写入策略。
