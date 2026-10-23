# M5 Hardening — 进度

日期：2026-10-23
起点：M4 完成（commit `1646050`）。

## 已完成：prefs 写盘 debounce + `.bak`

- `octos-storage::PrefsWriter`：后台线程串行写盘，UI 线程只提交快照；连续变更只保留最新快照，静默 **250ms** 后写入。
- 退出时 `flush_preferences`：提交最终快照、等待 writer 写完并 `join`，再关闭壁纸。
- `write_atomic`：
  - 临时文件名带进程 id + 原子计数，避免并发冲突；
  - Windows 使用 `ReplaceFileW(..., REPLACEFILE_IGNORE_MERGE_ERRORS)`，自动保留 `preferences.json.bak`；
  - 失败时回退到 best-effort copy + `MoveFileExW(REPLACE_EXISTING|WRITE_THROUGH)`；
  - 非 Windows 测试路径先 copy 再 rename，语义一致。
- 单测：`writer_flushes_the_newest_snapshot`、`writer_writes_after_the_debounce_window`、`second_write_keeps_the_previous_file_as_bak`、`first_write_has_no_backup`。

## 实机验证

环境：`OCTOS_DATA_DIR=E:\file\octos\data`，`OCTOS_LOG=debug`。

1. UI 切换一次“全屏时暂停”：`set-setting` 01:33:30.571Z，`preferences written` 01:33:30.868Z（约 297ms 后）。
2. 连续快速切换 6 次：只产生 **1 次** 写盘（01:34:11.306Z）。
3. 当前 `preferences.json` 的 `pauseFullscreen=true`；`preferences.json.bak` 保存上一次的 `false`。
4. 通过托盘 Quit（`WM_COMMAND` id=2）退出：01:34:47.242Z 再写一次最终快照，进程正常结束。
5. `data` 根目录无 `.tmp-*` 残留。

## 剩余 M5 项

1. 多显示器 / 按显示器设置 / `redirect` / 热插拔（当前环境只有 DISPLAY1）。
2. 真实游戏/视频全屏回归（现有边框全屏窗口证据见 M2）。
3. ProcessFailed renderer 崩溃恢复。
4. Explorer 重启、DPI 100/150/200% 的完整回归矩阵复核。

## 当前测试

- `octos-core` 31、`octos-protocol` 10、`octos-storage` 20，共 **61**。
