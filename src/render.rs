//! The deck, and slides rasterised or read as text off the UI thread.
//!
//! Each worker parses its own copy of the PDF: hayro's caches are not shared
//! across threads. Jobs wait in one queue that the UI empties and refills
//! whenever the view changes, so slides moved past are never rendered.
//! Workers also encode each slide for the terminal, which for kitty over ssh
//! means compressing it, so the UI thread only places finished images.

use crate::text;
use anyhow::{Context, anyhow};
use hayro::hayro_interpret::{InterpreterCache, InterpreterSettings};
use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::page::Page;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::vello_cpu::kurbo;
use hayro::{RenderCache, RenderSettings};
use image::imageops::{self, FilterType};
use image::{DynamicImage, RgbaImage};
use ratatui::layout::{Rect, Size};
use ratatui_image::Resize;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::Protocol;
use ratatui_image::protocol::halfblocks::Halfblocks;
use std::collections::VecDeque;
use std::io::{Read, Seek, SeekFrom};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::SystemTime;

/// Name of the worker threads, so their panics can be told apart.
pub const THREAD: &str = "render";

pub struct Deck {
    /// The files the deck is made of, most often one.
    pub paths: Vec<PathBuf>,
    /// Which of `paths` each page is from.
    pub from: Vec<usize>,
    pub data: Arc<Vec<u8>>,
    pub pages: usize,
    /// Width and height of each page, in points.
    pub sizes: Vec<(f32, f32)>,
    pub options: Options,
    /// What the status line should say about how the deck was made.
    pub note: Option<String>,
}

/// Making a deck stopped, as ohp quits: not worth saying as an error.
#[derive(Debug)]
pub struct Stopped;

impl std::fmt::Display for Stopped {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("stopped")
    }
}

impl std::error::Error for Stopped {}

/// How a deck that is not a PDF is made into one.
#[derive(Clone, Debug)]
#[cfg_attr(not(feature = "markdown"), allow(dead_code))]
pub struct Options {
    /// A typst paper size for markdown; its own default if `None`.
    pub paper: Option<String>,
    /// Run code chunks with knitr, where R is here.
    pub knit: bool,
    /// Set to stop typesetting under way, as a knitr run; shared by the
    /// options' clones, so by every reload of a deck.
    pub stop: Arc<AtomicBool>,
    /// The files of a deck of several, as shown, kept for its reloads;
    /// shared as `stop` is.
    #[cfg(feature = "markdown")]
    pub cache: Arc<crate::album::Cache>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            paper: None,
            knit: true,
            stop: Arc::default(),
            #[cfg(feature = "markdown")]
            cache: Arc::default(),
        }
    }
}

impl Options {
    /// Read every file again at the next reload, though unchanged: one may
    /// read others ohp does not watch, as markdown its bibliography.
    pub fn forget(&self) {
        #[cfg(feature = "markdown")]
        self.cache.clear();
    }
}

/// When a file last changed, and its length: enough to notice a rewrite.
pub type Stamp = Option<(SystemTime, u64)>;

pub fn stamp(path: &Path) -> Stamp {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// Whether the file ends the way a finished PDF does. LaTeX may pause while
/// writing one, so a file that stopped changing is not necessarily done.
/// Markdown is written by an editor, and an image by its program, all at
/// once.
pub fn finished(path: &Path) -> bool {
    if markdown(path) || image(path) {
        return path.is_file();
    }
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let len = file.metadata().map_or(0, |m| m.len());
    let mut tail = Vec::new();
    file.seek(SeekFrom::Start(len.saturating_sub(1024))).is_ok()
        && file.read_to_end(&mut tail).is_ok()
        && tail.windows(5).any(|w| w == b"%%EOF")
}

/// Whether `path` names markdown or R Markdown, typeset rather than read.
pub fn markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "md" | "markdown" | "rmd"))
}

/// Whether `path` names an image, shown as a slide of its own.
pub fn image(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg"
        )
    })
}

/// Whether `paths` is a PDF, read as it is rather than typeset.
pub fn pdf(paths: &[PathBuf]) -> bool {
    matches!(paths, [path] if !markdown(path) && !image(path))
}

/// The first of `paths`, which are some, and how many follow it.
pub fn shown(paths: &[PathBuf]) -> String {
    let first = paths[0].display();
    match paths.len() {
        1 => first.to_string(),
        n => format!("{first} +{}", n - 1),
    }
}

/// The name of the file `path` names.
pub fn name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into(),
    )
}

/// Width and height of each of `pdf`'s pages, in points.
pub fn sizes(pdf: &Pdf) -> Vec<(f32, f32)> {
    pdf.pages().iter().map(Page::render_dimensions).collect()
}

