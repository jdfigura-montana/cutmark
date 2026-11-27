//! The menus' items and their enable rules (spec "Library window", Menus).

use crate::editor;
use crate::formatting::{Action, BlockStyle, ListKind};
use crate::markup::{CueKind, Inline};

/// One menu item. An item with no action yet is listed and disabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    NewScript,
    NewFolder,
    Import,
    Export,
    Duplicate,
    Delete,
    Snapshots,
    CloseTab,
    Settings,
    Exit,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    PastePlain,
    SelectAll,
    FindInScript,
    SearchAll,
    /// An item of the Format menu, which the editor's toolbar has too.
    Format(Action),
    StartOrPause,
    ResetToTop,
    JumpToSection,
    ToggleMode,
    Views,
    ShowView,
    HideView,
    ClickThrough,
    Outline,
    ShowMarkdown,
    NextTab,
    PreviousTab,
    Appearance,
    Help,
    KeyboardShortcuts,
    Tour,
    About,
}

/// What the enable rules depend on.
#[derive(Debug, Clone, Copy, Default)]
pub struct MenuState {
    pub view_open: bool,
    /// False if the script library could not be opened.
    pub library_open: bool,
    pub script_open: bool,
    /// More than one tab is open.
    pub several_tabs: bool,
    /// The editor shows all its Markdown syntax.
    pub show_markdown: bool,
    /// All false with no script open.
    pub editor: editor::Status,
    pub clipboard_has_text: bool,
    pub clipboard_has_html: bool,
}

/// The paragraph styles, as the toolbar's dropdown and the Format menu list them.
pub const STYLES: &[Item] = &[
    Item::Format(Action::Style(BlockStyle::Paragraph)),
    Item::Format(Action::Style(BlockStyle::Heading(1))),
    Item::Format(Action::Style(BlockStyle::Heading(2))),
    Item::Format(Action::Style(BlockStyle::Heading(3))),
    Item::Format(Action::Style(BlockStyle::Quote)),
];

/// The cues, as the toolbar's Insert cue menu and the Format menu list them.
pub const CUES: &[Item] = &[
    Item::Format(Action::Cue(CueKind::Note)),
    Item::Format(Action::Cue(CueKind::Pause)),
    Item::Format(Action::Cue(CueKind::Mark)),
    Item::Format(Action::Cue(CueKind::End)),
];

/// The menus in bar order. Each is a list of groups, drawn with a separator between them.
pub const MENUS: [(&str, &[&[Item]]); 6] = [
    (
        "File",
        &[
            &[Item::NewScript, Item::NewFolder, Item::Import, Item::Export, Item::Duplicate, Item::Delete, Item::Snapshots],
            &[Item::CloseTab],
            &[Item::Settings],
            &[Item::Exit],
        ],
    ),
    ("Edit", &[&[Item::Undo, Item::Redo], &[Item::Cut, Item::Copy, Item::Paste, Item::PastePlain], &[Item::SelectAll, Item::FindInScript, Item::SearchAll]]),
    (
        "Format",
        &[
            STYLES,
            &[
                Item::Format(Action::Inline(Inline::Bold)),
                Item::Format(Action::Inline(Inline::Italic)),
                Item::Format(Action::Inline(Inline::Underline)),
                Item::Format(Action::Inline(Inline::Strike)),
            ],
            &[
                Item::Format(Action::List(ListKind::Bulleted)),
                Item::Format(Action::List(ListKind::Numbered)),
                Item::Format(Action::Outdent),
                Item::Format(Action::Indent),
                Item::Format(Action::Rule),
            ],
            CUES,
        ],
    ),
    ("Script", &[&[Item::StartOrPause, Item::ResetToTop, Item::JumpToSection, Item::ToggleMode]]),
    (
        "View",
        &[
            &[Item::Views, Item::ShowView, Item::HideView, Item::ClickThrough],
            &[Item::Outline, Item::ShowMarkdown],
            &[Item::NextTab, Item::PreviousTab],
            &[Item::Appearance],
        ],
    ),
    ("Help", &[&[Item::Help, Item::KeyboardShortcuts, Item::Tour], &[Item::About]]),
];

/// The editor's right-click menu.
pub const EDITOR_MENU: &[&[Item]] =
    &[&[Item::Undo, Item::Redo], &[Item::Cut, Item::Copy, Item::Paste, Item::PastePlain], &[Item::SelectAll, Item::FindInScript]];

/// The right-click menu on empty space in the Scripts panel.
pub const SCRIPTS_MENU: &[&[Item]] = &[&[Item::NewScript, Item::NewFolder, Item::Import]];

