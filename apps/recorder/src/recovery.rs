//! Marks recovery on launch: rebuilds the marks of takes that crashed and left a journal behind.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use ::marks::export::{ExportOptions, markdown_with};
use ::marks::{SessionWriter, csv, load};

/// What a recovery scan found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecoveryReport {
    /// The base names of the takes whose marks were recovered; a take whose fragmented MP4 or MOV files were also finalized has " (finalized)" after its name.
    pub recovered: Vec<String>,
    /// What went wrong, one message per failure; a failure on one take never stops the others.
    pub errors: Vec<String>,
}

/// How many folder levels below the takes root the scan looks into: the root, a project folder and a take folder need two, and older layouts fewer.
const MAX_DEPTH: usize = 4;

/// Adds every folder under `folder`, down to `depth` more levels, to `found`; a folder that cannot be listed is skipped.
fn collect_subfolders(folder: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(folder) else {
        log::info!("marks recovery: could not list {}, skipping it", folder.display());
        return;
    };
    for path in entries.flatten().map(|entry| entry.path()).filter(|path| path.is_dir()) {
        found.push(path.clone());
        collect_subfolders(&path, depth - 1, found);
    }
}

/// Recovers the marks of every crashed take in `root` and its subfolders, down to [`MAX_DEPTH`] levels.
///
/// `is_recording` is asked for each journal with its folder and base name; a journal it accepts belongs to the running take and is left alone.
pub fn scan(root: &Path, is_recording: &dyn Fn(&Path, &str) -> bool) -> RecoveryReport {
    log::info!("marks recovery: scanning {}", root.display());
    let mut report = RecoveryReport::default();
    let mut folders = vec![root.to_path_buf()];
    if let Err(err) = fs::read_dir(root) {
        // A takes folder that does not exist yet has nothing to recover.
        if root.exists() {
            report.errors.push(format!("could not list {}: {err}", root.display()));
        } else {
            log::info!("marks recovery: {} does not exist, nothing to recover", root.display());
        }
        return report;
    }
    collect_subfolders(root, MAX_DEPTH, &mut folders);
    for folder in folders {
        log::info!("marks recovery: checking {}", folder.display());
        let journals = match ::marks::write::find_journals(&folder) {
            Ok(journals) => journals,
            Err(err) => {
                report.errors.push(format!("could not look for marks journals in {}: {err}", folder.display()));
                continue;
            }
        };
        for base_name in journals {
            log::info!("marks recovery: found journal {base_name}.marks.journal in {}", folder.display());
            if is_recording(&folder, &base_name) {
                log::info!("marks recovery: skipped {base_name}, it belongs to the take being recorded");
                continue;
            }
            match recover_take(&folder, &base_name, &mut report.errors) {
                Ok(true) => {
                    log::info!("marks recovery: recovered {base_name}");
                    if finalize_take(&folder, &base_name, &mut report.errors) {
                        report.recovered.push(format!("{base_name}{FINALIZED_NOTE}"));
                    } else {
                        report.recovered.push(base_name);
                    }
                }
                Ok(false) => log::info!("marks recovery: skipped {base_name}, the journal was gone"),
                Err(err) => {
                    log::info!("marks recovery: skipped {base_name}, recovery failed: {err}");
                    report.errors.push(format!("could not recover the marks of {base_name}: {err}"));
                }
            }
        }
    }
    report
}

/// Recovers one take and rewrites its Markdown and CSV logs; false when there was no journal after all.
///
/// A log that cannot be written is added to `errors`; the recovered sidecar still counts.
fn recover_take(folder: &Path, base_name: &str, errors: &mut Vec<String>) -> Result<bool, String> {
    let Some(path) = SessionWriter::recover(folder, base_name).map_err(|err| err.to_string())? else { return Ok(false) };
    let (sidecar, _warnings) = load(&path).map_err(|err| format!("the recovered marks file could not be read: {err}"))?;
    let markdown = markdown_with(&sidecar, &ExportOptions { include_system: true });
    if let Err(err) = write_atomic(&folder.join(format!("{base_name}_marks.md")), markdown.as_bytes()) {
        errors.push(format!("the Markdown marks log of {base_name} could not be written: {err}"));
    }
    if let Err(err) = write_atomic(&folder.join(format!("{base_name}_marks.csv")), &csv(&sidecar)) {
        errors.push(format!("the CSV marks log of {base_name} could not be written: {err}"));
    }
    Ok(true)
}

