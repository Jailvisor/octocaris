# Upstream Map — Rust 实现 ↔ Octos C++

本文件记录 Rust 重写与上游 `/home/lzt/codehub/rust/octos/octos`（只读参考）的对应关系、**有意差异**和**已经踩过的偏差**。新增 Win32/WebView2 行为前，先查这里和 upstream。

> 原则：以 Octos 的行为为规范，用 Rust 重写；不是逐行翻译。任何 upstream 没有的 Win32 调用，都必须有理由并在此记录。

## 1. 模块对应

| Rust | Upstream | 说明 |
|---|---|---|
| `crates/octos-webview/src/lib.rs` | `src/WebView/WebView.cpp`、`Bootstrap.cpp` | WebView2 环境、普通/Composition controller、DComp、虚拟主机、导航 |
| `crates/octos-win/src/wallpaper.rs` | `src/Core/Core.cpp`、`src/Watchdog/Watchdog.cpp`、`src/Event/Event.cpp` | 显示器 registry、壁纸窗口、桌面挂载、z-order、全屏、鼠标钩子、watchdog |
| `crates/octos-win/src/lib.rs` | `src/main.cpp`、`src/TrayIcon/TrayIcon.cpp`、`Core.cpp` 主窗口部分 | 主窗口、托盘、单实例、消息循环 |
| `crates/octos-app/src/main.rs` | `src/main.cpp`、`src/CLI/CLI.cpp` | 启动编排、日志、WebView2 初始化、壁纸启动、CLI 执行 |
| `crates/octos-core/src/cli.rs` | `src/CLI/CLI.cpp::ParseCommandLineArgs` | 纯参数解析（可在 Linux 单测） |
| `crates/octos-storage/` | `src/Storage/Storage.cpp` | 路径、prefs、zip 安装、下载（M3/M4 补齐） |
| `crates/octos-protocol/` | `src/Bridge/Bridge.cpp`、`src/API/API.cpp`、`src/js/*` | UI/mod 消息协议；mod API 冻结（M3） |
| `crates/octos-core/` | `src/Storage`、`src/API` 的纯逻辑部分 | manifest/catalog/options/session/hooks（M3） |
| `ui/` | `assets/app/index.html`、`index.js`、`styles.css` | 全新 React UI；旧 App 窗口消息协议不保留 |
| `xtask/` | `Octos.vcxproj`、`.github/workflows` | 构建、部署、UI 打包 |

## 2. 必须对照的 upstream 函数

### 桌面挂载 / 壁纸窗口

- `Core.cpp::AttachWindow` — Progman + `SHELLDLL_DefView`（Win11）/ WorkerW（Win10）。
- `Core.cpp::CreateWallpaperWindow` — `WS_EX_LAYERED|TRANSPARENT|TOOLWINDOW|NOACTIVATE`。
- `Core.cpp::ExpandToMonitor` — 相对父窗口的坐标/尺寸。
- `Core.cpp::SetWallpaperVisibility` — ShowWindow/隐藏。
- `Core.cpp::NavigateWallpaperByMonitorId` / `NavigateAllWallpapers` — 窗口重建与导航。

### WebView2

- `WebView.cpp::InitializeWebViewEnvironment` — 环境、user data、浏览器参数。
- `WebView.cpp::AttachWebViewCompositionController` — CompositionController + DComp target/visual + `SetRootVisualTarget`。
- `WebView.cpp::OnWebViewControllerCreated` — bounds、visibility、settings、虚拟主机、导航、消息。

### Watchdog / 输入

- `Watchdog.cpp::FixRenderingTarget` — 全屏检测 + `Chrome_WidgetWin_*` exstyle 处理。
- `Watchdog.cpp::WatchdogProc` — 父级丢失、窗口死亡、显示器变化、z-order。
- `Watchdog.cpp::FixWallpaperOrder` — Win11 壁纸在 `SHELLDLL_DefView` 下方的顺序维护。
- `Event.cpp::MouseEventProc` — 低级鼠标钩子、`WindowFromPoint`、`SendMouseInput`、跨窗口 button-up。

### 主窗口 / 托盘 / 存储 / 桥

