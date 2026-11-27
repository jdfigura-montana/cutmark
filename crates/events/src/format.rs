//! The bytes: chunk framing and the payload of each chunk type. Decoders return None for anything
//! malformed and never panic or allocate more than the input can justify.

use crate::types::{
    AppSwitch, CaptureSource, Chord, Counts, CursorSample, CursorShape, DisplayInfo, GeometryChange, GeometryEvent, Header, Modifiers, MouseAction,
    MouseButton, MouseEvent, ScreenRect, ShapeFrame, Streams, Tail,
};
use crate::{EventsError, SCHEMA_VERSION};

pub(crate) const HEAD: [u8; 4] = *b"HEAD";
pub(crate) const SHAP: [u8; 4] = *b"SHAP";
pub(crate) const SAMP: [u8; 4] = *b"SAMP";
pub(crate) const MOUS: [u8; 4] = *b"MOUS";
pub(crate) const KEYS: [u8; 4] = *b"KEYS";
pub(crate) const APPS: [u8; 4] = *b"APPS";
pub(crate) const GEOM: [u8; 4] = *b"GEOM";
pub(crate) const TAIL: [u8; 4] = *b"TAIL";

const MAGIC: &[u8; 5] = b"CMEVT";
const MAX_SHAPE_SIDE: u32 = 256;
const MAX_SHAPE_FRAMES: usize = 1024;
/// LZ4 cannot expand a block by more than this factor, which bounds what a decoder allocates.
const LZ4_MAX_RATIO: usize = 255;

const SAMPLE_VISIBLE: u8 = 1;
const SAMPLE_HAS_SHAPE: u8 = 2;
const SAMPLE_NEW_SHAPE: u8 = 4;
/// The most bytes one sample takes before compression: time, x, y, flags and shape id.
const SAMPLE_MAX_LEN: usize = 10 + 5 + 5 + 1 + 5;
/// The fewest: one byte each for time, x, y and flags.
const SAMPLE_MIN_LEN: usize = 4;

fn chunk_crc(kind: [u8; 4], length: [u8; 4], payload: &[u8]) -> u32 {
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(&kind);
    hasher.update(&length);
    hasher.update(payload);
    hasher.finalize()
}

/// Appends one framed chunk to `out`.
pub(crate) fn put_chunk(out: &mut Vec<u8>, kind: [u8; 4], payload: &[u8]) -> Result<(), EventsError> {
    let length = u32::try_from(payload.len()).map_err(|_| EventsError::InvalidRecord("chunk longer than 4 GiB"))?.to_le_bytes();
    out.extend_from_slice(&kind);
    out.extend_from_slice(&length);
    out.extend_from_slice(&chunk_crc(kind, length, payload).to_le_bytes());
    out.extend_from_slice(payload);
    Ok(())
}

/// Takes the next whole chunk with a matching CRC; None at the end or at a bad or partial chunk.
pub(crate) fn next_chunk<'a>(input: &mut Input<'a>) -> Option<([u8; 4], &'a [u8])> {
    let kind: [u8; 4] = input.array()?;
    let length: [u8; 4] = input.array()?;
    let crc = input.u32()?;
    let payload = input.bytes(usize::try_from(u32::from_le_bytes(length)).ok()?)?;
    (chunk_crc(kind, length, payload) == crc).then_some((kind, payload))
}

/// A cursor over bytes being decoded.
pub(crate) struct Input<'a>(pub(crate) &'a [u8]);

