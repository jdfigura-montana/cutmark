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

fn release(key: Key, modifiers: Modifiers) -> Event {
    Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers }
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
    /// The cursor the last frame asked for.
    cursor: egui::CursorIcon,
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
        app.prefs = EditorPrefs::load(&folder);
        let clipboard = FakeClipboard::default();
        app.clipboard = Box::new(clipboard.clone());
        let frame = eframe::Frame::_new_kittest();
        let size = egui::vec2(1100.0, 720.0);
        let (hovered, dropped, shapes, cursor) = (Vec::new(), Vec::new(), Vec::new(), egui::CursorIcon::Default);
        let mut library = Self { ctx, app, frame, time: 0.0, clipboard, hovered, dropped, size, shapes, cursor, folder };
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
        self.cursor = output.platform_output.cursor_icon;
        self.shapes = output.shapes;
    }

    /// A Library with scripts "Test", the open one, "Alpha" and "Beta", all unfiled, and the
    /// empty folders "Ads" and "Clips".
    fn with_folders() -> Self {
        let mut library = Self::with_script("words");
        library.add_script("Alpha", "a");
        library.add_script("Beta", "b");
        let workspace = library.app.workspace.as_mut().unwrap();
        workspace.create_folder("Ads").unwrap();
        workspace.create_folder("Clips").unwrap();
        library.app.script_action(&library.ctx.clone(), Action::Open(library.id_of("Test")));
        library.settle();
        library
    }

    /// Puts the primary button down at `at`, and keeps it down.
    fn hold(&mut self, at: Pos2) {
        self.step(vec![Event::PointerMoved(at)]);
        self.step(vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }]);
    }

    fn move_to(&mut self, at: Pos2) {
        self.step(vec![Event::PointerMoved(at)]);
        self.step(Vec::new());
    }

    fn let_go(&mut self, at: Pos2) {
        self.step(vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
        self.settle();
    }

    fn drag_row(&mut self, from: &str, to: &str) {
        let (from, to) = (self.row(from).center(), self.row(to).center());
        self.hold(from);
        self.move_to(to);
        self.let_go(to);
    }

    /// A click with Ctrl or Shift held.
    fn click_holding(&mut self, at: Pos2, modifiers: Modifiers) {
        let event = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers };
        self.step(vec![Event::ModifiersChanged(modifiers), Event::PointerMoved(at)]);
        self.step(vec![event(true)]);
        self.step(vec![event(false)]);
        self.step(vec![Event::ModifiersChanged(Modifiers::NONE)]);
        self.settle();
    }

    fn folder_of(&self, title: &str) -> Option<String> {
        self.filed().into_iter().find(|(other, _)| other == title).unwrap().1
    }

    fn folders(&self) -> Vec<String> {
        self.workspace().store().library().folders.clone()
    }

    /// Where the last frame drew the line showing where a dragged folder would go.
    fn insertion_line(&self) -> Option<f32> {
        let accent = ui_kit::Theme::get(&self.ctx).accent.accent;
        self.shapes.iter().find_map(|clipped| match &clipped.shape {
            egui::Shape::LineSegment { points, stroke } if stroke.width == 2.0 && stroke.color == accent => Some(points[0].y),
            _ => None,
        })
    }

    /// Moves the keyboard with Tab to the widget around `at`.
    fn tab_to(&mut self, at: Pos2) {
        self.ctx.memory_mut(|memory| memory.stop_text_input());
        for _ in 0..80 {
            self.key(Modifiers::NONE, Key::Tab);
            let focused = self.focused().and_then(|id| self.ctx.read_response(id));
            // The row or header, not a row's star or anything larger around it.
            if focused.is_some_and(|response| response.rect.contains(at) && response.rect.width() > 24.0 && response.rect.height() < 40.0) {
                return;
            }
        }
        panic!("Tab never reached {at:?}");
    }

    /// Ctrl+V or Ctrl+Shift+V as the window reports it: for the press, no key, only a paste
    /// with the clipboard's text, which the app does not use, and nothing at all with no text.
    /// Then the release of V.
    fn press_paste(&mut self, modifiers: Modifiers) {
        self.step_paste(modifiers);
        self.step(vec![release(Key::V, modifiers)]);
        self.step(vec![Event::ModifiersChanged(Modifiers::NONE)]);
        self.settle();
    }

    /// The press alone.
    fn step_paste(&mut self, modifiers: Modifiers) {
        let window_text = self.clipboard.read_text().filter(|text| !text.is_empty());
        let mut events = vec![Event::ModifiersChanged(modifiers)];
        events.extend(window_text.map(Event::Paste));
        self.step(events);
    }

    /// Adds a script, which opens in a tab of its own.
    fn add_script(&mut self, title: &str, text: &str) {
        let workspace = self.app.workspace.as_mut().unwrap();
        workspace.new_script(title).unwrap();
        workspace.open_script_mut().unwrap().text = text.to_owned();
        workspace.text_changed(Instant::now());
        self.settle();
    }

    fn workspace(&self) -> &Workspace {
        self.app.workspace.as_ref().unwrap()
    }

    /// The tabs' titles, in order.
    fn tab_titles(&self) -> Vec<&str> {
        self.workspace().tabs().iter().map(|tab| self.workspace().title(&tab.id)).collect()
    }

    fn active_title(&self) -> &str {
        self.workspace().title(&self.workspace().open_script().unwrap().id)
    }

    fn id_of(&self, title: &str) -> String {
        let scripts = &self.workspace().store().library().scripts;
        scripts.iter().find(|script| script.title == title).unwrap().id.clone()
    }

    fn is_favorite(&self, title: &str) -> bool {
        self.workspace().store().library().scripts.iter().any(|script| script.title == title && script.favorite)
    }

    /// The tab at `at`, where the last frame drew it.
    fn tab_rect(&self, at: usize) -> Rect {
        self.app.tab_bar.rects[at]
    }

    /// The title of a script's row in the Scripts panel.
    fn row(&self, title: &str) -> Rect {
        self.find_text_in(title, Rect::from_min_max(Pos2::ZERO, egui::pos2(SCRIPTS_WIDTH, self.size.y))).unwrap_or_else(|| panic!("\"{title}\" has no row"))
    }

    fn find_text_in(&self, text: &str, area: Rect) -> Option<Rect> {
        self.text_rects(text).into_iter().find(|rect| area.contains(rect.center()))
    }

    /// Everywhere the last frame painted exactly `text`.
    fn text_rects(&self, text: &str) -> Vec<Rect> {
        fn search(shape: &egui::Shape, text: &str, found: &mut Vec<Rect>) {
            match shape {
                egui::Shape::Text(painted) if painted.galley.text() == text => found.push(painted.visual_bounding_rect()),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| search(shape, text, found)),
                _ => {}
            }
        }
        let mut found = Vec::new();
        self.shapes.iter().for_each(|clipped| search(&clipped.shape, text, &mut found));
        found
    }

    /// The star at the left of a script's row.
    fn star(&self, title: &str) -> Pos2 {
        let row = self.row(title);
        egui::pos2(row.left() - 8.0, row.center().y)
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
                // With a fill: the editor's focus outline is as heavy, and has none.
                egui::Shape::Rect(painted) if painted.stroke.width == DROP_STROKE && painted.fill != egui::Color32::TRANSPARENT => Some(painted.rect),
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

    /// Gives the editor the keyboard without a click, which empty text has nothing to aim at.
    fn focus_editor(&mut self) {
        let id = self.editor_id();
        self.ctx.memory_mut(|memory| memory.request_focus(id));
        self.settle();
        assert_eq!(self.focused(), Some(id));
    }

    /// True if the Scripts panel shows exactly `text`.
    fn in_scripts(&self, text: &str) -> bool {
        self.find_text_in(text, Rect::from_min_max(Pos2::ZERO, egui::pos2(SCRIPTS_WIDTH, self.size.y))).is_some()
    }

    /// Starts the app again on the same library.
    fn restart(&mut self) {
        eframe::App::on_exit(&mut self.app);
        self.ctx = egui::Context::default();
        ui_kit::apply(&self.ctx);
        let workspace = Workspace::new(Store::open(&self.folder).unwrap());
        self.app = PrompterApp::with_library(true, AlphaInUse::default(), Ok(workspace));
        self.app.prefs = EditorPrefs::load(&self.folder);
        self.app.clipboard = Box::new(self.clipboard.clone());
        self.settle();
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

#[test]
fn ctrl_tab_and_ctrl_w_work_from_the_editor_without_editing_it() {
    let mut library = Library::with_script("first words");
    library.add_script("Two", "second words");
    library.add_script("Three", "third words");
    library.click_into_editor();

    library.key(CTRL, Key::Tab);
    assert_eq!(library.active_title(), "Test");
    assert_eq!(library.focused(), Some(library.editor_id()));
    library.key(CTRL_SHIFT, Key::Tab);
    assert_eq!(library.active_title(), "Three");
    library.key(CTRL_SHIFT, Key::Tab);
    assert_eq!(library.active_title(), "Two");
    assert_eq!(library.script(), "second words");

    // egui's editor would delete a word on Ctrl+W.
    library.key(CTRL, Key::W);
    assert_eq!(library.tab_titles(), ["Test", "Three"]);
    assert_eq!(library.active_title(), "Three");
    assert_eq!(library.focused(), Some(library.editor_id()));
    let texts: Vec<&str> = library.workspace().tabs().iter().map(|tab| tab.text.as_str()).collect();
    assert_eq!(texts, ["first words", "third words"]);
    assert_eq!(library.workspace().store().load(&library.id_of("Two")).unwrap(), "second words");

    // With a dialog open, the keys leave the tabs alone.
    library.pick("File", Item::NewFolder);
    library.key(CTRL, Key::W);
    assert_eq!(library.tab_titles().len(), 2);
}

#[test]
fn each_tab_keeps_its_cursor_selection_and_undo_history() {
    let mut library = Library::with_script("alpha beta");
    library.add_script("Two", "gamma delta");
    library.key(CTRL, Key::Tab);
    library.click_into_editor();
    library.type_text(" one");
    library.key(CTRL, Key::Home);
    for _ in 0..5 {
        library.key(Modifiers::SHIFT, Key::ArrowRight);
    }

    library.key(CTRL, Key::Tab);
    library.key(CTRL, Key::End);
    library.type_text(" two");
    assert_eq!(library.script(), "gamma delta two");

    // Back on the first tab, the selection is still there.
    library.key(CTRL, Key::Tab);
    assert!(library.app.menu_state().editor.has_selection);
    assert!(library.app.menu_state().editor.can_undo);
    library.type_text("ALPHA");
    assert_eq!(library.script(), "ALPHA beta one");
    library.key(CTRL, Key::Z);
    library.key(CTRL, Key::Z);
    assert_eq!(library.script(), "alpha beta");

    // The other tab's history is its own.
    library.key(CTRL, Key::Tab);
    assert_eq!(library.script(), "gamma delta two");
    library.key(CTRL, Key::Z);
    assert_eq!(library.script(), "gamma delta");
}

#[test]
fn each_tab_keeps_its_scroll_until_it_is_closed() {
    let long = (1..=200).map(|n| format!("line {n}")).collect::<Vec<_>>().join("\n");
    let mut library = Library::with_script(&long);
    library.add_script("Two", "short");
    library.key(CTRL, Key::Tab);
    let top = |library: &Library| library.find_text(library.script()).unwrap().top();
    let unscrolled = top(&library);

    // Scrolled by the wheel, so the cursor stays at the top.
    library.step(vec![Event::PointerMoved(egui::pos2(500.0, 300.0))]);
    library.step(vec![Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -600.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    }]);
    for _ in 0..8 {
        library.settle();
    }
    let scrolled = top(&library);
    assert!(scrolled < unscrolled - 100.0, "{scrolled} {unscrolled}");

    library.key(CTRL, Key::Tab);
    library.key(CTRL, Key::Tab);
    assert_eq!(top(&library), scrolled);

    // A closed and reopened script starts at the top.
    library.key(CTRL, Key::W);
    library.app.script_action(&library.ctx.clone(), Action::Open(library.id_of("Test")));
    library.settle();
    assert_eq!(top(&library), unscrolled);
}

#[test]
fn a_closed_tab_leaves_no_editor_state_behind() {
    let mut library = Library::with_script("words");
    library.add_script("Two", "more");
    library.click_into_editor();
    let closed = library.editor_id();
    assert!(egui::text_edit::TextEditState::load(&library.ctx, closed).is_some());
    library.key(CTRL, Key::W);
    assert!(egui::text_edit::TextEditState::load(&library.ctx, closed).is_none());
}

#[test]
fn tabs_open_on_a_click_and_close_by_their_button_or_a_middle_click() {
    let mut library = Library::with_script("words");
    library.add_script("Two", "two");
    library.add_script("Three", "three");

    library.click(library.tab_rect(0).center(), PointerButton::Primary);
    assert_eq!(library.active_title(), "Test");

    // The close button shows only with the pointer over the tab.
    library.step(vec![Event::PointerGone]);
    library.settle();
    assert!(library.find_text(ui_kit::icons::regular::X).is_none());
    library.step(vec![Event::PointerMoved(library.tab_rect(1).center())]);
    library.settle();
    let close = library.find_text(ui_kit::icons::regular::X).unwrap();
    assert!(library.tab_rect(1).contains(close.center()));
    library.click(close.center(), PointerButton::Primary);
    assert_eq!(library.tab_titles(), ["Test", "Three"]);
    assert_eq!(library.active_title(), "Test", "closing a tab must not show it");

    library.click(library.tab_rect(1).center(), PointerButton::Middle);
    assert_eq!(library.tab_titles(), ["Test"]);
    library.click(library.tab_rect(0).center(), PointerButton::Middle);
    assert!(library.app.tab_bar.rects.is_empty());
    assert!(library.find_text("Pick a script or create one").is_some());
}

#[test]
fn twelve_tabs_in_the_smallest_window_scroll_the_shown_one_into_view() {
    let mut library = Library::with_script("words");
    library.size = egui::vec2(900.0, 480.0);
    for number in 2..=13 {
        library.add_script(&format!("Script {number}"), "words");
    }
    assert_eq!(library.app.tab_bar.rects.len(), 12);
    let [_, editor, _] = columns(Rect::from_min_size(Pos2::ZERO, library.size).shrink(space::SCALE_8));
    let in_view = |library: &Library, at: usize| {
        let tab = library.tab_rect(at);
        tab.left() >= editor.left() && tab.right() <= editor.right()
    };
    for _ in 0..4 {
        library.settle();
    }
    assert!(in_view(&library, 11), "{:?} {editor:?}", library.tab_rect(11));
    library.key(CTRL, Key::Tab);
    for _ in 0..4 {
        library.settle();
    }
    assert!(in_view(&library, 0), "{:?} {editor:?}", library.tab_rect(0));
}

#[test]
fn only_a_live_tab_shows_the_live_badge() {
    let mut library = Library::with_script("words");
    library.add_script("Two", "two");
    assert!(library.find_text("Live").is_none());
    let id = library.id_of("Test");
    library.app.workspace.as_mut().unwrap().tab_mut(&id).unwrap().live = true;
    library.settle();
    // It stays on the running script's tab with another one shown.
    let badge = library.find_text("Live").unwrap();
    assert!(library.tab_rect(0).contains(badge.center()), "{badge:?}");
    assert_eq!(library.text_rects("Live").len(), 1);
}

#[test]
fn the_star_marks_a_favorite_without_opening_the_script() {
    let mut library = Library::with_script("words");
    library.add_script("Alpha", "a");
    library.add_script("Zulu", "z");
    assert!(library.row("Alpha").top() < library.row("Zulu").top());

    library.click(library.star("Zulu"), PointerButton::Primary);
    library.click(library.star("Test"), PointerButton::Primary);
    assert!(library.is_favorite("Zulu") && library.is_favorite("Test"));
    library.click(library.star("Test"), PointerButton::Primary);
    assert!(!library.is_favorite("Test"));
    assert_eq!(library.active_title(), "Zulu");
    // Favorites come first.
    assert!(library.row("Zulu").top() < library.row("Alpha").top());

    // The row's menu, from the row and from its star.
    library.click(library.row("Alpha").center(), PointerButton::Secondary);
    library.click_text("Add to favorites");
    assert!(library.is_favorite("Alpha"));
    library.click(library.star("Alpha"), PointerButton::Secondary);
    library.click_text("Remove from favorites");
    assert!(!library.is_favorite("Alpha"));
}

#[test]
fn the_favorites_chip_lists_only_favorites() {
    let mut library = Library::with_script("shared words");
    library.add_script("Alpha", "shared words");
    library.app.workspace.as_mut().unwrap().create_folder("Ads").unwrap();
    library.settle();
    // The header has room for the title, the chip and New folder side by side.
    let [title, chip] = ["Scripts", "Favorites"].map(|text| library.row(text));
    let button = format!("{}{}", ui_kit::icons::regular::FOLDER_PLUS, Item::NewFolder.label());
    let button = library.find_text(&button).unwrap();
    assert!(title.right() < chip.left() && chip.right() < button.left() && button.right() < SCRIPTS_WIDTH, "{title:?} {chip:?} {button:?}");
    library.click_text("Favorites");
    assert!(library.find_text("No favorites yet").is_some());
    library.click_text("Favorites");

    library.click(library.star("Alpha"), PointerButton::Primary);
    library.click_text("Favorites");
    assert!(library.in_scripts("Alpha") && !library.in_scripts("Test"));
    assert!(!library.in_scripts("Ads"), "an empty folder is listed under the filter");

    // A search looks only through the favorites.
    library.key(CTRL_SHIFT, Key::F);
    library.type_text("shared");
    assert!(library.in_scripts("Alpha") && !library.in_scripts("Test"));
    library.click_text("Favorites");
    assert!(library.in_scripts("Alpha") && library.in_scripts("Test"));
}

#[test]
fn the_sort_and_closed_folders_are_remembered() {
    let mut library = Library::with_script("words");
    library.add_script("Alpha", "a");
    let workspace = library.app.workspace.as_mut().unwrap();
    workspace.create_folder("Ads").unwrap();
    let id = workspace.open_script().unwrap().id.clone();
    workspace.move_to_folder(&id, Some("Ads")).unwrap();
    library.settle();

    library.click_text("Last edited");
    assert_eq!(library.workspace().store().library().sort, prompter_core::store::Sort::Edited);

    library.click(library.row("Ads").center(), PointerButton::Primary);
    assert_eq!(library.workspace().store().library().collapsed_folders, ["Ads"]);
    for _ in 0..4 {
        library.settle();
    }
    assert!(!library.in_scripts("Alpha"));

    // The app starts again on the same folder, and shows both choices.
    library.restart();
    assert!(!library.in_scripts("Alpha"));
    assert_eq!(library.tab_titles(), ["Test", "Alpha"]);
    assert_eq!(library.active_title(), "Alpha");
    assert_eq!(library.script(), "a");

    // A search shows the match in a closed folder, and the folder stays closed after it.
    library.key(CTRL_SHIFT, Key::F);
    library.type_text("Alpha");
    for _ in 0..4 {
        library.settle();
    }
    assert!(library.in_scripts("Alpha"));
    assert_eq!(library.workspace().store().library().collapsed_folders, ["Ads"]);
    library.app.scripts = ScriptsPanel::default();
    library.settle();

    library.click(library.row("Ads").center(), PointerButton::Primary);
    assert!(library.workspace().store().library().collapsed_folders.is_empty());
}

#[test]
fn files_dropped_on_a_script_row_import_into_its_folder() {
    let mut library = Library::with_script("words");
    library.add_script("Filed", "f");
    let workspace = library.app.workspace.as_mut().unwrap();
    workspace.create_folder("Ads").unwrap();
    let id = workspace.open_script().unwrap().id.clone();
    workspace.move_to_folder(&id, Some("Ads")).unwrap();
    library.settle();
    // Past the title's end, as far across as the row reaches.
    let on_row = |library: &Library, title: &str| egui::pos2(200.0, library.row(title).center().y);

    let at = on_row(&library, "Filed");
    library.hovered = vec![egui::HoveredFile::default()];
    library.app.drag_pointer_at = Some(at);
    library.settle();
    let mark = library.drop_mark().unwrap();
    assert!(mark.contains(at) && mark.height() < 40.0, "{mark:?}");
    library.drop_files(&[library.outside_file("first.md", "one")], at);
    assert!(library.filed().contains(&("first".to_owned(), Some("Ads".to_owned()))), "{:?}", library.filed());

    // An unfiled script's row, then the Unfiled heading.
    for (name, row) in [("second", "Test"), ("third", "Unfiled")] {
        let file = [library.outside_file(&format!("{name}.md"), "two")];
        let at = on_row(&library, row);
        library.drop_files(&file, at);
        assert!(library.filed().contains(&(name.to_owned(), None)), "{row}: {:?}", library.filed());
    }
}

#[test]
fn ctrl_v_with_html_and_no_text_pastes_on_the_release_of_v() {
    let mut library = Library::with_script("");
    library.focus_editor();
    library.put_on_clipboard(Some("<b>only</b>"), None);
    library.press_paste(CTRL);
    assert_eq!(library.script(), "**only**");
    // One undo step, like any paste.
    library.key(CTRL, Key::Z);
    assert_eq!(library.script(), "");

    // Ctrl let go before V.
    library.step_paste(CTRL);
    library.step(vec![Event::ModifiersChanged(Modifiers::NONE)]);
    library.step(vec![release(Key::V, Modifiers::NONE)]);
    library.settle();
    assert_eq!(library.script(), "**only**");

    // Twice with Ctrl held throughout.
    library.step_paste(CTRL);
    library.step(vec![release(Key::V, CTRL)]);
    library.step(vec![release(Key::V, CTRL)]);
    library.step(vec![Event::ModifiersChanged(Modifiers::NONE)]);
    library.settle();
    assert_eq!(library.script(), "**only****only****only**");

    // A plain-text paste has nothing to take.
    library.press_paste(CTRL_SHIFT);
    assert_eq!(library.script(), "**only****only****only**");
}

#[test]
fn the_release_of_v_pastes_only_for_a_press_that_sent_nothing() {
    let mut library = Library::with_script("");
    library.focus_editor();
    library.put_on_clipboard(Some("<b>html</b>"), Some("html"));
    // With text too, the window's paste arrives, and the release adds nothing.
    library.press_paste(CTRL);
    assert_eq!(library.script(), "**html**");

    // A typed v, with Ctrl down before it is let go.
    library.put_on_clipboard(Some("<b>only</b>"), None);
    let press = Event::Key { key: Key::V, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    library.step(vec![press, Event::Text("v".to_owned())]);
    library.step(vec![Event::ModifiersChanged(CTRL)]);
    library.step(vec![release(Key::V, CTRL)]);
    library.step(vec![Event::ModifiersChanged(Modifiers::NONE)]);
    library.settle();
    assert_eq!(library.script(), "**html**v");

    // A release with Ctrl not held since the window came back.
    library.step(vec![Event::ModifiersChanged(CTRL), Event::WindowFocused(false), Event::ModifiersChanged(Modifiers::NONE)]);
    library.step(vec![Event::WindowFocused(true), release(Key::V, Modifiers::NONE)]);
    library.settle();
    library.focus_editor();
    assert_eq!(library.script(), "**html**v");

    // Shift+Insert pastes without a V, and the next Ctrl+V still works.
    library.step(vec![Event::Paste("x".to_owned())]);
    library.settle();
    library.press_paste(CTRL);
    assert_eq!(library.script(), "**html**v**only****only**");

    // In the find field, the release does not paste into the script.
    library.key(CTRL, Key::F);
    library.press_paste(CTRL);
    assert_eq!(library.script(), "**html**v**only****only**");
}

#[test]
fn a_dragged_script_row_moves_to_the_folder_header_or_unfiled_it_is_dropped_on() {
    let mut library = Library::with_folders();
    let [from, ads] = ["Alpha", "Ads"].map(|text| library.row(text).center());
    library.hold(from);

    // Over the editor there is nowhere to drop.
    library.move_to(egui::pos2(500.0, 300.0));
    assert_eq!(library.cursor, egui::CursorIcon::NotAllowed);
    assert_eq!(library.drop_mark(), None);
    // Nor on a script's row, or on the header of the folder it is in.
    for text in ["Beta", "Unfiled"] {
        library.move_to(library.row(text).center());
        assert_eq!((library.cursor, library.drop_mark()), (egui::CursorIcon::NotAllowed, None), "{text}");
    }

    // The header lights up across the panel, and the ghost follows beside the pointer.
    let on_ads = egui::pos2(200.0, ads.y);
    library.move_to(on_ads);
    assert_eq!(library.cursor, egui::CursorIcon::Grabbing);
    let mark = library.drop_mark().unwrap();
    assert!(mark.contains(ads) && mark.contains(on_ads) && mark.height() < 40.0, "{mark:?}");
    let beside = Rect::from_min_size(on_ads, egui::vec2(60.0, 40.0));
    assert!(library.find_text_in("Alpha", beside).is_some(), "no ghost");
    assert_eq!(library.folder_of("Alpha"), None, "moved before the drop");

    library.let_go(on_ads);
    assert_eq!(library.folder_of("Alpha").as_deref(), Some("Ads"));
    assert_eq!((library.cursor, library.drop_mark()), (egui::CursorIcon::Default, None));
    assert_eq!(library.active_title(), "Test", "a drag opened its script");

    library.drag_row("Alpha", "Unfiled");
    assert_eq!(library.folder_of("Alpha"), None);
}

#[test]
fn a_press_that_moves_less_than_four_px_is_a_click() {
    let mut library = Library::with_folders();
    let at = library.row("Alpha").center();
    library.hold(at);
    library.move_to(at + egui::vec2(3.0, 0.0));
    assert_eq!(library.cursor, egui::CursorIcon::Default);
    library.let_go(at + egui::vec2(3.0, 0.0));
    assert_eq!(library.active_title(), "Alpha");

    // A long press is still one. egui alone would drop it.
    let at = library.row("Beta").center();
    library.hold(at);
    for _ in 0..12 {
        library.step(Vec::new());
    }
    library.let_go(at);
    assert_eq!(library.active_title(), "Beta");

    // From 4 px it is a drag, though egui would still call 5 px a click.
    let at = library.row("Test").center();
    library.hold(at);
    library.move_to(at + egui::vec2(5.0, 0.0));
    assert_eq!(library.cursor, egui::CursorIcon::NotAllowed);
    library.let_go(at + egui::vec2(5.0, 0.0));
    assert_eq!(library.active_title(), "Beta");

    // The same on a folder header, which a click closes.
    let at = library.row("Ads").center();
    library.hold(at);
    library.move_to(at + egui::vec2(5.0, 0.0));
    library.let_go(at + egui::vec2(5.0, 0.0));
    assert!(library.workspace().store().library().collapsed_folders.is_empty());
    library.hold(at);
    library.move_to(at + egui::vec2(3.0, 0.0));
    library.let_go(at + egui::vec2(3.0, 0.0));
    assert_eq!(library.workspace().store().library().collapsed_folders, ["Ads"]);
}

#[test]
fn esc_cancels_a_drag_and_does_nothing_else() {
    let mut library = Library::with_folders();
    library.key(CTRL, Key::F);
    let [from, ads] = ["Alpha", "Ads"].map(|text| library.row(text).center());
    library.hold(from);
    library.move_to(ads);
    assert!(library.drop_mark().is_some());
    library.key(Modifiers::NONE, Key::Escape);
    assert_eq!((library.cursor, library.drop_mark()), (egui::CursorIcon::Default, None));
    assert!(library.app.find.is_open(), "Esc went past the drag");
    // With the button still down, moving does not start again.
    library.move_to(ads + egui::vec2(20.0, 0.0));
    assert_eq!(library.drop_mark(), None);
    library.let_go(ads);
    assert_eq!(library.folder_of("Alpha"), None);
    assert!(library.workspace().store().library().collapsed_folders.is_empty());

    // Back on the pressed row, the release is no click.
    library.hold(from);
    library.move_to(ads);
    library.key(Modifiers::NONE, Key::Escape);
    library.move_to(from);
    library.let_go(from);
    assert_eq!(library.active_title(), "Test");

    // The next drag works.
    library.drag_row("Alpha", "Ads");
    assert_eq!(library.folder_of("Alpha").as_deref(), Some("Ads"));
}

#[test]
fn a_drag_ends_without_a_drop_when_the_window_loses_focus_or_a_dialog_opens() {
    let mut library = Library::with_folders();
    let [from, ads] = ["Alpha", "Ads"].map(|text| library.row(text).center());
    library.hold(from);
    library.move_to(ads);
    library.step(vec![Event::WindowFocused(false)]);
    library.step(vec![Event::WindowFocused(true)]);
    library.let_go(ads);
    assert_eq!(library.folder_of("Alpha"), None);

    library.hold(from);
    library.move_to(ads);
    library.app.about_open = true;
    library.settle();
    assert_eq!((library.cursor, library.drop_mark()), (egui::CursorIcon::Default, None));
    library.let_go(ads);
    assert_eq!(library.folder_of("Alpha"), None);
}

#[test]
fn selected_scripts_drag_together() {
    let mut library = Library::with_folders();
    let row = |library: &Library, title: &str| library.row(title).center();
    library.click_holding(row(&library, "Beta"), CTRL);
    assert_eq!(library.active_title(), "Test", "Ctrl+click opened a script");

    // "Test" is open, so it is selected with "Beta".
    library.hold(row(&library, "Beta"));
    library.move_to(row(&library, "Ads"));
    assert!(library.find_text("2 scripts").is_some());
    library.let_go(row(&library, "Ads"));
    let ads = Some("Ads".to_owned());
    assert_eq!(library.filed(), [("Alpha".to_owned(), None), ("Beta".to_owned(), ads.clone()), ("Test".to_owned(), ads.clone())]);

    // Listed now: Alpha under Unfiled, then Beta and Test under Ads. A Ctrl+click on Alpha
    // anchors there, and a Shift+click on Beta takes the range.
    library.click_holding(row(&library, "Alpha"), CTRL);
    library.click_holding(row(&library, "Beta"), Modifiers::SHIFT);
    assert_eq!(library.active_title(), "Test", "Shift+click opened a script");
    library.drag_row("Alpha", "Clips");
    let clips = Some("Clips".to_owned());
    assert_eq!(library.filed(), [("Alpha".to_owned(), clips.clone()), ("Beta".to_owned(), clips.clone()), ("Test".to_owned(), ads)]);

    // A row outside the selection drags alone.
    library.drag_row("Test", "Unfiled");
    assert_eq!(library.folder_of("Test"), None);
    assert_eq!(library.folder_of("Beta"), clips);

    // A plain click goes back to one.
    library.click_holding(row(&library, "Alpha"), CTRL);
    library.click(row(&library, "Beta"), PointerButton::Primary);
    assert_eq!(library.active_title(), "Beta");
    library.drag_row("Beta", "Ads");
    assert_eq!((library.folder_of("Beta").as_deref(), library.folder_of("Alpha")), (Some("Ads"), clips));
}

#[test]
fn the_row_menu_moves_the_whole_selection_or_the_one_row_outside_it() {
    let mut library = Library::with_folders();
    library.click_holding(library.row("Beta").center(), CTRL);
    library.click(library.row("Test").center(), PointerButton::Secondary);
    library.click_text("Move 2 scripts to folder");
    // The submenu's "Ads" is painted over the folder's header.
    library.click_text("Ads");
    assert_eq!((library.folder_of("Test").as_deref(), library.folder_of("Beta").as_deref()), (Some("Ads"), Some("Ads")));
    assert_eq!(library.folder_of("Alpha"), None);

    library.click(library.row("Alpha").center(), PointerButton::Secondary);
    assert!(library.find_text("Move to folder").is_some());
}

#[test]
fn a_dragged_folder_header_reorders_the_folders() {
    let mut library = Library::with_folders();
    library.app.workspace.as_mut().unwrap().create_folder("Bits").unwrap();
    library.settle();
    // The panel keeps the library's order, with Unfiled first.
    let tops = ["Unfiled", "Ads", "Clips", "Bits"].map(|name| library.row(name).top());
    assert!(tops.is_sorted(), "{tops:?}");

    let [bits, ads, unfiled] = ["Bits", "Ads", "Unfiled"].map(|name| library.row(name));
    library.hold(bits.center());
    // Unfiled stays first.
    library.move_to(unfiled.center());
    assert_eq!((library.cursor, library.insertion_line()), (egui::CursorIcon::NotAllowed, None));
    let target = ads.center() - egui::vec2(0.0, 3.0);
    library.move_to(target);
    assert_eq!(library.cursor, egui::CursorIcon::Grabbing);
    let line = library.insertion_line().unwrap();
    assert!(line > unfiled.bottom() && line < ads.top(), "{line} {unfiled:?} {ads:?}");
    assert_eq!(library.drop_mark(), None);
    library.let_go(target);
    assert_eq!(library.folders(), ["Bits", "Ads", "Clips"]);
    assert_eq!(library.insertion_line(), None);

    // Below the last one.
    let from = library.row("Bits").center();
    library.hold(from);
    library.move_to(egui::pos2(100.0, 600.0));
    library.let_go(egui::pos2(100.0, 600.0));
    assert_eq!(library.folders(), ["Ads", "Clips", "Bits"]);
    assert!(library.workspace().store().library().collapsed_folders.is_empty());

    // The favorites filter hides folders, so their order is left alone.
    library.click(library.star("Alpha"), PointerButton::Primary);
    library.drag_row("Alpha", "Ads");
    library.click(library.star("Beta"), PointerButton::Primary);
    library.drag_row("Beta", "Clips");
    library.click_text("Favorites");
    library.drag_row("Clips", "Ads");
    assert_eq!(library.folders(), ["Ads", "Clips", "Bits"]);
}

#[test]
fn the_keys_alone_move_a_script_through_its_row_menu() {
    let mut library = Library::with_folders();
    library.tab_to(library.row("Alpha").center());
    library.key(Modifiers::SHIFT, Key::F10);
    assert!(library.find_text("Move to folder").is_some(), "Shift+F10 opened no menu");
    // Past the favorite, Rename and Duplicate items, then into the submenu.
    for _ in 0..3 {
        library.key(Modifiers::NONE, Key::ArrowDown);
    }
    library.key(Modifiers::NONE, Key::Enter);
    // Unfiled is off, so the first folder has the keyboard. Down is the second.
    library.key(Modifiers::NONE, Key::ArrowDown);
    library.key(Modifiers::NONE, Key::Enter);
    assert_eq!(library.folder_of("Alpha").as_deref(), Some("Clips"));
    assert!(library.find_text("Move to folder").is_none(), "the menu is still open");
}

#[test]
fn the_keys_alone_reorder_folders_through_the_header_menu() {
    let mut library = Library::with_folders();
    library.tab_to(library.row("Ads").center());
    library.key(Modifiers::SHIFT, Key::F10);
    assert!(library.find_text("Move down").is_some(), "Shift+F10 opened no menu");
    // From Rename past Move up, which is off for the first folder.
    library.key(Modifiers::NONE, Key::ArrowDown);
    library.key(Modifiers::NONE, Key::Enter);
    assert_eq!(library.folders(), ["Clips", "Ads"]);

    // The pointer's menu has them too.
    library.click(library.row("Ads").center(), PointerButton::Secondary);
    library.click_text("Move up");
    assert_eq!(library.folders(), ["Ads", "Clips"]);
}

#[test]
fn the_tab_menu_items_do_what_the_tab_keys_do() {
    let mut library = Library::with_script("words");
    library.add_script("Two", "two");
    library.add_script("Three", "three");
    library.pick("View", Item::NextTab);
    assert_eq!(library.active_title(), "Test");
    library.pick("View", Item::PreviousTab);
    assert_eq!(library.active_title(), "Three");
    library.pick("File", Item::CloseTab);
    assert_eq!(library.tab_titles(), ["Test", "Two"]);
    assert_eq!(library.active_title(), "Two");
    library.pick("File", Item::CloseTab);
    assert!(!library.app.menu_state().several_tabs);
    assert!(!Item::NextTab.enabled(library.app.menu_state()));
}

/// Where the editor drew its text, and the layout it drew.
fn editor_galley(library: &Library) -> (Pos2, Arc<egui::Galley>) {
    library
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Text(painted) if painted.galley.text() == library.script() => Some((painted.pos, Arc::clone(&painted.galley))),
            _ => None,
        })
        .unwrap()
}

