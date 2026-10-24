# 性能与省电

日期：2026-10-23
环境：单屏 DISPLAY2 2880×1800 @ 200%，壁纸 L2D（4K 资源）或静态图片。测量方式：`Get-Process` 采样 10 秒 CPU 时间，按单核百分比归一化；包含 `octos.exe` 及所有 `--webview-exe-name=octos.exe` 的 `msedgewebview2.exe` 子进程。

## 基线

| 场景 | 单核 CPU | 说明 |
|---|---:|---|
| L2D 动画，未限帧 | **52.3%** | 主要来自 WebView2 GPU 进程 + L2D renderer |
| L2D，30 FPS | **41.4%** | 默认值 |
| L2D，15 FPS | **32.5%** | 更省电，适合常驻 |
| 静态图片壁纸（30 FPS 设置不生效） | **14.1%** | 几乎只有宿主/合成基线 |
| L2D + 全屏应用遮挡 | **13.4%** | 壁纸 `SetIsVisible(false)` 后动画停止 |
| `octos.exe` 本体（watchdog 等） | **~0.5%** | 不是主要成本 |

内存：L2D 场景约 1.1 GB（其中 GPU 进程约 530–630 MB，L2D renderer 约 130–240 MB）；静态图片场景明显更低。

## 已做的优化

### 1. 壁纸帧率上限（设置项）

- `appOptions.maxFps`，默认 **30**；可选 15 / 30 / 60 / 不限制。
- 壁纸 WebView 创建时注入脚本包装 `requestAnimationFrame` / `cancelAnimationFrame`，按 `window.__octosFpsCap` 节流；切换设置时通过 `ExecuteScript` 立即更新，不需要重载页面。
- 实测 52.3% → 41.4%（30 FPS），15 FPS → 32.5%。
- 相关代码：`octos-core::frame_cap`、`WallpaperState::set_fps_cap`、Settings “壁纸帧率”。

### 2. watchdog 降频

- 全屏检测不再每 500ms 枚举全部窗口：前台窗口变化时立即检查，另有 2 秒兜底。
- Chrome 覆盖层 click-through 改为 controller 就绪时立即执行 + 2 秒兜底。
- `octos.exe` 本体空闲 CPU 约 0.5%，主要成本仍在 WebView2 渲染进程。

### 3. 已有的隐藏/全屏暂停

- 全屏应用遮挡时 `SetIsVisible(false)`，L2D 动画停止，CPU 从 52% 降到 13%。
- 这部分不是本轮新增，但它是当前最大的省电项。

### 4. 托盘挂起（MemorySaver）与轻量模式（2026-10-23 修订）

**MemorySaver（默认开）**

- 关闭 Octos 主窗口到托盘时，挂起 **Octos 主界面 WebView**（`SetIsVisible(false)` + `TrySuspend`），不挂起壁纸；壁纸继续在桌面渲染。
- 从托盘/第二实例恢复窗口时先 `Resume` 再显示，避免白屏。
- 与“全屏时暂停”独立：全屏暂停只把壁纸 `SetIsVisible(false)`，不再对壁纸做 `TrySuspend`。
- 设置关闭时，关闭到托盘不会挂起主界面。

**轻量模式（托盘菜单控制，会话级）**

- 参考 Clash for Windows：勾选托盘菜单“轻量模式”后**立即关闭并销毁主 React UI WebView**，只保留壁纸核心；壁纸继续渲染。
- 不简化界面：卡片预览图、动效、状态面板都保留；再次打开前端即退出轻量模式。
- 不写入 `preferences.json`；退出应用后重新启动直接回到普通模式，托盘不再勾选。
- 开启时强制把 **全屏时暂停** 和 **内存节省** 设为开；帧率仍是独立的 `maxFps` 设置。
- 详见 `docs/lightweight-mode.md`。

## 如何选择

- **最轻量**：静态图片壁纸（`添加图片`），约 14% 单核。
- **动画 + 平衡**：L2D + 30 FPS（默认），约 41%。
- **动画 + 省电**：L2D + 15 FPS，约 33%。
- **全屏游戏/视频**：自动暂停，约 13%。

## 后续可做

1. 限制同时动画的壁纸数量/分辨率，或为 mod 提供质量档位。
2. 若 L2D 是主要使用场景，优先建议 mod 使用 2K 资源而不是 4K/8K；宿主无法替 mod 降资源。
3. 如需进一步降低托盘常驻内存，评估关闭并重建主 WebView controller 的方案（恢复延迟 vs 内存）。

