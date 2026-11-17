//! A Word document's body as markdown.

mod styles;

use super::inline::{Inline, Nesting, Style, code_block, front, monospace, table};
use super::media::Media;
use super::omml;
use super::package::{Package, Rels, first};
use super::xml::{attr, child, children, chosen, find, is, on, parse, val};
use roxmltree::Node;
use std::collections::HashMap;
use styles::{Numbering, Role, Styles, list_of};

/// A block of the markdown, as it is joined to the next.
enum Block {
    Para(String),
    /// A list item, at its level, marked: items one after another are one
    /// list.
    Item(u8, String),
    /// A line of code: lines one after another are one block.
    Code(String),
}

/// Authors Office writes for whoever was not named.
const NO_AUTHOR: [&str; 5] = [
    "microsoft office user",
    "windows user",
    "user",
    "administrator",
    "author",
];

pub(super) fn read(pkg: &mut Package, media: &mut Media) -> anyhow::Result<String> {
    let main = pkg.main("word/document.xml");
    let text = pkg
        .text(&main)
        .ok_or_else(|| anyhow::anyhow!("it has no document part, {main}"))?;
    let doc = parse(&text).map_err(|e| anyhow::anyhow!("its {main} cannot be read: {e}"))?;
    let rels = pkg.rels(&main);
    let styles = Styles::read(pkg, &rels);
    let numbering = Numbering::read(pkg, &rels);
    let mut w = Writer {
        pkg,
        media,
        styles,
        numbering,
        title: None,
        subtitle: None,
        notes: Vec::new(),
    };
    let mut blocks = Vec::new();
    let body = find(doc.root_element(), "w:body").unwrap_or(doc.root_element());
    w.blocks(body, &rels, &mut blocks);
    let mut md = join(&blocks);
    let notes = w.footnotes(&rels);
    if !notes.is_empty() {
        md.push_str("\n\n");
        md.push_str(&notes);
    }
    let (core_title, author) = w.pkg.core();
    // The title on the page, over the properties', which are often a
    // template's.
    let title = w.title.take().or(core_title);
    let author = author.filter(|a| !NO_AUTHOR.contains(&a.to_ascii_lowercase().as_str()));
    let head = front(
        title.as_deref(),
        w.subtitle.as_deref(),
        author.as_deref(),
        &[],
    );
    Ok(format!("{head}{md}\n"))
}

/// Blocks as markdown: list items and lines of code run together, the
/// rest a blank line apart.
fn join(blocks: &[Block]) -> String {
    let mut out = String::new();
    let mut nesting = Nesting::default();
    let mut code: Vec<&str> = Vec::new();
    let mut last_item = false;
    for (i, block) in blocks.iter().enumerate() {
        if let Block::Code(line) = block {
            code.push(line);
            if !matches!(blocks.get(i + 1), Some(Block::Code(_))) {
                push_block(&mut out, &code_block(&code), false);
                code.clear();
                nesting.end();
                last_item = false;
            }
            continue;
        }
        match block {
            Block::Item(level, s) => {
                let indent = "    ".repeat(nesting.depth(*level));
                push_block(&mut out, &format!("{indent}{s}"), last_item);
                last_item = true;
            }
            Block::Para(s) => {
                push_block(&mut out, s, false);
                nesting.end();
                last_item = false;
            }
            Block::Code(_) => {}
        }
    }
    out
}

/// `block` after `out`: on the next line if it goes on a list, else a blank
/// line apart.
fn push_block(out: &mut String, block: &str, tight: bool) {
    if !out.is_empty() {
        out.push_str(if tight { "\n" } else { "\n\n" });
    }
    out.push_str(block);
}

struct Writer<'p> {
    pkg: &'p mut Package,
    media: &'p mut Media,
    styles: Styles,
    numbering: Numbering,
    title: Option<String>,
    subtitle: Option<String>,
    /// Footnotes and endnotes referred to, in turn: whether an endnote, and
    /// its id.
    notes: Vec<(bool, String)>,
}

