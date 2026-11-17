//! A PowerPoint presentation's slides as markdown slides: each slide's
//! title, then its text, tables and pictures in the order they are in the
//! slide, then its speaker notes.

use super::inline::{Inline, Nesting, Style, front, monospace, table};
use super::media::Media;
use super::omml;
use super::package::{Package, Rels, first};
use super::xml::{attr, child, children, chosen, find, is, parse};
use roxmltree::Node;

/// What a slide holds.
#[derive(Default)]
struct Slide {
    title: Option<String>,
    /// A title slide's subtitle.
    subtitle: Option<String>,
    /// Whether its title is a title slide's.
    centered: bool,
    blocks: Vec<Block>,
    notes: Vec<String>,
}

enum Block {
    Para(String),
    /// A bullet, at its level, marked: bullets one after another are one
    /// list.
    Item(u8, String),
}

/// The presentation part's name and text.
pub(super) fn presentation(pkg: &mut Package) -> anyhow::Result<(String, String)> {
    let main = pkg.main("ppt/presentation.xml");
    let text = pkg
        .text(&main)
        .ok_or_else(|| anyhow::anyhow!("it has no presentation part, {main}"))?;
    Ok((main, text))
}

/// The slides' parts, in order, as the presentation `root`, the part
/// `main`, lists them.
pub(super) fn slide_parts(pkg: &mut Package, main: &str, root: Node) -> Vec<String> {
    let rels = pkg.rels(main);
    find(root, "p:sldIdLst")
        .into_iter()
        .flat_map(|list| children(list, "p:sldId"))
        .filter_map(|s| rels.get(attr(s, "r:id")?))
        .filter(|r| !r.external)
        .map(|r| r.target.clone())
        .collect()
}

/// A placeholder: its type and index.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Ph {
    pub kind: String,
    pub idx: u32,
}

/// The placeholder shape `sp` is, if it is one: of type `body` when it
/// names none.
pub(super) fn ph(sp: Node) -> Option<Ph> {
    let nv = sp
        .children()
        .find(|c| c.is_element() && c.tag_name().name().starts_with("nv"))?;
    let p = child(child(nv, "p:nvPr")?, "p:ph")?;
    Some(Ph {
        kind: attr(p, "type").unwrap_or("body").to_string(),
        idx: attr(p, "idx").and_then(|i| i.parse().ok()).unwrap_or(0),
    })
}

pub(super) fn read(pkg: &mut Package, media: &mut Media) -> anyhow::Result<String> {
    let (main, text) = presentation(pkg)?;
    let doc = parse(&text).map_err(|e| anyhow::anyhow!("its {main} cannot be read: {e}"))?;
    let parts = slide_parts(pkg, &main, doc.root_element());
    let mut slides = Vec::new();
    for part in parts {
        if let Some(slide) = read_slide(pkg, media, &part) {
            slides.push(slide);
        }
    }
    // A title slide that holds only its title is the deck's title.
    let (mut title, mut subtitle) = (None, None);
    if let Some(first) = slides.first()
        && (first.centered || first.subtitle.is_some())
        && first.blocks.is_empty()
        && first.notes.is_empty()
    {
        title = first.title.clone();
        subtitle = first.subtitle.clone();
        slides.remove(0);
    }
    // A subtitle on any other slide is its first line.
    for slide in &mut slides {
        if let Some(sub) = slide.subtitle.take() {
            slide.blocks.insert(0, Block::Para(sub));
        }
    }
    let (core_title, author) = pkg.core();
    let title = title.or(core_title);
    let head = front(
        title.as_deref(),
        subtitle.as_deref(),
        author.as_deref(),
        &[
            "output:",
            "  powerpoint_presentation:",
            "    slide_level: 2",
        ],
    );
    let body: Vec<String> = slides.iter().map(render).collect();
    Ok(format!("{head}{}\n", body.join("\n\n---\n\n")))
}

