# AGENTS.md — Octos Rust 重构

本文件面向在本仓库工作的 AI agent 和开发者。开始任何任务前先读这里，再读 `docs/` 下的架构与迁移说明。

## 0. 当前状态

- 项目：把 Windows 动态壁纸引擎 **Octos** 从 C++/Win32 重写为 **Rust + WebView2 + React**，保留旧 mod 生态。
- 阶段：**M4 完成（验收报告 `docs/m4-acceptance.md`）**：i18n、Library、按显示器选择、选项表单、zip/文件夹/拖拽安装、卸载、错误 toast、`preferences.json` v2 持久化（选中项 + modOptions + appOptions + 窗口位置）、CLI（`run/reload/new/dev-tools` + `WM_COPYDATA` 第二实例转发）、Settings 真实开关（开机启动/全屏暂停/GPU/帧率/轻量模式/内存节省）、Win11 Fluent UI 收尾（骨架屏/空态/图片懒加载/键盘导航/InfoBar toast/窗口位置恢复）均已实装并实机验证。`允许通知` 按用户决定不做（M8 占位）。**M5 完成**：prefs 250ms debounce/退出 flush/`.bak`、ProcessFailed renderer 恢复、边框全屏 + 真实 PotPlayer 全屏、Explorer 重启、桌面矩阵、窗口位置均已完成并实测；DPI 100/150% UI 截图、200% 桌面层级通过（见 `docs/m5-hardening.md`）。**多显示器基础链路已完成**：设备名优先匹配、热插拔恢复/默认壁纸、UI 刷新、每屏壁纸/overlay、跨屏 `redirect`、`DisplaySwitch` 插拔、鼠标路由均已实测；**每屏 localStorage/sessionStorage 隔离**已实现并验证（见 `docs/multimonitor.md`）。**图片壁纸导入**已实现：顶栏“添加图片”、拖拽、`octos run image.png`，生成带 `fit` 选项的普通 mod（见 `docs/image-wallpapers.md`）。**闪烁修复**：watchdog 只在几何/尺寸/可见性变化时写窗口，全屏检测排除 Win11 `XamlExplorerHost*`/`ShellHandwritingCanvas*`（见 `docs/m5-hardening.md`）。**性能/省电第一轮**：壁纸帧率上限（默认 30 FPS，可 15/60/不限）、watchdog 全屏/覆盖层检查降频；实测 L2D 52%→41%（30 FPS）/33%（15 FPS），静态图片约 14%，全屏隐藏约 13%。**MemorySaver/轻量模式**：隐藏 3 秒后 `TrySuspend`、显示时 `Resume`，主窗口隐藏时挂起主 UI；轻量模式只简化 UI + 开 MemorySaver，不改帧率；实测 L2D 1098 MB→1013 MB→1054 MB（见 `docs/performance.md`）。下一项：M6 或打包。**M3 已完成**：`sdk/` 冻结、`octos-core`（35 tests）、`octos-protocol`（11 tests）、`octos-storage`（25 tests）、壁纸宿主请求链 + options 事件/hooks + 旧 `octos.min.js` conformance；跨屏 redirect 已在 M5 实机验证。M2 核心验收通过；多显示器/真实全屏/ProcessFailed 已在 M5 补齐。报告：`docs/m0-spike.md`、`docs/m1-shell.md`、`docs/m2-wallpaper.md`、`docs/m2-acceptance.md`、`docs/m3-plan.md`、`docs/m3-conformance.md`、`docs/m4-plan.md`、`docs/m4-acceptance.md`、`docs/m5-hardening.md`、`docs/multimonitor.md`、`docs/image-wallpapers.md`、`docs/performance.md`、`docs/upstream-map.md`、`docs/ux-review-checklist.md`。
- 新仓库：`/home/lzt/codehub/rust/octos/octos-rs`（本目录）。
- 上游参考：`/home/lzt/codehub/rust/octos/octos`，**只读**，用于行为对照和兼容性测试。Rust ↔ C++ 的模块映射、有意差异和已踩过的偏差见 `docs/upstream-map.md`；新增 Win32/WebView2 行为前先对照它。
- 已批准计划：`~/.claude/plans/unified-dancing-lollipop.md`。仓库内的架构决策写到 `docs/`。

