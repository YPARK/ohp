//! Tables, as markdown's pipe tables, each cell aligned as the page aligns it;
//! a table only laid out with, as mail and newsletters are, as its blocks.

use super::{Writer, push_escaped, words};
use scraper::ElementRef;

/// One cell of a markdown table, and how it is aligned.
type Cell = (String, Option<&'static str>);

impl Writer {
    pub(super) fn table(&mut self, el: ElementRef) {
        if laid_out(el) {
            return self.cells_as_blocks(el);
        }
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

    /// Each cell of a table laid out with, in order, as a block of its own.
    fn cells_as_blocks(&mut self, el: ElementRef) {
        for child in el.child_elements() {
            match child.value().name() {
                "td" | "th" | "caption" => self.block(child),
                "colgroup" | "col" => {}
                _ => self.cells_as_blocks(child),
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

/// Whether a table is only laid out with, not one of data: said to be, or
/// with a table in it, a cell of blocks, or a column alone.
fn laid_out(table: ElementRef) -> bool {
    if matches!(table.attr("role"), Some("presentation" | "none")) {
        return true;
    }
    let columns = own(table, &["tr"])
        .iter()
        .map(|tr| own(*tr, &["td", "th"]).len())
        .max()
        .unwrap_or(0);
    for cell in own(table, &["td", "th"]) {
        let blocks = cell
            .descendants()
            .filter_map(ElementRef::wrap)
            .filter(|d| {
                matches!(
                    d.value().name(),
                    "table"
                        | "img"
                        | "ul"
                        | "ol"
                        | "hr"
                        | "blockquote"
                        | "h1"
                        | "h2"
                        | "h3"
                        | "h4"
                        | "h5"
                        | "h6"
                )
            })
            .count();
        let paragraphs = cell
            .descendants()
            .filter_map(ElementRef::wrap)
            .filter(|d| matches!(d.value().name(), "p" | "div"))
            .count();
        if blocks > 0 || paragraphs > 1 {
            return true;
        }
    }
    columns < 2
}

/// The elements named `names` of `el` itself, not of a table in it.
fn own<'a>(el: ElementRef<'a>, names: &[&str]) -> Vec<ElementRef<'a>> {
    let mut out = Vec::new();
    for child in el.child_elements() {
        let name = child.value().name();
        if names.contains(&name) {
            out.push(child);
        } else if name != "table" {
            out.extend(own(child, names));
        }
    }
    out
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
