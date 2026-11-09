# Windows

[Guides · 使用指南](README.md) · [Recording · 刻录](zhcn/recording.md) · [Calibration · 标定](zhcn/calibration.md)

Use the release executable; no Rust or Python installation is required.
Windows recording is experimental. [Hardware scope](validation.md#hardware-scope).
使用发布版即可；Windows 刻录仍属实验性，已测范围见上方链接。

## Select the drive · 选择光驱

```powershell
discimage devices
discimage select DRIVE_ID
discimage select
```

Use the displayed hardware ID. A drive letter such as `E:` also works locally.
The drive needs a Windows letter. Close other applications using the disc if
exclusive access fails. Apple SuperDrive loading initialization is handled by
its separately installed device driver; DiscImage does not install drivers.
使用实际硬件 ID 或当前盘符。独占访问失败时关闭占用盘片的程序。
Apple SuperDrive 的加载初始化由单独安装的设备驱动处理。

## Same workflow · 同一流程

`build`, `burn`, `erase` and `calibrate` use the same CLI as macOS.
Calibration uses the same uncertainty and repeatability assessment, with no
Windows-specific relaxation. Higher RMS alone does not reject a noisy measurement.
命令与 macOS 相同，标定判据也相同；较高 RMS 不会单独导致失败。

## Optional trace · 按需诊断

```powershell
$env:DISCIMAGE_TRACE_MMC = '1'
discimage devices --json
Remove-Item Env:DISCIMAGE_TRACE_MMC
```

Logs include command, HRESULT and sense information. Timing uses exclusive
IMAPI2 access and QPC timestamps around synchronous native commands. Reservations
and tray prevention are released on exit, including errors.
日志包含命令和错误信息；退出或错误时释放保留和托盘锁定。
