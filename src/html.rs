//! HTML as markdown, typeset as markdown is.
//!
//! The page set is the first `article`, or else `main`, or else the body.
//! Scripts, styles, navigation, footers and a table of contents are left
//! out. An image embedded in the file is written beside it, and the
//! markdown names that file, so the image is shown.
//!
//! A formula in `\(...\)` or `\[...\]`, in a MathML annotation, or in the
//! TeX KaTeX or MathJax keep beside their drawing, is set as one written
//! in markdown. A picture drawn in the page is shown, and so is a mermaid
//! flowchart drawn from its source: the page's script that would draw it
//! does not run. A reveal.js deck, the sections in `.slides`, is
//! slides broken at rules. Remark's and xaringan's markdown, in a
//! `textarea` of id `source`, is that markdown.

use base64::Engine;
use scraper::{ElementRef, Html, Selector};
use std::path::Path;

/// HTML read as markdown.
pub struct Read {
    pub markdown: String,
    /// Images embedded in the file. The markdown names them, so this has to
    /// be kept while they are typeset.
    images: Option<tempfile::TempDir>,
}

impl Read {
    /// Where images embedded in the HTML were written, when there were any.
    pub(crate) fn files(&self) -> Option<&Path> {
        self.images.as_ref().map(tempfile::TempDir::path)
    }
}

/// `html` as markdown. Images embedded in it live as long as the result.
pub fn read(html: &str) -> Read {
    let doc = Html::parse_document(html);
    if let Some(source) = remark(&doc) {
        return Read {
            markdown: remark_markdown(&source),
            images: None,
        };
    }
    let mut writer = Writer::new();
    let slide = slides(&doc);
    if let Some(root) = slide {
        writer.deck(root);
    } else {
        writer.walk_children(content(&doc));
    }
    Read {
        markdown: front(&doc, &writer.body(), slide.is_some()),
        images: writer.images,
    }
}

/// Remark and xaringan keep their markdown in the page, and the browser
/// sets it. A deck with no front matter of its own is slides anyway.
fn remark(doc: &Html) -> Option<String> {
    let text: String = doc
        .select(&selector("textarea#source"))
        .next()?
        .text()
        .collect();
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn remark_markdown(text: &str) -> String {
    // The same openings markdown already treats as front matter.
    if text.starts_with("---\n")
        || text.starts_with("---\r\n")
        || text.starts_with("+++\n")
        || text.starts_with("+++\r\n")
    {
        return format!("{text}\n");
    }
    format!("---\noutput: revealjs\n---\n\n{text}\n")
}

fn slides(doc: &Html) -> Option<ElementRef<'_>> {
    let reveal = selector(".reveal .slides");
    let plain = selector(".slides");
    doc.select(&reveal)
        .find(|el| contains_section(*el))
        .or_else(|| doc.select(&plain).find(|el| contains_section(*el)))
}

fn contains_section(el: ElementRef) -> bool {
    el.descendent_elements()
        .any(|e| e.value().name() == "section" && e != el)
}

fn content(doc: &Html) -> ElementRef<'_> {
    for sel in ["article", "main", "[role=\"main\"]", "body"] {
        if let Some(el) = doc.select(&selector(sel)).next() {
            return el;
        }
    }
    doc.root_element()
}

/// Front matter markdown already understands: a title, and slides when the
/// page is a deck. A heading that only repeats the title is left out.
fn front(doc: &Html, body: &str, slides: bool) -> String {
    let mut lines = Vec::new();
    if slides {
        lines.push("output: revealjs".to_string());
    }
    if let Some(lang) = page_lang(doc) {
        lines.push(format!("lang: {lang}"));
    }
    let mut body = body.trim().to_string();
    if !slides {
        if let Some(title) = page_title(doc) {
            body = strip_title(&body, &title);
            lines.push(format!("title: {}", yaml_quote(&title)));
        }
        if let Some(author) = meta(doc, &["author", "citation_author"]) {
            lines.push(format!("author: {}", yaml_quote(&author)));
        }
        if let Some(date) = meta(doc, &["date", "citation_date", "dc.date"]) {
            lines.push(format!("date: {}", yaml_quote(&date)));
        }
    }
    if lines.is_empty() {
        return body;
    }
    let mut out = format!("---\n{}\n---\n", lines.join("\n"));
    if !body.is_empty() {
        out.push('\n');
        out.push_str(&body);
        out.push('\n');
    }
    out
}

fn page_title(doc: &Html) -> Option<String> {
    let title = words(
        &doc.select(&selector("title"))
            .next()?
            .text()
            .collect::<String>(),
    );
    (!title.is_empty()).then_some(title)
}