M0 的唯一目标：证明 WSL 能用 cargo-xwin 构建 Windows exe，并且 WebView2 CompositionController + DirectComposition + WorkerW 挂载、透明、鼠标转发、Explorer 重启恢复都能工作。当前结果见 `docs/m0-spike.md`。

### M2 关键结论（务必遵守）

1. **壁纸不是全屏应用。** 每个显示器一个 `WS_POPUP` + `WS_EX_LAYERED|TRANSPARENT|TOOLWINDOW|NOACTIVATE` 窗口，挂在桌面层、图标层下方；不要用 always-on-top 全屏窗口。
2. **Win11 挂载顺序**：先找 `Progman` 下的 `SHELLDLL_DefView`；存在则 `SetParent(Progman)` 并插到 `SHELLDLL_DefView` 下方。只有找不到时才走 Win10 的 WorkerW 路径。不要默认先挂顶层 WorkerW。
3. **全屏检测要排除 WebView2 自己的窗口**：`Chrome_WidgetWin_*`、`Chrome_RenderWidgetHostHWND`、`WebView2*`、`*OverlayWindow`；并用 `DwmGetWindowAttribute(DWMWA_CLOAKED)` 排除 cloaked 窗口，否则壁纸会每 500ms 闪一次。
4. **WebView2 顶层覆盖层要点击穿透**：对匹配显示器矩形的可见 `Chrome_WidgetWin_*` 只补 `WS_EX_LAYERED|WS_EX_TRANSPARENT`（可加 `SWP_FRAMECHANGED`）。**不要**调用 `SetLayeredWindowAttributes(alpha=255)`；那会把 WebView2 的 per-pixel 透明宿主变成不透明顶层窗口，盖住桌面图标。
5. **z-order 要主动校验**：每次 reconcile 校验 `parent` / `insert_after`（`SHELLDLL_DefView`）句柄，并用 `GetWindow(hwnd, GW_HWNDPREV)` 确认壁纸紧贴图标层下方；失效就重新挂载/插入。
6. **桌面图标是用户设置**：不要默认强制开启；需要时走 `System.setDesktopIconVisibility` / 设置项。
7. **诊断注意**：`WS_POPUP` + `SetParent` 的窗口 `GetParent` 返回 0，用 `EnumChildWindows`/`GetAncestor`；PowerShell 的 `FindWindowW(class, $null)` 可能把 null 变空字符串；`SendMouseInput` 用物理客户区坐标，页面 `clientX/Y` 是 CSS 坐标。
8. **验证桌面层级必须覆盖三种状态**：主窗口前台、主窗口最小化、显示桌面；同时检查壁纸 HWND 和 `msedgewebview2` 顶层 `Chrome_WidgetWin_*`，不能只看 `SysListView32 visible=True`。用 `scripts/verify-desktop.ps1`，并对照 `docs/upstream-map.md`。

## 1. 绝对规则

1. **不要修改上游 `../octos/`。** 那是 C++ 参考实现，不是本仓库的一部分。
2. **mod 可见协议是冻结契约。** `octos.json`、`octos.min.js` 的消息字段、事件名、monitor ID、`local.octos`/`app.octos` 虚拟主机、CLI 语义都不能随意改名或改变时序。
3. **只在 Windows 上验证 Windows 行为。** 没有在 `E:\file\octos` 实际跑过，就不能说“已验证透明/点击/挂载/热插拔”。
4. **Windows 产品验收只用 `octos.exe`。** `octos-spike` 是 M0 工具链/Composition 验证程序，已退役；它使用 WorkerW-first 挂载、没有 Chrome 覆盖层穿透/全屏门控/多显示器，不能代表产品行为。源码和说明保留在 `spikes/win-webview2/`。
5. **平台**：Windows 10/11，首版 x64。不要引入跨平台抽象或 macOS/Linux 分支，除非计划里明确要求。
6. **技术栈固定**：Rust 原生 Win32 + `windows` + `webview2-com`。不要换成 Tauri、wry、Electron、egui、Slint。
7. **UI 技术栈固定**：React + TypeScript + Vite；MVP 不做无边框自绘窗口。
8. **不碰真实用户数据做测试。** 测试统一使用 `OCTOS_DATA_DIR=E:\file\octos\data`，不要写 `%LOCALAPPDATA%\Octos`。
9. **不要擅自扩大范围。** 性能规则、i18n、自动更新、Studio、播放列表都在 M9；MVP 先把核心跑通。
10. **提交/推送/发布只在用户明确要求时做。** 不要自行 commit、push、打 tag 或上传产物。
11. **保留 MIT LICENSE 和上游署名。** 衍生作品要说明来源。

