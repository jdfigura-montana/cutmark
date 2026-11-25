//! The script editor: a monospace text field, and the Edit menu's commands for it.

use ui_kit::tokens::{color, typography};

use crate::menu::Item;

/// The editor's id for one opening of a script. Its cursor and undo history are kept under it.
pub fn id(serial: u64) -> egui::Id {
    egui::Id::new(("script-editor", serial))
}

/// Gives the editor the keyboard and has it do what an Edit menu item asks, as if the keys had
/// been pressed. Call it before [`show`], and not in the frame of the click that asked: the
/// editor drops the keyboard on a click elsewhere.
pub fn command(ui: &egui::Ui, id: egui::Id, item: Item) {
    let key = |key| egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers { ctrl: true, command: true, ..Default::default() },
    };
    let event = match item {
        Item::Undo => Some(key(egui::Key::Z)),
        Item::Redo => Some(key(egui::Key::Y)),
        Item::Cut => Some(egui::Event::Cut),
        Item::Copy => Some(egui::Event::Copy),
        // Only the window can read the clipboard. It sends the text with the next frame.
        Item::Paste => None,
        _ => return,
    };
    ui.memory_mut(|memory| memory.request_focus(id));
    match event {
        Some(event) => ui.input_mut(|input| input.events.push(event)),
        None => ui.ctx().send_viewport_cmd(egui::ViewportCommand::RequestPaste),
    }
}

/// The editor, filling the space left. True if it changed the text.
pub fn show(ui: &mut egui::Ui, id: egui::Id, text: &mut String) -> bool {
    egui::ScrollArea::vertical()
        .id_salt(id)
        .auto_shrink(false)
        .show(ui, |ui| {
            let font = ui_kit::fonts::mono(typography::BODY_SIZE);
            let line = ui.fonts_mut(|fonts| fonts.row_height(&font));
            // Enough empty rows that a click anywhere in the panel lands in the field.
            let rows = (ui.available_height() / line).floor().max(1.0) as usize;
            let edit = egui::TextEdit::multiline(text)
                .id(id)
                .font(font)
                .text_color(color::TEXT_PRIMARY)
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY)
                .desired_rows(rows);
            ui.add(edit).changed()
        })
        .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One frame of the editor, with the command given first as the Library does.
    fn frame(ctx: &egui::Context, time: f64, text: &mut String, item: Option<Item>, events: Vec<egui::Event>) -> egui::FullOutput {
        let input = egui::RawInput { time: Some(time), events, ..Default::default() };
        let mut output = ctx.run_ui(input, |ui| {
            if let Some(item) = item {
                command(ui, id(1), item);
            }
            show(ui, id(1), text);
        });
        output.textures_delta.clear();
        output
    }

    #[test]
    fn undo_and_redo_from_the_menu_reach_the_editor() {
        let ctx = egui::Context::default();
        ctx.set_fonts(ui_kit::fonts::definitions());
        let mut text = "one".to_owned();
        // The undo history takes a point once the text has rested for a second.
        ctx.memory_mut(|memory| memory.request_focus(id(1)));
        frame(&ctx, 0.0, &mut text, None, Vec::new());
        frame(&ctx, 2.0, &mut text, None, Vec::new());
        frame(&ctx, 3.0, &mut text, None, vec![egui::Event::Text(" two".to_owned())]);
        frame(&ctx, 5.0, &mut text, None, Vec::new());
        assert_eq!(text, "one two");
        // A menu click takes the keyboard away from the editor.
        ctx.memory_mut(|memory| memory.surrender_focus(id(1)));
        frame(&ctx, 6.0, &mut text, Some(Item::Undo), Vec::new());
        assert_eq!(text, "one");
        frame(&ctx, 7.0, &mut text, Some(Item::Redo), Vec::new());
        assert_eq!(text, "one two");
    }

    #[test]
    fn copy_cut_and_paste_reach_the_editor() {
        let ctx = egui::Context::default();
        ctx.set_fonts(ui_kit::fonts::definitions());
        let mut text = "words".to_owned();
        ctx.memory_mut(|memory| memory.request_focus(id(1)));
        frame(&ctx, 0.0, &mut text, None, Vec::new());
        let all = egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers { ctrl: true, command: true, ..Default::default() },
        };
        frame(&ctx, 1.0, &mut text, None, vec![all]);
        let copied = |output: egui::FullOutput| {
            output.platform_output.commands.into_iter().find_map(|command| match command {
                egui::OutputCommand::CopyText(text) => Some(text),
                _ => None,
            })
        };
        ctx.memory_mut(|memory| memory.surrender_focus(id(1)));
        assert_eq!(copied(frame(&ctx, 2.0, &mut text, Some(Item::Copy), Vec::new())).as_deref(), Some("words"));
        assert_eq!(copied(frame(&ctx, 3.0, &mut text, Some(Item::Cut), Vec::new())).as_deref(), Some("words"));
        assert_eq!(text, "");
        let paste = frame(&ctx, 4.0, &mut text, Some(Item::Paste), Vec::new());
        let commands = &paste.viewport_output[&egui::ViewportId::ROOT].commands;
        assert!(commands.contains(&egui::ViewportCommand::RequestPaste));
        frame(&ctx, 5.0, &mut text, None, vec![egui::Event::Paste("back".to_owned())]);
        assert_eq!(text, "back");
    }
}
