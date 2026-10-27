//! Build script: embeds the Windows icon and version info into the exe.

fn main() {
    // Only for Windows targets; other platforms need no resources.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon/app.ico");
        res.set("ProductName", "Audio Editor");
        res.set("FileDescription", "Audio Editor");
        res.set("FileVersion", &version);
        res.set("ProductVersion", &version);
        if let Err(e) = res.compile() {
            panic!("failed to embed Windows resources: {e}");
        }
    }
}
