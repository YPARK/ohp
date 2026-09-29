//! The deck, and slides rasterised or read as text off the UI thread.
//!
//! Each worker parses its own copy of the PDF: hayro's caches are not shared
//! across threads. Jobs wait in one queue that the UI empties and refills
//! whenever the view changes, so slides moved past are never rendered.
//! Workers also encode each slide for the terminal, which for kitty over ssh
//! means compressing it, so the UI thread only places finished images.

use crate::text;
use anyhow::{Context, anyhow};
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{RenderCache, RenderSettings};
use image::{DynamicImage, RgbaImage};
use ratatui::layout::{Rect, Size};
use ratatui_image::Resize;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::Protocol;
use std::collections::VecDeque;
use std::io::{Read, Seek, SeekFrom};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::SystemTime;

/// Name of the worker threads, so their panics can be told apart.
pub const THREAD: &str = "render";

pub struct Deck {
    pub path: PathBuf,
    pub name: String,
    pub data: Arc<Vec<u8>>,
    pub pages: usize,
    /// Width and height of the first page, in points.
    pub page_size: (f32, f32),
}

/// When a file last changed, and its length: enough to notice a rewrite.
pub type Stamp = Option<(SystemTime, u64)>;

pub fn stamp(path: &Path) -> Stamp {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// Whether the file ends the way a finished PDF does. LaTeX may pause while
/// writing one, so a file that stopped changing is not necessarily done.
pub fn finished(path: &Path) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let len = file.metadata().map_or(0, |m| m.len());
    let mut tail = Vec::new();
    file.seek(SeekFrom::Start(len.saturating_sub(1024))).is_ok()
        && file.read_to_end(&mut tail).is_ok()
        && tail.windows(5).any(|w| w == b"%%EOF")
}

impl Deck {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let data = Arc::new(data);
        let pdf =
            Pdf::new(data.clone()).map_err(|e| anyhow!("cannot open {}: {e:?}", path.display()))?;
        let pages = pdf.pages();
        let first = pages
            .first()
            .with_context(|| format!("{} has no pages", path.display()))?;
        let page_size = first.render_dimensions();
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into(),
        );
        Ok(Deck {
            path: path.to_path_buf(),
            name,
            pages: pages.len(),
            page_size,
            data,
        })
    }
}

/// A slide fitted into a box of terminal cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub page: usize,
    pub cols: u16,
    pub rows: u16,
}

impl Key {
    pub fn new(page: usize, area: Rect) -> Self {
        Key {
            page,
            cols: area.width,
            rows: area.height,
        }
    }
}

/// Work for a render worker: a slide for its cell box, as an image or as text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Job {
    Image(Key),
    Text(Key),
}

impl Job {
    pub fn key(self) -> Key {
        match self {
            Job::Image(key) | Job::Text(key) => key,
        }
    }
}

pub enum Slide {
    Image(Protocol),
    /// The slide's text, set over `backdrop`'s cells when there is one.
    Text {
        cells: text::Cells,
        backdrop: Option<Protocol>,
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
        let (tx, rx) = channel();
        // Text goes over a coarse image in coloured cells, which any terminal shows.
        let mut blocks = picker.clone();
        blocks.set_protocol_type(ProtocolType::Halfblocks);
        for _ in 0..workers.max(1) {
            let (data, queue, tx) = (deck.data.clone(), queue.clone(), tx.clone());
            let pickers = (picker.clone(), blocks.clone());
            std::thread::Builder::new()
                .name(THREAD.into())
                .spawn(move || work(data, &pickers, &queue, &tx))?;
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

/// `pickers` encode images and text backdrops.
fn work(data: Arc<Vec<u8>>, pickers: &(Picker, Picker), queue: &Queue, tx: &Sender<Done>) {
    let Ok(pdf) = Pdf::new(data) else {
        return;
    };
    let cache = RenderCache::new();
    let settings = InterpreterSettings::default();
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
        let slide = caught(|| match job {
            Job::Image(key) => {
                let image = rasterise(&pdf, &cache, &settings, &pickers.0, key)?;
                encode(&pickers.0, image, key).map(Slide::Image)
            }
            Job::Text(key) => read(&pdf, &cache, &settings, &pickers.1, key),
        });
        if tx.send(Done { job, slide }).is_err() {
            return;
        }
    }
}

fn encode(picker: &Picker, image: RgbaImage, key: Key) -> Option<Protocol> {
    let size = Size::new(key.cols, key.rows);
    picker
        .new_protocol(DynamicImage::ImageRgba8(image), size, Resize::Fit(None))
        .ok()
}

/// The slide's text, set on the cells of a backdrop that has the text
/// painted out; on the whole box if the backdrop cannot be made.
fn read<'a>(
    pdf: &'a Pdf,
    cache: &RenderCache<'a>,
    settings: &InterpreterSettings,
    picker: &Picker,
    key: Key,
) -> Option<Slide> {
    let page = text::extract(pdf, key.page)?;
    let backdrop = rasterise(pdf, cache, settings, picker, key).and_then(|mut image| {
        page.erase(&mut image);
        encode(picker, image, key)
    });
    let size = backdrop
        .as_ref()
        .map_or(Size::new(key.cols, key.rows), Protocol::size);
    let cells = page.layout(size.width, size.height);
    Some(Slide::Text { cells, backdrop })
}

/// `f`'s result, or `None` if it panics: hayro does on some malformed PDFs.
fn caught<T>(f: impl FnOnce() -> Option<T>) -> Option<T> {
    panic::catch_unwind(AssertUnwindSafe(f)).ok().flatten()
}

/// Page `key.page` as large as fits the pixels behind `key`'s cells, on white.
fn rasterise<'a>(
    pdf: &'a Pdf,
    cache: &RenderCache<'a>,
    settings: &InterpreterSettings,
    picker: &Picker,
    key: Key,
) -> Option<RgbaImage> {
    let page = pdf.pages().get(key.page)?;
    let font = picker.font_size();
    let (bw, bh) = (
        f32::from(key.cols) * f32::from(font.width),
        f32::from(key.rows) * f32::from(font.height),
    );
    let (w, h) = page.render_dimensions();
    let scale = (bw / w).min(bh / h);
    if !(scale.is_finite() && w * scale >= 1. && h * scale >= 1.) {
        return None;
    }
    let render = RenderSettings {
        x_scale: scale,
        y_scale: scale,
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

#[cfg(test)]
#[path = "tests/render.rs"]
mod tests;
