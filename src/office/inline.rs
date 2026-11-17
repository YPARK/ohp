//! Text as markdown: runs, their styles and links, tables and front matter.

/// How a stretch of text is set.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(super) struct Style {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
}

/// A paragraph's text as it is gathered: runs set alike merged, so their
/// marks are not broken, and markdown kept as it is, as pictures and
/// formulas.
#[derive(Default)]
pub(super) struct Inline {
    pieces: Vec<Piece>,
}

enum Piece {
    Text {
        text: String,
        style: Style,
        link: Option<String>,
    },
    Markdown(String),
}

impl Inline {
    pub fn text(&mut self, text: &str, style: Style, link: Option<&str>) {
        if text.is_empty() {
            return;
        }
        if let Some(Piece::Text {
            text: last,
            style: s,
            link: l,
        }) = self.pieces.last_mut()
            && *s == style
            && l.as_deref() == link
        {
            last.push_str(text);
            return;
        }
        self.pieces.push(Piece::Text {
            text: text.to_string(),
            style,
            link: link.map(str::to_string),
        });
    }

    pub fn markdown(&mut self, md: String) {
        if !md.is_empty() {
            self.pieces.push(Piece::Markdown(md));
        }
    }

    pub fn is_empty(&self) -> bool {
        self.pieces.iter().all(|p| match p {
            Piece::Text { text, .. } => text.trim().is_empty(),
            Piece::Markdown(md) => md.trim().is_empty(),
        })
    }

    /// The text alone, as for a title in front matter.
    pub fn plain(&self) -> String {
        let text: String = self
            .pieces
            .iter()
            .filter_map(|p| match p {
                Piece::Text { text, .. } => Some(text.as_str()),
                Piece::Markdown(_) => None,
            })
            .collect();
        squash(&text)
    }

    /// As markdown, on one line, its line breaks as `<br>`.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for p in &self.pieces {
            match p {
                Piece::Markdown(md) => out.push_str(md),
                Piece::Text { text, style, link } => {
                    for (i, line) in text.split('\n').enumerate() {
                        if i > 0 {
                            out.push_str("<br>");
                        }
                        out.push_str(&styled(line, *style, link.as_deref()));
                    }
                }
            }
        }
        out.trim().to_string()
    }
}

/// `text` set in `style`, and linked to `link`, as markdown: the spaces at
/// its ends outside the marks, as markdown reads marks only next to text.
fn styled(text: &str, style: Style, link: Option<&str>) -> String {
    let core = text.trim_matches([' ', '\t']);
    if core.is_empty() {
        return if text.is_empty() {
            String::new()
        } else {
            " ".into()
        };
    }
    let lead = if text.starts_with([' ', '\t']) {
        " "
    } else {
        ""
    };
    let trail = if text.ends_with([' ', '\t']) { " " } else { "" };
    let mut s = if style.code {
        crate::html::code::code_span(core)
    } else {
        let s = escape(&core.replace('\t', " "));
        match (style.bold, style.italic) {
            (true, true) => format!("***{s}***"),
            (true, false) => format!("**{s}**"),
            (false, true) => format!("*{s}*"),
            (false, false) => s,
        }
    };
    if style.strike {
        s = format!("~~{s}~~");
    }
    if let Some(url) = link {
        s = format!("[{s}](<{}>)", crate::html::clean_url(url));
    }
    format!("{lead}{s}{trail}")
}

/// `text` with what markdown would read as marks, or as a citation,
/// escaped.
pub(super) fn escape(text: &str) -> String {
    crate::html::escape_to_string(text).replace('@', "\\@")
}

/// Whether `font` is a typewriter face, so its text is code.
pub(super) fn monospace(font: &str) -> bool {
    let font = font.to_ascii_lowercase();
    [
        "mono",
        "courier",
        "consolas",
        "menlo",
        "monaco",
        "console",
        "inconsolata",
        "code",
    ]
    .iter()
    .any(|w| font.contains(w))
}

/// How deep list items nest, as markdown reads them: an item one level
/// under the one before it, however many levels its own says, and a list
/// that starts at a deep level at the top.
#[derive(Default)]
pub(super) struct Nesting {
    open: Vec<u8>,
}

impl Nesting {
    /// The depth, from 0, of an item at `level`.
    pub fn depth(&mut self, level: u8) -> usize {
        while self.open.last().is_some_and(|top| *top > level) {
            self.open.pop();
        }
        if self.open.last() != Some(&level) {
            self.open.push(level);
        }
        self.open.len() - 1
    }

    /// Something not a list item came between: the next item starts a list.
    pub fn end(&mut self) {
        self.open.clear();
    }
}

/// `lines` as a fenced code block, its fence longer than any run of
/// backticks in it.
pub(super) fn code_block(lines: &[&str]) -> String {
    let code = lines.join("\n");
    let fence = crate::html::code::fence_for(&code);
    format!("{fence}\n{code}\n{fence}")
}

/// Runs of white space as one space, and none at the ends.
pub(super) fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A table as markdown: its first row the header, every row as wide as the
/// widest.
pub(super) fn table(rows: &[Vec<String>]) -> String {
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    let Some((head, body)) = rows.split_first().filter(|_| width > 0) else {
        return String::new();
    };
    let line = |cells: &[String]| {
        let mut s = String::from("|");
        for i in 0..width {
            let cell = cells.get(i).map_or("", String::as_str);
            s.push(' ');
            s.push_str(&bare_pipes_escaped(cell));
            s.push_str(" |");
        }
        s
    };
    let mut out = vec![line(head), format!("|{}", " --- |".repeat(width))];
    out.extend(body.iter().map(|row| line(row)));
    out.join("\n")
}

/// `cell` with each `|` not yet escaped escaped, as one in a formula or a
/// code span, so it does not end the cell.
fn bare_pipes_escaped(cell: &str) -> String {
    let mut out = String::with_capacity(cell.len());
    let mut backslashes = 0;
    for c in cell.chars() {
        if c == '|' && backslashes % 2 == 0 {
            out.push('\\');
        }
        backslashes = if c == '\\' { backslashes + 1 } else { 0 };
        out.push(c);
    }
    out
}

/// Front matter: a title, subtitle and author, and `more` lines, or nothing
/// if none.
pub(super) fn front(
    title: Option<&str>,
    subtitle: Option<&str>,
    author: Option<&str>,
    more: &[&str],
) -> String {
    let mut lines = Vec::new();
    for (key, value) in [("title", title), ("subtitle", subtitle), ("author", author)] {
        if let Some(v) = value.map(squash).filter(|v| !v.is_empty()) {
            lines.push(format!("{key}: {}", crate::html::yaml_quote(&v)));
        }
    }
    lines.extend(more.iter().map(ToString::to_string));
    if lines.is_empty() {
        return String::new();
    }
    format!("---\n{}\n---\n\n", lines.join("\n"))
}
