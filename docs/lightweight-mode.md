# 轻量模式（Lite Mode）

日期：2026-10-23

参考 Clash for Windows 的 Lightweight Mode：轻量模式是一个**托盘会话状态**，开启后关闭前端，只保留壁纸核心继续工作；重新打开前端或重启应用即回到普通模式。

## 入口

轻量模式不在 Settings 里显示，只从托盘菜单控制：

- 右键托盘图标 → 菜单：`打开 Octos` / `轻量模式`（勾选状态）/ `退出`。
- 勾选“轻量模式”：
  - 立即销毁主 React UI WebView（`ICoreWebView2Controller::Close()`）；
  - 隐藏主窗口、进入托盘；
  - 壁纸核心继续运行，桌面壁纸不中断；
  - 强制开启“全屏时暂停”和“内存节省”。
- 再次从托盘“打开 Octos”或重新启动 `octocaris.exe`：
  - 重建前端并显示窗口；
  - 自动退出轻量模式，托盘勾选消失；
  - 壁纸选择、选项、窗口位置保留。
- 完全退出应用再启动：同样回到普通模式；轻量模式不写入 `preferences.json`。

## 不做的界面简化

轻量模式**不简化 React UI**：

- 卡片预览图始终加载（懒加载 + 淡入 + 失败占位）；
- 动效、状态面板、布局保持不变。

轻量化的收益来自关闭前端，而不是牺牲预览。

## 与 MemorySaver 的区别

| | 轻量模式 | 内存节省（普通模式托盘） |
|---|---|---|
| 关闭到托盘 | 销毁主 UI WebView | 挂起主 UI WebView（`TrySuspend`） |
| 壁纸 | 继续渲染 | 继续渲染 |
| 恢复 | 重建 controller | `Resume` + 显示 |
| 生命周期 | 本次会话；打开前端/重启即退出 | 持久化设置 |
| 入口 | 托盘菜单 | Settings |

两种模式都不对隐藏壁纸做 `TrySuspend`；全屏暂停只隐藏壁纸。

## 实测

环境：`OCTOS_DATA_DIR=E:\file\octos\data`，DISPLAY2 2880×1800 @ 200%，L2D 壁纸。

1. 勾选轻量模式：
   - 日志 `frontend closed (lite mode)`；
   - `MainWindowHandle=0`；
   - WebView2 renderer 从 2 个变为 1 个：主 UI renderer（约 82 MB）退出，L2D renderer（约 206 MB）继续；
   - 桌面壁纸仍在渲染。
2. 取消勾选或“打开 Octos”：
   - 日志 `recreating frontend` → `React UI is ready`；
   - 窗口恢复，UI 正常发送 `ping/get-settings/get-library`。
3. 轻量 → 退出应用 → 重新启动：
   - 直接显示完整 UI（预览图正常）；
   - 托盘“轻量模式”**不再勾选**；
   - `preferences.json` 中没有 `liteMode`。
4. 轻量开启/关闭多次，壁纸未中断。

截图：`E:\file\octos\m5-lite-menu-checked-zh.png`、`m5-lite-restart-unchecked.png`、`m5-tray-menu-dark.png`。

## 限制

- 前端关闭期间没有 Settings/Library，也没有前端 toast；导入/设置需先恢复界面。
- 前端功能（通知、更新检查等）在轻量期间不可用。
- 每次恢复会重建 React UI，首次显示比 `TrySuspend` 略慢。
- 托盘菜单是原生 Win32 菜单；深色模式下通过 uxtheme 的 `AllowDark`/`FlushMenuThemes` 跟随系统主题。
