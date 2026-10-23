# 轻量模式（Lite Mode）

日期：2026-10-23

参考 Clash for Windows 的 Lightweight Mode：开启后关闭界面，只保留壁纸核心继续工作；再次启动时恢复界面。

## 入口

轻量模式**不在 Settings 里显示**，只从托盘菜单控制：

- 右键托盘图标 → **轻量模式**（带勾选状态）。
- 勾选后立即关闭前端并隐藏主窗口，壁纸继续渲染。
- 再次从托盘“Open Octos”或重新启动 `octos.exe` 会重建前端并显示窗口；轻量模式标记仍保留，关闭窗口会再次回到轻量状态。
- 取消勾选会恢复前端并保持窗口可见。

## 行为

开启轻量模式时：

1. 强制开启 **全屏时暂停** 和 **内存节省**（Settings 中这两项显示为开且禁用）。
2. 调用 `ICoreWebView2Controller::Close()` 销毁主界面 WebView，并隐藏主窗口；
   - 壁纸 WebView 不受影响，继续在桌面渲染。
3. 托盘图标保留，右键菜单显示 `轻量模式` 勾选状态。
4. 主界面的 renderer 进程退出；壁纸 renderer 继续工作。
5. 壁纸帧率不变，仍是独立的 `maxFps` 设置。

重新打开时：

- `WM_APP_RESTORE`（托盘“Open Octos”/第二实例）调用前端工厂重建 `ICoreWebView2Controller`。
- 不重启进程，不重新加载壁纸；prefs 中的显示器选择、选项和窗口位置都保留。

## 与 MemorySaver 的区别

| | 轻量模式 | 内存节省（非轻量） |
|---|---|---|
| 关闭到托盘 | 销毁主 UI WebView | 挂起主 UI WebView（`TrySuspend`） |
| 壁纸 | 继续渲染 | 继续渲染 |
| 恢复 | 重建 controller | `Resume` + 显示 |
| 关闭 Settings 入口 | 无（托盘菜单） | 有 |

两种模式都不再对隐藏壁纸做 `TrySuspend`；全屏暂停只隐藏壁纸。

## 实测

环境：`OCTOS_DATA_DIR=E:\file\octos\data`，DISPLAY2 2880×1800 @ 200%，L2D 壁纸。

1. 托盘菜单显示 `Open Octos` / `轻量模式`（勾选）/ `Quit`。
2. 勾选轻量模式：
   - 日志 `frontend closed (lite mode)`；
   - `MainWindowHandle=0`；
   - WebView2 renderer 从 2 个变为 1 个：主 UI renderer（约 82 MB）退出，L2D renderer（约 206 MB）继续；
   - 桌面壁纸仍在渲染。
3. 取消勾选：
   - 日志 `recreating frontend` → `React UI is ready`；
   - 窗口恢复，UI 正常发送 `ping/get-settings/get-library`。
4. 轻量模式再次开启/关闭多次，壁纸未中断。

截图：`E:\file\octos\m5-lite-tray-menu.png`、`E:\file\octos\m5-settings-final.png`。

## 限制

- 前端关闭期间没有 Settings/Library，也没有前端 toast；导入/设置需先恢复界面。
- 通知、后台检查更新等前端功能在轻量模式下不可用（与参考实现一致）。
- 若主 UI 与壁纸共享 renderer 进程，销毁主 controller 的 RSS 收益会受进程模型影响；当前实测主 UI renderer 独立退出。
- 每次恢复会重建 React UI，首次显示比 `TrySuspend` 略慢。
