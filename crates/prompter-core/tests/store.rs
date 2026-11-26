#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

use prompter_core::index::{FORMAT_VERSION, WordIndex};
use prompter_core::limits::MAX_WORDS;
use prompter_core::script::{self, BlockKind, Cue};
use prompter_core::store::{LIBRARY_VERSION, Library, MAX_OPEN_TABS, MAX_SNAPSHOTS, ScriptMeta, Sort, Store, StoreError, sort_scripts};
use tempfile::TempDir;

fn temp_store() -> (TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    (dir, store)
}

fn add(store: &mut Store, title: &str, text: &str) -> String {
    let id = store.new(title, None).unwrap().id;
    store.save(&id, text).unwrap();
    id
}

/// Every file under `folder`, at any depth.
fn files(folder: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            found.extend(files(&path));
        } else {
            found.push(path);
        }
    }
    found
}

fn temp_files(folder: &Path) -> Vec<PathBuf> {
    files(folder).into_iter().filter(|path| path.extension().is_some_and(|e| e == "tmp")).collect()
}

fn deleted_files(base: &Path, id: &str) -> Vec<PathBuf> {
    let history = base.join("scripts/.history").join(id);
    files(&history).into_iter().filter(|path| path.file_name().unwrap().to_str().unwrap().starts_with("deleted-")).collect()
}

fn import_text(store: &mut Store, file: &str, text: &str) -> (TempDir, prompter_core::store::Imported) {
    let outside = tempfile::tempdir().unwrap();
    let source = outside.path().join(file);
    fs::write(&source, text).unwrap();
    let imported = store.import(&source).unwrap();
    assert_eq!(fs::read_to_string(&source).unwrap(), text, "the original is untouched");
    assert_eq!(files(outside.path()), [source], "nothing is added next to the original");
    (outside, imported)
}

#[test]
fn open_makes_the_folders_and_starts_empty() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("prompter");
    let store = Store::open(&base).unwrap();
    assert!(base.join("scripts/.history").is_dir());
    assert!(base.join("cache").is_dir());
    assert_eq!(*store.library(), Library::default());
    assert_eq!(store.library().version, LIBRARY_VERSION);
    assert!(!base.join("library.json").exists());
}

#[test]
fn a_bad_library_is_an_error_and_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.json");
    for bad in ["{ not json", "{}"] {
        fs::write(&path, bad).unwrap();
        assert!(matches!(Store::open(dir.path()), Err(StoreError::Library { .. })));
        assert_eq!(fs::read_to_string(&path).unwrap(), bad);
    }
}

#[test]
fn unknown_fields_survive_a_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.json");
    let id = "0192f0c1-2b3a-7c4d-8e5f-0123456789ab";
    let json = serde_json::json!({
        "version": 1,
        "window": { "x": 40 },
        "scripts": [{
            "id": id,
            "title": "Old",
            "created": "2026-01-02T03:04:05Z",
            "edited": "2026-01-02T03:04:05Z",
            "overrides": { "speed": 140 }
        }]
    });
    fs::write(&path, json.to_string()).unwrap();
    let mut store = Store::open(dir.path()).unwrap();
    store.rename(id, "New").unwrap();

    let written: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(written["window"], json["window"]);
    assert_eq!(written["scripts"][0]["overrides"], json["scripts"][0]["overrides"]);
    assert_eq!(written["scripts"][0]["title"], "New");
    assert_eq!(written["scripts"][0]["favorite"], false);
    assert_eq!(written["scripts"][0]["last_position"], serde_json::Value::Null);
    assert_eq!(*Store::open(dir.path()).unwrap().library(), *store.library());
}

#[test]
fn new_load_and_save() {
    let (dir, mut store) = temp_store();
    let meta = store.new("  Intro  ", None).unwrap();
    assert_eq!(meta.title, "Intro");
    assert_eq!(uuid::Uuid::parse_str(&meta.id).unwrap().get_version_num(), 7);
    assert_eq!(meta.created, meta.edited);
    assert!(!meta.favorite);
    assert_eq!(meta.last_position, None);
    assert_eq!(store.load(&meta.id).unwrap(), "");

    store.save(&meta.id, "# Hello\r\n\r\nWorld é\n").unwrap();
    let reopened = Store::open(dir.path()).unwrap();
    assert_eq!(reopened.load(&meta.id).unwrap(), "# Hello\r\n\r\nWorld é\n");
    assert!(reopened.library().scripts[0].edited >= meta.edited);
    assert_eq!(reopened.library(), store.library());
}

