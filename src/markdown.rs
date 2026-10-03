//! Markdown and R Markdown as typst markup, as slides or as pages.
//!
//! A file whose front matter names a slide format, as beamer, ioslides or
//! revealjs output does, or Marp's `marp: true`, is set as slides, broken
//! where pandoc would break it: at a rule, and at each heading of the slide
//! level, the highest level a heading of which is directly followed by
//! content. Any other file is set on pages of a paper size, breaking where
//! a page fills and at `\newpage`.
//!
//! Code chunks not run by knitr are shown as their code. Pandoc's speaker notes
//! are left out, and its other fenced divs show their content.
//!
//! A formula typst cannot set is shown as LaTeX rendered it, where it was,
//! and otherwise as its source. LaTeX macros, from `header-includes` or on
//! lines of their own as pandoc takes them, are for LaTeX alone.
//!
//! Citations, where front matter names a bibliography, are pandoc's, and
//! the works cited are listed at the end, or in a `::: {#refs}` div.

use crate::cite::{self, Bibliography, Item, Piece};
use crate::latex::{Formula, Rendered};
use crate::math;
use percent_encoding::percent_decode_str;
use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use serde_json::Value;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Where a `::: {#refs}` div was, for the works cited.
const REFS: &str = "<!-- ohp: refs -->";

/// The typst document a markdown file is set as.
pub struct Converted {
    pub source: String,
    /// Images found, in the order they appear: those typst could not read
    /// are named but not shown.
    pub images: Vec<Picture>,
    /// Where citations and the works cited are in `source`.
    pub citing: Vec<Range<usize>>,
    /// Files made here, by the name `source` gives each: formulas LaTeX
    /// rendered, bibliographies and a citation style.
    pub files: Vec<(String, Arc<[u8]>)>,
    /// Where each formula is in `source`, in the order they appear.
    pub formulas: Vec<Range<usize>>,
    /// Formulas shown as their source, which LaTeX could render.
    pub unset: Vec<Formula>,
    /// What LaTeX should read before a formula: the file's macros.
    pub preamble: String,
    /// Whether it lists works cited, from a bibliography typst reads.
    pub listed: bool,
    /// What is not shown as the file asks: a bibliography not read, a work
    /// cited that is not in it.
    pub warnings: Vec<String>,
}

/// An image file the document shows.
#[derive(Debug, PartialEq)]
pub struct Picture {
    /// The name `source` gives it.
    pub name: String,
    pub path: PathBuf,
    /// Where it is in `source`.
    pub at: Range<usize>,
}

/// How a markdown file is set.
pub struct Setting<'a> {
    /// The directory relative image and bibliography paths are in.
    pub base: &'a Path,
    /// Whether citations are set, where front matter names a bibliography,
    /// or left as they are written.
    pub citations: bool,
    /// A typst paper name, for slides and pages alike.
    pub paper: Option<&'a str>,
    /// Formulas, by index, typst could not set.
    pub raw_math: &'a HashSet<usize>,
    /// Formulas, by index, typst could not show even as LaTeX rendered them.
    pub source_only: &'a HashSet<usize>,
    /// Formulas typst cannot set, as LaTeX rendered them.
    pub latex: &'a HashMap<Formula, Rendered>,
    /// Images found, by index, typst could not show.
    pub broken: &'a HashSet<usize>,
}

/// Slides set on paper as wide as a beamer 16:9 frame.
const SLIDE_PAPER: &str = "presentation-16-9";
pub const PAGE_PAPER: &str = "us-letter";

/// YAML's markers of a block scalar, whose lines follow.
const BLOCK_SCALAR: [&str; 5] = ["", "|", ">", "|-", ">-"];

const STYLE: &str = r##"#show raw.where(block: true): block.with(fill: luma(244), inset: 0.6em, radius: 3pt, width: 100%)
#show link: set text(fill: rgb("#1a5fb4"))
#set table(stroke: 0.5pt + luma(170), inset: 0.45em)
#show quote.where(block: true): it => block(inset: (left: 1em, y: 0.2em), stroke: (left: 2pt + luma(200)), it.body)
#let ohp-title(body) = block(below: 0.9em, text(size: 1.35em, weight: "bold", fill: rgb("#23395b"), body))
#let ohp-section(body) = align(center + horizon, text(size: 1.8em, weight: "bold", fill: rgb("#23395b"), body))
"##;

