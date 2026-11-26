//! The control API's messages: parsing what clients send and building what the server replies and announces.
//!
//! A client message is `{"op": "...", "id"?: any, ...args}`. Arguments may sit at the top level or inside a `data`
//! object, as the spec's example does. The server echoes `id` in the reply to that message.

use ::marks::MarkType;
use serde::Serialize;
use serde_json::{Map, Value, json};
use thiserror::Error;

use crate::config::{CompositeMode, TrackKind};
#[cfg(test)]
use crate::engine::controller::WarningNote;
use crate::engine::controller::{ControllerState, Phase};

/// The longest mark label the API keeps, in bytes.
const LABEL_MAX_BYTES: usize = 200;

/// The largest script snapshot the API keeps, in bytes.
const MARKDOWN_MAX_BYTES: usize = 2 * 1024 * 1024;

/// What a client asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// The first message of a connection, carrying the token.
    Auth {
        /// The token the client holds.
        token: String,
        /// What kind of client this is, such as `prompter`; none when the client did not say.
        client: Option<String>,
    },
    /// Start a take.
    Start,
    /// Stop the running take.
    Stop,
    /// Pause the running take.
    Pause,
    /// Resume a paused take.
    Resume,
    /// Add a mark to the running take.
    Mark {
        /// The mark type, by category name.
        category: MarkType,
        /// The label, if any.
        label: Option<String>,
    },
    /// Toggle mute on a track.
    Mute {
        /// The track name, or none for the mic track.
        track: Option<String>,
    },
    /// Switch to a profile while idle.
    SetProfile {
        /// The profile name.
        name: String,
    },
    /// Ask for the current status.
    GetStatus,
    /// The Prompter's script, kept for the script file of the next take that gets a position.
    ScriptSnapshot {
        /// Identifies the script across messages.
        script_id: String,
        /// The script's title.
        title: String,
        /// The script as Markdown, up to 2 MB.
        markdown: String,
    },
    /// Where the Prompter is in the script.
    ScriptPosition {
        /// Identifies the script across messages.
        script_id: String,
        /// The script's title.
        title: String,
        /// The word or token the Prompter is at.
        token: String,
        /// The line number the Prompter is at.
        line: u64,
        /// The text of that line.
        text: String,
    },
}

impl Op {
    /// Whether this is one of the two Prompter script commands, which reply only when the message has an `id`.
    pub fn is_script(&self) -> bool {
        matches!(self, Self::ScriptSnapshot { .. } | Self::ScriptPosition { .. })
    }
}

/// One parsed client message.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    /// The client's `id`, echoed in the reply.
    pub id: Option<Value>,
    /// What the client asked for.
    pub op: Op,
}

/// Why a client message could not be understood.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ParseError {
    /// The text is not JSON.
    #[error("the message is not valid JSON: {0}")]
    NotJson(String),
    /// The JSON is not an object.
    #[error("the message must be a JSON object")]
    NotObject,
    /// The object has no `op`.
    #[error("the message has no op")]
    MissingOp,
    /// The `op` is not one the server knows.
    #[error("unknown op: {0}")]
    UnknownOp(String),
    /// An argument is missing, has the wrong type or has a value that is not allowed.
    #[error("bad argument: {0}")]
    BadArgument(String),
}

/// A message that could not be understood, with the `id` it carried if it got that far.
#[derive(Debug, Clone, PartialEq)]
pub struct Rejected {
    /// The client's `id`, if the message was an object that had one.
    pub id: Option<Value>,
    /// What was wrong.
    pub error: ParseError,
}

/// Reads one client message.
pub fn parse(text: &str) -> Result<Request, Rejected> {
    let value: Value = serde_json::from_str(text).map_err(|err| Rejected { id: None, error: ParseError::NotJson(err.to_string()) })?;
    let Value::Object(object) = value else {
        return Err(Rejected { id: None, error: ParseError::NotObject });
    };
    let id = object.get("id").cloned();
    match parse_op(&object) {
        Ok(op) => Ok(Request { id, op }),
        Err(error) => Err(Rejected { id, error }),
    }
}

