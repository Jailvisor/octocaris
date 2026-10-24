# 图片壁纸

日期：2026-10-23

Octos 现在可以直接把图片作为壁纸，不需要手工写 `octos.json`/`index.html`。

## 入口

1. **Library 顶栏“添加图片”**：打开文件选择器。
2. **拖拽**：把图片文件拖进主窗口。
3. **CLI**：`octos.exe run path\to\image.png`（会复制到 wallpapers 并应用到所有显示器）。

## 支持格式

`png`、`jpg`/`jpeg`、`webp`、`gif`、`bmp`。

- 单文件上限 **128 MiB**。
- 扩展名必须在白名单内，但**文件类型以文件头为准**：壁纸站常见的“JPEG 内容 + `.png` 后缀”会按真实格式导入为 `image.jpg`，不再报 `InvalidImage`。
- 文件头不属于上述任何一种格式时才拒绝，错误提示为 `not a valid image file: <path>`。
- 拒绝符号链接。
- GIF / 动态 WebP 会保持动画。
- 不接收 SVG（避免脚本内容）；视频壁纸后续单独做。

## 生成的 mod

每次导入生成一个普通 mod 文件夹 `image-<slug>-<hash>`：

```text
image-my-photo-1a2b3c4d/
├── image.png
├── thumbnail.jpg   # 卡片/轮换列表预览，最长边 512
├── index.html
└── octos.json
```

- `octos.json` 的 `image` 指向原图，壁纸页面继续用原图；Library 卡片和轮换列表优先加载 `thumbnail.jpg`，没有时回退 `image`；
- `entry` 为生成的 `index.html`；
- 自动带一个 `fit` 选项：`cover`（默认）/ `contain` / `fill`。
- 图片页监听 `options-change`，修改 `fit` 后即时生效；三种模式分别对应 CSS `object-fit`。
- 生成的 mod 与普通 mod 一样可选中、卸载、重新导入；重复导入会生成新的 hash 文件夹。

## 实测

- `octos run ...\preview.gif`：生成 `image-preview-<hash>`，两块屏 controller 都就绪。
- Library 3 个壁纸，卡片显示 GIF 缩略图。
- 双屏桌面 `verify-desktop` PASS：2 个壁纸窗口、2 个 Chrome overlay。
- 通过 UI 把 `Image fit` 从 `cover` 改为 `contain`：prefs 更新，页面重排为两侧黑边，截图 `E:\file\octos\m5-image-contain-fixed.png`。
- 修复过程中发现生成页在首次 options 响应后移除了监听器，导致后续 `options-change` 不生效；已修复并复测。

## 限制

- `fit` 是 mod 级选项（当前所有显示器共享）；不同 DPI 的屏幕用 `cover`/`contain` 通常已经足够。
- 缩略图生成会应用 EXIF 旋转；HDR/HEIC 仍不支持；单边超过 12000px 或超过 100MP 的图片跳过缩略图生成，卡片回退原图。
- 缩略图只用于卡片预览：导入时生成，最长边 512、JPEG q80、透明背景合成 `#111`；GIF/动图取第一帧。
- 存量图片壁纸在启动时后台补齐缩略图，完成后自动刷新壁纸库；顶栏「刷新」也会触发一次。
- 外部 mod 可以自带 `thumbnail.jpg` / `thumbnail.png`，同样会被卡片使用。

导入的多张图片可以加入壁纸轮换列表，按间隔自动切换；见 [`playlist.md`](playlist.md)。