/// The middle of the left edge of the character at `index` as the editor shows it now.
fn character_at(library: &Library, index: usize) -> Pos2 {
    let (pos, galley) = editor_galley(library);
    let cursor = galley.pos_from_cursor(egui::text::CCursor::new(egui::text::CharIndex(index)));
    pos + cursor.left_center().to_vec2()
}

/// The editor's selection, from where it started to the cursor.
fn selection(library: &Library) -> (usize, usize) {
    let state = egui::text_edit::TextEditState::load(&library.ctx, library.editor_id()).unwrap();
    let range = state.cursor.char_range().unwrap();
    (range.secondary.index.0, range.primary.index.0)
}

/// True if the syntax that starts at byte `at` is drawn see-through.
fn is_hidden(library: &Library, at: usize) -> bool {
    let (_, galley) = editor_galley(library);
    let section = galley.job.sections.iter().find(|section| section.byte_range.contains(&egui::text::ByteIndex(at))).unwrap();
    section.format.color == egui::Color32::TRANSPARENT
}

/// Two lines of ASCII, whose second starts with syntax at 16.
const TWO_LINES: &str = "**first** line\n\n**second** line";

#[test]
fn syntax_shows_on_the_line_the_cursor_moves_to() {
    let mut library = Library::with_script(TWO_LINES);
    assert!(!is_hidden(&library, 0) && is_hidden(&library, 16), "the cursor starts on the first line");
    let at = character_at(&library, 4);
    library.click(at, PointerButton::Primary);
    assert!(!is_hidden(&library, 0) && is_hidden(&library, 16));
    library.key(Modifiers::NONE, Key::ArrowDown);
    library.key(Modifiers::NONE, Key::ArrowDown);
    assert!(is_hidden(&library, 0) && !is_hidden(&library, 16));
}