/// Where a message keeps its arguments: the `data` object first, then the top level.
struct Args<'a> {
    top: &'a Map<String, Value>,
    data: Option<&'a Map<String, Value>>,
}

impl<'a> Args<'a> {
    /// The arguments of `object`; fails when `data` is present but is not an object.
    fn new(object: &'a Map<String, Value>) -> Result<Self, ParseError> {
        let data = match object.get("data") {
            None | Some(Value::Null) => None,
            Some(Value::Object(data)) => Some(data),
            Some(_) => return Err(ParseError::BadArgument("data must be an object".to_owned())),
        };
        Ok(Self { top: object, data })
    }

    /// The first of `keys` that is present and not null, as a string.
    fn string(&self, keys: &[&str]) -> Result<Option<String>, ParseError> {
        for key in keys {
            match self.data.and_then(|data| data.get(*key)).or_else(|| self.top.get(*key)) {
                None | Some(Value::Null) => {}
                Some(Value::String(text)) => return Ok(Some(text.clone())),
                Some(_) => return Err(ParseError::BadArgument(format!("{key} must be a string"))),
            }
        }
        Ok(None)
    }

    /// The first of `keys` that is present and not null, as a whole number.
    fn unsigned(&self, keys: &[&str]) -> Result<Option<u64>, ParseError> {
        for key in keys {
            match self.data.and_then(|data| data.get(*key)).or_else(|| self.top.get(*key)) {
                None | Some(Value::Null) => {}
                Some(value) => return value.as_u64().map(Some).ok_or_else(|| ParseError::BadArgument(format!("{key} must be a whole number"))),
            }
        }
        Ok(None)
    }

    /// Like [`Args::string`], but the argument must be there and not empty.
    fn required(&self, keys: &[&str]) -> Result<String, ParseError> {
        match self.string(keys)? {
            Some(text) if !text.trim().is_empty() => Ok(text),
            _ => Err(ParseError::BadArgument(format!("{} is required", keys.first().copied().unwrap_or("argument")))),
        }
    }
}

/// Reads the `op` and its arguments from a message object.
fn parse_op(object: &Map<String, Value>) -> Result<Op, ParseError> {
    let op = match object.get("op") {
        Some(Value::String(op)) => op.as_str(),
        Some(_) => return Err(ParseError::BadArgument("op must be a string".to_owned())),
        None => return Err(ParseError::MissingOp),
    };
    let args = Args::new(object)?;
    match op {
        "auth" => Ok(Op::Auth { token: args.required(&["token"])?, client: args.string(&["client"])?.map(|client| client.trim().to_owned()) }),
        "start" => Ok(Op::Start),
        "stop" => Ok(Op::Stop),
        "pause" => Ok(Op::Pause),
        "resume" => Ok(Op::Resume),
        "mark" => {
            let category = match args.string(&["type", "category"])? {
                Some(name) => parse_category(&name)?,
                None => MarkType::Note,
            };
            let label = args.string(&["label"])?.map(|label| clip_label(label.trim())).filter(|label| !label.is_empty());
            Ok(Op::Mark { category, label })
        }
        "mute" => Ok(Op::Mute { track: args.string(&["track"])?.filter(|track| !track.trim().is_empty()) }),
        "setProfile" => Ok(Op::SetProfile { name: args.required(&["name"])? }),
        "getStatus" => Ok(Op::GetStatus),
        "scriptSnapshot" => {
            let markdown = args.string(&["markdown"])?.unwrap_or_default();
            if markdown.len() > MARKDOWN_MAX_BYTES {
                return Err(ParseError::BadArgument("markdown is larger than 2 MB".to_owned()));
            }
            Ok(Op::ScriptSnapshot { script_id: args.required(&["scriptId"])?, title: args.string(&["title"])?.unwrap_or_default(), markdown })
        }
        "scriptPosition" => Ok(Op::ScriptPosition {
            script_id: args.required(&["scriptId"])?,
            title: args.string(&["title"])?.unwrap_or_default(),
            token: args.string(&["token"])?.unwrap_or_default(),
            line: args.unsigned(&["line"])?.ok_or_else(|| ParseError::BadArgument("line is required".to_owned()))?,
            text: args.string(&["text"])?.unwrap_or_default(),
        }),
        other => Err(ParseError::UnknownOp(other.to_owned())),
    }
}

