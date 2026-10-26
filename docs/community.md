# 社区：扁平仓库、发布工具与应用内拉取

日期：2026-10-25
状态：应用侧已在 Windows 实机验证（浏览/安装/更新/取消/坏 hash/选项保留）；社区仓库已切换为扁平结构，正按每批 20 项上传（滚动 Release `wallpapers`）。

## 1. 设计

社区是**一个公开 GitHub 仓库 + 一个滚动 Release**，没有服务端，也没有第二份元数据：

```
octocaris-community/
├── index.json          # 唯一真源：应用读它，人也能直接看
├── previews/<folder>.gif   # 每项一张动图预览（GitHub 直接渲染）
├── README.md           # 由 index 生成的预览目录（人看）
├── NOTICE.md
├── schema/index.schema.json
├── tools/publish.py    # 打包 / 上传 / 更新 index
├── tools/publish_lib/
└── dist/               # 本地打包产物（gitignore）
```

`index.json` 里每项的 **`folder` 就是目录键，也是安装目录名**：

```json
{
  "folder": "3252727328",
  "name": "L2D 碧蓝档案 8K | … | midori(maid)",
  "author": "Wallpaper Engine",
  "description": "",
  "version": "1.0.0",
  "tags": [],
  "entry": "index.html",
  "preview": { "url": "previews/3252727328.gif?v=f43a8ea9", "bytes": 719072 },
  "download": {
    "url": "https://github.com/Jailvisor/octocaris-community/releases/download/wallpapers/octocaris-3252727328-v1.0.0.zip",
    "bytes": 29552377,
    "sha256": "…"
  },
  "fileCount": 40,
  "files": [{ "path": "index.html", "bytes": 463 }, …],
  "source": { "kind": "wallpaper-engine", "workshopId": "…", "workshopUrl": "…" },
  "pageUrl": "https://github.com/…/blob/main/README.md#3252727328",
  "updatedAt": "…"
}
```

- 没有 `id`、`aliases`、`thumbnail`、`items/`、`meta.json`、每项 README。
- 预览只存 **gif**（用户选择）；卡片和详情都用它。
- 文件清单在 `files`，所以"能看到包内文件名"仍然满足。
- 缩略图/预览 URL 带 `?v=<sha8>`，换图后缓存自动失效。
- zip 在 Release `wallpapers`，资产名 `octocaris-<folder>-v<version>.zip`。

## 2. 发布

```bash
python tools/publish.py add <folder> [--tags live2d] [--bump patch] [--upload]
python tools/publish.py bulk /mnt/e/file/octos/data/wallpapers --upload
python tools/publish.py render
python tools/publish.py validate --strict
```

- 打包确定性：文件排序、固定时间戳、deflate；同内容重复打包 sha256 相同。
- 是否需要重打包由本地 `.publish-cache.json`（gitignored）记录的文件树指纹决定，不污染仓库。
- 默认标签为空；`Anime` / `Wallpaper Engine` / `Game` 会被过滤。
- 描述会去掉"设置请移动鼠标到右上角 / bug反馈 / Q群"这类模板文案。
- `bulk` 默认只收 `project.json` 且 `type=web` 的目录，跳过 `default/conformance/image-*/*-octos-package`。
- `--push` 才 git commit/push。

## 3. 应用行为

- "社区"页：搜索、分页、gif 预览、详情抽屉（文件清单、SHA-256、来源链接）。没有标签筛选，也不显示缓存/离线横幅。
- 预览加载失败（或缺失）统一使用内置壁纸的保底图：`https://local.octos/default/image.jpg`。
- 下载在后台线程（WinHTTP，系统代理/PAC/`OCTOS_COMMUNITY_PROXY`），进度 200ms 节流；`.part` → 校验大小和 sha256 → 后台解压 → **UI 线程**安全替换。
- 替换前先把正在显示该壁纸的显示器切走，再用备份 + rename 换入；失败回滚，崩溃后下次启动恢复。
- 已安装 = `wallpapers/<folder>` 存在；marker `octos-community.json` 记录 folder/version/sha/source。
- 只有远端版本更高才显示"更新"；folder 不变，`modOptions` 按绝对路径保留。
- 错误以 `{code, detail}` 返回，UI 翻译 `community.error.<code>`（`unknown-item` 等）。
- CLI：`octocaris community install <folder>`、`community refresh`、`community cancel`。
- 测试环境变量：`OCTOS_COMMUNITY_URL`、`OCTOS_COMMUNITY_ALLOW_INSECURE=1`（loopback http）。

## 4. 缓存

- **索引**：`data/community/index.json` + `index-meta.json`（来源 + sha 校验）。有缓存时启动不联网；手动刷新带 `If-None-Match`，304 复用。
- **预览 gif**：索引刷新后下载到 `data/community/previews`（按 URL 哈希命名），UI 通过 `cache.octos` 虚拟主机加载。36 项约 25 MB，一次下载后浏览不再请求 GitHub 图片；`?v=<sha8>` 变更时才重新下载。
- 详情动图与卡片用同一份本地缓存，不再单独请求。

## 5. 已知限制

- 取消只在读响应体时检查；卡在连接/等响应头时要等 WinHTTP 超时。
- 不做账号/私有仓库、评分评论、应用内上传、断点续传、增量更新。
- 社区仓库当前只上传了 3 个 zip；其余资产与扁平化改动等对齐后统一上传/推送。

## 6. 实机验证（2026-10-25）

本地 fixture（`python -m http.server 8765` + `OCTOS_COMMUNITY_URL`）：

1. 启动加载索引，`community install` 写入 `wallpapers/<folder>` + marker，zip 清理。
2. 连续更新多个版本成功；预置 `modOptions[...].speed = 42` 跨版本保留。
3. 错误 sha → `hash-mismatch`，已安装目录不变。
4. 慢速服务器 + `community cancel` → `download cancelled`，无 `.part`。
5. 停服重启 → `loaded cached community index`。

真实仓库：

1. 默认源加载 36 项。
2. 安装 `2925874645`：下载 30,881,818 字节，sha 校验通过，marker 带 Workshop 来源。
3. `octocaris run` 后 `wallpaper controller ready -> https://local.octos/2925874645/index.html`，桌面截图确认渲染。
4. 首次启动缓存 36 张预览（约 1 MB 的旧 jpg 版本；改成 gif 后会约 25 MB），重启只有 `loaded cached community index`，没有重复请求。
