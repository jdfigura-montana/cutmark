#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "tests")]

//! .docx and .odt import, on tiny files zipped in memory.

use std::fs;
use std::io::{Cursor, Write};

use prompter_core::convert::DocumentError;
use prompter_core::limits::MAX_WORDS;
use prompter_core::script::{self, Block, BlockKind, Cue};
use prompter_core::store::{Imported, Store, StoreError};
use tempfile::TempDir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

fn zipped(parts: &[(&str, &str)]) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, text) in parts {
        zip.start_file(*name, options).unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn import(file: &str, bytes: &[u8]) -> (TempDir, Store, Result<Imported, StoreError>) {
    let folder = tempfile::tempdir().unwrap();
    let source = folder.path().join(file);
    fs::write(&source, bytes).unwrap();
    let mut store = Store::open(&folder.path().join("data")).unwrap();
    let imported = store.import(&source);
    assert_eq!(fs::read(&source).unwrap(), bytes, "the source is only read");
    (folder, store, imported)
}

/// The Markdown and title of a file's one script.
fn converted(file: &str, bytes: &[u8]) -> (String, String) {
    let (_folder, store, imported) = import(file, bytes);
    let imported = imported.unwrap();
    assert_eq!(imported.scripts.len(), 1);
    (store.load(&imported.scripts[0].id).unwrap(), imported.scripts[0].title.clone())
}

fn refusal(file: &str, bytes: &[u8]) -> DocumentError {
    let (_folder, store, imported) = import(file, bytes);
    assert!(store.library().scripts.is_empty());
    match imported {
        Err(StoreError::Document(error)) => error,
        other => panic!("{other:?}"),
    }
}

fn document_xml(body: &str) -> String {
    format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="w" xmlns:mc="mc"><w:body>{body}</w:body></w:document>"#)
}

/// A .docx with list 1 bulleted, and list 2 numbered with letters on its second level.
fn docx(body: &str) -> Vec<u8> {
    let numbering = r#"<w:numbering xmlns:w="w">
        <w:abstractNum w:abstractNumId="10">
          <w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="bullet"/></w:lvl>
          <w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="bullet"/></w:lvl>
        </w:abstractNum>
        <w:abstractNum w:abstractNumId="20">
          <w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/></w:lvl>
          <w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="lowerLetter"/></w:lvl>
        </w:abstractNum>
        <w:num w:numId="1"><w:abstractNumId w:val="10"/></w:num>
        <w:num w:numId="2"><w:abstractNumId w:val="20"/></w:num>
        <w:num w:numId="3"><w:abstractNumId w:val="20"/><w:lvlOverride w:ilvl="0"><w:startOverride w:val="7"/></w:lvlOverride></w:num>
    </w:numbering>"#;
    zipped(&[("word/document.xml", &document_xml(body)), ("word/numbering.xml", numbering)])
}

fn paragraph(properties: &str, runs: &str) -> String {
    format!("<w:p><w:pPr>{properties}</w:pPr>{runs}</w:p>")
}

fn run(properties: &str, text: &str) -> String {
    format!(r#"<w:r><w:rPr>{properties}</w:rPr><w:t xml:space="preserve">{text}</w:t></w:r>"#)
}

fn listed(list: u32, level: u32, text: &str) -> String {
    paragraph(&format!(r#"<w:numPr><w:ilvl w:val="{level}"/><w:numId w:val="{list}"/></w:numPr>"#), &run("", text))
}

fn styled(style: &str, text: &str) -> String {
    paragraph(&format!(r#"<w:pStyle w:val="{style}"/>"#), &run("", text))
}

fn content_xml(styles: &str, body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content xmlns:office="o" xmlns:text="t" xmlns:style="s" xmlns:fo="f"><office:automatic-styles>{styles}</office:automatic-styles><office:body><office:text>{body}</office:text></office:body></office:document-content>"#
    )
}

/// The automatic styles: B bold, I italic, N neither, L1 bulleted and L2 numbered.
const ODT_STYLES: &str = r#"
    <style:style style:name="B" style:family="text"><style:text-properties fo:font-weight="bold"/></style:style>
    <style:style style:name="I" style:family="text"><style:text-properties fo:font-style="italic"/></style:style>
    <style:style style:name="N" style:family="text"><style:text-properties fo:font-weight="normal" fo:font-style="normal"/></style:style>
    <text:list-style style:name="L1"><text:list-level-style-bullet text:level="1"/><text:list-level-style-bullet text:level="2"/></text:list-style>
    <text:list-style style:name="L2"><text:list-level-style-number text:level="1" style:num-format="1"/><text:list-level-style-number text:level="2" style:num-format="a"/></text:list-style>
"#;

fn odt(body: &str) -> Vec<u8> {
    zipped(&[("mimetype", "application/vnd.oasis.opendocument.text"), ("content.xml", &content_xml(ODT_STYLES, body))])
}

fn item(ordered: bool, depth: u8, start: Option<u64>) -> BlockKind {
    BlockKind::ListItem { ordered, depth, continues: false, start }
}

/// What both of the sample files below should give.
const SAMPLE: &str =
    "# The Title\n\nPlain **bold** and *italic*.\n\n- apples\n- pears\n\nBetween\n\n1. one\n   1. inner\n   2. more\n2. two\n\nfirst line\\\nsecond line\n";

fn sample_docx() -> Vec<u8> {
    let body = [
        styled("Heading1", "The Title"),
        paragraph("", &[run("", "Plain "), run("<w:b/>", "bold"), run("", " and "), run("<w:i w:val=\"true\"/>", "italic"), run("", ".")].concat()),
        listed(1, 0, "apples"),
        listed(1, 0, "pears"),
        paragraph("", &run("", "Between")),
        listed(2, 0, "one"),
        listed(2, 1, "inner"),
        listed(2, 1, "more"),
        listed(2, 0, "two"),
        paragraph("", "<w:r><w:t>first line</w:t><w:br/><w:t>second line</w:t></w:r>"),
    ];
    docx(&body.concat())
}

fn sample_odt() -> Vec<u8> {
    odt(r#"
        <text:h text:outline-level="1">The Title</text:h>
        <text:p>Plain <text:span text:style-name="B">bold</text:span> and <text:span text:style-name="I">italic</text:span>.</text:p>
        <text:list text:style-name="L1">
          <text:list-item><text:p>apples</text:p></text:list-item>
          <text:list-item><text:p>pears</text:p></text:list-item>
        </text:list>
        <text:p>Between</text:p>
        <text:list text:style-name="L2">
          <text:list-item>
            <text:p>one</text:p>
            <text:list>
              <text:list-item><text:p>inner</text:p></text:list-item>
              <text:list-item><text:p>more</text:p></text:list-item>
            </text:list>
          </text:list-item>
          <text:list-item><text:p>two</text:p></text:list-item>
        </text:list>
        <text:p>first line<text:line-break/>second line</text:p>
    "#)
}

#[test]
fn docx_and_odt_samples_give_the_same_markdown() {
    for (file, bytes) in [("a.DOCX", sample_docx()), ("a.odt", sample_odt())] {
        let (markdown, title) = converted(file, &bytes);
        assert_eq!(markdown, SAMPLE, "{file}");
        assert_eq!(title, "The Title", "{file}");
        let blocks = script::parse(&markdown).blocks;
        let kinds: Vec<_> = blocks.iter().map(|block| block.kind.clone()).collect();
        assert_eq!(kinds[2..4], [item(false, 0, None), item(false, 0, None)], "{file}");
        assert_eq!(kinds[5..9], [item(true, 0, Some(1)), item(true, 1, Some(1)), item(true, 1, None), item(true, 0, None)], "{file}");
        assert_eq!(blocks[9].text(), "first line\nsecond line", "{file}");
    }
}

#[test]
fn docx_headings_and_run_properties() {
    let body = [
        styled("Title", "Top"),
        styled("Heading6", "Deep"),
        styled("Heading7", "Too deep"),
        // A numbered heading stays a heading.
        paragraph(r#"<w:pStyle w:val="Heading2"/><w:numPr><w:ilvl w:val="0"/><w:numId w:val="2"/></w:numPr>"#, &run("", "Numbered")),
        paragraph(
            // The paragraph mark's own formatting styles no text.
            "<w:rPr><w:b/><w:i/></w:rPr>",
            &[run(r#"<w:b w:val="0"/>"#, "a "), run(r#"<w:i w:val="false"/>"#, "b "), run(r#"<w:b w:val="1"/><w:i/>"#, "c")].concat(),
        ),
        // A tracked change keeps the old formatting inside the new.
        paragraph("", &run("<w:rPrChange><w:rPr><w:b/></w:rPr></w:rPrChange>", "d")),
    ];
    assert_eq!(converted("a.docx", &docx(&body.concat())).0, "# Top\n\n###### Deep\n\nToo deep\n\n## Numbered\n\na b ***c***\n\nd\n");
}

#[test]
fn docx_keeps_the_text_and_drops_the_rest() {
    let body = r#"<w:p><w:r><w:t>a</w:t><w:tab/><w:t>b</w:t><w:noBreakHyphen/><w:t>c &amp; &lt;d&gt; &#233;</w:t></w:r>
        <w:hyperlink><w:r><w:t xml:space="preserve"> link</w:t></w:r></w:hyperlink>
        <w:ins><w:r><w:t xml:space="preserve"> added</w:t></w:r></w:ins>
        <w:del><w:r><w:delText>deleted</w:delText><w:br/></w:r></w:del>
        <w:moveFrom><w:r><w:t>moved</w:t></w:r></w:moveFrom>
        <w:r><w:instrText>HYPERLINK x</w:instrText><w:drawing><w:t>drawn</w:t></w:drawing></w:r>
        <w:r><mc:AlternateContent><mc:Choice><w:pict><w:t>boxed</w:t></w:pict></mc:Choice><mc:Fallback><w:t>fallback</w:t></mc:Fallback></mc:AlternateContent></w:r></w:p>
        <w:p/><w:tbl><w:tr><w:tc><w:p><w:r><w:t>cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
        <w:sdt><w:sdtContent><w:p><w:r><w:t>*not* [[pause]]</w:t></w:r></w:p></w:sdtContent></w:sdt>
        <w:p><w:r><w:t># not a heading</w:t></w:r></w:p><w:sectPr/>"#;
    let (markdown, title) = converted("My talk.docx", &zipped(&[("word/document.xml", &document_xml(body))]));
    assert_eq!(title, "My talk");
    let blocks = script::parse(&markdown).blocks;
    let texts: Vec<_> = blocks.iter().map(Block::text).collect();
    assert_eq!(texts, ["a b-c & <d> \u{e9} link added", "cell", "*not*", "", "# not a heading"], "{markdown:?}");
    assert_eq!(blocks[3].kind, BlockKind::Cue(Cue::Pause));
    assert!(blocks.iter().all(|block| !matches!(block.kind, BlockKind::Heading(_))));
}

#[test]
fn docx_numbering_continues_and_restarts() {
    let body = [
        listed(2, 0, "one"),
        paragraph("", &run("", "break")),
        listed(2, 0, "two"),
        listed(3, 0, "seven"),
        paragraph("", &run("", "break")),
        // List 0 is no list, and a list with no definition is bulleted.
        listed(0, 0, "plain"),
        listed(9, 0, "unknown"),
    ];
    assert_eq!(converted("a.docx", &docx(&body.concat())).0, "1. one\n\nbreak\n\n2. two\n7. seven\n\nbreak\n\nplain\n\n- unknown\n");
}

#[test]
fn a_list_that_starts_deep_keeps_its_items_level() {
    let body = [listed(1, 1, "a"), listed(1, 1, "b")];
    assert_eq!(converted("a.docx", &docx(&body.concat())).0, "- a\n- b\n");
}

#[test]
fn odt_spans_nest_and_other_content_is_dropped() {
    let body = r#"
        <text:tracked-changes><text:changed-region><text:deletion><text:p>deleted</text:p></text:deletion></text:changed-region></text:tracked-changes>
        <text:h text:outline-level="3">Part<text:note><text:note-body><text:p>footnote</text:p></text:note-body></text:note></text:h>
        <text:h text:outline-level="9"><text:number>2.1</text:number>Numbered</text:h>
        <text:p><text:span text:style-name="B">bold <text:span text:style-name="I">both</text:span> <text:span text:style-name="N">neither</text:span></text:span></text:p>
        <text:p>a<text:s text:c="3"/>b<text:tab/>c &amp; <text:a>link</text:a><office:annotation><text:p>comment</text:p></office:annotation></text:p>
        <text:p><draw:frame><draw:text-box><text:p>boxed</text:p></draw:text-box></draw:frame></text:p>
        <text:section><table:table><table:table-row><table:table-cell><text:p>cell</text:p></table:table-cell></table:table-row></table:table></text:section>
        <text:p>1. not a list</text:p>
    "#;
    let expected = "### Part\n\n###### 2.1 Numbered\n\n**bold *both*** neither\n\na b c \\& link\n\ncell\n\n1\\. not a list\n";
    assert_eq!(converted("a.odt", &odt(body)).0, expected);
}

#[test]
fn odt_lists_with_two_paragraphs_headings_and_continued_numbers() {
    let body = r#"
        <text:list text:style-name="L2"><text:list-item><text:p>one</text:p><text:p>more</text:p></text:list-item></text:list>
        <text:p>break</text:p>
        <text:list text:style-name="L2" text:continue-numbering="true"><text:list-item><text:p>two</text:p></text:list-item></text:list>
        <text:p>break</text:p>
        <text:list text:style-name="L2"><text:list-item text:start-value="5"><text:p>five</text:p></text:list-item></text:list>
        <text:list text:style-name="L2"><text:list-item><text:h text:outline-level="2">Heading</text:h></text:list-item></text:list>
    "#;
    assert_eq!(converted("a.odt", &odt(body)).0, "1. one\\\n   more\n\nbreak\n\n2. two\n\nbreak\n\n5. five\n\n## Heading\n");
}

#[test]
fn odt_takes_a_named_list_style_from_the_styles_part() {
    let styles = r#"<office:document-styles><office:styles><text:list-style style:name="Numbering_20_123">
        <text:list-level-style-number text:level="1" style:num-format="1" text:start-value="3"/></text:list-style></office:styles></office:document-styles>"#;
    let body = r#"<text:list text:style-name="Numbering_20_123"><text:list-item><text:p>three</text:p></text:list-item></text:list>"#;
    let bytes = zipped(&[("content.xml", &content_xml("", body)), ("styles.xml", styles)]);
    assert_eq!(converted("a.odt", &bytes).0, "3. three\n");
}

#[test]
fn a_large_document_is_split_at_a_heading() {
    let words = "word ".repeat(MAX_WORDS * 3 / 5);
    let body = [styled("Heading1", "Long"), paragraph("", &run("", &words)), styled("Heading2", "Rest"), paragraph("", &run("", &words))];
    let (_folder, _store, imported) = import("a.docx", &docx(&body.concat()));
    let imported = imported.unwrap();
    assert!(imported.split);
    let titles: Vec<_> = imported.scripts.iter().map(|meta| meta.title.as_str()).collect();
    assert_eq!(titles, ["Long (part 1 of 2)", "Long (part 2 of 2)"]);
}

#[test]
fn damaged_files_are_refused() {
    let good = sample_docx();
    let deep = format!("{}{}", "<w:sdt>".repeat(100_000), "</w:sdt>".repeat(100_000));
    let documents = [
        // Cut off, closed in the wrong order, an entity that is not defined, and a NUL.
        document_xml("").replace("</w:body></w:document>", "<w:p>"),
        document_xml("<w:p></w:r>"),
        document_xml("<w:p><w:r><w:t>&unknown;</w:t></w:r></w:p>"),
        document_xml("<w:p><w:r><w:t>&#0;</w:t></w:r></w:p>"),
        document_xml(&deep),
        String::new(),
        "not XML".to_owned(),
        "<html><body/></html>".to_owned(),
    ];
    let mut files: Vec<Vec<u8>> = documents.iter().map(|xml| zipped(&[("word/document.xml", xml)])).collect();
    files.extend([Vec::new(), b"plain text".to_vec(), good[..good.len() / 2].to_vec(), zipped(&[("other.xml", "<a/>")]), sample_odt()]);
    for (number, bytes) in files.iter().enumerate() {
        assert_eq!(refusal("a.docx", bytes), DocumentError::Damaged, "file {number}");
    }
    assert_eq!(refusal("a.odt", &sample_docx()), DocumentError::Damaged);
    assert_eq!(refusal("a.odt", b"PK\x03\x04 and then nothing"), DocumentError::Damaged);
    let numbering = zipped(&[("word/document.xml", &document_xml("")), ("word/numbering.xml", "<w:numbering>")]);
    assert_eq!(refusal("a.docx", &numbering), DocumentError::Damaged);
}

/// Both files are a few kilobytes zipped.
#[test]
fn a_file_that_unpacks_to_too_much_is_refused() {
    for xml in [document_xml(&"<w:p/>".repeat(4_000_001)), " ".repeat(64 * 1024 * 1024 + 1)] {
        assert_eq!(refusal("a.docx", &zipped(&[("word/document.xml", &xml)])), DocumentError::TooLarge);
    }
}

#[test]
fn password_protected_files_are_refused() {
    // Word wraps a protected .docx in an OLE file.
    let mut wrapped = vec![0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];
    wrapped.resize(512, 0);
    assert_eq!(refusal("a.docx", &wrapped), DocumentError::Protected);
    // A protected .odt is a zip whose parts are scrambled.
    let manifest =
        r#"<manifest:manifest><manifest:file-entry manifest:full-path="content.xml"><manifest:encryption-data/></manifest:file-entry></manifest:manifest>"#;
    let protected = zipped(&[("META-INF/manifest.xml", manifest), ("content.xml", "\u{1}\u{2} scrambled")]);
    assert_eq!(refusal("a.odt", &protected), DocumentError::Protected);
    assert!(DocumentError::Protected.to_string().contains("password"));
}

/// Whatever one wrong byte or a cut does to a file, the import must return.
#[test]
fn no_broken_byte_or_cut_causes_a_panic() {
    let folder = tempfile::tempdir().unwrap();
    let mut store = Store::open(&folder.path().join("data")).unwrap();
    for (file, good) in [("a.docx", sample_docx()), ("a.odt", sample_odt())] {
        let source = folder.path().join(file);
        let mut refused = 0;
        for at in (0..good.len()).step_by(7) {
            let mut flipped = good.clone();
            flipped[at] ^= 0xff;
            for bytes in [&flipped[..], &good[..at]] {
                fs::write(&source, bytes).unwrap();
                refused += usize::from(store.import(&source).is_err());
            }
        }
        assert!(refused > 0, "{file}");
    }
}
