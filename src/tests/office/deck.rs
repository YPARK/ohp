use super::*;
use std::io::Write as _;

const NS: &str = r#"xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn rels(entries: &[(&str, &str, &str)]) -> String {
    let mut s = String::from(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
    for (id, kind, target) in entries {
        s.push_str(&format!(
            r#"<Relationship Id="{id}" Type="{REL}/{kind}" Target="{target}"/>"#
        ));
    }
    s + "</Relationships>"
}

/// A deck of one slide of `shapes`, on slides `cx` by `cy` EMU, whose
/// layout holds `layout_shapes` and whose master's body text is 32 points.
fn deck(dir: &Path, cx: u64, cy: u64, shapes: &str, layout_shapes: &str) -> std::path::PathBuf {
    deck_with(dir, cx, cy, shapes, layout_shapes, &png())
}

/// A two-by-two PNG.
fn png() -> Vec<u8> {
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::RgbImage::new(2, 2)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

/// As `deck`, its picture `picture`.
fn deck_with(
    dir: &Path,
    cx: u64,
    cy: u64,
    shapes: &str,
    layout_shapes: &str,
    picture: &[u8],
) -> std::path::PathBuf {
    let path = dir.join("deck.pptx");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let tree = |s: &str| format!("<p:cSld><p:spTree>{s}</p:spTree></p:cSld>");
    let parts: Vec<(&str, Vec<u8>)> = vec![
        ("_rels/.rels", rels(&[("r1", "officeDocument", "ppt/presentation.xml")]).into_bytes()),
        (
            "ppt/presentation.xml",
            format!(r#"<p:presentation {NS}><p:sldIdLst><p:sldId id="256" r:id="s1"/></p:sldIdLst><p:sldSz cx="{cx}" cy="{cy}"/></p:presentation>"#).into_bytes(),
        ),
        ("ppt/_rels/presentation.xml.rels", rels(&[("s1", "slide", "slides/slide1.xml")]).into_bytes()),
        ("ppt/slides/slide1.xml", format!("<p:sld {NS}>{}</p:sld>", tree(shapes)).into_bytes()),
        (
            "ppt/slides/_rels/slide1.xml.rels",
            rels(&[("l1", "slideLayout", "../slideLayouts/slideLayout1.xml"), ("i1", "image", "../media/pic.png")]).into_bytes(),
        ),
        ("ppt/slideLayouts/slideLayout1.xml", format!("<p:sldLayout {NS}>{}</p:sldLayout>", tree(layout_shapes)).into_bytes()),
        (
            "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
            rels(&[("m1", "slideMaster", "../slideMasters/slideMaster1.xml")]).into_bytes(),
        ),
        (
            "ppt/slideMasters/slideMaster1.xml",
            format!(r#"<p:sldMaster {NS}>{}<p:txStyles><p:bodyStyle><a:lvl1pPr><a:buChar char="•"/><a:defRPr sz="3200"/></a:lvl1pPr></p:bodyStyle></p:txStyles></p:sldMaster>"#, tree("")).into_bytes(),
        ),
        ("ppt/media/pic.png", picture.to_vec()),
    ];
    for (name, bytes) in parts {
        zip.start_file(name, stored).unwrap();
        zip.write_all(&bytes).unwrap();
    }
    zip.finish().unwrap();
    path
}

fn xfrm(x: u64, y: u64, cx: u64, cy: u64) -> String {
    format!(r#"<a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm>"#)
}

fn body_ph(spr: &str, text: &str) -> String {
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="b"/><p:cNvSpPr/><p:nvPr><p:ph idx="1"/></p:nvPr></p:nvSpPr><p:spPr>{spr}</p:spPr><p:txBody><a:bodyPr/><a:p><a:r><a:t>{text}</a:t></a:r></a:p></p:txBody></p:sp>"#
    )
}

#[test]
fn a_slide_is_a_page_of_the_deck_s_size_with_its_shapes_where_they_are() {
    let dir = tempfile::tempdir().unwrap();
    let pic = format!(
        r#"<p:pic><p:nvPicPr><p:cNvPr id="3" name="p"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="i1"/></p:blipFill><p:spPr>{}</p:spPr></p:pic>"#,
        xfrm(914_400, 457_200, 1_270_000, 635_000)
    );
    let text = r#"<p:sp><p:nvSpPr><p:cNvPr id="4" name="t"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="2540000" cy="508000"/></a:xfrm></p:spPr><p:txBody><a:bodyPr/><a:p><a:r><a:rPr sz="2400" b="1"/><a:t>Big</a:t></a:r></a:p></p:txBody></p:sp>"#;
    // 4:3, 720 by 540 points.
    let path = deck(
        dir.path(),
        9_144_000,
        6_858_000,
        &format!("{pic}{text}"),
        "",
    );
    let laid = lay_out(&path, &no_stop()).unwrap();
    assert!(
        laid.source
            .contains("#set page(width: 720.00pt, height: 540.00pt"),
        "{}",
        laid.source
    );
    assert!(
        laid.source.contains(
            "dx: 72.00pt, dy: 36.00pt, image(\"/office-1.png\", width: 100.00pt, height: 50.00pt"
        ),
        "{}",
        laid.source
    );
    assert!(
        laid.source.contains("size: 24.00pt, weight: \"bold\""),
        "{}",
        laid.source
    );
    assert_eq!(laid.pictures.len(), 1);
}

#[test]
fn a_placeholder_takes_its_place_from_its_layout_and_its_size_from_the_master() {
    let dir = tempfile::tempdir().unwrap();
    let on_layout = body_ph(
        &xfrm(1_270_000, 2_540_000, 6_350_000, 1_270_000),
        "Click to edit",
    );
    let path = deck(
        dir.path(),
        9_144_000,
        6_858_000,
        &body_ph("", "Point"),
        &on_layout,
    );
    let laid = lay_out(&path, &no_stop()).unwrap();
    let src = &laid.source;
    // The layout's placeholder is not drawn: only the slide's text is.
    assert!(!src.contains("Click to edit"), "{src}");
    assert!(
        src.contains("dx: 100.00pt, dy: 200.00pt, block(width: 500.00pt, height: 100.00pt"),
        "{src}"
    );
    assert!(src.contains("size: 32.00pt"), "{src}");
    assert!(src.contains("\"•\""), "{src}");
}

#[test]
fn a_laid_out_deck_is_typeset() {
    let dir = tempfile::tempdir().unwrap();
    let shapes = body_ph(&xfrm(0, 0, 6_350_000, 1_270_000), "Hello");
    let path = deck(dir.path(), 12_192_000, 6_858_000, &shapes, "");
    // The picture is not used, so its bytes not being a PNG does not matter.
    let deck = crate::render::Deck::open(&path).unwrap();
    assert_eq!(deck.pages, 1);
    let (w, h) = deck.sizes[0];
    assert!(
        (w - 960.0).abs() < 0.5 && (h - 540.0).abs() < 0.5,
        "{w}×{h}"
    );
}

fn no_stop() -> AtomicBool {
    AtomicBool::new(false)
}

fn pic() -> String {
    format!(
        r#"<p:pic><p:nvPicPr><p:cNvPr id="3" name="figure"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="i1"/></p:blipFill><p:spPr>{}</p:spPr></p:pic>"#,
        xfrm(0, 0, 1_270_000, 635_000)
    )
}

#[test]
fn a_picture_that_cannot_be_read_is_an_outline_not_a_failure() {
    let dir = tempfile::tempdir().unwrap();
    let path = deck_with(dir.path(), 9_144_000, 6_858_000, &pic(), "", b"not a png");
    let laid = lay_out(&path, &no_stop()).unwrap();
    assert!(laid.pictures.is_empty());
    assert!(laid.source.contains("\"figure\""), "{}", laid.source);
    assert_eq!(laid.notes, ["1 picture that cannot be read left out"]);
    assert_eq!(crate::render::Deck::open(&path).unwrap().pages, 1);
}

#[test]
fn laying_out_stops_when_asked() {
    let dir = tempfile::tempdir().unwrap();
    let path = deck(dir.path(), 9_144_000, 6_858_000, &pic(), "");
    let err = lay_out(&path, &AtomicBool::new(true)).err().unwrap();
    assert!(err.is::<crate::render::Stopped>());
}

fn numbered_paras(paras: &[(u8, &str)]) -> String {
    let body: String = paras
        .iter()
        .map(|(lvl, t)| {
            format!(r#"<a:p><a:pPr lvl="{lvl}"><a:buAutoNum type="alphaLcParenR"/></a:pPr><a:r><a:t>{t}</a:t></a:r></a:p>"#)
        })
        .collect();
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="5" name="n"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr>{}</p:spPr><p:txBody><a:bodyPr/>{body}</p:txBody></p:sp>"#,
        xfrm(0, 0, 2_540_000, 2_540_000)
    )
}

#[test]
fn numbering_restarts_under_a_shallower_item_and_follows_its_scheme() {
    let dir = tempfile::tempdir().unwrap();
    let shapes = numbered_paras(&[(0, "Alpha"), (1, "Inner"), (0, "Beta"), (1, "Second")]);
    let path = deck(dir.path(), 9_144_000, 6_858_000, &shapes, "");
    let src = lay_out(&path, &no_stop()).unwrap().source;
    let marks: Vec<&str> = src
        .match_indices("\"a)\"")
        .chain(src.match_indices("\"b)\""))
        .map(|(_, m)| m)
        .collect();
    // a) Alpha, a) Inner, b) Beta, a) Second.
    assert_eq!(src.matches("\"a)\"").count(), 3, "{src}");
    assert_eq!(src.matches("\"b)\"").count(), 1, "{marks:?}");
}

#[test]
fn a_slide_number_takes_no_body_bullet() {
    let dir = tempfile::tempdir().unwrap();
    let num = r#"<p:sp><p:nvSpPr><p:cNvPr id="6" name="n"/><p:cNvSpPr/><p:nvPr><p:ph type="sldNum" idx="12"/></p:nvPr></p:nvSpPr><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1270000" cy="635000"/></a:xfrm></p:spPr><p:txBody><a:bodyPr/><a:p><a:r><a:t>SEVEN</a:t></a:r></a:p></p:txBody></p:sp>"#;
    let path = deck(dir.path(), 9_144_000, 6_858_000, num, "");
    let src = lay_out(&path, &no_stop()).unwrap().source;
    assert!(src.contains("SEVEN") && !src.contains("\"•\""), "{src}");
}