## 2. 环境事实

| 项 | 状态 |
|---|---|
| 主环境 | WSL2 Ubuntu (noble)，Rust 1.94.1，Node 24.12，npm 11.16 |
| Windows | Windows 11 build 26300，WebView2 Runtime 155，VC++ 运行库已存在 |
| 测试目录 | `E:\file\octos`（WSL: `/mnt/e/file/octos`），可写 |
| 当前状态 | Windows target + cargo-xwin 0.23.1 已装；spike 无需 clang/lld 即可构建（依赖无 C 代码，链接用 rust-lld）；Windows 侧仍无 Rust/MSVC |
| 构建目标 | `x86_64-pc-windows-msvc` |

## 3. M0 工具链

### 安装（WSL）

当前环境已经完成：

```bash
rustup target add x86_64-pc-windows-msvc
cargo install cargo-xwin --version 0.23.1 --locked
```

首次 `cargo xwin` 构建会下载 MSVC CRT/Windows SDK 到 `~/.cache/cargo-xwin`（约 1GB）。如果后续引入 C/CMake 依赖，再安装 `clang lld llvm`；当前 spike 不依赖它们也能链接。如果 Microsoft 源太慢，给 apt/cargo 配代理，或按计划回退到 Windows 侧 Rust + VS Build Tools。

### 构建、部署、运行

```bash
# 构建整个 workspace（正式工程建立后）
cargo xwin build --workspace --target x86_64-pc-windows-msvc --release

# UI
npm --prefix ui ci
npm --prefix ui run build

# 部署 + 启动（xtask 建立后）
cargo xtask deploy
powershell.exe -NoProfile -Command "Start-Process 'E:\file\octos\octos.exe'"

# 带 mod 启动
powershell.exe -NoProfile -Command "Start-Process 'E:\file\octos\octos.exe' -ArgumentList 'run','E:\file\octos\wallpapers\test-mod','--dev-tools'"
```

> **`octos-spike` 已退役。** 它只保留作 cargo-xwin/Composition 工具链验证，不再复制到 `E:\file\octos`，也不用于产品验收。历史构建命令（仅参考）：
>
> ```bash
> cd spikes/win-webview2
> cargo xwin build --target x86_64-pc-windows-msvc --release
> ```
>
> 说明见 `spikes/win-webview2/README.md`。Windows 产品验收只用 `octos.exe`。

日志必须写文件（例如 `E:\file\octos\data\logs\octos.log`），GUI 进程不要依赖 stdout。

### 回退条件

如果 cargo-xwin 在链接、CRT、静态 WebView2Loader 或 COM 初始化上阻塞超过 1 天：

1. 在 Windows 安装 Rust + VS Build Tools + Windows 11 SDK。
2. 源码仍在 WSL，通过 `\\wsl$` 或复制到 Windows 构建。
3. CI 始终用 `windows-latest` 作为权威构建。

## 4. 仓库结构

```
octos-rs/
├── crates/
│   ├── octos-protocol/   # mod API/事件、UI 协议、monitor/options 的 serde 类型
│   ├── octos-core/       # manifest、catalog、prefs 模型、options、session、hooks
│   ├── octos-storage/    # paths、原子写、prefs store、zip 安装、下载
│   ├── octos-webview/    # WebView2 env/controller/DComp/虚拟主机（Windows）
│   ├── octos-win/        # HWND、WorkerW、显示器、输入钩子、托盘、watchdog（Windows）
│   └── octos-app/        # bin octos.exe：编排、CLI、UI/mod 桥
├── ui/                   # React + TS + Vite
├── sdk/                  # 原样复制的 src/js + rollup，产出 octos.min.js
├── assets/               # 图标、内置壁纸、LICENSE
├── docs/                 # 架构、冻结协议、迁移说明、ADR、路线图
├── tests/fixtures/       # 测试 mod、旧 prefs、zip 样本
├── tests/e2e/            # PowerShell 冒烟与协议一致性测试
├── xtask/                # build-ui / build / deploy / smoke
└── spikes/win-webview2/  # M0 toolchain spike（已退役，非产品参考）
```

依赖方向：`protocol <- core <- storage`，`protocol <- webview/win <- app`。`protocol`、`core` 和大部分 `storage` 必须能在 Linux 上单测；`webview`/`win`/`app` 只在 Windows target 构建。

