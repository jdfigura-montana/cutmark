//! Tests for chords and the hotkey map. No real global hotkeys are registered here.

use std::error::Error;

use cutmark_hotkeys::{Binding, Chord, ChordError, Conflict, HotkeyMap, Key, MapError, altgr_conflict};

type TestResult = Result<(), Box<dyn Error>>;

fn chord(text: &str) -> Result<Chord, ChordError> {
    text.parse()
}

#[test]
fn chord_parse_and_display_round_trip() -> TestResult {
    assert_eq!(chord("control+alt+r")?.to_string(), "Ctrl+Alt+R");
    assert_eq!(chord("shift+f9")?.to_string(), "Shift+F9");
    assert_eq!(chord("F9")?.to_string(), "F9");
    assert_eq!(chord("win+SHIFT+alt+CTRL+f24")?.to_string(), "Ctrl+Alt+Shift+Win+F24");
    // Modifiers come back in the canonical order, so the canonical text round trips.
    let canonical = "Ctrl+Alt+Shift+Win+PageDown";
    assert_eq!(chord(canonical)?.to_string(), canonical);
    Ok(())
}

#[test]
fn every_key_round_trips_bare_and_with_all_modifiers() -> TestResult {
    for &key in Key::ALL {
        let bare = key.to_string();
        assert_eq!(chord(&bare)?.to_string(), bare);
        let full = format!("Ctrl+Alt+Shift+Win+{key}");
        assert_eq!(chord(&full)?.to_string(), full);
    }
    Ok(())
}

#[test]
fn equality_ignores_how_the_chord_was_typed() -> TestResult {
    assert_eq!(chord("Ctrl+Alt+R")?, chord("control+alt+r")?);
    assert_eq!(chord("Ctrl+Alt+R")?, chord("ALT + CTRL + R")?);
    assert_ne!(chord("Ctrl+Alt+R")?, chord("Ctrl+Alt+Shift+R")?);
    assert_eq!(chord("Ctrl+Esc")?, chord("ctrl+escape")?);
    assert_eq!(chord("Ctrl+,")?, chord("Ctrl+Comma")?);
    Ok(())
}

#[test]
fn bad_chords_are_rejected() {
    for bad in ["", "   ", "+", "Ctrl+", "+R", "Ctrl++R", "Ctrl+Alt", "Shift", "Ctrl+Ctrl+R", "Control+Ctrl+R", "Hyper+R", "R+Ctrl", "Ctrl+F25", "Ctrl+Nope"] {
        assert!(chord(bad).is_err(), "{bad:?} should not parse");
    }
}

#[test]
fn bad_chords_name_the_problem() {
    assert_eq!(chord(""), Err(ChordError::Empty));
    assert_eq!(chord("Ctrl+Ctrl+R"), Err(ChordError::DuplicateModifier("Ctrl")));
    assert_eq!(chord("Hyper+R"), Err(ChordError::UnknownModifier("Hyper".to_owned())));
    assert_eq!(chord("Ctrl+Nope"), Err(ChordError::UnknownKey("Nope".to_owned())));
    assert!(matches!(chord("Ctrl+Alt"), Err(ChordError::NoKey(_))));
}

#[test]
fn chord_builds_a_global_hotkey_without_registering_it() -> TestResult {
    let same_a = chord("Ctrl+Alt+R")?.to_global_hotkey();
    let same_b = chord("alt+control+r")?.to_global_hotkey();
    let other = chord("Ctrl+Alt+P")?.to_global_hotkey();
    assert_eq!(same_a.id(), same_b.id());
    assert_ne!(same_a.id(), other.id());
    Ok(())
}

#[test]
fn ensure_defaults_adds_missing_defaults_and_keeps_user_changes() -> TestResult {
    let defaults = [("start_stop", "Ctrl+Alt+R"), ("pause", "Ctrl+Alt+P")];
    let mut map = HotkeyMap::default();

    assert!(map.ensure_defaults("recorder", &defaults).is_empty());
    assert_eq!(map.bindings_for("recorder"), vec![("start_stop".to_owned(), chord("Ctrl+Alt+R")?), ("pause".to_owned(), chord("Ctrl+Alt+P")?),]);

    // The user moves pause; a later start with a new default must keep that and add only the new one.
    map.assign("recorder", "pause", &chord("Ctrl+Alt+Shift+P")?)?;
    let more_defaults = [("start_stop", "Ctrl+Alt+R"), ("pause", "Ctrl+Alt+P"), ("mute", "Ctrl+Alt+M")];
    assert!(map.ensure_defaults("recorder", &more_defaults).is_empty());

    assert_eq!(
        map.bindings_for("recorder"),
        vec![("start_stop".to_owned(), chord("Ctrl+Alt+R")?), ("pause".to_owned(), chord("Ctrl+Alt+Shift+P")?), ("mute".to_owned(), chord("Ctrl+Alt+M")?),]
    );
    assert_eq!(map.bindings.len(), 3);
    Ok(())
}

#[test]
fn ensure_defaults_stores_the_canonical_chord_text() {
    let mut map = HotkeyMap::default();
    let conflicts = map.ensure_defaults("recorder", &[("start_stop", "control+alt+r")]);
    assert!(conflicts.is_empty());
    assert_eq!(map.bindings, vec![Binding { chord: "Ctrl+Alt+R".to_owned(), owner: "recorder".to_owned(), action: "start_stop".to_owned() }]);
}