#[test]
fn rename_changes_the_title_only() {
    let (_dir, mut store) = temp_store();
    let id = add(&mut store, "One", "text");
    let edited = store.library().scripts[0].edited;
    store.rename(&id, "Two").unwrap();
    assert_eq!(store.library().scripts[0].title, "Two");
    assert_eq!(store.library().scripts[0].edited, edited);
    assert_eq!(store.load(&id).unwrap(), "text");
    assert!(matches!(store.rename(&id, "  "), Err(StoreError::BlankName)));
    assert!(matches!(store.new("", None), Err(StoreError::BlankName)));
}

#[test]
fn unknown_and_unsafe_ids_are_refused() {
    let (dir, mut store) = temp_store();
    assert!(matches!(store.load("nope"), Err(StoreError::UnknownScript(_))));
    assert!(matches!(store.save("nope", "x"), Err(StoreError::UnknownScript(_))));
    assert!(matches!(store.delete("nope"), Err(StoreError::UnknownScript(_))));
    assert!(matches!(store.snapshots("nope"), Err(StoreError::UnknownScript(_))));

    // A hand-edited library must not reach files outside the scripts folder.
    let json = r#"{"version":1,"scripts":[{"id":"../library","title":"x","created":"2026-01-02T03:04:05Z","edited":"2026-01-02T03:04:05Z"}]}"#;
    fs::write(dir.path().join("library.json"), json).unwrap();
    let mut store = Store::open(dir.path()).unwrap();
    assert!(matches!(store.save("../library", "x"), Err(StoreError::UnknownScript(_))));
    store.cached_index("../escape", "words");
    assert!(!dir.path().join("escape.idx").exists());
}

#[test]
fn folders() {
    let (_dir, mut store) = temp_store();
    store.create_folder("Shows").unwrap();
    store.create_folder("Ads").unwrap();
    assert!(matches!(store.create_folder(" Shows "), Err(StoreError::FolderExists(_))));
    assert!(matches!(store.create_folder(" "), Err(StoreError::BlankName)));
    assert!(matches!(store.new("x", Some("Missing")), Err(StoreError::UnknownFolder(_))));

    let id = store.new("Episode", Some("Shows")).unwrap().id;
    assert!(matches!(store.delete_folder("Shows"), Err(StoreError::FolderNotEmpty(_))));
    assert!(matches!(store.rename_folder("Shows", "Ads"), Err(StoreError::FolderExists(_))));
    assert!(matches!(store.rename_folder("Missing", "x"), Err(StoreError::UnknownFolder(_))));
    store.rename_folder("Shows", "Shows").unwrap();
    store.rename_folder("Shows", "Series").unwrap();
    assert_eq!(store.library().folders, ["Series", "Ads"]);
    assert_eq!(store.library().scripts[0].folder.as_deref(), Some("Series"));

    assert!(matches!(store.move_to_folder(&id, Some("Shows")), Err(StoreError::UnknownFolder(_))));
    store.move_to_folder(&id, Some("Ads")).unwrap();
    assert_eq!(store.library().scripts[0].folder.as_deref(), Some("Ads"));
    store.move_to_folder(&id, None).unwrap();
    assert_eq!(store.library().scripts[0].folder, None);

    store.delete_folder("Ads").unwrap();
    store.delete_folder("Series").unwrap();
    assert!(store.library().folders.is_empty());
    assert!(matches!(store.delete_folder("Ads"), Err(StoreError::UnknownFolder(_))));
}

/// Each script's id and `edited` time, in library order.
fn edited_times(store: &Store) -> Vec<(String, String)> {
    store.library().scripts.iter().map(|script| (script.id.clone(), script.edited.to_rfc3339())).collect()
}

fn written_library(base: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(base.join("library.json")).unwrap()).unwrap()
}

