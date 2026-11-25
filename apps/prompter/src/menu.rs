//! The menu bar's items and their enable rules (spec "Library window", Menus).

/// One menu item. An item with no action yet is listed and disabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    New,
    Import,
    Export,
    Duplicate,
    Delete,
    Settings,
    Exit,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    PastePlain,
    Find,
    StartOrPause,
    ResetToTop,
    JumpToSection,
    ToggleMode,
    Views,
    ShowView,
    HideView,
    ClickThrough,
    Outline,
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
}

/// The menus in bar order. Each is a list of groups, drawn with a separator between them.
pub const MENUS: [(&str, &[&[Item]]); 5] = [
    ("File", &[&[Item::New, Item::Import, Item::Export, Item::Duplicate, Item::Delete], &[Item::Settings], &[Item::Exit]]),
    ("Edit", &[&[Item::Undo, Item::Redo], &[Item::Cut, Item::Copy, Item::Paste, Item::PastePlain], &[Item::Find]]),
    ("Script", &[&[Item::StartOrPause, Item::ResetToTop, Item::JumpToSection, Item::ToggleMode]]),
    ("View", &[&[Item::Views, Item::ShowView, Item::HideView, Item::ClickThrough], &[Item::Outline], &[Item::Appearance]]),
    ("Help", &[&[Item::Help, Item::KeyboardShortcuts, Item::Tour], &[Item::About]]),
];

impl Item {
    pub fn label(self) -> &'static str {
        match self {
            Self::New => "New",
            Self::Import => "Import",
            Self::Export => "Export",
            Self::Duplicate => "Duplicate",
            Self::Delete => "Delete",
            Self::Settings => "Settings",
            Self::Exit => "Exit",
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::Cut => "Cut",
            Self::Copy => "Copy",
            Self::Paste => "Paste",
            Self::PastePlain => "Paste as plain text",
            Self::Find => "Find",
            Self::StartOrPause => "Start or Pause",
            Self::ResetToTop => "Reset to top",
            Self::JumpToSection => "Jump to section",
            Self::ToggleMode => "Toggle mode",
            Self::Views => "Views",
            Self::ShowView => "Show view",
            Self::HideView => "Hide view",
            Self::ClickThrough => "Click-through",
            Self::Outline => "Outline",
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
            Self::ClickThrough => "Ctrl+Alt+C",
            _ => "",
        }
    }

    /// Whether the item can be used now.
    pub fn enabled(self, state: MenuState) -> bool {
        match self {
            Self::Exit | Self::ClickThrough | Self::Appearance | Self::About => true,
            Self::ShowView => !state.view_open,
            Self::HideView => state.view_open,
            Self::New | Self::Import | Self::Find => state.library_open,
            Self::Export | Self::Duplicate | Self::Delete | Self::Undo | Self::Redo | Self::Cut | Self::Copy | Self::Paste => state.script_open,
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
        let state = MenuState { view_open, library_open: true, script_open };
        all().into_iter().filter(|item| item.enabled(state)).collect()
    }

    #[test]
    fn with_no_script_the_items_that_need_one_are_disabled() {
        use Item::*;
        assert_eq!(enabled(false, false), [New, Import, Exit, Find, ShowView, ClickThrough, Appearance, About]);
    }

    #[test]
    fn an_open_script_enables_the_file_and_edit_items() {
        use Item::*;
        let expected = [New, Import, Export, Duplicate, Delete, Exit, Undo, Redo, Cut, Copy, Paste, Find, ShowView, ClickThrough, Appearance, About];
        assert_eq!(enabled(false, true), expected);
    }

    #[test]
    fn with_no_library_nothing_that_needs_it_is_enabled() {
        use Item::*;
        let enabled: Vec<Item> = all().into_iter().filter(|item| item.enabled(MenuState::default())).collect();
        assert_eq!(enabled, [Exit, ShowView, ClickThrough, Appearance, About]);
    }

    #[test]
    fn an_open_view_swaps_show_for_hide() {
        assert!(!enabled(true, false).contains(&Item::ShowView));
        assert!(enabled(true, false).contains(&Item::HideView));
        assert!(!enabled(false, false).contains(&Item::HideView));
    }

    #[test]
    fn every_item_from_the_spec_is_listed_once() {
        let counts: Vec<(&str, usize)> = MENUS.iter().map(|(title, groups)| (*title, groups.iter().map(|group| group.len()).sum())).collect();
        assert_eq!(counts, [("File", 7), ("Edit", 7), ("Script", 4), ("View", 6), ("Help", 4)]);
        let items = all();
        for (index, item) in items.iter().enumerate() {
            assert!(!items[..index].contains(item), "{item:?} is listed twice");
        }
    }
}
