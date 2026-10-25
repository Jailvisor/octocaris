# Octocaris

Octocaris 是 [Octos](https://github.com/underpig1/octos) 的 Rust + WebView2 重写：一个轻量的 Windows 动态壁纸运行器，兼容 Octos 的 mod 格式与 JS API。

## 功能

- 运行 L2D/Spine、HTML 和图片壁纸；支持 Octos mod（`octos.json`）
- 每显示器独立壁纸；WorkerW / Progman 挂载、透明、点击穿透
- 壁纸轮换队列：顺序或随机、自由间隔、下一张；队列自身决定顺序
- 图片直接导入（`fit` 选项）、zip / 文件夹安装、缩略图
- 200% DPI 下按物理像素渲染，L2D 清晰度与 Wallpaper Engine 一致
- 整体静音、帧率上限、全屏暂停、托盘、轻量模式、内存节省
- 中英双语、浅色 / 深色主题；Win10 / Win11

## 构建

需要 WSL2/Ubuntu（或 Windows）上的 Rust 1.94、`cargo-xwin`、Node.js。

```bash
cargo xtask build-ui     # 构建 React 界面
cargo xtask build        # cargo xwin build --target x86_64-pc-windows-msvc
cargo xtask deploy       # 部署到 E:\file\octos（OCTOS_DEPLOY_DIR 可覆盖）
```

运行需要 Microsoft Edge WebView2 Runtime（Win11 和大部分 Win10 已自带）。

## 测试

```bash
cargo test -p octos-core -p octos-protocol -p octos-storage
cargo xwin clippy --workspace --target x86_64-pc-windows-msvc -- -D warnings
cd ui && npx tsc --noEmit && npm run build
```

## 文档

设计、验收与兼容性记录在 [`docs/`](docs/)，开发约定见 [`AGENTS.md`](AGENTS.md)。

## 上游与许可

基于 [underpig1/octos](https://github.com/underpig1/octos)（MIT License, Copyright (c) 2025 underpig1）重写；本项目采用 MIT License，见 [LICENSE](LICENSE)。