#[test]
fn set_favorite_is_saved_and_leaves_edited_alone() {
    let (dir, mut store) = temp_store();
    let id = add(&mut store, "A", "text");
    let edited = edited_times(&store);
    for favorite in [true, true, false, true] {
        store.set_favorite(&id, favorite).unwrap();
        assert_eq!(store.library().scripts[0].favorite, favorite);
        assert_eq!(Store::open(dir.path()).unwrap().library(), store.library());
    }
    assert_eq!(edited_times(&store), edited);
    assert!(matches!(store.set_favorite("nope", true), Err(StoreError::UnknownScript(_))));
}

#[test]
fn sort_scripts_puts_favorites_first() {
    let (_dir, mut store) = temp_store();
    let template = store.new("x", None).unwrap();
    // A later `second` means edited more recently.
    let script = |title: &str, second: u64, favorite: bool| {
        let edited = template.edited + std::time::Duration::from_secs(second);
        ScriptMeta { title: title.to_owned(), edited, favorite, ..template.clone() }
    };
    let scripts = [script("delta", 4, false), script("Charlie", 1, true), script("alpha", 2, false), script("Bravo", 3, true)];
    let sorted = |sort| {
        let mut list: Vec<&ScriptMeta> = scripts.iter().collect();
        sort_scripts(&mut list, sort);
        list.iter().map(|script| script.title.as_str()).collect::<Vec<_>>()
    };
    assert_eq!(sorted(Sort::Name), ["Bravo", "Charlie", "alpha", "delta"]);
    assert_eq!(sorted(Sort::Edited), ["Bravo", "Charlie", "delta", "alpha"]);
}

#[test]
fn sort_scripts_keeps_the_given_order_for_ties() {
    let (_dir, mut store) = temp_store();
    let first = store.new("Same", None).unwrap();
    let second = ScriptMeta { id: "second".to_owned(), title: "same".to_owned(), ..first.clone() };
    for sort in [Sort::Name, Sort::Edited] {
        for given in [[&first, &second], [&second, &first]] {
            let mut list = given;
            sort_scripts(&mut list, sort);
            assert_eq!(list, given);
        }
    }
}

#[test]
fn move_folder_sets_the_saved_order() {
    let (dir, mut store) = temp_store();
    for folder in ["A", "B", "C", "D"] {
        store.create_folder(folder).unwrap();
    }
    for (folder, to, order) in [
        ("D", 0, ["D", "A", "B", "C"]),
        ("D", 2, ["A", "B", "D", "C"]),
        ("A", 3, ["B", "D", "C", "A"]),
        // Where it already is.
        ("C", 2, ["B", "D", "C", "A"]),
        // Past the end means last.
        ("B", 99, ["D", "C", "A", "B"]),
    ] {
        store.move_folder(folder, to).unwrap();
        assert_eq!(store.library().folders, order, "{folder} to {to}");
        assert_eq!(written_library(dir.path())["folders"], serde_json::json!(order));
    }
    assert_eq!(Store::open(dir.path()).unwrap().library().folders, ["D", "C", "A", "B"]);
    assert!(matches!(store.move_folder("Missing", 0), Err(StoreError::UnknownFolder(_))));
}

#[test]
fn tabs_sort_and_collapsed_folders_are_saved() {
    let (dir, mut store) = temp_store();
    store.create_folder("Shows").unwrap();
    store.create_folder("Ads").unwrap();
    let first = add(&mut store, "A", "text");
    let second = add(&mut store, "B", "text");
    let edited = edited_times(&store);

    store.set_tabs(&[second.clone(), first.clone()], Some(&first)).unwrap();
    store.set_sort(Sort::Edited).unwrap();
    store.set_folder_collapsed("Ads", true).unwrap();
    store.set_folder_collapsed("Shows", true).unwrap();
    // Closing a closed folder does not list it twice.
    store.set_folder_collapsed("Shows", true).unwrap();

    let written = written_library(dir.path());
    assert_eq!(written["open_tabs"], serde_json::json!([second, first]));
    assert_eq!(written["active_tab"], serde_json::json!(first));
    assert_eq!(written["sort"], "edited");
    assert_eq!(written["collapsed_folders"], serde_json::json!(["Ads", "Shows"]));
    assert_eq!(Store::open(dir.path()).unwrap().library(), store.library());

    store.set_folder_collapsed("Ads", false).unwrap();
    store.set_folder_collapsed("Ads", false).unwrap();
    assert_eq!(store.library().collapsed_folders, ["Shows"]);
    store.set_sort(Sort::Name).unwrap();
    assert_eq!(written_library(dir.path())["sort"], "name");
    store.set_tabs(&[], None).unwrap();
    assert!(store.library().open_tabs.is_empty());
    assert_eq!(store.library().active_tab, None);

    assert!(matches!(store.set_folder_collapsed("Missing", true), Err(StoreError::UnknownFolder(_))));
    assert_eq!(edited_times(&store), edited);
}

