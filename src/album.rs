//! Several files presented together, as one deck: a PDF's pages as they
//! are, an image on a page of its own, and markdown typeset first. typst
//! sets them one after another into one PDF, each page the shape of what it
//! shows, so the deck is drawn, read as text and read aloud as any other.
//!
//! A file that cannot be read is left out, and named in the status line.
//! Each file is kept as it was shown, so when one of the deck's files
//! changes, only that one is read again: a figure redrawn does not knit
//! the markdown beside it again.

use crate::markdown;
use crate::render::{self, Options, Stamp, Stopped};
use crate::typeset::{self, Doc, Typeset};
use anyhow::{Context, anyhow, bail};
use hayro::hayro_syntax::Pdf;
use image::imageops::FilterType;
use image::metadata::Orientation;
use image::{DynamicImage, GenericImageView, ImageDecoder, ImageFormat, ImageReader};
use std::collections::HashMap;
use std::fmt::Write;
use std::io::Cursor;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use typst::WorldExt;

/// Points an image's longer side is set at: only its shape matters, as a
/// slide is drawn as large as the screen fits.
const IMAGE_SIDE: f32 = 720.;

/// Most pixels across or down an image is shown with, more than a screen
/// has; a larger photo is made this small first, as one taken by a phone,
/// so it is not set, nor drawn on every turn to it, at its full size.
const IMAGE_PIXELS: u32 = 4096;

/// A file as typst shows it.
pub struct Shown {
    data: Arc<[u8]>,
    kind: Kind,
    note: Option<String>,
}

enum Kind {
    /// A PDF with pages of these sizes, in points.
    Pdf(Vec<(f32, f32)>),
    /// An image typst reads, of this many pixels across and down, upright.
    Image(u32, u32),
    /// Pixels as typst takes them, `rgb8` or `rgba8`.
    Pixels {
        encoding: &'static str,
        width: u32,
        height: u32,
    },
    /// A drawing, as large as it says it is.
    Svg,
}

impl Shown {
    fn pages(&self) -> usize {
        match &self.kind {
            Kind::Pdf(sizes) => sizes.len(),
            _ => 1,
        }
    }

    /// Set the file's pages in `source`, the file named as typst is to find
    /// it, and return that name.
    fn set(&self, n: usize, source: &mut String) -> String {
        let file = match self.kind {
            Kind::Pdf(_) => format!("/{n}.pdf"),
            Kind::Svg => format!("/{n}.svg"),
            // typst knows an image by what is in it.
            Kind::Image(..) | Kind::Pixels { .. } => format!("/{n}"),
        };
        let id = markdown::string(&file);
        let fill = "width: 100%, height: 100%, fit: \"contain\"";
        let mut page = |(w, h): (f32, f32), image: String| {
            let _ = writeln!(source, "#page(width: {w}pt, height: {h}pt, {image})");
        };
        match &self.kind {
            Kind::Pdf(sizes) => {
                for (k, &size) in sizes.iter().enumerate() {
                    page(size, format!("image({id}, page: {}, {fill})", k + 1));
                }
            }
            &Kind::Image(width, height) => {
                page(fitted(width, height), format!("image({id}, {fill})"))
            }
            &Kind::Pixels {
                encoding,
                width,
                height,
            } => {
                let format =
                    format!("(encoding: \"{encoding}\", width: {width}, height: {height})");
                page(
                    fitted(width, height),
                    format!("image({id}, format: {format}, {fill})"),
                );
            }
            Kind::Svg => {
                let _ = writeln!(source, "#page(width: auto, height: auto, image({id}))");
            }
        }
        file
    }
}

/// The page an image of `width` × `height` pixels is set on, in points.
fn fitted(width: u32, height: u32) -> (f32, f32) {
    let (w, h) = (width as f32, height as f32);
    let scale = IMAGE_SIDE / w.max(h);
    (w * scale, h * scale)
}

/// The files a deck was made of, as they were shown, to show again while
/// they are unchanged. Shared by the deck's reloads through its options.
#[derive(Default)]
pub struct Cache(Mutex<HashMap<PathBuf, (Stamp, Arc<Shown>)>>);

impl std::fmt::Debug for Cache {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("Cache")
    }
}

impl Cache {
    /// `path` as it was shown, if it has not changed since; else read.
    fn show(&self, path: &Path, options: &Options) -> anyhow::Result<Arc<Shown>> {
        let stamp = render::stamp(path);
        let kept = self.0.lock().expect("cache").get(path).cloned();
        if let Some((at, shown)) = kept
            && stamp.is_some()
            && at == stamp
        {
            return Ok(shown);
        }
        let shown = Arc::new(show(path, options)?);
        let mut cache = self.0.lock().expect("cache");
        cache.insert(path.to_path_buf(), (stamp, shown.clone()));
        Ok(shown)
    }

