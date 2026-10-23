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

**轻量模式（`liteMode`）**

- 简化 React UI：不加载卡片缩略图、关闭动效/模糊/阴影、隐藏状态面板。
- 强制开启 **全屏时暂停** 和 **内存节省**；这两个开关在轻量模式下显示为开且禁用。
- **不改变壁纸帧率**；帧率仍是独立的 `maxFps` 设置。
- 关闭轻量模式不会自动关闭全屏暂停/内存节省，它们可以继续独立控制。

**实测**

- 关闭到托盘：日志 `main UI suspended (window hidden)` → `main UI suspend completed: true`；桌面截图确认壁纸仍在渲染。
- 第二实例恢复：日志 `main UI resumed`，窗口恢复。
- WorkingSet 不会立即下降：主 UI 与壁纸共享 WebView2 进程/GPU 资源，挂起的收益主要是停止主界面页面工作与合成，而不是固定的 RSS 降幅。后续若要强制回收内存，需要关闭/重建主 controller，代价是恢复延迟。

## 如何选择

- **最轻量**：静态图片壁纸（`添加图片`），约 14% 单核。
- **动画 + 平衡**：L2D + 30 FPS（默认），约 41%。
- **动画 + 省电**：L2D + 15 FPS，约 33%。
- **全屏游戏/视频**：自动暂停，约 13%。

## 后续可做

1. 限制同时动画的壁纸数量/分辨率，或为 mod 提供质量档位。
2. 若 L2D 是主要使用场景，优先建议 mod 使用 2K 资源而不是 4K/8K；宿主无法替 mod 降资源。
3. 如需进一步降低托盘常驻内存，评估关闭并重建主 WebView controller 的方案（恢复延迟 vs 内存）。