/// The mark type a category name stands for, ignoring case, spaces, hyphens and underscores.
///
/// System marks belong to the app, so they cannot be asked for.
pub fn parse_category(name: &str) -> Result<MarkType, ParseError> {
    let key: String = name.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect();
    match key.as_str() {
        "highlight" => Ok(MarkType::Highlight),
        "cut" => Ok(MarkType::Cut),
        "retake" => Ok(MarkType::Retake),
        "chapter" => Ok(MarkType::Chapter),
        "note" => Ok(MarkType::Note),
        "broll" => Ok(MarkType::BRoll),
        _ => Err(ParseError::BadArgument(format!("unknown mark type: {name}"))),
    }
}

/// `label` cut to the longest length the API keeps, at a character boundary.
fn clip_label(label: &str) -> String {
    let mut end = label.len().min(LABEL_MAX_BYTES);
    while !label.is_char_boundary(end) {
        end -= 1;
    }
    label[..end].to_owned()
}

/// The events the server announces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    /// The countdown before a take began.
    CountdownStarted,
    /// The countdown was cancelled, so no take starts.
    CountdownCancelled,
    /// A take started.
    RecordingStarted,
    /// A take ended.
    RecordingStopped,
    /// The take was paused.
    Paused,
    /// The take was resumed.
    Resumed,
    /// A mark was added.
    MarkAdded,
    /// Something went wrong that did not stop the Recorder.
    Warning,
    /// The once-a-second status while recording.
    Status,
}

impl EventKind {
    /// The name the event has on the wire.
    pub fn name(self) -> &'static str {
        match self {
            Self::CountdownStarted => "countdownStarted",
            Self::CountdownCancelled => "countdownCancelled",
            Self::RecordingStarted => "recordingStarted",
            Self::RecordingStopped => "recordingStopped",
            Self::Paused => "paused",
            Self::Resumed => "resumed",
            Self::MarkAdded => "markAdded",
            Self::Warning => "warning",
            Self::Status => "status",
        }
    }
}

/// The reply to a good `auth`.
pub fn auth_ok() -> String {
    json!({ "op": "auth", "ok": true }).to_string()
}

/// A connection-level error such as `unauthorized`; the server closes the connection after it.
pub fn error_message(code: &str) -> String {
    json!({ "op": "error", "code": code }).to_string()
}

/// The reply to a command that worked.
pub fn result_ok(id: Option<&Value>, data: Value) -> String {
    json!({ "op": "result", "id": id, "ok": true, "data": data }).to_string()
}

/// The reply to a command that failed or could not be understood.
pub fn result_error(id: Option<&Value>, message: &str) -> String {
    json!({ "op": "result", "id": id, "ok": false, "error": message }).to_string()
}

/// An event message.
pub fn event(kind: EventKind, data: Value) -> String {
    json!({ "op": "event", "event": kind.name(), "data": data }).to_string()
}

/// The `warning` events for the warnings raised after number `after`, oldest first, each with its number.
///
/// A client that has sent everything up to number N calls this with N after every state change.
pub fn warning_events(state: &ControllerState, after: u64) -> Vec<(u64, String)> {
    state
        .warnings
        .iter()
        .filter(|note| note.seq > after)
        .map(|note| (note.seq, event(EventKind::Warning, json!({ "code": note.code, "message": note.message }))))
        .collect()
}

