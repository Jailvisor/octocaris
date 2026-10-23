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

## 已完成：真实视频全屏回归

使用 PotPlayer 全屏播放视频（2880×1800 物理 / 200% DPI）：

| 状态 | 结果 |
|---|---|
| 全屏视频 | 日志 `fullscreen=true`（02:39:25Z）；`OctosWallpaperWindow visible=False`；图标可见 |
| 退出全屏 | 日志 `fullscreen=false`（02:46:49Z）；壁纸 `visible=True`、`prev=SHELLDLL_DefView`、`belowIcons=True`；L2D 壁纸重新渲染 |
| 恢复后桌面 | `E:\file\octos\m5-fullscreen-restored.png`（壁纸 + 桌面图标同时可见） |

z-order 说明：退出全屏后，WebView2 `Chrome_WidgetWin_1` 与 Progman 之间夹着若干隐藏的 IME/helper 窗口，所以 `nextIsProgman=False`，但沿 z-order 跳过隐藏窗口后 `reachesProgman=True`。`verify-desktop.ps1` 已改为区分 `nextIsProgman` 和 `reachesProgman`，SUMMARY 的 `chromeNextProgman=1` 表示没有可见窗口挡在桌面之上。

## 桌面状态矩阵（M5 复测）

`verify-desktop.ps1`，当前 200% DPI：

| 状态 | SUMMARY | 结果 |
|---|---|---|
| 主窗口前台 | `iconsVisible=True wallpaperBelowIcons=True wallpaperVisible=True mainMinimized=False chromeOverlays=1 chromeLayeredTransparent=1` | PASS |
| 主窗口最小化 | `wallpaperVisible=True mainMinimized=True chromeOverlays=1` | PASS |
| 显示桌面（ToggleDesktop） | `wallpaperVisible=True mainMinimized=True chromeOverlays=1` | PASS |

三种状态下壁纸都在 `SHELLDLL_DefView` 下方，Chrome 覆盖层保持 layered+transparent。

## 已完成：Explorer 重启回归

执行：`Stop-Process -Name explorer -Force`，Explorer 自动重启；等待 8 秒后运行 `verify-desktop.ps1`。

- 02:16:30.212Z 首次 reconcile 出现一次 `could not prepare wallpaper window ... 类已存在 (0x80070582)`（Explorer 重建期间的瞬态）。
- 02:16:30.738Z 下一 tick `desktop attach: Progman path`，壁纸控制器重新就绪。
- 02:16:32.660Z `TaskbarCreated`，托盘图标重新添加。
- 最终 `SUMMARY ... wallpaperVisible=True chromeNextProgman=1 chromeLayeredTransparent=1`，PASS。

## DPI 回归

| DPI | 证据 | 结果 |
|---|---|---|
| 100% | `E:\file\octos\m5-dpi-100.png`（用户截图，2000×1250） | UI 布局/字号/卡片正常，无裁切 |
| 150% | `E:\file\octos\m5-dpi-150.png`（用户截图，2000×1250；卡片/字号约 1.5×） | Library/详情/状态面板正常，无裁切 |
| 200% | `verify-desktop.ps1` + `m4-ui-*.png` | 桌面层级与壁纸渲染 PASS |

**说明**：100%/150% 目前是 UI 截图证据；桌面层级检查（壁纸在图标层下、Chrome 覆盖层透明）只在 200% 实机跑过。若需要完整矩阵，请在 100% 或 150% 下停留并告知，我再跑 `verify-desktop.ps1`。

## M5 结论

- 代码项全部完成：prefs 写盘硬化、ProcessFailed 恢复、真实全屏门控、Explorer 重启恢复、桌面矩阵、窗口位置。
- DPI：UI 在 100% / 150% / 200% 均有截图或实机证据；桌面层级检查在 200% 完成。
- **多显示器 / `redirect` / 热插拔**已完成基础链路实测，见 `docs/multimonitor.md`；每屏 localStorage 隔离待做。
- 100%/150% 下的桌面层级若要补齐，需要保持对应缩放后再跑一次 `verify-desktop.ps1`。