/// What follows the name of a recovered take in the report when its video files were finalized.
const FINALIZED_NOTE: &str = " (finalized)";

/// Finalizes every MP4 and MOV file of a recovered take that is still fragmented, the take file and its camera file alike.
///
/// Returns true when at least one file was finalized. A file that cannot be finalized stays as it was and adds a message to `errors`.
fn finalize_take(folder: &Path, base_name: &str, errors: &mut Vec<String>) -> bool {
    let Ok(entries) = fs::read_dir(folder) else { return false };
    let mut files: Vec<(PathBuf, media_io::Container)> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter_map(|path| {
            let stem = path.file_stem()?.to_str()?;
            if stem != base_name && !stem.strip_prefix(base_name).is_some_and(|rest| rest.starts_with('_')) {
                return None;
            }
            let container = match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
                "mp4" => media_io::Container::Mp4,
                "mov" => media_io::Container::Mov,
                _ => return None,
            };
            Some((path, container))
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut finalized = false;
    for (path, container) in files {
        match is_fragmented(&path) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(err) => {
                log::info!("marks recovery: could not inspect {}: {err}", path.display());
                continue;
            }
        }
        log::info!("marks recovery: finalizing {}", path.display());
        let result = media_io::init().and_then(|()| media_io::mux::finalize(&path, container, &mut |_| {}));
        match result {
            Ok(()) => finalized = true,
            Err(err) => errors.push(format!("could not finalize {}, it stays fragmented: {err}", path.display())),
        }
    }
    finalized
}

/// Whether an MP4 or MOV file is still fragmented: a moof box follows the moov box, or the moov box holds an empty duration.
fn is_fragmented(path: &Path) -> std::io::Result<bool> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let mut offset = 0u64;
    let mut moov_seen = false;
    while offset + 8 <= len {
        file.seek(SeekFrom::Start(offset))?;
        let mut header = [0u8; 8];
        file.read_exact(&mut header)?;
        let mut size = u64::from(u32::from_be_bytes([header[0], header[1], header[2], header[3]]));
        let mut header_len = 8u64;
        if size == 1 {
            let mut large = [0u8; 8];
            file.read_exact(&mut large)?;
            size = u64::from_be_bytes(large);
            header_len = 16;
        } else if size == 0 {
            size = len - offset;
        }
        if size < header_len {
            break;
        }
        match &header[4..8] {
            b"moof" if moov_seen => return Ok(true),
            b"moov" if !moov_seen => {
                moov_seen = true;
                if moov_duration_is_empty(&mut file, size - header_len)? {
                    return Ok(true);
                }
            }
            _ => {}
        }
        offset = offset.saturating_add(size);
    }
    Ok(false)
}

/// Whether the mvhd box that opens the moov box, read from the current position of `file`, holds a duration of zero.
fn moov_duration_is_empty(file: &mut File, payload_len: u64) -> std::io::Result<bool> {
    let mut bytes = Vec::new();
    file.take(payload_len.min(48)).read_to_end(&mut bytes)?;
    // The child header takes 8 bytes, then the mvhd version and flags; the duration follows the creation time, modification time and time scale.
    if bytes.len() < 12 || &bytes[4..8] != b"mvhd" {
        return Ok(false);
    }
    let duration = match bytes[8] {
        0 => bytes.get(20..24).map(|field| u64::from(u32::from_be_bytes([field[0], field[1], field[2], field[3]]))),
        1 => bytes.get(32..40).map(|field| u64::from_be_bytes([field[0], field[1], field[2], field[3], field[4], field[5], field[6], field[7]])),
        _ => None,
    };
    Ok(duration == Some(0))
}