/// The recorder's status as clients see it, in `getStatus` and in the `status` and phase events.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusData {
    /// `idle`, `countdown`, `recording` or `paused`.
    pub state: &'static str,
    /// How much of the take has been recorded, in milliseconds; 0 when idle.
    pub elapsed_ms: u64,
    /// The path of the take's video file while a take runs.
    pub take: Option<String>,
    /// The active profile name.
    pub profile: String,
    /// How many marks the running or last take has.
    pub marks_count: usize,
    /// Frames repeated because no new one arrived in time.
    pub duplicated: u64,
    /// Frames dropped because the encoder was behind.
    pub dropped: u64,
    /// Bytes of all the take's files so far; 0 when idle.
    pub bytes: u64,
    /// The current data rate in kilobits a second; 0 when idle.
    pub bitrate_kbps: u64,
    /// Free space on the take's volume in bytes, or on the take folder's volume when idle; none when unknown.
    pub disk_free_bytes: Option<u64>,
    /// Seconds until the disk is full at the current data rate; none when unknown or not recording.
    pub remaining_secs: Option<u64>,
    /// Packets waiting for the muxer; 0 when idle.
    pub queue_depth: usize,
    /// The effective composite mode, `baked` or `clean`: the running take's, or the next take's when idle.
    pub composite_mode: CompositeMode,
    /// The audio tracks in the settings, in settings order.
    pub tracks: Vec<TrackStatus>,
}

/// One audio track as clients see it in the status.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrackStatus {
    /// The track name.
    pub name: String,
    /// `input`, `output` or `app`.
    pub kind: &'static str,
    /// The executable an app track records; absent for a device track.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exe: Option<String>,
    /// Whether the running take is capturing the track's device or app; false while no take records.
    pub attached: bool,
}

/// The status of every track in the settings. The take's counters cover only the enabled tracks, in the same order.
fn track_statuses(state: &ControllerState) -> Vec<TrackStatus> {
    let live = state.stats.as_ref().filter(|_| matches!(state.phase, Phase::Recording | Phase::Paused));
    let mut slot = 0;
    state
        .track_names
        .iter()
        .zip(&state.track_kinds)
        .zip(&state.track_enabled)
        .map(|((name, kind), enabled)| {
            let attached = *enabled && live.and_then(|stats| stats.tracks.get(slot)).is_some_and(|track| track.attached);
            slot += usize::from(*enabled);
            let (kind, exe) = match kind {
                TrackKind::Input => ("input", None),
                TrackKind::Output => ("output", None),
                TrackKind::App { exe, .. } => ("app", Some(exe.clone())),
            };
            TrackStatus { name: name.clone(), kind, exe, attached }
        })
        .collect()
}

impl StatusData {
    /// The status that the controller state describes.
    pub fn from_state(state: &ControllerState) -> Self {
        let fps = u64::from(state.fps.max(1));
        Self {
            state: match state.phase {
                Phase::Idle => "idle",
                Phase::Countdown(_) => "countdown",
                Phase::Recording => "recording",
                Phase::Paused => "paused",
            },
            elapsed_ms: state.stats.as_ref().map_or(0, |stats| stats.frames.saturating_mul(1000) / fps),
            take: state.take.as_ref().map(|take| take.video_file.display().to_string()),
            profile: state.active_profile.clone(),
            marks_count: state.marks_count,
            duplicated: state.stats.as_ref().map_or(0, |stats| stats.duplicated),
            dropped: state.stats.as_ref().map_or(0, |stats| stats.dropped),
            bytes: state.take_stats.as_ref().map_or(0, |live| live.bytes),
            bitrate_kbps: state.take_stats.as_ref().map_or(0, |live| live.bitrate_kbps().round() as u64),
            disk_free_bytes: state.take_stats.as_ref().map_or(state.idle_free_bytes, |live| live.disk_free_bytes),
            remaining_secs: state.take_stats.as_ref().and_then(|live| live.remaining_secs),
            queue_depth: state.take_stats.as_ref().map_or(0, |live| live.queue_depth),
            composite_mode: state.composite_mode,
            tracks: track_statuses(state),
        }
    }

