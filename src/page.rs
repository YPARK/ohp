//! A document's pages, drawn as images: what letterbox shows a message as.

use crate::render::{Deck, Options, raster};
use anyhow::{Context, anyhow};
use hayro::RenderCache;
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use image::RgbaImage;
use image::imageops::{self, FilterType};
use std::path::Path;

/// The pages of the file at `path`, a PDF, markdown, R Markdown, HTML or
/// an image, typeset as ohp presents it, its code chunks shown, not run;
/// each drawn `width` pixels wide, on white.
pub fn pages(path: &Path, width: u32) -> anyhow::Result<Vec<RgbaImage>> {
    let options = Options {
        knit: false,
        ..Options::default()
    };
    let deck = Deck::open_with(&[path.to_path_buf()], options)?;
    let pdf = Pdf::new(deck.data.clone()).map_err(|e| anyhow!("cannot read its pages: {e:?}"))?;
    let cache = RenderCache::new();
    let settings = InterpreterSettings::default();
    let width = width.max(1);
    pdf.pages()
        .iter()
        .map(|page| {
            let (w, _) = page.render_dimensions();
            let scale = width as f32 / w;
            let drawn = raster(page, &cache, &settings, scale, scale)
                .context("a page that cannot be drawn")?;
            Ok(if drawn.width() == width {
                drawn
            } else {
                let height = (drawn.height() as f32 * width as f32 / drawn.width() as f32)
                    .round()
                    .max(1.0) as u32;
                imageops::resize(&drawn, width, height, FilterType::Triangle)
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "tests/page.rs"]
mod tests;