#[test]
fn a_click_on_formatted_text_lands_on_that_character_and_selects_nothing() {
    let mut library = Library::with_script(TWO_LINES);
    // The "c" of "second", past the hidden syntax.
    let index = TWO_LINES.find("cond").unwrap();
    let at = character_at(&library, index);
    library.hold(at);
    // Held still, the layout stays as it was, so no selection is dragged out.
    for _ in 0..5 {
        library.step(Vec::new());
    }
    assert!(is_hidden(&library, 16), "the syntax must wait for the button to be let go");
    assert_eq!(selection(&library), (index, index));
    library.let_go(at);
    assert!(!is_hidden(&library, 16));
    assert_eq!(selection(&library), (index, index));
}

#[test]
fn a_drag_selects_the_real_text_under_it() {
    let mut library = Library::with_script(TWO_LINES);
    let (from, to) = (TWO_LINES.find("rst").unwrap(), TWO_LINES.find("cond").unwrap());
    let (start, end) = (character_at(&library, from), character_at(&library, to));
    library.hold(start);
    library.move_to(end);
    library.let_go(end);
    assert_eq!(selection(&library), (from, to));
    // The window reports Ctrl+C as a copy.
    library.step(vec![Event::Copy]);
    assert_eq!(library.copied(), TWO_LINES[from..to]);
}

