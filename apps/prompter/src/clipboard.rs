//! Whether the clipboard has text, for the Paste items. egui cannot be asked, so a menu that
//! offers Paste asks the window for a paste as it opens and keeps the reply from the widgets.
//! No reply means there is nothing to paste.

/// How many frames to wait for the reply. eframe sends it with the second one.
const REPLY_FRAMES: u8 = 3;

#[derive(Debug, Default)]
pub struct ClipboardCheck {
    has_text: bool,
    /// Frames left to wait for the reply; 0 when none is due.
    waiting: u8,
    /// A menu with Paste was open in the last frame.
    menu_open: bool,
}

impl ClipboardCheck {
    pub fn has_text(&self) -> bool {
        self.has_text
    }

    /// Takes a due reply out of the input. Call it before any widget reads the input.
    pub fn take_reply(&mut self, ui: &egui::Ui) {
        if self.waiting == 0 {
            return;
        }
        let mut replied = false;
        ui.input_mut(|input| {
            input.events.retain(|event| {
                let paste = matches!(event, egui::Event::Paste(_));
                replied |= paste;
                !paste
            });
        });
        self.waiting -= 1;
        if replied {
            self.has_text = true;
            self.waiting = 0;
        }
        // One more frame, to draw the answer or to go on waiting.
        if replied || self.waiting > 0 {
            ui.ctx().request_repaint();
        }
    }

    /// Call once a frame with whether a menu with Paste is open.
    pub fn watch(&mut self, ctx: &egui::Context, menu_open: bool) {
        if menu_open && !self.menu_open {
            self.has_text = false;
            self.waiting = REPLY_FRAMES;
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
            ctx.request_repaint();
        }
        self.menu_open = menu_open;
    }
}