pub fn convert(text: &str, setting: &Setting) -> Converted {
    let (front, body) = front_matter(text);
    let (body, macros) = strip_divs(body);
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_MATH
        | Options::ENABLE_HEADING_ATTRIBUTES;
    let events: Vec<Event> = Parser::new_ext(&body, options).collect();
    let (events, notes) = take_footnotes(events);

    let layout = match front.slides {
        None => Layout::Pages,
        Some(Split::Headings) => Layout::Slides(front.slide_level.or_else(|| slide_level(&events))),
        Some(Split::Rules) => Layout::Slides(None),
    };
    let bib = if setting.citations {
        Bibliography::load(&front.meta, setting.base)
    } else {
        None
    };
    let mut w = Writer {
        out: String::new(),
        line_start: true,
        layout,
        setting,
        images: Vec::new(),
        citing: Vec::new(),
        files: Vec::new(),
        formulas: Vec::new(),
        unset: Vec::new(),
        notes,
        code: None,
        bib,
        cited: false,
        listed: false,
        missing: Vec::new(),
    };
    w.prelude(&front);
    w.events(&events);
    if w.cited
        || w.bib
            .as_ref()
            .is_some_and(|b| b.all || !b.nocite.is_empty())
    {
        w.bibliography();
    }
    let mut preamble = front.header;
    preamble.extend(macros);
    let mut warnings = Vec::new();
    if let Some(bib) = w.bib {
        warnings = bib.warnings;
        w.files.extend(bib.files);
    }
    warnings.extend(
        w.missing
            .iter()
            .map(|k| format!("no {k} in the bibliography")),
    );
    Converted {
        source: w.out,
        images: w.images,
        citing: w.citing,
        files: w.files,
        formulas: w.formulas,
        unset: w.unset,
        preamble: preamble.join("\n"),
        listed: w.listed,
        warnings,
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Layout {
    Pages,
    /// Slides, broken at headings of this level, if any, and at rules.
    Slides(Option<u8>),
}

/// How slides are told apart.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Split {
    /// At rules and headings of the slide level, as pandoc does.
    Headings,
    /// At rules only, as Marp does.
    Rules,
}

#[derive(Debug, Default)]
struct Front {
    title: Option<String>,
    subtitle: Option<String>,
    author: Option<String>,
    date: Option<String>,
    slides: Option<Split>,
    slide_level: Option<u8>,
    /// `header-includes`, one line of LaTeX each.
    header: Vec<String>,
    /// All of it, as YAML reads it, for what is read no other way.
    meta: Value,
}

/// The front matter, and what follows it.
fn front_matter(text: &str) -> (Front, &str) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut front = Front::default();
    let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        return (front, text);
    };
    let mut end = None;
    let mut at = 0;
    for line in rest.split_inclusive('\n') {
        if matches!(line.trim_end(), "---" | "...") {
            end = Some((at, at + line.len()));
            break;
        }
        at += line.len();
    }
    let Some((yaml_end, body_start)) = end else {
        return (front, text);
    };
    let yaml = &rest[..yaml_end];
    front.meta = cite::yaml(yaml).unwrap_or_default();

    // Top-level keys, each with its value and the indented lines under it.
    let mut entries: Vec<(&str, String, Vec<&str>)> = Vec::new();
    for line in yaml.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if !line.starts_with([' ', '\t', '-'])
            && let Some((key, value)) = line.split_once(':')
        {
            entries.push((key.trim(), value.trim().to_string(), Vec::new()));
        } else if let Some(last) = entries.last_mut() {
            last.2.push(line);
        }
    }
    const SLIDE_FORMATS: [&str; 7] = [
        "beamer",
        "ioslides",
        "slidy",
        "revealjs",
        "xaringan",
        "powerpoint",
        "pptx",
    ];
    for (key, value, nested) in &entries {
        let all = std::iter::once(value.as_str()).chain(nested.iter().copied());
        match *key {
            "title" => front.title = scalar(value, nested),
            "subtitle" => front.subtitle = scalar(value, nested),
            "date" => front.date = scalar(value, nested),
            "author" => front.author = authors(value, nested),
            "header-includes" => {
                front.header = all
                    .map(|l| {
                        let l = l.trim();
                        unquote(l.strip_prefix("- ").unwrap_or(l))
                    })
                    .filter(|l| !BLOCK_SCALAR.contains(&l.as_str()))
                    .collect();
            }
            "marp" if value == "true" => front.slides = Some(Split::Rules),
            "output" | "format" => {
                let text: Vec<&str> = all.collect();
                let text = text.join("\n");
                if SLIDE_FORMATS.iter().any(|f| text.contains(f)) {
                    front.slides = front.slides.or(Some(Split::Headings));
                }
                front.slide_level = text.lines().find_map(|l| {
                    let (_, n) = l.trim().split_once("slide_level:")?;
                    n.trim().parse().ok()
                });
            }
            _ => {}
        }
    }
    (front, &rest[body_start..])
}

