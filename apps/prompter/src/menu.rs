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

    /// Whether the item can be used while no script is open, which is always the case until
    /// scripts exist.
    pub fn enabled(self, view_open: bool) -> bool {
        match self {
            Self::Exit | Self::ClickThrough | Self::Appearance | Self::About => true,
            Self::ShowView => !view_open,
            Self::HideView => view_open,
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

    fn enabled(view_open: bool) -> Vec<Item> {
        all().into_iter().filter(|item| item.enabled(view_open)).collect()
    }

    #[test]
    fn with_no_script_and_no_view_only_the_wired_items_are_enabled() {
        assert_eq!(enabled(false), [Item::Exit, Item::ShowView, Item::ClickThrough, Item::Appearance, Item::About]);
    }

    #[test]
    fn an_open_view_swaps_show_for_hide() {
        assert_eq!(enabled(true), [Item::Exit, Item::HideView, Item::ClickThrough, Item::Appearance, Item::About]);
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
