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

use scraper::{ElementRef, Html, Selector};
use std::path::Path;

mod code;
mod math;
mod media;
mod table;

use math::{bare_tex, formula_of};
use media::{diagram_lang, image_src, shows_image};

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

#[cfg(test)]
#[path = "tests/html.rs"]
mod tests;