/// Clicks `times` in a row at `at`, with a frame or more between the events, as a hand
/// would.
fn click_times(library: &mut Library, at: Pos2, times: usize) {
    let event = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    library.step(vec![Event::PointerMoved(at)]);
    for pressed in [true, false].repeat(times) {
        // Frames 10 ms apart, so that the clicks are close enough in time to count as one run.
        for events in [vec![event(pressed)], Vec::new(), Vec::new()] {
            library.time -= 0.09;
            library.step(events);
        }
    }
    library.settle();
}

#[test]
fn a_double_click_selects_the_word_aimed_at_on_a_line_that_then_shows_its_syntax() {
    let cases = [
        (
            "first

## Opening the big show today",
            "big",
        ),
        (
            "first

some **bold words** and more text here",
            "more",
        ),
        (
            "first

some **bold words** and more text here",
            "text",
        ),
        (
            "first

> one two three four",
            "two",
        ),
    ];
    for (text, word) in cases {
        let mut library = Library::with_script(text);
        let from = text.find(word).unwrap();
        let (start, end) = (character_at(&library, from), character_at(&library, from + word.len()));
        click_times(&mut library, start + (end - start) / 2.0, 2);
        assert_eq!(selection(&library), (from, from + word.len()), "{word} in {text:?}");
        let first_syntax = text.find(['#', '*', '>']).unwrap();
        assert!(!is_hidden(&library, first_syntax), "the line shows its syntax afterwards");
    }
}

