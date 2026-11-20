//! The marks of one take: the sidecar writer, the Markdown and CSV logs and the MKV chapters.
//!
//! A marks error never stops a recording. It is logged and left in a [`MarksWarning`] for the controller to show.

use std::collections::VecDeque;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use ::marks::{AutoInfo, Mark, MarkType, NewMark, ScriptRef, SessionWriter, Sidecar, Source, SystemClock, WriterInfo};

use super::clock::MediaClock;

/// How far back a highlight reaches, in milliseconds.
const HIGHLIGHT_SPAN_MS: u64 = 30_000;

/// How many recent marks the take keeps for the window.
const RECENT_LIMIT: usize = 10;

/// How much of a WAV file is read to find its format and data chunks.
const WAV_HEAD_BYTES: u64 = 4096;

/// A mark as the window lists it: timecode, category and label.
pub type RecentMark = (String, String, Option<String>);

/// A chapter for the muxer: start in ms, end in ms and title.
pub type MkvChapter = (u64, u64, String);

/// The text of the last marks error, shared between the take and the controller.
#[derive(Debug, Clone, Default)]
pub struct MarksWarning(Arc<Mutex<Option<String>>>);

impl MarksWarning {
    /// Keeps the text of an error for the controller to show.
    pub fn set(&self, text: String) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(text);
    }

    /// Takes the stored text, leaving none behind.
    pub fn take(&self) -> Option<String> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).take()
    }
}

/// The marks of one running take, timed from the take's media clock.
pub struct TakeMarks {
    writer: Option<SessionWriter>,
    clock: MediaClock,
    fps: u32,
    dir: PathBuf,
    base_name: String,
    count: usize,
    recent: VecDeque<RecentMark>,
    warning: MarksWarning,
    /// The script line being read, stamped on every mark made while it is known.
    script: Option<ScriptRef>,
}

impl TakeMarks {
    /// Starts the marks of a take whose video file is `video_file`, written next to it as `base_name`.
    ///
    /// When the sidecar cannot be started the take carries on without marks and `warning` says why.
    pub fn begin(dir: &Path, base_name: &str, video_file: &Path, fps: u32, clock: MediaClock, warning: MarksWarning) -> Self {
        let info = WriterInfo { app: "cutmark-recorder".to_owned(), version: env!("CARGO_PKG_VERSION").to_owned() };
        let writer = match SessionWriter::begin(dir, base_name, info, video_file, Box::new(SystemClock)) {
            Ok(writer) => Some(writer),
            Err(err) => {
                log::error!("the marks file could not be started: {err}");
                warning.set(format!("Marks are not being saved for this take: {err}"));
                None
            }
        };
        Self {
            writer,
            clock,
            fps: fps.max(1),
            dir: dir.to_path_buf(),
            base_name: base_name.to_owned(),
            count: 0,
            recent: VecDeque::new(),
            warning,
            script: None,
        }
    }

    /// Sets the script line that marks made from now on carry, as its line number and its label.
    pub fn set_script(&mut self, line: u64, label: String) {
        self.script = Some(ScriptRef { line: label, position: line });
    }

    /// The media time of a moment in ms on the take's clock, not snapped to a frame.
    pub fn clock_ms(&self, qpc: i64) -> u64 {
        self.clock.media_ms(qpc)
    }

    /// How many marks the take has so far.
    pub fn count(&self) -> usize {
        self.count
    }

    /// The last 10 marks, oldest first.
    pub fn recent(&self) -> &VecDeque<RecentMark> {
        &self.recent
    }

    /// Adds a Note with no label at the moment of the press.
    pub fn note(&mut self, press_qpc: i64) {
        let t_ms = self.media_ms(press_qpc);
        self.add(NewMark { media: Some(0), ..NewMark::point(MarkType::Note, t_ms, Source::Hotkey) });
    }

