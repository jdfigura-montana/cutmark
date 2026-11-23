//! Protocol v1 messages: parsing what the client sends, building what the app
//! answers, and diffing state snapshots (spec: Remote Control "Protocol v1").
//!
//! Every state and patch travels nested under its type key
//! (`{"type": "state", "state": {...}}`), so a state field can never collide with
//! the envelope fields.

use serde_json::{Map, Value, json};

/// The protocol version this crate speaks; a client that names another one is closed.
pub(crate) const PROTOCOL: u32 = 1;

/// A message from the client, parsed.
#[derive(Debug, PartialEq)]
pub(crate) enum ClientMsg {
    /// The authentication handshake.
    Hello { token: String, protocol: u32 },
    /// A command for the app, with its arguments (an empty object when the client sent none).
    Command { id: String, args: Value },
    /// A request for the full state.
    GetState,
}

/// A message the app sends to the client.
#[derive(Debug, PartialEq)]
pub(crate) enum ServerMsg {
    /// The handshake reply: the app's name, version and command list.
    Welcome { app: &'static str, version: String, commands: Vec<(String, String)> },
    /// A full state snapshot, sent after `welcome` and on `get_state`.
    State(Value),
    /// The changed top-level fields of the state.
    Patch(Value),
    /// An error, optionally about one command.
    Error { code: String, message: String, id: Option<String> },
}

/// Parses one client message; `None` for anything malformed or unknown.
pub(crate) fn parse(text: &str) -> Option<ClientMsg> {
    let value = serde_json::from_str::<Value>(text).ok()?;
    match value.get("type")?.as_str()? {
        "hello" => Some(ClientMsg::Hello {
            token: value.get("token")?.as_str()?.to_string(),
            protocol: value.get("protocol").and_then(Value::as_u64).unwrap_or(u64::from(PROTOCOL)) as u32,
        }),
        "command" => Some(ClientMsg::Command {
            id: value.get("id")?.as_str()?.to_string(),
            args: value.get("args").cloned().unwrap_or_else(|| Value::Object(Map::new())),
        }),
        "get_state" => Some(ClientMsg::GetState),
        _ => None,
    }
}

/// Builds the JSON text for a server message.
pub(crate) fn to_text(message: &ServerMsg) -> String {
    match message {
        ServerMsg::Welcome { app, version, commands } => json!({
            "type": "welcome",
            "app": app,
            "version": version,
            "protocol": PROTOCOL,
            "commands": commands.iter().map(|(id, label)| json!({ "id": id, "label": label })).collect::<Vec<_>>(),
        }),
        ServerMsg::State(state) => json!({ "type": "state", "state": state }),
        ServerMsg::Patch(patch) => json!({ "type": "patch", "patch": patch }),
        ServerMsg::Error { code, message, id } => match id {
            Some(id) => json!({ "type": "error", "code": code, "message": message, "id": id }),
            None => json!({ "type": "error", "code": code, "message": message }),
        },
    }
    .to_string()
}

/// The fields that turn `sent` into `latest`: the changed top-level fields, with
/// removed fields as `null`. `None` when nothing changed. A state that is not an
/// object cannot be patched and goes out whole.
pub(crate) fn diff(sent: &Value, latest: &Value) -> Option<Value> {
    let (sent, latest) = match (sent.as_object(), latest.as_object()) {
        (Some(sent), Some(latest)) => (sent, latest),
        _ => return Some(latest.clone()),
    };
    let mut patch = Map::new();
    for (key, value) in latest {
        if sent.get(key) != Some(value) {
            patch.insert(key.clone(), value.clone());
        }
    }
    for key in sent.keys() {
        if !latest.contains_key(key) {
            patch.insert(key.clone(), Value::Null);
        }
    }
    if patch.is_empty() { None } else { Some(Value::Object(patch)) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_hello_is_parsed_with_its_token_and_protocol() {
        let text = r#"{"type":"hello","token":"abc","client":"streamdeck","protocol":1}"#;
        assert_eq!(parse(text), Some(ClientMsg::Hello { token: "abc".to_string(), protocol: 1 }));
    }

    #[test]
    fn a_hello_without_a_token_is_rejected() {
        assert_eq!(parse(r#"{"type":"hello","protocol":1}"#), None);
    }

    #[test]
    fn a_command_is_parsed_with_or_without_args() {
        assert_eq!(
            parse(r#"{"type":"command","id":"play_pause","args":{"track":2}}"#),
            Some(ClientMsg::Command { id: "play_pause".to_string(), args: json!({ "track": 2 }) })
        );
        assert_eq!(parse(r#"{"type":"command","id":"play_pause"}"#), Some(ClientMsg::Command { id: "play_pause".to_string(), args: json!({}) }));
    }

    #[test]
    fn get_state_and_unknown_types_are_sorted_out() {
        assert_eq!(parse(r#"{"type":"get_state"}"#), Some(ClientMsg::GetState));
        assert_eq!(parse(r#"{"type":"nope"}"#), None);
        assert_eq!(parse("not json"), None);
    }

    #[test]
    fn a_welcome_is_built_with_the_command_list() {
        let commands = vec![("play_pause".to_string(), "Play or pause".to_string())];
        let text = to_text(&ServerMsg::Welcome { app: "audio", version: "1.3.0".to_string(), commands });
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["type"], "welcome");
        assert_eq!(value["app"], "audio");
        assert_eq!(value["version"], "1.3.0");
        assert_eq!(value["protocol"], 1);
        assert_eq!(value["commands"][0]["id"], "play_pause");
        assert_eq!(value["commands"][0]["label"], "Play or pause");
    }

    #[test]
    fn state_patch_and_error_texts_are_shaped() {
        let state: Value = serde_json::from_str(&to_text(&ServerMsg::State(json!({ "a": 1 })))).unwrap();
        assert_eq!(state["type"], "state");
        assert_eq!(state["state"]["a"], 1);
        let patch: Value = serde_json::from_str(&to_text(&ServerMsg::Patch(json!({ "a": 2 })))).unwrap();
        assert_eq!(patch["type"], "patch");
        assert_eq!(patch["patch"]["a"], 2);
        let with_id: Value =
            serde_json::from_str(&to_text(&ServerMsg::Error { code: "blocked".to_string(), message: "recording".to_string(), id: Some("record".to_string()) }))
                .unwrap();
        assert_eq!(with_id["type"], "error");
        assert_eq!(with_id["code"], "blocked");
        assert_eq!(with_id["message"], "recording");
        assert_eq!(with_id["id"], "record");
        let plain: Value = serde_json::from_str(&to_text(&ServerMsg::Error { code: "busy".to_string(), message: "busy".to_string(), id: None })).unwrap();
        assert_eq!(plain["code"], "busy");
        assert!(plain.get("id").is_none());
    }

    #[test]
    fn a_diff_reports_only_changed_fields() {
        let old = json!({ "a": 1, "b": 2 });
        assert_eq!(diff(&old, &json!({ "a": 1, "b": 2 })), None);
        assert_eq!(diff(&old, &json!({ "a": 9, "b": 2 })), Some(json!({ "a": 9 })));
        assert_eq!(diff(&old, &json!({ "a": 1, "b": 2, "c": 3 })), Some(json!({ "c": 3 })));
        assert_eq!(diff(&old, &json!({ "a": 1 })), Some(json!({ "b": null })));
        assert_eq!(diff(&json!([]), &json!({ "a": 1 })), Some(json!({ "a": 1 })));
    }
}