impl<'a> Input<'a> {
    fn bytes(&mut self, len: usize) -> Option<&'a [u8]> {
        let (head, rest) = self.0.split_at_checked(len)?;
        self.0 = rest;
        Some(head)
    }

    fn array<const N: usize>(&mut self) -> Option<[u8; N]> {
        self.bytes(N)?.try_into().ok()
    }

    fn rest(&mut self) -> &'a [u8] {
        std::mem::take(&mut self.0)
    }

    fn end(&self) -> Option<()> {
        self.0.is_empty().then_some(())
    }

    fn u8(&mut self) -> Option<u8> {
        self.array().map(u8::from_le_bytes)
    }

    fn u16(&mut self) -> Option<u16> {
        self.array().map(u16::from_le_bytes)
    }

    fn i16(&mut self) -> Option<i16> {
        self.array().map(i16::from_le_bytes)
    }

    fn u32(&mut self) -> Option<u32> {
        self.array().map(u32::from_le_bytes)
    }

    fn i32(&mut self) -> Option<i32> {
        self.array().map(i32::from_le_bytes)
    }

    fn u64(&mut self) -> Option<u64> {
        self.array().map(u64::from_le_bytes)
    }

    fn i64(&mut self) -> Option<i64> {
        self.array().map(i64::from_le_bytes)
    }

    fn varint(&mut self) -> Option<u64> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = self.u8()?;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    }

    fn string(&mut self) -> Option<String> {
        let len = usize::from(self.u16()?);
        String::from_utf8(self.bytes(len)?.to_vec()).ok()
    }

    fn rect(&mut self) -> Option<ScreenRect> {
        Some(ScreenRect { x: self.i32()?, y: self.i32()?, width: self.i32()?, height: self.i32()? })
    }

    fn displays(&mut self) -> Option<Vec<DisplayInfo>> {
        let count = self.u16()?;
        (0..count).map(|_| Some(DisplayInfo { name: self.string()?, rect: self.rect()?, dpi: self.u32()?, primary: self.u8()? != 0 })).collect()
    }

    /// A u32 record count followed by that many records, filling the rest of the payload.
    fn list<T>(&mut self, mut record: impl FnMut(&mut Self) -> Option<T>) -> Option<Vec<T>> {
        let count = self.u32()?;
        // Collecting through Option reserves nothing up front, so a false count costs nothing.
        let records = (0..count).map(|_| record(self)).collect::<Option<Vec<T>>>()?;
        self.end()?;
        Some(records)
    }
}

fn put_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push(value as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

/// Maps small values of either sign to small unsigned values.
fn zigzag(value: i64) -> u64 {
    ((value << 1) ^ (value >> 63)) as u64
}

fn unzigzag(value: u64) -> i64 {
    (value >> 1) as i64 ^ -((value & 1) as i64)
}

fn put_count(out: &mut Vec<u8>, count: usize) -> Result<(), EventsError> {
    let count = u32::try_from(count).map_err(|_| EventsError::InvalidRecord("too many records for one chunk"))?;
    out.extend_from_slice(&count.to_le_bytes());
    Ok(())
}

pub(crate) fn check_string(value: &str) -> Result<(), EventsError> {
    if value.len() > usize::from(u16::MAX) { Err(EventsError::InvalidRecord("string longer than 65535 bytes")) } else { Ok(()) }
}

pub(crate) fn check_displays(displays: &[DisplayInfo]) -> Result<(), EventsError> {
    if displays.len() > usize::from(u16::MAX) {
        return Err(EventsError::InvalidRecord("more than 65535 displays"));
    }
    displays.iter().try_for_each(|display| check_string(&display.name))
}

/// The byte length of one frame, if the size is allowed.
fn frame_len(width: u32, height: u32) -> Option<usize> {
    let allowed = 1..=MAX_SHAPE_SIDE;
    (allowed.contains(&width) && allowed.contains(&height)).then(|| width as usize * height as usize * 4)
}

pub(crate) fn check_shape(shape: &CursorShape) -> Result<(), EventsError> {
    let len = frame_len(shape.width, shape.height).ok_or(EventsError::InvalidRecord("cursor shape side outside 1 to 256"))?;
    if shape.frames.is_empty() || shape.frames.len() > MAX_SHAPE_FRAMES {
        return Err(EventsError::InvalidRecord("cursor shape frame count outside 1 to 1024"));
    }
    if shape.frames.iter().any(|frame| frame.bgra.len() != len) {
        return Err(EventsError::InvalidRecord("cursor shape pixels do not match its size"));
    }
    Ok(())
}

// Strings and display lists are length checked before they get here, so the casts below are exact.

fn put_string(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&(value.len() as u16).to_le_bytes());
    out.extend_from_slice(value.as_bytes());
}

fn put_rect(out: &mut Vec<u8>, rect: ScreenRect) {
    for value in [rect.x, rect.y, rect.width, rect.height] {
        out.extend_from_slice(&value.to_le_bytes());
    }
}

fn put_displays(out: &mut Vec<u8>, displays: &[DisplayInfo]) {
    out.extend_from_slice(&(displays.len() as u16).to_le_bytes());
    for display in displays {
        put_string(out, &display.name);
        put_rect(out, display.rect);
        out.extend_from_slice(&display.dpi.to_le_bytes());
        out.push(u8::from(display.primary));
    }
}

fn bits(flags: [bool; 4]) -> u8 {
    flags.iter().enumerate().fold(0, |byte, (index, on)| byte | (u8::from(*on) << index))
}

fn bit(byte: u8, index: u8) -> bool {
    byte & (1 << index) != 0
}

