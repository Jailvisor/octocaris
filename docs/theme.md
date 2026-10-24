# 主题（深色 / 浅色）

日期：2026-10-23

前端主题跟随 Windows 应用主题（`HKCU\...\Themes\Personalize\AppsUseLightTheme`），无需手动切换。

## 前端

- `ui/src/styles.css` 的 `:root` 定义深色 token，`@media (prefers-color-scheme: light)` 覆盖为浅色 token 和关键组件。
- `color-scheme: light dark` 让原生控件、滚动条和表单跟随。
- WebView2/Chromium 会根据 Windows 主题更新 `prefers-color-scheme`；实测切换系统主题时界面**无需重启**即时更新。
- 预览图、卡片、设置页、拖拽遮罩、toast、骨架屏都有浅色适配。
- 原生 `<select>` 下拉列表单独处理：控件上显式设置 `color-scheme: dark`（浅色主题下为 `light`），并给 `option` 设置 `background-color: var(--panel-solid)` / `color: var(--text)`；否则下拉弹层会沿用浏览器默认的浅色底。

## 标题栏

- `octos-win` 读取 `AppsUseLightTheme`，调用 `DwmSetWindowAttribute(DWMWA_USE_IMMERSIVE_DARK_MODE)`。
- 监听 `WM_SETTINGCHANGE` / `WM_THEMECHANGED`，系统切换主题时即时更新。

## 托盘菜单

- 原生 Win32 菜单通过 uxtheme 的 `AllowDark`（ordinal 135）和 `FlushMenuThemes`（ordinal 136）跟随系统深色；浅色系统下保持浅色。
- 菜单项已中文化：`打开 Octos` / `轻量模式` / `退出`，并加分隔线。
- 托盘 tooltip 在轻量模式时为 `Octos · 轻量模式`。

## 验证

| 系统主题 | 结果 | 证据 |
|---|---|---|
| 深色 | UI 深色、标题栏深色、托盘菜单深色 | `E:\file\octos\m5-theme-dark.png`、`m5-tray-menu-dark.png` |
| 浅色 | UI 浅色、标题栏浅色、设置页可读 | `E:\file\octos\m5-theme-light.png` |
| 深色下拉框 | 原生下拉弹层为深色底、浅色文字，选中项高亮正常 | `E:\file\octos\ui-select-popup-dark.png` |

切换方式：设置 `AppsUseLightTheme`/`SystemUsesLightTheme` 并广播 `WM_SETTINGCHANGE("ImmersiveColorSet")`；测试后已恢复原值。

## 限制

- 目前没有手动“强制深色/浅色”选项，完全跟随系统。
- 托盘菜单深色依赖 uxtheme 未文档化 ordinal；失败时回退到系统默认菜单样式。
