//! The dark script area, and the editor's live preview that follows the cursor.

use prompter_core::script;
use ui_kit::tokens::{radius, space};

use crate::renderer::{self, LaidBlock, RenderStyle};

/// The views' dark background with a script scrolling on it, filling the space left.
pub fn pane<R>(ui: &mut egui::Ui, salt: impl std::hash::Hash + std::fmt::Debug, style: &RenderStyle, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let frame = egui::Frame::new().fill(style.colors.background).corner_radius(radius::GROUP).inner_margin(egui::Margin::same(space::SCALE_12 as i8));
    frame.show(ui, |ui| egui::ScrollArea::vertical().id_salt(salt).auto_shrink(false).show(ui, add_contents).inner).inner
}

/// The preview beside the editor. The layout is kept until the text or the width changes.
#[derive(Default)]
pub struct Preview {
    pub open: bool,
    /// What `blocks` were laid out from: the editor's opening, the text and the width.
    made_from: Option<(u64, String, f32)>,
    blocks: Vec<LaidBlock>,
    /// The block last scrolled to. Until the cursor leaves it or the layout changes, the
    /// preview can be scrolled by hand.
    followed: Option<usize>,
}

impl Preview {
    /// Forgets the layout, so that the next showing goes to the cursor again.
    pub fn forget(&mut self) {
        self.made_from = None;
        self.blocks = Vec::new();
        self.followed = None;
    }

    /// Draws the preview of `text`, scrolled to the block on the cursor's line.
    pub fn show(&mut self, ui: &mut egui::Ui, serial: u64, text: &str, cursor_line: u32) {
        let style = RenderStyle::preview();
        pane(ui, ("script-preview", serial), &style, |ui| {
            let width = ui.available_width();
            if !self.made_from.as_ref().is_some_and(|(from, source, at)| *from == serial && source == text && *at == width) {
                self.blocks = renderer::layout_script(&script::parse(text), &style, width);
                self.made_from = Some((serial, text.to_owned(), width));
                self.followed = None;
            }
            let rects = renderer::show_blocks(ui, &self.blocks, &style);
            let cursor = block_on_line(&self.blocks, cursor_line);
            if cursor != self.followed
                && let Some(rect) = cursor.and_then(|at| rects.get(at))
            {
                // A block taller than the pane shows from its top.
                let align = if rect.height() > ui.clip_rect().height() { egui::Align::Min } else { egui::Align::Center };
                ui.scroll_to_rect(*rect, Some(align));
            }
            self.followed = cursor;
        });
    }

    /// The first line of the block the preview last scrolled to.
    #[cfg(test)]
    pub fn followed_line(&self) -> Option<u32> {
        self.followed.and_then(|at| self.blocks.get(at)).map(|block| block.source_line)
    }
}

/// The block holding a 1-based Markdown line: the last to start at or before it. A line before
/// the first block belongs to that one. Of the blocks a cue splits one line into, the first.
fn block_on_line(blocks: &[LaidBlock], line: u32) -> Option<usize> {
    let start = blocks.iter().map(|block| block.source_line).take_while(|&start| start <= line).last();
    match start {
        Some(start) => blocks.iter().position(|block| block.source_line == start),
        None => (!blocks.is_empty()).then_some(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cursor_line_picks_the_block_that_holds_it() {
        let blocks = renderer::layout_script(&script::parse("\n# Title\n\none\ntwo [[pause]] three\n\nlast"), &RenderStyle::preview(), 400.0);
        let lines: Vec<u32> = blocks.iter().map(|block| block.source_line).collect();
        assert_eq!(lines, [2, 4, 5, 5, 7]);
        let picked: Vec<Option<usize>> = (1..=8).map(|line| block_on_line(&blocks, line)).collect();
        assert_eq!(picked, [0, 0, 0, 1, 2, 2, 4, 4].map(Some));
        assert_eq!(block_on_line(&[], 1), None);
    }

    #[test]
    fn the_preview_scrolls_to_the_cursor_block_and_then_stays_put() {
        let ctx = egui::Context::default();
        ui_kit::apply(&ctx);
        let text = (1..=40).map(|n| format!("Paragraph {n}")).collect::<Vec<_>>().join("\n\n");
        let mut preview = Preview::default();
        // The first block's top on screen, which moves up as the preview scrolls down.
        let frame = |preview: &mut Preview, line: u32, time: f64| {
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 500.0));
            let input = egui::RawInput { screen_rect: Some(screen), time: Some(time), ..Default::default() };
            let mut output = ctx.run_ui(input, |ui| preview.show(ui, 1, &text, line));
            output.textures_delta.clear();
            output.shapes.iter().find_map(|clipped| match &clipped.shape {
                egui::Shape::Text(painted) if painted.galley.text() == "Paragraph 40" => Some(painted.pos.y),
                _ => None,
            })
        };
        let run = |preview: &mut Preview, line: u32, from: u32| (from..from + 10).map(|n| frame(preview, line, f64::from(n))).last().flatten();
        // At the top, the last paragraph is out of sight and so not painted.
        assert_eq!(run(&mut preview, 1, 0), None);
        assert!(run(&mut preview, 79, 10).is_some_and(|y| y < 500.0));
        assert_eq!(preview.followed_line(), Some(79));
        assert_eq!(run(&mut preview, 1, 20), None);
    }
}