/// A YAML scalar: quoted, plain, or a block under `|` or `>`.
fn scalar(value: &str, nested: &[&str]) -> Option<String> {
    let value = if BLOCK_SCALAR.contains(&value) {
        let lines: Vec<&str> = nested.iter().map(|l| l.trim()).collect();
        lines.join(" ")
    } else {
        value.to_string()
    };
    let value = unquote(&value);
    (!value.is_empty()).then_some(value)
}

/// Authors as one line, from a scalar or a list of names.
fn authors(value: &str, nested: &[&str]) -> Option<String> {
    if !value.is_empty() {
        return scalar(value, nested);
    }
    let names: Vec<String> = nested
        .iter()
        .filter_map(|l| {
            let item = l.trim_start().strip_prefix('-')?.trim();
            let name = item.strip_prefix("name:").unwrap_or(item);
            Some(unquote(name.trim()))
        })
        .filter(|n| !n.is_empty())
        .collect();
    (!names.is_empty()).then(|| names.join(", "))
}

/// The character and length of the code fence `line` opens, if it opens
/// one: a backtick fence's info has no backtick, so ```` ```x``` ```` is
/// inline code.
fn open_fence(line: &str) -> Option<(char, usize)> {
    let c = line.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let len = line.chars().take_while(|&x| x == c).count();
    (len >= 3 && !(c == '`' && line[len..].contains('`'))).then_some((c, len))
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    for q in ['"', '\''] {
        if let Some(inner) = s.strip_prefix(q).and_then(|s| s.strip_suffix(q)) {
            return inner.to_string();
        }
    }
    s.to_string()
}

/// Drop pandoc's fenced div markers, speaker notes with what they hold, and
/// `. . .` pauses, and take out LaTeX macro definitions, leaving code blocks
/// as they are. Returns the body and the macros.
fn strip_divs(body: &str) -> (String, Vec<String>) {
    const MACROS: [&str; 6] = [
        "\\newcommand",
        "\\renewcommand",
        "\\providecommand",
        "\\DeclareMathOperator",
        "\\def\\",
        "\\let\\",
    ];
    let mut out = String::with_capacity(body.len());
    let mut macros = Vec::new();
    // A definition whose braces are not yet closed.
    let mut open: Option<(String, i32)> = None;
    // The open code fence: its character and length.
    let mut fence: Option<(char, usize)> = None;
    // Depth of the notes div being skipped, counting divs within it.
    let mut notes = 0usize;
    for line in body.split_inclusive('\n') {
        let t = line.trim();
        let depth = |s: &str| s.matches('{').count() as i32 - s.matches('}').count() as i32;
        if let Some((mut def, d)) = open.take() {
            def.push_str(line);
            let d = d + depth(line);
            if d > 0 {
                open = Some((def, d));
            } else {
                macros.push(def.trim().to_string());
            }
            continue;
        }
        if fence.is_none() && MACROS.iter().any(|m| t.starts_with(m)) {
            match depth(t) {
                d if d > 0 => open = Some((line.to_string(), d)),
                _ => macros.push(t.to_string()),
            }
            continue;
        }
        if let Some((c, len)) = fence {
            // Closed by a run of its character as long, and nothing else.
            if t.len() >= len && t.chars().all(|x| x == c) {
                fence = None;
            }
            if notes == 0 {
                out.push_str(line);
            }
            continue;
        }
        if let Some(f) = open_fence(t) {
            fence = Some(f);
        } else if t.starts_with(":::") {
            let attrs = t.trim_start_matches(':').trim();
            if notes > 0 {
                notes = if attrs.is_empty() {
                    notes - 1
                } else {
                    notes + 1
                };
            } else if attrs == "notes" || attrs.contains(".notes") {
                notes = 1;
            } else if attrs.contains("#refs") {
                out.push_str(&format!("\n{REFS}\n\n"));
            }
            continue;
        } else if t == ". . ." {
            continue;
        }
        if notes == 0 {
            out.push_str(line);
        }
    }
    macros.extend(open.map(|(def, _)| def.trim().to_string()));
    (out, macros)
}

