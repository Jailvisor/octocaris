//! Frozen wallpaper-facing protocol (the old `octos.min.js` wire contract).
//!
//! ```text
//! request:     { type:"request", requestType, requestId, data }
//! response:    { type:"response", requestId, requestType, data }
//! command:     { type:"command", commandType, data }
//! subscribe:   { type:"subscribe"|"unsubscribe", eventType }
//! event:       { type:"event", eventType, data }
//! ```
//!
//! Field names are part of the public contract. Do not rename them.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MediaProperties {
    pub title: String,
    pub subtitle: String,
    pub album_artist: String,
    pub album_title: String,
    pub album_track_count: u32,
    pub artist: String,
    pub genres: Vec<String>,
    pub track_number: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackInfo {
    pub playback_status: String,
    pub playback_rate: f64,
    pub shuffle_active: bool,
    pub playback_type: String,
}

impl Default for PlaybackInfo {
    fn default() -> Self {
        Self {
            playback_status: "Closed".to_string(),
            playback_rate: 1.0,
            shuffle_active: false,
            playback_type: "Unknown".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TimelineProperties {
    pub start_time: f64,
    pub end_time: f64,
    pub position: f64,
    pub min_seek_time: f64,
    pub max_seek_time: f64,
}

/// Platform side of the protocol. Defaults are safe stubs so the dispatcher can
/// be tested without Windows; the real host overrides the relevant methods.
pub trait WallpaperHost {
    fn source(&mut self) -> String {
        String::new()
    }

    fn is_theme_light(&mut self) -> bool {
        false
    }

    fn visibility(&mut self) -> bool {
        true
    }

    fn monitor_id(&mut self) -> String {
        String::new()
    }

    fn siblings(&mut self) -> Vec<String> {
        Vec::new()
    }

    fn desktop_icons_visible(&mut self) -> bool {
        true
    }

    fn options(&mut self) -> Map<String, Value> {
        Map::new()
    }

    fn set_option(&mut self, _id: &str, _value: Value) -> bool {
        false
    }

    fn set_visible(&mut self, _visible: bool) -> bool {
        true
    }

    fn set_desktop_icons_visible(&mut self, _visible: bool) -> bool {
        true
    }

    fn open_dev_tools(&mut self) {}

    fn media_properties(&mut self) -> MediaProperties {
        MediaProperties::default()
    }

    fn playback_info(&mut self) -> PlaybackInfo {
        PlaybackInfo::default()
    }

    fn timeline_properties(&mut self) -> TimelineProperties {
        TimelineProperties::default()
    }

    fn thumbnail(&mut self) -> String {
        String::new()
    }

    fn media_command(&mut self, _cmd: &str, _position: Option<f64>) {}

    fn subscribe(&mut self, _event_type: &str) {}

    fn unsubscribe(&mut self, _event_type: &str) {}

    /// Send `redirect_event` to another monitor. The host knows its own monitor id.
    fn redirect(&mut self, _receiver_id: &str, _message: Value) {}
}

/// Handle one wallpaper message. Returns the response that should be posted back
/// to the same wallpaper, or `None` for commands/subscriptions/unknown messages.
pub fn dispatch(message: &Value, host: &mut dyn WallpaperHost) -> Option<Value> {
    let message_type = message.get("type").and_then(Value::as_str)?;
    match message_type {
        "request" => dispatch_request(message, host),
        "command" => {
            dispatch_command(message, host);
            None
        }
        "subscribe" | "unsubscribe" => {
            let event_type = message.get("eventType").and_then(Value::as_str)?;
            if message_type == "subscribe" {
                host.subscribe(event_type);
            } else {
                host.unsubscribe(event_type);
            }
            None
        }
        _ => None,
    }
}

fn dispatch_request(message: &Value, host: &mut dyn WallpaperHost) -> Option<Value> {
    let request_id = message.get("requestId").and_then(Value::as_str)?;
    let request_type = message.get("requestType").and_then(Value::as_str)?;

    let data = match request_type {
        "source" => Value::String(host.source()),
        "is-theme-light" => Value::Bool(host.is_theme_light()),
        "visibility" => Value::Bool(host.visibility()),
        "monitor-id" => Value::String(host.monitor_id()),
        "siblings" => json!(host.siblings()),
        "desktop-icons-visibility" => Value::Bool(host.desktop_icons_visible()),
        "options" => Value::Object(host.options()),
        "media-props" => serde_json::to_value(host.media_properties()).unwrap_or(Value::Null),
        "playback-info" => serde_json::to_value(host.playback_info()).unwrap_or(Value::Null),
        "timeline-props" => serde_json::to_value(host.timeline_properties()).unwrap_or(Value::Null),
        "thumbnail" => Value::String(host.thumbnail()),
        // Keep the old behavior: unknown requests do not resolve a promise.
        _ => return None,
    };

    Some(json!({
        "type": "response",
        "requestId": request_id,
        "requestType": request_type,
        "data": data
    }))
}

fn dispatch_command(message: &Value, host: &mut dyn WallpaperHost) {
    let Some(command_type) = message.get("commandType").and_then(Value::as_str) else {
        return;
    };
    let data = message.get("data").cloned().unwrap_or_else(|| json!({}));

    match command_type {
        "open-dev-tools" => host.open_dev_tools(),
        "set-visible" => {
            host.set_visible(true);
        }
        "set-hidden" => {
            host.set_visible(false);
        }
        "set-desktop-icons-visible" => {
            host.set_desktop_icons_visible(true);
        }
        "set-desktop-icons-hidden" => {
            host.set_desktop_icons_visible(false);
        }
        "media" => {
            let cmd = data.get("cmd").and_then(Value::as_str).unwrap_or_default();
            let position = data.get("position").and_then(Value::as_f64);
            host.media_command(cmd, position);
        }
        "set-option" => {
            if let (Some(id), Some(value)) = (
                data.get("id").and_then(Value::as_str),
                data.get("value").cloned(),
            ) {
                host.set_option(id, value);
            }
        }
        "redirect" => {
            if let (Some(receiver_id), Some(message)) = (
                data.get("receiver-id").and_then(Value::as_str),
                data.get("message").cloned(),
            ) {
                host.redirect(receiver_id, message);
            }
        }
        // Old App-window forwarding path; not used by the Rust host.
        "send-to-monitor-hwnd" => {}
        _ => {}
    }
}

pub fn event(event_type: &str, data: Value) -> Value {
    json!({
        "type": "event",
        "eventType": event_type,
        "data": data
    })
}

pub fn options_event(event_type: &str, id: &str, value: &Value) -> Value {
    event(
        event_type,
        json!({
            "id": id,
            "value": value
        }),
    )
}

pub fn redirect_event(sender_id: &str, receiver_id: &str, message: Value) -> Value {
    event(
        "redirect",
        json!({
            "sender-id": sender_id,
            "receiver-id": receiver_id,
            "message": message
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Default)]
    struct MockHost {
        source: String,
        theme_light: bool,
        visible: bool,
        monitor_id: String,
        siblings: Vec<String>,
        desktop_icons: bool,
        options: Map<String, Value>,
        set_options: Vec<(String, Value)>,
        visibility_calls: Vec<bool>,
        desktop_icon_calls: Vec<bool>,
        devtools_calls: u32,
        media_commands: Vec<(String, Option<f64>)>,
        subscriptions: Vec<String>,
        unsubscriptions: Vec<String>,
        redirects: Vec<(String, Value)>,
    }

    impl WallpaperHost for MockHost {
        fn source(&mut self) -> String {
            self.source.clone()
        }
        fn is_theme_light(&mut self) -> bool {
            self.theme_light
        }
        fn visibility(&mut self) -> bool {
            self.visible
        }
        fn monitor_id(&mut self) -> String {
            self.monitor_id.clone()
        }
        fn siblings(&mut self) -> Vec<String> {
            self.siblings.clone()
        }
        fn desktop_icons_visible(&mut self) -> bool {
            self.desktop_icons
        }
        fn options(&mut self) -> Map<String, Value> {
            self.options.clone()
        }
        fn set_option(&mut self, id: &str, value: Value) -> bool {
            self.set_options.push((id.to_string(), value));
            true
        }
        fn set_visible(&mut self, visible: bool) -> bool {
            self.visibility_calls.push(visible);
            true
        }
        fn set_desktop_icons_visible(&mut self, visible: bool) -> bool {
            self.desktop_icon_calls.push(visible);
            true
        }
        fn open_dev_tools(&mut self) {
            self.devtools_calls += 1;
        }
        fn media_command(&mut self, cmd: &str, position: Option<f64>) {
            self.media_commands.push((cmd.to_string(), position));
        }
        fn subscribe(&mut self, event_type: &str) {
            self.subscriptions.push(event_type.to_string());
        }
        fn unsubscribe(&mut self, event_type: &str) {
            self.unsubscriptions.push(event_type.to_string());
        }
        fn redirect(&mut self, receiver_id: &str, message: Value) {
            self.redirects.push((receiver_id.to_string(), message));
        }
    }

    fn request(request_type: &str) -> Value {
        json!({
            "type": "request",
            "requestType": request_type,
            "requestId": "req-1",
            "data": {}
        })
    }

    #[test]
    fn system_requests_echo_request_id_and_type() {
        let mut host = MockHost {
            source: "https://local.octos/test/index.html".to_string(),
            theme_light: true,
            visible: true,
            monitor_id: "\\\\.\\DISPLAY1".to_string(),
            siblings: vec!["\\\\.\\DISPLAY2".to_string()],
            desktop_icons: true,
            ..Default::default()
        };

        let response = dispatch(&request("source"), &mut host).unwrap();
        assert_eq!(response["type"], "response");
        assert_eq!(response["requestId"], "req-1");
        assert_eq!(response["requestType"], "source");
        assert_eq!(response["data"], "https://local.octos/test/index.html");

        assert_eq!(
            dispatch(&request("is-theme-light"), &mut host).unwrap()["data"],
            true
        );
        assert_eq!(
            dispatch(&request("visibility"), &mut host).unwrap()["data"],
            true
        );
        assert_eq!(
            dispatch(&request("monitor-id"), &mut host).unwrap()["data"],
            "\\\\.\\DISPLAY1"
        );
        assert_eq!(
            dispatch(&request("siblings"), &mut host).unwrap()["data"],
            json!(["\\\\.\\DISPLAY2"])
        );
        assert_eq!(
            dispatch(&request("desktop-icons-visibility"), &mut host).unwrap()["data"],
            true
        );
    }

    #[test]
    fn options_request_returns_full_object_or_empty_object() {
        let mut host = MockHost::default();
        assert_eq!(
            dispatch(&request("options"), &mut host).unwrap()["data"],
            json!({})
        );

        let mut options = Map::new();
        options.insert(
            "speed".to_string(),
            json!({"type": "range", "label": "Speed", "value": 3}),
        );
        host.options = options;
        let response = dispatch(&request("options"), &mut host).unwrap();
        assert_eq!(response["data"]["speed"]["value"], 3);
    }

    #[test]
    fn media_requests_resolve_stubs() {
        let mut host = MockHost::default();
        let media = dispatch(&request("media-props"), &mut host).unwrap();
        assert_eq!(media["data"]["title"], "");
        assert_eq!(media["data"]["genres"], json!([]));

        let playback = dispatch(&request("playback-info"), &mut host).unwrap();
        assert_eq!(playback["data"]["playbackStatus"], "Closed");
        assert_eq!(playback["data"]["playbackRate"], 1.0);
        assert_eq!(playback["data"]["playbackType"], "Unknown");

        let timeline = dispatch(&request("timeline-props"), &mut host).unwrap();
        assert_eq!(timeline["data"]["position"], 0.0);

        let thumbnail = dispatch(&request("thumbnail"), &mut host).unwrap();
        assert_eq!(thumbnail["data"], "");
    }

    #[test]
    fn unknown_requests_do_not_resolve() {
        let mut host = MockHost::default();
        assert!(dispatch(&request("custom-thing"), &mut host).is_none());
    }

    #[test]
    fn commands_apply_side_effects_without_a_response() {
        let mut host = MockHost::default();

        let command = json!({
            "type": "command",
            "commandType": "set-option",
            "data": {"id": "speed", "value": 4}
        });
        assert!(dispatch(&command, &mut host).is_none());
        assert_eq!(host.set_options, vec![("speed".to_string(), json!(4))]);

        let media = json!({
            "type": "command",
            "commandType": "media",
            "data": {"cmd": "set-playback-position", "position": 12.5}
        });
        dispatch(&media, &mut host);
        assert_eq!(
            host.media_commands,
            vec![("set-playback-position".to_string(), Some(12.5))]
        );

        dispatch(
            &json!({"type":"command","commandType":"set-visible","data":{}}),
            &mut host,
        );
        dispatch(
            &json!({"type":"command","commandType":"set-hidden","data":{}}),
            &mut host,
        );
        assert_eq!(host.visibility_calls, vec![true, false]);

        dispatch(
            &json!({"type":"command","commandType":"set-desktop-icons-hidden","data":{}}),
            &mut host,
        );
        assert_eq!(host.desktop_icon_calls, vec![false]);

        dispatch(
            &json!({"type":"command","commandType":"open-dev-tools","data":{}}),
            &mut host,
        );
        assert_eq!(host.devtools_calls, 1);

        dispatch(
            &json!({
                "type":"command",
                "commandType":"redirect",
                "data":{"receiver-id":"\\\\.\\DISPLAY2","message":{"hello":true}}
            }),
            &mut host,
        );
        assert_eq!(
            host.redirects,
            vec![("\\\\.\\DISPLAY2".to_string(), json!({"hello": true}))]
        );
    }

    #[test]
    fn subscriptions_are_forwarded() {
        let mut host = MockHost::default();
        dispatch(
            &json!({"type":"subscribe","eventType":"mediaChange"}),
            &mut host,
        );
        dispatch(
            &json!({"type":"unsubscribe","eventType":"mediaChange"}),
            &mut host,
        );
        assert_eq!(host.subscriptions, vec!["mediaChange"]);
        assert_eq!(host.unsubscriptions, vec!["mediaChange"]);
    }

    #[test]
    fn event_shapes_match_the_frozen_contract() {
        assert_eq!(
            options_event("options-change", "speed", &json!(3)),
            json!({
                "type": "event",
                "eventType": "options-change",
                "data": {"id": "speed", "value": 3}
            })
        );
        assert_eq!(
            redirect_event("\\\\.\\DISPLAY1", "\\\\.\\DISPLAY2", json!({"x": 1})),
            json!({
                "type": "event",
                "eventType": "redirect",
                "data": {
                    "sender-id": "\\\\.\\DISPLAY1",
                    "receiver-id": "\\\\.\\DISPLAY2",
                    "message": {"x": 1}
                }
            })
        );
    }
}
