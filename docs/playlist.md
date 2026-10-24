# 多图壁纸轮换（Playlist）

日期：2026-10-23
状态：已实现并实机验证（单屏、图片壁纸）

## 行为

- 壁纸库顶部常驻「壁纸轮换」面板；没有壁纸时只显示一行提示。
- 全局一个列表，**顺序由壁纸库网格决定**：拖动卡片排序，轮换按网格顺序取下一张（每屏独立列表留给以后）。
- 间隔用「分钟 + 秒」两个输入框自由填写，总范围 5 秒 – 7 天，默认 30 分 0 秒；默认顺序循环，可开「随机顺序」，随机不会连续两次同一张。
- 启用即对齐：当前壁纸不在列表里就立刻切到 `current`（或第一项）；已在列表里则保持，重新计一个完整间隔。
- 「下一张」忽略间隔，手动切一次。
- 入口：卡片**左上角的方形复选框**切换是否加入轮换，可以同时勾选多张；点击卡片其它区域仍是查看详情，选中卡片只用高亮边框表示。不再有「添加壁纸」下拉、圆点按钮或详情页的「加入轮换」。
- 面板里的「全选」一次把壁纸库中所有可运行壁纸加入列表；顺序仍由网格决定。
- 在壁纸库按 Ctrl+A（或点面板里的「全选」）把所有可运行壁纸加入列表；Ctrl+D 清空列表（列表为空时轮换自动关闭）。输入框内的 Ctrl+A/Ctrl+D 不触发这些动作。没有 `entry_url` 的 mod 复选框禁用。
- 内置的 `gradient` 和 `conformance` 不进入壁纸库和轮换列表（`gradient` 仍作为删除 mod 后的兜底）。
- 列表被删空时自动关闭轮换，开关不会卡在“开启”状态。
- 启用手动 `Set wallpaper` 之后提示：下一次切换会覆盖手动选择。

## 持久化

`preferences.json` 版本 2 → 3，新增顶层 `playlist`：

```json
"playlist": {
  "enabled": true,
  "items": ["E:\\...\\a", "E:\\...\\b"],
  "intervalSeconds": 1800,
  "random": false,
  "current": "E:\\...\\a"
}
```

- 纯增量字段：旧文件缺省为停用；未知键经 `extra` 原样保留，`version` 由 `from_json` 在内存里升级。
- `items` 用绝对 mod 文件夹路径，与 `selected`/`modOptions` 同一身份；不做 `canonicalize`，比较大小写敏感。
- `current` 由引擎写，启动优先恢复它；其次接受 `selected` 各屏一致的项；最后取第一个可运行项。
- `snapshot_preferences` 只覆盖 `selected`/`modOptions`，不碰 `playlist`。
- 旧格式迁移：第一版写入的 `intervalMinutes` 在加载时换算成 `intervalSeconds`，旧的 `window` 字段直接丢弃（不再支持时间段）。

## 引擎

- `crates/octos-app/src/rotation.rs`，UI 线程定时器 ID 4（2 = wallpaper watchdog，3 = devtools），1 秒一跳。
- 规则函数在 `crates/octos-core/src/playlist.rs`（Linux 可单测）：`is_due`、`next_index`、`normalize_items`、`clamp_interval`、`migrate_legacy`。
- 每跳顺序：启用？→ 全屏/清空门控 → 间隔到期 → **一次** catalog 扫描 → 可运行项 ≥ 2 → 选下一项 → 切换。
- 未到期不扫目录；全屏或清空壁纸时不推进计时，条件解除后第一跳补上。
- 缺失/不可运行条目跳过但**不删除**（外接盘离线不能毁掉列表）；只有卸载或在列表里移除才摘除，快照用 `missing` 暴露。
- 手动 `set-monitor-mod` 重置计时；若该 mod 在列表里则设为 `current`。
- `RemoveMod` 同步摘除条目；删的是当前项时，启用状态下立即切到下一个可运行项。
- 少于两个可运行项时不轮换；启用但只有一项时保持启用并在状态行提示。

## 平滑切换

新增 `octos-win::wallpaper::switch_mod_all`：更新 `Session` 与每个 slot 的 `entry_url` 后，对已有 controller 调 `octos_webview::navigate`，不 close/recreate。