## 5. 架构不变量

- **UI 线程是唯一 STA 线程**：所有 HWND、WebView2 controller、DComp 对象、托盘、钩子都在这里创建和释放。不要跨线程持有 COM 指针。
- **watchdog 必须幂等**：每 500ms 只检查状态；`SetWindowPos`、controller `SetBounds`、DComp commit、`SetIsVisible` 只在几何/尺寸/可见性真正变化时调用，否则分层壁纸会闪。
- **壁纸窗口永远不能在图标层上方**：`SendMouseInput` 会抬起 host HWND；壁纸窗口过程必须拦截 `WM_WINDOWPOSCHANGING`，把 `insert_after` 强制为 `SHELLDLL_DefView`。z-order 校验用从 DefView 沿 `GW_HWNDNEXT` 的链查找，不要要求直接相邻。
- **后台线程**只做 prefs 写盘、下载；M6 再加媒体/音频。结果通过 `crossbeam-channel` + `PostMessageW(WM_APP_WAKE)` 回 UI 线程。
- **WebView2 异步状态机**：`Empty -> EnvPending -> ControllerCreating -> Ready/Failed`。不要阻塞 UI 线程等待环境创建。每个窗口槽位带 generation，已销毁窗口的迟到回调直接丢弃；销毁前先 `Controller.Close()`。
- **显示器按设备名 `\\.\DISPLAYn` 关联**，`HMONITOR`/矩形只做辅助。reconcile 必须**先**按设备名匹配所有显示器，**再**用矩形兜底，否则主屏切换时会抢走其他屏的槽位。禁止用 vector 索引对应窗口和显示器。
- **mod 身份用绝对文件夹路径**（旧 prefs 的 `modOptions/selected` 键依赖它）。不要改成 folder name 归一化。
- **prefs 原子写**：同目录临时文件（进程 id + 原子计数）→ flush/`sync_all` → `ReplaceFileW`，保留 `.bak`；失败回退 `MoveFileExW`。UI 线程只提交快照，`PrefsWriter` 后台线程 debounce 250ms，退出前 flush 并 join。损坏文件隔离为 `.corrupt-<timestamp>`。
- **MVP 媒体/音频是协议桩**：requests 永远 resolve 空/默认值，命令静默忽略，订阅接受但不推送。不能模块化地“暂时不支持”导致 Promise 挂起。
- **虚拟主机**：`app.octos` → `<exe>/app`，`local.octos` → wallpapers 目录。mod entry/image 生成 URL 时做百分号编码。壁纸 WebView 注入每屏存储前缀（`octos-core::storage_scope`），主 UI 不注入。
- **错误处理**：库层 `thiserror`，app 层 `anyhow`；FFI 回调里不要 `unwrap`/`panic`，记录日志并返回失败。
- **日志**：`tracing` + 滚动文件，`%LOCALAPPDATA%\Octos\logs`（测试走 `OCTOS_DATA_DIR`）。GUI 进程没有控制台。

## 6. 兼容契约

### mod 格式

- 文件夹 + `octos.json`，回退 `mod.json`。
- 字段：`name/author/description/image/preview/entry/options/source/authorsite/version/keywords`。
- `options` 支持 `checkbox/range/file/description/select/dropdown/color-picker/number/text`，字段 `value/min/max/step/options/label/onchange/onload/onchangeload`。
- 旧的 `prefs` 是 `options` 的别名；未知字段保留不报错。
- entry：http(s) 直接通过；本地路径限制在 mod 目录内；缺失时回退 `index.html` 或第一个 HTML。

### 壁纸内 JS 协议（冻结）

```text
请求: { type:"request", requestType, requestId, data }
响应: { type:"response", requestId, requestType, data }
命令: { type:"command", commandType, data }
订阅: { type:"subscribe"|"unsubscribe", eventType }
事件: { type:"event", eventType, data }
```

- 事件：`mediaChange`、`playbackChange`、`timelineChange`、`audioStream`、`options-change`、`options-load`、`redirect`。
- `redirect` 字段：`sender-id`、`receiver-id`、`message`（保持 hyphen 命名）。
- 任何新增字段都不能改变旧字段的含义；不要“顺手”重命名。
- `setOption` 只更新存储值，不触发 `options-change`；UI 改选项才触发 event + hooks。
- hook 参数保持旧版字符串化语义：`true` → `"true"`，`3` → `"3"`；嵌入脚本前做 JSON 转义。
- 无选中 mod 时，`options` 请求返回 `{}`，不要挂起。

