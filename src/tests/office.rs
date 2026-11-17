use super::*;
use std::io::Write as _;
use std::path::PathBuf;

const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main""#;
const P: &str = r#"xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:a14="http://schemas.microsoft.com/office/drawing/2010/main""#;
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// A zip of `parts` at `dir/name`.
fn package(dir: &Path, name: &str, parts: &[(&str, Vec<u8>)]) -> PathBuf {
    let path = dir.join(name);
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (part, bytes) in parts {
        zip.start_file(*part, options).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
    path
}

fn rels(entries: &[(&str, &str, &str)]) -> Vec<u8> {
    let mut s = String::from(
        r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
    for (id, kind, target) in entries {
        let mode = if target.starts_with("http") {
            r#" TargetMode="External""#
        } else {
            ""
        };
        s.push_str(&format!(
            r#"<Relationship Id="{id}" Type="{REL}/{kind}" Target="{target}"{mode}/>"#
        ));
    }
    s.push_str("</Relationships>");
    s.into_bytes()
}

/// A Word document of `body`, with `more` parts, as markdown.
fn docx_with(body: &str, doc_rels: &[(&str, &str, &str)], more: &[(&str, Vec<u8>)]) -> Read {
    let dir = tempfile::tempdir().unwrap();
    let doc =
        format!(r#"<?xml version="1.0"?><w:document {W}><w:body>{body}</w:body></w:document>"#);
    let mut parts = vec![
        (
            "_rels/.rels",
            rels(&[("rId1", "officeDocument", "word/document.xml")]),
        ),
        ("word/document.xml", doc.into_bytes()),
        ("word/_rels/document.xml.rels", rels(doc_rels)),
    ];
    parts.extend(more.iter().cloned());
    let path = package(dir.path(), "doc.docx", &parts);
    read(&path, Pictures::Written).unwrap()
}

fn docx(body: &str) -> String {
    docx_with(body, &[], &[]).markdown
}

fn p(runs: &str) -> String {
    format!("<w:p>{runs}</w:p>")
}

fn r(text: &str) -> String {
    format!("<w:r><w:t xml:space=\"preserve\">{text}</w:t></w:r>")
}

#[test]
fn paragraphs_keep_their_styles_merged() {
    let md = docx(&p(&format!(
        "{}<w:r><w:rPr><w:b/></w:rPr><w:t>bold </w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>still</w:t></w:r>{}<w:r><w:rPr><w:i/></w:rPr><w:t>it</w:t></w:r>",
        r("Some "),
        r(" and "),
    )));
    assert_eq!(md.trim(), "Some **bold still** and *it*");
}

#[test]
fn text_that_looks_like_markdown_is_escaped() {
    let md = docx(&p(&r("1. not a list *or* $5 @me")));
    assert_eq!(md.trim(), r"1\. not a list \*or\* \$5 \@me");
}

#[test]
fn headings_come_from_styles_and_the_title_is_front_matter() {
    let styles = format!(
        r#"<w:styles {W}><w:style w:styleId="T"><w:name w:val="Title"/></w:style><w:style w:styleId="H2"><w:name w:val="heading 2"/></w:style><w:style w:styleId="Mine"><w:name w:val="My head"/><w:basedOn w:val="H2"/></w:style></w:styles>"#
    );
    let body = format!(
        "<w:p><w:pPr><w:pStyle w:val=\"T\"/></w:pPr>{}</w:p><w:p><w:pPr><w:pStyle w:val=\"H2\"/></w:pPr>{}</w:p><w:p><w:pPr><w:pStyle w:val=\"Mine\"/></w:pPr>{}</w:p>",
        r("The title"),
        r("Part"),
        r("Inherited")
    );
    let md = docx_with(
        &body,
        &[("rId1", "styles", "styles.xml")],
        &[("word/styles.xml", styles.into_bytes())],
    )
    .markdown;
    assert_eq!(
        md,
        "---\ntitle: \"The title\"\n---\n\n## Part\n\n## Inherited\n"
    );
}

#[test]
fn lists_are_bulleted_or_numbered_as_their_numbering_says() {
    let numbering = format!(
        r#"<w:numbering {W}><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:numFmt w:val="bullet"/></w:lvl><w:lvl w:ilvl="1"><w:numFmt w:val="decimal"/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>"#
    );
    let item = |level: u8, text: &str| {
        format!(
            "<w:p><w:pPr><w:numPr><w:ilvl w:val=\"{level}\"/><w:numId w:val=\"1\"/></w:numPr></w:pPr>{}</w:p>",
            r(text)
        )
    };
    let body = format!(
        "{}{}{}{}",
        item(0, "one"),
        item(1, "sub"),
        item(0, "two"),
        p(&r("after"))
    );
    let md = docx_with(
        &body,
        &[("rId1", "numbering", "numbering.xml")],
        &[("word/numbering.xml", numbering.into_bytes())],
    )
    .markdown;
    assert_eq!(md, "- one\n    1. sub\n- two\n\nafter\n");
}

#[test]
fn tables_links_and_footnotes() {
    let body = format!(
        "<w:tbl><w:tr><w:tc>{}</w:tc><w:tc>{}</w:tc></w:tr><w:tr><w:tc><w:tcPr><w:gridSpan w:val=\"2\"/></w:tcPr>{}</w:tc></w:tr></w:tbl>{}",
        p(&r("a")),
        p(&r("b|c")),
        p(&r("wide")),
        p(&format!(
            "<w:hyperlink r:id=\"rId5\">{}</w:hyperlink>{}<w:r><w:footnoteReference w:id=\"2\"/></w:r>",
            r("ohp"),
            r(" rocks")
        )),
    );
    let notes = format!(
        r#"<w:footnotes {W}><w:footnote w:id="2"><w:p><w:r><w:t>A note.</w:t></w:r></w:p></w:footnote></w:footnotes>"#
    );
    let md = docx_with(
        &body,
        &[
            ("rId5", "hyperlink", "https://example.com/x"),
            ("rId6", "footnotes", "footnotes.xml"),
        ],
        &[("word/footnotes.xml", notes.into_bytes())],
    )
    .markdown;
    assert_eq!(
        md,
        "| a | b\\|c |\n| --- | --- |\n| wide |  |\n\n[ohp](<https://example.com/x>) rocks[^2]\n\n[^2]: A note\\.\n"
    );
}

#[test]
fn code_paragraphs_are_one_block() {
    let styles = format!(
        r#"<w:styles {W}><w:style w:styleId="Src"><w:name w:val="Source Code"/></w:style></w:styles>"#
    );
    let code = |t: &str| {
        format!(
            "<w:p><w:pPr><w:pStyle w:val=\"Src\"/></w:pPr>{}</w:p>",
            r(t)
        )
    };
    let body = format!("{}{}{}", code("let x = *y;"), code("x"), p(&r("then")));
    let md = docx_with(
        &body,
        &[("rId1", "styles", "styles.xml")],
        &[("word/styles.xml", styles.into_bytes())],
    )
    .markdown;
    assert_eq!(md, "```\nlet x = *y;\nx\n```\n\nthen\n");
}

#[test]
fn equations_are_latex() {
    let frac =
        "<m:f><m:num><m:r><m:t>a</m:t></m:r></m:num><m:den><m:r><m:t>b</m:t></m:r></m:den></m:f>";
    let body = format!(
        "{}<w:p><m:oMathPara><m:oMath>{frac}</m:oMath></m:oMathPara></w:p>",
        p(&format!(
            "{}<m:oMath><m:sSup><m:e><m:r><m:t>x</m:t></m:r></m:e><m:sup><m:r><m:t>2</m:t></m:r></m:sup></m:sSup></m:oMath>",
            r("so ")
        ))
    );
    assert_eq!(docx(&body), "so $x^{2}$\n\n$$\\frac{a}{b}$$\n");
}

#[test]
fn pictures_are_written_aside_and_unreadable_ones_noted() {
    let drawing = |id: &str| {
        format!(
            "<w:r><w:drawing><wp:inline><wp:docPr id=\"1\" name=\"p\" descr=\"A box\"/><a:graphic><a:graphicData><a:blip r:embed=\"{id}\"/></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"
        )
    };
    let body = p(&format!("{}{}", drawing("rId7"), drawing("rId8")));
    let read = docx_with(
        &body,
        &[
            ("rId7", "image", "media/one.png"),
            ("rId8", "image", "media/two.emf"),
        ],
        &[
            ("word/media/one.png", b"png".to_vec()),
            ("word/media/two.emf", b"emf".to_vec()),
        ],
    );
    let md = read.markdown.trim();
    let path = md
        .strip_prefix("![A box](<")
        .and_then(|s| s.strip_suffix(">)"))
        .unwrap_or_else(|| panic!("{md}"));
    assert_eq!(std::fs::read(path).unwrap(), b"png");
    assert_eq!(
        read.notes,
        ["1 picture in a format typst cannot read left out (emf)"]
    );
}

#[test]
fn not_a_package_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.docx");
    std::fs::write(&path, b"\xd0\xcf\x11\xe0 an old binary file").unwrap();
    let err = read(&path, Pictures::Written).err().unwrap().to_string();
    assert!(err.contains("is not a Word or PowerPoint file"), "{err}");
}

fn slide(shapes: &str) -> Vec<u8> {
    format!(
        r#"<?xml version="1.0"?><p:sld {P}><p:cSld><p:spTree>{shapes}</p:spTree></p:cSld></p:sld>"#
    )
    .into_bytes()
}

fn sp(kind: Option<&str>, paras: &str) -> String {
    let ph = match kind {
        Some("") => "<p:ph/>".to_string(),
        Some(t) => format!("<p:ph type=\"{t}\"/>"),
        None => String::new(),
    };
    format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"1\" name=\"s\"/><p:cNvSpPr/><p:nvPr>{ph}</p:nvPr></p:nvSpPr><p:txBody>{paras}</p:txBody></p:sp>"
    )
}

fn ap(text: &str) -> String {
    format!("<a:p><a:r><a:t>{text}</a:t></a:r></a:p>")
}

fn pptx(slides: &[Vec<u8>], notes: &[(usize, &str)]) -> String {
    let dir = tempfile::tempdir().unwrap();
    let ids: String = (1..=slides.len())
        .map(|i| format!("<p:sldId id=\"{}\" r:id=\"s{i}\"/>", 255 + i))
        .collect();
    let pres = format!(
        r#"<?xml version="1.0"?><p:presentation {P}><p:sldIdLst>{ids}</p:sldIdLst></p:presentation>"#
    );
    let targets: Vec<(String, String)> = (1..=slides.len())
        .map(|i| (format!("s{i}"), format!("slides/slide{i}.xml")))
        .collect();
    let entries: Vec<(&str, &str, &str)> = targets
        .iter()
        .map(|(id, t)| (id.as_str(), "slide", t.as_str()))
        .collect();
    let mut parts = vec![
        (
            "_rels/.rels".to_string(),
            rels(&[("rId1", "officeDocument", "ppt/presentation.xml")]),
        ),
        ("ppt/presentation.xml".into(), pres.into_bytes()),
        ("ppt/_rels/presentation.xml.rels".into(), rels(&entries)),
    ];
    for (i, s) in slides.iter().enumerate() {
        parts.push((format!("ppt/slides/slide{}.xml", i + 1), s.clone()));
    }
    for (i, text) in notes {
        let target = format!("../notesSlides/notesSlide{i}.xml");
        parts.push((
            format!("ppt/slides/_rels/slide{i}.xml.rels"),
            rels(&[("n1", "notesSlide", &target)]),
        ));
        let body = sp(Some("body"), &ap(text));
        parts.push((
            format!("ppt/notesSlides/notesSlide{i}.xml"),
            format!(r#"<p:notes {P}><p:cSld><p:spTree>{body}</p:spTree></p:cSld></p:notes>"#)
                .into_bytes(),
        ));
    }
    let parts: Vec<(&str, Vec<u8>)> = parts.iter().map(|(n, b)| (n.as_str(), b.clone())).collect();
    let path = package(dir.path(), "deck.pptx", &parts);
    read(&path, Pictures::Written).unwrap().markdown
}

#[test]
fn slides_have_titles_bullets_and_notes() {
    let title = slide(&format!(
        "{}{}",
        sp(Some("ctrTitle"), &ap("Deck")),
        sp(Some("subTitle"), &ap("Sub"))
    ));
    let one = slide(&format!(
        "{}{}{}{}",
        sp(Some("title"), &ap("First")),
        sp(
            Some(""),
            &format!(
                "{}<a:p><a:pPr lvl=\"1\"/><a:r><a:t>deeper</a:t></a:r></a:p>",
                ap("point")
            )
        ),
        sp(None, &ap("a text box")),
        sp(Some("sldNum"), &ap("2")),
    ));
    let two = slide(&sp(Some("title"), &ap("Second")));
    let md = pptx(&[title, one, two], &[(2, "Say this")]);
    assert_eq!(
        md,
        "---\ntitle: \"Deck\"\nsubtitle: \"Sub\"\noutput:\n  powerpoint_presentation:\n    slide_level: 2\n---\n\n\
         ## First\n\n- point\n    - deeper\n\na text box\n\n::: notes\nSay this\n:::\n\n---\n\n## Second\n"
    );
}

#[test]
fn slide_equations_are_latex() {
    let math = "<mc:AlternateContent><mc:Choice Requires=\"a14\"><a14:m><m:oMathPara><m:oMath><m:r><m:t>E=m</m:t></m:r><m:sSup><m:e><m:r><m:t>c</m:t></m:r></m:e><m:sup><m:r><m:t>2</m:t></m:r></m:sup></m:sSup></m:oMath></m:oMathPara></a14:m></mc:Choice><mc:Fallback><a:r><a:t>picture</a:t></a:r></mc:Fallback></mc:AlternateContent>";
    let one = slide(&sp(None, &format!("<a:p>{math}</a:p>")));
    assert!(pptx(&[one], &[]).ends_with("$E=mc^{2}$\n"));
}

/// `OHP_OFFICE=file.docx cargo test dump -- --ignored --nocapture` prints a
/// file's markdown.
#[test]
#[ignore]
fn dump() {
    let path = std::env::var("OHP_OFFICE").unwrap();
    let read = read(Path::new(&path), Pictures::Written).unwrap();
    println!("{}\n{:?}", read.markdown, read.notes);
}

#[test]
fn a_title_slide_without_a_subtitle_is_the_title() {
    let title = slide(&sp(Some("ctrTitle"), &ap("Deck")));
    let one = slide(&sp(Some("title"), &ap("One")));
    let md = pptx(&[title, one], &[]);
    assert!(md.starts_with("---\ntitle: \"Deck\"\n"), "{md}");
    assert!(md.ends_with("---\n\n## One\n"), "{md}");
}

#[test]
fn old_binary_files_are_told_to_be_saved_anew() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("talk.ppt");
    std::fs::write(&path, b"\xd0\xcf\x11\xe0").unwrap();
    let err = read(&path, Pictures::Written).err().unwrap().to_string();
    assert!(
        err.contains("ohp reads .pptx, not Office's old .ppt"),
        "{err}"
    );
}

#[test]
fn a_word_file_is_typeset() {
    let dir = tempfile::tempdir().unwrap();
    let doc = format!(
        r#"<?xml version="1.0"?><w:document {W}><w:body>{}<w:p><m:oMathPara><m:oMath><m:f><m:num><m:r><m:t>1</m:t></m:r></m:num><m:den><m:r><m:t>2</m:t></m:r></m:den></m:f></m:oMath></m:oMathPara></w:p></w:body></w:document>"#,
        p(&r("Hello"))
    );
    let path = package(
        dir.path(),
        "hello.docx",
        &[
            (
                "_rels/.rels",
                rels(&[("rId1", "officeDocument", "word/document.xml")]),
            ),
            ("word/document.xml", doc.into_bytes()),
        ],
    );
    let deck = crate::render::Deck::open(&path).unwrap();
    assert_eq!(deck.pages, 1);
}

#[test]
fn a_code_line_holding_a_fence_does_not_end_its_block() {
    let styles = format!(
        r#"<w:styles {W}><w:style w:styleId="Src"><w:name w:val="Source Code"/></w:style></w:styles>"#
    );
    let code = |t: &str| {
        format!(
            "<w:p><w:pPr><w:pStyle w:val=\"Src\"/></w:pPr>{}</w:p>",
            r(t)
        )
    };
    let body = format!("{}{}{}", code("```"), code("code"), p(&r("after")));
    let md = docx_with(
        &body,
        &[("rId1", "styles", "styles.xml")],
        &[("word/styles.xml", styles.into_bytes())],
    )
    .markdown;
    assert_eq!(md, "````\n```\ncode\n````\n\nafter\n");
}

#[test]
fn a_list_starting_deep_is_a_list_not_code() {
    let numbering = format!(
        r#"<w:numbering {W}><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="1"><w:numFmt w:val="bullet"/></w:lvl><w:lvl w:ilvl="3"><w:numFmt w:val="bullet"/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>"#
    );
    let item = |level: u8, text: &str| {
        format!(
            "<w:p><w:pPr><w:numPr><w:ilvl w:val=\"{level}\"/><w:numId w:val=\"1\"/></w:numPr></w:pPr>{}</w:p>",
            r(text)
        )
    };
    let body = format!("{}{}{}", p(&r("Intro")), item(1, "one"), item(3, "deeper"));
    let md = docx_with(
        &body,
        &[("rId1", "numbering", "numbering.xml")],
        &[("word/numbering.xml", numbering.into_bytes())],
    )
    .markdown;
    assert_eq!(md, "Intro\n\n- one\n    - deeper\n");
}

#[test]
fn the_title_on_the_page_wins_over_the_properties() {
    let styles = format!(
        r#"<w:styles {W}><w:style w:styleId="T"><w:name w:val="Title"/></w:style></w:styles>"#
    );
    let core = r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Old template title</dc:title></cp:coreProperties>"#;
    let body = format!(
        "<w:p><w:pPr><w:pStyle w:val=\"T\"/></w:pPr>{}</w:p>",
        r("Real Title")
    );
    let md = docx_with(
        &body,
        &[("rId1", "styles", "styles.xml")],
        &[
            ("word/styles.xml", styles.into_bytes()),
            ("docProps/core.xml", core.as_bytes().to_vec()),
        ],
    )
    .markdown;
    assert!(md.starts_with("---\ntitle: \"Real Title\"\n"), "{md}");
}