- 每屏 storage scope 与帧率脚本通过 `AddScriptToExecuteOnDocumentCreated` 注册，对后续文档继续生效。
- `NavigationCompleted` 与 mod 消息处理都读实时 `Session`，所以 `options-load` 和协议请求指向新 mod。
- 已知限制：新页面绘制前可能有一帧透明（看到桌面）。双 controller 交叉淡入不在本次范围。
- 创建中竞态：切换若发生在 controller 创建期间，完成回调会比较“实际导航的 URL”与 `slot.entry_url`，不同则补一次 `navigate`。

## 界面

- `.playlist-panel`：启用开关（列表为空时禁用）、分钟/秒两个数字框、随机顺序、提示与批量操作。
- 面板本身只保留：启用开关、分钟/秒间隔、随机顺序、提示行、`全选` / `下一张`。不再有展开的轮换列表；成员由卡片复选框决定，顺序由壁纸库网格决定。
- 卡片左上角方形复选框 = 加入/移出轮换（不改变详情选择，可多选）；卡片主体 = 查看详情；**拖动卡片调整网格顺序（Alt+↑/↓ 也可）**，该顺序即轮换顺序。卡片正文只保留名称，详情只保留名称、显示器、设为壁纸和选项（去掉了作者、版本、描述等元信息）。
- 网格顺序持久化在 `preferences.json` 的 `libraryOrder`（绝对 folder path）；新导入的壁纸先排在末尾，下一次拖动后写入完整顺序。
- 图片壁纸的 `fit` 选项在 UI 中本地化：`图片适配` / `裁剪填满` / `完整显示` / `拉伸填满`（英文 `Image fit` / `Cover` / `Contain` / `Fill`）；未知选项仍显示 mod 自己的文本。
- 中英双语；深/浅色主题；720px 窄窗口下控件换行且按钮保持可见。
- 主界面按桌面应用处理：全局 `user-select: none`，输入框仍可选中编辑；除壁纸卡片排序外，`dragstart` 被拦下、图片 `draggable=false`，不会出现网页式的拖拽/框选复制。

## 实测记录（2026-10-23，`E:\file\octos`）

1. 面板显示正常；列表为空时启用开关为禁用态，状态行显示“未启用”。
2. 预置两张图片 + 1 分钟间隔并启用：启动日志出现 `playlist aligned to ...image-1-223...`（启用即对齐）。
3. 之后每约 60 秒一条轮换日志，顺序在两项间交替：
   `playlist rotation: ...image-1-bf8... (reason=interval, index=2/2, random=false)` /
   `...image-1-223... (index=1/2, random=false)`。
4. 轮换期间没有新的 `wallpaper controller ready ->` 日志，确认是原地导航而非重建 controller。
5. `preferences.json` 升到 `version: 3`；`selected` 与 `playlist.current` 同步指向当前图片。
6. 截图确认：卡片 `轮换` 徽标、展开列表中的 `当前` 胶囊、选择器的“已加入”勾选、详情里的“移出轮换”和覆盖提示。
7. 测试结束后已把 `preferences.json` 恢复为测试前的 L2D 选择。
8. 2026-10-24 第二轮：移除时间段与 `intervalMinutes`，改为自由秒/分钟；迁移后 3 项列表（含 L2D）随机轮换，日志 `index=2/3/3/2`，轮换期间同样没有重建 controller。
9. 显示器名称通过 `EnumDisplayDevicesW` 读取 EDID 名称（本机显示 “Lenovo DisplayHDR”），取不到时回退 `DISPLAYn`。
10. 2026-10-24 第三轮：分钟+秒双输入框、去掉作者/版本/描述、`fit` 选项中英文本地化；删除最后一张壁纸后列表清空、`enabled` 自动写回 false。
11. 2026-10-24 第四/五轮：卡片点击改为查看详情；轮换改为左上角方形复选框（去掉右上角选中圆点，避免两个同色圆形混淆），支持多选、Ctrl+A / 「全选」、Ctrl+D 清空；复选框可 Tab 到达，方向键仍可继续在网格里导航。
12. 2026-10-24 第六轮：网格改为拖拽排序，顺序写入 `libraryOrder` 并驱动轮换；实测拖动第一张到第三张后网格与 prefs 同步，重启保持；从复选框拖拽不会触发排序也不会误改成员；Alt+↓/↑ 可键盘移动且焦点跟随；点「下一张」切到网格顺序中的下一张。

未实机覆盖（逻辑有单测或门控）：全屏中暂停/恢复、显示器热插拔、多显示器同名去重。下次回归时优先补这些。