fn render(slide: &Slide) -> String {
    let mut out = String::new();
    if let Some(title) = &slide.title {
        out.push_str(&format!("## {title}"));
    }
    let mut last_item = false;
    let mut nesting = Nesting::default();
    for block in &slide.blocks {
        let (item, s) = match block {
            Block::Item(level, s) => {
                let indent = "    ".repeat(nesting.depth(*level));
                (true, format!("{indent}{s}"))
            }
            Block::Para(s) => {
                nesting.end();
                (false, s.clone())
            }
        };
        if !out.is_empty() {
            out.push_str(if item && last_item { "\n" } else { "\n\n" });
        }
        out.push_str(&s);
        last_item = item;
    }
    if !slide.notes.is_empty() {
        out.push_str("\n\n::: notes\n");
        out.push_str(&slide.notes.join("\n\n"));
        out.push_str("\n:::");
    }
    out
}

fn read_slide(pkg: &mut Package, media: &mut Media, part: &str) -> Option<Slide> {
    let text = pkg.text(part)?;
    let doc = parse(&text).ok()?;
    let root = doc.root_element();
    if attr(root, "show") == Some("0") {
        return None;
    }
    let rels = pkg.rels(part);
    let mut slide = Slide::default();
    let mut w = Writer {
        pkg,
        media,
        rels: &rels,
    };
    if let Some(tree) = find(root, "p:spTree") {
        w.shapes(tree, &mut slide);
    }
    if let Some(notes) = first(&rels, "notesSlide") {
        slide.notes = speaker_notes(pkg, &notes);
    }
    Some(slide)
}