    /// Adds a point mark of `category` with an optional label at the moment of the press, stored with `source`.
    pub fn labeled(&mut self, press_qpc: i64, category: MarkType, label: Option<String>, source: Source) {
        let t_ms = self.media_ms(press_qpc);
        let mut mark = NewMark { media: Some(0), ..NewMark::point(category, t_ms, source) };
        mark.label = label;
        self.add(mark);
    }

    /// Adds a Highlight that ends at the press and starts 30 seconds earlier, or at 0.
    pub fn highlight_last(&mut self, press_qpc: i64) {
        let end_ms = self.media_ms(press_qpc);
        let start_ms = ::marks::snap_to_frame(end_ms.saturating_sub(HIGHLIGHT_SPAN_MS), self.fps, 1);
        self.add(NewMark { media: Some(0), end_ms: Some(end_ms), ..NewMark::point(MarkType::Highlight, start_ms, Source::Hotkey) });
    }

    /// Pauses the take's clock and adds a System mark named `recorder.pause`.
    pub fn pause(&mut self, qpc: i64) {
        self.clock.pause(qpc);
        let t_ms = self.media_ms(qpc);
        self.add_system("recorder.pause", "Paused", t_ms);
    }

    /// Resumes the take's clock and adds a System mark named `recorder.resume`.
    pub fn resume(&mut self, qpc: i64) {
        self.clock.resume(qpc);
        let t_ms = self.media_ms(qpc);
        self.add_system("recorder.resume", "Resumed", t_ms);
    }

    /// Adds a System mark with an automatic kind such as `recorder.device_lost` and a label, at the moment `qpc`.
    pub fn system(&mut self, qpc: i64, kind: &str, label: &str) {
        let t_ms = self.media_ms(qpc);
        self.add_system(kind, label, t_ms);
    }

    /// Ends the take's marks: writes the sidecar with the Markdown and CSV logs, and returns the chapters for the MKV.
    ///
    /// `wav_files` are the WAV sidecars of the take. `durations_ms` holds the video's length, then each WAV's length.
    pub fn finish(mut self, wav_files: &[PathBuf], durations_ms: &[u64]) -> Vec<MkvChapter> {
        let Some(mut writer) = self.writer.take() else { return Vec::new() };
        let video_ms = durations_ms.first().copied().unwrap_or(0);
        let mut durations = vec![video_ms];
        for (wav, length) in wav_files.iter().zip(durations_ms.iter().skip(1)) {
            match writer.add_media(wav, 0) {
                Ok(_) => durations.push(*length),
                Err(err) => self.fail(&format!("a WAV file could not be listed in the marks: {err}")),
            }
        }
        let path = match writer.finish(&durations) {
            Ok(path) => path,
            Err(err) => {
                self.fail(&format!("the marks file could not be written: {err}"));
                return Vec::new();
            }
        };
        let sidecar = match ::marks::load(&path) {
            Ok((sidecar, _warnings)) => sidecar,
            Err(err) => {
                self.fail(&format!("the marks file could not be read back: {err}"));
                return Vec::new();
            }
        };
        let markdown = ::marks::export::markdown_with(&sidecar, &::marks::export::ExportOptions { include_system: true });
        if let Err(err) = write_atomic(&self.dir.join(format!("{}_marks.md", self.base_name)), markdown.as_bytes()) {
            self.fail(&format!("the Markdown marks log could not be written: {err}"));
        }
        let csv = ::marks::csv(&sidecar);
        if let Err(err) = write_atomic(&self.dir.join(format!("{}_marks.csv", self.base_name)), &csv) {
            self.fail(&format!("the CSV marks log could not be written: {err}"));
        }
        mkv_chapters(&sidecar)
    }

    /// The media time of a press in ms, snapped to the take's frame rate.
    fn media_ms(&self, press_qpc: i64) -> u64 {
        ::marks::snap_to_frame(self.clock.media_ms(press_qpc), self.fps, 1)
    }

