# 导入 Wallpaper Engine web 壁纸

日期：2026-10-24
状态：已实现并实机验证

## 行为

Steam 创意工坊里的 `.zip` 或文件夹通常只有 `project.json`，没有 Octos 的 `octos.json`/`mod.json`。安装时现在会识别 **Wallpaper Engine web 包** 并自动生成 `octos.json`：

- `project.json` 的 `type` 必须是 `"web"`；
- 用 `file` 作为 `entry`（默认 `index.html`，必须存在且在包内）；
- 用 `title`、`description` 作为名称和描述；
- 用 `preview`（存在时）作为卡片图；
- `author` 写 `Wallpaper Engine`，`version` 写 `1.0.0`。

生成的 `octos.json` 直接落在安装后的 mod 目录里，原包内容不改。zip 和文件夹导入走同一套转换。

## 不支持

- `type` 为 `scene` / `video` 的包（Wallpaper Engine 原生场景/视频）会返回明确错误，不生成 mod。
- 没有 `project.json` 且没有 Octos manifest 的目录仍然按“不是 Octos mod”拒绝。

## 限制

- 只提供包内自带的 HTML/JS 运行环境；Wallpaper Engine 的 `wallpaperPropertyListener`、音频监听等 JS API 不会注入。页面能正常渲染，自带的设置面板/交互可用，但依赖 WE 宿主的属性同步、音频响应不会工作。
- 包内 `preview` 作为卡片图（GIF 会动画）；不会自动生成缩略图。

## 清理

- 安装失败或进程中断留下的 `.octos-install-*` / `.octos-image-*` staging 目录会在下次启动和每次安装开始时清理。
- 安装错误现在指向原始 zip/文件夹路径，不再显示 staging 临时目录。

## 实测

`C:\application\steam\steamapps\workshop\content\431960\2961764492`（L2D 碧蓝档案 纱织，WE web 包）：

1. `octos run <folder>` 自动生成 `octos.json`，安装为 `data\wallpapers\2961764492`。
2. 壁纸 controller 导航到 `https://local.octos/2961764492/index.html`，桌面截图确认 L2D 角色渲染，包内设置面板可用。
3. `cargo test -p octos-storage install` 覆盖 web 文件夹、web zip、scene 拒绝和 staging 清理。