/// A PDF made of files, what the status line should say about how, and
/// which file each page is from, if they are several; else empty.
type Made = (Vec<u8>, Option<String>, Vec<usize>);

#[cfg(feature = "markdown")]
fn typeset(paths: &[PathBuf], options: &Options) -> anyhow::Result<Made> {
    let (done, from) = match paths {
        [path] if markdown(path) => (crate::typeset::typeset(path, options)?, Vec::new()),
        _ => crate::album::gather(paths, options)?,
    };
    Ok((done.pdf, done.note, from))
}

#[cfg(not(feature = "markdown"))]
fn typeset(paths: &[PathBuf], _: &Options) -> anyhow::Result<Made> {
    let why = match paths {
        [path] if markdown(path) => format!("{} is markdown", path.display()),
        [path] => format!("{} is an image", path.display()),
        _ => "showing several files together needs typst".into(),
    };
    anyhow::bail!("{why}, and ohp was built without its `markdown` feature")
}

impl Deck {
    #[cfg(test)]
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        Self::open_with(&[path.to_path_buf()], Options::default())
    }

    /// The deck in `paths`: a PDF, or markdown typeset into one, or several
    /// files, PDFs, images and markdown, set one after another into one.
    pub fn open_with(paths: &[PathBuf], options: Options) -> anyhow::Result<Self> {
        let [first, ..] = paths else {
            anyhow::bail!("no file to present");
        };
        let (data, note, from) = if pdf(paths) {
            let data =
                std::fs::read(first).with_context(|| format!("reading {}", first.display()))?;
            (data, None, Vec::new())
        } else {
            typeset(paths, &options)?
        };
        let data = Arc::new(data);
        let what = shown(paths);
        let pdf = Pdf::new(data.clone()).map_err(|e| anyhow!("cannot open {what}: {e:?}"))?;
        let sizes = sizes(&pdf);
        if sizes.is_empty() {
            anyhow::bail!("{what} has no pages");
        }
        let from = if from.is_empty() {
            vec![0; sizes.len()]
        } else {
            from
        };
        Ok(Deck {
            paths: paths.to_vec(),
            from,
            pages: sizes.len(),
            sizes,
            data,
            options,
            note,
        })
    }

    /// Which of `paths` page `page` is from.
    pub fn source(&self, page: usize) -> usize {
        self.from[page]
    }

    /// When each of the deck's files last changed.
    pub fn stamps(&self) -> Vec<Stamp> {
        self.paths.iter().map(|p| stamp(p)).collect()
    }

    /// Whether every one of the deck's files is written in full.
    pub fn finished(&self) -> bool {
        self.paths.iter().all(|p| finished(p))
    }
}

/// Most pixels a zoomed page is rendered with: hayro renders whole pages,
/// so past this the part shown is rendered smaller and enlarged.
const MAX_PIXELS: f32 = 16e6;

/// A slide fitted into a box of terminal cells, or part of it enlarged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub page: usize,
    pub cols: u16,
    pub rows: u16,
    pub zoom: Zoom,
}

/// How much a slide is enlarged past fitting its box, and which part of it
/// the box shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Zoom {
    pub percent: u16,
    /// The box's top-left within the enlarged slide, in cells; kept within it.
    pub x: u16,
    pub y: u16,
}

impl Zoom {
    pub const FIT: Zoom = Zoom {
        percent: 100,
        x: 0,
        y: 0,
    };

    pub fn scale(self) -> f32 {
        f32::from(self.percent) / 100.
    }
}

impl Key {
    pub fn new(page: usize, area: Rect) -> Self {
        Key {
            page,
            cols: area.width,
            rows: area.height,
            zoom: Zoom::FIT,
        }
    }

    pub fn size(self) -> Size {
        Size::new(self.cols, self.rows)
    }
}

/// How a slide is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Look {
    Image,
    Text,
}

/// Work for a render worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Job {
    pub key: Key,
    pub look: Look,
    /// The sentence being read aloud, lit on the slide.
    pub lit: Option<usize>,
    /// The slide's key sentences, marked on it, as its summary picks them.
    pub marked: text::Picked,
}

pub enum Slide {
    Image(Protocol),
    /// The slide's text, set over the cells of `backdrop`.
    Text {
        cells: text::Cells,
        backdrop: Protocol,
    },
}

/// A finished job; `slide` is `None` when it could not be done.
pub struct Done {
    pub job: Job,
    pub slide: Option<Slide>,
}

