//! Headless tests of the whole Library: real frames, pointer and keys, with the window's
//! clipboard stood in for.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect};

use super::*;

const CTRL: Modifiers = Modifiers { alt: false, ctrl: true, shift: false, mac_cmd: false, command: true };
const CTRL_SHIFT: Modifiers = Modifiers { shift: true, ..CTRL };

struct Library {
    ctx: egui::Context,
    app: PrompterApp,
    frame: eframe::Frame,
    time: f64,
    clipboard: String,
    /// Frames until a requested paste arrives.
    paste_due: Option<u8>,
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
        let app = PrompterApp::with_library(true, AlphaInUse::default(), Ok(workspace));
        let frame = eframe::Frame::_new_kittest();
        let mut library = Self { ctx, app, frame, time: 0.0, clipboard: String::new(), paste_due: None, shapes: Vec::new(), folder };
        library.settle();
        library
    }

    fn step(&mut self, mut events: Vec<Event>) {
        if let Some(due) = &mut self.paste_due {
            *due -= 1;
            if *due == 0 {
                self.paste_due = None;
                // As eframe does, an empty clipboard sends nothing.
                if !self.clipboard.is_empty() {
                    events.push(Event::Paste(self.clipboard.clone()));
                }
            }
        }
        self.time += 0.1;
        let screen = Rect::from_min_size(Pos2::ZERO, egui::vec2(1100.0, 720.0));
        let input = egui::RawInput { screen_rect: Some(screen), time: Some(self.time), focused: true, events, ..Default::default() };
        let (app, frame) = (&mut self.app, &self.frame);
        let mut output = self.ctx.run_ui(input, |ui| {
            egui::CentralPanel::default().frame(egui::Frame::new()).show(ui, |ui| app.library(ui, frame));
        });
        output.textures_delta.clear();
        for command in &output.platform_output.commands {
            if let egui::OutputCommand::CopyText(text) = command {
                self.clipboard.clone_from(text);
            }
        }
        if output.viewport_output.values().any(|viewport| viewport.commands.contains(&egui::ViewportCommand::RequestPaste)) {
            self.paste_due = Some(2);
        }
        self.shapes = output.shapes;
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

    /// Where the last frame painted exactly `text`. A menu row's text is its label and then
    /// its shortcut.
    fn find_text(&self, text: &str) -> Option<Rect> {
        fn search(shape: &egui::Shape, text: &str) -> Option<Rect> {
            match shape {
                egui::Shape::Text(painted) if painted.galley.text() == text => Some(painted.visual_bounding_rect()),
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| search(shape, text)),
                _ => None,
            }
        }
        self.shapes.iter().find_map(|clipped| search(&clipped.shape, text))
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
    assert_eq!(library.clipboard, "alpha");
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
    library.clipboard.clear();
    library.pick("Edit", Item::Cut);
    assert_eq!((library.script(), library.clipboard.as_str()), ("", "alpha betaalpha"));
    library.pick("Edit", Item::Paste);
    assert_eq!(library.script(), "alpha betaalpha");
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
