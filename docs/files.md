# Files · 文件结构

[Guides · 使用指南](README.md)

Keep editable inputs in the album, generated packages in its `output/`, and one
preset per medium/batch in `calibrations/`.
原始素材放专辑内，成品放专辑的 `output/`，不同盘型或批次分别保存预设。

```text
projects/my-album/
  audio/01.wav
  artwork.png
  cdrw.toml
  cdr.toml
  output/cdrw/
    01.wav                  # frozen PCM / 冻结音频
    artwork.png             # frozen image / 冻结图案
    manifest.toml           # settings, hashes, journal / 参数、校验、状态
    preview.svg
    session2.iso            # generated on export or burn / 导出或刻录时生成
calibrations/my-batch/
  disc.toml                 # album preset / 专辑引用的预设
  record/                   # target ISO and receipt / 标靶与回执
  timing/                   # latest measurement / 最近一轮测量
  timing-previous-001/      # retained independent round / 保留的另一轮
  mark.json
  photo.jpg                 # optional / 可选
  points.json               # optional / 可选
```

`build cdrw.toml` defaults to `output/cdrw/` beside that TOML. Files inside the
recording package are flat. Edit the album and rebuild; do not hand-edit the package.
`build` 默认按配置文件名输出平铺成品；修改专辑配置后重建，不手改成品。

| Repository path / 仓库目录 | Purpose / 用途 |
| --- | --- |
| [projects/](../projects/README.md) | Album starter and private projects / 专辑模板与本地工程 |
| [calibrations/](../calibrations/README.md) | Media presets / 盘片预设 |
| [examples/album.toml](../examples/album.toml) | Full configuration reference / 完整参数 |
| [src/](../src/) · [native/](../native/) | Shared implementation and native adapters / 核心与平台接口 |
| [tests/](../tests/) | Offline and recorder tests / 离线与模拟光驱测试 |
| [.github/workflows/](../.github/workflows/) | Checks and release packaging / 检查与发布 |

Private albums, generated media and raw calibration evidence are Git-ignored.
Retain both timing rounds and the target bundle for subsequent photo refinement.
个人专辑、生成物和原始证据默认不入 Git；后续照片精修需要保留标靶和两轮报告。