## 当前测试

- `octos-core` 31、`octos-protocol` 10、`octos-storage` 20，共 **61**。

## 闪烁/点击桌面刷新修复

现象：壁纸每隔一小段时间闪一下；左键单击桌面时更明显。

根因（两个）：

1. **watchdog 每 500ms 无条件重设窗口/合成**
   - `reconcile` 每个 tick 都调用 `SetWindowPos`、`SetBounds`、`DComp Commit`、`SetIsVisible(true)`。
   - 即使几何/尺寸/可见性没有变化，也会让分层窗口和 Composition surface 重新合成，表现为闪烁。
   - 修复：只在几何或 z-order 变化时 `SetWindowPos`；只在 client size 变化时 resize；只在实际可见性变化时 `SetIsVisible`/`ShowWindow`。这符合上游 `WatchdogProc`/`FixRenderingTarget`“只在需要时修复”的意图。
2. **Win11 shell 窗口被误判为全屏**
   - 点击桌面时 `XamlExplorerHostIslandWindow` 会短暂铺满主屏；之前 `ShellHandwritingCanvas` 也会。
   - 全屏检测把它们当成了全屏应用 → 隐藏壁纸 → 恢复，看起来就是“刷新”。
   - 修复：全屏检测排除 `XamlExplorerHost*` 和 `ShellHandwritingCanvas*`；这两类不是用户全屏应用。

实测：
- 模拟桌面左键单击：`fullscreen=true` 计数不再增加。
- `fullscreen-probe.ps1`：仍然正确隐藏/恢复；两屏桌面验证 PASS。
- 双屏 wallpaper/Chrome overlay 数量恢复为 2。

补充：多显示器下，两块壁纸都是 `SHELLDLL_DefView` 下方的兄弟窗口。旧检查要求每块壁纸的**直接 prev** 都是 DefView，第二块屏永远不满足，于是每 500ms 重新 `SetWindowPos`。现在 `z_order_below` 允许连续的 Octos wallpaper 链；启动后只修正一次，稳定后 10 秒空闲内 `reposition=0`、`resize=0`、`visibility=0`、`fullscreen=0`（调试日志实测）。

进一步：点击应用/桌面时 Explorer 会在 DefView 与壁纸之间插入辅助窗口。逐级 `GW_HWNDPREV` 检查仍会把它当成 z-order 错误，每点一次就重排一次。最终改为上游 `FixWallpaperOrder` 的算法：从 DefView 沿 `GW_HWNDNEXT` 向下查找壁纸，只要仍在链上就认为正确。实测连续 3 次“点击应用 → 点击桌面”，`reposition` 增量为 0（禁用鼠标转发时）。

最后一个根因：**`SendMouseInput` 会把壁纸 HWND 抬到 DefView 上方**。日志显示点击后 `prev=None, next=SHELLDLL_DefView`。已在壁纸窗口过程拦截 `WM_WINDOWPOSCHANGING`：任何请求把壁纸排到别处（包括 `HWND_TOP`）时，强制改回 `insert_after = SHELLDLL_DefView`。

验证：手动对壁纸 HWND 调 `SetWindowPos(HWND_TOP)`，日志 `keeping wallpaper ... below ... (requested HWND(0x0))`，随后 `verify-desktop` 仍为 `prev=SHELLDLL_DefView`、`belowIcons=True`、PASS。双屏点击复测暂缓：当前系统只剩 DISPLAY2，扩展屏掉线/休眠。

与上游对照：
- 上游 `SetWebViewVisibility` 用 `hidden` 标记避免重复 `put_IsVisible`；Rust 之前没有等价缓存，已补上。
- 上游全屏检测要求 `WS_OVERLAPPEDWINDOW`、无 owner、root、非 cloaked、覆盖工作区；Rust 版本更宽松，因此额外加了 Win11 shell 类名排除。没有直接照搬 `WS_OVERLAPPEDWINDOW`，是为了保留无边框全屏游戏/视频的检测。
