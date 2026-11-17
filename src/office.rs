//! Word and PowerPoint files, `.docx` and `.pptx`, as markdown, typeset as
//! markdown is.
//!
//! A `.docx` is its body: headings by their style, lists by their
//! numbering, tables, links, bold, italic, struck and code text, footnotes,
//! pictures, and equations, as LaTeX between dollars. A `.pptx` is slides:
//! each slide's title, text, bullets, tables and pictures, in turn, with
//! its speaker notes. A picture in a format typst cannot read, as a Windows
//! metafile, is left out, and the status line says so.
//!
//! The files are zip packages of XML, read with zip and roxmltree. Office's
//! old binary formats, `.doc` and `.ppt`, are not read: save them as
//! `.docx` and `.pptx`.
//!
//! - [`package`]: the zip, its parts and the relationships between them.
//! - [`xml`]: elements and attributes by the names Office writes them with.
//! - [`media`]: pictures, written aside for the markdown to name.
//! - [`inline`]: text, its styles and links, tables and front matter, as
//!   markdown.
//! - [`omml`]: Office Math as LaTeX.
//! - [`docx`] and [`pptx`]: the documents and the presentations, as
//!   markdown.
//! - [`deck`]: a presentation laid out as its slides are, which is how it
//!   is shown; its markdown is what an assistant reads.
//!
//! How equations are read, and which parts of a package hold what, follow
//! the Office Math and Office Open XML readers of storytold's craft-libs
//! (MIT or Apache-2.0), written again for markdown.

use std::path::Path;

pub mod deck;
mod docx;
mod inline;
mod media;
mod omml;
mod package;
mod pptx;
mod xml;

use crate::render::{Office, office_kind};
use media::Media;
use package::Package;

/// A Word or PowerPoint file read as markdown.
pub struct Read {
    pub markdown: String,
    /// What was left out, for the status line.
    pub notes: Vec<String>,
    /// Where the pictures were written: the markdown names them, so this
    /// has to be kept while they are typeset.
    pub kept: Option<tempfile::TempDir>,
}

/// Whether a file's pictures are written aside, to be typeset, or only
/// named by their parts, as `word/media/image1.png`, to be read.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pictures {
    Written,
    Named,
}

/// Whether `path` names a PowerPoint deck, laid out as its slides are.
pub fn slides(path: &Path) -> bool {
    office_kind(path) == Some(Office::Slides)
}

/// `n` of `what`, as `1 picture` or `3 pictures`.
pub(crate) fn count(n: usize, what: &str) -> String {
    format!("{n} {what}{}", if n == 1 { "" } else { "s" })
}

/// The Word or PowerPoint file at `path`, as markdown.
pub fn read(path: &Path, pictures: Pictures) -> anyhow::Result<Read> {
    let kind = match office_kind(path) {
        Some(Office::Old(new)) => {
            let old = path.extension().unwrap_or_default().to_string_lossy();
            anyhow::bail!(
                "ohp reads .{new}, not Office's old .{old}: save {} as .{new}",
                path.display()
            )
        }
        Some(kind) => kind,
        None => anyhow::bail!("{} is not a Word or PowerPoint file", path.display()),
    };
    let mut pkg = Package::read(path)?;
    let mut media = Media::new(pictures);
    let markdown = match kind {
        Office::Slides => pptx::read(&mut pkg, &mut media)?,
        _ => docx::read(&mut pkg, &mut media)?,
    };
    Ok(Read {
        markdown,
        notes: media.notes(),
        kept: media.into_dir(),
    })
}

#[cfg(test)]
#[path = "tests/office.rs"]
mod tests;
