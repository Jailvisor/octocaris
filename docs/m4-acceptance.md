# M4 Acceptance — Library、CLI、Settings 持久化

日期：2026-10-23
环境：WSL2 Ubuntu + cargo-xwin 0.23.1，Windows 11 build 26300，`OCTOS_DATA_DIR=E:\file\octos\data`。

## 1. 自动化检查

| 检查 | 结果 |
|---|---|
| `cargo fmt --check` | 通过 |
| `cargo test -p octos-protocol -p octos-core -p octos-storage` | 57 通过（protocol 10 / core 31 / storage 16） |
| `cargo clippy -p octos-protocol -p octos-core -p octos-storage --all-targets -- -D warnings` | 通过 |
| `npm --prefix ui run typecheck` | 通过 |
| `npm --prefix ui run build` | 通过 |
| `cargo xtask deploy`（cargo xwin release + UI + 部署） | 通过，`E:\file\octos\octocaris.exe` |

> `cargo clippy --workspace` 在 Linux 宿主仍无法运行：`rfd` 引入的 `wayland-sys` 需要 pkg-config，`windows-future` 不能为宿主目标编译。Windows 侧以 `cargo xwin build --workspace` 为准。

## 2. CLI 实测

第二实例通过 `WM_COPYDATA` 转发完整命令行；接收端在 UI 线程解析执行。

| 命令 | 结果 | 证据 |
|---|---|---|
| `octocaris.exe reload` | 重新加载 1 个壁纸 WebView | 日志 `cli reload: 1 wallpaper(s)`，18:23:13Z |
| `octocaris.exe dev-tools` | 延迟 1s 后打开 1 个壁纸 DevTools | 日志 `opened DevTools on 1 wallpaper(s)`，18:23:18Z |
| `octocaris.exe run E:\file\octos\cli-fixtures\run-test-mod --dev-tools` | 复制到 wallpapers、设为所有显示器、保存 prefs、打开 DevTools | 日志 18:24:03Z；`data\wallpapers\run-test-mod`；`preferences.json` 的 `selected.DISPLAY1` 指向该目录；`wallpaper controller ready -> https://local.octos/run-test-mod/index.html` |
| `octocaris.exe new E:\file\octos\cli-new-test` | 创建 `octos.json`（name/entry）和 `index.html` | 文件实测存在；成功 MessageBox 弹出 |
| 无参数第二实例 | 只恢复主窗口，不转发命令 | 代码路径 `args.len() <= 1 -> restore_existing_instance` |

补充：
- `run` 的源目录若已在 wallpapers 下，直接使用该顶层的**非 verbatim 绝对路径**（`E:\...`），保证与 catalog/prefs key 一致，不再复制。
- 新目录仍要求 `octos.json`/`mod.json`（M4 误装防护）；缺 manifest 时显示错误框。
- 错误与 `new` 成功提示沿用原版的 MessageBox 文案结构。

## 3. Settings 持久化实测

| 设置 | 存储 | 生效 | 证据 |
|---|---|---|---|
| `pauseFullscreen` | `appOptions.pauseFullscreen` | 立即 | UI 切换后日志 `pause on fullscreen set to false`（18:37:05Z），prefs 写入 `false`；再切回写入 `true`（18:37:42Z） |
| `enableGPU` | `appOptions.enableGPU` | 重启 | 关闭后 prefs 为 `false`，重启日志 `WebView2 environment: gpu=false`（18:39:35Z）；恢复后重启日志 `gpu=true`（18:40:18Z） |
| `runOnStartup` | `appOptions.runOnStartup` + HKCU Run | 立即写注册表 | 代码与 IPC 已接入；**未在实机点击**，避免改动当前用户的启动项 |
| `allowNotifications` | — | — | 按用户 2026-10-23 决定不做，保留 Settings 占位（M8） |
| 桌面图标 | 不持久化 | 立即 | 与上游一致，仅临时 `ShowWindow` |

UI 截图：`E:\file\octos\m4-settings.png`（中文设置页，含三个真实开关）。

prefs 兼容性：
- `version` 升级到 2，`selected` / `modOptions` 沿用旧键名。
- `appOptions` 只类型化已实现的三个键；`memorySaver`、`allowNotifs`、`enableSandboxing`、`disableMouseInput` 等未知键原样写回。
- 损坏文件隔离为 `preferences.corrupt-<unix>.json`；写盘使用同目录临时文件 + `MoveFileExW(REPLACE_EXISTING|WRITE_THROUGH)`。

## 4. 验证中发现并修复的问题