#[test]
fn set_tabs_refuses_bad_tabs_and_changes_nothing() {
    let (_dir, mut store) = temp_store();
    let ids: Vec<String> = (0..=MAX_OPEN_TABS).map(|n| store.new(&format!("{n}"), None).unwrap().id).collect();
    assert_eq!(MAX_OPEN_TABS, 12);
    store.set_tabs(&ids[..MAX_OPEN_TABS], None).unwrap();
    let before = store.library().clone();

    assert!(matches!(store.set_tabs(&ids, None), Err(StoreError::TooManyTabs)));
    assert!(matches!(store.set_tabs(&["nope".to_owned()], None), Err(StoreError::UnknownScript(_))));
    assert!(matches!(store.set_tabs(&[ids[0].clone(), ids[0].clone()], None), Err(StoreError::RepeatedTab(_))));
    assert!(matches!(store.set_tabs(&ids[..2], Some(&ids[2])), Err(StoreError::TabNotOpen(_))));
    assert_eq!(*store.library(), before);
}

#[test]
fn a_library_without_the_view_fields_gets_the_defaults() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("library.json"), r#"{"version":1,"scripts":[]}"#).unwrap();
    assert_eq!(*Store::open(dir.path()).unwrap().library(), Library::default());
    assert_eq!(Library::default().sort, Sort::Name);
}

#[test]
fn open_drops_view_entries_that_do_not_fit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.json");
    let ids: Vec<String> = (0..14).map(|n| format!("0192f0c1-2b3a-7c4d-8e5f-0123456789{n:02}")).collect();
    let scripts: Vec<_> =
        ids.iter().map(|id| serde_json::json!({ "id": id, "title": "t", "created": "2026-01-02T03:04:05Z", "edited": "2026-01-02T03:04:05Z" })).collect();
    // An unlisted id and a repeat, then more than fit.
    let mut tabs = vec!["gone".to_owned(), ids[0].clone()];
    tabs.extend_from_slice(&ids);
    let json = serde_json::json!({
        "version": 1,
        "scripts": scripts,
        "folders": ["Ads"],
        "open_tabs": tabs,
        // Past the cap, so its tab is dropped.
        "active_tab": ids[13],
        "collapsed_folders": ["Gone", "Ads", "Ads"]
    })
    .to_string();
    fs::write(&path, &json).unwrap();

    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.library().open_tabs, ids[..MAX_OPEN_TABS]);
    assert_eq!(store.library().active_tab, None);
    assert_eq!(store.library().collapsed_folders, ["Ads"]);
    assert_eq!(fs::read_to_string(&path).unwrap(), json, "opening does not write");
}

#[test]
fn delete_closes_the_tab() {
    let (dir, mut store) = temp_store();
    let ids = ["A", "B", "C"].map(|title| add(&mut store, title, "text"));
    store.set_tabs(&ids, Some(&ids[1])).unwrap();
    let edited = edited_times(&store);

    // A tab that is not active.
    store.delete(&ids[0]).unwrap();
    assert_eq!(store.library().open_tabs, ids[1..]);
    assert_eq!(store.library().active_tab.as_ref(), Some(&ids[1]));

    // The active tab.
    store.delete(&ids[1]).unwrap();
    assert_eq!(store.library().open_tabs, ids[2..]);
    assert_eq!(store.library().active_tab, None);
    assert_eq!(Store::open(dir.path()).unwrap().library(), store.library());
    assert_eq!(edited_times(&store), edited[2..]);
}

