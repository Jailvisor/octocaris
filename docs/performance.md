# 性能与省电

日期：2026-10-23
环境：单屏 DISPLAY2 2880×1800 @ 200%，壁纸 L2D（4K 资源）或静态图片。测量方式：`Get-Process` 采样 10 秒 CPU 时间，按单核百分比归一化；包含 `octocaris.exe` 及所有 `--webview-exe-name=octocaris.exe` 的 `msedgewebview2.exe` 子进程。

## 基线

| 场景 | 单核 CPU | 说明 |
|---|---:|---|
| L2D 动画，未限帧 | **52.3%** | 主要来自 WebView2 GPU 进程 + L2D renderer |
| L2D，30 FPS | **41.4%** | 默认值 |
| L2D，15 FPS | **32.5%** | 更省电，适合常驻 |
| 静态图片壁纸（30 FPS 设置不生效） | **14.1%** | 几乎只有宿主/合成基线 |
| L2D + 全屏应用遮挡 | **13.4%** | 壁纸 `SetIsVisible(false)` 后动画停止 |
| `octocaris.exe` 本体（watchdog 等） | **~0.5%** | 不是主要成本 |

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
- `octocaris.exe` 本体空闲 CPU 约 0.5%，主要成本仍在 WebView2 渲染进程。

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
方法：10 秒采样 `octocaris.exe` + 所有带 `--webview-exe-name=octocaris.exe` 的 `msedgewebview2.exe` 的 `TotalProcessorTime` 增量，按单核归一化；内存为这些进程 WorkingSet 之和。

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

## 2026-10-24 Wallpaper Engine 对照（同一小桃女仆壁纸）

用户在 Wallpaper Engine 中打开 workshop `3253308696`（小桃女仆）后的实测（10 秒采样，按进程路径过滤 `wallpaper_engine`）：

| 进程 | 工作集 | 私有内存 | 单核 CPU |
|---|---:|---:|---:|
| `wallpaper64.exe`（主程序） | 56 MB | 24 MB | 0.0% |
| `webwallpaper64.exe` 宿主 | 134 MB | 50 MB | 1.2% |
| `webwallpaper64.exe` GPU 进程 | 947 MB | 851 MB | 22.7% |
| `webwallpaper64.exe` 页面渲染进程 | 178 MB | 61 MB | 21.1% |
| 网络/其他 `webwallpaper64.exe` | 66 MB | 25 MB | 0.3% |
| **合计** | **1381 MB** | **1012 MB** | **45.3%** |

对比 Octocaris 渲染同一张 `3253308696`：**1444 MB 工作集 / 27.8% CPU（30 FPS）**。

结论：两者内存几乎相同（Wallpaper Engine 略低约 4%），主要占用都来自 Chromium 的 GPU/渲染进程；这不是 Octocaris 的泄漏。CPU 差异来自帧率：Wallpaper Engine 不限制帧率（约 45%），Octocaris 默认 30 FPS（约 28%）；Octocaris 不限帧时 CPU 会更高。

## 2026-10-24 壁纸清晰度修复后的开销

把壁纸页 zoom 改为 `96 / dpi`（192 DPI 时 0.5），页面画布从 1440×900 提升到 2880×1800：

| 场景 | CPU（单核） | 工作集 |
|---|---:|---:|
| 修复前（zoom 1，画布 1440×900 上采样） | 27.8% | 1444 MB |
| 修复后（zoom 0.5，画布 2880×1800） | 26.9% | 1563 MB |

CPU 持平（填充率增加被更少的缩放开销抵消），内存增加约 120 MB，主要是更大的画布/纹理。细节与验证见 `docs/upstream-map.md` §4.6。

## 2026-10-24 壁纸库分页、轮换队列与整体静音

- **问题**：壁纸库一次渲染全部 45 个卡片，其中 37 个是 `preview.gif` 动图，WebView2 GPU 进程占用偏高，界面滚动/交互发沉。
- **分页**：壁纸库网格每页 24 个卡片、卡片放大，底部「上一页 / 下一页 + 第 X / Y 页」；同一时间最多 24 个动图预览。分页只影响浏览，不再承担排序。
- **顺序解耦**：轮换顺序改由右侧队列栏自身维护（连续的队列列表，不分页）。壁纸库只负责收藏和加入队列，因此“分页 vs 排序”的冲突从结构上消失；`libraryOrder` 不再参与轮换。
- **整体静音**：设置页开关 + 托盘「静音壁纸」菜单项，调用 WebView2 `ICoreWebView2_8::SetIsMuted`，对已存在的壁纸页立即生效，新建的 controller 在导航前就带上静音标志。实测日志：`wallpaper audio muted at creation: true`（含 `IsMuted` 回读）。

## 2026-10-25 双屏 L2D 采样

双屏：主屏 2880×1800 @200%，副屏 3840×2160 @150%，同一 L2D 壁纸（30 FPS）：

| 场景 | CPU（单核） | 工作集 |
|---|---:|---:|
| 双屏 L2D（8 个进程） | 66.9% | 1976 MB |
| 单屏 L2D（参考） | ~28% | ~1500 MB |

双屏的 CPU/内存基本随渲染面积线性增加；GPU 进程仍是共享的。副屏按 `GetDpiForMonitor` 得到 144 DPI、zoom 0.6667，页面视口等于物理 3840×2160。
