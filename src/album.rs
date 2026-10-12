//! Several files presented together, as one deck: a PDF's pages as they
//! are, an image on a page of its own, and markdown typeset first. typst
//! sets them one after another into one PDF, each page the shape of what it
//! shows, so the deck is drawn, read as text and read aloud as any other.
//!
//! A file that cannot be read is left out, and named in the status line.

use crate::markdown;
use crate::render::{self, Options, Stopped};
use crate::typeset::{self, Doc, Typeset};
use anyhow::{Context, anyhow};
use hayro::hayro_syntax::Pdf;
use std::collections::HashMap;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use typst::foundations::Bytes;
use typst::visualize::{ExchangeFormat, RasterImage};

/// Points an image's longer side is set at: only its shape matters, as a
/// slide is drawn as large as the screen fits.
const IMAGE_SIDE: f32 = 720.;

/// A file as typst shows it: its data, whether it is a PDF, and the size of
/// each of its pages, in points.
struct Shown {
    data: Arc<[u8]>,
    pdf: bool,
    sizes: Vec<(f32, f32)>,
    note: Option<String>,
}

pub fn gather(paths: &[PathBuf], options: &Options) -> anyhow::Result<Typeset> {
    let mut source = String::from("#set page(margin: 0pt)\n");
    let mut files: Vec<(String, Arc<[u8]>)> = Vec::new();
    let mut notes = Vec::new();
    for path in paths {
        if options.stop.load(Ordering::Relaxed) {
            return Err(Stopped.into());
        }
        let name = render::name(path);
        let shown = match show(path, options) {
            Ok(shown) => shown,
            Err(e) if e.is::<Stopped>() => return Err(e),
            Err(e) => {
                notes.push(format!("cannot show {name}: {e:#}"));
                continue;
            }
        };
        // typst knows an image by what is in it, but a PDF by its name.
        let file = format!("/{}{}", files.len(), if shown.pdf { ".pdf" } else { "" });
        let id = markdown::string(&file);
        for (page, (w, h)) in shown.sizes.iter().enumerate() {
            let page = if shown.pdf {
                format!(", page: {}", page + 1)
            } else {
                String::new()
            };
            let _ = writeln!(
                source,
                "#page(width: {w}pt, height: {h}pt, image({id}{page}, width: 100%, height: 100%, fit: \"contain\"))"
            );
        }
        notes.extend(shown.note.map(|note| format!("{name}: {note}")));
        files.push((file, shown.data));
    }
    let note = (!notes.is_empty()).then(|| notes.join("; "));
    if files.is_empty() {
        return Err(anyhow!("{}", note.unwrap_or_default()));
    }
    let world = Doc::new(source, &[], &files, &mut HashMap::new());
    let pdf = typeset::compile(&world).map_err(|e| anyhow!("{}", e[0].message))?;
    Ok(Typeset { pdf, note })
}

/// `path` read, or typeset, and measured.
fn show(path: &Path, options: &Options) -> anyhow::Result<Shown> {
    if render::markdown(path) {
        let done = typeset::typeset(path, options)?;
        return pdf(done.pdf.into(), done.note);
    }
    let data: Arc<[u8]> = std::fs::read(path)
        .with_context(|| format!("reading {}", path.display()))?
        .into();
    let Some(format) = ExchangeFormat::detect(&data) else {
        return pdf(data, None);
    };
    let image = RasterImage::plain(Bytes::new(data.clone()), format).map_err(|e| anyhow!("{e}"))?;
    let (w, h) = (image.width() as f32, image.height() as f32);
    let scale = IMAGE_SIDE / w.max(h);
    Ok(Shown {
        data,
        pdf: false,
        sizes: vec![(w * scale, h * scale)],
        note: None,
    })
}

fn pdf(data: Arc<[u8]>, note: Option<String>) -> anyhow::Result<Shown> {
    let doc =
        Pdf::new(Arc::new(data.clone())).map_err(|e| anyhow!("not a PDF or an image: {e:?}"))?;
    let sizes = render::sizes(&doc);
    if sizes.is_empty() {
        return Err(anyhow!("no pages"));
    }
    Ok(Shown {
        data,
        pdf: true,
        sizes,
        note,
    })
}

#[cfg(test)]
#[path = "tests/album.rs"]
mod tests;