#[test]
fn folder_changes_update_collapsed_folders() {
    let (dir, mut store) = temp_store();
    for folder in ["Shows", "Ads", "Drafts"] {
        store.create_folder(folder).unwrap();
    }
    store.set_folder_collapsed("Shows", true).unwrap();
    store.set_folder_collapsed("Ads", true).unwrap();

    store.rename_folder("Shows", "Series").unwrap();
    assert_eq!(store.library().collapsed_folders, ["Series", "Ads"]);
    // An open folder stays open.
    store.rename_folder("Drafts", "Notes").unwrap();
    assert_eq!(store.library().collapsed_folders, ["Series", "Ads"]);

    store.delete_folder("Series").unwrap();
    assert_eq!(store.library().collapsed_folders, ["Ads"]);
    assert_eq!(written_library(dir.path())["collapsed_folders"], serde_json::json!(["Ads"]));
    // A new folder with the old name starts open.
    store.create_folder("Series").unwrap();
    assert_eq!(store.library().collapsed_folders, ["Ads"]);
}

#[test]
fn duplicate_copies_the_text() {
    let (_dir, mut store) = temp_store();
    store.create_folder("Shows").unwrap();
    let id = add(&mut store, "Episode", "the text");
    store.move_to_folder(&id, Some("Shows")).unwrap();
    let copy = store.duplicate(&id).unwrap();
    assert_ne!(copy.id, id);
    assert_eq!(copy.title, "Episode copy");
    assert_eq!(copy.folder.as_deref(), Some("Shows"));
    store.save(&copy.id, "changed").unwrap();
    assert_eq!(store.load(&id).unwrap(), "the text");
    assert_eq!(store.library().scripts.len(), 2);
}

#[test]
fn delete_keeps_the_text_in_history() {
    let (dir, mut store) = temp_store();
    let id = add(&mut store, "Gone", "keep me");
    store.snapshot(&id).unwrap();
    store.delete(&id).unwrap();
    assert!(store.library().scripts.is_empty());
    assert!(Store::open(dir.path()).unwrap().library().scripts.is_empty());
    assert!(!dir.path().join(format!("scripts/{id}.md")).exists());
    let deleted = deleted_files(dir.path(), &id);
    assert_eq!(deleted.len(), 1);
    assert_eq!(fs::read_to_string(&deleted[0]).unwrap(), "keep me");
    // The snapshot is kept too.
    assert_eq!(files(&dir.path().join("scripts/.history").join(&id)).len(), 2);
}

#[test]
fn delete_removes_an_entry_whose_file_is_gone() {
    let (dir, mut store) = temp_store();
    let id = add(&mut store, "Lost", "x");
    fs::remove_file(dir.path().join(format!("scripts/{id}.md"))).unwrap();
    store.delete(&id).unwrap();
    assert!(store.library().scripts.is_empty());
}

#[test]
fn a_failed_library_write_changes_nothing() {
    let (dir, mut store) = temp_store();
    let id = add(&mut store, "Stays", "text");
    let before = store.library().clone();
    // A folder in the way makes the rename over library.json fail.
    fs::remove_file(dir.path().join("library.json")).unwrap();
    fs::create_dir(dir.path().join("library.json")).unwrap();

    assert!(matches!(store.rename(&id, "Other"), Err(StoreError::Io { .. })));
    assert!(store.delete(&id).is_err());
    assert!(store.create_folder("x").is_err());
    assert_eq!(*store.library(), before);
    assert_eq!(store.load(&id).unwrap(), "text");
    assert!(temp_files(dir.path()).is_empty());
}

#[test]
fn writes_leave_no_temp_files() {
    let (dir, mut store) = temp_store();
    let id = add(&mut store, "A", "first");
    store.save(&id, "second").unwrap();
    store.duplicate(&id).unwrap();
    store.snapshot(&id).unwrap();
    let first = store.snapshots(&id).unwrap()[0];
    store.restore(&id, first).unwrap();
    store.cached_index(&id, "second");
    store.cached_index(&id, "third");
    store.export_md(&id, &dir.path().join("out.md")).unwrap();
    import_text(&mut store, "in.txt", "words");
    store.delete(&id).unwrap();
    assert!(temp_files(dir.path()).is_empty());
}