**`\\?\` verbatim 路径导致 mod key 分裂**：首次实现 `run` 时对“已在 wallpapers 下的目录”使用 `canonicalize()`，得到的 `\\?\E:\...` 与 catalog 的 `E:\...` 不匹配。后果是选中项写入后在下次启动被判为“壁纸不存在”，回退到 gradient，同时 `modOptions` 出现重复键。

修复：`canonicalize()` 只用于判断目录是否位于 wallpapers 内，实际 mod key 使用 `std::path::absolute()` 的结果。重测：`run` 已安装的 L2D 目录后日志为 `cli run applied E:\file\octos\data\wallpapers\3591542544-octos-package`，prefs 只剩两个 mod key（L2D + gradient），重启恢复正常。证据：18:47:50Z / 18:48:22Z 日志。

## 5. 未覆盖 / 已知缺口

1. **多显示器**：当前环境只有 `\\.\DISPLAY1`；按显示器选择、`redirect`、热插拔未实测。
2. **`runOnStartup` 实机切换**：会写 HKCU Run，留待用户确认后手工验证。
3. **`run` 无 manifest 目录**：有意拒绝（与安装防护一致），与上游“任意 HTML 目录”不同，已记入 `docs/upstream-map.md`。
4. **prefs 写盘频率**：每次 `set-option` 同步全量写盘，尚无 250ms debounce 与 `.bak`，属 M5。
5. **通知 / memorySaver / 真实媒体**：按计划分别在 M8 / M6。
6. **全屏暂停行为**：只验证了设置读写；真实游戏全屏回归仍是 M2/M5 缺口。

## 6. 后续更新（M5 / 性能轮）

- 测试数量在 M5/性能改动后增至 **protocol 11 / core 35 / storage 25（共 71）**；`cargo clippy --workspace --target x86_64-pc-windows-msvc -- -D warnings` 通过。
- `appOptions` 现在类型化 `runOnStartup`、`enableGPU`、`pauseFullscreen`、`maxFps`、`memorySaver`、`liteMode`；其余未知键仍原样保留。
- prefs 已按 M5 硬化：250ms debounce、退出 flush、`.bak`、`ReplaceFileW`；损坏隔离不变。
- 全屏暂停、ProcessFailed、Explorer 重启、双屏/热插拔/redirect 已在 `docs/m5-hardening.md`、`docs/multimonitor.md` 实机验证。
- MemorySaver/轻量模式见 `docs/lightweight-mode.md`、`docs/performance.md`：轻量模式是托盘会话状态，销毁主 UI WebView、壁纸继续渲染；打开前端/重启即退出；不简化预览图；强制全屏暂停 + 内存节省，不改变壁纸帧率。
- 主题见 `docs/theme.md`：跟随系统深/浅色，标题栏和托盘菜单同步。

## 7. UI / UX 实测（Win11 Fluent）

| 项 | 结果 | 证据 |
|---|---|---|
| Library 视觉 | Fluent 深色：分层 Mica 背景、Segoe UI Variable、8px 卡片/4px 控件、细滚动条、焦点环 | `E:\file\octos\m4-ui-library.png` |
| Settings 视觉 | 行式设置卡片、Fluent 开关、Settings 页隐藏 Library 操作按钮 | `E:\file\octos\m4-ui-settings.png` |
| 加载态 | 首次 `get-library` 前显示卡片/详情骨架屏 | `isHosted() && library === null` 分支 |
| 空态 | 0 个 mod 时显示插图、安装/导入主操作、渐变/清除次操作 | 临时数据目录 `ui-empty` 实测，`E:\file\octos\m4-ui-empty.png` |
| 图片加载 | `loading="lazy"` + 淡入；失败回退首字母占位 | L2D 封面与 Gradient 首字母实测 |
| 键盘导航 | UIA 聚焦 Gradient，按右方向键后 L2D `IsSelected=True`、Gradient `False` | UI Automation `SelectionItemPattern` |
| 安装反馈 | 安装中按钮 spinner/禁用；新增 mod 时显示成功 toast；错误走错误色 InfoBar | 代码路径 + 类型检查；未额外构造失败 zip |
| 当前壁纸 | 首次加载默认选中显示器正在使用的 mod，并显示“使用中”pill | `E:\file\octos\m4-ui-library.png` |
| 窗口位置 | 移到 (180,110) 1300×850，退出后 prefs 保存该矩形；重启恢复到同一矩形；随后清除 `window` 恢复默认居中 | 19:25–19:27Z 日志/实测；`preferences.json` |

> UI 保持深色 Fluent（Win11 深色模式）；未做浅色主题。

## 7. 测试产物

- 日志：`E:\file\octos\data\logs\octos.log.2026-10-22`
- 截图：`E:\file\octos\m4-ui-library.png`、`m4-ui-settings.png`、`m4-ui-empty.png`、`m4-settings.png`
- CLI fixtures：`E:\file\octos\cli-fixtures\`、`E:\file\octos\cli-new-test`
- 测试数据根：`E:\file\octos\data`（未触碰 `%LOCALAPPDATA%\Octos`）
