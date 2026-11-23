//! The Glass Deck components (spec: Design Language "Components"): one file
//! per widget, every widget reads only the design tokens and the runtime
//! [`Theme`](crate::Theme).
//!
//! Shared helpers: [`focus::focus_ring`] and [`capsule_radius`].

pub mod about;
pub mod accent_picker;
pub mod caption_buttons;
pub mod chip;
pub mod dialog;
pub mod fader;
pub mod focus;
pub mod glass;
pub mod icon_button;
pub mod inspector;
pub mod list_row;
pub mod main_toolbar;
pub mod menu_bar;
pub mod panel;
pub mod primary_button;
pub mod scroll_thumb;
pub mod search_field;
pub mod segmented;
pub mod slider;
pub mod status_bar;
pub mod switch;
pub mod text_field;
pub mod timeline;
pub mod title_block;
pub mod toast;
pub mod tool_strip;
pub mod track_header;
pub mod track_row;
pub mod transport;

pub use about::{AboutInfo, about_dialog};
pub use accent_picker::{accent_picker, appearance_dialog};
pub use caption_buttons::{CaptionRects, caption_buttons};
pub use chip::chip;
pub use dialog::dialog;
pub use fader::fader;
pub use focus::focus_ring;
pub use glass::{glass_frame, paint_glass_highlight};
pub use icon_button::icon_button;
pub use inspector::{inspector_group, inspector_header, inspector_row};
pub use list_row::list_row;
pub use main_toolbar::{MainToolbar, main_toolbar};
pub use menu_bar::{Menu, MenuBarOutput, menu_bar};
pub use panel::panel;
pub use primary_button::primary_button;
pub use scroll_thumb::scroll_thumb;
pub use search_field::search_field;
pub use segmented::segmented;
pub use slider::{SliderKind, slider};
pub use status_bar::status_bar;
pub use switch::switch;
pub use text_field::text_field;
pub use timeline::{
    ClipKind, ClipLook, MarkerKind, marker_color, paint_clip, paint_marker, paint_marquee, paint_pending_band, paint_playhead, paint_ruler, paint_snap_line,
    paint_waveform, waveform_color,
};
pub use title_block::{Title, title_block};
pub use toast::{set_toast_time, show_toasts, toast};
pub use tool_strip::tool_strip;
pub use track_header::{Toggle, ToggleKind, track_header};
pub use track_row::{paint_track_area_empty, paint_track_header_column, paint_track_row};
pub use transport::{DeckState, PlayKind, deck_button, play_button, timecode, transport_deck};

use egui::CornerRadius;

/// The capsule radius rule (spec "Radii": `radius.capsule` is height / 2,
/// a rule rather than a number, so it is computed from the widget height).
pub(crate) fn capsule_radius(height: f32) -> CornerRadius {
    (height / 2.0).into()
}