#[test]
fn snapshots_are_capped_and_newest_first() {
    let (_dir, mut store) = temp_store();
    let id = add(&mut store, "A", "");
    assert!(store.snapshots(&id).unwrap().is_empty());
    let mut taken = Vec::new();
    // Taken back to back, so several fall in the same millisecond.
    for version in 0..MAX_SNAPSHOTS + 3 {
        store.save(&id, &format!("version {version}")).unwrap();
        taken.push(store.snapshot(&id).unwrap());
    }
    let kept = store.snapshots(&id).unwrap();
    assert_eq!(MAX_SNAPSHOTS, 10);
    taken.reverse();
    assert_eq!(kept, taken[..MAX_SNAPSHOTS]);
    assert!(kept.windows(2).all(|pair| pair[0].taken > pair[1].taken));
}

#[test]
fn a_new_snapshot_is_newest_even_if_the_clock_went_back() {
    let (dir, mut store) = temp_store();
    let id = add(&mut store, "A", "now");
    let history = dir.path().join("scripts/.history").join(&id);
    fs::create_dir_all(&history).unwrap();
    for second in 0..MAX_SNAPSHOTS {
        fs::write(history.join(format!("29990101T0000{second:02}.000Z.md")), "future").unwrap();
    }
    let new = store.snapshot(&id).unwrap();
    let kept = store.snapshots(&id).unwrap();
    assert_eq!(kept.len(), MAX_SNAPSHOTS);
    assert_eq!(kept[0], new);
}

#[test]
fn restore_snapshots_the_current_text_first() {
    let (_dir, mut store) = temp_store();
    let id = add(&mut store, "A", "old");
    let old = store.snapshot(&id).unwrap();
    store.save(&id, "new").unwrap();
    store.restore(&id, old).unwrap();
    assert_eq!(store.load(&id).unwrap(), "old");
    let snapshots = store.snapshots(&id).unwrap();
    assert_eq!(snapshots.len(), 2);
    store.restore(&id, snapshots[0]).unwrap();
    assert_eq!(store.load(&id).unwrap(), "new");
}

#[test]
fn restore_works_on_the_oldest_of_a_full_history() {
    let (_dir, mut store) = temp_store();
    let id = add(&mut store, "A", "oldest");
    let oldest = store.snapshot(&id).unwrap();
    for version in 1..MAX_SNAPSHOTS {
        store.save(&id, &format!("version {version}")).unwrap();
        store.snapshot(&id).unwrap();
    }
    store.restore(&id, oldest).unwrap();
    assert_eq!(store.load(&id).unwrap(), "oldest");
    // The restore pushed it out of the history, so it cannot be restored again.
    assert!(matches!(store.restore(&id, oldest), Err(StoreError::UnknownSnapshot)));
    assert_eq!(store.snapshots(&id).unwrap().len(), MAX_SNAPSHOTS);
}

#[test]
fn import_html_converts_and_takes_the_heading() {
    let folder = tempfile::tempdir().unwrap();
    let mut store = Store::open(folder.path()).unwrap();
    let html = "<html><head><title>Not this</title></head><body><p>Intro [[pause]]</p>                <h1>The Real <i>Title</i></h1><ul><li><b>one</b></li></ul></body></html>";
    let (outside, imported) = import_text(&mut store, "page.HTML", html);
    assert!(!imported.split);
    assert_eq!(imported.scripts[0].title, "The Real Title");
    let saved = store.load(&imported.scripts[0].id).unwrap();
    assert_eq!(
        saved,
        "Intro [[pause]]

# The Real *Title*

- **one**
"
    );
    assert_eq!(fs::read_to_string(outside.path().join("page.HTML")).unwrap(), html);

    // Without a heading the title is the file's name.
    let (_outside, imported) = import_text(&mut store, "My page.html", "<p>words</p>");
    assert_eq!(imported.scripts[0].title, "My page");
}

#[test]
fn import_htm_is_read_like_html() {
    let (_dir, mut store) = temp_store();
    for file in ["page.htm", "page.HTM"] {
        let (_outside, imported) = import_text(&mut store, file, "<h1>Title</h1><p>a<br>b</p><hr>");
        assert_eq!(imported.scripts[0].title, "Title");
        assert_eq!(store.load(&imported.scripts[0].id).unwrap(), "# Title\n\na\\\nb\n\n---\n");
    }
}