#[derive(Default)]
struct Jobs {
    waiting: VecDeque<Job>,
    /// The renderer was dropped: workers exit.
    closed: bool,
}

type Queue = Arc<(Mutex<Jobs>, Condvar)>;

/// A page drawn unlit, with its number and the pixels a point it was drawn
/// at, so the next sentence read aloud is lit on it without drawing the
/// page again. The workers share it, as any of them may take that sentence.
type Kept = Mutex<Option<((usize, f32, f32), Arc<RgbaImage>)>>;

/// Workers for one version of the deck; dropping it stops them.
pub struct Renderer {
    queue: Queue,
    pub done: Receiver<Done>,
}

impl Drop for Renderer {
    fn drop(&mut self) {
        let (lock, cvar) = &*self.queue;
        lock.lock().expect("render queue").closed = true;
        cvar.notify_all();
    }
}

impl Renderer {
    pub fn spawn(deck: &Deck, picker: &Picker, workers: usize) -> anyhow::Result<Self> {
        let queue: Queue = Arc::default();
        let kept: Arc<Kept> = Arc::default();
        let (tx, rx) = channel();
        for _ in 0..workers.max(1) {
            let (data, queue, kept, tx, picker) = (
                deck.data.clone(),
                queue.clone(),
                kept.clone(),
                tx.clone(),
                picker.clone(),
            );
            std::thread::Builder::new()
                .name(THREAD.into())
                .spawn(move || work(data, &picker, &queue, &kept, &tx))?;
        }
        Ok(Renderer { queue, done: rx })
    }

    /// Drop the jobs no worker has started, returning them.
    pub fn clear(&self) -> Vec<Job> {
        let (lock, _) = &*self.queue;
        lock.lock()
            .expect("render queue")
            .waiting
            .drain(..)
            .collect()
    }

    pub fn push(&self, jobs: impl IntoIterator<Item = Job>) {
        let (lock, cvar) = &*self.queue;
        lock.lock().expect("render queue").waiting.extend(jobs);
        cvar.notify_all();
    }
}

fn work(data: Arc<Vec<u8>>, picker: &Picker, queue: &Queue, kept: &Kept, tx: &Sender<Done>) {
    let Ok(pdf) = Pdf::new(data) else {
        return;
    };
    let cache = RenderCache::new();
    let fonts = InterpreterCache::new();
    let settings = InterpreterSettings::default();
    // The text of the page last asked for, kept as each sentence read aloud
    // asks for the page again.
    let mut text: Option<(usize, text::Page)> = None;
    let (lock, cvar) = &**queue;
    loop {
        let job = {
            let mut jobs = lock.lock().expect("render queue");
            loop {
                if jobs.closed {
                    return;
                }
                if let Some(job) = jobs.waiting.pop_front() {
                    break job;
                }
                jobs = cvar.wait(jobs).expect("render queue");
            }
        };
        let slide = caught(|| match job.look {
            Look::Image => {
                // Where the sentence read aloud is, and the key sentences,
                // in page points.
                let (mut lit, mut marked) = (Vec::new(), Vec::new());
                if (job.lit.is_some() || !job.marked.is_empty())
                    && let Some(page) = text_of(&mut text, &pdf, job.key.page, &fonts, &settings)
                {
                    lit = job.lit.map(|k| page.light(k).boxes).unwrap_or_default();
                    marked = page.mark(job.marked).boxes;
                }
                let image = rasterise(
                    &pdf,
                    &cache,
                    &settings,
                    picker,
                    job.key,
                    (&lit, &marked),
                    kept,
                )?;
                let image = DynamicImage::ImageRgba8(image);
                let proto = picker.new_protocol(image, job.key.size(), Resize::Fit(None));
                proto.ok().map(Slide::Image)
            }
            Look::Text => {
                let page = text_of(&mut text, &pdf, job.key.page, &fonts, &settings)?;
                read(&pdf, &cache, page, &settings, picker, job, kept)
            }
        });
        if tx.send(Done { job, slide }).is_err() {
            return;
        }
    }
}

/// The text of `page`, from `last` if it is that page's, else extracted
/// and kept there.
fn text_of<'p, 'a>(
    last: &'p mut Option<(usize, text::Page)>,
    pdf: &'a Pdf,
    page: usize,
    fonts: &InterpreterCache<'a>,
    settings: &InterpreterSettings,
) -> Option<&'p mut text::Page> {
    if last.as_ref().is_none_or(|(at, _)| *at != page) {
        *last = Some((page, text::extract(pdf, page, fonts, settings)?));
    }
    last.as_mut().map(|(_, text)| text)
}

