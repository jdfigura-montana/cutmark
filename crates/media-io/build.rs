//! Build script: copies the FFmpeg DLLs next to the built binaries so they
//! can be loaded.

fn main() {
    copy_ffmpeg_dlls();
}

/// Copies the FFmpeg DLLs from third_party/ffmpeg/dist/bin into the profile
/// folder and its deps and examples folders so binaries, tests and examples
/// load them. Best effort: warns and returns when anything is off, never panics.
fn copy_ffmpeg_dlls() {
    let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") else {
        println!("cargo:warning=FFmpeg DLL copy skipped: CARGO_MANIFEST_DIR not set");
        return;
    };
    let root = std::path::Path::new(&manifest_dir).join("..").join("..");
    let bin_dir = root.join("third_party").join("ffmpeg").join("dist").join("bin");
    println!("cargo:rerun-if-changed={}", bin_dir.display());
    if !bin_dir.is_dir() {
        println!("cargo:warning=FFmpeg DLLs not found; run third_party\\ffmpeg\\get-ffmpeg.ps1");
        return;
    }
    // OUT_DIR is target/<profile>/build/<pkg>-<hash>/out; the 3rd ancestor is <profile>.
    let profile_dir = match std::env::var_os("OUT_DIR") {
        Some(out) => std::path::PathBuf::from(out).ancestors().nth(3).map(std::path::Path::to_path_buf),
        None => None,
    };
    let Some(profile_dir) = profile_dir else {
        println!("cargo:warning=FFmpeg DLL copy skipped: could not locate the target profile folder");
        return;
    };
    let deps_dir = profile_dir.join("deps");
    let examples_dir = profile_dir.join("examples");
    // The examples folder may not exist yet; the copy below creates nothing.
    if !examples_dir.is_dir()
        && let Err(e) = std::fs::create_dir_all(&examples_dir)
    {
        println!("cargo:warning=FFmpeg DLL copy skipped for the examples folder: {e}");
    }
    let entries = match std::fs::read_dir(&bin_dir) {
        Ok(entries) => entries,
        Err(e) => {
            println!("cargo:warning=FFmpeg DLL copy skipped: cannot read {}: {e}", bin_dir.display());
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()).map(|ext| ext.eq_ignore_ascii_case("dll")) != Some(true) {
            continue;
        }
        let Some(name) = path.file_name() else { continue };
        let dst_dirs = [profile_dir.join(name), deps_dir.join(name), examples_dir.join(name)];
        let fresh = match std::fs::metadata(&path) {
            Ok(src) => dst_dirs
                .iter()
                .all(|dst| std::fs::metadata(dst).is_ok_and(|m| m.len() == src.len() && m.modified().is_ok_and(|t| t >= src.modified().unwrap_or(t)))),
            Err(e) => {
                println!("cargo:warning=FFmpeg DLL copy skipped for {}: {e}", name.to_string_lossy());
                continue;
            }
        };
        if fresh {
            continue;
        }
        for dst in &dst_dirs {
            if let Err(e) = std::fs::copy(&path, dst) {
                println!("cargo:warning=FFmpeg DLL copy failed for {}: {e}", name.to_string_lossy());
            }
        }
    }
}
