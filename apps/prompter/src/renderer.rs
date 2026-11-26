//! How the app draws. First the wgpu setup: DX12 with DirectComposition presentation, so a
//! view's surface can offer a non-opaque alpha mode. Then the script renderer, which the
//! editor's preview uses and the views will share.

use std::sync::{Arc, OnceLock};

use eframe::egui_wgpu::{WgpuSetup, WgpuSetupCreateNew};
use eframe::wgpu;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontFamily, FontId, Rect, Stroke, pos2, vec2};
use prompter_core::script::{Block, BlockKind, Cue, Script};
use ui_kit::tokens::color;

/// The composite alpha mode in use, filled in once the first surface exists.
pub type AlphaInUse = Arc<OnceLock<wgpu::CompositeAlphaMode>>;

/// The renderer setup. The adapter selector is the one place eframe hands out a real surface, so
/// the surface's alpha modes are read and logged there. It runs once.
pub fn wgpu_setup(alpha: AlphaInUse) -> WgpuSetup {
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = wgpu::Backends::DX12;
    setup.instance_descriptor.backend_options.dx12.presentation_system = wgpu::Dx12SwapchainKind::DxgiFromVisual;
    let power = setup.power_preference;
    setup.native_adapter_selector = Some(Arc::new(move |adapters, surface| {
        // A selector replaces wgpu's own choice, so this ranks by device type the way wgpu does.
        let adapter = adapters
            .iter()
            .filter(|adapter| surface.is_none_or(|s| adapter.is_surface_supported(s)))
            .min_by_key(|adapter| rank(adapter.get_info().device_type, power))
            .ok_or_else(|| "no DX12 adapter supports the window surface".to_owned())?;
        if let Some(surface) = surface {
            let supported = surface.get_capabilities(adapter).alpha_modes;
            let used = pick_alpha_mode(&supported);
            log::info!("surface composite alpha modes: {supported:?}; in use for transparent windows: {used:?}");
            // A second run would be a new render state; the first answer stands.
            let _ = alpha.set(used);
        }
        Ok(adapter.clone())
    }));
    setup.into()
}

fn rank(device: wgpu::DeviceType, power: wgpu::PowerPreference) -> u8 {
    let low_power = power == wgpu::PowerPreference::LowPower;
    match device {
        wgpu::DeviceType::DiscreteGpu => u8::from(low_power),
        wgpu::DeviceType::IntegratedGpu => u8::from(!low_power),
        wgpu::DeviceType::Other => 2,
        wgpu::DeviceType::VirtualGpu => 3,
        wgpu::DeviceType::Cpu => 4,
    }
}

/// The rule egui-wgpu 0.36 applies when the root viewport is transparent. It does not expose its
/// choice, so this repeats it; check it again when egui-wgpu is upgraded.
fn pick_alpha_mode(supported: &[wgpu::CompositeAlphaMode]) -> wgpu::CompositeAlphaMode {
    use wgpu::CompositeAlphaMode::{Auto, PostMultiplied, PreMultiplied};
    [PreMultiplied, PostMultiplied].into_iter().find(|mode| supported.contains(mode)).unwrap_or(Auto)
}

/// The text size of the editor's preview.
const PREVIEW_SIZE: f32 = 28.0;
/// Line height as a multiple of the text size.
const LINE_HEIGHT: f32 = 1.35;
/// Heading sizes for levels 1 to 6, as multiples of the text size.
pub(crate) const HEADING_SCALE: [f32; 6] = [1.6, 1.4, 1.25, 1.15, 1.15, 1.15];
/// The size of the pause glyph and of a mark's flag and label.
const CUE_SCALE: f32 = 0.7;

// The lengths below are multiples of the text size.

/// The indent of one list level. A two-digit number fits in it.
pub(crate) const LIST_INDENT: f32 = 1.8;
/// The gap between a list marker and its text.
pub(crate) const MARKER_GAP: f32 = 0.35;
const QUOTE_INDENT: f32 = 0.8;
const QUOTE_BAR: f32 = 0.15;
/// The space between two blocks, and between two list items.
const BLOCK_GAP: f32 = 0.6;
const ITEM_GAP: f32 = 0.15;
/// The gap between "End" and the rule on each side of it.
const END_GAP: f32 = 0.5;
const RULE_WIDTH: f32 = 0.06;

