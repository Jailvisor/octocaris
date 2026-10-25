# M0 Spike Report

日期：2026-10-22
状态：**完成（spike 已退役，不是产品验收对象）**。核心验证、Explorer 重启恢复、21.5 分钟稳定性全部通过。Go/no-go：**go**。

> **注意**：本报告只证明工具链和 Composition 路径。`octos-spike` 使用 WorkerW-first 挂载，没有 Chrome 覆盖层点击穿透、全屏/可见性门控、多显示器；**不要用它判断产品壁纸行为**。Windows 验收只用 `octocaris.exe`。差异表见下方 §Spike vs M2。

## 环境

| 项 | 值 |
|---|---|
| WSL | Ubuntu noble，Rust 1.94.1 |
| 交叉编译 | cargo-xwin 0.23.1，target `x86_64-pc-windows-msvc` |
| Windows | Windows 11 build 26300 |
| WebView2 Runtime | 155.0.4283.62 |
| 显示器 | 2880×1800 物理，200% 缩放（DPI 192） |
| 测试目录 | `E:\file\octos` |
| 产物 | `octos-spike.exe`，约 228 KB，x64 GUI |

## 构建

```bash
cd spikes/win-webview2
cargo xwin build --target x86_64-pc-windows-msvc --release
cp target/x86_64-pc-windows-msvc/release/octos-spike.exe /mnt/e/file/octos/
powershell.exe -NoProfile -Command "Start-Process 'E:\file\octos\octos-spike.exe'"
```

cargo-xwin 首次构建会下载 MSVC CRT/Windows SDK；本机在未安装 clang/lld 的情况下也能完成链接（`webview2-com-sys` 在 MSVC target 下静态链接 `WebView2LoaderStatic.lib`，exe 不依赖 `WebView2Loader.dll`）。

## 已验证

| 项 | 结果 | 证据 |
|---|---|---|
| WSL 交叉编译 Windows GUI exe | 通过 | `cargo xwin build` 成功，PE32+ x64 GUI |
| Win32 窗口 + 消息循环 | 通过 | 进程存活，`WM_DESTROY` 清理 |
| Win11 桌面挂载 | 通过 | 日志 `attached to Progman below SHELLDLL_DefView (Win11 path)` |
| D3D11 + DirectComposition | 通过 | 日志 `d3d11 + directcomposition ready` |
| CompositionController | 通过 | 日志 `composition controller ready, HTML navigated` |
| 透明背景 | 通过 | 全屏截图中原壁纸透过卡片可见 |
| JS → Rust 消息 | 通过 | 页面 `ready` 消息写入日志 |
| 鼠标转发 | 通过 | 合成点击命中 “Click me”，日志 `page message: {"type":"ping","count":1}` |
| 桌面命中保护 | 通过 | 点击任务栏时 `class=Shell_TrayWnd desktop=false`，未转发 |
| Explorer 重启恢复 | 通过 | 2026-10-22 16:11:40 重启 explorer.exe；watchdog 在 Progman 重建前重试 2 次，第 3 次挂载成功，页面重新 `ready` |
| 恢复后渲染 | 通过 | `E:\file\octos\desktop-after-explorer.png`，卡片仍居中透明 |
| 恢复后输入 | 通过 | 恢复前合成点击命中 `Click me`，日志 `ping` |

## 关键测量

```
geometry: dpi=192 window={0,0,2880,1800} client={0,0,2880,1800}
          parent=None bounds={0,0,2880,1800} raster=1
mouse button 201: screen={1110,910} top=SHELLDLL_DefView desktop=true
page message: {"type":"ping","count":1,...}
```

注意：`GetParent` 对 `WS_POPUP` + `SetParent` 的窗口返回空；后续定位必须用 `GetWindowRect`/屏幕坐标，不能依赖 `GetParent` 的返回值。显示器位置与父窗口坐标要在 M2 用 `GetMonitorInfoW` + 父窗口 rect 计算。