- `main.cpp` — 单实例、启动顺序、`WM_COPYDATA`、窗口过程。
- `TrayIcon.cpp` — `Shell_NotifyIcon`、菜单、`TaskbarCreated`。
- `Storage.cpp` — `%LOCALAPPDATA%\Octos`、prefs、zip 安装、虚拟路径。
- `Bridge.cpp`、`API.cpp` — App 消息、mod request/command/subscribe/event。

## 3. 有意差异（Rust 新实现）

| 项 | Upstream | Rust | 理由 |
|---|---|---|---|
| 桌面 App UI | vanilla HTML/CSS/JS，DOM 即状态 | React 19 + TS + Vite + 类型化 IPC | 现代化、可维护；App 窗口内部协议不属于 mod 契约 |
| 显示器关联 | vector 索引 | 设备名 `\\.\DISPLAYn` 优先两遍匹配 + rect 兜底 | 修复热插拔/主屏切换抢槽位 |
| 热插拔 | 只向 UI 发 monitor-ids | reconcile + 恢复/默认分配 + Library 快照推送 | UI 与新显示器立即一致 |
| WebView2 初始化 | 阻塞等待环境 | 异步状态机 + generation | 避免 UI 线程卡死 |
| prefs | 非原子写 | 原子 replace + `.bak` + 损坏隔离；250ms debounce + 退出 flush | 防数据丢失/减少滑块写盘 |
| 沙箱 | `enableSandboxing` 取反 | 默认启用；不传 `--no-sandbox` | 修复旧 bug |
| WebView2 进程失败 | 无处理 | 主 UI `Reload()`；壁纸 1s 后重建 controller | renderer 崩溃恢复 |
| 日志 | 无控制台、无文件 | `tracing` 滚动文件 | 可诊断 |
| 全屏检测 | 工作区+样式+owner+cloaked | 额外排除 `Chrome_WidgetWin_*`、`WebView2*`、`*OverlayWindow`、`ShellHandwritingCanvas*`、`XamlExplorerHost*` | 避免 WebView2 自身和 Win11 shell 窗口导致闪烁 |
| z-order | 周期 watchdog 维护 | 显式校验 `parent`/`insert_after` 句柄 + `GW_HWNDPREV` | Explorer 重建句柄时不落到图标上方 |
| Chrome 覆盖层 | 只加 `WS_EX_LAYERED|WS_EX_TRANSPARENT` | 同上游；**禁止额外 `SetLayeredWindowAttributes(alpha=255)`** | 见 §4 |
| Watchdog 写操作 | 只在 rect/order 需要时 `SetWindowPos`；`hidden` 标记避免重复 `put_IsVisible` | 同样只在变化时 SetWindowPos/SetBounds/SetIsVisible | 避免每 500ms 重新合成导致闪烁 |
| 多屏 z-order | `FixWallpaperOrder` 从 shellView 沿 `GW_HWNDNEXT` 查找 | 同样从 DefView 沿 `GW_HWNDNEXT` 查找（保留 64 步上限） | 多块壁纸/Explorer 辅助窗口插在中间都不应触发重排 |
| CLI 转发 | 第二实例 `WM_COPYDATA` 发送 `GetCommandLineW()` | 同上游；接收端在 UI 线程解析并执行 | 保持引号/Unicode 原样 |
| `run` 已在 wallpapers 内 | 复制到自身，通常失败 | 直接使用该顶层 mod 目录 | 开发时运行已安装 mod 更顺手 |
| `run` 新目录无 manifest | 允许任意含 HTML 的目录 | `install_folder` 要求 `octos.json`/`mod.json` | 与 M4 误装防护一致；无 manifest 的目录请先 `octos new` |
| GPU 默认 | 缺省 `appOptions` 时 `enableGPU=false` → `--disable-gpu` | 缺省 `true`；显式 `enableGPU=false` 时加 `--disable-gpu` | 保持 Rust 现有渲染行为，同时尊重旧 prefs |
| `pauseFullscreen` | 无持久化开关（始终暂停） | `appOptions.pauseFullscreen` 持久化，可在 Settings 关闭 | Rust 新增设置，不改变 mod 协议 |
| 沙箱 | `enableSandboxing=true` 反而加 `--no-sandbox` | 始终启用，不暴露开关 | 安全规则；不复制旧 bug |

