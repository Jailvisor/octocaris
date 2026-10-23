# M2 Acceptance Report — Wallpaper Core

日期：2026-10-22
状态：**M2 核心验收通过**，带已知缺口（多显示器、真实游戏全屏、ProcessFailed）。下一阶段：M3 协议与 Mod API 兼容。

## 1. 验收范围

- 每显示器一个壁纸窗口，挂到 Progman/WorkerW，位于桌面图标下方。
- CompositionController + DirectComposition，透明、点击穿透。
- 低层鼠标钩子 → `SendMouseInput`。
- 全屏暂停/恢复、Explorer 重启恢复、z-order 与 Chrome 覆盖层处理。
- 内置 gradient mod、Clear/Show。
- 桌面图标可见性 API。
- 只读桌面诊断脚本与三种状态回归。

## 2. 验收结果

| 验收项 | 结果 | 证据 |
|---|---|---|
| WSL 构建 x64 Windows exe | 通过 | `cargo xtask deploy`；`octos.exe` |
| Win11 桌面挂载 | 通过 | 日志 `desktop attach: Progman path` |
| 壁纸在图标下方 | 通过 | `OctosWallpaperWindow prev=SHELLDLL_DefView`、`belowIcons=True` |
| 桌面图标可见/可点 | 通过 | `SysListView32 visible=True`；截图 `m2-icons.png`、`desktop-alpha-fixed.png` |
| 桌面右键菜单 | 通过 | `m2-rightclick.png` |
| 鼠标输入转发 | 通过 | 日志 `wallpaper page: {"type":"click",...}` |
| Clear wallpaper | 通过 | 日志 `clear-wallpaper`；`visible=False`、`chromeOverlays=0` |
| Show gradient | 通过 | 日志 `show-wallpaper`；壁纸/Chrome 宿主恢复 |
| 全屏暂停/恢复 | 通过 | `fullscreen=true` → `visible=False`；关闭后 `fullscreen=false` → `visible=True` |
| Explorer 重启恢复 | 通过 | 新 Progman/SHELLDLL_DefView；Chrome 宿主重建；tray `TaskbarCreated` 恢复；截图 `explorer-restart-fixed.png` |
| 主窗口前台/最小化/显示桌面 | 通过 | 三种状态 `verify-desktop.ps1` 均 PASS |
| 桌面图标 API | 通过 | Settings 开关；Rust test `desktop_icon_visibility_round_trip ... ok` |
| 诊断脚本 | 通过 | `scripts/verify-desktop.ps1`，检查壁纸 HWND + `msedgewebview2` 宿主 |
| 质量门禁 | 通过 | fmt、clippy `-D warnings`、Linux 单测 |

## 3. 关键修复记录

1. **挂载顺序**：Win11 必须 Progman + `SHELLDLL_DefView` first，WorkerW 只做回退。
2. **全屏误判**：排除 `Chrome_WidgetWin_*`、`WebView2*`、`*OverlayWindow`，加 DWM cloaked。
3. **Chrome 宿主透明**：只加 `WS_EX_LAYERED|WS_EX_TRANSPARENT`；**禁止** `SetLayeredWindowAttributes(alpha=255)`。
4. **z-order**：校验 `parent`/`insert_after` 句柄 + `GW_HWNDPREV`。
5. **Explorer recovery**：挂载失败时不创建 controller；窗口定位后重新同步 controller bounds。
6. **诊断盲区**：必须同时看壁纸 HWND 和 `msedgewebview2` 顶层宿主；`wallpaperVisible && chromeOverlays==0` 判 WARN。
7. **桌面图标 API**：只做临时 ShowWindow，不写 `HideIcons`（与 upstream 一致）。

详见 `docs/m2-wallpaper.md`、`docs/upstream-map.md`。

## 4. 已知缺口 / 延后

- **多显示器**：当前环境只有 `\\.\DISPLAY1`；registry/定位代码存在但未实测。
- **真实游戏/视频全屏**：用边框全屏测试窗口验证过；建议再用真实应用回归。
- **ProcessFailed**：WebView2 renderer 崩溃恢复未实现（只有窗口/reconcile 级恢复）。
- **按显示器设置/清除**：后端只有 `show_all`/`clear_all`，UI 没有按显示器选择。
- **壁纸 JS API**：`System`/`UserOptions`/`MonitorBridge` 还没接；`MediaController`/音频还是 M6。
- **prefs/library/CLI**：M3/M4。
- **托盘菜单交互**：添加/移除/退出已验证，菜单项鼠标交互未自动化。
- **桌面图标 API 持久化**：按 upstream 语义是临时 toggle，不写用户设置。

## 5. 退出标准（进入 M3）

M2 不再有阻塞项。M3 以“冻结协议 + 旧 mod 可运行”为目标：

1. `sdk/` 原样保留旧 `octos.min.js` 作为兼容基准。
2. `octos-core` 实现 manifest/catalog/options/session/hooks。
3. `octos-protocol` 实现 mod request/command/subscribe/event 类型与 `WallpaperHost` 分发。
4. 壁纸侧 `System`/`UserOptions`/`MonitorBridge` 全量；媒体/音频协议桩。
5. 选项由 Rust 直接回答；hooks 顺序/字符串化保持旧语义。
6. 用旧 `octos.min.js` + 一致性 mod 验收。

M3 详细计划见 `docs/m3-plan.md`。
