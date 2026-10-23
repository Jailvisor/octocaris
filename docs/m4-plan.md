# M4 Plan — React Library + 安装/选项/CLI

目标：把 M2/M3 的运行时能力变成可用的桌面 App：扫描 Library、安装/卸载 mod、按显示器选择、编辑选项、设置、CLI。

## 已完成

- [x] i18n 骨架：`react-i18next` + en/zh-CN，Settings 语言选择，52+ key 对齐。
- [x] 后端 `GetLibrary` 快照：mods、monitors、selected、options、errors。
- [x] `SetMonitorMod`：按显示器选择 mod，加载 manifest options，重建 controller。
- [x] `SetOption`：UI-originated 选项更新，走 `apply_option_from_ui` → `options-change` + hooks。
- [x] React Library：卡片网格、详情、显示器选择、设为壁纸、显示器分配、选项表单（checkbox/range/number/select/color/text/description）。
- [x] 内置 Gradient 增加 speed/accent 选项，作为选项链路测试。
- [x] 测试用 conformance mod 从 Library 快照过滤，避免作为正式内置壁纸。
- [x] zip 安装/卸载：path traversal/绝对路径/symlink/zip bomb 防护、单顶层目录拍平、`RemoveMod` 仅允许 wallpapers 根目录内路径；安装要求 `octos.json`/`mod.json`，防止把普通目录当 mod。
- [x] 文件夹导入：`install_folder` 安全递归复制（拒绝 symlink/reparse point、文件数/大小上限、替换已有同名 mod），UI “导入文件夹”按钮 + `rfd` 目录选择。
- [x] 拖拽导入：页面 `drop` → `chrome.webview.postMessageWithAdditionalObjects` → Rust `ICoreWebView2File::Path`；支持文件夹和 `.zip`。主窗口 `WM_DROPFILES` 作为标题栏/非客户区兜底。拖入时显示“松开以导入壁纸”遮罩和 copy 光标。
- [x] 安装错误 toast：`UiResponse::Notice` + UI 自动消失提示；不再把安装失败误报成宿主断连。
- [x] 主窗口 DPI 修复：PMv2 下 `GetDpiForSystem` 返回 96，改用 `GetDpiForWindow` 并按工作区钳制/居中；200% 缩放下窗口不再只有 550 CSS px。
- [x] Library 布局修复：移除 M1 hero 横幅、`apply-row` 窄面板换行、显示器下拉恢复可读宽度；里程碑文案更新到 M4。
- [x] 真实社区 mod 回归：导入 L2D 碧蓝档案包、设为壁纸并成功渲染；用户确认文件夹和 zip 拖拽均可导入。
- [x] 卸载验证：`Test Mod` 经 UI 确认后删除文件夹，Library 从 3 个 mod 降为 2 个。
- [x] 选项链路验证：UI Automator 将 Animation speed 从 26 改为 42，宿主收到 `set-option` 并返回新快照。
- [x] 误装防护：安装/导入必须包含 `octos.json` 或 `mod.json`；把 React 的 `app/` 目录拖进来会被拒绝并显示错误 toast。
- [x] 选项持久化：`preferences.json` v2（`octos-core::prefs`），原子写、损坏隔离、未知字段保留；启动恢复 `selected`/`modOptions`，变更即保存。
- [x] appOptions：`runOnStartup`、`enableGPU`、`pauseFullscreen` 持久化；未知 `appOptions` 键原样保留。
- [x] CLI：`run [folder] [--dev-tools]`、`reload`、`new [folder]`、`dev-tools`；第二实例通过 `WM_COPYDATA` 转发完整命令行。
- [x] Settings 实装：开机启动（HKCU Run）、全屏暂停（立即生效）、硬件加速（重启生效）三个真实开关；`允许通知` 按用户决定不做，保留 M8 占位。
- [x] UI 细节（Win11 Fluent）：Mica 风格分层背景、Segoe UI Variable、4/8px 控件几何、焦点环、细滚动条、减少动效；卡片图片懒加载/淡入/失败占位；Library 骨架屏与空态；安装中 spinner + 成功/错误 InfoBar 式 toast；方向键在卡片间移动并按焦点切换详情；默认选中当前正在使用的壁纸。
- [x] 窗口位置恢复：保存 `WINDOWPLACEMENT.rcNormalPosition` 到 prefs 的 `window`，退出时落盘，启动时校验显示器后恢复；最大化时保留还原尺寸。

## 下一步

1. **多显示器回归**：按显示器选择、`redirect`、热插拔（当前环境只有 DISPLAY1）。
2. **M5 硬化**：prefs 写盘 debounce/.bak、全屏真实游戏回归、ProcessFailed 恢复。
3. `允许通知` 推迟到 M8；`memorySaver` 留在 M6。
4. 用真实社区 mod 做多显示器/选项回归。

## 验收

- 从 UI 安装一个 zip、选择显示器、设置壁纸、改选项（事件/hooks 生效）、重启后保留。
- Library 正确显示卡片/图片/作者/版本；损坏 mod 有错误提示。
- 测试 mod（conformance）不出现在正式 Library。
- i18n 中英切换即时生效；无缺失 key。
- `cargo fmt`/clippy/单测、`npm run typecheck`/build 通过。