/// The text of a notes page's body.
pub(super) fn speaker_notes(pkg: &mut Package, part: &str) -> Vec<String> {
    let Some(text) = pkg.text(part) else {
        return Vec::new();
    };
    let Ok(doc) = parse(&text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for sp in doc.descendants().filter(|n| is(*n, "p:sp")) {
        if ph(sp).map(|p| p.kind).as_deref() != Some("body") {
            continue;
        }
        let Some(body) = child(sp, "p:txBody") else {
            continue;
        };
        for p in children(body, "a:p") {
            let mut inline = Inline::default();
            paragraph_text(p, None, &mut inline);
            if !inline.is_empty() {
                out.push(inline.render());
            }
        }
    }
    out
}

struct Writer<'p, 'r> {
    pkg: &'p mut Package,
    media: &'p mut Media,
    rels: &'r Rels,
}

impl Writer<'_, '_> {
    fn shapes(&mut self, tree: Node, slide: &mut Slide) {
        for c in tree.children().filter(Node::is_element) {
            if is(c, "p:sp") {
                self.shape(c, slide);
            } else if is(c, "p:pic") {
                self.picture(c, slide);
            } else if is(c, "p:graphicFrame") {
                if let Some(tbl) = find(c, "a:tbl") {
                    let rows = self.rows(tbl);
                    slide.blocks.push(Block::Para(table(&rows)));
                }
            } else if is(c, "p:grpSp") {
                self.shapes(c, slide);
            } else if is(c, "mc:AlternateContent")
                && let Some(c) = chosen(c)
            {
                self.shapes(c, slide);
            }
        }
    }

    fn shape(&mut self, sp: Node, slide: &mut Slide) {
        let kind = ph(sp).map(|p| p.kind);
        let Some(body) = child(sp, "p:txBody") else {
            return;
        };
        match kind.as_deref() {
            Some("dt" | "ftr" | "sldNum" | "hdr") => {}
            Some(kind @ ("title" | "ctrTitle")) if slide.title.is_none() => {
                let lines = self.lines(body);
                slide.title = Some(lines.join(" ")).filter(|t| !t.is_empty());
                slide.centered = kind == "ctrTitle";
            }
            Some("subTitle") if slide.subtitle.is_none() => {
                let lines = self.lines(body);
                slide.subtitle = Some(lines.join("<br>")).filter(|t| !t.is_empty());
            }
            kind => {
                let bulleted = matches!(kind, Some("body" | "obj"));
                self.text_body(body, bulleted, slide);
            }
        }
    }

    /// The paragraphs of a text body, each on its own.
    fn lines(&mut self, body: Node) -> Vec<String> {
        children(body, "a:p")
            .filter_map(|p| {
                let mut inline = Inline::default();
                paragraph_text(p, Some(self.rels), &mut inline);
                (!inline.is_empty()).then(|| inline.render())
            })
            .collect()
    }

    /// A text body's paragraphs, a placeholder's as bullets unless they say
    /// they are not, any other's only if they say they are.
    fn text_body(&mut self, body: Node, bulleted: bool, slide: &mut Slide) {
        for p in children(body, "a:p") {
            let mut inline = Inline::default();
            paragraph_text(p, Some(self.rels), &mut inline);
            if inline.is_empty() {
                continue;
            }
            let text = inline.render();
            let ppr = child(p, "a:pPr");
            let level: u8 = ppr
                .and_then(|p| attr(p, "lvl"))
                .and_then(|l| l.parse().ok())
                .unwrap_or(0)
                .min(8);
            let none = ppr.is_some_and(|p| child(p, "a:buNone").is_some());
            let numbered = ppr.is_some_and(|p| child(p, "a:buAutoNum").is_some());
            let bullet = ppr.is_some_and(|p| child(p, "a:buChar").is_some());
            if !none && (bulleted || numbered || bullet) {
                let mark = if numbered { "1." } else { "-" };
                slide
                    .blocks
                    .push(Block::Item(level, format!("{mark} {text}")));
            } else {
                slide.blocks.push(Block::Para(text));
            }
        }
    }

    fn picture(&mut self, pic: Node, slide: &mut Slide) {
        let alt = find(pic, "p:cNvPr")
            .and_then(|c| attr(c, "descr").or_else(|| attr(c, "title")))
            .unwrap_or("");
        let Some(id) = find(pic, "a:blip").and_then(|b| attr(b, "r:embed")) else {
            return;
        };
        let md = self.media.image(self.pkg, self.rels, id, alt);
        if !md.is_empty() {
            slide.blocks.push(Block::Para(md));
        }
    }

    fn rows(&mut self, tbl: Node) -> Vec<Vec<String>> {
        children(tbl, "a:tr")
            .map(|tr| {
                children(tr, "a:tc")
                    .map(|tc| {
                        // A cell merged into the one before it is empty.
                        child(tc, "a:txBody")
                            .map_or_else(String::new, |b| self.lines(b).join("<br>"))
                    })
                    .collect()
            })
            .collect()
    }
}

/// A paragraph's runs, gathered into `out`; their links read from `rels`.
fn paragraph_text(p: Node, rels: Option<&Rels>, out: &mut Inline) {
    for c in p.children().filter(Node::is_element) {
        if is(c, "a:r") || is(c, "a:fld") {
            if is(c, "a:fld") && attr(c, "type") == Some("slidenum") {
                continue;
            }
            let rpr = child(c, "a:rPr");
            let flag = |name: &str| {
                rpr.and_then(|p| attr(p, name))
                    .is_some_and(|v| v == "1" || v == "true")
            };
            let font = rpr
                .and_then(|p| child(p, "a:latin"))
                .and_then(|f| attr(f, "typeface"))
                .unwrap_or("");
            let style = Style {
                bold: flag("b"),
                italic: flag("i"),
                strike: rpr
                    .and_then(|p| attr(p, "strike"))
                    .is_some_and(|s| s != "noStrike"),
                code: monospace(font),
            };
            let link = rels.and_then(|rels| {
                let id = attr(child(rpr?, "a:hlinkClick")?, "r:id")?;
                rels.get(id)
                    .filter(|r| r.external)
                    .map(|r| r.target.as_str())
            });
            let text: String = children(c, "a:t").filter_map(|t| t.text()).collect();
            out.text(&text, style, link);
        } else if is(c, "a:br") {
            out.text("\n", Style::default(), None);
        } else if is(c, "a:pPr") || is(c, "a:endParaRPr") {
        } else if let Some(math) = c
            .descendants()
            .find(|d| is(*d, "m:oMathPara") || is(*d, "m:oMath"))
        {
            let tex = omml::latex(math);
            if !tex.is_empty() {
                out.markdown(format!("${tex}$"));
            }
        }
    }
}
