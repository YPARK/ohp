//! Speaker notes, shown to the presenter beside the slides.
//!
//! They come from markdown's `::: notes` divs, as pandoc writes speaker
//! notes, placed on the page their slide is set on, and from a markdown
//! file named as each of the deck's files is: `talk.md` for `talk.pdf`, in
//! which a heading that is a number, as `## 3` or `# Slide 3`, begins the
//! notes of that page of the file:
//!
//! ```text
//! # 1
//! Welcome; say who you are.
//!
//! # 3
//! The figure is from the 2024 run.
//! ```
//!
//! `talk.md` is as often what the PDF was made from, by pandoc, or the
//! slides themselves, so it is taken for notes only while empty or begun by
//! a page's heading; else the notes are in `talk.notes.md` (or
//! `talk.notes.txt`, `talk.notes`).
//!
//! Beamer's `\note{}`, set beside each slide with `show notes on second
//! screen=right`, is a page of its own half, which [`crate::split`] cuts
//! off.

use crate::render::{self, Stamp};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Notes by page of the deck, from 0.
pub type Notes = BTreeMap<usize, String>;

/// The endings a notes file beside a deck's file has, past `talk.md`, in
/// the order looked for.
const ENDINGS: [&str; 3] = ["notes.md", "notes.txt", "notes"];

/// `talk.md` beside `talk.pdf`, unless that is `path` itself.
fn same_name(path: &Path) -> Option<PathBuf> {
    let md = path.with_extension("md");
    (md != path && !render::document(path)).then_some(md)
}

/// Whether `text` is notes: nothing yet, or begun by a page's heading.
/// Anything else is the slides, or what they were made from.
pub fn is_notes(text: &str) -> bool {
    let first = text.lines().find(|l| l.trim_start().starts_with('#'));
    match first {
        Some(line) => heading(line).is_some(),
        None => text.trim().is_empty(),
    }
}

/// The notes file beside `path`, if there is one.
pub fn beside(path: &Path) -> Option<PathBuf> {
    let stem = path.file_stem()?.to_string_lossy();
    let dir = path.parent().unwrap_or(Path::new(""));
    let own =
        same_name(path).filter(|md| std::fs::read_to_string(md).is_ok_and(|text| is_notes(&text)));
    own.into_iter()
        .chain(ENDINGS.iter().map(|end| dir.join(format!("{stem}.{end}"))))
        .find(|p| p.is_file())
}

/// The notes file to write `path`'s notes into: the one beside it, else a
/// new `talk.md` for `talk.pdf`, or `talk.notes.md` where `talk.md` is
/// taken, by the slides or what they were made from.
pub fn file_for(path: &Path) -> PathBuf {
    beside(path).unwrap_or_else(|| match same_name(path) {
        Some(md) if !md.exists() => md,
        _ => {
            let stem = path
                .file_stem()
                .map_or_else(Default::default, |s| s.to_string_lossy());
            path.with_file_name(format!("{stem}.notes.md"))
        }
    })
}