    /// The status as a JSON value.
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn parsed(text: &str) -> Request {
        parse(text).unwrap_or_else(|rejected| Request { id: rejected.id, op: Op::Auth { token: format!("rejected: {}", rejected.error), client: None } })
    }

    fn rejected(text: &str) -> Rejected {
        match parse(text) {
            Err(rejected) => rejected,
            Ok(request) => Rejected { id: request.id, error: ParseError::UnknownOp(format!("accepted as {:?}", request.op)) },
        }
    }

    #[test]
    fn every_plain_command_parses_and_keeps_its_id() {
        for (text, op) in [
            (r#"{"op":"start","id":1}"#, Op::Start),
            (r#"{"op":"stop","id":"a"}"#, Op::Stop),
            (r#"{"op":"pause"}"#, Op::Pause),
            (r#"{"op":"resume"}"#, Op::Resume),
            (r#"{"op":"getStatus"}"#, Op::GetStatus),
        ] {
            assert_eq!(parsed(text).op, op, "{text}");
        }
        assert_eq!(parsed(r#"{"op":"start","id":{"n":7}}"#).id, Some(json!({ "n": 7 })));
        assert_eq!(parsed(r#"{"op":"start"}"#).id, None);
    }

    #[test]
    fn auth_needs_a_token() {
        assert_eq!(parsed(r#"{"op":"auth","token":"abc"}"#).op, Op::Auth { token: "abc".to_owned(), client: None });
        assert_eq!(parsed(r#"{"op":"auth","token":"abc","client":"prompter"}"#).op, Op::Auth { token: "abc".to_owned(), client: Some("prompter".to_owned()) });
        assert!(matches!(rejected(r#"{"op":"auth","token":"abc","client":3}"#).error, ParseError::BadArgument(_)));
        assert!(matches!(rejected(r#"{"op":"auth"}"#).error, ParseError::BadArgument(_)));
        assert!(matches!(rejected(r#"{"op":"auth","token":5}"#).error, ParseError::BadArgument(_)));
        assert!(matches!(rejected(r#"{"op":"auth","token":"  "}"#).error, ParseError::BadArgument(_)));
    }

    #[test]
    fn the_status_lists_each_track_with_its_kind_and_exe() {
        let state = ControllerState {
            track_names: vec!["Mic".to_owned(), "Discord".to_owned()],
            track_kinds: vec![TrackKind::Input, TrackKind::App { exe: "discord.exe".to_owned(), app_name: "Discord".to_owned() }],
            track_enabled: vec![true, true],
            ..ControllerState::default()
        };
        assert_eq!(
            StatusData::from_state(&state).to_value()["tracks"],
            json!([{ "name": "Mic", "kind": "input", "attached": false }, { "name": "Discord", "kind": "app", "exe": "discord.exe", "attached": false }])
        );
    }

    #[test]
    fn a_mark_defaults_to_a_note_and_takes_its_arguments_from_the_top_level_or_data() {
        assert_eq!(parsed(r#"{"op":"mark"}"#).op, Op::Mark { category: MarkType::Note, label: None });
        assert_eq!(
            parsed(r#"{"op":"mark","type":"Retake","label":"redo intro"}"#).op,
            Op::Mark { category: MarkType::Retake, label: Some("redo intro".to_owned()) }
        );
        let spec = parsed(r#"{"op":"mark","id":7,"data":{"category":"b-roll","label":"  cover  "}}"#);
        assert_eq!(spec.id, Some(json!(7)));
        assert_eq!(spec.op, Op::Mark { category: MarkType::BRoll, label: Some("cover".to_owned()) });
        assert_eq!(parsed(r#"{"op":"mark","label":""}"#).op, Op::Mark { category: MarkType::Note, label: None });
    }

    #[test]
    fn a_mark_with_a_bad_type_or_label_is_rejected_with_its_id() {
        let unknown = rejected(r#"{"op":"mark","id":3,"type":"Banana"}"#);
        assert_eq!(unknown.id, Some(json!(3)));
        assert!(matches!(unknown.error, ParseError::BadArgument(_)));
        assert!(matches!(rejected(r#"{"op":"mark","type":"System"}"#).error, ParseError::BadArgument(_)));
        assert!(matches!(rejected(r#"{"op":"mark","type":4}"#).error, ParseError::BadArgument(_)));
        assert!(matches!(rejected(r#"{"op":"mark","label":false}"#).error, ParseError::BadArgument(_)));
        assert!(matches!(rejected(r#"{"op":"mark","data":"x"}"#).error, ParseError::BadArgument(_)));
    }

    #[test]
    fn a_long_label_is_cut_at_a_character_boundary() {
        let long = "é".repeat(150);
        let request = parsed(&json!({ "op": "mark", "label": long }).to_string());
        let label = match request.op {
            Op::Mark { label, .. } => label.unwrap_or_default(),
            _ => String::new(),
        };
        assert_eq!(label.len(), 200);
        assert!(label.chars().all(|c| c == 'é'));
    }

    #[test]
    fn mute_and_set_profile_take_names() {
        assert_eq!(parsed(r#"{"op":"mute"}"#).op, Op::Mute { track: None });
        assert_eq!(parsed(r#"{"op":"mute","track":"Desktop"}"#).op, Op::Mute { track: Some("Desktop".to_owned()) });
        assert_eq!(parsed(r#"{"op":"setProfile","name":"Premiere HEVC 1080p60"}"#).op, Op::SetProfile { name: "Premiere HEVC 1080p60".to_owned() });
        assert!(matches!(rejected(r#"{"op":"setProfile"}"#).error, ParseError::BadArgument(_)));
        assert!(matches!(rejected(r#"{"op":"mute","track":[1]}"#).error, ParseError::BadArgument(_)));
    }

    #[test]
    fn script_commands_parse_with_or_without_an_id() {
        let snapshot = parsed(r##"{"op":"scriptSnapshot","id":4,"scriptId":"s1","title":"Fake script","markdown":"# Hi"}"##);
        assert_eq!(snapshot.id, Some(json!(4)));
        assert_eq!(snapshot.op, Op::ScriptSnapshot { script_id: "s1".to_owned(), title: "Fake script".to_owned(), markdown: "# Hi".to_owned() });
        let position = parsed(r#"{"op":"scriptPosition","data":{"scriptId":"s1","title":"T","token":"Hi","line":8,"text":"Hi there"}}"#);
        assert_eq!(position.id, None);
        assert_eq!(
            position.op,
            Op::ScriptPosition { script_id: "s1".to_owned(), title: "T".to_owned(), token: "Hi".to_owned(), line: 8, text: "Hi there".to_owned() }
        );
        assert!(position.op.is_script() && snapshot.op.is_script() && !Op::Start.is_script());
        assert!(matches!(rejected(r#"{"op":"scriptPosition","scriptId":"s1"}"#).error, ParseError::BadArgument(_)));
        assert!(matches!(rejected(r#"{"op":"scriptPosition","scriptId":"s1","line":"x"}"#).error, ParseError::BadArgument(_)));
        assert!(matches!(rejected(r#"{"op":"scriptSnapshot","markdown":"x"}"#).error, ParseError::BadArgument(_)));
    }

    #[test]
    fn a_snapshot_may_be_2_mb_but_not_more() {
        let fits = json!({ "op": "scriptSnapshot", "scriptId": "s", "markdown": "x".repeat(MARKDOWN_MAX_BYTES) });
        assert!(parse(&fits.to_string()).is_ok());
        let too_big = json!({ "op": "scriptSnapshot", "id": 2, "scriptId": "s", "markdown": "x".repeat(MARKDOWN_MAX_BYTES + 1) });
        let failure = parse(&too_big.to_string()).err().map(|failed| (failed.id, failed.error));
        assert!(matches!(failure, Some((Some(_), ParseError::BadArgument(_)))));
    }

    #[test]
    fn unknown_ops_and_malformed_messages_are_rejected() {
        let unknown = rejected(r#"{"op":"explode","id":9}"#);
        assert_eq!(unknown.id, Some(json!(9)));
        assert_eq!(unknown.error, ParseError::UnknownOp("explode".to_owned()));
        assert_eq!(rejected(r#"{"id":1}"#).error, ParseError::MissingOp);
        assert!(matches!(rejected(r#"{"op":5}"#).error, ParseError::BadArgument(_)));
        assert_eq!(rejected("[1,2]").error, ParseError::NotObject);
        assert_eq!(rejected("\"start\"").error, ParseError::NotObject);
        assert!(matches!(rejected("{not json").error, ParseError::NotJson(_)));
        assert!(matches!(rejected("").error, ParseError::NotJson(_)));
    }

    #[test]
    fn replies_and_events_have_the_agreed_shape() {
        let ok: Value = serde_json::from_str(&result_ok(Some(&json!(7)), json!({ "a": 1 }))).unwrap_or(Value::Null);
        assert_eq!(ok, json!({ "op": "result", "id": 7, "ok": true, "data": { "a": 1 } }));
        let failed: Value = serde_json::from_str(&result_error(None, "nope")).unwrap_or(Value::Null);
        assert_eq!(failed, json!({ "op": "result", "id": null, "ok": false, "error": "nope" }));
        let unauthorized: Value = serde_json::from_str(&error_message("unauthorized")).unwrap_or(Value::Null);
        assert_eq!(unauthorized, json!({ "op": "error", "code": "unauthorized" }));
        let auth: Value = serde_json::from_str(&auth_ok()).unwrap_or(Value::Null);
        assert_eq!(auth, json!({ "op": "auth", "ok": true }));
        let started: Value = serde_json::from_str(&event(EventKind::RecordingStarted, json!({}))).unwrap_or(Value::Null);
        assert_eq!(started, json!({ "op": "event", "event": "recordingStarted", "data": {} }));
    }

    #[test]
    fn the_status_of_an_idle_recorder_is_all_zeros() {
        let state = ControllerState { active_profile: "P".to_owned(), ..ControllerState::default() };
        let status = StatusData::from_state(&state);
        assert_eq!(status.state, "idle");
        assert_eq!((status.elapsed_ms, status.marks_count, status.duplicated, status.dropped), (0, 0, 0, 0));
        assert_eq!(status.take, None);
        assert_eq!((status.bytes, status.bitrate_kbps, status.disk_free_bytes, status.remaining_secs, status.queue_depth), (0, 0, None, None, 0));
        let mut raised = ControllerState::default();
        raised.warnings.push_back(WarningNote { seq: 1, code: "disk_low".to_owned(), message: "x".to_owned() });
        raised.warnings.push_back(WarningNote { seq: 2, code: "disk_full".to_owned(), message: "y".to_owned() });
        let sent = warning_events(&raised, 1);
        assert_eq!(sent.len(), 1);
        let wire: Value = serde_json::from_str(&sent[0].1).unwrap_or(Value::Null);
        assert_eq!(wire, json!({ "op": "event", "event": "warning", "data": { "code": "disk_full", "message": "y" } }));
        assert_eq!(sent[0].0, 2);
        assert_eq!(status.to_value()["marksCount"], json!(0));
        assert_eq!(status.to_value()["elapsedMs"], json!(0));
        assert_eq!(status.to_value()["compositeMode"], json!("baked"));
        let clean = ControllerState { composite_mode: crate::config::CompositeMode::Clean, ..ControllerState::default() };
        assert_eq!(StatusData::from_state(&clean).to_value()["compositeMode"], json!("clean"));
    }
}
