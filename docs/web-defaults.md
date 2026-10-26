# Web 壁纸默认参数（octos-defaults.json）

日期：2026-10-25
状态：已实现（`octos-core::web_defaults`），并在 Windows 上验证了 `loadJson` 老式包的默认值注入。

## Context

用户希望能给 web 壁纸指定默认的 **BGM 音量** 和 **MouseTracking（鼠标轨迹 / 视线追踪）**，以后批量下载壁纸后统一设置。

以 `E:\file\octos\data\wallpapers\2945877251`（花凛 兔女郎）为例，壁纸包是一个 Vue + PIXI + Spine 的 web 应用：

- `loadJson.json` 的 `fileNames[0]` 是 **设置存储键**，这个包是 `CH0100_home`；
- 壁纸自己的设置对象在压缩 JS 里，默认值：

  ```js
  {
    language: "ch",
    bgmVolume: 0.3,
    fontSize: 18,
    position: { x: 0, y: 0 },
    rotation: 0,
    scale: 1,
    talkVolume: 0.5,
    textPointX: 100,
    textPointY: 100,
    textureResolution: "2k",
    mouseTracking: false,   // 视线追踪：眼睛跟随鼠标
    panelDisplay: true,
    mouseTrial: true,       // 鼠标轨迹：光标拖尾
    talking: true,
    talkingInterval: 0,
    penetration: false,
    autoTalk: false
  }
  ```

- 启动时 `localStorage.getItem(fileNames[0])` 读出用户值并 `Object.assign` 到默认对象；
- 面板修改后 `localStorage.setItem(fileNames[0], JSON.stringify(settings))`；
- 面板标签对应关系：
  - `BGM` → `bgmVolume`（0–1）
  - `Voice` → `talkVolume`（0–1）
  - `MouseTracking(视线追踪)` → `mouseTracking`
  - `MouseTracking(鼠标轨迹)` → `mouseTrial`

应用已经注入每屏存储前缀（`__octos_monitor__\\.\DISPLAYn:`），所以同一个壁纸在不同显示器上可以有不同的设置。

## 目标

1. 不改壁纸自带的 JS（第三方内容，更新会覆盖，且每个包压缩方式不同）；
2. 每个壁纸包可以声明自己的默认参数；
3. 默认参数在**页面脚本之前**生效，且遵守每屏存储前缀；
4. 默认值不覆盖用户后来在壁纸面板里改过的值（可选强制模式）；
5. 批量下载后能用一条命令给整个系列生成默认参数。

## 1. 配置文件

约定放在 mod 根目录：`octos-defaults.json`。

```json
{
  "schemaVersion": 1,
  "mode": "fill",
  "storage": {
    "CH0100_home": {
      "bgmVolume": 0,
      "mouseTracking": false,
      "mouseTrial": true
    }
  }
}
```

- `storage`：localStorage 键 → 要补的默认值；一个包可以有多个键（普通 mod 也可以用它自己的键）。
- `mode`：
  - `fill`（默认）：只写当前对象里**不存在**的键，用户已有值优先；
  - `force`：每次加载都覆盖指定键（适合“BGM 永远静音”）。
- 只允许 JSON 对象/字符串/数字/布尔/null；不执行任何代码。
- 文件随包一起走，社区更新会保留；不需要改 `index.json`。

示例：2945877251 关闭 BGM、保持鼠标轨迹、关闭视线追踪。

## 2. 应用侧实现

新增 `crates/octos-core/src/web_defaults.rs`（Linux 可测）：

```rust
pub const FILE: &str = "octos-defaults.json";

pub struct WebDefaults { pub mode: FillOrForce, pub storage: BTreeMap<String, Map<String, Value>> }

pub fn load(folder: &Path) -> Option<WebDefaults>;       // 解析 + 校验
pub fn script(defaults: &WebDefaults) -> String;         // 生成注入脚本
```

生成的脚本（在存储前缀脚本之后执行）：

```js
(function () {
  const defaults = { "CH0100_home": { "bgmVolume": 0 } };
  const force = false;
  for (const key of Object.keys(defaults)) {
    try {
      const current = JSON.parse(localStorage.getItem(key) || "{}");
      const merged = force
        ? Object.assign({}, current, defaults[key])
        : Object.assign({}, defaults[key], current);
      localStorage.setItem(key, JSON.stringify(merged));
    } catch (_) {}
  }
})();
```

