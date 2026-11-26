//! Headless tests of the whole Library: real frames, pointer and keys, with the system
//! clipboard and files dragged from another program stood in for.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect};

use super::*;

const CTRL: Modifiers = Modifiers { alt: false, ctrl: true, shift: false, mac_cmd: false, command: true };
const CTRL_SHIFT: Modifiers = Modifiers { shift: true, ..CTRL };

/// What is on the stand-in clipboard.
#[derive(Default)]
struct Board {
    html: Option<String>,
    text: Option<String>,
}

#[derive(Clone, Default)]
struct FakeClipboard(Rc<RefCell<Board>>);

impl Clipboard for FakeClipboard {
    fn read_html(&self) -> Option<String> {
        self.0.borrow().html.clone()
    }

    fn read_text(&self) -> Option<String> {
        self.0.borrow().text.clone()
    }

    fn write_text(&self, text: &str) -> prompter_platform::Result<()> {
        *self.0.borrow_mut() = Board { html: None, text: Some(text.to_owned()) };
        Ok(())
    }
}

/// A file as egui reports one that was dropped.
#[derive(Debug)]
struct Dropped(PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|err| err.to_string())
    }
}

struct Library {
    ctx: egui::Context,
    app: PrompterApp,
    frame: eframe::Frame,
    time: f64,
    clipboard: FakeClipboard,
    /// Files dragged over the window.
    hovered: Vec<egui::HoveredFile>,
    /// Files dropped on the window, for the next frame.
    dropped: Vec<egui::DroppedFileHandle>,
    /// The window's size.
    size: egui::Vec2,
    /// What the last frame painted.
    shapes: Vec<egui::epaint::ClippedShape>,
    folder: PathBuf,
}

impl Drop for Library {
    fn drop(&mut self) {
        // A folder left behind in the temp folder does no harm.
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}

impl Library {
    /// A Library with one open script holding `text`, kept in a folder of its own.
    fn with_script(text: &str) -> Self {
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        let name = format!("cutmark-prompter-test-{}-{}", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed));
        let folder = std::env::temp_dir().join(name);
        let mut workspace = Workspace::new(Store::open(&folder).unwrap());
        workspace.new_script("Test").unwrap();
        workspace.open_script_mut().unwrap().text = text.to_owned();
        workspace.text_changed(Instant::now());
        let ctx = egui::Context::default();
        ui_kit::apply(&ctx);
        let mut app = PrompterApp::with_library(true, AlphaInUse::default(), Ok(workspace));
        let clipboard = FakeClipboard::default();
        app.clipboard = Box::new(clipboard.clone());
        let frame = eframe::Frame::_new_kittest();
        let size = egui::vec2(1100.0, 720.0);
        let mut library = Self { ctx, app, frame, time: 0.0, clipboard, hovered: Vec::new(), dropped: Vec::new(), size, shapes: Vec::new(), folder };
        library.settle();
        library
    }

    fn step(&mut self, events: Vec<Event>) {
        self.time += 0.1;
        let screen = Rect::from_min_size(Pos2::ZERO, self.size);
        let hovered_files = self.hovered.clone();
        let dropped_files = std::mem::take(&mut self.dropped);
        let input =
            egui::RawInput { screen_rect: Some(screen), time: Some(self.time), focused: true, events, hovered_files, dropped_files, ..Default::default() };
        let (app, frame) = (&mut self.app, &self.frame);
        let mut output = self.ctx.run_ui(input, |ui| {
            egui::CentralPanel::default().frame(egui::Frame::new()).show(ui, |ui| app.library(ui, frame));
        });
        output.textures_delta.clear();
        let left_to_egui = output.platform_output.commands.iter().any(|command| matches!(command, egui::OutputCommand::CopyText(_)));
        assert!(!left_to_egui, "a copy did not go through the app's clipboard");
        self.shapes = output.shapes;
    }

