//! Windows desktop icon visibility.
//!
//! Mirrors the original `System.setDesktopIconVisibility`: toggle the
//! `SysListView32` window, and persist the choice in the same HKCU value that
//! Explorer uses for "Show desktop icons".

use anyhow::{Context, Result};
use tracing::info;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, FindWindowW, IsWindowVisible, SW_HIDE, SW_SHOW, ShowWindow,
};
use windows::core::{PCWSTR, w};

fn find_icon_list() -> Result<HWND> {
    unsafe {
        let progman =
            FindWindowW(w!("Progman"), PCWSTR::null()).context("Progman is not available")?;
        let mut shell =
            FindWindowExW(Some(progman), None, w!("SHELLDLL_DefView"), PCWSTR::null()).ok();

        // Windows 10 moves SHELLDLL_DefView under a top-level WorkerW once the
        // wallpaper is attached (Progman 0x052C). Windows 11 keeps it under
        // Progman. Mirror the original app and fall back to the WorkerW chain.
        if shell.is_none() {
            let mut after: Option<HWND> = None;
            loop {
                let Ok(worker) = FindWindowExW(None, after, w!("WorkerW"), PCWSTR::null()) else {
                    break;
                };
                if let Ok(found) =
                    FindWindowExW(Some(worker), None, w!("SHELLDLL_DefView"), PCWSTR::null())
                {
                    shell = Some(found);
                    break;
                }
                after = Some(worker);
            }
        }

        let shell = shell.context("SHELLDLL_DefView is not available")?;
        FindWindowExW(Some(shell), None, w!("SysListView32"), w!("FolderView"))
            .context("SysListView32 is not available")
    }
}

pub fn is_visible() -> bool {
    find_icon_list()
        .map(|hwnd| unsafe { IsWindowVisible(hwnd).as_bool() })
        .unwrap_or(false)
}

pub fn set_visible(visible: bool) -> Result<()> {
    let hwnd = find_icon_list()?;
    unsafe {
        let _ = ShowWindow(hwnd, if visible { SW_SHOW } else { SW_HIDE });
    }

    // Match the original System.setDesktopIconVisibility: this is a live
    // ShowWindow toggle, not a persisted Windows setting. Do not write
    // HideIcons here; Explorer owns that preference.
    info!("desktop icons visible={visible}");
    Ok(())
}

#[cfg(test)]
mod tests {
    /// Live desktop test. Only runs when explicitly enabled, because it briefly
    /// hides and restores the real desktop icons.
    #[test]
    fn desktop_icon_visibility_round_trip() {
        if std::env::var_os("OCTOS_DESKTOP_ICON_TEST").is_none() {
            return;
        }

        let before = super::is_visible();
        super::set_visible(!before).expect("could not toggle desktop icons");
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert_eq!(
            super::is_visible(),
            !before,
            "desktop icon visibility did not change"
        );

        super::set_visible(before).expect("could not restore desktop icons");
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert_eq!(
            super::is_visible(),
            before,
            "desktop icon visibility did not restore"
        );
    }
}