/// How a script is drawn. The views' settings will add to it.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderStyle {
    pub font: FontFamily,
    /// For headings and bold spans.
    pub bold_font: FontFamily,
    pub size: f32,
    /// A multiple of the text size.
    pub line_height: f32,
    pub colors: ScriptColors,
}

/// The colors of the script itself. They are content, so they do not follow the UI's tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptColors {
    pub background: Color32,
    pub text: Color32,
    /// Notes, the other cues and list markers.
    pub dimmed: Color32,
    /// Rules and quote bars.
    pub line: Color32,
}

impl RenderStyle {
    /// White text on the views' dark background.
    pub fn preview() -> Self {
        // The views' panel color without its see-through.
        let [r, g, b, _] = color::GLASS_FILL.to_srgba_unmultiplied();
        Self {
            font: FontFamily::Proportional,
            bold_font: ui_kit::fonts::semibold(PREVIEW_SIZE).family,
            size: PREVIEW_SIZE,
            line_height: LINE_HEIGHT,
            colors: ScriptColors {
                background: Color32::from_rgb(r, g, b),
                text: Color32::WHITE,
                dimmed: Color32::from_white_alpha(140),
                line: Color32::from_white_alpha(90),
            },
        }
    }

    fn format(&self, font: FontId, color: Color32) -> TextFormat {
        let line_height = Some(font.size * self.line_height);
        TextFormat { line_height, ..TextFormat::simple(font, color) }
    }
}

/// What a block has besides its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ornament {
    None,
    /// A list item's bullet or number, left of the text.
    Marker(String),
    QuoteBar,
    /// A line across, with no text.
    Rule,
    /// A line across with the text in the middle of it.
    End,
}

/// One script block, ready to be laid out at the width it was made for.
#[derive(Debug, Clone)]
pub struct LaidBlock {
    /// 1-based line in the Markdown where the block starts.
    pub source_line: u32,
    /// For a block with spans, the job's text is the block's text, so a position in one is the
    /// same position in the other. Markers are not part of it.
    pub job: LayoutJob,
    /// Room left of the text for a marker or a quote bar.
    pub indent: f32,
    /// The space above the block.
    pub space_before: f32,
    pub ornament: Ornament,
}

/// One [`LaidBlock`] for each of the script's blocks, in order, wrapped to `width`.
pub fn layout_script(script: &Script, style: &RenderStyle, width: f32) -> Vec<LaidBlock> {
    // The number of the last ordered item at each list depth.
    let mut numbers = Vec::new();
    let mut after_item = false;
    let mut laid = Vec::with_capacity(script.blocks.len());
    for block in &script.blocks {
        let item = matches!(block.kind, BlockKind::ListItem { .. });
        let gap = match () {
            () if laid.is_empty() => 0.0,
            () if item && after_item => ITEM_GAP,
            () => BLOCK_GAP,
        };
        after_item = item;
        let (job, indent, ornament) = lay_block(block, style, &mut numbers);
        let mut block = LaidBlock { source_line: block.source_line, job, indent: indent * style.size, space_before: gap * style.size, ornament };
        block.job.wrap.max_width = (width - block.indent).max(style.size);
        laid.push(block);
    }
    laid
}

/// A block's text, its indent as a multiple of the text size, and its ornament.
fn lay_block(block: &Block, style: &RenderStyle, numbers: &mut Vec<u64>) -> (LayoutJob, f32, Ornament) {
    match &block.kind {
        BlockKind::Heading(level) => {
            let scale = HEADING_SCALE[usize::from(level.saturating_sub(1)).min(HEADING_SCALE.len() - 1)];
            (spans_job(block, style, style.size * scale, true), 0.0, Ornament::None)
        }
        BlockKind::Paragraph => (spans_job(block, style, style.size, false), 0.0, Ornament::None),
        &BlockKind::ListItem { ordered, depth, continues, start } => {
            let marker = if continues {
                Ornament::None
            } else if ordered {
                Ornament::Marker(format!("{}.", next_number(numbers, usize::from(depth), start)))
            } else {
                Ornament::Marker("\u{2022}".to_owned())
            };
            (spans_job(block, style, style.size, false), LIST_INDENT * (f32::from(depth) + 1.0), marker)
        }
        BlockKind::Quote => (spans_job(block, style, style.size, false), QUOTE_INDENT, Ornament::QuoteBar),
        BlockKind::Rule => (LayoutJob::default(), 0.0, Ornament::Rule),
        BlockKind::Cue(cue) => {
            let (job, ornament) = cue_job(cue, style);
            (job, 0.0, ornament)
        }
    }
}

