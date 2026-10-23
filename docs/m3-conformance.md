# M3 Conformance — Rust vs C++

日期：2026-10-22
范围：壁纸内 mod 可见协议。对照 upstream `src/API/API.cpp`、`src/Bridge/Bridge.cpp`、`src/js/*`。

## 1. Request 面（已对齐）

| requestType | C++ 行为 | Rust 行为 | 状态 |
|---|---|---|---|
| `source` | `ICoreWebView2::get_Source` | `webview.Source()` | ✅ |
| `media-props` | GSMTC 当前媒体属性 | 空 `MediaProperties` 桩 | ✅ 形状对齐；数据 M6 |
| `playback-info` | GSMTC 播放信息 | `Closed/1.0/false/Unknown` 桩 | ✅ |
| `timeline-props` | GSMTC 时间轴 | 全 0 桩 | ✅ |
| `thumbnail` | base64 缩略图 | 空字符串桩 | ✅ |
| `is-theme-light` | 注册表 `AppsUseLightTheme` | 同左 | ✅ |
| `visibility` | controller `IsVisible` | `WallpaperWebView::is_visible` | ✅ |
| `siblings` | 同 mod 的其他显示器 | `Session::siblings` | ✅（单显示器为空） |
| `monitor-id` | `\\.\DISPLAYn` | slot 的 `device_name` | ✅ |
| `desktop-icons-visibility` | `SysListView32` 可见性 | `desktop_icons::is_visible` | ✅ |
| `options` | App 窗口从 prefs 读 `modOptions` | Rust `Session::options_for`，无 mod 返回 `{}` | ✅ 契约一致；来源改为 Rust |

响应 envelope 保持 `{type:"response", requestId, requestType, data}`，`requestId` 原样回显。

## 2. Command 面

| commandType | C++ 行为 | Rust 行为 | 状态 |
|---|---|---|---|
| `open-dev-tools` | 打开当前壁纸 DevTools | 同左 | ✅ |
| `media` | GSMTC 控制（play/pause/next/...） | 静默忽略（M6） | ✅ 协议桩 |
| `set-visible` / `set-hidden` | 当前壁纸 controller 可见性 | slot `user_visible` + `apply_visibility` | ✅ |
| `set-desktop-icons-visible/-hidden` | ShowWindow `SysListView32` | `desktop_icons::set_visible` | ✅ |
| `set-option` | 转发给 App，写 prefs，不发 change | 写 Rust Session，不发 event | ✅ 语义一致 |
| `redirect` | 给目标显示器发 `redirect` event | 查 slot，发 `redirect_event` | ✅ 逻辑已接；单显示器无法实测 |
| `send-to-monitor-hwnd` | 旧 App 内部转发 | 忽略 | ✅ 不需要（Rust 直接 routing） |
| 未知 command | 忽略 | 忽略 | ✅ |

## 3. Subscription / Event

| eventType | C++ | Rust | 状态 |
|---|---|---|---|
| `options-change` | UI 改选项时发 | `apply_option_from_ui` 已接线并测试 | ✅ |
| `options-load` | 壁纸导航完成后发 | `handle_wallpaper_loaded` 已接线并测试 | ✅ |
| `redirect` | 跨显示器消息 | host 已发送，字段 `sender-id/receiver-id/message` 对齐 | ✅ 形状；多显示器未测 |
| `mediaChange/playbackChange/timelineChange` | GSMTC 推送 | 订阅接受，不推送（桩） | ✅ 不挂起；数据 M6 |
| `audioStream` | WASAPI PCM 块 | 订阅接受，不推送（桩） | ✅ 不挂起；数据 M6 |

`onchange/onload/onchangeload` 的脚本生成、字符串化和 JSON 转义已在 `octos-core::hooks` 实现并测试；现在由 `apply_option_from_ui` / `handle_wallpaper_loaded` 执行。

实测：
- 加载时旧 SDK `UserOptions.on('load')` 收到 `speed=3`、`dark=true`；日志 `hook-fired onchangeload "3"`。
- UI-originated 选项改为 5 后，`on('change')` 收到 `speed=5`；日志 `hook-fired onchange "5"` → `onchangeload "5"`。
- 旧 `octos.min.js` 的 `requestOptions/setOption` 与事件监听都正常。
- 测试触发命令 `octos-internal-apply-option` 仅在 `OCTOS_TEST_HOOKS=1` 时启用，不属于 mod 协议。

## 4. 实测

- 用 `sdk/` 构建的**原版** `octos.min.js`（5.0 KB）加载到 `assets/wallpapers/conformance/`。
- `OCTOS_TEST_MOD_FOLDER=conformance`、`OCTOS_LOG=debug` 启动。
- 结果：`System`（source/theme/visibility/desktop icons）、`MonitorBridge`（id/siblings）、`UserOptions`（requestOptions/setOption）、`MediaController`（四个 request）全部拿到正确响应。
- options 默认值从 `octos.json` 加载；`set-option speed=4` 后再次 `requestOptions` 返回 `value: 4`。
- 证据：`E:\file\octos\m3-conformance.png`；`E:\file\octos\data\logs\octos.log.2026-10-22` 中的 `wallpaper protocol response`。

> `assets/wallpapers/conformance/` 是**测试用** mod；M4 的 Library/安装包应把它标记为 test-only 或排除，不要作为正式内置壁纸发布。

## 5. 尚未对齐 / 延后

1. **真实媒体/音频**：GSMTC/WASAPI 在 M6。
2. **多显示器 redirect**：当前只有 `\\.\DISPLAY1`；逻辑/单测已覆盖，实机未测。
3. **options 持久化**：已在 M4 完成（`preferences.json` v2 + 启动恢复 + appOptions）；写盘 debounce/`.bak` 仍在 M5。
4. **ProcessFailed renderer 恢复**：M2 已知缺口。
5. **`set-visible`/`set-hidden` UI 回归**：协议单测覆盖，conformance 页面未自动点。

结论：**协议请求面已经和 C++ 版对齐并有旧 SDK 实测证据；事件/hooks 的触发时机和真实媒体数据是下一阶段工作。**
