use super::*;
use crate::markup::BULLET;

/// Each Markdown and cue form on a line of its own.
const FORMS: &str = "# Heading\n## Closed ##\nSetext\n===\n\na **bold** b\n\na *slanted* b\n\na __two__ _under_ ***both***\n\n- bullet\n  * nested\n+ plus\n\n3. three\n4) four\n\n> quoted\n> > deeper\n\na [[note: wave]] b\n\n[[pause]]\n\n[[mark: B-roll: desk]]\n\n[[end]]";

/// Runs one frame to get at the fonts.
fn with_ui<R>(add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let ctx = egui::Context::default();
    ui_kit::apply(&ctx);
    let mut add = Some(add);
    let mut result = None;
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| result = add.take().map(|add| add(ui)));
    output.textures_delta.clear();
    result.unwrap()
}

fn job(text: &str, reveal: Reveal) -> LayoutJob {
    with_ui(|ui| layout_job(ui, text, &Markup::parse(text), reveal, &[], 600.0))
}

fn format_at(job: &LayoutJob, at: usize) -> TextFormat {
    job.sections.iter().find(|section| section.byte_range.contains(&ByteIndex(at))).unwrap().format.clone()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Seen {
    Hidden,
    Dimmed,
    Plain,
}

fn seen(format: &TextFormat) -> Seen {
    if format.color == Color32::TRANSPARENT && format.font_id.size == HIDDEN_SIZE {
        Seen::Hidden
    } else if format.color == color::TEXT_TERTIARY {
        Seen::Dimmed
    } else {
        assert_eq!(format.color, color::TEXT_PRIMARY);
        Seen::Plain
    }
}

/// Checks every character of [`FORMS`]: syntax looks as `marker` says for its line, and all
/// other text is plain.
fn check(reveal: Reveal, marker: impl Fn(usize) -> Seen) {
    let job = job(FORMS, reveal);
    assert_eq!(job.text, FORMS, "the layout has to be of the text itself");
    let markup = Markup::parse(FORMS);
    for run in markup.runs.iter().filter(|run| &FORMS[run.range.clone()] != "\n") {
        let wanted = if run.style.marker { marker(run.line) } else { Seen::Plain };
        for at in run.range.clone() {
            assert_eq!(seen(&format_at(&job, at)), wanted, "byte {at} of line {}: {:?}", run.line, &FORMS[run.range.clone()]);
        }
    }
}

fn line_of(needle: &str) -> usize {
    FORMS[..FORMS.find(needle).unwrap()].matches('\n').count()
}

/// Something on each line that has syntax.
const WITH_SYNTAX: [&str; 17] = [
    "# Heading",
    "## Closed",
    "===",
    "**bold**",
    "*slanted*",
    "***both***",
    "- bullet",
    "* nested",
    "+ plus",
    "3. three",
    "4) four",
    "> quoted",
    "> > deeper",
    "[[note",
    "[[pause",
    "[[mark",
    "[[end",
];

#[test]
fn every_form_has_syntax_to_check() {
    let markup = Markup::parse(FORMS);
    for needle in WITH_SYNTAX {
        let line = line_of(needle);
        assert!(markup.runs.iter().any(|run| run.line == line && run.style.marker), "{needle}");
    }
}

#[test]
fn every_form_has_its_syntax_hidden_off_the_cursor_line() {
    check(Reveal::default(), |_| Seen::Hidden);
}

#[test]
fn every_form_has_its_syntax_dimmed_on_the_cursor_line_only() {
    for needle in WITH_SYNTAX {
        let cursor = line_of(needle);
        check(Reveal { line: Some(cursor), all: false }, |line| if line == cursor { Seen::Dimmed } else { Seen::Hidden });
    }
}

#[test]
fn show_markdown_dims_every_marker_on_every_line() {
    check(Reveal { line: None, all: true }, |_| Seen::Dimmed);
    check(Reveal { line: Some(0), all: true }, |_| Seen::Dimmed);
}

#[test]
fn headings_are_larger_by_level_and_bold_and_spans_keep_their_style() {
    let text = "# one\n## two\n### three\nbody **bold** *slanted*";
    let job = job(text, Reveal::default());
    let format = |needle: &str| format_at(&job, text.find(needle).unwrap());
    let bold = ui_kit::fonts::semibold(1.0).family;
    let sizes = ["one", "two", "three", "body"].map(|needle| format(needle).font_id.size);
    assert!(sizes.windows(2).all(|pair| pair[0] > pair[1]), "{sizes:?}");
    assert_eq!(sizes[3], typography::BODY_SIZE);
    for needle in ["one", "two", "three", "bold"] {
        assert_eq!(format(needle).font_id.family, bold, "{needle}");
    }
    assert_eq!(format("body").font_id, body_font());
    assert!(format("slanted").italics && !format("bold").italics && !format("body").italics);
}

#[test]
fn a_find_match_is_backed_across_runs() {
    let text = "a **bold** c";
    let tint = color::SEM_PENDING;
    let job = with_ui(|ui| layout_job(ui, text, &Markup::parse(text), Reveal::default(), &[(0..6, tint), (11..12, tint)], 600.0));
    let slice = |section: &LayoutSection| &text[section.byte_range.start.0..section.byte_range.end.0];
    let backed: String = job.sections.iter().filter(|section| section.format.background == tint).map(slice).collect();
    assert_eq!(backed, "a **boc");
    let covered: String = job.sections.iter().map(slice).collect();
    assert_eq!(covered, text);
}

/// The x of the character at `index`, and the top and height of each row.
fn measure(text: &str, reveal: Reveal, index: usize) -> (f32, Vec<(f32, f32)>) {
    with_ui(|ui| {
        let markup = Markup::parse(text);
        let galley = layout(ui, &markup, layout_job(ui, text, &markup, reveal, &[], 600.0));
        assert_eq!(galley.text(), text);
        let x = galley.pos_from_cursor(egui::text::CCursor::new(CharIndex(index))).left();
        (x, galley.rows.iter().map(|row| (row.min_y(), row.height())).collect())
    })
}

const ALL: Reveal = Reveal { line: None, all: true };

#[test]
fn hidden_syntax_takes_next_to_no_width() {
    for (text, content) in [("# Title", 2), ("**bold**", 2), ("_slanted_", 1), ("> quoted", 2)] {
        let hidden = measure(text, Reveal::default(), content).0;
        assert!(hidden < 1.0, "{text}: {hidden}");
        assert!(measure(text, ALL, content).0 > 3.0, "{text}");
    }
}

#[test]
fn revealing_syntax_moves_no_row_up_or_down() {
    // With lines that are nothing but syntax, and empty lines.
    for text in ["Setext\n===\n\n# \n> \n- \n**x**\n[[pause]]\n", FORMS] {
        assert_eq!(measure(text, Reveal::default(), 0).1, measure(text, ALL, 0).1, "{text}");
    }
}

#[test]
fn a_list_items_text_stays_put_when_its_marker_shows() {
    for (text, content) in [("- item", 2), ("- one\n  - two", 10), ("12. item", 4)] {
        let hidden = measure(text, Reveal::default(), content).0;
        let shown = measure(text, ALL, content).0;
        assert!(hidden >= LIST_INDENT * typography::BODY_SIZE - 1.0, "{text}: {hidden}");
        assert!((hidden - shown).abs() <= 1.5, "{text}: {hidden} and {shown}");
    }
    // A deeper item is further in.
    assert!(measure("- one\n  - two", Reveal::default(), 10).0 > measure("- one\n  - two", Reveal::default(), 2).0);
}

/// What is drawn around some text.
struct Drawn {
    /// Bullets and numbers.
    labels: Vec<String>,
    bars: usize,
    chips: Vec<(Rect, Color32)>,
}

fn drawn(text: &str, reveal: Reveal, wrap_width: f32) -> Drawn {
    with_ui(|ui| {
        let markup = Markup::parse(text);
        let galley = layout(ui, &markup, layout_job(ui, text, &markup, reveal, &[], wrap_width));
        let drawn = decorations(ui, &markup, &galley, vec2(GUTTER, 0.0), reveal);
        let labels = drawn.front.iter().filter_map(|shape| match shape {
            Shape::Text(text) => Some(text.galley.text().to_owned()),
            _ => None,
        });
        let bars = drawn.front.iter().filter(|shape| matches!(shape, Shape::Rect(_))).count();
        let chips = drawn.behind.iter().filter_map(|shape| match shape {
            Shape::Rect(chip) => Some((chip.rect, chip.fill)),
            _ => None,
        });
        Drawn { labels: labels.collect(), bars, chips: chips.collect() }
    })
}

#[test]
fn bullets_and_numbers_stand_in_for_hidden_list_markers() {
    let text = "- a\n- b\n\ntext\n\n3. c\n3. d";
    assert_eq!(drawn(text, Reveal::default(), 600.0).labels, [BULLET, BULLET, "3.", "4."]);
    // The cursor's line shows its own marker instead.
    assert_eq!(drawn(text, Reveal { line: Some(1), all: false }, 600.0).labels, [BULLET, "3.", "4."]);
    assert!(drawn(text, ALL, 600.0).labels.is_empty());
    // An item that wraps gets one bullet.
    assert_eq!(drawn(&format!("- {}", "word ".repeat(60)), Reveal::default(), 200.0).labels, [BULLET]);
}

#[test]
fn every_row_of_a_quote_gets_a_bar_whether_or_not_its_marker_shows() {
    let text = "> one\n> two\n\nplain";
    for reveal in [Reveal::default(), Reveal { line: Some(0), all: false }, ALL] {
        assert_eq!(drawn(text, reveal, 600.0).bars, 2, "{reveal:?}");
    }
    assert!(drawn(&format!("> {}", "word ".repeat(60)), Reveal::default(), 200.0).bars > 2);
}

#[test]
fn each_cue_gets_a_chip_tinted_by_its_type() {
    let chips = drawn("[[note: a]] x [[pause]]\n\n[[mark: b]]\n\n[[end]]", Reveal::default(), 600.0).chips;
    let fills: Vec<Color32> = chips.iter().map(|(_, fill)| *fill).collect();
    let accent = ui_kit::AppAppearance::new("prompter", ui_kit::Preset::Violet);
    assert_eq!(fills.len(), 4, "{accent:?}");
    for (at, fill) in fills.iter().enumerate() {
        assert!(!fills[..at].contains(fill), "two types share a tint: {fills:?}");
    }
    assert_eq!(fills[0], color::TEXT_TERTIARY.gamma_multiply(NOTE_TINT));
    assert_eq!(fills[1], color::BG_HOVER);
    assert_eq!(fills[3], color::SEM_RECORD.gamma_multiply(ACCENT_TINT));
    // A chip at the start of a line starts at its left edge, and chips do not overlap.
    assert!((chips[0].0.left() - GUTTER).abs() <= 1.0, "{:?}", chips[0]);
    assert!(chips[0].0.right() < chips[1].0.left(), "{chips:?}");
}

#[test]
fn a_chip_surrounds_its_text_whether_or_not_the_brackets_show() {
    let line = with_ui(|ui| ui.fonts_mut(|fonts| fonts.row_height(&body_font())));
    for reveal in [Reveal::default(), ALL] {
        let chip = drawn("[[pause]]", reveal, 600.0).chips[0].0;
        assert!(chip.top() < 0.0 && chip.bottom() > line, "{reveal:?}: {chip:?}");
        assert!((chip.center().y - line / 2.0).abs() <= 0.5, "{reveal:?}: {chip:?}");
    }
}

#[test]
fn a_cue_that_wraps_gets_a_chip_on_each_row() {
    let long = format!("[[note: {}]]", "word ".repeat(40));
    assert!(drawn(&long, Reveal::default(), 200.0).chips.len() > 1);
}

#[test]
fn underline_and_strikethrough_are_drawn_on_their_text_alone() {
    let text = "a <u>under</u> b ~~gone~~ <u>~~both~~</u>";
    for reveal in [Reveal::default(), ALL] {
        let job = job(text, reveal);
        let lines = |needle: &str| {
            let format = format_at(&job, text.find(needle).unwrap());
            (format.underline.width > 0.0, format.strikethrough.width > 0.0)
        };
        assert_eq!(["a ", "under", "gone", "both"].map(lines), [(false, false), (true, false), (false, true), (true, true)]);
        // The markers have no lines of their own.
        for marker in ["<u>", "</u>", "~~"] {
            assert_eq!(lines(marker), (false, false), "{marker}");
        }
    }
}

/// A row as laid out: where its first character past `skip` is, its right edge, its text.
struct Placed {
    start: f32,
    right: f32,
    text: String,
}

/// The rows of `text`'s layout. The galley is checked to map every character back to itself.
fn rows(text: &str, reveal: Reveal, wrap_width: f32) -> Vec<Placed> {
    with_ui(|ui| {
        let markup = Markup::parse(text);
        let galley = layout(ui, &markup, layout_job(ui, text, &markup, reveal, &[], wrap_width));
        assert_eq!(galley.text(), text);
        assert_eq!(galley.end().index.0, text.chars().count(), "every character has a place: {text:?}");
        // The plain layout, which egui makes by itself, has the same rows of each plain line.
        for index in 0..=text.chars().count() {
            let cursor = egui::text::CCursor { index: CharIndex(index), prefer_next_row: true };
            let at = galley.pos_from_cursor(cursor);
            assert_eq!(galley.layout_from_cursor(galley.cursor_from_pos(at.center().to_vec2())).row, galley.layout_from_cursor(cursor).row);
        }
        galley
            .rows
            .iter()
            .map(|row| Placed {
                start: row.pos.x + row.glyphs.first().map_or(0.0, |glyph| glyph.pos.x),
                right: row.rect().right(),
                text: row.glyphs.iter().map(|glyph| glyph.chr).collect(),
            })
            .collect()
    })
}

/// The x of the character at `index`, and the x each later row starts at.
fn hang_of(text: &str, content: usize, reveal: Reveal) -> (f32, Vec<f32>) {
    let first = measure_at(text, reveal, content, 200.0);
    (first, rows(text, reveal, 200.0).iter().skip(1).map(|row| row.start).collect())
}

fn measure_at(text: &str, reveal: Reveal, index: usize, wrap_width: f32) -> f32 {
    with_ui(|ui| {
        let markup = Markup::parse(text);
        let galley = layout(ui, &markup, layout_job(ui, text, &markup, reveal, &[], wrap_width));
        galley.pos_from_cursor(egui::text::CCursor::new(CharIndex(index))).left()
    })
}

#[test]
fn the_wrapped_rows_of_items_and_quotes_line_up_under_their_text() {
    let long = "word ".repeat(30);
    for prefix in ["- ", "12. ", "- a\n  - ", "> ", "> > ", "> - ", "- [[pause]] "] {
        for reveal in [Reveal::default(), ALL] {
            let text = format!("{prefix}{long}");
            // The last line is the long one.
            let line = text.rfind('\n').map_or(0, |at| at + 1);
            let content = line + Markup::parse(&text).hangs.last().unwrap().1 - line;
            let (first, rest) = hang_of(&text, content, reveal);
            let rest = &rest[usize::from(line > 0)..];
            assert!(rest.len() > 2, "{prefix:?} wraps");
            for x in rest {
                assert!((x - first).abs() <= 1.0, "{prefix:?} {reveal:?}: a row at {x}, the text at {first}");
            }
            for row in rows(&text, reveal, 200.0) {
                assert!(row.right <= 201.0, "{prefix:?}: a row ends at {}", row.right);
            }
        }
    }
}

#[test]
fn lines_around_a_hanging_one_are_laid_out_as_before() {
    let text = "plain\n\n- item\n\n- next\ntail line\n\n> quote\n";
    let laid = rows(text, Reveal::default(), 600.0);
    let texts: Vec<&str> = laid.iter().map(|row| row.text.as_str()).collect();
    assert_eq!(texts, text.split('\n').collect::<Vec<_>>());
    assert!(laid[0].start < 1.0 && laid[7].start < 1.0);
    // The same rows, at the same heights, as egui's own layout of the whole text.
    let tops = |hung: bool| {
        with_ui(|ui| {
            let markup = Markup::parse(text);
            let job = layout_job(ui, text, &markup, Reveal::default(), &[], 600.0);
            let galley = if hung { layout(ui, &markup, job) } else { ui.fonts_mut(|fonts| fonts.layout_job(job)) };
            galley.rows.iter().map(|row| (row.min_y(), row.height())).collect::<Vec<_>>()
        })
    };
    assert_eq!(tops(true), tops(false));
    // With nothing after a last item, there is still a row for the caret.
    for text in ["- a\n", "- a", "- \n", "> \n\n", "- a\n\n- b\n"] {
        assert_eq!(rows(text, Reveal::default(), 600.0).len(), text.split('\n').count(), "{text:?}");
    }
}

#[test]
fn text_is_parsed_once_for_each_change() {
    let mut cache = ParseCache::default();
    let first = cache.get("**a**");
    assert!(Rc::ptr_eq(&first, &cache.get("**a**")));
    assert_eq!(cache.parses, 1);
    assert!(!Rc::ptr_eq(&first, &cache.get("**ab**")));
    assert_eq!(cache.parses, 2);
}