    /// Ctrl+V or Ctrl+Shift+V as the window reports it: no key press, only a paste with the
    /// clipboard's text, which the app does not use.
    fn press_paste(&mut self, modifiers: Modifiers) {
        let window_text = self.clipboard.read_text().unwrap_or_default();
        self.step(vec![Event::ModifiersChanged(modifiers), Event::Paste(window_text)]);
        self.step(vec![Event::ModifiersChanged(Modifiers::NONE)]);
        self.settle();
    }

    fn put_on_clipboard(&self, html: Option<&str>, text: Option<&str>) {
        *self.clipboard.0.borrow_mut() = Board { html: html.map(str::to_owned), text: text.map(str::to_owned) };
    }

    fn copied(&self) -> String {
        self.clipboard.read_text().unwrap_or_default()
    }

    /// A file outside the library, to drag in.
    fn outside_file(&self, name: &str, text: &str) -> PathBuf {
        let folder = self.folder.join("outside");
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join(name);
        std::fs::write(&path, text).unwrap();
        path
    }

    /// Drags files over `at` and drops them there.
    fn drop_files(&mut self, files: &[PathBuf], at: Pos2) {
        self.app.drag_pointer_at = Some(at);
        self.hovered = files.iter().map(|path| egui::HoveredFile { path: Some(path.clone()), ..Default::default() }).collect();
        self.settle();
        self.hovered.clear();
        self.dropped = files.iter().map(|path| Arc::new(Dropped(path.clone())) as egui::DroppedFileHandle).collect();
        self.settle();
    }

    /// Where the last frame drew the outline of a drop target.
    fn drop_mark(&self) -> Option<Rect> {
        fn search(shape: &egui::Shape) -> Option<Rect> {
            match shape {
                egui::Shape::Rect(painted) if painted.stroke.width == DROP_STROKE => Some(painted.rect),
                egui::Shape::Vec(shapes) => shapes.iter().find_map(search),
                _ => None,
            }
        }
        self.shapes.iter().find_map(|clipped| search(&clipped.shape))
    }

    /// Each script's title and folder, by title.
    fn filed(&self) -> Vec<(String, Option<String>)> {
        let scripts = &self.app.workspace.as_ref().unwrap().store().library().scripts;
        let mut filed: Vec<_> = scripts.iter().map(|script| (script.title.clone(), script.folder.clone())).collect();
        filed.sort();
        filed
    }

    fn settle(&mut self) {
        for _ in 0..5 {
            self.step(Vec::new());
        }
    }

    fn key(&mut self, modifiers: Modifiers, key: Key) {
        let event = Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        self.step(vec![Event::ModifiersChanged(modifiers), event]);
        self.step(vec![Event::ModifiersChanged(Modifiers::NONE)]);
        self.settle();
    }

    fn type_text(&mut self, text: &str) {
        self.step(vec![Event::Text(text.to_owned())]);
        self.settle();
    }

    fn click(&mut self, at: Pos2, button: PointerButton) {
        let event = |pressed| Event::PointerButton { pos: at, button, pressed, modifiers: Modifiers::NONE };
        self.step(vec![Event::PointerMoved(at)]);
        self.step(vec![event(true)]);
        self.step(vec![event(false)]);
        self.settle();
    }

    /// Where the last frame painted exactly `text`, the topmost if it did so twice. A menu
    /// row's text is its label and then its shortcut.
    fn find_text(&self, text: &str) -> Option<Rect> {
        fn search(shape: &egui::Shape, text: &str) -> Option<Rect> {
            match shape {
                egui::Shape::Text(painted) if painted.galley.text() == text => Some(painted.visual_bounding_rect()),
                egui::Shape::Vec(shapes) => shapes.iter().rev().find_map(|shape| search(shape, text)),
                _ => None,
            }
        }
        self.shapes.iter().rev().find_map(|clipped| search(&clipped.shape, text))
    }

