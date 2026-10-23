# M0 Toolchain Spike — retired

**This is NOT the wallpaper implementation.**

This crate was the M0 proof that WSL + `cargo-xwin` can build a Windows x64 exe and that
WebView2 CompositionController + DirectComposition + a desktop-parented window can render
and receive input. It is kept as a historical/toolchain reference only.

Do **not** use this binary to judge product behavior: wallpaper z-order, desktop icons,
click-through, right-click, fullscreen pause, or multi-monitor. Use `octos.exe` instead.

## Known limitations (intentional in M0)

| Area | Spike | Production (`crates/octos-win/src/wallpaper.rs`) |
|---|---|---|
| Attach order | WorkerW first, Progman fallback | Progman + `SHELLDLL_DefView` first, WorkerW fallback |
| Z-order | WorkerW path uses `SWP_NOZORDER`; not maintained | Insert below `SHELLDLL_DefView`, re-positioned by watchdog |
| WebView2 `Chrome_WidgetWin_*` overlay | Not made click-through | Forced `WS_EX_LAYERED|WS_EX_TRANSPARENT` when matching a monitor |
| Fullscreen detection | None; always shown | `is_fullscreen` + DWM cloaked + class exclusions, pauses wallpaper |
| Desktop icons | No handling | Icon visibility remains a user setting; documented procedure |
| Monitors | One primary monitor (`SM_CXSCREEN`) | `EnumDisplayMonitors` registry keyed by device name |
| Visibility gating | Only the page's Quit button | `global_visible` / `user_visible` / fullscreen / controller state |
| Input guard | 3 desktop classes | 4 desktop classes + visibility/rect/fullscreen checks |
| Startup | Created `WS_VISIBLE` full-screen before `SetParent` | Hidden until attached, then shown by visibility policy |
| Acceptance | Toolchain proof only | `octos.exe` is the only product acceptance target |

## Historical note

In the M0 run the WorkerW search happened to find nothing, so the spike took the Progman
fallback and logged `attached to Progman below SHELLDLL_DefView (Win11 path)`. That was
luck, not a design guarantee. On another Explorer state the WorkerW path can put the canvas
above the icon layer. See `docs/m2-wallpaper.md` for the full post-mortem.

## Build (historical)

```bash
cd spikes/win-webview2
cargo xwin build --target x86_64-pc-windows-msvc --release
```

Do not copy the exe into `E:\file\octos`; that directory is reserved for the real app.
