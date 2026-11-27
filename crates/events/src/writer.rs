//! Writing an events file, one batch of chunks at a time.

use std::fs::File;
use std::io::Write;
use std::path::Path;

use crate::EventsError;
use crate::format::{self, put_chunk};
use crate::types::{AppSwitch, Chord, Counts, CursorSample, CursorShape, GeometryEvent, Header, InputEvent, MouseEvent, Streams, Tail};

/// Events pushed since the last flush.
#[derive(Default)]
struct Pending {
    shapes: Vec<CursorShape>,
    samples: Vec<CursorSample>,
    mouse: Vec<MouseEvent>,
    chords: Vec<Chord>,
    apps: Vec<AppSwitch>,
    geometry: Vec<GeometryEvent>,
}

impl Pending {
    /// The chunks for what is pending; a kind with nothing pending gets no chunk. Shapes come first
    /// so that a file cut short never holds a sample without its shape.
    fn encode(&self) -> Result<Vec<u8>, EventsError> {
        let mut out = Vec::new();
        for shape in &self.shapes {
            put_chunk(&mut out, format::SHAP, &format::encode_shape(shape)?)?;
        }
        if !self.samples.is_empty() {
            put_chunk(&mut out, format::SAMP, &format::encode_samples(&self.samples)?)?;
        }
        if !self.mouse.is_empty() {
            put_chunk(&mut out, format::MOUS, &format::encode_mouse(&self.mouse)?)?;
        }
        if !self.chords.is_empty() {
            put_chunk(&mut out, format::KEYS, &format::encode_chords(&self.chords)?)?;
        }
        if !self.apps.is_empty() {
            put_chunk(&mut out, format::APPS, &format::encode_apps(&self.apps)?)?;
        }
        if !self.geometry.is_empty() {
            put_chunk(&mut out, format::GEOM, &format::encode_geometry(&self.geometry)?)?;
        }
        Ok(out)
    }

    fn add_to(&self, counts: &mut Counts) {
        counts.shapes += self.shapes.len() as u64;
        counts.samples += self.samples.len() as u64;
        counts.mouse += self.mouse.len() as u64;
        counts.chords += self.chords.len() as u64;
        counts.apps += self.apps.len() as u64;
        counts.geometry += self.geometry.len() as u64;
    }
}

/// Writes one events file. Call [`flush_second`](Self::flush_second) once a second, and
/// [`finish`](Self::finish) when the take ends. A writer dropped without `finish` leaves a file
/// that reads as incomplete.
pub struct EventsWriter<W: Write> {
    sink: W,
    streams: Streams,
    pending: Pending,
    counts: Counts,
    broken: bool,
}

impl EventsWriter<File> {
    /// Creates the file and writes its header. Fails if the file already exists.
    pub fn create(path: &Path, header: &Header) -> Result<Self, EventsError> {
        // Checked before the file exists, so a bad header leaves nothing behind.
        format::encode_head(header)?;
        Self::new(File::create_new(path)?, header)
    }
}

impl<W: Write> EventsWriter<W> {
    /// Writes the header to `sink`.
    pub fn new(sink: W, header: &Header) -> Result<Self, EventsError> {
        let mut out = Vec::new();
        put_chunk(&mut out, format::HEAD, &format::encode_head(header)?)?;
        let mut writer = Self { sink, streams: header.streams, pending: Pending::default(), counts: Counts::default(), broken: false };
        writer.write(&out)?;
        Ok(writer)
    }

    /// Queues an event for the next flush. An event of a stream the header has off is dropped, so a
    /// take without keyboard shortcuts can never hold one.
    pub fn push(&mut self, event: InputEvent) -> Result<(), EventsError> {
        if self.broken {
            return Err(EventsError::Broken);
        }
        let streams = self.streams;
        match event {
            InputEvent::Sample(sample) if streams.cursor => self.pending.samples.push(sample),
            InputEvent::Shape(shape) if streams.cursor => {
                format::check_shape(&shape)?;
                self.pending.shapes.push(shape);
            }
            InputEvent::Mouse(mouse) if streams.clicks => self.pending.mouse.push(mouse),
            InputEvent::Chord(chord) if streams.keys => {
                format::check_string(&chord.key_name)?;
                self.pending.chords.push(chord);
            }
            InputEvent::App(app) if streams.apps => {
                format::check_string(&app.exe)?;
                format::check_string(&app.display_name)?;
                self.pending.apps.push(app);
            }
            InputEvent::Geometry(geometry) => {
                if let crate::GeometryChange::Displays(displays) = &geometry.change {
                    format::check_displays(displays)?;
                }
                self.pending.geometry.push(geometry);
            }
            InputEvent::Sample(_) | InputEvent::Shape(_) | InputEvent::Mouse(_) | InputEvent::Chord(_) | InputEvent::App(_) => {}
        }
        Ok(())
    }

    /// Writes the pending chunks and flushes the sink. After this, they survive the process dying.
    pub fn flush_second(&mut self) -> Result<(), EventsError> {
        if self.broken {
            return Err(EventsError::Broken);
        }
        let pending = std::mem::take(&mut self.pending);
        let out = pending.encode()?;
        if !out.is_empty() {
            self.write(&out)?;
            pending.add_to(&mut self.counts);
        }
        Ok(())
    }

    /// Writes what is pending, then `TAIL`, and returns the sink. `end_qpc` is when the take ended.
    pub fn finish(mut self, end_qpc: i64) -> Result<W, EventsError> {
        self.flush_second()?;
        let mut out = Vec::new();
        put_chunk(&mut out, format::TAIL, &format::encode_tail(&Tail { end_qpc, counts: self.counts }))?;
        self.write(&out)?;
        Ok(self.sink)
    }

    /// What has been written so far.
    pub fn counts(&self) -> Counts {
        self.counts
    }

    /// The sink being written to.
    pub fn get_ref(&self) -> &W {
        &self.sink
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), EventsError> {
        let result = self.sink.write_all(bytes).and_then(|()| self.sink.flush());
        self.broken |= result.is_err();
        Ok(result?)
    }
}