pub(crate) fn encode_head(header: &Header) -> Result<Vec<u8>, EventsError> {
    if header.qpc_frequency <= 0 {
        return Err(EventsError::InvalidRecord("QPC frequency must be above zero"));
    }
    check_displays(&header.displays)?;
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&SCHEMA_VERSION.to_le_bytes());
    out.extend_from_slice(&header.qpc_frequency.to_le_bytes());
    out.extend_from_slice(&header.first_frame_qpc.to_le_bytes());
    match header.source {
        CaptureSource::Monitor(rect) => {
            out.push(0);
            put_rect(&mut out, rect);
        }
        CaptureSource::Region(rect) => {
            out.push(1);
            put_rect(&mut out, rect);
        }
        CaptureSource::Window { handle } => {
            out.push(2);
            out.extend_from_slice(&handle.to_le_bytes());
        }
    }
    put_displays(&mut out, &header.displays);
    out.extend_from_slice(&header.cursor_size.to_le_bytes());
    let streams = header.streams;
    out.push(bits([streams.cursor, streams.clicks, streams.keys, streams.apps]));
    Ok(out)
}

pub(crate) fn decode_head(payload: &[u8]) -> Result<Header, EventsError> {
    let mut input = Input(payload);
    if input.array::<5>().as_ref() != Some(MAGIC) {
        return Err(EventsError::BadHeader);
    }
    // The version comes before anything a later schema may lay out differently.
    let version = input.u16().ok_or(EventsError::BadHeader)?;
    if version != SCHEMA_VERSION {
        return Err(EventsError::UnsupportedVersion(version));
    }
    decode_head_fields(&mut input).ok_or(EventsError::BadHeader)
}

fn decode_head_fields(input: &mut Input<'_>) -> Option<Header> {
    let qpc_frequency = input.i64()?;
    let first_frame_qpc = input.i64()?;
    let source = match input.u8()? {
        0 => CaptureSource::Monitor(input.rect()?),
        1 => CaptureSource::Region(input.rect()?),
        2 => CaptureSource::Window { handle: input.i64()? },
        _ => return None,
    };
    let displays = input.displays()?;
    let cursor_size = input.u32()?;
    let on = input.u8()?;
    input.end()?;
    let streams = Streams { cursor: bit(on, 0), clicks: bit(on, 1), keys: bit(on, 2), apps: bit(on, 3) };
    (qpc_frequency > 0).then_some(Header { qpc_frequency, first_frame_qpc, source, displays, cursor_size, streams })
}

/// The shape must have passed [`check_shape`].
pub(crate) fn encode_shape(shape: &CursorShape) -> Result<Vec<u8>, EventsError> {
    let mut out = Vec::new();
    for value in [shape.id, shape.width, shape.height, shape.hotspot_x, shape.hotspot_y] {
        out.extend_from_slice(&value.to_le_bytes());
    }
    put_count(&mut out, shape.frames.len())?;
    let mut pixels = Vec::new();
    for frame in &shape.frames {
        out.extend_from_slice(&frame.duration_us.to_le_bytes());
        pixels.extend_from_slice(&frame.bgra);
    }
    put_count(&mut out, pixels.len())?;
    out.extend_from_slice(&lz4_flex::block::compress(&pixels));
    Ok(out)
}

/// Decompresses an LZ4 block that must come to exactly `len` bytes.
fn decompress(block: &[u8], len: usize) -> Option<Vec<u8>> {
    if len > block.len().saturating_mul(LZ4_MAX_RATIO).saturating_add(16) {
        return None;
    }
    let data = lz4_flex::block::decompress(block, len).ok()?;
    (data.len() == len).then_some(data)
}

pub(crate) fn decode_shape(payload: &[u8]) -> Option<CursorShape> {
    let mut input = Input(payload);
    let (id, width, height) = (input.u32()?, input.u32()?, input.u32()?);
    let (hotspot_x, hotspot_y) = (input.u32()?, input.u32()?);
    let count = usize::try_from(input.u32()?).ok()?;
    let len = frame_len(width, height)?;
    if count == 0 || count > MAX_SHAPE_FRAMES {
        return None;
    }
    let durations = (0..count).map(|_| input.u32()).collect::<Option<Vec<_>>>()?;
    let raw_len = usize::try_from(input.u32()?).ok()?;
    if len.checked_mul(count) != Some(raw_len) {
        return None;
    }
    let pixels = decompress(input.rest(), raw_len)?;
    let frames = durations.into_iter().zip(pixels.chunks_exact(len)).map(|(duration_us, bgra)| ShapeFrame { duration_us, bgra: bgra.to_vec() }).collect();
    Some(CursorShape { id, width, height, hotspot_x, hotspot_y, frames })
}

