# 多显示器支持

日期：2026-10-23
环境：DISPLAY2（主屏）2880×1800 @ 200%，DISPLAY1 3840×2160 @ 150%。

## 1. 实测拓扑

| | DISPLAY2（主屏） | DISPLAY1 |
|---|---|---|
| 物理分辨率 | 2880×1800 | 3840×2160 |
| DPI | 192（200%） | 144（150%） |
| CSS viewport | 1440×900（16:10） | 2560×1440（16:9） |
| devicePixelRatio | 2 | 1.5 |

壁纸页面由 WebView2 按各屏 viewport 渲染，因此同一壁纸在不同屏上的布局/缩放可能不同。

## 2. 已修复的问题

### 2.1 槽位按设备名优先匹配

原 `reconcile` 对每个显示器先按设备名匹配，再按矩形兜底，两个分支在同一次循环里执行。显示器顺序变化（例如主屏从 DISPLAY1 变为 DISPLAY2）时，矩形兜底会把 `DISPLAY1` 的槽位“抢”给 `DISPLAY2`，导致真正的 `DISPLAY1` 变成没有壁纸的新槽位。

修复：两遍匹配。第一遍只按设备名匹配；第二遍才用矩形兜底；仍未匹配的才创建新槽位。

### 2.2 热插拔通知与默认壁纸

- `WM_DISPLAYCHANGE` 现在除 reconcile 外，还会调用 app 层的显示变化处理器。
- 处理器：
  - 对新显示器，若 prefs 有保存的选中项则恢复；
  - 否则取主屏当前 mod；主屏也没有则用内置 gradient；
  - 恢复/分配后保存 prefs，并把最新的 Library 快照推给 UI。
- 移除显示器时保留其 prefs 选择，重新插回时恢复。
- `library_snapshot` 的 `selected` 只包含当前存在的显示器，避免 UI 显示已移除的屏幕。

### 2.3 控制器创建竞态

启动两个显示器时，第二个 `ensure_webview` 可能在同一 HWND 上再次创建 CompositionController，触发 `DCOMPOSITION_ERROR_WINDOW_ALREADY_COMPOSED (0x88980800)`。

修复：每个槽位增加 `creating` 门闩；创建回调成功/失败都会清除；同步创建失败进入 1 秒重试。启动日志现在两个 controller 都一次成功。

### 2.4 全屏误判

`ShellHandwritingCanvas`（Windows 手写系统窗口）会铺满主屏，被全屏检测误判并隐藏壁纸。已加入类名排除。

### 2.5 诊断脚本

`scripts/verify-desktop.ps1` 现在：

- 枚举所有显示器，按 monitor rect 匹配壁纸窗口和 WebView2 Chrome host；
- SUMMARY 增加 `monitors / wallpaperMonitors / chromeMonitors`；
- 跳过隐藏 IME/helper、其他 WebView2 host、任务栏等不遮挡桌面的窗口来判断 `reachesProgman`；
- 全屏隐藏壁纸时给出专门的 PASS 文案。

## 3. 实测结果

| 场景 | 结果 |
|---|---|
| 启动两屏 | 2 个壁纸窗口、2 个 Chrome overlay，`wallpaperMonitors=2 chromeMonitors=2`，PASS |
| 独立选择 | DISPLAY1=L2D、DISPLAY2=Gradient；通过 UI 把 DISPLAY2 改为 L2D，prefs 两条选择分别保存 |
| 跨屏 redirect | 临时 mod 在两屏运行，互为 siblings；双向 `redirect` 命令/事件都收到，`sender-id` 正确 |
| 重启持久化 | 两屏都选 L2D，重启后两个 controller 都恢复，PASS |
| 热插拔 | `DisplaySwitch /internal` 后只剩 DISPLAY2，UI 更新为 1 屏；`/extend` 后自动恢复 DISPLAY1 的 L2D，controller 重建，PASS |
| 鼠标路由 | 光标移到 DISPLAY1 区域，日志显示转发到对应壁纸 HWND |

## 4. 每屏 localStorage 隔离（已实现）

L2D 壁纸把 `scale/x/y` 等设置存在 **localStorage**，而所有显示器共用 `https://local.octos` origin，存储原本是共享的。同一份设置被应用到 1440×900（DPR 2）和 2560×1440（DPR 1.5），加上 16:10 / 16:9 比例差异，就会出现位置/大小不一致。

实现：

- 壁纸 WebView 创建时用 `AddScriptToExecuteOnDocumentCreated` 注入 `octos_core::storage_scope::script`。
- 脚本在页面脚本之前包装 `Storage.prototype.getItem/setItem/removeItem/clear/key/length`，key 统一加 `__octos_monitor__<device>:` 前缀。
- 旧的无前缀 localStorage 值会在首次加载时复制到每块屏的命名空间，保留升级前设置。
- 已修复首版迁移造成的嵌套前缀，并在脚本启动时自愈清理嵌套 key。
- 只影响壁纸 WebView；主 UI 不注入。origin、mod 协议、`local.octos` 都不变。

实测：

| 测试 | 结果 |
|---|---|
| 新 key 首次写入 | 两屏 `existing=null`，分别写入自己的 monitor id |
| 重启后 | DISPLAY1 读到 DISPLAY1，DISPLAY2 读到 DISPLAY2 |
| 旧 L2D 设置迁移 | 两屏都能读到 `CH0326_home.skel` 的 `scale=0.42 x=727 y=798 resolution=4k` |
| 清理 | 测试 key 删除后每屏只剩自己的 L2D 设置命名空间 |

现在可以在每块屏打开 L2D 自带的设置面板（鼠标移到右上角），分别调整 `scale/x/y`；两块屏互不覆盖，重启后各自保留。

**边界**：该方案覆盖 localStorage/sessionStorage。若某个壁纸使用 IndexedDB 或 cookie 存设置，需要后续扩展；若壁纸没有内部设置，则需要宿主级 zoom/offset 方案。

## 5. 未覆盖

- 三块及以上显示器。
- 不同方向的排列（上方/下方/左侧）。
- 每屏不同刷新率/可变刷新率。
- 每屏 DPI 变化时的完整回归（当前 200%/150% 已覆盖）。
