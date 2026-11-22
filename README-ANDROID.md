# RustCal Android 构建说明

本分支以原始 RustCal 桌面项目为基础，初步完成Android 版构建，在手机上安装并实测同步正常。

## 已做的适配

- 增加 `cdylib`，Android 入口为 `android_main(AndroidApp)`；桌面端仍由 `src/main.rs` 启动。
- Android 使用 `AndroidApp::internal_data_path()`，数据放在应用私有目录下的 `rustcal/`
- SQLite、主题配置、同步配置共用平台路径服务。
- `rfd` 仅作为非 Android 依赖，桌面端文件对话框行为保留。
- Android 提供私有 `import/`、`export/` 文件交换目录，支持 ICS 导入/导出和 JSON 备份/恢复。
- 增加 Android 系统字体路径探测；egui 内置默认字体仍作为回退。
- 窄屏时折叠日历侧栏，并让顶部导航和状态栏自动换行。

## Android 文件导入导出说明

当前这个源码包没有伪装成已完成 SAF 系统文件选择器：Android 端先使用应用私有目录作为可靠的跨版本文件交换方案。

应用启动后会创建：

```text
<internal_data_path>/rustcal/
├── rustcal.db
├── config/
├── import/
└── export/
```

- 导入 ICS：放入 `import/rustcal_import.ics`，然后在应用设置中点“导入 ICS 文件”。
- 恢复 JSON：放入 `import/rustcal_backup.json`，然后点“从 JSON 备份恢复”。
- 导出文件：在应用中点导出，文件写入 `export/`。

可用 adb 进行文件交换（包名为 `com.rustcal.calendar`）：

```powershell
# 推送待导入文件到设备临时目录
adb push .\my-calendar.ics /data/local/tmp/rustcal_import.ics

# 复制到 RustCal 私有导入目录
adb shell run-as com.rustcal.calendar mkdir -p files/rustcal/import
adb shell run-as com.rustcal.calendar cp /data/local/tmp/rustcal_import.ics files/rustcal/import/rustcal_import.ics

# 从应用私有导出目录取回 ICS
adb exec-out run-as com.rustcal.calendar cat files/rustcal/export/rustcal_export.ics > .\rustcal_export.ics

# 从应用私有导出目录取回 JSON
adb exec-out run-as com.rustcal.calendar cat files/rustcal/export/rustcal_backup.json > .\rustcal_backup.json
```

注：不同 Android 版本/设备的 `run-as` 路径布局可能略有差异；`AndroidApp::internal_data_path()` 返回的目录是本程序的权威路径。如果设备上 `run-as` 的相对路径不匹配，可先运行：

```powershell
adb shell run-as com.rustcal.calendar pwd
adb shell run-as com.rustcal.calendar ls -R files
```

### 当前限制 / 后续工作

1. Android 文件选择暂时采用私有目录交换，不是系统 SAF picker；后续可增加原生 `ACTION_OPEN_DOCUMENT` / `ACTION_CREATE_DOCUMENT` 桥接。
2. Android 系统字体文件名因厂商/Android 版本而异。若设备没有候选字体，会回退到内置字体，中文覆盖可能不足。
3. 本次保留原有同步实现；CalDAV/WebDAV 网络同步正常，同步密码不写入配置文件。
4. 中文输入和系统剪贴板功能未实现，目前使用 rustcal 管理日程并同步后，再用手机版日程同步即可，基本可用，后面有时间再解决。