/// The notes `file` holds for page `page` of its deck's file, from 0.
pub fn written(file: &Path, page: usize) -> String {
    let text = std::fs::read_to_string(file).unwrap_or_default();
    parse(&text)
        .into_iter()
        .filter(|(p, _)| *p == page)
        .map(|(_, note)| note)
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Write `note` as page `page`'s, from 0, into `file`, in place of what
/// it had, the other pages' notes, and anything before them, as they are.
/// An empty note takes the page's out.
pub fn save(file: &Path, page: usize, note: &str) -> std::io::Result<()> {
    let text = std::fs::read_to_string(file).unwrap_or_default();
    let written = rewrite(&text, page, note);
    let part = file.with_extension("md.part");
    std::fs::write(&part, written)?;
    std::fs::rename(part, file)
}

/// `text`, a notes file, with page `page`'s notes `note`.
pub fn rewrite(text: &str, page: usize, note: &str) -> String {
    // What comes before the first page's notes, then each page's: its
    // page, its heading and what follows it, as written.
    let mut before = String::new();
    let mut pages: Vec<(usize, String)> = Vec::new();
    for line in text.split_inclusive('\n') {
        if let Some(p) = heading(line) {
            pages.push((p, String::new()));
        }
        match pages.last_mut() {
            Some((_, section)) => section.push_str(line),
            None => before.push_str(line),
        }
    }
    pages.retain(|(p, _)| *p != page);
    let note = note.trim();
    if !note.is_empty() {
        let at = pages
            .iter()
            .position(|(p, _)| *p > page)
            .unwrap_or(pages.len());
        pages.insert(at, (page, format!("# {}\n{note}\n", page + 1)));
    }
    let mut out = before;
    for (_, section) in pages {
        if !out.is_empty() && !out.ends_with("\n\n") {
            out.push_str(if out.ends_with('\n') { "\n" } else { "\n\n" });
        }
        out.push_str(section.trim_end());
        out.push('\n');
    }
    out
}

/// Notes files beside a deck's files, read again whenever one changes.
#[derive(Default)]
pub struct Beside {
    /// For each of the deck's files: its notes file, if any, when each
    /// file it may be last changed, and the deck's page the file begins on.
    files: Vec<(Option<PathBuf>, Vec<Stamp>, usize)>,
}

/// When each file that may be `path`'s notes last changed: only when one
/// has is any read, to see which is its notes.
fn stamps(path: &Path) -> Vec<Stamp> {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let dir = path.parent().unwrap_or(Path::new(""));
    same_name(path)
        .into_iter()
        .chain(ENDINGS.iter().map(|end| dir.join(format!("{stem}.{end}"))))
        .map(|p| render::stamp(&p))
        .collect()
}

impl Beside {
    /// The notes files of `paths`, whose pages begin at `starts`.
    pub fn new(paths: &[PathBuf], starts: &[usize]) -> Self {
        let files = paths
            .iter()
            .zip(starts)
            .map(|(path, &start)| (beside(path), stamps(path), start))
            .collect();
        Beside { files }
    }

    /// Whether a notes file has been written, made or removed since read.
    pub fn changed(&mut self, paths: &[PathBuf]) -> bool {
        let mut changed = false;
        for ((notes, at, _), path) in self.files.iter_mut().zip(paths) {
            let now = stamps(path);
            if now != *at {
                (*notes, *at) = (beside(path), now);
                changed = true;
            }
        }
        changed
    }

    /// The notes the files hold, by page of the deck.
    pub fn read(&self) -> Notes {
        let mut all = Notes::new();
        for (path, _, start) in &self.files {
            let Some(text) = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()) else {
                continue;
            };
            for (page, note) in parse(&text) {
                add(&mut all, start + page, note);
            }
        }
        all
    }
}

/// `note` added to page `page`'s, after any it has.
pub fn add(notes: &mut Notes, page: usize, note: String) {
    let note = note.trim();
    if note.is_empty() {
        return;
    }
    notes
        .entry(page)
        .and_modify(|had| {
            had.push_str("\n\n");
            had.push_str(note);
        })
        .or_insert_with(|| note.to_string());
}

/// `b`'s notes added to `a`'s, page by page.
pub fn merge(mut a: Notes, b: Notes) -> Notes {
    for (page, note) in b {
        add(&mut a, page, note);
    }
    a
}

/// The notes a notes file holds, by page of its file, from 0.
pub fn parse(text: &str) -> Vec<(usize, String)> {
    let mut out: Vec<(usize, String)> = Vec::new();
    for line in text.lines() {
        if let Some(page) = heading(line) {
            out.push((page, String::new()));
        } else if let Some((_, note)) = out.last_mut() {
            note.push_str(line);
            note.push('\n');
        }
    }
    out.into_iter()
        .map(|(page, note)| (page, note.trim().to_string()))
        .filter(|(_, note)| !note.is_empty())
        .collect()
}

/// The page, from 0, a heading such as `## 3` or `# Slide 3` begins.
fn heading(line: &str) -> Option<usize> {
    let rest = line
        .trim()
        .strip_prefix('#')?
        .trim_start_matches('#')
        .trim();
    let lower = rest.to_ascii_lowercase();
    let number = ["slide", "page"]
        .iter()
        .find_map(|word| lower.strip_prefix(word))
        .unwrap_or(&lower)
        .trim();
    let n: usize = number.parse().ok()?;
    n.checked_sub(1)
}

#[cfg(test)]
#[path = "tests/notes.rs"]
mod tests;
