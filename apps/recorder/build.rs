//! Build script: embeds the Windows icon and version info into the exe and
//! copies the FFmpeg DLLs next to the built binaries so they can be loaded.

fn main() -> std::io::Result<()> {
    copy_ffmpeg_dlls();

    // Only for Windows targets; other platforms need no resources.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resources()?;
    }
    Ok(())
}

/// Embeds the Recorder's icon and version info into the exe.
fn embed_resources() -> std::io::Result<()> {
    let icon = "../../crates/ui-kit/assets/brand/icon-recorder.ico";
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={icon}");
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let mut res = winresource::WindowsResource::new();
    res.set_icon(icon);
    res.set("ProductName", "CutMark Recorder");
    res.set("FileDescription", "CutMark Recorder");
    res.set("FileVersion", &version);
    res.set("ProductVersion", &version);
    res.compile()
}

/// Copies the FFmpeg DLLs from third_party/ffmpeg/dist/bin into the profile
/// folder, its deps folder (where test binaries run) and its examples folder
/// (where example binaries run) so they can be loaded. Best effort: warns and
/// returns when anything is off, never panics.
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
    let examples_dir = profile_dir.join("examples");
    let _ = std::fs::create_dir_all(&examples_dir);
    let dst_dirs = [profile_dir.clone(), profile_dir.join("deps"), examples_dir];
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
        let fresh = match std::fs::metadata(&path) {
            Ok(src) => dst_dirs.iter().all(|dst| {
                std::fs::metadata(dst.join(name)).is_ok_and(|m| m.len() == src.len() && m.modified().is_ok_and(|t| t >= src.modified().unwrap_or(t)))
            }),
            Err(e) => {
                println!("cargo:warning=FFmpeg DLL copy skipped for {}: {e}", name.to_string_lossy());
                continue;
            }
        };
        if fresh {
            continue;
        }
        for dst_dir in &dst_dirs {
            if let Err(e) = std::fs::copy(&path, dst_dir.join(name)) {
                println!("cargo:warning=FFmpeg DLL copy failed for {}: {e}", name.to_string_lossy());
            }
        }
    }
}
