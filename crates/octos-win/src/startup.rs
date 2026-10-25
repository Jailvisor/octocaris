//! HKCU Run registration for the "start with Windows" setting.

use anyhow::{Result, anyhow};
use tracing::info;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_SZ, RegCloseKey, RegOpenKeyExW, RegSetValueExW,
};
use windows::core::{PCWSTR, w};

const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const VALUE_NAME: PCWSTR = w!("Octocaris");
const LEGACY_VALUE_NAME: PCWSTR = w!("Octos");

pub fn set_run_on_startup(enabled: bool) -> Result<()> {
    if enabled {
        add_to_startup()
    } else {
        remove_from_startup()
    }
}

fn add_to_startup() -> Result<()> {
    let exe = std::env::current_exe()?;
    // `--startup` keeps the main window hidden until the tray icon is opened.
    let command = format!("\"{}\" --startup", exe.display());
    let wide: Vec<u16> = command.encode_utf16().chain(std::iter::once(0)).collect();

    let mut key = HKEY::default();
    let status = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, None, KEY_WRITE, &mut key) };
    if status != ERROR_SUCCESS {
        return Err(anyhow!("could not open HKCU Run: {status:?}"));
    }
    let status = unsafe {
        RegSetValueExW(
            key,
            VALUE_NAME,
            None,
            REG_SZ,
            Some(std::slice::from_raw_parts(
                wide.as_ptr().cast::<u8>(),
                wide.len() * std::mem::size_of::<u16>(),
            )),
        )
    };
    unsafe {
        let _ = RegCloseKey(key);
    }
    if status != ERROR_SUCCESS {
        return Err(anyhow!("could not write HKCU Run value: {status:?}"));
    }
    // Clean up the pre-rename value so the app never starts twice.
    let _ = unsafe { windows::Win32::System::Registry::RegDeleteValueW(key, LEGACY_VALUE_NAME) };
    info!("registered Octocaris to start with Windows (hidden in tray)");
    Ok(())
}

fn remove_from_startup() -> Result<()> {
    let mut key = HKEY::default();
    let status = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, None, KEY_WRITE, &mut key) };
    if status != ERROR_SUCCESS {
        return Err(anyhow!("could not open HKCU Run: {status:?}"));
    }
    let status = unsafe { windows::Win32::System::Registry::RegDeleteValueW(key, VALUE_NAME) };
    let _ = unsafe { windows::Win32::System::Registry::RegDeleteValueW(key, LEGACY_VALUE_NAME) };
    unsafe {
        let _ = RegCloseKey(key);
    }
    if status != ERROR_SUCCESS && status != ERROR_FILE_NOT_FOUND {
        return Err(anyhow!("could not remove HKCU Run value: {status:?}"));
    }
    info!("removed Octocaris from Windows startup");
    Ok(())
}