impl Writer<'_> {
    fn blocks(&mut self, n: Node, rels: &Rels, out: &mut Vec<Block>) {
        for c in n.children().filter(Node::is_element) {
            if is(c, "w:p") {
                self.paragraph(c, rels, out);
            } else if is(c, "w:tbl") {
                let rows = self.rows(c, rels);
                out.push(Block::Para(table(&rows)));
            } else if is(c, "w:sdt") {
                // A table of contents is left out: the headings are there.
                let toc = find(c, "w:docPartGallery")
                    .and_then(|g| attr(g, "w:val"))
                    .is_some_and(|v| v.contains("Table of Contents"));
                if let (false, Some(content)) = (toc, child(c, "w:sdtContent")) {
                    self.blocks(content, rels, out);
                }
            } else if is(c, "mc:AlternateContent") {
                if let Some(c) = chosen(c) {
                    self.blocks(c, rels, out);
                }
            } else if is(c, "m:oMathPara") {
                out.push(Block::Para(format!("$${}$$", omml::latex(c))));
            } else if is(c, "w:customXml") || is(c, "w:ins") || is(c, "w:moveTo") {
                self.blocks(c, rels, out);
            }
        }
    }

    fn paragraph(&mut self, p: Node, rels: &Rels, out: &mut Vec<Block>) {
        let ppr = child(p, "w:pPr");
        let style = ppr.and_then(|p| val(p, "w:pStyle")).unwrap_or("");
        let mut role = self.styles.role(style);
        if role == Role::Body
            && let Some(level) = ppr
                .and_then(|p| val(p, "w:outlineLvl"))
                .and_then(|v| v.parse::<u8>().ok())
                .filter(|v| *v < 9)
        {
            role = Role::Heading((level + 1).min(6));
        }
        if role == Role::Code {
            let mut line = String::new();
            raw_text(p, &mut line);
            for l in line.split('\n') {
                out.push(Block::Code(l.to_string()));
            }
            return;
        }
        let mut inline = Inline::default();
        self.inline(p, rels, None, &mut inline);
        if inline.is_empty() {
            return;
        }
        let text = inline.render();
        let list = ppr
            .and_then(list_of)
            .or_else(|| self.styles.list(style))
            // Word's list 0 is none.
            .filter(|(id, _)| id != "0");
        out.push(match role {
            Role::Title if self.title.is_none() => {
                self.title = Some(inline.plain());
                return;
            }
            Role::Subtitle if self.subtitle.is_none() => {
                self.subtitle = Some(inline.plain());
                return;
            }
            Role::Heading(level) => Block::Para(format!("{} {text}", "#".repeat(level.into()))),
            Role::Title => Block::Para(format!("# {text}")),
            Role::Quote => Block::Para(format!("> {text}")),
            _ => match list {
                Some((id, level)) => {
                    let mark = if self.numbering.ordered(&id, level) {
                        "1."
                    } else {
                        "-"
                    };
                    Block::Item(level, format!("{mark} {text}"))
                }
                None => Block::Para(text),
            },
        });
    }

    /// The runs under `n`, gathered into `out`, linked to `link`.
    fn inline(&mut self, n: Node, rels: &Rels, link: Option<&str>, out: &mut Inline) {
        for c in n.children().filter(Node::is_element) {
            if is(c, "w:r") {
                self.run(c, rels, link, out);
            } else if is(c, "w:hyperlink") {
                let url = attr(c, "r:id")
                    .and_then(|id| rels.get(id))
                    .filter(|r| r.external)
                    .map(|r| r.target.clone());
                self.inline(c, rels, url.as_deref().or(link), out);
            } else if is(c, "m:oMath") {
                let tex = omml::latex(c);
                if !tex.is_empty() {
                    out.markdown(format!("${tex}$"));
                }
            } else if is(c, "m:oMathPara") {
                out.markdown(format!("$${}$$", omml::latex(c)));
            } else if is(c, "w:sdt") {
                if let Some(content) = child(c, "w:sdtContent") {
                    self.inline(content, rels, link, out);
                }
            } else if is(c, "mc:AlternateContent") {
                if let Some(c) = chosen(c) {
                    self.inline(c, rels, link, out);
                }
            } else if [
                "w:ins",
                "w:smartTag",
                "w:customXml",
                "w:fldSimple",
                "w:moveTo",
                "w:dir",
                "w:bdo",
            ]
            .iter()
            .any(|name| is(c, name))
            {
                self.inline(c, rels, link, out);
            }
        }
    }

    fn run(&mut self, r: Node, rels: &Rels, link: Option<&str>, out: &mut Inline) {
        let rpr = child(r, "w:rPr");
        let prop = |name: &str| on(rpr.and_then(|p| child(p, name)));
        if prop("w:vanish") {
            return;
        }
        let (bold, italic, code) = rpr
            .and_then(|p| val(p, "w:rStyle"))
            .map_or((false, false, false), |id| self.styles.character(id));
        let font = rpr
            .and_then(|p| child(p, "w:rFonts"))
            .and_then(|f| attr(f, "w:ascii").or_else(|| attr(f, "w:hAnsi")))
            .unwrap_or("");
        let style = Style {
            bold: bold || prop("w:b"),
            italic: italic || prop("w:i"),
            strike: prop("w:strike") || prop("w:dstrike"),
            code: code || monospace(font),
        };
        self.run_content(r, rels, style, link, out);
    }

    fn run_content(
        &mut self,
        r: Node,
        rels: &Rels,
        style: Style,
        link: Option<&str>,
        out: &mut Inline,
    ) {
        for c in r.children().filter(Node::is_element) {
            let name = c.tag_name().name();
            if is(c, "mc:AlternateContent") {
                if let Some(c) = chosen(c) {
                    self.run_content(c, rels, style, link, out);
                }
                continue;
            }
            if !is(c, &format!("w:{name}")) {
                continue;
            }
            match name {
                "t" => out.text(c.text().unwrap_or(""), style, link),
                "tab" | "ptab" => out.text("\t", style, link),
                "br" if !matches!(attr(c, "w:type"), Some("page" | "column")) => {
                    out.text("\n", style, link);
                }
                "cr" => out.text("\n", style, link),
                "noBreakHyphen" => out.text("-", style, link),
                "sym" => {
                    let c = attr(c, "w:char")
                        .and_then(|h| u32::from_str_radix(h, 16).ok())
                        .and_then(char::from_u32)
                        .filter(|c| !('\u{F000}'..='\u{F0FF}').contains(c));
                    if let Some(c) = c {
                        out.text(&c.to_string(), style, link);
                    }
                }
                "drawing" | "pict" | "object" => out.markdown(self.pictures(c, rels)),
                "footnoteReference" | "endnoteReference" => {
                    if let Some(id) = attr(c, "w:id") {
                        let end = name == "endnoteReference";
                        self.notes.push((end, id.to_string()));
                        out.markdown(format!("[^{}{id}]", if end { "e" } else { "" }));
                    }
                }
                _ => {}
            }
        }
    }

    /// The pictures in a drawing, as markdown.
    fn pictures(&mut self, n: Node, rels: &Rels) -> String {
        let alt = find(n, "wp:docPr")
            .and_then(|d| attr(d, "descr").or_else(|| attr(d, "title")))
            .unwrap_or("");
        let ids: Vec<&str> = n
            .descendants()
            .filter_map(|d| {
                if is(d, "a:blip") {
                    attr(d, "r:embed")
                } else if is(d, "v:imagedata") {
                    attr(d, "r:id")
                } else {
                    None
                }
            })
            .collect();
        ids.into_iter()
            .map(|id| self.media.image(self.pkg, rels, id, alt))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// A table's rows, each a list of cells, a merged cell as many.
    fn rows(&mut self, tbl: Node, rels: &Rels) -> Vec<Vec<String>> {
        let mut rows = Vec::new();
        for tr in children(tbl, "w:tr") {
            let mut row = Vec::new();
            for tc in children(tr, "w:tc") {
                let span = child(tc, "w:tcPr")
                    .and_then(|p| val(p, "w:gridSpan"))
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(1)
                    .clamp(1, 64);
                row.push(self.cell(tc, rels));
                row.extend(std::iter::repeat_n(String::new(), span - 1));
            }
            rows.push(row);
        }
        rows
    }

    /// A cell's paragraphs on one line, a table in it as its text.
    fn cell(&mut self, tc: Node, rels: &Rels) -> String {
        let mut parts = Vec::new();
        for c in tc.children().filter(Node::is_element) {
            if is(c, "w:p") {
                let mut inline = Inline::default();
                self.inline(c, rels, None, &mut inline);
                if !inline.is_empty() {
                    parts.push(inline.render());
                }
            } else if is(c, "w:tbl") {
                let rows = self.rows(c, rels);
                parts.extend(rows.into_iter().map(|r| r.join(" ")));
            } else if is(c, "w:sdt")
                && let Some(content) = child(c, "w:sdtContent")
            {
                parts.push(self.cell(content, rels));
            }
        }
        parts.join("<br>")
    }

    /// The footnotes and endnotes referred to, as markdown's.
    fn footnotes(&mut self, rels: &Rels) -> String {
        let notes = std::mem::take(&mut self.notes);
        let mut out = Vec::new();
        for (end, kind) in [(false, "footnotes"), (true, "endnotes")] {
            let wanted: Vec<&String> = notes
                .iter()
                .filter(|(e, _)| *e == end)
                .map(|(_, id)| id)
                .collect();
            if wanted.is_empty() {
                continue;
            }
            let Some(part) = first(rels, kind) else {
                continue;
            };
            let Some(text) = self.pkg.text(&part) else {
                continue;
            };
            let Ok(doc) = parse(&text) else {
                continue;
            };
            let part_rels = self.pkg.rels(&part);
            let by_id: HashMap<&str, Node> = doc
                .root_element()
                .children()
                .filter(|n| n.is_element())
                .filter_map(|n| Some((attr(n, "w:id")?, n)))
                .collect();
            for id in wanted {
                let Some(note) = by_id.get(id.as_str()) else {
                    continue;
                };
                let mut blocks = Vec::new();
                self.blocks(*note, &part_rels, &mut blocks);
                let body: Vec<String> = blocks
                    .into_iter()
                    .map(|b| match b {
                        Block::Para(s) | Block::Item(_, s) | Block::Code(s) => s,
                    })
                    .collect();
                let mark = if end { "e" } else { "" };
                out.push(format!("[^{mark}{id}]: {}", body.join(" ")));
            }
        }
        out.join("\n")
    }
}

/// The text of the runs under `n`, as it is, for code.
fn raw_text(n: Node, out: &mut String) {
    for c in n.children().filter(Node::is_element) {
        if is(c, "w:t") {
            out.push_str(c.text().unwrap_or(""));
        } else if is(c, "w:tab") {
            out.push('\t');
        } else if is(c, "w:br") || is(c, "w:cr") {
            out.push('\n');
        } else if !is(c, "w:del") && !is(c, "w:pPr") && !is(c, "w:rPr") && !is(c, "w:instrText") {
            raw_text(c, out);
        }
    }
}
