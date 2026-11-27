//! Reading an events file, whole or cut short.

use std::collections::BTreeMap;
use std::path::Path;

use crate::EventsError;
use crate::format::{self, Input};
use crate::types::{AppSwitch, Chord, CursorSample, CursorShape, GeometryEvent, Header, MouseAction, MouseEvent, Tail};

/// A record with its time on the video's clock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Timed<T> {
    /// Microseconds from the first video frame; negative before it.
    pub time_us: i64,
    /// The record as written, with its QPC stamp.
    pub event: T,
}

/// What a renderer needs to draw the cursor at one instant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorState<'a> {
    /// Hotspot position in physical virtual desktop pixels, horizontal.
    pub x: i32,
    /// Hotspot position, vertical.
    pub y: i32,
    /// Whether Windows was showing the cursor.
    pub visible: bool,
    /// The shape in use; None while hidden or when it could not be read.
    pub shape: Option<&'a CursorShape>,
}

/// The contents of an events file. Every list is in time order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventsTrack {
    header: Header,
    shapes: BTreeMap<u32, CursorShape>,
    samples: Vec<Timed<CursorSample>>,
    mouse: Vec<Timed<MouseEvent>>,
    chords: Vec<Timed<Chord>>,
    apps: Vec<Timed<AppSwitch>>,
    geometry: Vec<Timed<GeometryEvent>>,
    tail: Option<Tail>,
}

fn timed<T>(header: &Header, list: &mut Vec<Timed<T>>, events: Vec<T>, qpc: impl Fn(&T) -> i64) {
    list.extend(events.into_iter().map(|event| Timed { time_us: header.micros(qpc(&event)), event }));
}

impl EventsTrack {
    /// Reads a file. See [`from_bytes`](Self::from_bytes).
    pub fn open(path: &Path) -> Result<Self, EventsError> {
        Self::from_bytes(&std::fs::read(path)?)
    }

    /// Reads the contents of a file. Only a missing or damaged header, or another schema version, is
    /// an error. Reading stops at the first bad or partial chunk and keeps everything before it;
    /// chunks of unknown types are skipped.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, EventsError> {
        let mut input = Input(bytes);
        let header = match format::next_chunk(&mut input) {
            Some((format::HEAD, payload)) => format::decode_head(payload)?,
            _ => return Err(EventsError::BadHeader),
        };
        let mut track = Self {
            header,
            shapes: BTreeMap::new(),
            samples: Vec::new(),
            mouse: Vec::new(),
            chords: Vec::new(),
            apps: Vec::new(),
            geometry: Vec::new(),
            tail: None,
        };
        while track.tail.is_none() {
            let Some((kind, payload)) = format::next_chunk(&mut input) else { break };
            if track.add_chunk(kind, payload).is_none() {
                break;
            }
        }
        // The writer keeps each list in order; lookups by time rely on it.
        track.samples.sort_by_key(|sample| sample.time_us);
        track.mouse.sort_by_key(|mouse| mouse.time_us);
        track.chords.sort_by_key(|chord| chord.time_us);
        track.apps.sort_by_key(|app| app.time_us);
        track.geometry.sort_by_key(|geometry| geometry.time_us);
        Ok(track)
    }

    /// Adds a chunk's records, all or none; None means the chunk is bad.
    fn add_chunk(&mut self, kind: [u8; 4], payload: &[u8]) -> Option<()> {
        let header = &self.header;
        match kind {
            format::SHAP => {
                let shape = format::decode_shape(payload)?;
                self.shapes.insert(shape.id, shape);
            }
            format::SAMP => timed(header, &mut self.samples, format::decode_samples(payload)?, |e| e.qpc),
            format::MOUS => timed(header, &mut self.mouse, format::decode_mouse(payload)?, |e| e.qpc),
            format::KEYS => timed(header, &mut self.chords, format::decode_chords(payload)?, |e| e.qpc),
            format::APPS => timed(header, &mut self.apps, format::decode_apps(payload)?, |e| e.qpc),
            format::GEOM => timed(header, &mut self.geometry, format::decode_geometry(payload)?, |e| e.qpc),
            format::TAIL => self.tail = Some(format::decode_tail(payload)?),
            // A file has one header.
            format::HEAD => return None,
            _ => {}
        }
        Some(())
    }

    /// The file's header. [`Header::micros`] converts any QPC stamp.
    pub fn header(&self) -> &Header {
        &self.header
    }

    /// The cursor at a time: the latest sample at or before it. None before the first sample.
    pub fn cursor_at(&self, time_us: i64) -> Option<CursorState<'_>> {
        let after = self.samples.partition_point(|sample| sample.time_us <= time_us);
        let sample = &self.samples.get(after.checked_sub(1)?)?.event;
        let shape = sample.shape.and_then(|id| self.shapes.get(&id));
        Some(CursorState { x: sample.x, y: sample.y, visible: sample.visible, shape })
    }

    /// The shape with an id.
    pub fn shape(&self, id: u32) -> Option<&CursorShape> {
        self.shapes.get(&id)
    }

    /// Every shape, in id order.
    pub fn shapes(&self) -> impl Iterator<Item = &CursorShape> {
        self.shapes.values()
    }

    /// The cursor samples.
    pub fn samples(&self) -> &[Timed<CursorSample>] {
        &self.samples
    }

    /// Every button and wheel event.
    pub fn mouse(&self) -> &[Timed<MouseEvent>] {
        &self.mouse
    }

    /// The clicks: every button going down.
    pub fn clicks(&self) -> impl Iterator<Item = &Timed<MouseEvent>> {
        self.mouse.iter().filter(|mouse| matches!(mouse.event.action, MouseAction::Down(_)))
    }

    /// The keyboard shortcuts; empty when the stream was off.
    pub fn chords(&self) -> &[Timed<Chord>] {
        &self.chords
    }

    /// The foreground app changes.
    pub fn apps(&self) -> &[Timed<AppSwitch>] {
        &self.apps
    }

    /// Window rectangle and display layout changes.
    pub fn geometry(&self) -> &[Timed<GeometryEvent>] {
        &self.geometry
    }

    /// The `TAIL` chunk, if it was reached.
    pub fn tail(&self) -> Option<&Tail> {
        self.tail.as_ref()
    }

    /// Whether `TAIL` was found. False means the input track is incomplete.
    pub fn is_complete(&self) -> bool {
        self.tail.is_some()
    }
}