fn next_number(numbers: &mut Vec<u64>, depth: usize, start: Option<u64>) -> u64 {
    if numbers.len() <= depth {
        numbers.resize(depth + 1, 0);
    }
    numbers[depth] = start.unwrap_or_else(|| numbers[depth].saturating_add(1));
    numbers[depth]
}

fn spans_job(block: &Block, style: &RenderStyle, size: f32, heading: bool) -> LayoutJob {
    let mut job = LayoutJob::default();
    for span in &block.spans {
        let family = if heading || span.bold { &style.bold_font } else { &style.font };
        let mut format = style.format(FontId::new(size, family.clone()), style.colors.text);
        format.italics = span.italic;
        job.append(&span.text, 0.0, format);
    }
    job
}

fn cue_job(cue: &Cue, style: &RenderStyle) -> (LayoutJob, Ornament) {
    let body = style.format(FontId::new(style.size, style.font.clone()), style.colors.dimmed);
    let small = style.size * CUE_SCALE;
    let glyph = style.format(ui_kit::fonts::icon_regular(small), style.colors.dimmed);
    let mut job = LayoutJob::default();
    let mut ornament = Ornament::None;
    match cue {
        Cue::Note(note) => job.append(note, 0.0, TextFormat { italics: true, ..body }),
        Cue::Pause => job.append(ui_kit::icons::regular::PAUSE, 0.0, glyph),
        Cue::Mark { label, .. } => {
            job.append(ui_kit::icons::regular::FLAG, 0.0, glyph);
            if !label.is_empty() {
                let format = style.format(FontId::new(small, style.font.clone()), style.colors.dimmed);
                job.append(label, small * MARKER_GAP, format);
            }
        }
        Cue::End => {
            job.append("End", 0.0, body);
            ornament = Ornament::End;
        }
    }
    (job, ornament)
}