/// The slide's text set over a coarse image of it, in coloured half-block
/// cells, which any terminal shows. Zoomed, the text is set on a grid as
/// much larger, and the box shows part of it. The sentence being read
/// aloud is lit.
fn read<'a>(
    pdf: &'a Pdf,
    cache: &RenderCache<'a>,
    text: &mut text::Page,
    settings: &InterpreterSettings,
    picker: &Picker,
    job: Job,
    kept: &Kept,
) -> Option<Slide> {
    let key = job.key;
    let page = pdf.pages().get(key.page)?;
    let (w, h) = page.render_dimensions();
    let font = picker.font_size();
    let (fw, fh) = (f32::from(font.width), f32::from(font.height));
    let enlarge = |len: u16| (f32::from(len) * key.zoom.scale()).min(f32::from(u16::MAX)) as u16;
    let whole = Key {
        cols: enlarge(key.cols),
        rows: enlarge(key.rows),
        ..key
    };
    let scale = fit(whole, (fw, fh), (w, h));
    let cells = |v: f32, box_: u16| (v.round() as u16).min(box_);
    let size = Size::new(
        cells(w * scale / fw, whole.cols),
        cells(h * scale / fh, whole.rows),
    );
    // Four pixels a half-block each way, so glyphs are painted out finely.
    let x_scale = 4. * f32::from(size.width) / w;
    let y_scale = 8. * f32::from(size.height) / h;
    let lit = job.lit.map(|k| text.light(k).glyphs).unwrap_or_default();
    let marked = text.mark(job.marked).glyphs;
    let image = unlit(
        kept,
        key.page,
        page,
        cache,
        settings,
        (x_scale, y_scale),
        lit.contains(&true) || marked.contains(&true),
    )?;
    let mut image = Arc::unwrap_or_clone(image);
    let cells = text.set_over(&mut image, (size.width, size.height), &lit, &marked);

    let shown = Size::new(size.width.min(key.cols), size.height.min(key.rows));
    let x = key.zoom.x.min(size.width - shown.width);
    let y = key.zoom.y.min(size.height - shown.height);
    let span = |at: u16, len: u16| usize::from(at)..usize::from(at + len);
    let cells = cells[span(y, shown.height)]
        .iter()
        .map(|row| row[span(x, shown.width)].to_vec())
        .collect();
    let (px, py) = (4 * u32::from(x), 8 * u32::from(y));
    let (pw, ph) = (4 * u32::from(shown.width), 8 * u32::from(shown.height));
    let part = imageops::crop_imm(&image, px, py, pw, ph).to_image();
    let backdrop = Halfblocks::new(DynamicImage::ImageRgba8(part), shown).ok()?;
    Some(Slide::Text {
        cells,
        backdrop: Protocol::Halfblocks(backdrop),
    })
}

/// `f`'s result, or `None` if it panics: hayro does on some malformed PDFs.
pub fn caught<T>(f: impl FnOnce() -> Option<T>) -> Option<T> {
    panic::catch_unwind(AssertUnwindSafe(f)).ok().flatten()
}

/// The scale that fits a `page`-sized page into the pixels behind `key`'s
/// cells of `font` size.
pub fn fit(key: Key, font: (f32, f32), page: (f32, f32)) -> f32 {
    let (bw, bh) = (f32::from(key.cols) * font.0, f32::from(key.rows) * font.1);
    (bw / page.0).min(bh / page.1)
}

/// Page `key.page` as large as fits the pixels behind `key`'s cells, on
/// white, the boxes `lit` lit and `marked` marked; when zoomed, the part
/// of it enlarged that the box shows.
fn rasterise<'a>(
    pdf: &'a Pdf,
    cache: &RenderCache<'a>,
    settings: &InterpreterSettings,
    picker: &Picker,
    key: Key,
    (lit, marked): (&[kurbo::Rect], &[kurbo::Rect]),
    kept: &Kept,
) -> Option<RgbaImage> {
    let page = pdf.pages().get(key.page)?;
    let font = picker.font_size();
    let (fw, fh) = (f32::from(font.width), f32::from(font.height));
    let (w, h) = page.render_dimensions();
    let scale = fit(key, (fw, fh), (w, h)) * key.zoom.scale();
    let drawn = if key.zoom == Zoom::FIT {
        scale
    } else {
        scale.min((MAX_PIXELS / (w * h)).sqrt())
    };
    let whole = unlit(
        kept,
        key.page,
        page,
        cache,
        settings,
        (drawn, drawn),
        !lit.is_empty() || !marked.is_empty(),
    )?;
    if key.zoom == Zoom::FIT {
        let mut image = Arc::unwrap_or_clone(whole);
        light(&mut image, (lit, marked), drawn, (0, 0));
        return Some(image);
    }
    // The box, in the pixels drawn.
    let k = drawn / scale;
    let (bw, bh) = (f32::from(key.cols) * fw * k, f32::from(key.rows) * fh * k);
    let (x, y) = (
        f32::from(key.zoom.x) * fw * k,
        f32::from(key.zoom.y) * fh * k,
    );
    let cut = |at: f32, len: f32, max: u32| {
        let len = (len as u32).clamp(1, max);
        ((at as u32).min(max - len), len)
    };
    let (x, cw) = cut(x, bw, whole.width());
    let (y, ch) = cut(y, bh, whole.height());
    // Only the part shown is copied and lit, not the whole page drawn.
    let mut part = imageops::crop_imm(&*whole, x, y, cw, ch).to_image();
    light(&mut part, (lit, marked), drawn, (x, y));
    if k >= 1. {
        return Some(part);
    }
    let (ow, oh) = (
        (cw as f32 / k).round() as u32,
        (ch as f32 / k).round() as u32,
    );
    Some(imageops::resize(&part, ow, oh, FilterType::Triangle))
}

