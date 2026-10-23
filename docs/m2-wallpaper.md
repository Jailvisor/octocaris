# M2 Wallpaper Core — 阶段报告（暂停点）

日期：2026-10-22
状态：**核心垂直切片可用，M2 尚未完成验收**，按用户要求暂停并记录问题。

## 结论先行

壁纸引擎**不是全屏应用**。Octos 原版的做法是：

- 每个显示器一个 `WS_POPUP` 窗口，样式 `WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE`。
- 挂到桌面层：Win11 是 `SetParent(Progman)` 并插到 `SHELLDLL_DefView` 下方；Win10 是挂到正确的 `WorkerW`。
- 用 `ICoreWebView2CompositionController` + DirectComposition 渲染，背景透明。
- 鼠标由低级钩子转发进 CompositionController；`CallNextHookEx` 保证桌面图标/右键仍然可用。

我们不需要换成“全屏应用”方案，也不需要另找一套壁纸软件的做法。之前的“像全屏应用”是**挂载层级和全屏误判**导致的 bug。

> **`octos-spike` 已退役**：它是 M0 工具链/Composition 验证程序，使用 WorkerW-first 挂载，没有 Chrome 覆盖层穿透、全屏/可见性门控、多显示器。Windows 产品验收只用 `octos.exe`。差异表见下。

## Spike vs production

| 项 | M0 spike | M2 production |
|---|---|---|
| 挂载顺序 | WorkerW first，Progman 回退 | Progman + `SHELLDLL_DefView` first，WorkerW 回退 |
| z-order | WorkerW 路径 `SWP_NOZORDER` | 记住 parent/insert_after，watchdog 维护 |
| Chrome 覆盖层 | 未处理，可能吞右键 | `WS_EX_LAYERED|WS_EX_TRANSPARENT` + `SWP_FRAMECHANGED` |
| 全屏 | 无 | DWM cloaked + 类名排除 + 暂停/恢复 |
| 显示器 | 单主屏 | `EnumDisplayMonitors` + 设备名 registry |
| 可见性 | 永远显示 | global/user/fullscreen/controller 门控 |
| 输入守卫 | 3 个桌面类 | 4 个桌面类 + 可见性/rect/fullscreen |
| 启动 | 全屏 `WS_VISIBLE` 再挂载 | 挂载后按可见性显示 |
| 验收 | 工具链 proof | 唯一产品验收对象 |

## 已验证

| 项 | 结果 | 证据 |
|---|---|---|
| gradient 壁纸渲染 | 通过 | `E:\file\octos\m2-icons.png` |
| 壁纸位于图标下方 | 通过 | 截图中桌面图标显示在 gradient 之上 |
| 桌面右键菜单 | 通过 | `E:\file\octos\m2-rightclick.png`，右键出现“刷新/查看/新建”等菜单 |
| WebView2 覆盖层点击穿透 | 通过 | 日志 `made WebView2 overlay HWND(...) click-through` |
| 鼠标输入转发到壁纸页 | 通过 | 日志 `wallpaper page: {"type":"click","x":994,"y":295,"target":"DIV"}` |
| Win11 挂载路径 | 通过 | 日志 `desktop attach: Progman path (insert_after=SHELLDLL_DefView)` |
| 桌面图标恢复 | 通过 | `HideIcons=0`，`SysListView32` 可见，截图确认图标显示 |
| 主窗口最小化时图标可见 | 通过 | `E:\file\octos\desktop-alpha-fixed.png`；诊断 `SUMMARY ... mainMinimized=True ... PASS` |
| 主窗口恢复前台时图标可见 | 通过 | `E:\file\octos\desktop-fixed-window.png`；诊断 `mainVisible=True mainMinimized=False ... PASS` |
| 显示桌面状态图标可见 | 通过 | 诊断 `SUMMARY ... chromeNextProgman=0 ... PASS` |
| Clear wallpaper | 通过 | 日志 `ui -> host: {"type":"clear-wallpaper"}`；`OctosWallpaperWindow visible=False`，`chromeOverlays=0` |
| Show gradient | 通过 | 日志 `ui -> host: {"type":"show-wallpaper"}`；壁纸/Chrome 覆盖层重新出现，`belowIcons=True` |
| 全屏暂停/恢复 | 通过 | `scripts/fullscreen-probe.ps1`；日志 `fullscreen=true` → 壁纸 `visible=False`，关闭测试窗口后 `fullscreen=false` → `visible=True` |
| Explorer 重启恢复 | 通过 | 新 Progman/SHELLDLL_DefView；壁纸 `parent=新Progman`、`prev=新SHELLDLL_DefView`、`belowIcons=True`；Chrome 宿主重新出现 `chromeOverlays=1`；截图 `E:\file\octos\explorer-restart-fixed.png` |
| 桌面图标可见性 API | 通过 | Settings 开关 → `ui -> host: {"type":"set-desktop-icons","visible":false}`；Rust 测试 `desktop_icon_visibility_round_trip ... ok`（0.60s），恢复后 `HideIcons` 仍为 0 |
| DPI | 通过 | 物理 2880×1800，200% 缩放；页面 CSS 坐标约为物理坐标的一半 |