/// Draws laid blocks down `ui`, and returns where each one went.
pub fn show_blocks(ui: &mut egui::Ui, blocks: &[LaidBlock], style: &RenderStyle) -> Vec<Rect> {
    ui.spacing_mut().item_spacing.y = 0.0;
    let width = ui.available_width();
    let em = style.size;
    let rule = Stroke::new(RULE_WIDTH * em, style.colors.line);
    blocks
        .iter()
        .map(|block| {
            ui.add_space(block.space_before);
            let galley = ui.fonts_mut(|fonts| fonts.layout_job(block.job.clone()));
            let height = if block.ornament == Ornament::Rule { em * style.line_height / 2.0 } else { galley.size().y };
            let (rect, _) = ui.allocate_exact_size(vec2(width, height), egui::Sense::hover());
            if !ui.is_rect_visible(rect) {
                return rect;
            }
            let mut at = pos2(rect.left() + block.indent, rect.top());
            match &block.ornament {
                Ornament::None => {}
                Ornament::Marker(marker) => {
                    // The text's own line height, so that the two share a baseline.
                    let format = style.format(FontId::new(em, style.font.clone()), style.colors.dimmed);
                    let marker = ui.fonts_mut(|fonts| fonts.layout_job(LayoutJob::single_section(marker.clone(), format)));
                    ui.painter().galley(pos2(at.x - MARKER_GAP * em - marker.size().x, at.y), marker, style.colors.dimmed);
                }
                Ornament::QuoteBar => {
                    let bar = Rect::from_min_size(rect.min, vec2(QUOTE_BAR * em, height));
                    ui.painter().rect_filled(bar, 0.0, style.colors.line);
                }
                Ornament::Rule => {
                    ui.painter().hline(rect.x_range(), rect.center().y, rule);
                }
                Ornament::End => {
                    at.x = rect.center().x - galley.size().x / 2.0;
                    let before = egui::Rangef::new(rect.left(), at.x - END_GAP * em);
                    let after = egui::Rangef::new(at.x + galley.size().x + END_GAP * em, rect.right());
                    ui.painter().hline(before, rect.center().y, rule);
                    ui.painter().hline(after, rect.center().y, rule);
                }
            }
            ui.painter().galley(at, galley, style.colors.text);
            rect
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use prompter_core::script::parse;
    use wgpu::CompositeAlphaMode::{Auto, Opaque, PostMultiplied, PreMultiplied};

    fn laid(markdown: &str) -> Vec<LaidBlock> {
        layout_script(&parse(markdown), &RenderStyle::preview(), 400.0)
    }

    #[test]
    fn each_script_block_gets_one_laid_block_with_its_text() {
        let markdown = "# Title\n\nSome **bold** and *slanted* words.\n\n- one [[pause]] two\n\n> quoted\n\n---\n\n[[note: wave]]\n\n[[end]]";
        let script = parse(markdown);
        let blocks = laid(markdown);
        assert_eq!(blocks.len(), script.blocks.len());
        for (source, block) in script.blocks.iter().zip(&blocks) {
            assert_eq!(block.source_line, source.source_line);
            if !source.spans.is_empty() {
                assert_eq!(block.job.text, source.text());
            }
            assert_eq!(block.job.wrap.max_width, 400.0 - block.indent);
        }
        let kinds: Vec<&Ornament> = blocks.iter().map(|block| &block.ornament).collect();
        let bullet = Ornament::Marker("\u{2022}".to_owned());
        // The cue splits the list item, and only the first part gets a bullet.
        let none = Ornament::None;
        assert_eq!(kinds, [&none, &none, &bullet, &none, &none, &Ornament::QuoteBar, &Ornament::Rule, &none, &Ornament::End]);
        assert_eq!(blocks[2].indent, blocks[4].indent);
    }

    #[test]
    fn bold_and_italic_spans_keep_their_style() {
        let style = RenderStyle::preview();
        let blocks = laid("plain **bold** *slanted*");
        let sections: Vec<(bool, bool)> = blocks[0].job.sections.iter().map(|s| (s.format.font_id.family == style.bold_font, s.format.italics)).collect();
        assert_eq!(sections, [(false, false), (true, false), (false, false), (false, true)]);
    }

    #[test]
    fn a_heading_is_larger_than_body_text_and_bold() {
        let style = RenderStyle::preview();
        let blocks = laid("# One\n\n###### Six\n\nbody");
        let format = |at: usize| &blocks[at].job.sections[0].format;
        assert!(format(0).font_id.size > format(1).font_id.size);
        assert!(format(1).font_id.size > format(2).font_id.size);
        assert_eq!(format(2).font_id.size, 28.0);
        assert_eq!(format(0).font_id.family, style.bold_font);
        assert!(format(0).line_height > format(2).line_height);
    }

    #[test]
    fn cue_blocks_get_the_cue_styles() {
        let style = RenderStyle::preview();
        let blocks = laid("[[note: wave]]\n\n[[pause]]\n\n[[mark: B-roll: desk]]\n\n[[end]]");
        let texts: Vec<&str> = blocks.iter().map(|block| block.job.text.as_str()).collect();
        let flagged = format!("{}desk", ui_kit::icons::regular::FLAG);
        assert_eq!(texts, ["wave", ui_kit::icons::regular::PAUSE, flagged.as_str(), "End"]);
        let sections = blocks.iter().flat_map(|block| &block.job.sections);
        assert!(sections.clone().all(|section| section.format.color == style.colors.dimmed));
        assert!(blocks[0].job.sections[0].format.italics);
        assert!(!blocks[3].job.sections[0].format.italics);
        // The pause glyph and the flag with its label are small.
        let small = blocks[1].job.sections.iter().chain(&blocks[2].job.sections);
        assert!(small.into_iter().all(|section| section.format.font_id.size < style.size));
        assert_eq!(blocks[3].ornament, Ornament::End);
    }

    #[test]
    fn ordered_items_count_on_from_their_lists_start() {
        let markers: Vec<Ornament> = laid("3. a\n4. b\n   1. inner\n   2. inner\n5. c").into_iter().map(|block| block.ornament).collect();
        let expected = ["3.", "4.", "1.", "2.", "5."].map(|number| Ornament::Marker(number.to_owned()));
        assert_eq!(markers, expected);
    }

    #[test]
    fn premultiplied_wins_then_postmultiplied_then_auto() {
        assert_eq!(pick_alpha_mode(&[Opaque, PostMultiplied, PreMultiplied]), PreMultiplied);
        assert_eq!(pick_alpha_mode(&[Opaque, PostMultiplied]), PostMultiplied);
        assert_eq!(pick_alpha_mode(&[Opaque]), Auto);
    }
}