/// Writes a file through a temp file in the same folder, then renames it over the target.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
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
    use ::marks::{MarkType, NewMark, Source, SystemClock, WriterInfo};

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("cutmark_recovery_{name}_{}_{nanos}", std::process::id()));
        fs::create_dir_all(&dir).expect("scratch folder");
        dir
    }

    /// Starts a session with one mark and drops it without finishing, as a crash would.
    fn crashed_take(dir: &Path, base_name: &str) {
        let info = WriterInfo { app: "test".to_owned(), version: "0".to_owned() };
        let mut writer = SessionWriter::begin(dir, base_name, info, &dir.join(format!("{base_name}.mkv")), Box::new(SystemClock)).expect("writer");
        let mut mark = NewMark::point(MarkType::Note, 1500, Source::Hotkey);
        mark.label = Some("before the crash".to_owned());
        writer.add_mark(mark).expect("mark");
    }

    #[test]
    fn a_crashed_take_is_recovered_with_its_logs() {
        let root = scratch("crashed");
        let sub = root.join("project");
        fs::create_dir_all(&sub).expect("subfolder");
        crashed_take(&root, "top");
        crashed_take(&sub, "nested");
        let report = scan(&root, &|_, _| false);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        let mut recovered = report.recovered.clone();
        recovered.sort();
        assert_eq!(recovered, vec!["nested".to_owned(), "top".to_owned()]);
        assert!(root.join("top.marks.json").exists());
        assert!(!root.join("top.marks.journal").exists());
        let md = fs::read_to_string(sub.join("nested_marks.md")).expect("markdown log");
        assert!(md.contains("before the crash"), "{md}");
        let csv = fs::read_to_string(sub.join("nested_marks.csv")).expect("csv log");
        assert!(csv.contains("before the crash"), "{csv}");
        // A second scan finds nothing left to do.
        assert_eq!(scan(&root, &|_, _| false), RecoveryReport::default());
        let _ = fs::remove_dir_all(&root);
    }

    /// Writes a fragmented MP4 with the muxer: three tiny MJPEG key frames, no trailer index in front of them.
    fn fragmented_mp4(path: &Path) {
        use ffmpeg_next::codec::Parameters;
        use ffmpeg_next::ffi;
        use ffmpeg_next::packet::{Flags, Packet};
        use media_io::encode::EncodedPacket;
        media_io::init().expect("ffmpeg init");
        let mut params = Parameters::new();
        // SAFETY: params owns a fresh AVCodecParameters, so writing its plain fields is sound.
        #[allow(unsafe_code)]
        unsafe {
            let raw = params.as_mut_ptr();
            (*raw).codec_type = ffi::AVMediaType::AVMEDIA_TYPE_VIDEO;
            (*raw).codec_id = ffi::AVCodecID::AV_CODEC_ID_MJPEG;
            (*raw).width = 16;
            (*raw).height = 16;
        }
        let mut muxer = media_io::mux::Muxer::create(path, media_io::Container::Mp4).expect("muxer");
        muxer.add_video(params, (1, 90_000), (30, 1)).expect("video stream");
        muxer.start().expect("start");
        for index in 0..3i64 {
            let mut packet = Packet::copy(&[0xFF, 0xD8, 0xFF, 0xD9]);
            packet.set_pts(Some(index * 3000));
            packet.set_dts(Some(index * 3000));
            packet.set_duration(3000);
            packet.set_flags(Flags::KEY);
            muxer.write(0, &EncodedPacket::from_raw_packet(packet)).expect("packet");
        }
        muxer.finish().expect("finish");
    }

    #[test]
    fn a_crashed_take_has_its_fragmented_mp4_files_finalized() {
        let root = scratch("finalize");
        let base = "2026-11-19_04-20-23_take01";
        crashed_take(&root, base);
        let main = root.join(format!("{base}.mp4"));
        let cam = root.join(format!("{base}_cam.mp4"));
        // A file of another take whose name starts the same way stays alone.
        let other = root.join(format!("{base}2.mp4"));
        for path in [&main, &cam, &other] {
            fragmented_mp4(path);
            assert!(is_fragmented(path).expect("inspect"), "{} should start fragmented", path.display());
        }
        let report = scan(&root, &|_, _| false);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_eq!(report.recovered, vec![format!("{base} (finalized)")]);
        assert!(!is_fragmented(&main).expect("inspect"));
        assert!(!is_fragmented(&cam).expect("inspect"));
        assert!(is_fragmented(&other).expect("inspect"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_fragmented_file_that_cannot_be_finalized_stays_and_adds_an_error() {
        let root = scratch("finalize_fails");
        let base = "take01";
        crashed_take(&root, base);
        // A moov box followed by a moof box marks the file as fragmented, but there is nothing to read in it.
        let broken = [0, 0, 0, 8, b'm', b'o', b'o', b'v', 0, 0, 0, 8, b'm', b'o', b'o', b'f'];
        let path = root.join(format!("{base}.mp4"));
        fs::write(&path, broken).expect("broken file");
        let report = scan(&root, &|_, _| false);
        assert_eq!(report.recovered, vec![base.to_owned()]);
        assert_eq!(report.errors.len(), 1, "{:?}", report.errors);
        assert!(report.errors[0].contains("stays fragmented"), "{:?}", report.errors);
        assert_eq!(fs::read(&path).expect("file kept"), broken);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_journal_in_a_take_folder_under_the_root_is_recovered() {
        let root = scratch("take_folder");
        let base = "2026-11-19_04-20-23_take11";
        let take_folder = root.join(base);
        fs::create_dir_all(&take_folder).expect("take folder");
        crashed_take(&take_folder, base);
        assert!(take_folder.join(format!("{base}.marks.journal")).exists());
        let report = scan(&root, &|_, _| false);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_eq!(report.recovered, vec![base.to_owned()]);
        assert!(take_folder.join(format!("{base}_marks.md")).exists());
        assert!(take_folder.join(format!("{base}.marks.json")).exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_journal_in_a_project_and_take_folder_is_recovered() {
        let root = scratch("project_layout");
        let base = "2026-11-19_04-20-23_take11";
        let take_folder = root.join("Cutmark").join(base);
        fs::create_dir_all(&take_folder).expect("take folder");
        crashed_take(&take_folder, base);
        // A journal deeper than the layout needs is still found, one beyond the depth limit is not.
        let deep = root.join("a").join("b").join("c").join("d");
        let too_deep = deep.join("e");
        fs::create_dir_all(&too_deep).expect("deep folders");
        crashed_take(&deep, "deep");
        crashed_take(&too_deep, "too_deep");
        let report = scan(&root, &|_, _| false);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        let mut recovered = report.recovered.clone();
        recovered.sort();
        assert_eq!(recovered, vec![base.to_owned(), "deep".to_owned()]);
        assert!(take_folder.join(format!("{base}_marks.md")).exists());
        assert!(take_folder.join(format!("{base}.marks.json")).exists());
        assert!(!take_folder.join(format!("{base}.marks.journal")).exists());
        assert!(too_deep.join("too_deep.marks.journal").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_journal_two_folders_below_the_root_is_found_and_reported() {
        let root = scratch("two_below");
        let base = "2026-11-19_04-20-23_take01";
        let take_folder = root.join("Test4a").join(base);
        fs::create_dir_all(&take_folder).expect("take folder");
        crashed_take(&take_folder, base);
        let report = scan(&root, &|_, _| false);
        assert_eq!(report, RecoveryReport { recovered: vec![base.to_owned()], errors: Vec::new() });
        assert!(take_folder.join(format!("{base}.marks.json")).exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_running_take_is_skipped() {
        let root = scratch("running");
        crashed_take(&root, "live");
        crashed_take(&root, "old");
        let report = scan(&root, &|_, base| base == "live");
        assert_eq!(report.recovered, vec!["old".to_owned()]);
        assert!(root.join("live.marks.journal").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_folder_is_not_an_error() {
        let root = scratch("missing").join("nothing_here");
        assert_eq!(scan(&root, &|_, _| false), RecoveryReport::default());
    }
}