注入顺序（`crates/octos-webview/src/lib.rs`，壁纸 controller 创建处）：

1. `octos_core::storage_scope::script(monitor_id)`（现有）
2. `octos_core::frame_cap::script(fps)`（现有）
3. **新增** `web_defaults::script(...)`（在页面脚本之前）

`crates/octos-win/src/wallpaper.rs` 在创建 controller 时按 slot 的 mod 目录读取 `octos-defaults.json`，把脚本传给 `octos_webview::create_wallpaper_controller`。读取失败只 warn，不影响壁纸加载。

为什么不改 package JS：

- 包是 Wallpaper Engine 创意工坊内容，更新会覆盖；
- 每个包压缩/变量名不同，没有通用改法；
- localStorage 是壁纸自己的持久化入口，官方面板也用它。

## 3. 发布/批量工具

`tools/publish.py` 增加一个子命令（实现阶段加）：

```bash
python tools/publish.py defaults <folder> \
  --bgm 0 \
  --trail on|off|keep \
  --tracking on|off|keep \
  [--voice 0.5] \
  [--mode fill|force] \
  [--bump patch]
```

行为：

1. 读 `loadJson.json` 的 `fileNames[0]` 作为 storage 键；
2. 读已有 `octos-defaults.json`（如果有）并合并；
3. 写入源目录；
4. 可选 `--bump` 后重新打包（新的 `octos-defaults.json` 会进 zip）；
5. 上传/推送仍然分开，遵守“只在明确要求时上传/推送”。

批量：

```bash
python tools/publish.py bulk /mnt/e/file/octos/data/wallpapers --defaults-profile l2d-bgm-off
```

或先对每个目录跑 `defaults`，再统一 `bulk --upload`。

对于非 L2D 的 web 壁纸，如果它没有 `loadJson.json`，可以手写 `octos-defaults.json`；工具只做校验。

## 4. 与现有设置的关系

- 应用设置里的 **“静音壁纸”** 是 WebView2 级别的全局静音，作用于所有声音（BGM + 语音）；本方案是**壁纸内部音量**，更细，两者可以并存。
- 如果只是想让所有壁纸都没声音，用全局静音即可，不需要本方案。
- 本方案不引入新的应用设置项；默认值属于壁纸内容。

## 5. 边界情况

| 情况 | 处理 |
|---|---|
| 用户已在壁纸面板改过值 | `fill` 模式保留用户值；`force` 模式每次覆盖 |
| 每屏不同设置 | 注入脚本在 `storage_scope` 之后，自动写当前显示器的命名空间 |
| 首次安装 | 没有旧值，直接写入默认值 |
| 更新壁纸 | `octos-defaults.json` 随包更新；旧值仍按 `fill` 保留 |
| 默认键拼错 | 只是多一个 localStorage 字段，壁纸忽略；不报错 |
| JSON 损坏 | 只 warn，跳过默认值，壁纸正常加载 |
| 值超出范围 | 壁纸自己 clamp；工具对 `bgmVolume/talkVolume` 做 0–1 校验 |
| 同一包多个设置键 | `storage` 支持多个键 |
| 旧版应用 | 不读 `octos-defaults.json`，行为不变 |

## 6. 测试与验收

Linux：

- `web_defaults::load`：合法/非法 JSON、缺 `storage`、未知字段保留、`mode` 默认 fill；
- `script`：JSON 转义、`fill`/`force` 合并语义（用一个小型 JS 解释器不现实，改为断言生成的字符串包含关键片段）；
- 数值范围校验（0–1）。

Windows（`E:\file\octos`）：

1. 给 2945877251 放 `octos-defaults.json`（`bgmVolume: 0`）；
2. 重新加载壁纸，打开右上角面板：BGM 显示 0；听不到背景音乐；语音仍可播放；
3. 在面板里把 BGM 调到 0.3，重启应用 → `fill` 模式保持 0.3；
4. 改成 `force` 模式 → 重启后回到 0；
5. `mouseTrial` / `mouseTracking` 开关与面板显示一致，鼠标行为可观察；
6. 双屏分别设置不同值 → 两个 localStorage 命名空间互不影响；
7. 删除 `octos-defaults.json` → 行为回到壁纸自带默认值。

## 7. 实施顺序（等批量下载后）