#[test]
fn a_default_clashing_with_another_owner_comes_back_as_a_conflict() -> TestResult {
    let mut map = HotkeyMap::default();
    map.assign("prompter", "scroll_toggle", &chord("F9")?)?;

    let conflicts = map.ensure_defaults("recorder", &[("silent_mark", "f9"), ("pause", "Ctrl+Alt+P")]);

    assert_eq!(conflicts, vec![Conflict { action: "silent_mark".to_owned(), chord: "f9".to_owned(), held_by: "prompter".to_owned() }]);
    // The clashing default is not added; the other one is.
    assert_eq!(map.bindings_for("recorder"), vec![("pause".to_owned(), chord("Ctrl+Alt+P")?)]);
    assert_eq!(map.bindings_for("prompter").len(), 1);
    Ok(())
}

#[test]
fn assign_refuses_a_chord_owned_by_another_app_and_names_it() -> TestResult {
    let mut map = HotkeyMap::default();
    map.assign("prompter", "scroll_toggle", &chord("Ctrl+Alt+S")?)?;

    let refused = map.assign("recorder", "start_stop", &chord("control+ALT+s")?);
    match refused {
        Err(MapError::OwnedBy { ref owner }) => assert_eq!(owner, "prompter"),
        other => return Err(format!("expected OwnedBy, got {other:?}").into()),
    }
    assert!(refused.err().is_some_and(|err| err.to_string().contains("prompter")));
    assert!(map.bindings_for("recorder").is_empty());
    Ok(())
}

#[test]
fn assign_replaces_the_owners_own_binding_for_the_action() -> TestResult {
    let mut map = HotkeyMap::default();
    map.assign("recorder", "start_stop", &chord("Ctrl+Alt+R")?)?;
    map.assign("recorder", "start_stop", &chord("Ctrl+Alt+T")?)?;
    // Assigning the chord the owner already holds is not a conflict.
    map.assign("recorder", "start_stop", &chord("Ctrl+Alt+T")?)?;

    assert_eq!(map.bindings_for("recorder"), vec![("start_stop".to_owned(), chord("Ctrl+Alt+T")?)]);
    assert_eq!(map.bindings.len(), 1);
    Ok(())
}

#[test]
fn unassign_removes_only_that_binding() -> TestResult {
    let mut map = HotkeyMap::default();
    map.assign("recorder", "start_stop", &chord("Ctrl+Alt+R")?)?;
    map.assign("recorder", "pause", &chord("Ctrl+Alt+P")?)?;

    assert!(map.unassign("recorder", "pause"));
    assert!(!map.unassign("recorder", "pause"));
    assert_eq!(map.bindings_for("recorder").len(), 1);
    Ok(())
}

#[test]
fn bindings_that_no_longer_parse_are_skipped_and_reported() -> TestResult {
    let mut map = HotkeyMap::default();
    map.assign("recorder", "start_stop", &chord("Ctrl+Alt+R")?)?;
    map.bindings.push(Binding { chord: "Ctrl+Nope".to_owned(), owner: "recorder".to_owned(), action: "broken".to_owned() });

    assert_eq!(map.bindings_for("recorder").len(), 1);
    let invalid = map.invalid_bindings();
    assert_eq!(invalid.len(), 1);
    let (binding, reason) = invalid.first().ok_or("one invalid binding")?;
    assert_eq!(binding.action, "broken");
    assert_eq!(*reason, ChordError::UnknownKey("Nope".to_owned()));
    Ok(())
}

#[test]
fn save_then_load_round_trips() -> TestResult {
    let dir = tempfile::tempdir()?;
    // The Cutmark folder does not exist yet; save creates it.
    let path = dir.path().join("Cutmark").join("hotkeys.json");

    let mut map = HotkeyMap::default();
    map.ensure_defaults("recorder", &[("start_stop", "Ctrl+Alt+R"), ("mark", "F9")]);
    map.assign("prompter", "scroll_toggle", &chord("Ctrl+Alt+S")?)?;
    map.save(&path)?;

    assert_eq!(HotkeyMap::load(&path)?, map);

    let text = std::fs::read_to_string(&path)?;
    assert!(!text.contains('\r'), "the file uses LF line endings");
    assert!(text.ends_with('\n'));
    assert!(text.contains("\"version\": 1"));

    // Saving again over an existing file works and leaves no temp file behind.
    map.unassign("recorder", "mark");
    map.save(&path)?;
    assert_eq!(HotkeyMap::load(&path)?, map);
    let leftovers = std::fs::read_dir(dir.path().join("Cutmark"))?.count();
    assert_eq!(leftovers, 1);
    Ok(())
}

#[test]
fn a_missing_file_loads_as_an_empty_map() -> TestResult {
    let dir = tempfile::tempdir()?;
    let map = HotkeyMap::load(dir.path().join("missing.json"))?;
    assert_eq!(map, HotkeyMap::default());
    assert_eq!(map.version, 1);
    assert!(map.bindings.is_empty());
    Ok(())
}

#[test]
fn bad_json_is_an_error_for_the_caller_to_handle() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("hotkeys.json");
    std::fs::write(&path, "{ not json")?;
    assert!(matches!(HotkeyMap::load(&path), Err(MapError::Json { .. })));
    Ok(())
}

#[test]
fn default_path_is_the_cutmark_folder_in_the_config_dir() {
    if let Some(path) = HotkeyMap::default_path() {
        assert!(path.ends_with("Cutmark/hotkeys.json"));
    }
}

#[test]
fn altgr_is_only_checked_for_ctrl_plus_alt() -> TestResult {
    assert_eq!(altgr_conflict(&chord("Ctrl+R")?), None);
    assert_eq!(altgr_conflict(&chord("Alt+R")?), None);
    assert_eq!(altgr_conflict(&chord("F9")?), None);
    Ok(())
}