    fn click_text(&mut self, text: &str) {
        let rect = self.find_text(text).unwrap_or_else(|| panic!("\"{text}\" is not on screen"));
        self.click(rect.center(), PointerButton::Primary);
    }

    fn pick(&mut self, menu: &str, item: Item) {
        self.click_text(menu);
        self.click_text(&format!("{}{}", item.label(), item.chord()));
    }

    fn script(&self) -> &str {
        &self.app.workspace.as_ref().unwrap().open_script().unwrap().text
    }

    fn editor_id(&self) -> egui::Id {
        editor::id(self.app.workspace.as_ref().unwrap().open_script().unwrap().serial)
    }

    fn focused(&self) -> Option<egui::Id> {
        self.ctx.memory(|memory| memory.focused())
    }

    /// Clicks into the editor, past the end of its text.
    fn click_into_editor(&mut self) {
        let text = self.script().to_owned();
        let rect = self.find_text(&text).unwrap();
        self.click(rect.left_bottom() + egui::vec2(20.0, 60.0), PointerButton::Primary);
        assert_eq!(self.focused(), Some(self.editor_id()));
    }
}

#[test]
fn ctrl_f_opens_the_find_bar_with_focus_in_its_field() {
    let mut library = Library::with_script("words");
    library.click_into_editor();
    library.key(CTRL, Key::F);
    assert!(library.app.find.is_open());
    assert!(library.app.find.field().is_some());
    assert_eq!(library.focused(), library.app.find.field());
}

#[test]
fn find_counts_three_matches_and_next_moves_on() {
    let text = "one take, two Take, three TAKE";
    let mut library = Library::with_script(text);
    library.key(CTRL, Key::F);
    library.type_text("take");
    assert!(library.find_text("1 of 3").is_some());
    library.click_text(ui_kit::icons::regular::CARET_DOWN);
    assert!(library.find_text("2 of 3").is_some());

    // The keys step both ways and wrap.
    library.key(Modifiers::NONE, Key::Enter);
    assert!(library.find_text("3 of 3").is_some());
    library.key(Modifiers::NONE, Key::F3);
    assert!(library.find_text("1 of 3").is_some());
    library.key(Modifiers::SHIFT, Key::Enter);
    assert!(library.find_text("3 of 3").is_some());
    library.key(Modifiers::SHIFT, Key::F3);
    assert!(library.find_text("2 of 3").is_some());
    assert_eq!(library.focused(), library.app.find.field());

    // Esc closes the bar, and the editor has the current match selected.
    library.key(Modifiers::NONE, Key::Escape);
    assert!(!library.app.find.is_open());
    assert_eq!(library.focused(), Some(library.editor_id()));
    library.type_text("TOOK");
    assert_eq!(library.script(), "one take, two TOOK, three TAKE");

    // Typing in the editor updates the count of an open bar. The bar reopens where it was.
    library.key(CTRL, Key::F);
    assert!(library.find_text("2 of 2").is_some());
    library.click_into_editor();
    library.type_text(" take");
    assert!(library.find_text("2 of 3").is_some(), "{}", library.script());
}

#[test]
fn ctrl_shift_f_puts_focus_in_the_scripts_search_field() {
    let mut library = Library::with_script("words");
    library.click_into_editor();
    library.key(CTRL_SHIFT, Key::F);
    assert!(library.app.scripts.search_id.is_some());
    assert_eq!(library.focused(), library.app.scripts.search_id);
    assert!(!library.app.find.is_open());

    // The text is selected, so typing replaces it. The menu item does the same.
    library.type_text("first");
    library.click_into_editor();
    library.pick("Edit", Item::SearchAll);
    assert_eq!(library.focused(), library.app.scripts.search_id);
    library.type_text("zz");
    assert!(library.find_text("zz").is_some());
    assert_eq!(library.script(), "words");
}