#[test]
fn a_later_click_in_the_same_place_uses_the_layout_on_screen() {
    let mut library = Library::with_script(TWO_LINES);
    let index = TWO_LINES.find("cond").unwrap();
    let at = character_at(&library, index);
    library.click(at, PointerButton::Primary);
    // The line now shows its syntax, so the same place holds an earlier character.
    let moved = (0..TWO_LINES.len()).min_by_key(|&n| (character_at(&library, n).distance(at) * 100.0) as u32).unwrap();
    assert!(moved < index);
    for _ in 0..5 {
        library.step(Vec::new());
    }
    library.click(at, PointerButton::Primary);
    assert_eq!(selection(&library), (moved, moved));
}

#[test]
fn typing_markdown_formats_it_and_keeps_the_text() {
    let mut library = Library::with_script("");
    library.focus_editor();
    library.type_text("# T");
    library.key(Modifiers::NONE, Key::Enter);
    library.type_text("[[pause]]");
    assert_eq!(library.script(), "# T\n[[pause]]");
    assert!(is_hidden(&library, 0), "the cursor has left the heading");
    assert!(!is_hidden(&library, 4));
}

#[test]
fn show_markdown_in_the_view_menu_shows_every_marker_and_is_remembered() {
    let mut library = Library::with_script(TWO_LINES);
    library.pick("View", Item::ShowMarkdown);
    assert!(library.app.menu_state().show_markdown);
    assert!(!is_hidden(&library, 0) && !is_hidden(&library, 16));
    library.restart();
    assert!(library.app.menu_state().show_markdown);
    assert!(!is_hidden(&library, 0) && !is_hidden(&library, 16));
    library.pick("View", Item::ShowMarkdown);
    library.restart();
    assert!(!library.app.menu_state().show_markdown);
    // The first line has the caret, as at any launch.
    assert!(!is_hidden(&library, 0) && is_hidden(&library, 16));
}