/// Encodes samples by column. Times are stored as the change in the step between samples, which
/// stays small at a steady sampling rate, and positions as the step itself. Each chunk starts from
/// zero, so it decodes without the ones before it.
pub(crate) fn encode_samples(samples: &[CursorSample]) -> Result<Vec<u8>, EventsError> {
    let mut raw = Vec::with_capacity(samples.len() * 8);
    let (mut last_qpc, mut last_step) = (0i64, 0i64);
    for (index, sample) in samples.iter().enumerate() {
        let step = sample.qpc.wrapping_sub(last_qpc);
        put_varint(&mut raw, zigzag(step.wrapping_sub(last_step)));
        last_qpc = sample.qpc;
        // The first value is the whole stamp, not a step to compare the next one with.
        if index > 0 {
            last_step = step;
        }
    }
    for axis in [|s: &CursorSample| s.x, |s: &CursorSample| s.y] {
        let mut last = 0i32;
        for sample in samples {
            put_varint(&mut raw, zigzag(i64::from(axis(sample).wrapping_sub(last))));
            last = axis(sample);
        }
    }
    let mut new_shapes = Vec::new();
    let mut last_shape = None;
    for sample in samples {
        let mut flags = if sample.visible { SAMPLE_VISIBLE } else { 0 };
        if let Some(id) = sample.shape {
            flags |= SAMPLE_HAS_SHAPE;
            if last_shape != Some(id) {
                flags |= SAMPLE_NEW_SHAPE;
                put_varint(&mut new_shapes, u64::from(id));
                last_shape = Some(id);
            }
        }
        raw.push(flags);
    }
    raw.extend_from_slice(&new_shapes);

    let mut out = Vec::new();
    put_count(&mut out, samples.len())?;
    put_count(&mut out, raw.len())?;
    out.extend_from_slice(&lz4_flex::block::compress(&raw));
    Ok(out)
}

pub(crate) fn decode_samples(payload: &[u8]) -> Option<Vec<CursorSample>> {
    let mut input = Input(payload);
    let count = usize::try_from(input.u32()?).ok()?;
    let raw_len = usize::try_from(input.u32()?).ok()?;
    if raw_len < count.checked_mul(SAMPLE_MIN_LEN)? || raw_len > count.checked_mul(SAMPLE_MAX_LEN)? {
        return None;
    }
    let raw = decompress(input.rest(), raw_len)?;
    let mut raw = Input(&raw);
    let mut samples = vec![CursorSample { qpc: 0, x: 0, y: 0, shape: None, visible: false }; count];

    let (mut last_qpc, mut last_step) = (0i64, 0i64);
    for (index, sample) in samples.iter_mut().enumerate() {
        let step = last_step.wrapping_add(unzigzag(raw.varint()?));
        last_qpc = last_qpc.wrapping_add(step);
        sample.qpc = last_qpc;
        if index > 0 {
            last_step = step;
        }
    }
    let mut last = 0i32;
    for sample in &mut samples {
        last = last.wrapping_add(i32::try_from(unzigzag(raw.varint()?)).ok()?);
        sample.x = last;
    }
    last = 0;
    for sample in &mut samples {
        last = last.wrapping_add(i32::try_from(unzigzag(raw.varint()?)).ok()?);
        sample.y = last;
    }
    let flags = raw.bytes(count)?;
    let mut last_shape = None;
    for (sample, flags) in samples.iter_mut().zip(flags) {
        sample.visible = flags & SAMPLE_VISIBLE != 0;
        if flags & SAMPLE_NEW_SHAPE != 0 {
            last_shape = Some(u32::try_from(raw.varint()?).ok()?);
        }
        if flags & SAMPLE_HAS_SHAPE != 0 {
            sample.shape = Some(last_shape?);
        }
    }
    raw.end()?;
    Some(samples)
}

const BUTTONS: [MouseButton; 5] = [MouseButton::Left, MouseButton::Right, MouseButton::Middle, MouseButton::X1, MouseButton::X2];

fn button_code(button: MouseButton) -> i16 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Right => 1,
        MouseButton::Middle => 2,
        MouseButton::X1 => 3,
        MouseButton::X2 => 4,
    }
}

