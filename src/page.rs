//! A document's pages, drawn as images: what letterbox shows a message as.

use crate::render::{Deck, Options, at_width};
use anyhow::{Context, anyhow};
use hayro::RenderCache;
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use image::RgbaImage;
use std::path::Path;

/// The pages of the file at `path`, a PDF, markdown, R Markdown, HTML or
/// an image, typeset as ohp presents it, its code chunks shown, not run;
/// each drawn `width` pixels wide, on white.
pub fn pages(path: &Path, width: u32) -> anyhow::Result<Vec<RgbaImage>> {
    let options = Options {
        knit: false,
        screen: true,
        ..Options::default()
    };
    let deck = Deck::open_with(&[path.to_path_buf()], options)?;
    let pdf = Pdf::new(deck.data.clone()).map_err(|e| anyhow!("cannot read its pages: {e:?}"))?;
    let cache = RenderCache::new();
    let settings = InterpreterSettings::default();
    pdf.pages()
        .iter()
        .map(|page| at_width(page, &cache, &settings, width).context("a page that cannot be drawn"))
        .collect()
}

#[cfg(test)]
#[path = "tests/page.rs"]
mod tests;
