# 项目范围（2026-10-23 确认）

目标：一个**轻量的 Windows 动态壁纸运行器**，主要运行：

- L2D / Spine 资源包（例如 `L2D 碧蓝档案 8K | 玲纱(魔法少女)`）
- 图片壁纸（PNG/JPG/WebP/GIF/BMP）
- 普通 HTML/CSS/JS、Canvas/WebGL 壁纸

不追求成为完整的社区平台或媒体中心。

## 需要

- WebView2 渲染、透明/点击穿透、桌面图标下方挂载
- 图片/文件夹/zip/拖拽安装，Library、选项、按显示器选择、持久化
- 托盘、轻量模式、全屏暂停、帧率上限、内存节省
- 跟随系统的深/浅色主题
- 免环境变量启动（便携数据目录）
- L2D 自带 BGM/语音自动播放（`--autoplay-policy=no-user-gesture-required`）

## 暂不需要 / 可选

| 项 | 决定 |
|---|---|
| M6 GSMTC 媒体会话 + WASAPI 系统音频 | **延后/可选**；L2D/图片不使用 |
| 视频壁纸（mp4/webm） | 可选；当前 HTML5 video 理论可跑，未作为目标验证 |
| 社区 Explore/下载、Studio | 不做（M7/M9） |
| 多图壁纸轮换（播放列表） | **已实现**（2026-10-23，见 `docs/playlist.md`） |
| 通知、自动更新、安装包 | 需要日常使用时再做（M8） |

媒体/音频 API 仍保留**协议桩**：调用会 resolve 空/默认值，不会让壁纸 Promise 挂起；只是不提供真实媒体数据。

## 日常启动

- 直接双击 `E:\file\octos\octos.exe`。
- 数据目录优先：`OCTOS_DATA_DIR` → 便携目录 `<exe>\data`（存在 `data\wallpapers` 时）→ `%LOCALAPPDATA%\Octos`。
- 当前 `E:\file\octos\data` 会被自动识别，不需要设置环境变量。
- 壁纸库、选项、显示器选择保存在 `E:\file\octos\data\preferences.json`。

## 后续建议

1. 打包一个便携 zip / 安装包，创建开始菜单快捷方式。
2. 验证 `runOnStartup` 开机自启（写入 HKCU Run），确保登录后壁纸自动恢复。
3. 如果遇到具体 L2D 资源不兼容，再针对该资源补 WebGL/Spine 运行时或音频策略。
4. 只有明确需要音乐可视化/桌面歌词时，才启动 M6。