### API

`sdk/` 里的 `System`、`UserOptions`、`MonitorBridge`、`MediaController` 原样复用。字段、返回值、resolve 行为都不改。

### CLI

`run [folder] [--dev-tools]`、`reload`、`new [folder]`、`dev-tools`；第二实例通过 `WM_COPYDATA` 转发完整命令行，接收端在 UI 线程解析执行。

- `run`：解析为绝对路径；文件路径取父目录；新目录经 `install_folder` 复制到 wallpapers（必须含 manifest），已在 wallpapers 下的顶层目录直接使用；随后设为**所有显示器**、保存 prefs、推送 Library 快照。
- `reload`：对每个壁纸 WebView 调 `Reload()`。
- `new`：创建目录（如缺失）并写入 `octos.json` + `index.html`；不覆盖已有文件，成功/失败用 MessageBox。
- `dev-tools`：1 秒后对全部壁纸打开 DevTools，失败重试一次。
- 解析规则在 `octos-core::cli`（Linux 可单测）；执行在 `octos-app`。`--auto` 接受但忽略，未知子命令静默忽略（与上游一致）。

## 7. 编码约定

- Rust 2024 edition，MSRV 1.94。
- 每个 crate 的公开 API 尽量小；平台相关代码只出现在 `octos-win`/`octos-webview`。
- serde 字段用 `#[serde(rename = "...")]` 固定线上名称，不依赖 Rust 命名习惯。
- 路径内部用 `PathBuf`/`OsString`，跨 JSON/JS 边界才转 UTF-8；不要让非 UTF-8 路径导致 panic。
- 不为“以后可能用到”加抽象；MVP 先直白实现，但把兼容契约和线程边界守住。
- 注释只解释“为什么”和契约，不写显而易见的“做了什么”。
- TypeScript 开启 `strict`；UI 与 Rust 的类型由 `octos-protocol` 导出，不手写重复类型。
- 新增依赖前先确认版本、MSRV、Windows/MSVC 支持；MVP 不引入 Tokio。

## 8. 测试与完成的定义

### Linux 可跑

```bash
cargo test -p octos-protocol -p octos-core -p octos-storage
cargo clippy --workspace -- -D warnings
cargo fmt --check
cargo xtask build-ui
```

至少要覆盖：manifest 解析、entry/image 解析、options 合并、hook 字符串化/转义、prefs 迁移与损坏恢复、`appOptions` 往返与未知键保留、CLI 参数解析、`new` 模板、zip 拍平/路径穿越、显示器 reconciliation、协议 serde 往返、无 mod 的 options 返回、媒体桩 resolve。

### Windows 冒烟

```bash
cargo xtask build
cargo xtask deploy
powershell.exe -NoProfile -Command "Start-Process 'E:\file\octos\octos.exe'"

# 只读桌面层级诊断（图标、壁纸窗口、msedgewebview2 覆盖层）
powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/verify-desktop.ps1
```

桌面回归至少覆盖三种状态：主窗口前台、主窗口最小化、显示桌面。每次都要看 `SUMMARY` 和 `PASS/WARN`，并确认 Chrome 覆盖层没有 uniform alpha（见 `docs/upstream-map.md` §4）。

手工检查：透明、点击穿透、桌面图标可点、页面按钮可点、滚轮/右键、全屏暂停、Explorer 重启、显示器热插拔、DPI 100/150/200%、第二实例转发、旧 prefs 迁移、恶意 zip 拒绝。

### Definition of Done

一个改动只有同时满足以下条件才算完成：

1. `cargo fmt --check`、`clippy -D warnings`、相关单测通过。
2. 涉及 Windows 行为时，在 `E:\file\octos` 实际运行并记录结果；不能只靠编译通过。
3. 兼容契约没有被破坏，或已在 `docs/` 写清迁移/兼容说明。
4. 失败、未验证、跳过的部分在回复里明确说明，不用“应该可以”带过。

## 9. 安全规则