type Events<'a> = Vec<Event<'a>>;

/// The document without its footnote definitions, and those by label.
fn take_footnotes(events: Events) -> (Events, HashMap<String, Events>) {
    let mut body = Vec::new();
    let mut notes = HashMap::new();
    let mut current: Option<(String, Events)> = None;
    for event in events {
        match event {
            Event::Start(Tag::FootnoteDefinition(label)) => {
                current = Some((label.to_string(), Vec::new()));
            }
            Event::End(TagEnd::FootnoteDefinition) => notes.extend(current.take()),
            e => match &mut current {
                Some((_, events)) => events.push(e),
                None => body.push(e),
            },
        }
    }
    (body, notes)
}

/// Pandoc's slide level: the highest heading level directly followed by
/// something other than a heading.
fn slide_level(events: &[Event]) -> Option<u8> {
    let mut best: Option<u8> = None;
    for (i, e) in events.iter().enumerate() {
        let Event::End(TagEnd::Heading(level)) = e else {
            continue;
        };
        let next = events.get(i + 1);
        let content = match next {
            None | Some(Event::Start(Tag::Heading { .. }) | Event::Rule) => false,
            Some(_) => true,
        };
        if content {
            let level = *level as u8;
            best = Some(best.map_or(level, |b| b.min(level)));
        }
    }
    best
}

struct Writer<'a, 'e> {
    out: String,
    /// Nothing but whitespace written since the last line or block began:
    /// text here could start markup.
    line_start: bool,
    layout: Layout,
    setting: &'a Setting<'a>,
    images: Vec<Picture>,
    citing: Vec<Range<usize>>,
    files: Vec<(String, Arc<[u8]>)>,
    formulas: Vec<Range<usize>>,
    unset: Vec<Formula>,
    /// Footnotes, written where they are referenced.
    notes: HashMap<String, Events<'e>>,
    /// The code block being read: its language and its text so far.
    code: Option<(Option<String>, String)>,
    /// What citations are to: none where they are left as written.
    bib: Option<Bibliography>,
    /// Whether a work in the bibliography is cited, so it is to be listed.
    cited: bool,
    /// Whether the works cited are listed.
    listed: bool,
    /// Keys cited that are not in the bibliography.
    missing: Vec<String>,
}

impl Writer<'_, '_> {
    fn slides(&self) -> bool {
        self.layout != Layout::Pages
    }

    fn prelude(&mut self, front: &Front) {
        let slides = self.slides();
        let paper = self
            .setting
            .paper
            .unwrap_or(if slides { SLIDE_PAPER } else { PAGE_PAPER });
        let page = if slides {
            format!(
                "#set page(paper: {}, margin: (x: 1.6cm, y: 1.3cm))\n",
                string(paper)
            )
        } else {
            format!("#set page(paper: {}, numbering: \"1\")\n", string(paper))
        };
        self.out.push_str(&page);
        self.out.push_str(if slides {
            "#set text(size: 24pt)\n#set par(leading: 0.6em)\n"
        } else {
            "#set text(size: 11pt)\n#set par(justify: true)\n"
        });
        // The language citations are set in, and hyphenation.
        if let Some((lang, region)) = front
            .meta
            .get("lang")
            .and_then(Value::as_str)
            .and_then(lang)
        {
            let region = region.map_or(String::new(), |r| format!(", region: {}", string(&r)));
            self.out
                .push_str(&format!("#set text(lang: {}{region})\n", string(&lang)));
        }
        self.out.push_str(STYLE);

        let title = front.title.is_some() || front.author.is_some();
        if !title {
            return;
        }
        self.raw(if slides {
            "#align(center + horizon)[\n"
        } else {
            "#align(center)[\n"
        });
        if let Some(t) = &front.title {
            self.raw("#text(size: 1.7em, weight: \"bold\")[");
            self.text(t);
            self.raw("] \\\n");
        }
        if let Some(t) = &front.subtitle {
            self.raw("#text(size: 1.25em)[");
            self.text(t);
            self.raw("] \\\n");
        }
        self.raw("#v(0.8em)\n");
        for t in [&front.author, &front.date].into_iter().flatten() {
            self.text(t);
            self.raw(" \\\n");
        }
        self.raw("]\n");
        self.raw(if slides {
            "#pagebreak(weak: true)\n"
        } else {
            "#v(1.5em)\n"
        });
    }