## 2026-10-24 终测：图片/L2D、全屏、息屏

环境：Windows 11，单屏 `DISPLAY2` 2880×1800 @200%，`maxFps = 30`，轮换关闭。
方法：10 秒采样 `octos.exe` + 所有带 `--webview-exe-name=octos.exe` 的 `msedgewebview2.exe` 的 `TotalProcessorTime` 增量，按单核归一化；内存为这些进程 WorkingSet 之和。

| 场景 | 单核 CPU | 内存 | 说明 |
|---|---:|---:|---|
| L2D 壁纸（3253308696，WE web 包）空闲 | **27.8%** | **1444 MB** | 7 个进程 |
| L2D + 模拟全屏窗口 | **3.0%** | **1299 MB** | 壁纸 `SetIsVisible(false)`；日志 `fullscreen=true/false` 正常 |
| L2D + 关闭显示器 | **19.4%** | **1338 MB** | 没有显式的息屏/锁屏暂停 |
| 图片壁纸（缩略图先到、原图淡入）空闲 | **3.3%** | **1039 MB** | 静态壁纸，CPU 很低 |

结论：

- **图片壁纸正常**：3.3% / 1.0 GB，符合预期。
- **L2D 正常**：30 FPS 下 27.8%，与之前 32–41% 的基线同一量级（资源包不同）。
- **全屏游戏/视频正常**：壁纸被隐藏，CPU 掉到 3%，退出全屏自动恢复。
- **息屏没有专门处理**：显示器关闭后 Chromium/DWM 会自然降频（27.8% → 19.4%），但仍有约 19% 的 CPU 占用。建议后续增加锁屏/息屏暂停：
  - `WTSRegisterSessionNotification` + `WM_WTSSESSION_CHANGE`：锁屏时 `set_visible(false)`，解锁恢复；
  - 或对壁纸 controller 调用 `TrySuspend`，同时降低内存；
  - 该项不影响当前可用性，只是省电优化。
- 内存隐藏后不释放（全屏 1.3 GB、息屏 1.34 GB），属于 WebView2 保持 controller 的预期行为；`TrySuspend` 可进一步降低。

## 2026-10-24 与原版 C++ Octos 对比（同一 L2D 资源）

资源：`3591542544-octos`（C++）与 `3591542544-octos-package`（Rust），两者均为 58 MB 的同一套 L2D 内容。
环境/方法同上；原版 `E:\application\Octos\Octos.exe` 使用 `%LOCALAPPDATA%\Octos` 数据，其 `preferences.json` 里的 `maxFps: 30` 在 C++ 源码中没有对应实现（全仓搜不到 fps），因此原版实际不限帧；Rust 版默认 30 FPS。

| 场景 | 原版 C++ | Octocaris（Rust） |
|---|---:|---:|
| L2D 空闲 | **48.0% / 1085 MB** | **48.8% / 1522 MB**（30 FPS）<br>**71.7% / 1509 MB**（不限帧） |
| 全屏窗口 | 52.3% / 1131 MB（未识别无边框窗口） | 37.2% / 1277 MB（已隐藏壁纸） |
| 关闭显示器 | 34.4% / 1109 MB | 38.4% / 1297 MB |

结论：

- **CPU**：默认 30 FPS 下两者几乎相同（48.8% vs 48.0%）；Rust 版不限帧时会到 71.7%，说明 C++ 版实际渲染帧率更低或受 vsync 限制。
- **内存**：Rust 版高约 40%（~1.5 GB vs ~1.1 GB），主要来自 React 主界面和较新的 WebView2 运行时；仍在可接受范围。
- **全屏**：Rust 版能识别“精确覆盖显示器”的无边框窗口并隐藏壁纸；C++ 版 `Watchdog.cpp` 只接受带 `WS_OVERLAPPEDWINDOW` 的窗口，会跳过无边框/`WS_POPUP` 全屏游戏。
- **隐藏不等于停算**：同一 L2D 资源在全屏隐藏后 Rust 版仍有 37% CPU（WE 的 L2D 包隐藏后只有 3%），说明部分 L2D 页面在 `SetIsVisible(false)` 后仍继续跑 JS/rAF；后续应对隐藏的壁纸 controller 调 `TrySuspend` 或注入暂停脚本。
- **息屏**：两者都没有显式暂停；Rust 版 38.4% vs C++ 34.4%，都需要后续的锁屏/息屏暂停规则。
- 图片壁纸（Rust）3.3% / 1039 MB；原版没有图片壁纸可比。