## 踩到的坑与修复

### 1. 挂载顺序错误，壁纸跑到图标上方

**现象**：壁纸显示，但图标不显示/被遮住，用户感觉像全屏应用。

**原因**：M2 初版沿用“先找 WorkerW，再回退 Progman”的顺序。Win11 桌面层级是：

```
Progman
├── SHELLDLL_DefView   (图标层)
└── WorkerW             (桌面层)
```

顶层还有一些不可见/很小的 `WorkerW`。如果先随便挂到一个顶层 WorkerW，壁纸就会脱离图标层，或者层级在上方。

**修复**：先检查 `Progman` 下是否存在 `SHELLDLL_DefView`；存在就走 Win11 路径，`SetParent(Progman)` + `SetWindowPos(hwnd, shell_view, ...)` 插到图标层下方。只有找不到 `SHELLDLL_DefView` 时才走 WorkerW。

**规则**：不要用“先 WorkerW”作为默认顺序；先判断当前桌面的真实层级。

### 2. 全屏检测误判，壁纸每 500ms 闪一次

**现象**：壁纸反复显示/隐藏；日志每 500ms 交替 `fullscreen=true/false`。

**原因**：
- WebView2 自己的 `Chrome_WidgetWin_*` 覆盖整个显示器，被当成全屏应用。
- Windows Spotlight / Start 的 `SSTDimOverlayWindow` 也覆盖全屏。
- `ToggleDesktop` 使用 DWM cloak，`IsIconic` 仍为 false，普通可见性判断会误判。

**修复**：
- 排除 `Chrome_WidgetWin_*`、`Chrome_RenderWidgetHostHWND`、`WebView2*`、`*OverlayWindow`、`OctosMainWindow`、`OctosWallpaperWindow`、`Shell_TrayWnd`、`Progman`、`WorkerW`。
- 用 `DwmGetWindowAttribute(DWMWA_CLOAKED)` 排除 cloaked 窗口。
- 只在全屏状态变化时打 info 日志，候选窗口打到 debug。

**待办**：`is_fullscreen` 还是启发式，正式版应补上原版的窗口样式/owner 检查，并用真实全屏游戏/视频回归。

### 3. WebView2 覆盖层挡住桌面右键

**现象**：桌面右键菜单不出现，无法“刷新”。

**原因**：`Chrome_WidgetWin_*` 顶层覆盖层没有点击穿透。

**修复**（原版 Watchdog 同款）：对可见、矩形匹配显示器、类名以 `Chrome_WidgetWin_` 开头的窗口补：

```
WS_EX_LAYERED | WS_EX_TRANSPARENT
SetLayeredWindowAttributes(alpha=255)
SetWindowPos(... SWP_FRAMECHANGED)
```

日志 `made WebView2 overlay HWND(...) click-through` 确认生效。

### 4. 桌面图标被用户关闭

**现象**：用户之前关闭了 Windows 桌面图标，截图里没有图标。

**处理**：
- 注册表 `HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced\HideIcons = 0`。
- 找到 `Progman -> SHELLDLL_DefView -> SysListView32`，`ShowWindow(SW_SHOW)`。
- 截图确认图标恢复，且位于壁纸之上。

**注意**：这是用户设置，正式应用不应默认强制开启图标；应通过 `System.setDesktopIconVisibility` API/设置项让用户控制。

### 5. Chrome_WidgetWin 顶层宿主被强制不透明，盖住图标

**现象**：主窗口最小化/终端在前台时，桌面只有 gradient 和时钟，图标消失；`ToggleDesktop` 后图标又出现。验证脚本显示壁纸 HWND 的 `prev=SHELLDLL_DefView`、`SysListView32 visible=True`，看起来 z-order 正常。

**根因**：真正显示 gradient 的不是壁纸 HWND，而是 `msedgewebview2` 的顶层 `Chrome_WidgetWin_1` 宿主窗口。它在顶层 z-order 中位于 Progman 之上；我们额外调用了：

```rust
SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
```

把它从“按像素透明”的 WebView2 宿主变成了不透明窗口，于是它盖住图标。上游 Octos 的 `FixRenderingTarget` **只加 exstyle**，不调用 `SetLayeredWindowAttributes`。

**修复**：
- `Chrome_WidgetWin_*` 只做 `WS_EX_LAYERED | WS_EX_TRANSPARENT`，保留 WebView2 自己的 per-pixel alpha；**禁止**加 `alpha=255`。
- 每次 reconcile 校验 `parent` 和 `insert_after`（`SHELLDLL_DefView`）句柄；失效就重新挂载。
- `SetWindowPos` 后用 `GetWindow(hwnd, GW_HWNDPREV)` 校验壁纸是否紧贴 `SHELLDLL_DefView` 下方，不对就重新插入。