    /// Adds a System mark with the given automatic kind and label.
    fn add_system(&mut self, kind: &str, label: &str, t_ms: u64) {
        let mut mark = NewMark::point(MarkType::System, t_ms, Source::Auto);
        mark.media = Some(0);
        mark.label = Some(label.to_owned());
        mark.auto = Some(AutoInfo { kind: kind.to_owned(), data: Default::default() });
        self.add(mark);
    }

    /// Writes one mark, and keeps it for the window's list.
    fn add(&mut self, mut mark: NewMark) {
        let Some(writer) = &mut self.writer else { return };
        if mark.script.is_none() {
            mark.script.clone_from(&self.script);
        }
        let (t_ms, category, label) = (mark.t_ms, category_name(mark.mark_type), mark.label.clone());
        match writer.add_mark(mark) {
            Ok(Some(_)) => {
                self.count += 1;
                if self.recent.len() == RECENT_LIMIT {
                    self.recent.pop_front();
                }
                self.recent.push_back((::marks::format_tc(t_ms), category.to_owned(), label));
            }
            Ok(None) => log::info!("a repeated {category} key press was not made a second mark"),
            Err(err) => self.fail(&format!("a mark could not be written: {err}")),
        }
    }

    /// Logs a marks error and leaves it for the controller's banner.
    fn fail(&self, text: &str) {
        log::error!("marks: {text}");
        self.warning.set(format!("Marks problem: {text}"));
    }
}

/// The length of a WAV file in ms, read from its format and data chunks; `None` when it cannot be read.
///
/// A data chunk with no size yet, or one that runs past the end of the file, counts up to the end of the file.
pub fn wav_length_ms(path: &Path) -> Option<u64> {
    let file_len = fs::metadata(path).ok()?.len();
    let mut head = Vec::new();
    fs::File::open(path).ok()?.take(WAV_HEAD_BYTES).read_to_end(&mut head).ok()?;
    if head.get(..4)? != b"RIFF" || head.get(8..12)? != b"WAVE" {
        return None;
    }
    let mut pos = 12usize;
    let mut byte_rate = None;
    loop {
        let id = head.get(pos..pos + 4)?;
        let size = u64::from(u32::from_le_bytes(head.get(pos + 4..pos + 8)?.try_into().ok()?));
        let body = pos + 8;
        if id == b"fmt " {
            byte_rate = Some(u64::from(u32::from_le_bytes(head.get(body + 8..body + 12)?.try_into().ok()?)));
        } else if id == b"data" {
            let available = file_len.saturating_sub(body as u64);
            let data = if size == 0 || size > available { available } else { size };
            let rate = byte_rate.filter(|rate| *rate > 0)?;
            return Some(data.saturating_mul(1000) / rate);
        }
        pos = body.checked_add(usize::try_from(size + (size & 1)).ok()?)?;
    }
}

/// The name of a mark's category as people read it.
fn category_name(mark_type: MarkType) -> &'static str {
    match mark_type {
        MarkType::Highlight => "Highlight",
        MarkType::Cut => "Cut",
        MarkType::Retake => "Retake",
        MarkType::Chapter => "Chapter",
        MarkType::Note => "Note",
        MarkType::BRoll => "B-roll",
        MarkType::System => "System",
    }
}

/// The chapters of the video file: one per user mark, titled "Category: label" or just the category.
///
/// System marks are left out. A range ends where it ends; a point lasts until the next mark or the end of the video.
fn mkv_chapters(sidecar: &Sidecar) -> Vec<MkvChapter> {
    let mut marks: Vec<&Mark> = sidecar.marks.iter().filter(|mark| mark.media == 0 && mark.mark_type != MarkType::System).collect();
    marks.sort_by_key(|mark| mark.t_ms);
    let duration_ms = sidecar.media.first().map_or(0, |entry| entry.duration_ms);
    marks
        .iter()
        .enumerate()
        .map(|(position, mark)| {
            let next = marks.get(position + 1).map_or(duration_ms, |following| following.t_ms);
            let end_ms = mark.end_ms.unwrap_or(next).max(mark.t_ms);
            let category = category_name(mark.mark_type);
            let title = match mark.label.as_deref() {
                Some(label) if !label.is_empty() => format!("{category}: {label}"),
                _ => category.to_owned(),
            };
            (mark.t_ms, end_ms, title)
        })
        .collect()
}

