//! The Media page: a placeholder until the media widgets get their gallery
//! sections (the audio editor restyle adopts them in S1 phases 11 and 12).

use eframe::egui;
use ui_kit::fonts;
use ui_kit::panel;
use ui_kit::tokens::{color, typography};

/// Draws the Media page placeholder panel.
pub fn show(ui: &mut egui::Ui) {
    panel(ui, |ui| {
        ui.label(egui::RichText::new("Media").font(fonts::semibold(typography::HEADLINE_SIZE)).color(color::TEXT_PRIMARY));
        ui.label(
            egui::RichText::new("The media widgets (transport deck, meters, tracks, clips) get their sections here with the audio editor restyle.")
                .font(fonts::medium(typography::BODY_SIZE))
                .color(color::TEXT_SECONDARY),
        );
    });
}