## 截图的坑

`System.Windows.Forms.Screen.PrimaryScreen.Bounds` 在 DPI-unaware 的 PowerShell 里返回 1440×900，`CopyFromScreen` 只截了 2880×1800 屏幕的左上角，看起来像卡片偏到右边缘。先调用 `SetProcessDPIAware()` 再截，得到完整 2880×1800，卡片居中。验证脚本要显式处理 DPI。

## 稳定性观察

2026-10-22 16:10:42 启动，16:32:17 停止，共 **21.5 分钟**，期间经历一次 Explorer 重启：

```
CPU: 1.33s   Handles: 263   WorkingSet: 46.0 MB   Responding: True
```

句柄和内存没有增长，进程持续响应。M0 验收完成。

## 剩余（非阻塞）

1. 把 `E:\file\octos\logs\spike.log` 的验证方式固化到 `xtask`/脚本。
2. 记录 WebView2 缺失时的行为（当前直接创建失败，只写日志）。

## Watchdog 记录

Explorer 重启时，旧的壁纸窗口被销毁，`WM_DESTROY` 不再直接退出进程；500ms 定时器检测到窗口失效后重建。Progman 尚未恢复时 `FindWindowW` 返回失败（注意：Windows crate 会给出 HRESULT 0 的误导性错误），watchdog 每 500ms 重试，最终挂载成功并复用 WebView2 环境：

```
watchdog: wallpaper window lost, recreating
watchdog: recreate failed: Error { code: HRESULT(0x00000000), ... }   // Progman 尚未回来
...
attached to Progman below SHELLDLL_DefView (Win11 path)
wallpaper recreated: HWND(0x202b2)
reusing WebView2 environment
composition controller ready, HTML navigated
page message: {"type":"ready","url":"about:blank"}
```

正式实现里要把“Progman 不存在”当作可重试状态，不要当成致命错误；窗口创建失败要清理，避免孤儿窗口。

## 对正式工程的影响

- `octos-webview` 的异步创建、Composition + DComp 路径已验证可行。
- 鼠标钩子必须做 `WindowFromPoint` + class 命中判断，否则覆盖在壁纸上的普通窗口会“漏”点击给 mod。
- 需要维护“按下时在壁纸、松开时在别的窗口”的合成 button-up 逻辑（spike 未实现）。
- 显示器定位/DPI 要按物理像素 + 父窗口坐标计算，`GetSystemMetrics` 在 PerMonitorV2 下返回物理像素。
- 主窗口使用普通 controller，壁纸使用 composition controller；两者共用环境。


## Spike vs M2（为什么退役）

| 项 | M0 spike | M2 正式实现 |
|---|---|---|
| 挂载顺序 | WorkerW first，Progman 回退 | Progman + `SHELLDLL_DefView` first，WorkerW 回退 |
| WorkerW 路径 z-order | `SWP_NOZORDER`，可能盖住图标 | 记住 parent/insert_after，watchdog 重新定位 |
| Chrome `Chrome_WidgetWin_*` 覆盖层 | 未处理，可能吞桌面右键 | 强制 `WS_EX_LAYERED|WS_EX_TRANSPARENT` |
| 全屏/可见性 | 无；永远显示 | DWM cloaked + 类名排除 + 可见性门控 |
| 显示器 | 单个主屏 | `EnumDisplayMonitors` + 设备名 registry |
| 桌面图标 | 无处理 | 保持用户设置；有诊断/恢复流程 |
| 输入守卫 | 3 个桌面类 | 4 个桌面类 + 可见性/rect/fullscreen 检查 |
| 启动 | 先 `WS_VISIBLE` 全屏再挂载 | 挂载后再按可见性显示 |
| 验收角色 | 工具链 proof | 唯一产品验收对象 |

结论：spike 的“全屏感”来自缺失的产品级桌面集成，不是壁纸画布架构错误。源码保留在 `spikes/win-webview2/`，不再部署。