fn page_lang(doc: &Html) -> Option<String> {
    let lang = doc.select(&selector("html")).next()?.attr("lang")?;
    let lang = lang.split_whitespace().next()?.trim();
    ((1..=20).contains(&lang.len()) && lang.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        .then(|| lang.to_string())
}

fn meta(doc: &Html, names: &[&str]) -> Option<String> {
    for el in doc.select(&selector("meta")) {
        let name = el.attr("name").unwrap_or("");
        if names.iter().any(|n| name.eq_ignore_ascii_case(n)) {
            let value = words(el.attr("content").unwrap_or(""));
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

fn strip_title(body: &str, title: &str) -> String {
    let body = body.trim_start();
    let Some(rest) = body.strip_prefix("# ") else {
        return body.trim().to_string();
    };
    let (head, after) = rest.split_once('\n').unwrap_or((rest, ""));
    if head.trim() == escape_to_string(title) {
        after.trim().to_string()
    } else {
        body.trim().to_string()
    }
}

fn yaml_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn selector(sel: &str) -> Selector {
    Selector::parse(sel).expect("selector")
}

/// One cell of a markdown table, and how it is aligned.
type Cell = (String, Option<&'static str>);

struct Writer {
    out: String,
    /// Spaces before a continuation line.
    indent: usize,
    /// The next text starts a line: it is indented, and not spaced.
    at_line_start: bool,
    /// A space belongs before the next word.
    space: bool,
    /// A list marker was just written; its first text stays on that line.
    after_marker: bool,
    /// A slide was opened, so the next one is broken with a rule.
    opened: bool,
    images: Option<tempfile::TempDir>,
    nimage: usize,
}

impl Writer {
    fn new() -> Self {
        Writer {
            out: String::new(),
            indent: 0,
            at_line_start: true,
            space: false,
            after_marker: false,
            opened: false,
            images: None,
            nimage: 0,
        }
    }

    fn body(&self) -> String {
        self.out.trim().to_string()
    }

    fn walk_children(&mut self, el: ElementRef) {
        self.walk(el, false);
    }

    fn walk(&mut self, el: ElementRef, skip_sections: bool) {
        for child in el.children() {
            if let Some(inner) = ElementRef::wrap(child) {
                if skip_sections && inner.value().name() == "section" {
                    continue;
                }
                self.element(inner);
            } else if let Some(text) = child.value().as_text() {
                self.prose(text);
            }
        }
    }

    fn element(&mut self, el: ElementRef) {
        if let Some((tex, display)) = formula_of(el) {
            self.formula(&tex, display);
            return;
        }
        // A drawn SVG is the picture. One marked hidden is decoration.
        if el.value().name() == "svg" && !skipped(el) {
            self.drawing(el);
            return;
        }
        if diagram_lang(el).is_some() {
            self.diagram(el);
            return;
        }
        if skipped(el) {
            if matches!(el.value().name(), "object" | "embed") && image_src(el).is_some() {
                self.image(el);
            }
            return;
        }
        if let Some(level) = heading_level(el.value().name()) {
            self.heading(level, el);
            return;
        }
        match el.value().name() {
            "p" | "figcaption" | "summary" | "caption" | "dd" => self.block(el),
            "dt" => self.term(el),
            "br" => self.line_break(),
            "hr" => self.rule(),
            "em" | "i" | "dfn" | "cite" | "var" => self.wrap("*", el),
            "strong" | "b" => self.wrap("**", el),
            "s" | "del" | "strike" => self.wrap("~~", el),
            "code" | "tt" | "kbd" | "samp" => self.inline_code(el),
            "pre" => self.code_block(el),
            "a" => self.link(el),
            "img" => self.image(el),
            "ul" => self.list(el, false),
            "ol" => self.list(el, true),
            "blockquote" => self.quote(el),
            "table" => self.table(el),
            "sup" => self.script(el, true),
            "sub" => self.script(el, false),
            "dl" | "div" | "section" | "article" | "header" | "figure" | "main" | "details"
            | "center" | "picture" => self.block(el),
            _ => self.walk_children(el),
        }
    }

    fn deck(&mut self, root: ElementRef) {
        let subs: Vec<ElementRef> = root
            .child_elements()
            .filter(|c| c.value().name() == "section")
            .collect();
        if subs.is_empty() {
            for child in root.child_elements() {
                if contains_section(child) {
                    self.deck(child);
                }
            }
            return;
        }
        for sub in subs {
            self.section(sub);
        }
    }

    fn section(&mut self, el: ElementRef) {
        if skipped(el) {
            return;
        }
        let nested: Vec<ElementRef> = el
            .child_elements()
            .filter(|c| c.value().name() == "section")
            .collect();
        // A formula on the section itself is a slide. Nested sections are
        // slides of their own, so they are not this one's text.
        let own = if nested.is_empty() {
            has_body(el)
        } else {
            children_have_body(el)
        };
        if own {
            self.break_slide();
            self.walk(el, !nested.is_empty());
        }
        for sub in nested {
            self.section(sub);
        }
    }

    fn break_slide(&mut self) {
        self.indent = 0;
        self.after_marker = false;
        self.space = false;
        if self.opened {
            if !self.out.ends_with('\n') {
                self.out.push('\n');
            }
            if !self.out.ends_with("\n\n") {
                self.out.push('\n');
            }
            self.out.push_str("---\n\n");
        }
        self.opened = true;
        self.at_line_start = true;
    }

    /// Run `write` into its own buffer, so a link or a cell can be one line.
    fn capture(&mut self, write: impl FnOnce(&mut Self)) -> String {
        let out = std::mem::take(&mut self.out);
        let (indent, at_line_start, space, after_marker) = (
            self.indent,
            self.at_line_start,
            self.space,
            self.after_marker,
        );
        self.indent = 0;
        self.at_line_start = true;
        self.space = false;
        self.after_marker = false;
        write(self);
        self.indent = indent;
        self.at_line_start = at_line_start;
        self.space = space;
        self.after_marker = after_marker;
        std::mem::replace(&mut self.out, out)
    }

    fn block(&mut self, el: ElementRef) {
        if !self.after_marker {
            self.block_break();
        }
        self.walk_children(el);
        // An empty paragraph leaves a list marker for the text after it.
        if !self.after_marker {
            self.block_break();
        }
    }

    fn block_break(&mut self) {
        self.after_marker = false;
        self.space = false;
        if self.out.is_empty() {
            self.at_line_start = true;
            return;
        }
        if !self.out.ends_with('\n') {
            self.out.push('\n');
        }
        if !self.out.ends_with("\n\n") {
            self.out.push('\n');
        }
        self.at_line_start = true;
    }

    fn ensure_line(&mut self) {
        if self.at_line_start {
            self.out.push_str(&" ".repeat(self.indent));
            self.at_line_start = false;
        }
    }

    fn heading(&mut self, level: u8, el: ElementRef) {
        let text = words(&self.capture(|w| w.walk_children(el)));
        if text.is_empty() {
            return;
        }
        if self.after_marker && !self.out.ends_with('\n') {
            self.out.push('\n');
            self.after_marker = false;
            self.at_line_start = true;
        }
        self.block_break();
        self.ensure_line();
        self.out.push_str(&"#".repeat(level as usize));
        self.out.push(' ');
        self.out.push_str(&text);
        self.block_break();
    }

    fn term(&mut self, el: ElementRef) {
        if !self.after_marker {
            self.block_break();
        }
        self.wrap("**", el);
        self.block_break();
    }

    fn wrap(&mut self, marker: &str, el: ElementRef) {
        let inner = self.capture(|w| w.walk_children(el));
        let lead = inner.starts_with(|c: char| c.is_whitespace());
        let trail = inner.ends_with(|c: char| c.is_whitespace());
        let inner = inner.trim();
        if inner.is_empty() {
            if lead || trail {
                self.space = true;
            }
            return;
        }
        if lead {
            self.space = true;
        }
        self.inline(&format!("{marker}{inner}{marker}"));
        self.space = trail;
    }

    fn inline(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        let separate = self.space && !self.at_line_start;
        self.ensure_line();
        if separate {
            self.out.push(' ');
        }
        self.out.push_str(s);
        self.space = false;
        self.after_marker = false;
    }

    fn prose(&mut self, s: &str) {
        let mut start = 0;
        let mut i = 0;
        while i < s.len() {
            let rest = &s[i..];
            let (len, close, display) = if rest.starts_with("\\[") {
                (2, "\\]", true)
            } else if rest.starts_with("\\(") {
                (2, "\\)", false)
            } else {
                i += s[i..].chars().next().map_or(1, char::len_utf8);
                continue;
            };
            if let Some(end) = rest[len..].find(close) {
                let tex = rest[len..len + end].trim();
                if !tex.is_empty() {
                    self.plain(&s[start..i]);
                    self.formula(tex, display);
                    i += len + end + close.len();
                    start = i;
                    continue;
                }
            }
            // A backslash that does not open a formula.
            i += 1;
        }
        if start < s.len() {
            self.plain(&s[start..]);
        }
    }

    fn plain(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        let lead = s.starts_with(|c: char| c.is_whitespace());
        let trail = s.ends_with(|c: char| c.is_whitespace());
        let word = words(s);
        if word.is_empty() {
            if lead {
                self.space = true;
            }
            return;
        }
        // The list marker already ends with a space.
        let separate = (lead || self.space) && !self.at_line_start && !self.after_marker;
        self.ensure_line();
        if separate {
            self.out.push(' ');
        }
        push_escaped(&mut self.out, &word);
        self.space = trail;
        self.after_marker = false;
    }

    fn line_break(&mut self) {
        if !self.out.ends_with('\n') {
            self.out.push_str("  \n");
        } else {
            self.out.push('\n');
        }
        self.at_line_start = true;
        self.space = false;
        self.after_marker = false;
    }

    fn rule(&mut self) {
        // `<hr>` stays a line inside a slide; a markdown rule would split it.
        self.block_break();
        self.ensure_line();
        self.out.push_str("<hr>");
        self.block_break();
    }

    fn raw_line(&mut self, line: &str) {
        self.ensure_line();
        self.out.push_str(line);
        self.out.push('\n');
        self.at_line_start = true;
        self.space = false;
        self.after_marker = false;
    }

    fn formula(&mut self, tex: &str, display: bool) {
        let Some((tex, display)) = bare_tex(tex, display) else {
            return;
        };
        if display {
            self.block_break();
            self.raw_line("$$");
            for line in tex.lines() {
                self.raw_line(line.trim_end());
            }
            self.raw_line("$$");
            self.block_break();
            return;
        }
        self.inline(&format!("${}$", tex.replace('\n', " ")));
    }

    fn script(&mut self, el: ElementRef, up: bool) {
        let text = words(&el.text().collect::<String>());
        if !simple_script(&text) {
            self.plain(&text);
            return;
        }
        let tex = brace_escape(&text);
        let tex = if up {
            format!("^{{{tex}}}")
        } else {
            format!("_{{{tex}}}")
        };
        self.formula(&tex, false);
    }

    fn inline_code(&mut self, el: ElementRef) {
        let text: String = el.text().collect();
        if text.is_empty() {
            return;
        }
        self.inline(&code_span(&text));
    }

    fn code_block(&mut self, el: ElementRef) {
        let lang = diagram_lang(el)
            .map(str::to_string)
            .unwrap_or_else(|| code_lang(el));
        let text: String = el.text().collect();
        let text = trim_code(&text);
        let fence = fence_for(text);
        self.block_break();
        self.raw_line(&format!("{fence}{lang}"));
        for line in text.lines() {
            self.raw_line(line);
        }
        self.raw_line(&fence);
        self.block_break();
    }

    fn link(&mut self, el: ElementRef) {
        let href = el.attr("href").unwrap_or("").trim();
        if href.is_empty() || href.to_ascii_lowercase().starts_with("javascript:") {
            self.walk_children(el);
            return;
        }
        // The capture is already escaped. The address, used when there is
        // no text, is not.
        let captured = words(&self.capture(|w| w.walk_children(el)));
        let text = if captured.is_empty() {
            escape_to_string(href)
        } else {
            captured
        };
        self.inline(&format!("[{text}](<{}>)", clean_url(href)));
    }

    fn image(&mut self, el: ElementRef) {
        let alt = words(
            el.attr("alt")
                .or_else(|| el.attr("aria-label"))
                .unwrap_or(""),
        );
        let Some(src) = image_src(el) else {
            self.plain(&alt);
            return;
        };
        let dest = if src.starts_with("data:") {
            let Some(path) = self.embed(src) else {
                self.plain(&alt);
                return;
            };
            path
        } else {
            let src = src.split(['?', '#']).next().unwrap_or(src);
            clean_url(src)
        };
        if dest.is_empty() {
            self.plain(&alt);
            return;
        }
        self.inline(&format!("![{}](<{dest}>)", escape_to_string(&alt)));
    }

    /// A diagram already drawn, or a mermaid flowchart drawn from its source.
    /// Anything else is left as code: its script never runs here.
    fn diagram(&mut self, el: ElementRef) {
        let Some(picture) = diagram_picture(el) else {
            let owned = el.text().collect::<String>();
            let text = trim_code(&owned);
            if text.is_empty() {
                return;
            }
            if let Some(svg) = crate::mermaid::svg(text)
                && let Some(path) = self.embed_bytes(svg.as_bytes(), "svg")
            {
                self.block_break();
                self.inline(&format!("![](<{path}>)"));
                self.block_break();
                return;
            }
            self.code_block(el);
            return;
        };
        if picture.value().name() == "svg" {
            self.drawing(picture);
            return;
        }
        self.block_break();
        self.image(picture);
        self.block_break();
    }

    /// An SVG drawn in the page, written out so typst can read it.
    fn drawing(&mut self, el: ElementRef) {
        if !drawn(el) {
            return;
        }
        let markup = svg_markup(el);
        let Some(path) = self.embed_bytes(markup.as_bytes(), "svg") else {
            return;
        };
        let alt = words(
            el.attr("aria-label")
                .or_else(|| el.attr("title"))
                .unwrap_or(""),
        );
        self.block_break();
        self.inline(&format!("![{}](<{path}>)", escape_to_string(&alt)));
        self.block_break();
    }

    /// Write an embedded image aside and return the path the markdown uses.
    fn embed(&mut self, src: &str) -> Option<String> {
        let (bytes, ext) = data_image(src)?;
        self.embed_bytes(&bytes, ext)
    }

    fn embed_bytes(&mut self, bytes: &[u8], ext: &str) -> Option<String> {
        if self.images.is_none() {
            self.images = tempfile::tempdir().ok();
        }
        let dir = self.images.as_ref()?;
        let path = dir.path().join(format!("image{}.{ext}", self.nimage));
        self.nimage += 1;
        std::fs::write(&path, bytes).ok()?;
        Some(path.to_string_lossy().into_owned())
    }

    fn list(&mut self, el: ElementRef, ordered: bool) {
        if self.after_marker {
            if !self.out.ends_with('\n') {
                self.out.push('\n');
            }
            self.after_marker = false;
            self.at_line_start = true;
            self.space = false;
        } else {
            self.block_break();
        }
        let mut n = el
            .attr("start")
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|n| *n >= 1)
            .unwrap_or(1);
        for li in el.child_elements().filter(|c| c.value().name() == "li") {
            if only_lists(li) {
                self.walk_children(li);
                continue;
            }
            let marker = if let Some(done) = task(li) {
                if done { "- [x] " } else { "- [ ] " }.to_string()
            } else if ordered {
                let marker = format!("{n}. ");
                n += 1;
                marker
            } else {
                "- ".to_string()
            };
            self.begin_item(&marker);
            let indent = self.indent;
            self.indent += marker.chars().count();
            self.walk_children(li);
            self.indent = indent;
            if !self.out.ends_with('\n') {
                self.out.push('\n');
            }
            self.at_line_start = true;
            self.space = false;
            self.after_marker = false;
        }
    }

    fn begin_item(&mut self, marker: &str) {
        if !self.out.is_empty() && !self.out.ends_with('\n') {
            self.out.push('\n');
        }
        self.out.push_str(&" ".repeat(self.indent));
        self.out.push_str(marker);
        self.at_line_start = false;
        self.space = false;
        self.after_marker = true;
    }

    fn quote(&mut self, el: ElementRef) {
        let inner = self
            .capture(|w| w.walk_children(el))
            .trim_matches('\n')
            .to_string();
        if inner.trim().is_empty() {
            return;
        }
        self.block_break();
        for line in inner.lines() {
            self.ensure_line();
            self.out.push_str("> ");
            self.out.push_str(line);
            self.out.push('\n');
            self.at_line_start = true;
            self.space = false;
        }
        self.block_break();
    }

    fn table(&mut self, el: ElementRef) {
        let mut header = None;
        let mut rows = Vec::new();
        let mut caption = String::new();
        self.collect_table(el, false, &mut header, &mut rows, &mut caption);
        let header = match header {
            Some(header) => header,
            None if rows.is_empty() => return,
            None => rows.remove(0),
        };
        let cols = header
            .len()
            .max(rows.iter().map(Vec::len).max().unwrap_or(0))
            .max(1);
        let header = pad(&header, cols);
        let indent = self.indent;
        self.indent = 0;
        self.block_break();
        self.write_row(&header);
        self.write_sep(&header);
        for row in &rows {
            self.write_row(&pad(row, cols));
        }
        self.block_break();
        self.indent = indent;
        if !caption.is_empty() {
            self.block_break();
            self.ensure_line();
            push_escaped(&mut self.out, &caption);
            self.block_break();
        }
    }

    fn collect_table(
        &mut self,
        el: ElementRef,
        in_head: bool,
        header: &mut Option<Vec<Cell>>,
        rows: &mut Vec<Vec<Cell>>,
        caption: &mut String,
    ) {
        for child in el.child_elements() {
            match child.value().name() {
                "thead" => self.collect_table(child, true, header, rows, caption),
                "tbody" | "tfoot" => self.collect_table(child, false, header, rows, caption),
                "caption" if caption.is_empty() => {
                    *caption = words(&child.text().collect::<String>());
                }
                "tr" => {
                    let (cells, all_th) = self.row(child);
                    if cells.is_empty() {
                        continue;
                    }
                    if header.is_none() && (in_head || all_th) {
                        *header = Some(cells);
                    } else {
                        rows.push(cells);
                    }
                }
                "colgroup" | "col" | "caption" => {}
                _ => self.collect_table(child, in_head, header, rows, caption),
            }
        }
    }

    fn row(&mut self, tr: ElementRef) -> (Vec<Cell>, bool) {
        let mut cells = Vec::new();
        let mut all_th = true;
        for cell in tr.child_elements() {
            let name = cell.value().name();
            if name != "th" && name != "td" {
                continue;
            }
            if name != "th" {
                all_th = false;
            }
            let text = words(&self.capture(|w| w.walk_children(cell)));
            cells.push((text, cell_align(cell)));
        }
        let all_th = all_th && !cells.is_empty();
        (cells, all_th)
    }

    fn write_row(&mut self, cells: &[Cell]) {
        self.ensure_line();
        self.out.push('|');
        for (text, _) in cells {
            self.out.push(' ');
            if text.is_empty() {
                self.out.push(' ');
            } else {
                self.out.push_str(text);
                self.out.push(' ');
            }
            self.out.push('|');
        }
        self.out.push('\n');
        self.at_line_start = true;
        self.space = false;
    }

    fn write_sep(&mut self, cells: &[Cell]) {
        self.ensure_line();
        self.out.push('|');
        for (_, align) in cells {
            let spec = match *align {
                Some("left") => ":---",
                Some("center") => ":---:",
                Some("right") => "---:",
                _ => "---",
            };
            self.out.push(' ');
            self.out.push_str(spec);
            self.out.push_str(" |");
        }
        self.out.push('\n');
        self.at_line_start = true;
    }
}

fn heading_level(name: &str) -> Option<u8> {
    let level: u8 = name.strip_prefix('h')?.parse().ok()?;
    (1..=6).contains(&level).then_some(level)
}

fn skipped(el: ElementRef) -> bool {
    if matches!(
        el.value().name(),
        "script"
            | "style"
            | "noscript"
            | "template"
            | "iframe"
            | "object"
            | "canvas"
            | "form"
            | "button"
            | "input"
            | "select"
            | "textarea"
            | "nav"
            | "footer"
            | "aside"
            | "link"
            | "meta"
            | "head"
            | "title"
            | "colgroup"
            | "col"
    ) {
        return true;
    }
    if el.attr("hidden").is_some()
        || el
            .attr("aria-hidden")
            .is_some_and(|v| v.eq_ignore_ascii_case("true"))
    {
        return true;
    }
    if el
        .value()
        .id()
        .is_some_and(|id| matches!(id, "TOC" | "toc" | "table-of-contents"))
    {
        return true;
    }
    if el
        .attr("role")
        .is_some_and(|role| matches!(role, "navigation" | "contentinfo" | "doc-toc"))
    {
        return true;
    }
    class_in(
        el,
        &[
            "toc",
            "nav",
            "navbar",
            "footer",
            "sidebar",
            "notes",
            "katex",
            "mathjax",
            "mjx-container",
        ],
    )
}

fn shows_image(el: ElementRef) -> bool {
    match el.value().name() {
        "svg" => !skipped(el) && drawn(el),
        "img" | "object" | "embed" => image_src(el).is_some(),
        _ => false,
    }
}

fn image_src(el: ElementRef<'_>) -> Option<&str> {
    let name = el.value().name();
    let src = match name {
        "img" => el
            .attr("src")
            .or_else(|| el.attr("data-src"))
            .or_else(|| srcset_url(el)),
        "object" => el.attr("data").or_else(|| el.attr("src")),
        "embed" => el.attr("src").or_else(|| el.attr("data")),
        _ => None,
    }?;
    let src = src.trim();
    if src.is_empty() {
        return None;
    }
    if name == "img" || looks_like_image(src, el.attr("type").unwrap_or("")) {
        Some(src)
    } else {
        None
    }
}

fn srcset_url(el: ElementRef<'_>) -> Option<&str> {
    let url = el
        .attr("srcset")?
        .split(',')
        .next()?
        .split_whitespace()
        .next()?;
    (!url.is_empty()).then_some(url)
}

fn looks_like_image(src: &str, typ: &str) -> bool {
    if ascii_starts(src, "data:image") || ascii_starts(typ, "image/") {
        return true;
    }
    let name = src
        .split(['?', '#'])
        .next()
        .unwrap_or(src)
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(src);
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg"
    )
}

fn diagram_lang(el: ElementRef) -> Option<&'static str> {
    const NAMES: &[&str] = &["mermaid", "plantuml", "graphviz", "d2", "nomnoml", "pikchr"];
    el.value().classes().find_map(|class| {
        let bare = strip_prefix_ignore(class, "language-")
            .or_else(|| strip_prefix_ignore(class, "lang-"))
            .unwrap_or(class);
        NAMES
            .iter()
            .copied()
            .find(|name| bare.eq_ignore_ascii_case(name))
    })
}

fn strip_prefix_ignore<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    ascii_starts(s, prefix).then(|| &s[prefix.len()..])
}

/// The picture a diagram widget drew, rather than its source.
fn diagram_picture(el: ElementRef<'_>) -> Option<ElementRef<'_>> {
    el.descendent_elements().find(|child| {
        *child != el
            && match child.value().name() {
                "svg" => drawn(*child),
                "img" => image_src(*child).is_some(),
                _ => false,
            }
    })
}

fn drawn(el: ElementRef) -> bool {
    el.descendent_elements().any(|child| child != el)
}

fn svg_markup(el: ElementRef) -> String {
    let markup = el.html();
    // The HTML parser does not keep the SVG namespace, which typst wants.
    if markup.to_ascii_lowercase().contains("xmlns") {
        markup
    } else if let Some(rest) = markup.strip_prefix("<svg") {
        format!("<svg xmlns=\"http://www.w3.org/2000/svg\"{rest}")
    } else {
        markup
    }
}

fn class_in(el: ElementRef, names: &[&str]) -> bool {
    el.value()
        .classes()
        .any(|c| names.iter().any(|name| c.eq_ignore_ascii_case(name)))
}

fn has_body(el: ElementRef) -> bool {
    if shows_image(el) || formula_of(el).is_some() {
        return true;
    }
    children_have_body(el)
}

/// Nested sections are slides of their own, so they are not this element's text.
fn children_have_body(el: ElementRef) -> bool {
    for child in el.children() {
        if let Some(inner) = ElementRef::wrap(child) {
            if inner.value().name() == "section" {
                continue;
            }
            // A rendered formula is skipped as chrome, and it is still the slide.
            if skipped(inner) && formula_of(inner).is_none() && !shows_image(inner) {
                continue;
            }
            if has_body(inner) {
                return true;
            }
        } else if child
            .value()
            .as_text()
            .is_some_and(|t| !t.chars().all(char::is_whitespace))
        {
            return true;
        }
    }
    false
}

fn formula_of(el: ElementRef) -> Option<(String, bool)> {
    let name = el.value().name();
    let math_class = el
        .value()
        .classes()
        .any(|c| c.eq_ignore_ascii_case("math") || ascii_starts(c, "math-"));
    let mathml = name == "math";
    let script = name == "script" && el.attr("type").is_some_and(|t| ascii_starts(t, "math/tex"));
    // The drawing is what the page shows. The TeX is an annotation or a
    // script beside it, and the drawing is not readable as the formula.
    if !math_class && !mathml && !script && !rendered_math(el) {
        return None;
    }
    let display = math_display(el);
    if let Some(tex) = annotation_tex(el) {
        return bare_tex(&tex, display);
    }
    if let Some((tex, disp)) = tex_script(el) {
        return bare_tex(&tex, display || disp);
    }
    if !math_class && !mathml {
        return None;
    }
    let visible = visible_text(el);
    delimited(&visible).map(|(tex, disp)| (tex, display || disp))
}

fn rendered_math(el: ElementRef) -> bool {
    el.value().name() == "mjx-container"
        || class_in(
            el,
            &[
                "katex",
                "katex-display",
                "mathjax",
                "mathjax_display",
                "mjx-container",
            ],
        )
}

fn math_display(el: ElementRef) -> bool {
    let mut cur = Some(el);
    for _ in 0..4 {
        let Some(node) = cur else {
            break;
        };
        if node.value().classes().any(|c| {
            c.eq_ignore_ascii_case("display")
                || c.eq_ignore_ascii_case("katex-display")
                || c.eq_ignore_ascii_case("mathjax_display")
                || ascii_ends(c, "-display")
                || ascii_ends(c, "_display")
        }) || node.attr("display").is_some_and(|d| {
            let d = d.trim();
            d.eq_ignore_ascii_case("block") || d.eq_ignore_ascii_case("true")
        }) {
            return true;
        }
        cur = node.parent().and_then(ElementRef::wrap);
    }
    el.attr("type")
        .is_some_and(|t| t.to_ascii_lowercase().contains("display"))
}

fn tex_script(el: ElementRef) -> Option<(String, bool)> {
    for script in el.descendent_elements() {
        if script.value().name() != "script" {
            continue;
        }
        let typ = script.attr("type").unwrap_or("");
        if !ascii_starts(typ, "math/tex") {
            continue;
        }
        let tex: String = script.text().collect();
        let tex = tex.trim();
        if tex.is_empty() {
            continue;
        }
        let display = typ.to_ascii_lowercase().contains("display");
        return Some((tex.to_string(), display));
    }
    None
}

fn annotation_tex(el: ElementRef) -> Option<String> {
    for ann in el.descendent_elements() {
        if ann.value().name() != "annotation" {
            continue;
        }
        let enc = ann.attr("encoding").unwrap_or("").to_ascii_lowercase();
        if !enc.contains("tex") {
            continue;
        }
        let tex: String = ann.text().collect();
        let tex = tex.trim();
        if !tex.is_empty() {
            return Some(tex.to_string());
        }
    }
    None
}

fn visible_text(el: ElementRef) -> String {
    let mut out = String::new();
    append_visible(el, &mut out);
    out
}

fn append_visible(el: ElementRef, out: &mut String) {
    for child in el.children() {
        if let Some(inner) = ElementRef::wrap(child) {
            if hide_math(inner) {
                continue;
            }
            append_visible(inner, out);
        } else if let Some(text) = child.value().as_text() {
            out.push_str(text);
        }
    }
}

fn hide_math(el: ElementRef) -> bool {
    if el
        .attr("aria-hidden")
        .is_some_and(|v| v.eq_ignore_ascii_case("true"))
        || matches!(el.value().name(), "script" | "style" | "svg")
    {
        return true;
    }
    class_in(el, &["katex", "katex-html", "mathjax", "mjx-container"])
}

fn ascii_starts(s: &str, prefix: &str) -> bool {
    s.len() >= prefix.len() && s.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

fn ascii_ends(s: &str, suffix: &str) -> bool {
    s.len() >= suffix.len()
        && s.as_bytes()[s.len() - suffix.len()..].eq_ignore_ascii_case(suffix.as_bytes())
}

fn delimited(s: &str) -> Option<(String, bool)> {
    for (open, close, display) in [
        ("\\[", "\\]", true),
        ("\\(", "\\)", false),
        ("$$", "$$", true),
    ] {
        if let Some(tex) = between(s, open, close) {
            return Some((tex, display));
        }
    }
    single_dollar(s).map(|tex| (tex, false))
}

fn between(s: &str, open: &str, close: &str) -> Option<String> {
    let rest = &s[s.find(open)? + open.len()..];
    let tex = rest[..rest.find(close)?].trim();
    (!tex.is_empty()).then(|| tex.to_string())
}

fn single_dollar(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' && bytes.get(i + 1) != Some(&b'$') {
            let rest = &s[i + 1..];
            let j = rest.find('$')?;
            if rest.as_bytes().get(j + 1) == Some(&b'$') {
                return None;
            }
            let tex = rest[..j].trim();
            if !tex.is_empty() && !tex.contains('\n') {
                return Some(tex.to_string());
            }
            return None;
        }
        i += 1;
    }
    None
}

/// TeX, and whether it is displayed. One pair of delimiters is taken off
/// when an annotation or a script included them.
fn bare_tex(tex: &str, display: bool) -> Option<(String, bool)> {
    let tex = tex.trim();
    if tex.is_empty() {
        return None;
    }
    for (open, close, disp) in [
        ("\\[", "\\]", true),
        ("$$", "$$", true),
        ("\\(", "\\)", false),
    ] {
        if let Some(inner) = tex.strip_prefix(open).and_then(|s| s.strip_suffix(close)) {
            let inner = inner.trim();
            if !inner.is_empty() {
                return Some((inner.to_string(), display || disp));
            }
        }
    }
    if let Some(inner) = tex.strip_prefix('$').and_then(|s| s.strip_suffix('$')) {
        let inner = inner.trim();
        if !inner.is_empty() && !inner.contains('$') {
            return Some((inner.to_string(), display));
        }
    }
    Some((tex.to_string(), display))
}

fn only_lists(el: ElementRef) -> bool {
    let mut found = false;
    for child in el.children() {
        if let Some(inner) = ElementRef::wrap(child) {
            match inner.value().name() {
                "ul" | "ol" => found = true,
                _ => return false,
            }
        } else if child
            .value()
            .as_text()
            .is_some_and(|t| !t.chars().all(char::is_whitespace))
        {
            return false;
        }
    }
    found
}

fn task(el: ElementRef) -> Option<bool> {
    el.descendent_elements()
        .find(|e| {
            e.value().name() == "input"
                && e.attr("type")
                    .is_some_and(|t| t.eq_ignore_ascii_case("checkbox"))
        })
        .map(|e| e.attr("checked").is_some())
}

fn code_lang(el: ElementRef) -> String {
    lang_of(el)
        .or_else(|| {
            el.child_elements()
                .find(|c| c.value().name() == "code")
                .and_then(lang_of)
        })
        .unwrap_or_default()
}

fn lang_of(el: ElementRef) -> Option<String> {
    let classes: Vec<&str> = el.value().classes().collect();
    for class in &classes {
        if let Some(lang) = class
            .strip_prefix("language-")
            .or_else(|| class.strip_prefix("lang-"))
        {
            let lang = clean_lang(lang);
            if !lang.is_empty() {
                return Some(lang);
            }
        }
    }
    if !classes.contains(&"sourceCode") {
        return None;
    }
    const SKIP: &[&str] = &["sourceCode", "numberLines", "number-lines"];
    classes.iter().copied().find_map(|c| {
        if SKIP.contains(&c) {
            return None;
        }
        let lang = clean_lang(c);
        (!lang.is_empty()).then_some(lang)
    })
}

fn clean_lang(lang: &str) -> String {
    lang.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '#' | '-' | '_'))
        .collect()
}