    /// Write typst markup as it is.
    fn raw(&mut self, s: &str) {
        self.out.push_str(s);
        self.line_start = s.ends_with(['\n', '[']);
    }

    /// Write `s` as text, escaping what typst would read as markup.
    fn text(&mut self, s: &str) {
        if let Some((_, code)) = &mut self.code {
            code.push_str(s);
            return;
        }
        let mut chars = s.chars().peekable();
        if self.line_start {
            while chars.next_if(|c| c.is_whitespace()).is_some() {}
            match chars.peek() {
                Some('=' | '-' | '+') => self.out.push('\\'),
                Some(c) if c.is_ascii_digit() => {
                    while let Some(d) = chars.next_if(char::is_ascii_digit) {
                        self.out.push(d);
                    }
                    if chars.peek() == Some(&'.') {
                        self.out.push('\\');
                    }
                }
                _ => {}
            }
        }
        for c in chars {
            if matches!(
                c,
                '\\' | '#' | '*' | '_' | '$' | '<' | '>' | '@' | '[' | ']' | '`' | '~' | '/'
            ) {
                self.out.push('\\');
            }
            self.out.push(c);
        }
        self.line_start = self.line_start && s.trim().is_empty();
    }

    /// Start a block on a line of its own.
    fn block(&mut self) {
        if !self.out.is_empty() && !self.out.ends_with(['\n', '[']) {
            self.out.push('\n');
        }
        self.line_start = true;
    }

    fn events(&mut self, events: &[Event]) {
        let mut i = 0;
        while i < events.len() {
            i += self.event(events, i);
        }
    }

