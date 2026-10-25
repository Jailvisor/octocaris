# M1 Shell Report

日期：2026-10-22
状态：**完成**，质量门禁通过。

## 范围

M1 目标是把 M0 的 spike 变成一个可持续开发的 Rust workspace + 可启动宿主：

- Rust workspace 与 crate 拆分。
- Windows GUI 入口、日志、单实例。
- 主窗口（标准 Win32 边框）+ 普通 WebView2 controller。
- `app.octos` / `local.octos` 虚拟主机。
- React 19 + TypeScript + Vite 壳。
- 原生托盘图标、关闭到托盘、干净退出。
- `cargo xtask build-ui|build|deploy|smoke`。

## 构建与运行

```bash
cd /home/lzt/codehub/rust/octos/octos-rs
cargo xtask deploy
powershell.exe -NoProfile -Command "\$env:OCTOS_DATA_DIR='E:\file\octos\data'; Start-Process 'E:\file\octos\octocaris.exe'"
```

部署目录：

```
E:\file\octos\
├── octocaris.exe
├── app\              # React 构建产物
├── data\             # OCTOS_DATA_DIR（日志、WebView2 缓存）
└── (wallpapers\ 由后续 milestone 生成)
```

## 已验证

| 项 | 结果 | 证据 |
|---|---|---|
| workspace 编译（Windows target） | 通过 | `cargo check --workspace --target x86_64-pc-windows-msvc` |
| UI 构建 | 通过 | `npm run build`，Vite 8.3.5 |
| 主窗口 | 通过 | `Get-Process octos`，标题 `Octos` |
| WebView2 普通 controller | 通过 | 日志 `main WebView2 controller ready -> https://app.octos/index.html` |
| React UI 渲染 | 通过 | `E:\file\octos\m1-ui-2.png`，Library/Settings 壳 |
| `app.octos` 虚拟主机 | 通过 | UI 从 `https://app.octos/index.html` 加载 |
| React → Rust ping | 通过 | 日志 `ui -> host: {"type":"ping","id":...}`；UI 显示 `Rust host connected` |
| Rust → React pong | 通过 | UI 显示 `Host build 0.1` / `Transport WebView2 postMessage` |
| DPI 缩放下窗口尺寸 | 通过 | 200% 缩放下按 1100×720 逻辑尺寸创建（2200×1440 物理） |
| 托盘图标 | 通过 | 日志 `tray icon added`；退出时 `tray icon removed` |
| 关闭到托盘 | 通过 | `WM_CLOSE` 后 `IsWindowVisible=false`，进程仍存活 |
| 第二实例唤起 | 通过 | 第二次启动日志 `another Octos instance is running`，主窗口重新可见，进程数仍为 1 |
| 干净退出 | 通过 | 日志 `main window destroyed` → `message loop stopped` → `octos exited cleanly` |
| 单实例互斥 | 通过 | `Global\OctosMutex` + `FindWindowW`/`WM_APP_RESTORE` |

## 质量门禁

```text
cargo fmt --check                          pass
cargo test -p octos-protocol -p octos-core -p octos-storage   3 passed
cargo clippy --workspace --target x86_64-pc-windows-msvc -- -D warnings   exit 0
npm run typecheck (ui)                     pass
npm run build (ui)                         pass
```

## 结构

```
octos-rs/
├── crates/octos-protocol   # UI <-> host 的 serde 消息
├── crates/octos-core       # 版本/领域逻辑占位
├── crates/octos-storage    # OCTOS_DATA_DIR / LOCALAPPDATA 路径
├── crates/octos-webview    # WebView2 环境 + 主窗口 controller + 虚拟主机
├── crates/octos-win        # 主窗口、托盘、单实例
├── crates/octos-app        # bin octocaris.exe
├── ui/                     # React 19 + TS + Vite
└── xtask/                  # build-ui / build / deploy / smoke
```

## 已知限制 / 遗留

- 托盘菜单只验证了添加/移除和退出路径，菜单项的鼠标交互未自动化测试。
- 工作区尚未 `git init`；等你确定远端后再初始化并提交，避免擅自建立提交历史。
- `OCTOS_UI_DEV_URL` 开发模式尚未在 Windows 上实测；生产路径已通过。
- 主窗口仍使用标准 Win32 边框，无边框自绘在 M9。
- 还没有 prefs、wallpapers 扫描、壁纸窗口；这些进入 M2/M3。
- `show_error` 在 GUI 进程里弹 MessageBox，但没有写入 Windows 事件日志。
- `GetLastError` 在 `FindWindowW` 失败时可能返回 0（M0 已记录），M2 的 watchdog 要区分“暂时不存在”和“真正失败”。

## 下一步（M2）

1. 把 spike 的 Composition + DComp + WorkerW 代码整理进 `octos-webview`/`octos-win`。
2. 建立 `MonitorRegistry`（设备名主键、HMONITOR、rect、hwnd、mod）。
3. 内置 gradient mod，按显示器设置/清除。
4. 鼠标钩子（含 `WindowFromPoint` 命中守卫、按下/松开跨窗口补偿）。
5. 全屏暂停、Explorer 重启恢复、ProcessFailed 重建。