    /// Forget all but `paths`.
    fn keep(&self, paths: &[PathBuf]) {
        self.0
            .lock()
            .expect("cache")
            .retain(|p, _| paths.contains(p));
    }

    /// Forget every file, so each is read again.
    pub fn clear(&self) {
        self.0.lock().expect("cache").clear();
    }
}

pub fn gather(paths: &[PathBuf], options: &Options) -> anyhow::Result<Typeset> {
    let mut notes = Vec::new();
    // Each file shown: where it is among `paths`, its name, and it.
    let mut shown = Vec::new();
    for (i, path) in paths.iter().enumerate() {
        if options.stop.load(Ordering::Relaxed) {
            return Err(Stopped.into());
        }
        let name = render::name(path);
        match options.cache.show(path, options) {
            Ok(file) => {
                notes.extend(file.note.as_ref().map(|note| format!("{name}: {note}")));
                shown.push((i, name, file));
            }
            Err(e) if e.is::<Stopped>() => return Err(e),
            Err(e) => notes.push(format!("cannot show {name}: {e:#}")),
        }
    }
    options.cache.keep(paths);
    // A file typst cannot set, as an image it cannot decode, is left out,
    // and the rest set again.
    loop {
        if shown.is_empty() {
            bail!("{}", notes.join("; "));
        }
        let mut source = String::from("#set page(margin: 0pt)\n");
        let mut files = Vec::new();
        let mut spans: Vec<Range<usize>> = Vec::new();
        let mut from = Vec::new();
        for (n, (i, _, file)) in shown.iter().enumerate() {
            let start = source.len();
            files.push((file.set(n, &mut source), file.data.clone()));
            spans.push(start..source.len());
            from.extend(std::iter::repeat_n(*i, file.pages()));
        }
        let world = Doc::new(source, &[], &files, &mut HashMap::new());
        let errors = match typeset::compile(&world) {
            Ok(pdf) => {
                let note = (!notes.is_empty()).then(|| notes.join("; "));
                return Ok(Typeset { pdf, note, from });
            }
            Err(errors) => errors,
        };
        let mut bad: Vec<(usize, &str)> = errors
            .iter()
            .filter_map(|e| {
                let at = world.range(e.span)?.start;
                let n = spans.iter().position(|s| s.contains(&at))?;
                Some((n, e.message.as_str()))
            })
            .collect();
        if bad.is_empty() {
            bail!("{}", errors[0].message);
        }
        bad.sort_by_key(|&(n, _)| std::cmp::Reverse(n));
        bad.dedup_by_key(|&mut (n, _)| n);
        for (n, message) in bad {
            let (_, name, _) = shown.remove(n);
            notes.push(format!("cannot show {name}: {message}"));
        }
    }
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
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("svg"))
    {
        return Ok(Shown {
            data,
            kind: Kind::Svg,
            note: None,
        });
    }
    match image::guess_format(&data) {
        Ok(
            format @ (ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Gif | ImageFormat::WebP),
        ) => raster(data, format),
        _ => pdf(data, None),
    }
}

/// An image, measured from its header; made smaller first if it is larger
/// than a screen shows.
fn raster(data: Arc<[u8]>, format: ImageFormat) -> anyhow::Result<Shown> {
    let mut decoder = ImageReader::with_format(Cursor::new(&*data), format).into_decoder()?;
    let orientation = decoder.orientation()?;
    let (width, height) = decoder.dimensions();
    if width.max(height) <= IMAGE_PIXELS {
        drop(decoder);
        let turned = matches!(
            orientation,
            Orientation::Rotate90
                | Orientation::Rotate270
                | Orientation::Rotate90FlipH
                | Orientation::Rotate270FlipH
        );
        let (width, height) = if turned {
            (height, width)
        } else {
            (width, height)
        };
        return Ok(Shown {
            data,
            kind: Kind::Image(width, height),
            note: None,
        });
    }
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    let image = image.resize(IMAGE_PIXELS, IMAGE_PIXELS, FilterType::Triangle);
    let (width, height) = image.dimensions();
    let (encoding, pixels) = if image.color().has_alpha() {
        ("rgba8", image.into_rgba8().into_raw())
    } else {
        ("rgb8", image.into_rgb8().into_raw())
    };
    Ok(Shown {
        data: pixels.into(),
        kind: Kind::Pixels {
            encoding,
            width,
            height,
        },
        note: None,
    })
}

fn pdf(data: Arc<[u8]>, note: Option<String>) -> anyhow::Result<Shown> {
    let doc =
        Pdf::new(Arc::new(data.clone())).map_err(|e| anyhow!("not a PDF or an image: {e:?}"))?;
    let sizes = render::sizes(&doc);
    if sizes.is_empty() {
        bail!("no pages");
    }
    Ok(Shown {
        data,
        kind: Kind::Pdf(sizes),
        note,
    })
}

#[cfg(test)]
#[path = "tests/album.rs"]
mod tests;