impl Item {
    pub fn label(self) -> &'static str {
        match self {
            Self::NewScript => "New script",
            Self::NewFolder => "New folder",
            Self::Import => "Import",
            Self::Export => "Export",
            Self::Duplicate => "Duplicate",
            Self::Delete => "Delete",
            Self::Snapshots => "Snapshots",
            Self::CloseTab => "Close tab",
            Self::NextTab => "Next tab",
            Self::PreviousTab => "Previous tab",
            Self::Settings => "Settings",
            Self::Exit => "Exit",
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::Cut => "Cut",
            Self::Copy => "Copy",
            Self::Paste => "Paste",
            Self::PastePlain => "Paste as plain text",
            Self::SelectAll => "Select all",
            Self::FindInScript => "Find in script",
            Self::SearchAll => "Search all scripts",
            Self::Format(action) => action.label(),
            Self::StartOrPause => "Start or Pause",
            Self::ResetToTop => "Reset to top",
            Self::JumpToSection => "Jump to section",
            Self::ToggleMode => "Toggle mode",
            Self::Views => "Views",
            Self::ShowView => "Show view",
            Self::HideView => "Hide view",
            Self::ClickThrough => "Click-through",
            Self::Outline => "Outline",
            Self::ShowMarkdown => "Show Markdown",
            Self::Appearance => "Appearance",
            Self::Help => "Help",
            Self::KeyboardShortcuts => "Keyboard shortcuts",
            Self::Tour => "Take the tour",
            Self::About => "About",
        }
    }

    /// The shortcut shown beside the label, or "" for none.
    pub fn chord(self) -> &'static str {
        match self {
            Self::NewScript => "Ctrl+N",
            Self::NewFolder => "Ctrl+Shift+N",
            Self::Undo => "Ctrl+Z",
            Self::Redo => "Ctrl+Y",
            Self::Cut => "Ctrl+X",
            Self::Copy => "Ctrl+C",
            Self::Paste => "Ctrl+V",
            Self::PastePlain => "Ctrl+Shift+V",
            Self::SelectAll => "Ctrl+A",
            Self::FindInScript => "Ctrl+F",
            Self::SearchAll => "Ctrl+Shift+F",
            Self::Format(action) => action.chord(),
            Self::ClickThrough => "Ctrl+Alt+C",
            Self::CloseTab => "Ctrl+W",
            Self::NextTab => "Ctrl+Tab",
            Self::PreviousTab => "Ctrl+Shift+Tab",
            _ => "",
        }
    }

    /// Whether the item can be used now.
    pub fn enabled(self, state: MenuState) -> bool {
        match self {
            Self::Exit | Self::ClickThrough | Self::ShowMarkdown | Self::Appearance | Self::About => true,
            Self::ShowView => !state.view_open,
            Self::HideView => state.view_open,
            Self::NewScript | Self::NewFolder | Self::Import | Self::SearchAll => state.library_open,
            Self::Export | Self::Duplicate | Self::Delete | Self::Snapshots | Self::CloseTab | Self::FindInScript => state.script_open,
            Self::NextTab | Self::PreviousTab => state.several_tabs,
            Self::Undo => state.editor.can_undo,
            Self::Redo => state.editor.can_redo,
            Self::Cut | Self::Copy => state.editor.has_selection,
            Self::Paste => state.script_open && (state.clipboard_has_text || state.clipboard_has_html),
            Self::PastePlain => state.script_open && state.clipboard_has_text,
            Self::SelectAll => state.editor.has_text,
            Self::Format(Action::Indent) => state.editor.format.can_indent,
            Self::Format(Action::Outdent) => state.editor.format.can_outdent,
            Self::Format(_) => state.script_open,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Vec<Item> {
        MENUS.iter().flat_map(|(_, groups)| groups.iter().copied().flatten().copied()).collect()
    }

    fn enabled(view_open: bool, script_open: bool) -> Vec<Item> {
        enabled_in(MenuState { view_open, library_open: true, script_open, ..Default::default() })
    }

    fn enabled_in(state: MenuState) -> Vec<Item> {
        all().into_iter().filter(|item| item.enabled(state)).collect()
    }

    #[test]
    fn with_no_script_the_items_that_need_one_are_disabled() {
        use Item::*;
        assert_eq!(enabled(false, false), [NewScript, NewFolder, Import, Exit, SearchAll, ShowView, ClickThrough, ShowMarkdown, Appearance, About]);
        // Text on the clipboard has nowhere to go.
        let state = MenuState { library_open: true, clipboard_has_text: true, ..Default::default() };
        assert_eq!(enabled_in(state), enabled(false, false));
    }

    #[test]
    fn an_open_script_with_nothing_to_edit_enables_its_file_items_and_find() {
        use Item::*;
        let mut expected = vec![NewScript, NewFolder, Import, Export, Duplicate, Delete, Snapshots, CloseTab, Exit];
        expected.extend([FindInScript, SearchAll]);
        expected.extend(format_items().into_iter().filter(|item| !nesting().contains(item)));
        expected.extend([ShowView, ClickThrough, ShowMarkdown, Appearance, About]);
        assert_eq!(enabled(false, true), expected);
    }

    fn format_items() -> Vec<Item> {
        all().into_iter().filter(|item| matches!(item, Item::Format(_))).collect()
    }

    fn nesting() -> [Item; 2] {
        [Item::Format(Action::Outdent), Item::Format(Action::Indent)]
    }

    #[test]
    fn indent_and_outdent_wait_for_a_list_item_that_can_move() {
        let [outdent, indent] = nesting();
        let base = MenuState { library_open: true, script_open: true, ..Default::default() };
        let at = |can_indent, can_outdent| {
            let format = crate::formatting::Active { can_indent, can_outdent, ..Default::default() };
            MenuState { editor: editor::Status { format, ..Default::default() }, ..base }
        };
        assert!(!indent.enabled(base) && !outdent.enabled(base));
        assert!(indent.enabled(at(true, false)) && !outdent.enabled(at(true, false)));
        assert!(!indent.enabled(at(false, true)) && outdent.enabled(at(false, true)));
    }

    #[test]
    fn every_format_item_needs_only_an_open_script_but_for_nesting() {
        let closed = MenuState { library_open: true, ..Default::default() };
        let open = MenuState { script_open: true, ..closed };
        for item in format_items() {
            assert!(!item.enabled(closed), "{item:?}");
            assert_eq!(item.enabled(open), !nesting().contains(&item), "{item:?}");
        }
    }

    #[test]
    fn the_format_menu_lists_every_action_with_its_shortcut() {
        let chords: Vec<(&str, &str)> = format_items().into_iter().map(|item| (item.label(), item.chord())).collect();
        let expected = [
            ("Paragraph", "Ctrl+0"),
            ("Heading 1", "Ctrl+1"),
            ("Heading 2", "Ctrl+2"),
            ("Heading 3", "Ctrl+3"),
            ("Quote", "Ctrl+Shift+."),
            ("Bold", "Ctrl+B"),
            ("Italic", "Ctrl+I"),
            ("Underline", "Ctrl+U"),
            ("Strikethrough", "Ctrl+Shift+X"),
            ("Bulleted list", "Ctrl+Shift+8"),
            ("Numbered list", "Ctrl+Shift+7"),
            ("Outdent", "Ctrl+["),
            ("Indent", "Ctrl+]"),
            ("Horizontal rule", ""),
            ("Insert note cue", ""),
            ("Insert pause cue", ""),
            ("Insert mark cue", ""),
            ("Insert end cue", ""),
        ];
        assert_eq!(chords, expected);
    }

    #[test]
    fn next_and_previous_tab_need_a_second_tab() {
        use Item::*;
        let one = MenuState { library_open: true, script_open: true, ..Default::default() };
        let two = MenuState { several_tabs: true, ..one };
        for item in [NextTab, PreviousTab] {
            assert!(!item.enabled(one) && item.enabled(two), "{item:?}");
        }
        assert!(CloseTab.enabled(one));
        assert_eq!([CloseTab, NextTab, PreviousTab].map(Item::chord), ["Ctrl+W", "Ctrl+Tab", "Ctrl+Shift+Tab"]);
    }

    #[test]
    fn each_edit_item_waits_for_something_to_do() {
        use Item::*;
        let base = MenuState { library_open: true, script_open: true, ..Default::default() };
        let added = |state: MenuState| -> Vec<Item> {
            let before = enabled_in(base);
            enabled_in(state).into_iter().filter(|item| !before.contains(item)).collect()
        };
        let editor = |editor| MenuState { editor, ..base };
        assert_eq!(added(editor(editor::Status { can_undo: true, ..Default::default() })), [Undo]);
        assert_eq!(added(editor(editor::Status { can_redo: true, ..Default::default() })), [Redo]);
        assert_eq!(added(editor(editor::Status { has_selection: true, ..Default::default() })), [Cut, Copy]);
        assert_eq!(added(editor(editor::Status { has_text: true, ..Default::default() })), [SelectAll]);
        assert_eq!(added(MenuState { clipboard_has_text: true, ..base }), [Paste, PastePlain]);
        // Plain text cannot come from HTML alone.
        assert_eq!(added(MenuState { clipboard_has_html: true, ..base }), [Paste]);
    }

    #[test]
    fn with_no_library_nothing_that_needs_it_is_enabled() {
        use Item::*;
        assert_eq!(enabled_in(MenuState::default()), [Exit, ShowView, ClickThrough, ShowMarkdown, Appearance, About]);
    }

    #[test]
    fn an_open_view_swaps_show_for_hide() {
        assert!(!enabled(true, false).contains(&Item::ShowView));
        assert!(enabled(true, false).contains(&Item::HideView));
        assert!(!enabled(false, false).contains(&Item::HideView));
    }

    #[test]
    fn the_right_click_menus_list_menu_bar_items() {
        let items = all();
        for item in EDITOR_MENU.iter().chain(SCRIPTS_MENU).copied().flatten() {
            assert!(items.contains(item), "{item:?} is not in the menu bar");
        }
    }

    #[test]
    fn every_item_from_the_spec_is_listed_once() {
        let counts: Vec<(&str, usize)> = MENUS.iter().map(|(title, groups)| (*title, groups.iter().map(|group| group.len()).sum())).collect();
        assert_eq!(counts, [("File", 10), ("Edit", 9), ("Format", 18), ("Script", 4), ("View", 9), ("Help", 4)]);
        let items = all();
        for (index, item) in items.iter().enumerate() {
            assert!(!items[..index].contains(item), "{item:?} is listed twice");
        }
    }
}
