//! Windows only: embed version info (VERSIONINFO) into `cutmark-video.exe`, so it shows in
//! Explorer, the taskbar, the Start menu and Alt-Tab. The window uses egui's default icon.
//!
//! On every other target this does nothing (`winresource` is only a build-dependency on Windows
//! hosts). A missing resource compiler is a warning.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    windows::embed_resources();
}

#[cfg(windows)]
mod windows {
    pub fn embed_resources() {
        let mut res = winresource::WindowsResource::new();
        res.set("ProductName", "Cutmark Video")
            .set("FileDescription", "Cutmark Video video editor")
            .set("LegalCopyright", "Copyright (c) Cutmark. All rights reserved.")
            .set("OriginalFilename", "cutmark-video.exe")
            .set("InternalName", "cutmark-video");
        if let Err(e) = res.compile() {
            println!("cargo:warning=cutmark-video.exe built without version resources: {e}");
        }
    }
}

/// Cross-compiling for Windows from macOS or Linux: `winresource` is not available on this host.
#[cfg(not(windows))]
mod windows {
    pub fn embed_resources() {
        println!("cargo:warning=cutmark-video.exe built without version resources (cross-compiled from a non-Windows host)");
    }
}