/// About 10,000 words with each Markdown and cue form the editor styles.
fn long_script() -> String {
    let words = "the quick brown fox jumps over a lazy dog while the camera keeps rolling on";
    let sentence: Vec<&str> = words.split(' ').collect();
    let line = |count: usize| sentence.iter().cycle().take(count).copied().collect::<Vec<_>>().join(" ");
    let mut text = String::new();
    for section in 1..=50 {
        text.push_str(&format!("## Section {section}\n\n"));
        text.push_str(&format!("{} **{}** {} *{}* [[pause]] {}.\n\n", line(20), line(3), line(20), line(3), line(14)));
        text.push_str(&format!("- {}\n- {} [[mark: B-roll: desk]]\n- {}\n\n", line(10), line(10), line(10)));
        text.push_str(&format!("> {}\n\n[[note: wave at the camera]]\n\n{}.\n\n", line(20), line(80)));
    }
    text.push_str("[[end]]\n");
    text
}

/// Measures the time of a whole Library frame, layout and tessellation, while typing in the
/// middle of a long script. Run it in release:
/// `cargo test -p cutmark-prompter --release -q typing_time -- --ignored --nocapture`
#[test]
#[ignore = "a measurement, run by hand in release"]
fn typing_time_on_10_000_words() {
    let text = long_script();
    let words = text.split_whitespace().count();
    let mut library = Library::with_script(&text);
    library.focus_editor();
    let id = library.editor_id();
    // The cursor goes to the end of a paragraph near the middle.
    let middle = text.find("## Section 26").unwrap() - 2;
    let at = egui::text::CCursor::new(egui::text::CharIndex(text[..middle].chars().count()));
    let mut state = egui::text_edit::TextEditState::load(&library.ctx, id).unwrap();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::one(at)));
    state.store(&library.ctx, id);
    library.settle();

    let mut timed = |events: &mut dyn FnMut(usize) -> Vec<Event>| {
        let mut times: Vec<Duration> = (0..300)
            .map(|n| {
                let events = events(n);
                let start = Instant::now();
                library.step(events);
                let primitives = library.ctx.tessellate(std::mem::take(&mut library.shapes), 1.0);
                let time = start.elapsed();
                assert!(!primitives.is_empty());
                time
            })
            .collect();
        times.sort();
        let ms = |time: Duration| time.as_secs_f64() * 1000.0;
        let mean = times.iter().sum::<Duration>() / times.len() as u32;
        format!("mean {:.2} ms, median {:.2} ms, p95 {:.2} ms, worst {:.2} ms", ms(mean), ms(times[150]), ms(times[285]), ms(times[299]))
    };
    let typing = timed(&mut |n| vec![Event::Text(if n % 6 == 5 { " " } else { "a" }.to_owned())]);
    let idle = timed(&mut |_| Vec::new());
    println!("{words} words, {} bytes\ntyping: {typing}\nidle:   {idle}", text.len());
    assert!(library.script().len() > text.len());
}

/// The part of the window the editor panel takes.
fn editor_panel(library: &Library) -> Rect {
    let mut body = Rect::from_min_size(Pos2::ZERO, library.size);
    body.min.y += size::TOOLBAR_MAIN_HEIGHT;
    body.max.y -= size::STATUS_BAR_HEIGHT;
    columns(body.shrink(space::SCALE_8))[1]
}

/// Every filled or outlined rectangle the last frame drew.
fn rects(library: &Library) -> Vec<egui::epaint::RectShape> {
    fn search(shape: &egui::Shape, found: &mut Vec<egui::epaint::RectShape>) {
        match shape {
            egui::Shape::Rect(rect) => found.push(rect.clone()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| search(shape, found)),
            _ => {}
        }
    }
    let mut found = Vec::new();
    library.shapes.iter().for_each(|clipped| search(&clipped.shape, &mut found));
    found
}

/// True if a frame in the next blink of the caret draws it, thin and in the accent color.
fn caret_shows(library: &mut Library) -> bool {
    let accent = ui_kit::Theme::get(&library.ctx).accent.accent;
    (0..12).any(|_| {
        library.step(Vec::new());
        library.shapes.iter().any(|clipped| {
            matches!(&clipped.shape, egui::Shape::LineSegment { points, stroke }
                if stroke.width == editor::CARET_WIDTH && stroke.color == accent && points[0].x == points[1].x)
        })
    })
}

/// True if no box is drawn around the editor: no large field fill and no accent outline.
fn is_unboxed(library: &Library) -> bool {
    let panel = editor_panel(library);
    let accent = ui_kit::Theme::get(&library.ctx).accent.accent;
    !rects(library).iter().any(|shape| {
        let inside = panel.contains_rect(shape.rect) && shape.rect.height() > 200.0;
        inside && (shape.fill == ui_kit::tokens::color::BG_CONTROL || shape.stroke.color == accent && !shape.stroke.is_empty())
    })
}

#[test]
fn a_script_at_launch_has_no_box_and_a_thin_accent_caret_to_type_at() {
    let mut library = Library::with_script("# Title\n\nwords");
    // As at the next launch: the tab comes back from the disk, and nothing is clicked.
    library.restart();

    let id = library.editor_id();
    assert_eq!(library.focused(), Some(id));
    assert_eq!(selection(&library), (0, 0));
    assert!(is_unboxed(&library));
    assert!(caret_shows(&mut library));

    library.ctx.memory_mut(|memory| memory.surrender_focus(id));
    library.settle();
    assert!(is_unboxed(&library));
    assert!(!caret_shows(&mut library));
}

#[test]
fn the_text_is_a_centered_column_no_wider_than_760_px_with_48_px_above() {
    let mut library = Library::with_script(&"word ".repeat(400));
    for width in [900.0, 1100.0, 2200.0] {
        library.size = egui::vec2(width, 720.0);
        library.settle();
        let panel = editor_panel(&library);
        let (pos, galley) = editor_galley(&library);
        let wrap = galley.job.wrap.max_width;
        assert!(wrap <= editor::COLUMN_WIDTH, "{width}: {wrap}");
        let (left, right) = (pos.x - panel.left(), panel.right() - (pos.x + wrap));
        assert!((left - right).abs() <= 3.0, "{width}: {left} at the left, {right} at the right");
        if width > 2000.0 {
            assert!(wrap > editor::COLUMN_WIDTH - 3.0, "{wrap}");
        }
    }
    // The toolbar is one row in a panel this wide.
    let toolbar_bottom = library.find_text(ui_kit::icons::regular::TEXT_B).unwrap().center().y + size::PANEL_TOOLBAR_HEIGHT / 2.0;
    let (pos, _) = editor_galley(&library);
    assert!((pos.y - toolbar_bottom - editor::TOP_PAD).abs() <= 6.0, "{} below the toolbar", pos.y - toolbar_bottom);
}

