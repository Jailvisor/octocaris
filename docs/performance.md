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

### 4. MemorySaver 与轻量模式（2026-10-23）

**MemorySaver（默认开）**

- 壁纸因全屏/隐藏而 `SetIsVisible(false)` 超过 **3 秒**后，对该显示器调用 WebView2 `TrySuspend`；重新显示时先 `Resume` 再 `SetIsVisible(true)`。
- 快速显示/隐藏不会立刻挂起，避免来回切换时反复销毁/重建渲染状态。
- 主窗口关闭到托盘时也会挂起主 UI 的 WebView；从托盘恢复时 `Resume`。
- 启动时应用 `appOptions.memorySaver`；`liteMode` 开启时强制为开。

**轻量模式（`liteMode`）**

- 只做两件事：简化 React UI（不加载卡片缩略图、关闭动效/模糊、隐藏状态面板）+ 打开 MemorySaver。
- **不改变壁纸帧率**；帧率仍是独立的 `maxFps` 设置（实测切到轻量模式后仍为 30 FPS）。
- 轻量模式开启时“内存节省”开关强制为开并禁用，避免 UI 显示关、运行时却挂起的不一致。
- 关闭轻量模式不会自动关闭内存节省；内存节省可以单独关。

**实测（L2D 4K，DISPLAY2 2880×1800 @ 200%）**

用 7 秒全屏窗口触发隐藏，采样 `octos.exe` + 其 `msedgewebview2.exe` 子进程的 WorkingSet：

| 状态 | 内存 |
|---|---:|
| 壁纸可见 | 1098 MB |
| 全屏隐藏、挂起后 | 1013 MB |
| 退出全屏、恢复后 | 1054 MB |

日志证据：`fullscreen=true` → 3 秒后 `suspending hidden wallpaper` → `fullscreen=false` 后立即 `resumed wallpaper`（07:59:04Z / 07:59:07Z / 07:59:11Z）。这次只隐藏了 7 秒，释放量有限；更长的锁屏/全屏停留会释放更多。

## 如何选择

- **最轻量**：静态图片壁纸（`添加图片`），约 14% 单核。
- **动画 + 平衡**：L2D + 30 FPS（默认），约 41%。
- **动画 + 省电**：L2D + 15 FPS，约 33%。
- **全屏游戏/视频**：自动暂停，约 13%。

## 后续可做

1. 限制同时动画的壁纸数量/分辨率，或为 mod 提供质量档位。
2. 若 L2D 是主要使用场景，优先建议 mod 使用 2K 资源而不是 4K/8K；宿主无法替 mod 降资源。
3. 在 15/30 FPS + MemorySaver 组合下做更长时间（锁屏/全屏 30 分钟）的内存曲线，确认 renderer 真正回收。
