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
- 校验扩展名 + 文件头 magic；拒绝改名伪装的文件。
- 拒绝符号链接。
- GIF / 动态 WebP 会保持动画。
- 不接收 SVG（避免脚本内容）；视频壁纸后续单独做。

## 生成的 mod

每次导入生成一个普通 mod 文件夹 `image-<slug>-<hash>`：

```text
image-my-photo-1a2b3c4d/
├── image.png
├── index.html
└── octos.json
```

- `octos.json` 的 `image` 指向图片，Library 卡片直接显示；
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
- 不处理 EXIF 旋转、HDR/HEIC、超大分辨率解码限制；WebView2 能解码的格式才能显示。
- 没有缩略图缓存生成；Library 直接加载原图（懒加载 + 淡入）。