pub(crate) fn encode_mouse(events: &[MouseEvent]) -> Result<Vec<u8>, EventsError> {
    let mut out = Vec::new();
    put_count(&mut out, events.len())?;
    for event in events {
        out.extend_from_slice(&event.qpc.to_le_bytes());
        out.extend_from_slice(&event.x.to_le_bytes());
        out.extend_from_slice(&event.y.to_le_bytes());
        let (kind, value) = match event.action {
            MouseAction::Down(button) => (0u8, button_code(button)),
            MouseAction::Up(button) => (1, button_code(button)),
            MouseAction::Wheel(delta) => (2, delta),
            MouseAction::HorizontalWheel(delta) => (3, delta),
        };
        out.push(kind);
        out.extend_from_slice(&value.to_le_bytes());
    }
    Ok(out)
}

pub(crate) fn decode_mouse(payload: &[u8]) -> Option<Vec<MouseEvent>> {
    Input(payload).list(|input| {
        let (qpc, x, y) = (input.i64()?, input.i32()?, input.i32()?);
        let (kind, value) = (input.u8()?, input.i16()?);
        let button = || BUTTONS.get(usize::try_from(value).ok()?).copied();
        let action = match kind {
            0 => MouseAction::Down(button()?),
            1 => MouseAction::Up(button()?),
            2 => MouseAction::Wheel(value),
            3 => MouseAction::HorizontalWheel(value),
            _ => return None,
        };
        Some(MouseEvent { qpc, x, y, action })
    })
}

pub(crate) fn encode_chords(chords: &[Chord]) -> Result<Vec<u8>, EventsError> {
    let mut out = Vec::new();
    put_count(&mut out, chords.len())?;
    for chord in chords {
        out.extend_from_slice(&chord.qpc.to_le_bytes());
        let held = chord.modifiers;
        out.push(bits([held.ctrl, held.alt, held.shift, held.win]));
        put_string(&mut out, &chord.key_name);
    }
    Ok(out)
}

pub(crate) fn decode_chords(payload: &[u8]) -> Option<Vec<Chord>> {
    Input(payload).list(|input| {
        let (qpc, held) = (input.i64()?, input.u8()?);
        let modifiers = Modifiers { ctrl: bit(held, 0), alt: bit(held, 1), shift: bit(held, 2), win: bit(held, 3) };
        Some(Chord { qpc, modifiers, key_name: input.string()? })
    })
}

pub(crate) fn encode_apps(apps: &[AppSwitch]) -> Result<Vec<u8>, EventsError> {
    let mut out = Vec::new();
    put_count(&mut out, apps.len())?;
    for app in apps {
        out.extend_from_slice(&app.qpc.to_le_bytes());
        put_string(&mut out, &app.exe);
        put_string(&mut out, &app.display_name);
    }
    Ok(out)
}

pub(crate) fn decode_apps(payload: &[u8]) -> Option<Vec<AppSwitch>> {
    Input(payload).list(|input| Some(AppSwitch { qpc: input.i64()?, exe: input.string()?, display_name: input.string()? }))
}

pub(crate) fn encode_geometry(events: &[GeometryEvent]) -> Result<Vec<u8>, EventsError> {
    let mut out = Vec::new();
    put_count(&mut out, events.len())?;
    for event in events {
        out.extend_from_slice(&event.qpc.to_le_bytes());
        match &event.change {
            GeometryChange::WindowRect(rect) => {
                out.push(0);
                put_rect(&mut out, *rect);
            }
            GeometryChange::Displays(displays) => {
                out.push(1);
                put_displays(&mut out, displays);
            }
        }
    }
    Ok(out)
}

pub(crate) fn decode_geometry(payload: &[u8]) -> Option<Vec<GeometryEvent>> {
    Input(payload).list(|input| {
        let qpc = input.i64()?;
        let change = match input.u8()? {
            0 => GeometryChange::WindowRect(input.rect()?),
            1 => GeometryChange::Displays(input.displays()?),
            _ => return None,
        };
        Some(GeometryEvent { qpc, change })
    })
}

pub(crate) fn encode_tail(tail: &Tail) -> Vec<u8> {
    let counts = tail.counts;
    let mut out = tail.end_qpc.to_le_bytes().to_vec();
    for count in [counts.shapes, counts.samples, counts.mouse, counts.chords, counts.apps, counts.geometry] {
        out.extend_from_slice(&count.to_le_bytes());
    }
    out
}

pub(crate) fn decode_tail(payload: &[u8]) -> Option<Tail> {
    let mut input = Input(payload);
    let end_qpc = input.i64()?;
    let counts = Counts { shapes: input.u64()?, samples: input.u64()?, mouse: input.u64()?, chords: input.u64()?, apps: input.u64()?, geometry: input.u64()? };
    input.end()?;
    Some(Tail { end_qpc, counts })
}