#[test]
fn an_empty_script_shows_the_placeholder_until_something_is_typed() {
    let mut library = Library::with_script("");
    let placeholder = |library: &Library| {
        library.shapes.iter().find_map(|clipped| match &clipped.shape {
            egui::Shape::Text(shape) if shape.galley.text() == editor::PLACEHOLDER => Some(shape.galley.job.sections[0].format.clone()),
            _ => None,
        })
    };
    let format = placeholder(&library).unwrap();
    assert!(format.italics);
    assert_eq!(format.color, ui_kit::tokens::color::TEXT_TERTIARY);
    assert_eq!(editor::PLACEHOLDER, "Enter text or type '/' for commands");
    library.focus_editor();
    assert!(placeholder(&library).is_some(), "it stays while the editor has the keyboard");
    library.type_text("a");
    assert!(placeholder(&library).is_none());
}

#[test]
fn a_quote_bar_is_drawn_left_of_the_column_inside_the_panel() {
    let library = Library::with_script("> quoted");
    let (pos, _) = editor_galley(&library);
    let bar = rects(&library).into_iter().find(|shape| shape.fill == ui_kit::tokens::color::TEXT_TERTIARY && shape.rect.width() == 2.0).unwrap();
    assert!(editor_panel(&library).contains_rect(bar.rect));
    assert!(bar.rect.right() < pos.x, "{:?} and the text at {pos:?}", bar.rect);
}

/// Selects characters `from` to `to` in the editor, with the cursor at `to`.
fn select_in_editor(library: &mut Library, from: usize, to: usize) {
    library.focus_editor();
    let id = library.editor_id();
    let cursor = |at: usize| egui::text::CCursor::new(egui::text::CharIndex(at));
    let mut state = egui::text_edit::TextEditState::load(&library.ctx, id).unwrap();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(cursor(from), cursor(to))));
    state.store(&library.ctx, id);
    library.settle();
}

/// The shortcut of a format item, as the window reports it.
fn press_format(library: &mut Library, item: Item) {
    match item.chord() {
        "Ctrl+Shift+X" => {
            // egui-winit sends this one as a cut, with no key.
            library.step(vec![Event::ModifiersChanged(CTRL_SHIFT), Event::Cut]);
            library.step(vec![Event::ModifiersChanged(Modifiers::NONE)]);
            library.settle();
        }
        "Ctrl+B" => library.key(CTRL, Key::B),
        "Ctrl+I" => library.key(CTRL, Key::I),
        "Ctrl+U" => library.key(CTRL, Key::U),
        "Ctrl+0" => library.key(CTRL, Key::Num0),
        "Ctrl+1" => library.key(CTRL, Key::Num1),
        "Ctrl+2" => library.key(CTRL, Key::Num2),
        "Ctrl+3" => library.key(CTRL, Key::Num3),
        "Ctrl+Shift+8" => library.key(CTRL_SHIFT, Key::Num8),
        "Ctrl+Shift+7" => library.key(CTRL_SHIFT, Key::Num7),
        "Ctrl+Shift+." => library.key(CTRL_SHIFT, Key::Period),
        "Ctrl+]" => library.key(CTRL, Key::CloseBracket),
        "Ctrl+[" => library.key(CTRL, Key::OpenBracket),
        chord => panic!("no keys for {chord:?}"),
    }
}

#[test]
fn every_format_shortcut_changes_the_script_as_one_undo_step() {
    use crate::formatting::{Action as A, BlockStyle as S, ListKind as L};
    use crate::markup::Inline as I;
    // Each starts on the second item of a list, with its word selected.
    let start = "- one\n- two";
    let cases = [
        (A::Inline(I::Bold), "- one\n- **two**"),
        (A::Inline(I::Italic), "- one\n- *two*"),
        (A::Inline(I::Underline), "- one\n- <u>two</u>"),
        (A::Inline(I::Strike), "- one\n- ~~two~~"),
        (A::Style(S::Heading(1)), "- one\n# two"),
        (A::Style(S::Heading(2)), "- one\n## two"),
        (A::Style(S::Heading(3)), "- one\n### two"),
        (A::Style(S::Quote), "- one\n> two"),
        (A::Style(S::Paragraph), "- one\n\ntwo"),
        (A::List(L::Bulleted), "- one\n\ntwo"),
        (A::List(L::Numbered), "- one\n1. two"),
        (A::Indent, "- one\n  - two"),
    ];
    for (action, wanted) in cases {
        let mut library = Library::with_script(start);
        select_in_editor(&mut library, 8, 11);
        // The undo history takes a point once the text has rested.
        library.time += 2.0;
        library.settle();
        press_format(&mut library, Item::Format(action));
        assert_eq!(library.script(), wanted, "{action:?}");
        assert_eq!(library.focused(), Some(library.editor_id()), "{action:?}");
        library.key(CTRL, Key::Z);
        assert_eq!(library.script(), start, "{action:?} is not one undo step");
        library.key(CTRL, Key::Y);
        assert_eq!(library.script(), wanted, "{action:?}");
    }
}

/// Checks that one undo takes back `change`, made with no rest after the text was typed,
/// and leaves what was typed.
fn undoes_in_one_step(start: &str, name: &str, change: impl FnOnce(&mut Library)) {
    let mut library = Library::with_script(start);
    let end = start.chars().count();
    select_in_editor(&mut library, end, end);
    library.type_text("x");
    let typed = format!("{start}x");
    assert_eq!(library.script(), typed, "{name}");
    change(&mut library);
    let changed = library.script().to_owned();
    assert_ne!(changed, typed, "{name} did nothing");
    library.key(CTRL, Key::Z);
    assert_eq!(library.script(), typed, "{name} is not one undo step");
    library.key(CTRL, Key::Y);
    assert_eq!(library.script(), changed, "{name}");
}

#[test]
fn actions_right_after_typing_are_their_own_undo_step() {
    use crate::formatting::Action;
    use crate::markup::CueKind;
    let press = |action| move |library: &mut Library| press_format(library, Item::Format(action));
    undoes_in_one_step("- one\n  - two", "outdent", press(Action::Outdent));
    undoes_in_one_step("one", "bold", press(Action::Inline(Inline::Bold)));
    undoes_in_one_step("- one\n- two", "tab", |library| library.key(Modifiers::NONE, Key::Tab));
    undoes_in_one_step("- one\n  - two", "shift tab", |library| library.key(Modifiers::SHIFT, Key::Tab));
    undoes_in_one_step("one", "rule", |library| library.click_text(ui_kit::icons::regular::MINUS));
    for kind in [CueKind::Note, CueKind::Pause, CueKind::Mark, CueKind::End] {
        let action = Action::Cue(kind);
        undoes_in_one_step("one", action.label(), |library| library.pick("Format", Item::Format(action)));
    }
}

#[test]
fn ctrl_bracket_takes_a_nested_item_out() {
    let mut library = Library::with_script("- one\n  - two");
    select_in_editor(&mut library, 13, 13);
    press_format(&mut library, Item::Format(crate::formatting::Action::Outdent));
    assert_eq!(library.script(), "- one\n- two");
}

#[test]
fn tab_and_shift_tab_nest_list_items_and_stay_in_the_editor() {
    let mut library = Library::with_script("- one\n- two");
    select_in_editor(&mut library, 11, 11);
    library.key(Modifiers::NONE, Key::Tab);
    assert_eq!(library.script(), "- one\n  - two");
    assert_eq!(library.focused(), Some(library.editor_id()));
    // With nowhere further to go, Tab does nothing: no tab character, and no move away.
    library.key(Modifiers::NONE, Key::Tab);
    assert_eq!(library.script(), "- one\n  - two");
    assert_eq!(library.focused(), Some(library.editor_id()));
    library.key(Modifiers::SHIFT, Key::Tab);
    assert_eq!(library.script(), "- one\n- two");
    library.key(Modifiers::SHIFT, Key::Tab);
    assert_eq!(library.script(), "- one\n- two");
    assert_eq!(library.focused(), Some(library.editor_id()));
}