**证据**：隐藏 Chrome 宿主后图标出现、gradient 消失；移除 alpha 调用后，主窗口最小化和前台两种状态图标都正常显示。

### 6. Explorer 重启后的 controller 生命周期

**现象**：Explorer 重启后 z-order/图标 PASS，但 `chromeOverlays=0`，gradient 没有真正恢复渲染。

**根因**：
- `reconcile` 在 `ensure_window` 失败（Progman 尚未恢复）后仍然调用了 `ensure_webview`，为隐藏的 0x0 窗口创建了 controller。
- 新窗口随后挂载/定位，但 controller 的 bounds 没有重新同步，WebView2 宿主仍是 0x0 或不可见。

**修复**：
- `ensure_window` 失败时 `continue`，不要为该 slot 创建 controller。
- `ensure_window` 末尾对已有 webview 调 `resize(hwnd)`，窗口尺寸变化后重新同步 bounds。
- `verify-desktop.ps1` 新增判断：`wallpaperVisible=True && chromeOverlays=0` 输出 WARN，避免再次把“空壁纸”误判为 PASS。

### 7. 诊断时的坑

- `GetParent` 对 `WS_POPUP` + `SetParent` 的窗口返回 0；不能用它判断父级。诊断用 `EnumChildWindows` / `GetAncestor`。
- PowerShell 里 `FindWindowW('Class', $null)` 可能把 `$null` 编组为空字符串而不是 NULL，导致“明明有窗口却找不到”。用 C# helper 内部传 `null`，或显式传标题。
- `SendMouseInput` 使用**物理客户区坐标**；页面 `event.clientX/Y` 是 CSS 坐标。200% 缩放下，物理 (1989,591) 对应页面 (994,295)。

## 诊断与回归矩阵

`scripts/verify-desktop.ps1` 已增强，除壁纸 HWND 外还会检查：

- `msedgewebview2` 顶层 `Chrome_WidgetWin_*` 的 PID/进程名、rect、exstyle、相对 Progman 的 z-order；
- 壁纸 HWND 的 parent / `prev` / `next` / `belowIcons`；
- 主窗口 visible/minimized；
- `SUMMARY` 与 `PASS/WARN`。

三种状态均 PASS：

| 状态 | 关键输出 |
|---|---|
| 主窗口前台 | `mainVisible=True mainMinimized=False wallpaperBelowIcons=True` |
| 主窗口最小化 | `mainVisible=True mainMinimized=True wallpaperBelowIcons=True` |
| 显示桌面 | `SysListView32 visible=True wallpaperBelowIcons=True` |

注意：脚本无法从 Win32 直接检测 Chrome 覆盖层是否被调用过 `SetLayeredWindowAttributes(alpha=255)`；这是代码级约束，已写入 `docs/upstream-map.md` §4。

## 尚未完成 / 未验证

- 真实全屏应用（游戏/视频）暂停：已用边框全屏测试窗口验证 pause/resume；建议再用真实游戏/视频回归一次。
- 多显示器：当前环境只有 `\\.\DISPLAY1`，多屏定位/z-order 未测。
- 按显示器设置/清除：后端只有 `show_all`/`clear_all`，UI 还没有按显示器选择。
- 桌面图标 API 已接入 App UI；壁纸侧 `System.setDesktopIconVisibility` 的 JS 协议封装仍属于 M3。
- 桌面图标 API 是**临时 ShowWindow**，不写 `HideIcons`（与上游一致）；不会持久化用户设置。
- 鼠标：只转发鼠标，不转发键盘；指针从壁纸移动到其他窗口时没有合成 button-up。
- watchdog 每 500ms 都 `SetWindowPos`，后续应只在 rect/z-order 变化时调整。


## 下一步（M2 收尾）

1. 多显示器（如可接入）与设备名 registry。
2. 用真实游戏/视频再回归一次全屏 pause/resume。
3. 把 M2 剩余诊断日志收敛（已把逐点击/页面/z-order retry 降到 `debug`）。
4. 完成后写 M2 验收报告，再进入 M3。

## 关键文件

- `scripts/verify-desktop.ps1` — 只读桌面层级诊断（图标、壁纸窗口 exstyle/z-order、`msedgewebview2` 覆盖层 PID/z-order）。
- `scripts/fullscreen-probe.ps1` — 边框全屏测试窗口，验证壁纸暂停/恢复。
- `docs/upstream-map.md` — Rust ↔ C++ 模块映射、有意差异、已修复偏差和新增 Win32 行为规则。
- `crates/octos-win/src/wallpaper.rs` — 显示器 registry、挂载、输入、watchdog、全屏、覆盖层穿透。
- `crates/octos-webview/src/lib.rs` — CompositionController + DComp + 透明背景 + 壁纸导航。
- `assets/wallpapers/gradient/` — 内置测试壁纸。
- `crates/octos-app/src/main.rs` — 启动时初始化壁纸、显示 gradient、UI 命令。