fn trim_code(s: &str) -> &str {
    let s = s
        .strip_prefix("\r\n")
        .or_else(|| s.strip_prefix('\n'))
        .unwrap_or(s);
    s.strip_suffix("\r\n")
        .or_else(|| s.strip_suffix('\n'))
        .unwrap_or(s)
}

fn fence_for(code: &str) -> String {
    "`".repeat(longest(code, '`').max(2) + 1)
}

fn code_span(text: &str) -> String {
    let text = text.replace(['\n', '\r'], " ");
    let fence = "`".repeat(longest(&text, '`') + 1);
    let pad = if text.starts_with('`') || text.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{fence}{pad}{text}{pad}{fence}")
}

fn longest(s: &str, c: char) -> usize {
    let mut best = 0;
    let mut run = 0;
    for ch in s.chars() {
        if ch == c {
            run += 1;
            best = best.max(run);
        } else {
            run = 0;
        }
    }
    best
}

fn cell_align(el: ElementRef) -> Option<&'static str> {
    el.attr("align")
        .and_then(align_word)
        .or_else(|| el.attr("style").and_then(align_style))
}

fn align_word(s: &str) -> Option<&'static str> {
    match s.trim().to_ascii_lowercase().as_str() {
        "left" => Some("left"),
        "center" | "middle" => Some("center"),
        "right" => Some("right"),
        _ => None,
    }
}

