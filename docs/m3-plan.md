# M3 Plan — 协议与 Mod API 兼容

目标：让现有 Octos mod 和旧 `octos.min.js` 在 Rust 宿主上按原语义运行。M2 已提供可用的壁纸窗口/输入/恢复；M3 补协议和业务层。

## 进度

- [x] `sdk/` 冻结旧 JS API（src + rollup + README）
- [x] `octos-core::manifest`：octos.json/mod.json、legacy prefs、entry/image、未知字段、confined path、虚拟 URL
- [x] manifest fixtures + 7 个单测
- [x] `octos-core::catalog/options/session/hooks` + 单测（core 共 20 个测试）
- [x] `octos-protocol` 类型 + `WallpaperHost` 分发 + mock 单测（9 个协议测试）
- [x] 壁纸宿主 `System`/`UserOptions`(requestOptions/setOption)/`MonitorBridge` + 媒体桩
- [x] 旧 `octos.min.js` conformance：source/theme/visibility/desktop-icons/monitor-id/siblings/options/media 桩全部通过
- [x] 对齐报告：`docs/m3-conformance.md`
- [x] UserOptions `load`/`change` 事件 + hooks（旧 SDK 实测通过）
- [ ] MonitorBridge redirect（需要第二显示器/多实例）
- [ ] options 持久化（prefs/M5）
- [ ] 旧 `octos.min.js` 一致性 mod 验收

## 范围

### 1. SDK 冻结

- 原样复制上游 `src/js/*` 到 `sdk/src/`，保留 rollup 配置。
- 产出仍叫 `octos.min.js`，UMD global `octos`。
- M3 的验收基准是**旧 bundle**，不先重写 API。

### 2. `octos-protocol`

- mod 侧消息类型：
  - `request/response`（`requestId`、`requestType`）
  - `command/commandType`
  - `subscribe/unsubscribe`（`eventType`）
  - `event/eventType`
- 事件：`mediaChange`、`playbackChange`、`timelineChange`、`audioStream`、`options-change`、`options-load`、`redirect`。
- `WallpaperHost` trait：`source/theme/media/options/visibility/desktop_icons/siblings/monitor_id/redirect`。
- `dispatch(msg, &mut dyn WallpaperHost)`：可在 Linux 用 mock host 单测完整分支。
- 媒体/音频桩：request resolve 空/默认值，command 静默忽略，订阅接受不推送。

### 3. `octos-core`

- `manifest`：`octos.json`/`mod.json`、legacy `prefs`、entry/image 解析、未知字段保留。
- `catalog`：扫描 wallpapers，生成卡片数据；无 entry 的 mod 标记不可用。
- `options`：默认值合并、用户值保留、字符串化（`true` → `"true"`、`3` → `"3"`）。
- `session`：显示器↔mod 选择、siblings、redirect、options 路由。
- `hooks`：`onchange/onload/onchangeload` 脚本生成 + JSON 转义；顺序 event → onchange/onload → onchangeload。
- 单测覆盖：manifest fixtures、options、hooks、session、协议 serde。

### 4. 壁纸宿主接线

- `octos-webview` 壁纸 controller 的 `WebMessageReceived` 交给 `protocol::dispatch`。
- `System`：source/theme/visibility/desktop-icons/devtools。
- `UserOptions`：requestOptions 由 Rust 直接回答；`setOption` 更新存储、不发 change；UI 改选项才发 event + hooks。
- `MonitorBridge`：monitor-id/siblings/redirect。
- `MediaController`/`audioStream`：协议桩。
- 选项状态由 Rust 持有（M3 先用内存 + `preferences.json` 简单持久化，M5 再做完整迁移）。

### 5. 验收

- 旧 `octos.min.js`（本地 + CDN）在壁纸中加载。
- 一致性 mod 覆盖：
  - `System.getSource/getTheme/getVisibility/setVisibility/setDesktopIconVisibility`
  - `UserOptions.requestOptions/on('load'/'change')/setOption`
  - `MonitorBridge.requestId/requestSiblingIds/send`
  - 媒体 request 不挂起
- 日志/结果文件可由 WSL 读取；Desktop 行为仍用 `scripts/verify-desktop.ps1`。
- `cargo fmt --check`、clippy、Linux 单测通过。

## 顺序

1. `sdk/` 冻结 + fixtures。
2. `octos-protocol` 类型 + mock host + serde 单测。
3. `octos-core::manifest/catalog/options/hooks/session` + 单测。
4. 壁纸宿主接线 `System/UserOptions/MonitorBridge` + 媒体桩。
5. 一致性 mod 与旧 bundle 验收。
6. 写 M3 验收报告，进入 M4（Library/CLI）。

## 非目标

- 真实 GSMTC/WASAPI（M6）。
- 完整 prefs 迁移/原子写（M5）。
- Explore/社区（M7）。
- React Library 安装/选项 UI（M4）。
- 新 mod 格式或新 API 版本。

## 风险

- 旧 bundle 的字段名/时序必须逐字保留；先以 fixtures/一致性 mod 锁定。
- CompositionController 的输入坐标与 CSS 坐标差异（物理 vs CSS）已在 M2 记录。
- options 由 Rust 直接回答后，不能再依赖旧 App 窗口的 options 路由。
- hooks 的字符串化语义容易“现代化”改坏；必须按旧模板行为测试。


## UX 人工审查时机（建议）

1. **现在（M2 完成 / M3 开始）——轻量早期审查**
   - 桌面交互是否正确：壁纸在图标下方、图标可点、右键刷新、壁纸页面可点。
   - App 外壳是否顺手：窗口大小/边框、最小化、托盘、第二实例、Settings 开关。
   - 只收集“交互模型/布局方向”的问题，不要求最终视觉完成度。

2. **M4 完成——完整 UI 审查**
   - Library、mod 安装/卸载、按显示器选择、选项表单、CLI。
   - 这是第一次功能完整的端到端体验，适合逐项提 UX 问题。

3. **M5 完成——最终验收**
   - 多显示器/全屏/Explorer 重启/prefs 迁移下的稳定性与体验。
   - 性能/省电、文案、空态/错误态、键盘/可访问性。

当前 UI 仍是 M1 shell：Library 为空态、Explore/Settings 大部分是占位；现在不要按最终产品标准评价。