    /// Write `events[i]`, returning how many events it took.
    fn event(&mut self, events: &[Event], i: usize) -> usize {
        match &events[i] {
            Event::Start(Tag::Paragraph) => {
                if let [Event::Text(t), Event::End(TagEnd::Paragraph), ..] = &events[i + 1..]
                    && matches!(t.trim(), "\\newpage" | "\\pagebreak" | "\\clearpage")
                {
                    self.block();
                    self.raw("#pagebreak(weak: true)\n");
                    return 3;
                }
                self.block();
            }
            Event::End(TagEnd::Paragraph) => self.raw("\n\n"),
            Event::Start(Tag::Heading { level, .. }) => self.heading(*level),
            Event::End(TagEnd::Heading(_)) => self.raw("]\n"),
            Event::Start(Tag::BlockQuote(_)) => {
                self.block();
                self.raw("#quote(block: true)[\n");
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                self.block();
                self.raw("]\n");
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => language(info),
                    CodeBlockKind::Indented => None,
                };
                self.code = Some((lang, String::new()));
            }
            Event::End(TagEnd::CodeBlock) => {
                let Some((lang, code)) = self.code.take() else {
                    return 1;
                };
                self.block();
                let lang = lang.map_or(String::new(), |l| format!("lang: {}, ", string(&l)));
                let code = code.strip_suffix('\n').unwrap_or(&code);
                self.raw(&format!("#raw(block: true, {lang}{})\n", string(code)));
            }
            Event::Start(Tag::List(start)) => {
                self.block();
                match start {
                    Some(n) => self.raw(&format!("#enum(start: {n},\n")),
                    None => self.raw("#list(\n"),
                }
            }
            Event::End(TagEnd::List(_)) => {
                self.block();
                self.raw(")\n");
            }
            Event::Start(Tag::Item) => {
                self.block();
                self.raw("[");
            }
            Event::End(TagEnd::Item) => {
                let out = self.out.trim_end().len();
                self.out.truncate(out);
                self.raw("],\n");
            }
            Event::TaskListMarker(done) => self.raw(if *done { "☒ " } else { "☐ " }),
            Event::Start(Tag::Table(aligns)) => self.table(aligns),
            Event::End(TagEnd::Table) => {
                self.block();
                self.raw(")\n");
            }
            Event::Start(Tag::TableHead) => self.raw("table.header(\n"),
            Event::End(TagEnd::TableHead) => self.raw("),\n"),
            Event::Start(Tag::TableRow) | Event::End(TagEnd::TableRow) => {}
            Event::Start(Tag::TableCell) => self.raw("["),
            Event::End(TagEnd::TableCell) => self.raw("],\n"),
            Event::Start(Tag::Emphasis) => self.raw("#emph["),
            Event::Start(Tag::Strong) => self.raw("#strong["),
            Event::Start(Tag::Strikethrough) => self.raw("#strike["),
            Event::End(TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough) => {
                self.raw("];");
            }
            // typst has no link to nowhere: a placeholder shows its text.
            Event::Start(Tag::Link { dest_url, .. }) if dest_url.is_empty() => self.raw("#["),
            Event::Start(Tag::Link { dest_url, .. }) => {
                self.raw(&format!("#link({})[", string(dest_url)));
            }
            Event::End(TagEnd::Link) => self.raw("];"),
            Event::Start(Tag::Image { dest_url, .. }) => return self.image(events, i, dest_url),
            Event::Text(_) if self.bib.is_some() && self.code.is_none() => {
                // A citation's brackets come as text events of their own.
                let mut text = String::new();
                let mut taken = 0;
                for e in &events[i..] {
                    match e {
                        Event::Text(t) => text.push_str(t),
                        Event::SoftBreak => text.push(' '),
                        _ => break,
                    }
                    taken += 1;
                }
                for piece in cite::parse(&text) {
                    match piece {
                        Piece::Text(t) => self.text(t),
                        Piece::Cite { items, bracketed } => self.cite(&items, bracketed),
                    }
                }
                return taken;
            }
            Event::Text(t) => self.text(t),
            Event::Code(t) => self.raw(&format!("#raw({});", string(t))),
            Event::InlineMath(t) => self.formula(t, false),
            Event::DisplayMath(t) => self.formula(t, true),
            Event::SoftBreak => self.text(" "),
            // As a call, not ` \`: an item or note ends trimmed, and a
            // backslash left before its `]` would escape it.
            Event::HardBreak => self.raw("#linebreak();"),
            Event::Rule => {
                self.block();
                self.raw(if self.slides() {
                    "#pagebreak(weak: true)\n"
                } else {
                    "#line(length: 100%, stroke: 0.5pt + luma(170))\n"
                });
            }
            // Taken out while it is written, so a note that refers to
            // itself shows the reference as written.
            Event::FootnoteReference(label) => match self.notes.remove(label.as_ref()) {
                Some(note) => {
                    self.raw("#footnote[");
                    self.events(&note);
                    let end = self.out.trim_end().len();
                    self.out.truncate(end);
                    self.raw("];");
                    self.notes.insert(label.to_string(), note);
                }
                None => self.text(&format!("[^{label}]")),
            },
            Event::Html(html) if html.trim() == REFS => self.bibliography(),
            Event::InlineHtml(html) | Event::Html(html) => {
                let tag = html.trim().to_ascii_lowercase();
                if matches!(tag.as_str(), "<br>" | "<br/>" | "<br />") {
                    self.raw("#linebreak();");
                }
            }
            _ => {}
        }
        1
    }

    fn heading(&mut self, level: HeadingLevel) {
        let level = level as u8;
        self.block();
        match self.layout {
            Layout::Slides(Some(slide)) if level < slide => {
                self.raw("#pagebreak(weak: true)\n#ohp-section[");
            }
            Layout::Slides(Some(slide)) if level == slide => {
                self.raw("#pagebreak(weak: true)\n#ohp-title[");
            }
            Layout::Slides(Some(slide)) => {
                self.raw(&format!("#heading(level: {})[", level - slide));
            }
            _ => self.raw(&format!("#heading(level: {level})[")),
        }
    }

    /// A table, its columns aligned as the markdown aligns them.
    fn table(&mut self, aligns: &[Alignment]) {
        let align: Vec<&str> = aligns
            .iter()
            .map(|a| match a {
                Alignment::None => "auto",
                Alignment::Left => "left",
                Alignment::Center => "center",
                Alignment::Right => "right",
            })
            .collect();
        self.block();
        self.raw(&format!(
            "#table(columns: {}, align: ({},),\n",
            aligns.len().max(1),
            align.join(", ")
        ));
    }

    /// An image, alone in its paragraph as a block, or else in the line.
    /// Returns the events it took: through the end of its description.
    fn image(&mut self, events: &[Event], i: usize, url: &str) -> usize {
        let mut depth = 0;
        let mut end = i;
        for (j, e) in events.iter().enumerate().skip(i) {
            match e {
                Event::Start(Tag::Image { .. }) => depth += 1,
                Event::End(TagEnd::Image) => {
                    depth -= 1;
                    if depth == 0 {
                        end = j;
                        break;
                    }
                }
                _ => {}
            }
        }
        let alt: String = events[i + 1..end]
            .iter()
            .filter_map(|e| match e {
                Event::Text(t) | Event::Code(t) => Some(t.as_ref()),
                _ => None,
            })
            .collect();
        let alone = matches!(
            i.checked_sub(1).map(|p| &events[p]),
            Some(Event::Start(Tag::Paragraph))
        ) && matches!(events.get(end + 1), Some(Event::End(TagEnd::Paragraph)));
        let found = self.find(url).map(|path| {
            let ext = path
                .extension()
                .map_or(String::new(), |e| e.to_string_lossy().to_lowercase());
            let name = format!("/img/{}.{ext}", self.images.len());
            (name, path)
        });
        // One typst could not read is shown as if it were not found.
        let index = self.images.len();
        let broken = self.setting.broken.contains(&index);
        let start = self.out.len();
        match &found {
            Some((name, _)) if !broken => {
                let name = string(name);
                if alone && self.slides() {
                    // What is left of the slide, so a plot never spills
                    // onto a slide of its own.
                    self.raw(&format!(
                        "#block(height: 1fr, width: 100%, image({name}, width: 100%, height: 100%, fit: \"contain\"))\n"
                    ));
                } else if alone {
                    self.raw(&format!("#align(center, image({name}))\n"));
                } else {
                    self.raw(&format!("#box(image({name}, height: 1.2em));"));
                }
            }
            _ => {
                let shown = if alt.is_empty() { url } else { &alt };
                self.raw("#text(fill: luma(120))[");
                self.text(&format!("[image: {shown}]"));
                self.raw("];");
            }
        }
        if let Some((name, path)) = found {
            let at = start..self.out.len();
            self.images.push(Picture { name, path, at });
        }
        end - i + 1
    }

    /// The file an image's URL names, if it is here: never fetched.
    fn find(&self, url: &str) -> Option<PathBuf> {
        let path = match url.strip_prefix("file:") {
            // `file://host/path`: only this machine's.
            Some(rest) => match rest.strip_prefix("//") {
                Some(rest) => {
                    let at = rest.find('/')?;
                    if !matches!(&rest[..at], "" | "localhost") {
                        return None;
                    }
                    &rest[at..]
                }
                None => rest,
            },
            None if url.contains("://") => return None,
            None => url,
        };
        // As written, or with its `%` escapes read, as pandoc reads a path.
        let unescaped = percent_decode_str(path)
            .decode_utf8()
            .ok()
            .filter(|u| u != path);
        std::iter::once(Cow::Borrowed(path))
            .chain(unescaped)
            .map(|p| self.setting.base.join(p.as_ref()))
            .find(|p| p.is_file())
    }

    fn formula(&mut self, latex: &str, display: bool) {
        let index = self.formulas.len();
        let start = self.out.len();
        let typst = (!self.setting.raw_math.contains(&index))
            .then(|| math::convert(latex))
            .flatten();
        let formula: Formula = (latex.trim().to_string(), display);
        let source_only = self.setting.source_only.contains(&index);
        match (typst, display) {
            (Some(m), false) => self.raw(&format!("${m}$")),
            (Some(m), true) => self.raw(&format!("$ {m} $")),
            (None, _) => match self.setting.latex.get(&formula) {
                Some(r) if !source_only => self.rendered(r, display),
                _ => {
                    let source = string(&formula.0);
                    self.raw(&if display {
                        format!("#align(center, raw(block: true, {source}));")
                    } else {
                        format!("#raw({source});")
                    });
                    if !source_only {
                        self.unset.push(formula);
                    }
                }
            },
        }
        self.formulas.push(start..self.out.len());
    }

    /// A formula as LaTeX rendered it, sized to the text around it.
    fn rendered(&mut self, r: &Rendered, display: bool) {
        let name = format!("/latex/{}.pdf", self.files.len());
        self.files.push((name.clone(), r.pdf.clone()));
        let name = string(&name);
        let image = format!("image({name}, height: {:.4}em)", r.height + r.depth);
        self.raw(&if display {
            format!("#align(center, {image});")
        } else {
            format!("#box(baseline: {:.4}em, {image});", r.depth)
        });
    }

    /// A citation: in parentheses or as the style has it where bracketed,
    /// and as `Doe (1999)` in the text. A prefix, which typst's citations
    /// have no place for, is written before.
    fn cite(&mut self, items: &[Item], bracketed: bool) {
        for (n, item) in items.iter().enumerate() {
            if !item.prefix.is_empty() {
                if n > 0 {
                    self.text(" ");
                }
                self.text(&item.prefix);
                self.text(" ");
            }
            let known = self
                .bib
                .as_ref()
                .is_some_and(|b| b.keys.contains(&item.key));
            if !known {
                // As pandoc shows it.
                if !self.missing.contains(&item.key) {
                    self.missing.push(item.key.clone());
                }
                self.raw(if bracketed { "(#strong[" } else { "#strong[" });
                self.text(&format!("{}?", item.key));
                self.raw(if bracketed { "];)" } else { "];" });
                continue;
            }
            self.cited = true;
            let year = bracketed && item.suppress_author;
            let form = match (bracketed, year) {
                (false, _) => ", form: \"prose\"",
                (true, true) => ", form: \"year\"",
                (true, false) => "",
            };
            if year {
                self.raw("(");
            }
            let start = self.out.len();
            self.raw(&format!("#cite(label({}){form}", string(&item.key)));
            if !item.suffix.is_empty() {
                self.raw(", supplement: [");
                self.text(&item.suffix);
                self.raw("]");
            }
            self.raw(");");
            self.citing.push(start..self.out.len());
            if year {
                self.raw(")");
            }
        }
    }

    /// The works cited, once: on a slide of their own, or after a heading
    /// of `reference-section-title`.
    fn bibliography(&mut self) {
        let Some(bib) = &self.bib else {
            return;
        };
        if self.listed || bib.sources.is_empty() {
            return;
        }
        self.listed = true;
        let sources: Vec<String> = bib.sources.iter().map(|s| string(s)).collect();
        let list = format!(
            "#bibliography(({},), style: {}, title: none, full: {})\n",
            sources.join(", "),
            string(&bib.style),
            bib.all
        );
        let nocite: Vec<String> = bib
            .nocite
            .iter()
            .filter(|k| bib.keys.contains(*k))
            .map(|k| format!("#cite(label({}), form: none)\n", string(k)))
            .collect();
        let (title, suppress) = (bib.title.clone(), bib.suppress);

        self.block();
        // The works cited and what wraps them, not the heading: an error
        // there is not the bibliography's.
        let mut start = self.out.len();
        if suppress {
            // Typst sets no citation without a bibliography.
            self.raw("#[#show bibliography: none\n");
        } else {
            if self.slides() && (title.is_some() || !self.after_title()) {
                self.raw("#pagebreak(weak: true)\n");
            }
            if let Some(title) = title {
                let level = match self.layout {
                    Layout::Slides(Some(level)) => level as usize,
                    _ => 1,
                };
                self.heading(HeadingLevel::try_from(level).unwrap_or(HeadingLevel::H1));
                self.text(&title);
                self.raw("]\n");
            }
            start = self.out.len();
            self.raw(if self.slides() {
                "#[#set text(size: 0.8em)\n"
            } else {
                "#[\n"
            });
        }
        for cite in nocite {
            self.raw(&cite);
        }
        self.raw(&list);
        self.raw("]\n");
        self.citing.push(start..self.out.len());
    }

    /// Whether a slide's title, or a section's, is the last written: a
    /// slide for the works cited.
    fn after_title(&self) -> bool {
        let last = self.out.trim_end().lines().next_back().unwrap_or("");
        last.contains("#ohp-title[") || last.contains("#ohp-section[")
    }
}

/// A BCP 47 tag, `de-DE`, as typst's language and region, if it is one.
fn lang(tag: &str) -> Option<(String, Option<String>)> {
    let mut parts = tag.trim().split(['-', '_']);
    let lang = parts.next()?.to_ascii_lowercase();
    if !(2..=3).contains(&lang.len()) || !lang.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let region = parts
        .find(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_alphabetic()))
        .map(|r| r.to_ascii_uppercase());
    Some((lang, region))
}

/// The language of a fenced code block: `r` for an R Markdown chunk's
/// `{r name, echo=FALSE}`.
fn language(info: &str) -> Option<String> {
    let info = info.trim().trim_start_matches('{').trim_start_matches('.');
    let lang: String = info
        .chars()
        .take_while(|c| !c.is_whitespace() && !matches!(c, ',' | '}' | '{'))
        .collect();
    (!lang.is_empty()).then(|| lang.to_lowercase())
}

/// `s` as a typst string literal.
pub fn string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
#[path = "tests/markdown.rs"]
mod tests;