#[test]
fn tab_outside_a_list_moves_the_keyboard_on_as_before() {
    let mut library = Library::with_script("words");
    select_in_editor(&mut library, 5, 5);
    library.key(Modifiers::NONE, Key::Tab);
    assert_eq!(library.script(), "words");
    assert_ne!(library.focused(), Some(library.editor_id()));
}

#[test]
fn the_format_keys_are_left_alone_in_other_fields() {
    let mut library = Library::with_script("words");
    library.key(CTRL, Key::F);
    library.type_text("wo");
    library.key(CTRL, Key::B);
    assert_eq!(library.script(), "words");
}

#[test]
fn a_toolbar_button_formats_the_selection_and_then_shows_as_on() {
    let mut library = Library::with_script("one two");
    select_in_editor(&mut library, 4, 7);
    let glyph = ui_kit::icons::regular::TEXT_B;
    let lit = |library: &Library| {
        let at = library.find_text(glyph).unwrap().center();
        let accent = ui_kit::Theme::get(&library.ctx).accent.accent;
        library.shapes.iter().any(
            |clipped| matches!(&clipped.shape, egui::Shape::Circle(circle) if circle.center.distance(at) < 3.0 && circle.fill == accent.gamma_multiply(0.35)),
        )
    };
    assert!(!lit(&library));
    library.click_text(glyph);
    assert_eq!(library.script(), "one **two**");
    assert_eq!(selection(&library), (6, 9));
    assert!(lit(&library));
    assert_eq!(library.focused(), Some(library.editor_id()));
    // The change is saved like typing.
    library.app.workspace.as_mut().unwrap().flush().unwrap();
    library.restart();
    assert_eq!(library.script(), "one **two**");
    select_in_editor(&mut library, 6, 9);
    library.click_text(glyph);
    assert_eq!(library.script(), "one two");
    assert!(!lit(&library));
}

#[test]
fn the_style_dropdown_shows_the_caret_lines_style_and_changes_it() {
    let mut library = Library::with_script("## two\n\nbody");
    let dropdown = |name: &str| format!("{name}{}", ui_kit::icons::regular::CARET_DOWN);
    select_in_editor(&mut library, 4, 4);
    assert!(library.find_text(&dropdown("Heading 2")).is_some());
    select_in_editor(&mut library, 10, 10);
    library.click_text(&dropdown("Paragraph"));
    library.click_text("QuoteCtrl+Shift+.");
    assert_eq!(library.script(), "## two\n\n> body");
    assert!(library.find_text(&dropdown("Quote")).is_some());
    library.click_text(&dropdown("Quote"));
    library.click_text("Heading 1Ctrl+1");
    assert_eq!(library.script(), "## two\n\n# body");
    library.click_text(&dropdown("Heading 1"));
    library.click_text("ParagraphCtrl+0");
    assert_eq!(library.script(), "## two\n\nbody");
}

#[test]
fn the_toolbar_inserts_cues_from_a_menu_and_a_rule_from_its_button() {
    for (name, cue) in [("Note", "[[note: ]]"), ("Pause", "[[pause]]"), ("Mark", "[[mark: ]]"), ("End", "[[end]]")] {
        let mut library = Library::with_script("words");
        select_in_editor(&mut library, 5, 5);
        library.click_text(ui_kit::icons::regular::BRACKETS_SQUARE);
        library.click_text(name);
        assert_eq!(library.script(), format!("words {cue}"));
    }
    let mut library = Library::with_script("words");
    select_in_editor(&mut library, 5, 5);
    library.click_text(ui_kit::icons::regular::MINUS);
    assert_eq!(library.script(), "words\n\n---\n");
}

#[test]
fn the_toolbars_undo_redo_and_find_buttons_work() {
    let mut library = Library::with_script("words");
    select_in_editor(&mut library, 0, 5);
    library.time += 2.0;
    library.settle();
    library.click_text(ui_kit::icons::regular::TEXT_ITALIC);
    library.click_text(ui_kit::icons::regular::ARROW_COUNTER_CLOCKWISE);
    assert_eq!(library.script(), "words");
    library.click_text(ui_kit::icons::regular::ARROW_CLOCKWISE);
    assert_eq!(library.script(), "*words*");
    library.click_text(ui_kit::icons::regular::MAGNIFYING_GLASS);
    assert!(library.app.find.is_open());
}

#[test]
fn the_format_menu_acts_on_the_selection_the_editor_had() {
    let mut library = Library::with_script("one two");
    select_in_editor(&mut library, 4, 7);
    library.pick("Format", Item::Format(crate::formatting::Action::Inline(crate::markup::Inline::Strike)));
    assert_eq!(library.script(), "one ~~two~~");
    library.pick("Format", Item::Format(crate::formatting::Action::Cue(crate::markup::CueKind::End)));
    assert!(library.script().contains("[[end]]"), "{}", library.script());
}

#[test]
fn the_format_menu_and_the_whole_toolbar_fit_the_smallest_window() {
    let mut library = Library::with_script("- one\n- two");
    library.size = egui::vec2(900.0, 480.0);
    library.settle();
    let panel = editor_panel(&library);
    use ui_kit::icons::regular as icon;
    let glyphs = [
        icon::TEXT_B,
        icon::TEXT_ITALIC,
        icon::TEXT_UNDERLINE,
        icon::TEXT_STRIKETHROUGH,
        icon::LIST_BULLETS,
        icon::LIST_NUMBERS,
        icon::TEXT_OUTDENT,
        icon::TEXT_INDENT,
        icon::MINUS,
        icon::BRACKETS_SQUARE,
        icon::ARROW_COUNTER_CLOCKWISE,
        icon::ARROW_CLOCKWISE,
        icon::MAGNIFYING_GLASS,
    ];
    let mut places: Vec<Rect> = Vec::new();
    for glyph in glyphs {
        let rect = library.find_text(glyph).unwrap();
        assert!(panel.contains_rect(rect), "{glyph:?} at {rect:?} is outside {panel:?}");
        assert!(places.iter().all(|other| !other.intersects(rect)), "{glyph:?} overlaps another");
        places.push(rect);
    }
    let preview = library.find_text("Preview").unwrap();
    assert!(panel.contains_rect(preview) && places.iter().all(|other| !other.intersects(preview)), "{preview:?}");
    assert!(preview.right() > panel.right() - 40.0 && preview.center().y <= places[0].center().y + 1.0, "{preview:?}");
    // The text starts under the lowest button, and still has room.
    let (pos, _) = editor_galley(&library);
    assert!(places.iter().all(|button| button.bottom() < pos.y), "{pos:?} is under a button");
    assert!(pos.y < panel.bottom() - 60.0, "{pos:?}");

    library.click_text("Format");
    let window = Rect::from_min_size(Pos2::ZERO, library.size);
    for item in crate::menu::MENUS.iter().find(|(title, _)| *title == "Format").unwrap().1.iter().copied().flatten() {
        let row = format!("{}{}", item.label(), item.chord());
        let rect = library.find_text(&row).unwrap_or_else(|| panic!("{row} is not painted"));
        assert!(window.contains_rect(rect), "{row} at {rect:?} is off screen");
    }
}

#[test]
fn a_click_in_a_wrapped_list_row_lands_on_that_character() {
    let text = format!("- {}end", "word ".repeat(60));
    let mut library = Library::with_script(&text);
    // Past the first row, whatever its length.
    let index = text.len() - 40;
    let at = character_at(&library, index);
    assert!(at.y > character_at(&library, 2).y, "the item wraps");
    library.click(at, PointerButton::Primary);
    assert_eq!(selection(&library), (index, index));
    // Down and up keep to the same place across the rows.
    library.key(Modifiers::NONE, Key::ArrowUp);
    assert!(selection(&library).1 < index);
    library.key(Modifiers::NONE, Key::ArrowDown);
    assert_eq!(selection(&library), (index, index));
    // A drag selects the real text.
    let (from, to) = (character_at(&library, index), character_at(&library, index + 9));
    library.hold(from);
    library.move_to(to);
    library.let_go(to);
    assert_eq!(selection(&library), (index, index + 9));
}