#[test]
fn file_new_folder_opens_the_naming_dialog() {
    let mut library = Library::with_script("words");
    library.pick("File", Item::NewFolder);
    assert!(matches!(library.app.dialog, Some(Dialog::Name { naming: Naming::NewFolder, .. })));
    library.type_text("Ads");
    library.key(Modifiers::NONE, Key::Enter);
    assert!(library.app.dialog.is_none());
    assert_eq!(library.app.workspace.as_ref().unwrap().store().library().folders, ["Ads"]);

    library.pick("File", Item::NewScript);
    assert!(matches!(library.app.dialog, Some(Dialog::Name { naming: Naming::NewScript, .. })));
    library.type_text("Intro");
    library.key(Modifiers::NONE, Key::Enter);
    let workspace = library.app.workspace.as_ref().unwrap();
    assert_eq!(workspace.title(&workspace.open_script().unwrap().id), "Intro");
}

#[test]
fn the_editor_context_menu_copies_and_pastes_the_selection() {
    let mut library = Library::with_script("alpha beta");
    library.click_into_editor();
    library.key(CTRL, Key::Home);
    for _ in 0..5 {
        library.key(Modifiers::SHIFT, Key::ArrowRight);
    }

    // A right-click inside the selection keeps it. With nothing on the clipboard, Paste is off.
    let line = library.find_text("alpha beta").unwrap();
    library.click(line.left_center() + egui::vec2(12.0, 0.0), PointerButton::Secondary);
    assert!(library.find_text("CopyCtrl+C").is_some());
    assert!(library.app.menu_state().editor.has_selection);
    assert!(!library.app.menu_state().clipboard_has_text);
    library.click_text("CopyCtrl+C");
    assert_eq!(library.copied(), "alpha");
    assert!(library.find_text("CopyCtrl+C").is_none(), "the menu is still open");

    // A right-click past the text moves the cursor to the end. Asking the clipboard whether it
    // has text must not paste it.
    library.click(line.left_bottom() + egui::vec2(20.0, 60.0), PointerButton::Secondary);
    assert!(library.app.menu_state().clipboard_has_text);
    assert!(!library.app.menu_state().editor.has_selection);
    assert_eq!(library.script(), "alpha beta");
    library.click_text("PasteCtrl+V");
    assert_eq!(library.script(), "alpha betaalpha");

    // The menu bar's Edit menu finds the selection too, after the menu has had the keyboard.
    library.key(CTRL, Key::A);
    library.put_on_clipboard(None, None);
    library.pick("Edit", Item::Cut);
    assert_eq!((library.script(), library.copied().as_str()), ("", "alpha betaalpha"));
    library.pick("Edit", Item::Paste);
    assert_eq!(library.script(), "alpha betaalpha");
}

#[test]
fn ctrl_v_pastes_markdown_over_the_selection_as_one_undo_step() {
    let mut library = Library::with_script("alpha beta");
    library.click_into_editor();
    library.key(CTRL, Key::Home);
    for _ in 0..5 {
        library.key(Modifiers::SHIFT, Key::ArrowRight);
    }
    library.put_on_clipboard(Some("<p><b>bold</b></p>"), Some("bold"));
    library.press_paste(CTRL);
    assert_eq!(library.script(), "**bold** beta");
    library.key(CTRL, Key::Z);
    assert_eq!(library.script(), "alpha beta");
    library.key(CTRL, Key::Y);
    assert_eq!(library.script(), "**bold** beta");

    // Typing right before and right after a paste, with no pause, stays apart from it.
    library.key(CTRL, Key::End);
    library.type_text(" one ");
    library.press_paste(CTRL_SHIFT);
    assert_eq!(library.script(), "**bold** beta one bold");
    library.type_text(" two");
    library.key(CTRL, Key::Z);
    assert_eq!(library.script(), "**bold** beta one bold");
    library.key(CTRL, Key::Z);
    assert_eq!(library.script(), "**bold** beta one ");
}

