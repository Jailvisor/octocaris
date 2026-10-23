//! Typed messages between the React UI and the Rust host.
//!
//! The wallpaper-facing protocol (mod API) is intentionally *not* here yet; that
//! protocol is frozen against the original Octos JavaScript SDK and will live in
//! its own module once M3 starts.

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: &str = "0.1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum UiRequest {
    Ping {
        id: u64,
    },
    ShowWallpaper,
    ClearWallpaper,
    GetDesktopIcons,
    SetDesktopIcons {
        visible: bool,
    },
    GetSettings,
    SetSetting {
        key: String,
        value: serde_json::Value,
    },
    GetLibrary,
    SetMonitorMod {
        monitor_id: String,
        folder_path: String,
    },
    SetOption {
        folder_path: String,
        id: String,
        value: serde_json::Value,
    },
    InstallMod,
    InstallFolder,
    RemoveMod {
        folder_path: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum UiResponse {
    Pong { id: u64, version: String },
    WallpaperState { active: bool },
    DesktopIcons { visible: bool },
    Settings { data: serde_json::Value },
    Library { data: serde_json::Value },
    Notice { message: String },
    Error { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_round_trips() {
        let request = UiRequest::Ping { id: 42 };
        let json = serde_json::to_string(&request).unwrap();
        assert_eq!(json, r#"{"type":"ping","id":42}"#);
        let parsed: UiRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, request);
    }

    #[test]
    fn settings_requests_round_trip() {
        let request = UiRequest::SetSetting {
            key: "pauseFullscreen".to_string(),
            value: serde_json::json!(false),
        };
        let json = serde_json::to_string(&request).unwrap();
        assert_eq!(
            json,
            r#"{"type":"set-setting","key":"pauseFullscreen","value":false}"#
        );
        assert_eq!(serde_json::from_str::<UiRequest>(&json).unwrap(), request);
        assert_eq!(
            serde_json::to_string(&UiRequest::GetSettings).unwrap(),
            r#"{"type":"get-settings"}"#
        );
    }

    #[test]
    fn pong_shape_is_stable() {
        let response = UiResponse::Pong {
            id: 7,
            version: PROTOCOL_VERSION.to_string(),
        };
        let json = serde_json::to_string(&response).unwrap();
        assert_eq!(json, r#"{"type":"pong","id":7,"version":"0.1"}"#);
    }
}

pub mod mod_api;
pub use mod_api::*;