- zip：拒绝 `..`、绝对路径、符号链接；限制解压总大小/文件数。
- 删除：只允许删除 wallpapers 目录内的路径；绝不信任 UI 传来的任意路径。
- 外部链接：只允许 http(s)；`open-file` 只允许 wallpapers 内路径。
- WebView2：默认启用沙箱；不要加 `--no-sandbox`；devtools 只在 dev/显式请求时开。
- hooks：只在对应壁纸 WebView 执行；值做 JSON 转义；限制 hook 长度/类型。
- 不把用户数据、路径或日志上传到任何外部服务。

## 10. 里程碑

| 阶段 | 内容 |
|---|---|
| M0 | 工具链 + WebView2/WorkerW spike（完成） |
| M1 | 可启动宿主、React 壳、托盘、单实例、日志（完成） |
| M2 | 壁纸核心：挂载、透明、输入、全屏、Explorer 恢复（核心验收通过；多显示器/ProcessFailed 为已知缺口） |
| M3 | 协议与 System/UserOptions/MonitorBridge 兼容（媒体桩） |
| M4 | React Library + 选项 + CLI + Settings 持久化（完成，见 `docs/m4-acceptance.md`） |
| M5 | 硬化：prefs debounce/.bak、ProcessFailed、真实全屏、Explorer 重启、DPI UI、多显示器基础（完成） |
| M6 | 真实 GSMTC 媒体 + WASAPI 音频 |
| M7 | 社区 Explore/下载 |
| M8 | 完整 Settings、安装包、文档 |
| M9 | 性能/省电、i18n、GitHub 自动更新、Studio、播放列表 |

M2 已知缺口（不阻塞 M3，后续补）：

1. ~~多显示器与按显示器设置~~（M5 已完成基础链路，见 `docs/multimonitor.md`；每屏壁纸内部设置隔离待做）。
2. ~~真实游戏/视频全屏回归~~（M5 已用 PotPlayer 实测通过）。
3. ~~ProcessFailed renderer 崩溃恢复~~（M5 已完成，见 `docs/m5-hardening.md`）。

M3 进度 / 下一步（详见 `docs/m3-plan.md`）：

1. [x] `sdk/` 冻结旧 JS API（src + rollup + README）。
2. [x] `octos-core::manifest` + fixtures + 7 个单测。
3. [x] `octos-core::catalog/options/session/hooks/prefs/cli/storage_scope/frame_cap` + 单测（core 共 35 个测试）。
4. [x] `octos-protocol`：mod request/command/subscribe/event + `WallpaperHost` 分发 + mock 单测（protocol 共 10 个测试）。
5. [x] 壁纸侧 `System`/`UserOptions`(requestOptions/setOption)/`MonitorBridge` + 媒体桩；旧 `octos.min.js` conformance 请求侧通过。
6. [x] 选项由 Rust 直接回答；options-load/change + hooks 顺序/字符串化已实测。
7. [x] 用旧 `octos.min.js` + 一致性 mod 验收（见 `docs/m3-conformance.md`）。

M4 进度 / 下一步（详见 `docs/m4-plan.md`、`docs/m4-acceptance.md`）：

1. [x] Library 快照/卡片/详情/显示器选择/选项表单。
2. [x] zip/文件夹/拖拽安装、卸载、错误 toast、误装防护。
3. [x] `preferences.json` v2：选中项/modOptions/appOptions，原子写、损坏隔离、未知字段保留。
4. [x] CLI 四个子命令 + 第二实例转发（`run` 也接受图片文件）。
5. [x] Settings：开机启动、全屏暂停、硬件加速。
6. [x] UI 细节（Win11 Fluent、安装反馈、图片懒加载、空态/骨架屏、键盘导航、窗口位置恢复）。
9. [x] 图片壁纸导入（`docs/image-wallpapers.md`）。
7. [ ] 多显示器与真实社区 mod 的 M4 回归。
8. [x] prefs 写盘 debounce + `.bak`（M5，见 `docs/m5-hardening.md`）。
10. [x] 性能/省电第一轮：帧率上限 + watchdog 降频（见 `docs/performance.md`）。

## 11. 工作流

- 开始前：`git status` 确认工作区状态；读相关 `docs/` 和本文件。
- 小步提交式修改；一次只解决一个可验证的问题。
- 不确定兼容性时，先写一个 fixture/测试，再改实现。
- 遇到需要 sudo、安装大依赖、修改 Windows 注册表/启动项、发布产物等动作，先说明并等待用户确认。
- 报告结果时先给结论，再给证据（命令、日志、失败输出）。不要隐瞒失败或跳过验证。