#[test]
fn edit_paste_takes_the_html_and_paste_as_plain_text_does_not() {
    let mut library = Library::with_script("");
    library.put_on_clipboard(Some("<i>soft</i>"), Some("soft"));
    library.pick("Edit", Item::Paste);
    assert_eq!(library.script(), "*soft*");
    library.pick("Edit", Item::PastePlain);
    assert_eq!(library.script(), "*soft*soft");
    // A menu paste is one step too.
    library.key(CTRL, Key::Z);
    assert_eq!(library.script(), "*soft*");

    // With HTML and no text, only Paste has something to paste.
    library.put_on_clipboard(Some("<b>only</b>"), None);
    library.pick("Edit", Item::PastePlain);
    assert_eq!(library.script(), "*soft*");
    library.key(Modifiers::NONE, Key::Escape);
    library.pick("Edit", Item::Paste);
    assert_eq!(library.script(), "*soft***only**");
}

#[test]
fn a_paste_into_the_find_field_stays_as_the_window_sent_it() {
    let mut library = Library::with_script("some words");
    library.key(CTRL, Key::F);
    library.put_on_clipboard(Some("<b>words</b>"), Some("words"));
    library.press_paste(CTRL);
    assert!(library.find_text("1 of 1").is_some());
    assert_eq!(library.script(), "some words");
}

#[test]
fn files_dropped_on_a_folder_row_import_into_it_and_elsewhere_into_no_folder() {
    let mut library = Library::with_script("words");
    library.app.workspace.as_mut().unwrap().create_folder("Ads").unwrap();
    library.settle();
    let heading = library.find_text("Ads").unwrap();
    // The row reaches past its text, across the panel.
    let on_row = egui::pos2(200.0, heading.center().y);

    let files = [library.outside_file("one.md", "# One"), library.outside_file("two.txt", "second"), library.outside_file("three.png", "")];
    library.drop_files(&files, on_row);
    let ads = Some("Ads".to_owned());
    assert_eq!(library.filed(), [("One".to_owned(), ads.clone()), ("Test".to_owned(), None), ("two".to_owned(), ads.clone())]);
    assert!(library.find_text("Not imported, unsupported file type:\nthree.png").is_some());
    assert_eq!(library.script(), "second\n");

    // Over the editor, which is outside the Scripts panel.
    let elsewhere = [library.outside_file("four.html", "<h1>Four</h1>")];
    library.drop_files(&elsewhere, egui::pos2(500.0, 300.0));
    assert!(library.filed().contains(&("Four".to_owned(), None)), "{:?}", library.filed());
}

#[test]
fn a_file_drag_marks_the_folder_row_under_the_pointer_or_else_the_scripts_panel() {
    let mut library = Library::with_script("words");
    library.app.workspace.as_mut().unwrap().create_folder("Ads").unwrap();
    library.settle();
    assert_eq!(library.drop_mark(), None);
    let heading = library.find_text("Ads").unwrap();

    library.hovered = vec![egui::HoveredFile::default()];
    library.app.drag_pointer_at = Some(egui::pos2(200.0, heading.center().y));
    library.settle();
    let row = library.drop_mark().unwrap();
    assert!(row.contains(heading.center()) && row.height() < 40.0 && row.width() > 200.0, "{row:?}");

    library.app.drag_pointer_at = Some(egui::pos2(500.0, 300.0));
    library.settle();
    let panel = library.drop_mark().unwrap();
    assert!(panel.contains_rect(row) && panel.width() <= SCRIPTS_WIDTH && panel.height() > 500.0, "{panel:?}");

    // The drag leaves the window.
    library.hovered.clear();
    library.settle();
    assert_eq!(library.drop_mark(), None);
}

