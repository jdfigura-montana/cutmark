//! Which files can be imported, for the Import dialog and for files dragged in from another
//! program.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The extensions of the files the store imports, in lower case.
pub const EXTENSIONS: [&str; 6] = ["txt", "md", "html", "htm", "docx", "odt"];

/// The most names the toast for unsupported files has lines for.
const LISTED: usize = 10;

/// Splits files into those that can be imported and those that cannot, by extension, keeping
/// their order.
pub fn by_support(paths: Vec<PathBuf>) -> (Vec<PathBuf>, Vec<PathBuf>) {
    paths.into_iter().partition(|path| {
        let extension = path.extension().and_then(OsStr::to_str);
        extension.is_some_and(|extension| EXTENSIONS.iter().any(|supported| extension.eq_ignore_ascii_case(supported)))
    })
}

/// A file's name without its folders.
pub fn name(path: &Path) -> String {
    path.file_name().unwrap_or(path.as_os_str()).to_string_lossy().into_owned()
}

/// What to say about files that cannot be imported, one name to a line.
pub fn unsupported_message(paths: &[PathBuf]) -> String {
    let mut message = "Not imported, unsupported file type:".to_owned();
    for path in paths.iter().take(LISTED) {
        message.push('\n');
        message.push_str(&name(path));
    }
    if paths.len() > LISTED {
        message.push_str(&format!("\nand {} more", paths.len() - LISTED));
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn dropped_files_are_filtered_by_extension_without_case() {
        let supported = ["a.txt", "b.md", "c.html", "d.htm", "e.docx", "f.odt", "G.DOCX", "h.tar.Md"];
        let unsupported = ["i.doc", "j.pdf", "k.rtf", "notes", ".md", "l.md.bak", "a folder"];
        let all: Vec<&str> = supported.iter().chain(&unsupported).copied().collect();
        assert_eq!(by_support(paths(&all)), (paths(&supported), paths(&unsupported)));
    }

    #[test]
    fn the_unsupported_files_are_listed_by_name() {
        let message = unsupported_message(&paths(&["folder/a.png", "b.pdf"]));
        assert_eq!(message, "Not imported, unsupported file type:\na.png\nb.pdf");
    }

    #[test]
    fn a_long_list_is_cut_with_a_count() {
        let many: Vec<PathBuf> = (1..=13).map(|number| PathBuf::from(format!("{number}.png"))).collect();
        let message = unsupported_message(&many);
        assert!(message.ends_with("\n10.png\nand 3 more"), "{message}");
    }
}
