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
    InstallImage,
    RemoveMod {
        folder_path: String,
    },
    SetPlaylistItems {
        items: Vec<String>,
    },
    SetPlaylistEnabled {
        enabled: bool,
    },
    SetPlaylistInterval {
        seconds: u32,
    },
    SetPlaylistRandom {
        random: bool,
    },
    PlaylistNext,
    OpenWallpapersFolder,
    RefreshLibrary,
    GetWallpaperVisibility,
    SetWallpaperVisibility {
        visible: bool,
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
    WallpaperVisibility { visible: bool },
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
    fn install_image_round_trips() {
        let json = serde_json::to_string(&UiRequest::InstallImage).unwrap();
        assert_eq!(json, r#"{"type":"install-image"}"#);
        assert_eq!(
            serde_json::from_str::<UiRequest>(&json).unwrap(),
            UiRequest::InstallImage
        );
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

    #[test]
    fn playlist_requests_round_trip() {
        let items = UiRequest::SetPlaylistItems {
            items: vec![r"C:\mods\a".into(), r"C:\mods\b".into()],
        };
        let json = serde_json::to_string(&items).unwrap();
        assert_eq!(
            json,
            r#"{"type":"set-playlist-items","items":["C:\\mods\\a","C:\\mods\\b"]}"#
        );
        assert_eq!(serde_json::from_str::<UiRequest>(&json).unwrap(), items);

        let enabled = UiRequest::SetPlaylistEnabled { enabled: true };
        let json = serde_json::to_string(&enabled).unwrap();
        assert_eq!(json, r#"{"type":"set-playlist-enabled","enabled":true}"#);
        assert_eq!(serde_json::from_str::<UiRequest>(&json).unwrap(), enabled);

        let interval = UiRequest::SetPlaylistInterval { seconds: 90 };
        let json = serde_json::to_string(&interval).unwrap();
        assert_eq!(json, r#"{"type":"set-playlist-interval","seconds":90}"#);
        assert_eq!(serde_json::from_str::<UiRequest>(&json).unwrap(), interval);

        let random = UiRequest::SetPlaylistRandom { random: true };
        let json = serde_json::to_string(&random).unwrap();
        assert_eq!(json, r#"{"type":"set-playlist-random","random":true}"#);
        assert_eq!(serde_json::from_str::<UiRequest>(&json).unwrap(), random);

        let next = UiRequest::PlaylistNext;
        let json = serde_json::to_string(&next).unwrap();
        assert_eq!(json, r#"{"type":"playlist-next"}"#);
        assert_eq!(serde_json::from_str::<UiRequest>(&json).unwrap(), next);

        let open = UiRequest::OpenWallpapersFolder;
        let json = serde_json::to_string(&open).unwrap();
        assert_eq!(json, r#"{"type":"open-wallpapers-folder"}"#);
        assert_eq!(serde_json::from_str::<UiRequest>(&json).unwrap(), open);

        let refresh = UiRequest::RefreshLibrary;
        let json = serde_json::to_string(&refresh).unwrap();
        assert_eq!(json, r#"{"type":"refresh-library"}"#);
        assert_eq!(serde_json::from_str::<UiRequest>(&json).unwrap(), refresh);

        let visibility = UiRequest::SetWallpaperVisibility { visible: false };
        let json = serde_json::to_string(&visibility).unwrap();
        assert_eq!(
            json,
            r#"{"type":"set-wallpaper-visibility","visible":false}"#
        );
        assert_eq!(
            serde_json::from_str::<UiRequest>(&json).unwrap(),
            visibility
        );
    }
}

pub mod mod_api;
pub use mod_api::*;