1. `octos-core::web_defaults` + 测试；
2. `octos-webview` 注入参数 + `octos-win::wallpaper` 读取；
3. `publish.py defaults` 子命令；
4. 选 2945877251 做单包验证；
5. 批量给 L2D 系列生成 `octos-defaults.json`，按需 bump 版本、重新打包；
6. 上传/推送（用户明确要求时）；
7. `docs/web-defaults.md` 更新为已实现，AGENTS 状态补一行。

## 8. 待确认

1. BGM 默认值：0（关闭）、0.1、还是保持 0.3？
2. 鼠标轨迹 `mouseTrial`：默认开还是关？
3. 视线追踪 `mouseTracking`：默认开还是关？
4. `mode`：默认 `fill`（用户改过就保留）还是 `force`（每次强制）？
5. 应用范围：只 2945877251、整个 L2D 系列、还是先只做机制？

## 9. 旧版 B（`loadJson.json` 只有 `fileNames`）的具体规则

以 `2900937551` 为例：`loadJson.json` 只有 559 字节，BGM/鼠标默认值硬编码在压缩 JS 里：

```js
{
  language: "ch",
  bgmVolume: 0.3,
  talkVolume: 0.5,
  mouseTracking: false, // 视线追踪
  mouseTrial: true      // 鼠标轨迹（拖尾）
}
```

初始化时：

```js
const saved = localStorage.getItem(loadJson.fileNames[0]); // 例如 "CH0178_home"
if (saved) Object.assign(settings, JSON.parse(saved));
```

所以旧版 B 的默认值**不能改 `loadJson.json`**（它没有设置字段），只能：

1. 改压缩 JS 的默认对象（不推荐：压缩代码、更新会被覆盖）；
2. **在页面脚本之前预写 `localStorage[fileNames[0]]`**（推荐，`octos-defaults.json` 就按这个做）。

### 9.1 `octos-defaults.json`（旧版 B）

```json
{
  "schemaVersion": 1,
  "mode": "fill",
  "storage": {
    "CH0178_home": {
      "bgmVolume": 0,
      "talkVolume": 0.5,
      "mouseTracking": false,
      "mouseTrial": true
    }
  }
}
```

- `storage` 的键必须等于该包 `loadJson.json` 的 `fileNames[0]`。
- `mode: "fill"`：只补用户当前设置里没有的键，用户改过的值保留。
- `mode: "force"`：每次加载都覆盖这几个键。
- 旧版没有 `version` 字段，所以“改了默认值但用户本地已有设置”时，`fill` 不会生效；要生效用 `force`，或删掉对应的 localStorage 键。

### 9.2 注入脚本

在 `storage_scope` 之后、页面脚本之前执行：

```js
(function () {
  const defaults = { "CH0178_home": { "bgmVolume": 0, "mouseTracking": false, "mouseTrial": true } };
  const force = false;
  for (const key of Object.keys(defaults)) {
    try {
      const current = JSON.parse(localStorage.getItem(key) || "{}");
      const merged = force
        ? Object.assign({}, current, defaults[key])
        : Object.assign({}, defaults[key], current);
      localStorage.setItem(key, JSON.stringify(merged));
    } catch (_) {}
  }
})();
```

因为跑在每屏存储前缀脚本之后，`localStorage` 的读写会自动带上 `__octos_monitor__\\.\DISPLAYn:` 前缀，双屏互不影响。

### 9.3 发布工具生成规则

实现阶段的 `publish.py defaults` 子命令按包格式自动选择：

- 读 `loadJson.json`：
  - 有 `SettingModel` → 新版 B：直接改 `SettingModel`（`bgmVolume`/`talkVolume`/`trackMouse`），用户本地同 `version` 的缓存会覆盖，必要时把 `version` 加 1；
  - 有 `fileNames` → 旧版 B：取 `fileNames[0]` 作为 `storage` 键，写 `octos-defaults.json`；
- 两者都没有（单个大 `index.html`）→ 旧版 A：走 `project.json` 属性 + `wallpaperPropertyListener` 桥，不在这个 localStorage 方案里。

### 9.4 旧版 B 的注意点

- `fileNames[0]` 是设置键；不同角色不同（`CH0178_home`、`CH0355_home.skel` 在新版是另一种规则）。
- 设置对象里的键名：`bgmVolume`、`talkVolume`、`mouseTracking`（视线）、`mouseTrial`（轨迹）；不要写成新版的 `trackMouse`。
- 用户点开面板并保存后，localStorage 里就是完整设置对象；`fill` 会保留这些值。
- 如果包内没有 `loadJson.json`，不要套这个规则。