/// Page number `n`, `page`, drawn unlit as `raster` draws it at `scale`
/// pixels a point, x and y: the one kept if it was drawn so, else drawn,
/// and kept if a sentence is `lit` on it.
fn unlit<'a>(
    kept: &Kept,
    n: usize,
    page: &'a Page<'a>,
    cache: &RenderCache<'a>,
    settings: &InterpreterSettings,
    scale: (f32, f32),
    lit: bool,
) -> Option<Arc<RgbaImage>> {
    let at = (n, scale.0, scale.1);
    let hit = (kept.lock().expect("kept page").as_ref())
        .filter(|(drawn, _)| *drawn == at)
        .map(|(_, image)| image.clone());
    if hit.is_some() {
        return hit;
    }
    let image = Arc::new(raster(page, cache, settings, scale.0, scale.1)?);
    if lit {
        *kept.lock().expect("kept page") = Some((at, image.clone()));
    }
    Some(image)
}

/// The page drawn `x_scale` and `y_scale` pixels a point, on white.
pub fn raster<'a>(
    page: &'a Page<'a>,
    cache: &RenderCache<'a>,
    settings: &InterpreterSettings,
    x_scale: f32,
    y_scale: f32,
) -> Option<RgbaImage> {
    let (w, h) = page.render_dimensions();
    let sane = |s: f32, len: f32| s.is_finite() && len * s >= 1.;
    if !(sane(x_scale, w) && sane(y_scale, h)) {
        return None;
    }
    let render = RenderSettings {
        x_scale,
        y_scale,
        bg_color: WHITE,
        ..Default::default()
    };
    let pixmap = hayro::render(page, cache, settings, &render);
    // Premultiplied, but every pixel is opaque over the white background.
    RgbaImage::from_raw(
        pixmap.width().into(),
        pixmap.height().into(),
        pixmap.data_as_u8_slice().to_vec(),
    )
}

/// Light the boxes in `lit`, and mark those in `marked`, in page points, on
/// `image`: the part from pixel `at` of the page drawn `scale` pixels a
/// point. The light multiplies what is there, so text stays as dark as it
/// was; a pixel both lit and marked is lit only.
fn light(
    image: &mut RgbaImage,
    (lit, marked): (&[kurbo::Rect], &[kurbo::Rect]),
    scale: f32,
    at: (u32, u32),
) {
    if lit.is_empty() && marked.is_empty() {
        return;
    }
    let (w, h) = image.dimensions();
    let px = |v: f64, from: u32, max: u32| {
        ((v * f64::from(scale)).max(0.) as u32)
            .saturating_sub(from)
            .min(max)
    };
    // Boxes overlap, as a line's ascent reaches the descent above it.
    let mut done = vec![false; (w * h) as usize];
    for (boxes, colour) in [(lit, text::LIGHT), (marked, text::MARK)] {
        for r in boxes {
            let (x0, x1) = (px(r.x0, at.0, w), px(r.x1, at.0, w));
            let (y0, y1) = (px(r.y0, at.1, h), px(r.y1, at.1, h));
            for y in y0..y1 {
                for x in x0..x1 {
                    let done = &mut done[(y * w + x) as usize];
                    if std::mem::replace(done, true) {
                        continue;
                    }
                    let p = image.get_pixel_mut(x, y);
                    for (c, l) in p.0.iter_mut().zip(colour) {
                        *c = (u16::from(*c) * u16::from(l) / 255) as u8;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/render.rs"]
mod tests;