#[test]
fn a_dropped_file_that_cannot_be_read_is_named_in_a_toast() {
    let mut library = Library::with_script("words");
    let missing = library.folder.join("gone.md");
    library.drop_files(&[missing], egui::pos2(100.0, 500.0));
    let named = |shape: &egui::Shape| matches!(shape, egui::Shape::Text(text) if text.galley.text().starts_with("gone.md could not be imported: "));
    assert!(library.shapes.iter().any(|clipped| named(&clipped.shape)));
    assert_eq!(library.filed(), [("Test".to_owned(), None)]);
}

#[test]
fn the_preview_chip_shows_the_script_and_follows_the_cursor() {
    let mut library = Library::with_script("# Title\n\nSome **bold** words");
    // The preview shows the text without its Markdown markers.
    assert!(library.find_text("Some bold words").is_none());
    library.click_text("Preview");
    assert!(library.find_text("Some bold words").is_some());
    assert_eq!(library.app.preview.followed_line(), Some(1));

    library.click_into_editor();
    library.key(CTRL, Key::End);
    assert_eq!(library.app.preview.followed_line(), Some(3));
    library.type_text("\n\nmore");
    assert_eq!(library.app.preview.followed_line(), Some(5));

    library.click_text("Preview");
    assert!(library.find_text("Some bold words").is_none());
}

#[test]
fn file_snapshots_takes_one_and_restores_it_only_after_asking() {
    let mut library = Library::with_script("first text");
    library.pick("File", Item::Snapshots);
    assert!(library.find_text("No snapshots yet.").is_some());
    library.click_text("Snapshot now");
    assert!(library.find_text("No snapshots yet.").is_none());
    library.click_text("Close");

    library.click_into_editor();
    library.key(CTRL, Key::A);
    library.type_text("second");
    library.pick("File", Item::Snapshots);
    library.click_text("Preview");
    assert!(library.find_text("first text").is_some());
    library.click_text("Back");

    // Cancel at the question changes nothing.
    library.click_text("Restore");
    library.click_text("Cancel");
    assert_eq!(library.script(), "second");
    library.click_text("Restore");
    assert_eq!(library.script(), "second", "restored without asking");
    library.click_text("Restore");
    assert!(library.app.dialog.is_none());
    assert_eq!(library.script(), "first text");

    // The text from before the restore is the newest snapshot, and the restored one is on disk.
    let store = library.app.workspace.as_ref().unwrap().store();
    let id = store.library().scripts[0].id.clone();
    let kept: Vec<String> = store.snapshots(&id).unwrap().into_iter().map(|meta| store.snapshot_text(&id, meta).unwrap()).collect();
    assert_eq!(kept, ["second", "first text"]);
    assert_eq!(store.load(&id).unwrap(), "first text");
}

#[test]
fn ten_snapshots_leave_the_dialog_buttons_on_screen_in_the_smallest_window() {
    let mut library = Library::with_script("words");
    library.size = egui::vec2(900.0, 480.0);
    library.settle();
    library.pick("File", Item::Snapshots);
    for _ in 0..11 {
        library.click_text("Snapshot now");
    }
    let button = library.find_text("Snapshot now").unwrap();
    assert!(button.bottom() < 480.0, "{button:?}");
    let id = library.app.workspace.as_ref().unwrap().open_script().unwrap().id.clone();
    assert_eq!(library.app.workspace.as_ref().unwrap().store().snapshots(&id).unwrap().len(), 10);
}

#[test]
fn a_right_click_on_empty_space_in_the_scripts_panel_offers_new_and_import() {
    let mut library = Library::with_script("words");
    let empty = egui::pos2(100.0, 500.0);
    library.click(empty, PointerButton::Secondary);
    for item in SCRIPTS_MENU.iter().copied().flatten() {
        assert!(library.find_text(&format!("{}{}", item.label(), item.chord())).is_some(), "{item:?}");
    }
    library.click_text("New folderCtrl+Shift+N");
    assert!(matches!(library.app.dialog, Some(Dialog::Name { naming: Naming::NewFolder, .. })));
}