## 4. 已发现的偏差（已修复）

### 4.1 Chrome_WidgetWin 被强制不透明

- **偏差**：Rust 版在 `make_chrome_overlays_click_through` 里额外调用了
  `SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA)`。
- **后果**：WebView2 的顶层 `Chrome_WidgetWin_1`（`msedgewebview2.exe`）从 per-pixel 透明宿主变成不透明窗口，位于 `Progman` 之上，盖住桌面图标。
- **修复**：只设置 `WS_EX_LAYERED | WS_EX_TRANSPARENT`，与 upstream `FixRenderingTarget` 一致；不调用 `SetLayeredWindowAttributes`。
- **证据**：`docs/m2-wallpaper.md` §5；隐藏 Chrome 宿主后图标出现、gradient 消失；移除 alpha 调用后最小化/前台两种状态图标正常。

### 4.2 `insert_after` 句柄失效

- **偏差**：只校验了 `parent`（Progman）有效，没有校验 `insert_after`（`SHELLDLL_DefView`）。
- **后果**：Explorer 重建 `SHELLDLL_DefView` 但 Progman 存活时，`SetWindowPos` 使用失效句柄，z-order 可能落到图标上方。
- **修复**：每次 reconcile 校验 `parent` 和 `insert_after`；失效就重新 `attach_to_desktop`。
- **证据**：`crates/octos-win/src/wallpaper.rs::ensure_window`。

### 4.3 Explorer 重启后的 controller 生命周期

- **偏差**：`reconcile` 在 `ensure_window` 失败后仍创建 controller；新窗口定位后没有重新同步 controller bounds。
- **后果**：Explorer 重启后壁纸 HWND 层级正确，但 WebView2 宿主是 0x0/不可见，gradient 不渲染；诊断只看 z-order 会误判 PASS。
- **修复**：`ensure_window` 失败时跳过 `ensure_webview`；`ensure_window` 末尾对已有 webview 调 `resize(hwnd)`；诊断脚本在 `wallpaperVisible && chromeOverlays==0` 时 WARN。
- **证据**：`docs/m2-wallpaper.md` §6；Explorer 重启回归日志与 `explorer-restart-fixed.png`。

### 4.4 桌面图标可见性

- Rust `crates/octos-win/src/desktop_icons.rs` 对应 upstream `API.cpp` 的 `set-desktop-icons-visible/-hidden`。
- 行为与上游一致：只对 `Progman -> SHELLDLL_DefView -> SysListView32` 调 `ShowWindow`。
- **不写** `HideIcons` 注册表；那是用户设置，Explorer 自己维护。写入会与用户的“显示桌面图标”设置打架。
- App 的 Settings 开关调用 Rust API；壁纸侧的 `System.setDesktopIconVisibility` JS 封装在 M3 接。

### 4.5 `GetParent` 诊断误用

- **偏差**：`WS_POPUP` + `SetParent` 的窗口 `GetParent` 返回 0。
- **修复**：诊断用 `EnumChildWindows`/`GetAncestor`；不在生产逻辑里依赖 `GetParent` 判断父级。

## 5. 新增 Win32 行为的规则

1. 先在 upstream 里搜索同类调用；在提交/说明中引用 `文件:行号`。
2. 如果 upstream 没有，写清“为什么需要”；更新本文件的差异表。
3. 必须跑 `scripts/verify-desktop.ps1`，并覆盖回归矩阵：
   - 主窗口前台；
   - 主窗口最小化；
   - 显示桌面（`ToggleDesktop`）；
   - 桌面图标可见；
   - 右键菜单；
   - 壁纸页面点击日志；
   - Explorer 重启（经用户同意后）。
4. 涉及透明/点击穿透时，**同时检查壁纸 HWND 和 `msedgewebview2` 顶层 `Chrome_WidgetWin_*`**，不能只看 `SysListView32 visible=True`。
5. 失败就记录在 `docs/m2-wallpaper.md` / 本文件，不要用“应该可以”带过。