#[test]
fn import_html_splits_large_input() {
    let folder = tempfile::tempdir().unwrap();
    let mut store = Store::open(folder.path()).unwrap();
    let body = "<p>word word word word word</p>".repeat(6_000);
    let (_outside, imported) = import_text(&mut store, "long.html", &format!("<h1>Long</h1>{body}<h2>Rest</h2>{body}"));
    assert!(imported.split);
    let titles: Vec<_> = imported.scripts.iter().map(|meta| meta.title.as_str()).collect();
    assert_eq!(titles, ["Long (part 1 of 2)", "Long (part 2 of 2)"]);
    assert!(store.load(&imported.scripts[1].id).unwrap().starts_with(
        "## Rest
"
    ));
}

#[test]
fn import_md_keeps_the_text_and_takes_the_heading() {
    let (_dir, mut store) = temp_store();
    let md = "Intro\n\n## The **Real** Title\n\n- one\n- two\n";
    let (_outside, imported) = import_text(&mut store, "file name.MD", md);
    assert!(!imported.split);
    assert_eq!(imported.scripts.len(), 1);
    assert_eq!(imported.scripts[0].title, "The Real Title");
    assert_eq!(store.load(&imported.scripts[0].id).unwrap(), md);
    assert_eq!(store.library().scripts, imported.scripts);
}

#[test]
fn import_md_drops_a_byte_order_mark() {
    let (_dir, mut store) = temp_store();
    let (_outside, imported) = import_text(&mut store, "a.md", "\u{feff}# Title\n");
    assert_eq!(imported.scripts[0].title, "Title");
    assert_eq!(store.load(&imported.scripts[0].id).unwrap(), "# Title\n");
}

#[test]
fn import_txt_makes_one_paragraph_per_block() {
    let (_dir, mut store) = temp_store();
    let text = "\r\nfirst line\r\nsecond line  \r\n \t\r\n\r\n    indented block\r\n";
    let (_outside, imported) = import_text(&mut store, "My talk.txt", text);
    assert_eq!(imported.scripts[0].title, "My talk");
    let saved = store.load(&imported.scripts[0].id).unwrap();
    assert_eq!(saved, "first line\nsecond line\n\nindented block\n");
    let blocks = script::parse(&saved).blocks;
    assert_eq!(blocks.iter().map(|block| block.text()).collect::<Vec<_>>(), ["first line second line", "indented block"]);
    assert!(blocks.iter().all(|block| block.kind == BlockKind::Paragraph));
}

#[test]
fn import_txt_escapes_markdown_at_line_starts() {
    let lines = [
        "# not a heading",
        "### nor this",
        "> not a quote",
        "- not a list",
        "+ nor this",
        "* or this",
        "1. not numbered",
        "42) nor this",
        "7.",
        "---",
        "===",
        "***",
        "___",
        "```",
        "~~~",
        "<div> is typed",
        "[label]: http://example.com",
        "![image](a.png)",
        "&amp; stays",
        "\\* keeps its backslash",
        "**stars** stay",
        "3.5 percent",
        "2026 was good",
    ];
    // Each line is tried alone, and after another line, where it could end the paragraph.
    for line in lines {
        for text in [line.to_owned(), format!("before\n{line}")] {
            let (_dir, mut store) = temp_store();
            let (_outside, imported) = import_text(&mut store, "a.txt", &text);
            let blocks = script::parse(&store.load(&imported.scripts[0].id).unwrap()).blocks;
            assert_eq!(blocks.len(), 1, "{text:?}");
            assert_eq!(blocks[0].kind, BlockKind::Paragraph, "{text:?}");
            assert_eq!(blocks[0].text(), text.replace('\n', " "));
        }
    }
}

#[test]
fn import_txt_keeps_cues() {
    let (_dir, mut store) = temp_store();
    let (_outside, imported) = import_text(&mut store, "a.txt", "[[pause]]\n");
    let blocks = script::parse(&store.load(&imported.scripts[0].id).unwrap()).blocks;
    assert_eq!(blocks[0].kind, BlockKind::Cue(Cue::Pause));
}

#[test]
fn import_splits_large_input() {
    let (_dir, mut store) = temp_store();
    let half = "word ".repeat(MAX_WORDS / 2 + 100);
    let md = format!("# One\n\n{half}\n\n# Two\n\n{half}\n");
    let (_outside, imported) = import_text(&mut store, "long.md", &md);
    assert!(imported.split);
    let titles: Vec<_> = imported.scripts.iter().map(|meta| meta.title.as_str()).collect();
    assert_eq!(titles, ["One (part 1 of 2)", "One (part 2 of 2)"]);
    let parts: Vec<_> = imported.scripts.iter().map(|meta| store.load(&meta.id).unwrap()).collect();
    assert!(parts[1].starts_with("# Two"));
    assert!(parts.iter().all(|part| part.split_whitespace().count() <= MAX_WORDS));
    assert_eq!(parts.concat(), md);
    assert_eq!(store.library().scripts, imported.scripts);
}