/// Writes a file through a temp file in the same folder, then renames it over the target.
pub(super) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    fs::write(&temp, bytes)?;
    fs::rename(&temp, path).inspect_err(|_| {
        let _ = fs::remove_file(&temp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FREQ: i64 = 10_000_000;
    const START: i64 = 1_000_000_000;

    fn at_seconds(seconds: i64) -> i64 {
        START + seconds * FREQ
    }

    fn scratch(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("cutmark_marks_{name}_{}_{nanos}", std::process::id()));
        fs::create_dir_all(&dir).expect("scratch folder");
        dir
    }

    #[test]
    fn a_take_writes_its_sidecar_logs_and_chapters() {
        let dir = scratch("take");
        let video = dir.join("take.mkv");
        let warning = MarksWarning::default();
        let mut take = TakeMarks::begin(&dir, "take", &video, 60, MediaClock::new(START, FREQ), warning.clone());
        take.note(at_seconds(2));
        take.pause(at_seconds(4));
        take.resume(at_seconds(9));
        take.highlight_last(at_seconds(40));
        assert_eq!(take.count(), 4);
        assert_eq!(take.recent().back().map(|mark| mark.1.as_str()), Some("Highlight"));
        let chapters = take.finish(&[], &[35_000]);
        assert_eq!(warning.take(), None);
        // The note and the highlight, which starts at 5 s; the pause and resume System marks are not chapters.
        assert_eq!(chapters.len(), 2);
        assert!(chapters.iter().any(|chapter| chapter.0 == 2000 && chapter.2 == "Note"));
        assert!(chapters.iter().any(|chapter| chapter.0 == 5000 && chapter.1 == 35_000 && chapter.2 == "Highlight"));
        assert!(dir.join("take.marks.json").exists());
        let md = fs::read_to_string(dir.join("take_marks.md")).expect("markdown log");
        assert!(md.contains("| System | Paused |") && md.contains("| System | Resumed |"), "{md}");
        let csv = fs::read_to_string(dir.join("take_marks.csv")).expect("csv log");
        assert!(csv.contains(",system,Paused,") && csv.contains(",system,Resumed,"), "{csv}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_popup_mark_is_a_point_at_the_press_even_for_a_highlight() {
        let dir = scratch("popup");
        let mut take = TakeMarks::begin(&dir, "popup", &dir.join("popup.mkv"), 60, MediaClock::new(START, FREQ), MarksWarning::default());
        take.labeled(at_seconds(3), MarkType::Cut, Some("oops".to_owned()), Source::Hotkey);
        take.labeled(at_seconds(5), MarkType::Highlight, None, Source::Hotkey);
        let chapters = take.finish(&[], &[10_000]);
        assert_eq!(chapters, vec![(3000, 5000, "Cut: oops".to_owned()), (5000, 10_000, "Highlight".to_owned())]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_api_mark_is_stored_with_source_api_and_its_category_and_label() {
        let dir = scratch("api");
        let mut take = TakeMarks::begin(&dir, "api", &dir.join("api.mkv"), 60, MediaClock::new(START, FREQ), MarksWarning::default());
        take.labeled(at_seconds(4), MarkType::Retake, Some("redo intro".to_owned()), Source::Api);
        take.labeled(at_seconds(6), MarkType::Note, None, Source::Hotkey);
        take.finish(&[], &[10_000]);
        let (sidecar, _) = ::marks::load(&dir.join("api.marks.json")).expect("load");
        let api: Vec<_> = sidecar.marks.iter().filter(|mark| mark.source == Source::Api).collect();
        assert_eq!(api.len(), 1);
        assert_eq!((api[0].mark_type, api[0].label.as_deref()), (MarkType::Retake, Some("redo intro")));
        assert!(sidecar.marks.iter().any(|mark| mark.source == Source::Hotkey && mark.mark_type == MarkType::Note));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn marks_made_while_a_script_line_is_known_carry_it_and_the_logs_gain_a_script_column() {
        let dir = scratch("script");
        let mut take = TakeMarks::begin(&dir, "script", &dir.join("script.mkv"), 60, MediaClock::new(START, FREQ), MarksWarning::default());
        take.note(at_seconds(1));
        take.set_script(8, "L8: Now run the build".to_owned());
        take.labeled(at_seconds(4), MarkType::Retake, Some("flub".to_owned()), Source::Api);
        take.pause(at_seconds(5));
        take.finish(&[], &[10_000]);
        let (sidecar, _) = ::marks::load(&dir.join("script.marks.json")).expect("load");
        let scripts: Vec<(Source, Option<&str>)> = sidecar.marks.iter().map(|mark| (mark.source, mark.script.as_ref().map(|s| s.line.as_str()))).collect();
        assert_eq!(scripts, vec![(Source::Hotkey, None), (Source::Api, Some("L8: Now run the build")), (Source::Auto, Some("L8: Now run the build"))]);
        assert_eq!(sidecar.marks[1].script.as_ref().map(|s| s.position), Some(8));
        let csv = fs::read_to_string(dir.join("script_marks.csv")).expect("csv log");
        assert!(csv.lines().next().is_some_and(|header| header.ends_with(",script")), "{csv}");
        assert!(csv.contains(",api,L8: Now run the build"), "{csv}");
        let md = fs::read_to_string(dir.join("script_marks.md")).expect("markdown log");
        assert!(md.contains("| Script |") && md.contains("| L8: Now run the build |"), "{md}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_highlight_in_the_first_seconds_starts_at_zero() {
        let dir = scratch("early");
        let mut take = TakeMarks::begin(&dir, "early", &dir.join("early.mkv"), 30, MediaClock::new(START, FREQ), MarksWarning::default());
        take.highlight_last(at_seconds(10));
        let chapters = take.finish(&[], &[12_000]);
        assert_eq!(chapters, vec![(0, 10_000, "Highlight".to_owned())]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_labelled_mark_is_titled_with_its_category() {
        let sidecar_dir = scratch("title");
        let mut writer = SessionWriter::begin(
            &sidecar_dir,
            "t",
            WriterInfo { app: "test".to_owned(), version: "0".to_owned() },
            &sidecar_dir.join("t.mkv"),
            Box::new(SystemClock),
        )
        .expect("writer");
        let mut mark = NewMark::point(MarkType::Note, 1000, Source::Ui);
        mark.label = Some("intro".to_owned());
        writer.add_mark(mark).expect("mark");
        let path = writer.finish(&[5000]).expect("finish");
        let (sidecar, _) = ::marks::load(&path).expect("load");
        assert_eq!(mkv_chapters(&sidecar), vec![(1000, 5000, "Note: intro".to_owned())]);
        let _ = fs::remove_dir_all(&sidecar_dir);
    }

    #[test]
    fn a_wav_length_comes_from_its_chunks() {
        let dir = scratch("wav");
        let path = dir.join("a.wav");
        // 48 kHz, 2 channels, 16 bit: 192000 bytes a second, and one second of data.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36u32 + 192_000).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&48_000u32.to_le_bytes());
        bytes.extend_from_slice(&192_000u32.to_le_bytes());
        bytes.extend_from_slice(&4u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&192_000u32.to_le_bytes());
        bytes.extend(std::iter::repeat_n(0u8, 192_000));
        fs::write(&path, &bytes).expect("wav");
        assert_eq!(wav_length_ms(&path), Some(1000));
        let _ = fs::remove_dir_all(&dir);
    }
}