fn align_style(style: &str) -> Option<&'static str> {
    let style = style.to_ascii_lowercase();
    let rest = style.split("text-align").nth(1)?;
    let token = rest
        .trim_start_matches(|c: char| c == ':' || c.is_whitespace())
        .split(|c: char| !c.is_ascii_alphabetic())
        .next()
        .unwrap_or("");
    align_word(token)
}

fn pad(row: &[Cell], cols: usize) -> Vec<Cell> {
    let mut out = row.to_vec();
    out.resize(cols, (String::new(), None));
    out
}

fn words(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn simple_script(s: &str) -> bool {
    !s.is_empty()
        && s.chars().count() <= 16
        && s.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(c, '+' | '-' | '=' | ',' | '.' | '(' | ')' | '*' | '/')
        })
}

fn brace_escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if matches!(c, '\\' | '{' | '}') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn clean_url(src: &str) -> String {
    src.replace(['<', '>', '\n', '\r'], "")
}

fn escape_to_string(s: &str) -> String {
    let mut out = String::new();
    push_escaped(&mut out, s);
    out
}

fn push_escaped(out: &mut String, s: &str) {
    for c in s.chars() {
        if escaped(c) {
            out.push('\\');
        }
        out.push(c);
    }
}

fn escaped(c: char) -> bool {
    matches!(
        c,
        '\\' | '`'
            | '*'
            | '_'
            | '{'
            | '}'
            | '['
            | ']'
            | '('
            | ')'
            | '#'
            | '+'
            | '-'
            | '.'
            | '!'
            | '|'
            | '<'
            | '>'
            | '$'
            | '~'
    )
}

fn data_image(src: &str) -> Option<(Vec<u8>, &'static str)> {
    let rest = src.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    let ext = image_ext(meta);
    let bytes = if meta
        .split(';')
        .any(|p| p.trim().eq_ignore_ascii_case("base64"))
    {
        decode_base64(data)?
    } else {
        percent_encoding::percent_decode_str(data).collect()
    };
    (!bytes.is_empty()).then_some((bytes, ext))
}

fn image_ext(meta: &str) -> &'static str {
    let meta = meta.to_ascii_lowercase();
    if meta.contains("jpeg") || meta.contains("/jpg") {
        "jpg"
    } else if meta.contains("gif") {
        "gif"
    } else if meta.contains("webp") {
        "webp"
    } else if meta.contains("svg") {
        "svg"
    } else {
        "png"
    }
}

fn decode_base64(data: &str) -> Option<Vec<u8>> {
    let mut clean: String = data.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    // The standard engine rejects a missing `=`, which embedded images omit.
    let pad = (4 - clean.len() % 4) % 4;
    clean.extend(std::iter::repeat_n('=', pad));
    base64::engine::general_purpose::STANDARD.decode(clean).ok()
}

#[cfg(test)]
#[path = "tests/html.rs"]
mod tests;