#[test]
fn import_at_the_limit_is_not_split() {
    let (_dir, mut store) = temp_store();
    let (_outside, imported) = import_text(&mut store, "full.txt", &"word ".repeat(MAX_WORDS));
    assert!(!imported.split);
    assert_eq!(imported.scripts.len(), 1);
}

#[test]
fn import_refuses_other_types_and_text_that_is_not_utf8() {
    let (dir, mut store) = temp_store();
    assert!(matches!(store.import(Path::new("a.doc")), Err(StoreError::UnsupportedImport(_))));
    let source = dir.path().join("latin1.txt");
    fs::write(&source, b"caf\xe9").unwrap();
    assert!(matches!(store.import(&source), Err(StoreError::ImportEncoding)));
    assert!(store.library().scripts.is_empty());
}

#[test]
fn export_writes_and_replaces() {
    let (_dir, mut store) = temp_store();
    let id = add(&mut store, "A", "# Exported\n");
    let outside = tempfile::tempdir().unwrap();
    let dest = outside.path().join("copy.md");
    fs::write(&dest, "older").unwrap();
    store.export_md(&id, &dest).unwrap();
    assert_eq!(fs::read_to_string(&dest).unwrap(), "# Exported\n");
    assert_eq!(files(outside.path()), [dest]);

    let blocked = outside.path().join("folder.md");
    fs::create_dir(&blocked).unwrap();
    assert!(matches!(store.export_md(&id, &blocked), Err(StoreError::Io { .. })));
    assert!(temp_files(outside.path()).is_empty());
}

#[test]
fn cached_index_misses_then_hits() {
    let (dir, mut store) = temp_store();
    let id = add(&mut store, "A", "");
    let path = dir.path().join(format!("cache/{id}.idx"));
    let built = WordIndex::build(&script::parse("hello world"));
    assert_eq!(store.cached_index(&id, "hello world"), built);
    assert!(path.is_file());

    // Altering the cached word shows that the next call reads the file.
    let altered = fs::read_to_string(&path).unwrap().replace("hello", "howdy");
    fs::write(&path, &altered).unwrap();
    assert_eq!(store.cached_index(&id, "hello world").tokens()[0].display, "howdy");

    // New text misses and rewrites the cache.
    let changed = store.cached_index(&id, "hello there");
    assert_eq!(changed, WordIndex::build(&script::parse("hello there")));
    assert_eq!(serde_json::from_slice::<WordIndex>(&fs::read(&path).unwrap()).unwrap(), changed);

    // So does a cache from another format version.
    let mut json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    json["format_version"] = (FORMAT_VERSION + 1).into();
    json["tokens"][0]["display"] = "stale".into();
    fs::write(&path, json.to_string()).unwrap();
    assert_eq!(store.cached_index(&id, "hello there"), changed);
}

#[test]
fn cached_index_rebuilds_a_corrupt_file() {
    let (dir, mut store) = temp_store();
    let id = add(&mut store, "A", "");
    let path = dir.path().join(format!("cache/{id}.idx"));
    let built = WordIndex::build(&script::parse("some words"));
    for corrupt in [&b""[..], b"{\"format_version\":1", b"\xff\x00garbage", b"{}"] {
        fs::write(&path, corrupt).unwrap();
        assert_eq!(store.cached_index(&id, "some words"), built);
        assert_eq!(serde_json::from_slice::<WordIndex>(&fs::read(&path).unwrap()).unwrap(), built);
    }
}

#[test]
fn cached_index_works_when_the_cache_cannot_be_written() {
    let (dir, mut store) = temp_store();
    let id = add(&mut store, "A", "");
    fs::create_dir(dir.path().join(format!("cache/{id}.idx"))).unwrap();
    assert_eq!(store.cached_index(&id, "still fine").token_count(), 2);
    assert!(temp_files(dir.path()).is_empty());
}
