# M5 Hardening — 进度

日期：2026-10-23
起点：M4 完成（commit `1646050`）。

## 已完成：prefs 写盘 debounce + `.bak`

- `octos-storage::PrefsWriter`：后台线程串行写盘，UI 线程只提交快照；连续变更只保留最新快照，静默 **250ms** 后写入。
- 退出时 `flush_preferences`：提交最终快照、等待 writer 写完并 `join`，再关闭壁纸。
- `write_atomic`：
  - 临时文件名带进程 id + 原子计数，避免并发冲突；
  - Windows 使用 `ReplaceFileW(..., REPLACEFILE_IGNORE_MERGE_ERRORS)`，自动保留 `preferences.json.bak`；
  - 失败时回退到 best-effort copy + `MoveFileExW(REPLACE_EXISTING|WRITE_THROUGH)`；
  - 非 Windows 测试路径先 copy 再 rename，语义一致。
- 单测：`writer_flushes_the_newest_snapshot`、`writer_writes_after_the_debounce_window`、`second_write_keeps_the_previous_file_as_bak`、`first_write_has_no_backup`。

## 实机验证

环境：`OCTOS_DATA_DIR=E:\file\octos\data`，`OCTOS_LOG=debug`。

1. UI 切换一次“全屏时暂停”：`set-setting` 01:33:30.571Z，`preferences written` 01:33:30.868Z（约 297ms 后）。
2. 连续快速切换 6 次：只产生 **1 次** 写盘（01:34:11.306Z）。
3. 当前 `preferences.json` 的 `pauseFullscreen=true`；`preferences.json.bak` 保存上一次的 `false`。
4. 通过托盘 Quit（`WM_COMMAND` id=2）退出：01:34:47.242Z 再写一次最终快照，进程正常结束。
5. `data` 根目录无 `.tmp-*` 残留。

## 已完成：ProcessFailed renderer 恢复

- 主 UI：注册 `ProcessFailed`，记录 kind 后对主 WebView 调 `Reload()`。
- 壁纸：`ProcessFailed` 只给槽位设置 `recreate_after = now + 1s`；watchdog 到期后关闭旧 controller 并重新创建，避免在 WebView2 事件回调里重入销毁。
- controller 重建失败时同样设 1s 重试，不会每 500ms 刷屏。
- 兼容性：主 UI 与壁纸使用同一环境；renderer 退出后 WebView2 会自动拉起新 renderer，再重建 controller。

实机验证（杀 `msedgewebview2.exe` renderer）：

| 目标 | 结果 |
|---|---|
| 主 UI renderer（PID 9456） | 日志 `main WebView2 process failed (Some(COREWEBVIEW2_PROCESS_FAILED_KIND(1))); reloading`，随后 UI 重新发 `get-library` |
| 壁纸 renderer（PID 21448） | 日志 `wallpaper process failed for \\.\DISPLAY1 (kind=1)` → 1 秒后 `recreating wallpaper WebView` → `wallpaper controller ready` |
| 恢复后桌面 | `verify-desktop.ps1` PASS：图标可见、壁纸在图标层下、Chrome 覆盖层 layered+transparent |

## 已完成：全屏回归（边框全屏窗口）

`scripts/fullscreen-probe.ps1`（2880×1800 物理 / 200% DPI）：

- 全屏窗口打开：`OctosWallpaperWindow visible=False`，日志 `monitor \\.\DISPLAY1 fullscreen=true`。
- 关闭后：`visible=True`，日志 `fullscreen=false`。
- 结论：全屏门控与恢复通过。

## 桌面状态矩阵（M5 复测）

`verify-desktop.ps1`，当前 200% DPI：

| 状态 | SUMMARY | 结果 |
|---|---|---|
| 主窗口前台 | `iconsVisible=True wallpaperBelowIcons=True wallpaperVisible=True mainMinimized=False chromeOverlays=1 chromeLayeredTransparent=1` | PASS |
| 主窗口最小化 | `wallpaperVisible=True mainMinimized=True chromeOverlays=1` | PASS |
| 显示桌面（ToggleDesktop） | `wallpaperVisible=True mainMinimized=True chromeOverlays=1` | PASS |

三种状态下壁纸都在 `SHELLDLL_DefView` 下方，Chrome 覆盖层保持 layered+transparent。

## 剩余 M5 项（需要人工/环境配合）

1. **Explorer 重启**：需要用户同意后执行（会短暂重建桌面）。
2. **DPI 100% / 150%**：需要用户切换 Windows 缩放；当前环境为 200%，已覆盖 200%。
3. **真实游戏/视频全屏**：需要启动真实应用；边框全屏窗口已通过。
4. **多显示器 / `redirect` / 热插拔**：按用户决定另说。

## 当前测试

- `octos-core` 31、`octos-protocol` 10、`octos-storage` 20，共 **61**。
