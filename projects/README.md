# Projects

每张专辑放一个子目录。此处的本地项目、音频、图案和刻录输出默认由 Git 忽略。

从仓库根目录复制模板：

```sh
cp -R projects/template projects/my-album
mkdir -p projects/my-album/audio
```

放入 44.1 kHz、16 位、双声道 PCM WAV，按顺序编辑 `album.toml` 的
`[[tracks]]`。路径以 TOML 所在目录为基准。加图后取消注释 `[artwork]`。已有标定时可在专辑顶层[引用预设](../calibrations/README.md)。

```sh
mkdir -p projects/my-album/output
discimage plan projects/my-album/album.toml
discimage template projects/my-album/album.toml --output projects/my-album/output/templates
discimage preview projects/my-album/album.toml --output projects/my-album/output/preview.svg
```

把模板目录里的 `layout.json` 导入网页编辑器。离线镜像和刻录作业也放在专辑自己的 `output/` 下，每次使用新的子目录。
[完整参数示例](../examples/album.toml) · [标定配置](../calibrations/README.md) · [刻录流程](../docs/recording.md)
