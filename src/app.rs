//! The two views, one slide or a grid of them, and the loop that drives them.
//!
//! A slide is rendered for the exact cell box it is drawn in, so each box
//! size is its own cache entry. Input is drained before each redraw, so a
//! burst of key presses costs one frame.
//!
//! The PDF is reloaded when it changes on disk, as after a LaTeX run. It is
//! read, or typeset, off the UI thread, so a knitr run that takes a minute
//! does not freeze the slides. The slides already on screen stay until their
//! new renders arrive, so a reload does not flash the screen empty. `r`
//! reloads at once, as after fixing what made a reload fail.
//!
//! Where the terminal has no graphics protocol, slides are shown as their
//! text over a coarse image instead; `t` switches between the two anywhere.
//!
//! Ctrl-O picks another file to present, in a list over the slides. It is
//! read off the UI thread as a reload is, and the slides it replaces stay
//! until it is.
//!
//! `s` reads the slides aloud, from the current one on, turning to each next
//! one as the last is read, until the end or `s` again. Turning to another
//! slide while reading reads that one. `v` picks the voice it reads in,
//! in a list over the slides, which offers Piper's voices to download too.
//!
//! While reading aloud, a click on the presented slide reads it from the
//! sentence there, found from the word of text set in that cell, or else
//! from the point of the slide the cell shows; otherwise a click does not
//! speak. A click in the grid presents the slide there.
//!
//! `k` sums up the slide on screen, in a column beside the slides, and
//! marks its key sentences on it; the column follows the slides as they
//! turn. `K` sums up the whole file there instead, and marks its key
//! sentences on whatever slides they are. Either again closes the column.
//! `m` picks the model that sums up, in a list over the slides.
//!
//! `A` annotates a PDF: each click puts a signature, text or a mark where
//! it is, in a copy shown in place of the PDF, with a palette of the tools
//! beside it, as `signing` has it. `P` or ctrl-P prints what is shown, as
//! `printing` has it.

use crate::browse::{self, Browser};
#[cfg(feature = "summary")]
use crate::models::Models;
use crate::remote::Link;
use crate::render::{self, Deck, Done, Job, Key, Look, Options, Renderer, Slide, Stamp, Zoom};
use crate::speak::{self, Speaker};
use crate::summary::{self, Summarizer};
use crate::text::Picked;
#[cfg(feature = "speech")]
use crate::voices::{self, Voices};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, DisableMouseCapture};
use ratatui::crossterm::execute;
use ratatui::crossterm::style::Print;
use ratatui::layout::{Rect, Size};
use ratatui::widgets::Block;
use ratatui_image::FontSize;
use ratatui_image::picker::cap_parser::QueryStdioOptions;
use ratatui_image::picker::{Picker, ProtocolType};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

mod draw;
mod field;
mod input;
mod printing;
mod search;
#[cfg(feature = "sign")]
mod signing;

use draw::{marked, split};

/// Slides the grid tries to show at once.
const GRID_TARGET: usize = 12;
const GRID_MAX_COLS: u16 = 8;
/// Zoom steps for the presented slide, in percent of fitting the screen.
const ZOOMS: [u16; 5] = [100, 150, 200, 300, 400];
/// Narrowest grid slot +/- zooms out to, in cells.
const MIN_SLOT_WIDTH: u16 = 10;
/// Full-size slides kept either side of the current one.
const KEEP: usize = 2;
const MAX_WORKERS: usize = 4;
/// How often the PDF is checked for changes.
const WATCH_EVERY: Duration = Duration::from_millis(250);

/// `text` starts with slides as text even where images can be shown.
/// `links` are the connections the slides come over, one a machine, for
/// slides on others.
/// `local` is how files picked with Ctrl-O, which are here, are read.
/// `signaled` is set when a signal asks ohp to stop.
/// `speaker` reads the slides aloud, and `summarizer` sums them up.
pub fn run(
    deck: Deck,
    text: bool,
    links: Vec<Link>,
    local: Options,
    speaker: Speaker,
    summarizer: Summarizer,
    signaled: Arc<AtomicUsize>,
) -> anyhow::Result<()> {
    let mut terminal = ratatui::init();
    // Taken, the terminal's own selecting needs Shift, or Option on macOS.
    // Presses, drags and the wheel, as SGR codes: crossterm's capture would
    // send every move of the mouse too, over ssh as well.
    let _ = execute!(
        std::io::stdout(),
        Print("\x1b[?1000h\x1b[?1002h\x1b[?1006h")
    );
    let result = (|| {
        let picker = pick();
        quiet_render_panics();
        let workers = std::thread::available_parallelism().map_or(1, |n| n.get().min(MAX_WORKERS));
        let renderer = Renderer::spawn(&deck, &picker, workers)?;
        let look = if text || picker.protocol_type() == ProtocolType::Halfblocks {
            Look::Text
        } else {
            Look::Image
        };
        let mut app = App::new(deck, picker, renderer, workers, look, signaled);
        app.notice = app.deck.note.clone();
        app.links = links;
        app.local = local;
        app.speaker = speaker;
        app.summarizer = summarizer;
        app.summarizer
            .set_deck(app.deck.data.clone(), app.deck.pages);
        app.run(&mut terminal)?;
        app.quit_signing();
        // What is being handed to lp, handed over whole before ohp ends.
        app.wait_for_printing(Duration::from_secs(30));
        app.clear_images()
    })();
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}

fn pick() -> Picker {
    // Slides are flat colour and compress well; over ssh the link, not the
    // CPU, is what limits how fast a slide appears.
    let options = QueryStdioOptions {
        kitty_compression: over_ssh(),
        ..Default::default()
    };
    let timeout = options.timeout;
    let asked = Instant::now();
    let mut picker =
        Picker::from_query_stdio_with_options(options).unwrap_or_else(|_| Picker::halfblocks());
    // Unanswered, as when tmux does not pass it on to the terminal, the
    // query leaves a thread reading stdin, which would take the first key
    // pressed. A status report, which tmux and every terminal answer, ends
    // it.
    if asked.elapsed() >= timeout {
        let _ = execute!(std::io::stdout(), Print("\x1b[5n"));
    }
    // Eat, the terminal inside Emacs, draws sixel but cannot say how many
    // pixels a cell is, so it is left with half-blocks. Emacs says instead.
    if picker.protocol_type() == ProtocolType::Halfblocks
        && let Some(cell) = eat_cell_size()
    {
        #[allow(deprecated)]
        let mut eat = Picker::from_fontsize(cell);
        eat.set_protocol_type(ProtocolType::Sixel);
        picker = eat;
    }
    // Pads an image rounded a pixel short of its cells, as the slide's own white.
    picker.set_background_color(Some([255, 255, 255, 255]));
    picker
}

/// The pixels of a cell in Eat, from `OHP_CELL_SIZE=WxH`, as Emacs sets it.
fn eat_cell_size() -> Option<FontSize> {
    if !std::env::var("TERM").is_ok_and(|t| t.starts_with("eat")) {
        return None;
    }
    let size = std::env::var("OHP_CELL_SIZE").ok()?;
    let (w, h) = size.split_once('x')?;
    let (w, h) = (w.trim().parse().ok()?, h.trim().parse().ok()?);
    (w > 0 && h > 0).then(|| FontSize::new(w, h))
}

fn over_ssh() -> bool {
    ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"]
        .iter()
        .any(|v| std::env::var_os(v).is_some())
}

/// A slide hayro cannot render or read aloud shows as failed; its panic
/// must not reach ratatui's hook, which would tear down the terminal.
fn quiet_render_panics() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        if !matches!(
            thread.name(),
            Some(render::THREAD | speak::THREAD | speak::VOICES_THREAD | summary::THREAD)
        ) {
            // ratatui's hook, which this one wraps, restores all but this.
            let _ = execute!(std::io::stdout(), DisableMouseCapture);
            prev(info);
        }
    }));
}

/// Set while the app reads keys, where crossterm is stuck for ever once the
/// terminal hangs up: only then is ohp ended a while after a hang-up.
pub static READING_KEYS: AtomicBool = AtomicBool::new(false);

/// `READING_KEYS` set until it is dropped.
struct ReadingKeys;

impl ReadingKeys {
    fn start() -> Self {
        READING_KEYS.store(true, Ordering::SeqCst);
        ReadingKeys
    }
}

impl Drop for ReadingKeys {
    fn drop(&mut self) {
        READING_KEYS.store(false, Ordering::SeqCst);
    }
}

#[derive(Clone, Copy, PartialEq)]
enum View {
    Present,
    Grid,
}

/// The summary shown beside the slides.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Panel {
    /// The slide on screen's.
    Slide,
    /// The whole file's.
    Whole,
}

/// Where the grid's slots go.
struct Grid {
    cols: usize,
    /// Rows on screen.
    rows: usize,
    slot: Size,
    /// Top-left of the first slot; the grid is centred across the screen.
    origin: (u16, u16),
}

#[derive(PartialEq)]
enum Then {
    /// The file changed again while it was being read: read it once more.
    Reload,
    /// Files picked while another was being read.
    Open(Vec<PathBuf>),
    /// Files asked to be read in place of the deck's, as it is read, and
    /// what to say once they are: the last asked.
    Read(Vec<PathBuf>, Option<String>),
}

/// What a read under way was asked with: its files as they were just
/// before it, so what changes after is read again, and what to say once
/// it is done, in place of `reloaded`.
struct Asked {
    stamps: Vec<Stamp>,
    said: Option<String>,
}

struct App {
    deck: Deck,
    picker: Picker,
    renderer: Renderer,
    workers: usize,
    view: View,
    look: Look,
    cur: usize,
    /// First grid row on screen.
    top: usize,
    /// Grid columns chosen with +/-; `None` fits the grid to the screen.
    grid_cols: Option<u16>,
    /// How the presented slide is zoomed, kept from slide to slide.
    zoom: Zoom,
    /// The slide last drawn when presenting, shown while its replacement
    /// at a new zoom renders.
    shown: Cell<Option<Job>>,
    /// The presented slide on screen, and the cells it is drawn in.
    drawn: Cell<Option<(Job, Rect)>>,
    /// The column beside the slides, as last drawn: the summary, the
    /// annotate palette or the print form.
    side: Cell<Rect>,
    /// Where a drag panning the zoomed slide began, and the zoom then.
    panning: Option<(ratatui::layout::Position, Zoom)>,
    /// Where slides are drawn: the screen above the status line, but for
    /// the summary's column.
    main: Rect,
    /// Finished slides, `None` where one could not be rendered.
    slides: HashMap<Job, Option<Slide>>,
    /// Slides of the deck before the last reload, shown until replaced.
    stale: HashMap<Job, Option<Slide>>,
    /// Jobs queued or being done.
    requested: HashSet<Job>,
    /// The deck's files as last loaded.
    stamp: Vec<Stamp>,
    /// A change seen on disk, waiting for the files to stop changing.
    settling: Option<Vec<Stamp>>,
    checked: Instant,
    /// The deck being read again, off this thread, and that thread.
    loading: Option<(Receiver<anyhow::Result<Deck>>, JoinHandle<()>)>,
    /// The files being read in place of these, if any are.
    opening: Option<Vec<PathBuf>>,
    /// What to read once the read under way is done.
    then: Option<Then>,
    /// What the read under way was asked with.
    asked: Option<Asked>,
    /// The PDF being signed, while it is.
    #[cfg(feature = "sign")]
    signing: Option<Box<signing::Signing>>,
    /// A search under way, as `ctrl-s` begins it.
    search: Option<search::Search>,
    /// The last query searched for, searched again by `ctrl-s` on none.
    last_query: String,
    /// The slides' text, as searching reads it, until the deck is read
    /// again.
    texts: Option<Vec<crate::text::Searchable>>,
    /// The slides' text being read for searching, off the screen's thread.
    reading: Option<Receiver<Vec<crate::text::Searchable>>>,
    /// What was sent to the printer, each being taken by `lp`.
    sending: Vec<Receiver<String>>,
    /// What is picked to print, while `P` is picking it.
    printing: Option<printing::Printing>,
    /// How files picked with Ctrl-O are read.
    local: Options,
    /// The list another file is picked from, open over the slides.
    browser: Option<Browser>,
    /// Set when a signal asks ohp to stop.
    signaled: Arc<AtomicUsize>,
    /// Shown in the status line until the next key press.
    notice: Option<String>,
    links: Vec<Link>,
    /// What is wrong with `links`, shown in the status line until it is not.
    trouble: Option<String>,
    speaker: Speaker,
    /// The sentence being read aloud, lit on the slide, as last seen: the
    /// speaker moves on under the screen, and a frame's jobs must agree.
    lit: Option<usize>,
    /// The voice picker, its list open over the slides when picking.
    #[cfg(feature = "speech")]
    voices: Voices,
    summarizer: Summarizer,
    /// The summary shown beside the slides, if one is; the slides' key
    /// sentences are marked while one is.
    panel: Option<Panel>,
    /// The model picker, its list open over the slides when picking.
    #[cfg(feature = "summary")]
    models: Models,
    dirty: bool,
    quit: bool,
}

impl App {
    fn new(
        deck: Deck,
        picker: Picker,
        renderer: Renderer,
        workers: usize,
        look: Look,
        signaled: Arc<AtomicUsize>,
    ) -> Self {
        App {
            stamp: deck.stamps(),
            local: deck.options.clone(),
            deck,
            picker,
            renderer,
            workers,
            view: View::Present,
            look,
            cur: 0,
            top: 0,
            grid_cols: None,
            zoom: Zoom::FIT,
            shown: Cell::new(None),
            drawn: Cell::new(None),
            side: Cell::new(Rect::default()),
            panning: None,
            main: Rect::default(),
            slides: HashMap::new(),
            stale: HashMap::new(),
            requested: HashSet::new(),
            settling: None,
            checked: Instant::now(),
            loading: None,
            opening: None,
            then: None,
            asked: None,
            #[cfg(feature = "sign")]
            signing: None,
            printing: None,
            sending: Vec::new(),
            search: None,
            last_query: String::new(),
            texts: None,
            reading: None,
            browser: None,
            signaled,
            notice: None,
            links: Vec::new(),
            trouble: None,
            speaker: Speaker::new(None, None),
            lit: None,
            #[cfg(feature = "speech")]
            voices: Voices::default(),
            summarizer: Summarizer::new(None),
            panel: None,
            #[cfg(feature = "summary")]
            models: Models::default(),
            dirty: true,
            quit: false,
        }
    }

    fn run(&mut self, terminal: &mut DefaultTerminal) -> anyhow::Result<()> {
        let _reading = ReadingKeys::start();
        // The screen, and whether a summary is shown beside the slides.
        let mut laid = (Rect::default(), false);
        while !self.quit {
            let size = terminal.size()?;
            let now = (Rect::new(0, 0, size.width, size.height), self.side_open());
            if now != laid {
                laid = now;
                self.main = split(now.0, now.1).0;
                self.forget();
            }
            if self.dirty {
                self.dirty = false;
                self.scroll();
                self.clamp_pan();
                self.schedule();
                terminal.draw(|f| self.draw(f))?;
            }
            if event::poll(Duration::from_millis(30))? {
                self.handle(event::read()?);
                while !self.quit && event::poll(Duration::ZERO)? {
                    self.handle(event::read()?);
                }
            }
            while let Ok(done) = self.renderer.done.try_recv() {
                self.receive(done);
            }
            self.settle_wheel(false);
            self.poll_printing();
            self.read_texts();
            self.watch();
            self.loaded();
            self.read_along();
            #[cfg(feature = "speech")]
            self.poll_voices();
            let slide = self.panel == Some(Panel::Slide);
            self.summarizer.want(slide.then_some(self.cur));
            self.dirty |= self.summarizer.changed();
            if self.signaled.load(Ordering::Relaxed) != 0 {
                self.quit = true;
            }
        }
        Ok(())
    }

    /// Drop every slide: they were rendered for boxes the screen no longer has.
    fn forget(&mut self) {
        self.renderer.clear();
        self.requested.clear();
        self.slides.clear();
        self.stale.clear();
        self.dirty = true;
    }

    fn receive(&mut self, done: Done) {
        // Unrequested: rendered for a screen size since abandoned.
        if !self.requested.remove(&done.job) {
            return;
        }
        self.slides.insert(done.job, done.slide);
        if self.requested.is_empty() {
            self.stale.clear();
        }
        self.dirty = true;
    }

    /// Reload the PDF once a change to it has settled: LaTeX writes it over
    /// a while, and a half-written file does not parse.
    fn watch(&mut self) {
        if self.checked.elapsed() < WATCH_EVERY {
            return;
        }
        self.checked = Instant::now();
        let trouble = self.links.iter().find_map(Link::trouble);
        if trouble != self.trouble {
            self.trouble = trouble;
            self.dirty = true;
        }
        // What is being read is looked at once it is: it knows its files
        // as they were.
        if self.loading.is_some() {
            return;
        }
        let now = self.deck.stamps();
        if now == self.stamp {
            self.settling = None;
        } else if self.settling.as_ref() == Some(&now) && self.deck.finished() {
            self.settling = None;
            self.stamp = now;
            self.reload();
        } else if self.settling.as_ref() != Some(&now) {
            self.settling = Some(now);
            let what = if render::pdf(&self.deck.paths) {
                "PDF"
            } else {
                "file"
            };
            self.notice = Some(format!("{what} changing…"));
            self.dirty = true;
        }
    }

    /// Read the deck again on a thread of its own.
    fn reload(&mut self) {
        if self.loading.is_some() {
            // A file picked replaces this one: no need to read this one again.
            self.then.get_or_insert(Then::Reload);
            return;
        }
        self.read(self.deck.paths.clone(), self.deck.options.clone(), None);
    }

    /// Files just added, then the ones the deck already had, so the new
    /// one is what comes up.
    fn added_first(&self, paths: Vec<PathBuf>) -> Vec<PathBuf> {
        let had: HashSet<_> = self
            .deck
            .paths
            .iter()
            .filter_map(|p| browse::as_listed(p))
            .collect();
        let (mut fresh, kept): (Vec<_>, Vec<_>) = paths
            .into_iter()
            .partition(|path| browse::as_listed(path).is_none_or(|listed| !had.contains(&listed)));
        fresh.extend(kept);
        fresh
    }

    /// Read `paths` on a thread of its own, to present in place of the deck.
    fn open(&mut self, paths: Vec<PathBuf>) {
        // What is annotated is of the file shown: another, it is done.
        self.quit_signing();
        if self.loading.is_some() {
            self.then = Some(Then::Open(paths));
            return;
        }
        self.opening = Some(paths.clone());
        self.read(paths, self.local.clone(), None);
    }

    /// Read `paths` on a thread of its own, to show in place of the deck,
    /// `said` once they are; or, while a read is under way, once it is done.
    fn read(&mut self, paths: Vec<PathBuf>, options: Options, said: Option<String>) {
        if self.loading.is_some() {
            self.then = Some(Then::Read(paths, said));
            return;
        }
        let stamps = paths.iter().map(|p| render::stamp(p)).collect();
        self.asked = Some(Asked { stamps, said });
        let (tx, rx) = std::sync::mpsc::channel();
        // Nothing to do if the app is gone.
        let reader = std::thread::spawn(move || {
            let _ = tx.send(Deck::open_with(&paths, options));
        });
        self.loading = Some((rx, reader));
        self.dirty = true;
    }

    /// Open the list to pick another file from, in the directory of the
    /// file on screen, with a deck of several files marked, so enter adds
    /// the file picked.
    fn browse(&mut self) {
        let paths = &self.deck.paths;
        // A file on another machine is copied to a temporary directory.
        let here = self.links.is_empty();
        let current = &paths[self.deck.source(self.cur)];
        let dir = current
            .parent()
            .filter(|dir| here && !dir.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        match Browser::new(dir) {
            Ok(mut browser) => {
                if here {
                    if let Some(name) = current.file_name() {
                        browser.select(&name.to_string_lossy());
                    }
                    if paths.len() > 1 {
                        browser.mark_all(paths);
                    }
                }
                self.browser = Some(browser);
            }
            Err(e) => self.notice = Some(format!("cannot list {}: {e}", dir.display())),
        }
    }

    /// Show the deck read again, once it is.
    fn loaded(&mut self) {
        let Some((rx, _)) = &self.loading else {
            return;
        };
        let fresh = match rx.try_recv() {
            Ok(fresh) => fresh,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(anyhow::anyhow!("the reader stopped")),
        };
        self.loading = None;
        let fresh = fresh.and_then(|deck| {
            let renderer = Renderer::spawn(&deck, &self.picker, self.workers)?;
            Ok((deck, renderer))
        });
        let opening = self.opening.take();
        let asked = self.asked.take();
        match fresh {
            Ok((deck, renderer)) => {
                if opening.is_some() {
                    self.speaker.stop();
                    self.stamp = deck.stamps();
                    self.settling = None;
                    // Slides of another file: not to be shown for this one.
                    self.slides.clear();
                    self.shown.set(None);
                    (self.cur, self.top) = (0, 0);
                    self.zoom = Zoom::FIT;
                    self.view = View::Present;
                    self.links.clear();
                    self.trouble = None;
                    // A reload asked for was of the file this replaces.
                    self.then = self.then.take().filter(|then| *then != Then::Reload);
                    self.notice = deck.note.clone();
                } else {
                    self.notice = Some(
                        match (asked.as_ref().and_then(|a| a.said.clone()), &deck.note) {
                            (Some(said), _) => said,
                            (None, Some(note)) => format!("reloaded; {note}"),
                            (None, None) => "reloaded".into(),
                        },
                    );
                }
                self.cur = self.cur.min(deck.pages - 1);
                self.summarizer.set_deck(deck.data.clone(), deck.pages);
                self.deck = deck;
                self.renderer = renderer;
                self.searched_anew();
                // Its files as they were read: what changed since is read again.
                if let Some(asked) = asked {
                    self.stamp = asked.stamps;
                    self.settling = None;
                }
                self.requested.clear();
                self.stale = std::mem::take(&mut self.slides);
                // The slide being read is read again from what it says now.
                if self.speaker.reading() {
                    self.speak();
                }
            }
            Err(e) => {
                self.notice = Some(match opening {
                    Some(paths) => format!("cannot open {}: {e:#}", render::shown(&paths)),
                    None => format!("reload failed: {e:#}"),
                });
            }
        }
        self.dirty = true;
        match self.then.take() {
            Some(Then::Open(path)) => self.open(path),
            Some(Then::Reload) => self.reload(),
            Some(Then::Read(paths, said)) => self.read(paths, self.deck.options.clone(), said),
            None => {}
        }
    }

    /// Read the current slide aloud, in place of any being read.
    fn speak(&mut self) {
        self.speak_from(0);
    }

    /// Read the current slide aloud from its sentence `from` on, in place
    /// of any being read.
    fn speak_from(&mut self, from: usize) {
        if let Err(e) = self.speaker.read(self.deck.data.clone(), self.cur, from) {
            self.notice = Some(e);
        }
        // The sentence lit was the last reading's, not one of this slide's.
        self.lit = None;
        self.dirty = true;
    }

    /// Keep reading aloud: the slide turned to, or the next once one is
    /// read; and light the sentence being read.
    fn read_along(&mut self) {
        let lit = self.speaker.sentence();
        if lit != self.lit {
            self.lit = lit;
            self.dirty = true;
        }
        let Some(page) = self.speaker.page() else {
            return;
        };
        if page != self.cur {
            self.speak();
            return;
        }
        match self.speaker.ended() {
            None => return,
            Some(Ok(())) if self.cur + 1 < self.deck.pages => {
                self.cur += 1;
                self.speak();
            }
            Some(Ok(())) => self.notice = Some("read to the end".into()),
            Some(Err(e)) => self.notice = Some(e),
        }
        self.dirty = true;
    }

    /// Open the list to pick the voice slides are read in from.
    #[cfg(feature = "speech")]
    fn choose_voice(&mut self) {
        self.voices.open(self.speaker.voice());
    }

    #[cfg(not(feature = "speech"))]
    fn choose_voice(&mut self) {
        self.notice = Some(speak::NO_SPEECH.into());
    }

    /// Do what picking voices calls for.
    #[cfg(feature = "speech")]
    fn voiced(&mut self, event: voices::Event) {
        match event {
            voices::Event::Use(name, command) => {
                self.speaker.set_voice(command);
                self.notice = Some(format!("reading in {name}"));
                // The slide being read is read again in it.
                if self.speaker.reading() {
                    self.speak();
                }
            }
            voices::Event::Notice(notice) => self.notice = Some(notice),
        }
    }

    /// List Piper's voices once they are fetched, and read in a voice
    /// downloaded once it is.
    #[cfg(feature = "speech")]
    fn poll_voices(&mut self) {
        let (redraw, event) = self.voices.poll();
        self.dirty |= redraw;
        if let Some(event) = event {
            self.voiced(event);
        }
    }

    /// Show `panel`'s summary beside the slides, or stop showing it if it
    /// is shown. The whole file is summed up while a summary is shown,
    /// once its own was asked for.
    fn summarize(&mut self, panel: Panel) {
        if !cfg!(feature = "summary") {
            self.notice = Some(summary::NO_SUMMARY.into());
            return;
        }
        if self.panel == Some(panel) {
            self.panel = None;
            self.summarizer.want_whole(false);
            return;
        }
        if self.panel.is_none() {
            self.summarizer.retry();
        }
        self.panel = Some(panel);
        if panel == Panel::Whole {
            self.summarizer.want_whole(true);
        }
    }

    /// Open the list to pick the model that sums up from.
    #[cfg(feature = "summary")]
    fn choose_model(&mut self) {
        self.models.open(&self.summarizer.weights());
    }

    #[cfg(not(feature = "summary"))]
    fn choose_model(&mut self) {
        self.notice = Some(summary::NO_SUMMARY.into());
    }

    /// Sum up with the model in `weights` from now on, showing the slide's
    /// summary if none is shown.
    #[cfg(feature = "summary")]
    fn summarize_with(&mut self, weights: summary::Weights) {
        self.notice = Some(format!("summing up with {}", weights.name()));
        self.summarizer.set_weights(weights);
        self.summarizer.retry();
        self.panel.get_or_insert(Panel::Slide);
    }

    /// The key sentences marked on `page`, while a summary is shown: its
    /// own summary's, or the whole file's.
    fn marked(&self, page: usize) -> Picked {
        marked(&self.summarizer, self.panel, page)
    }

    /// Stop a reload under way and wait for it, so a knitr run it started
    /// does not outlive ohp, nor leave its files.
    fn stop(&mut self) {
        if let Some((_, reader)) = self.loading.take() {
            self.deck.options.stop.store(true, Ordering::Relaxed);
            let _ = reader.join();
        }
    }

    // ── input ───────────────────────────────────────────────────────────

    // ── layout ──────────────────────────────────────────────────────────

    /// Most grid columns the screen and the deck allow.
    fn widest(&self) -> u16 {
        (self.main.width / MIN_SLOT_WIDTH)
            .min(u16::try_from(self.deck.pages).unwrap_or(u16::MAX))
            .max(1)
    }

    fn grid(&self) -> Grid {
        if let Some(cols) = self.grid_cols {
            return self.grid_with(cols.clamp(1, self.widest()));
        }
        let want = self.deck.pages.min(GRID_TARGET);
        let widest = GRID_MAX_COLS.min(self.widest());
        let mut grid = self.grid_with(1);
        for cols in 2..=widest {
            if grid.cols * grid.rows >= want {
                break;
            }
            grid = self.grid_with(cols);
        }
        grid
    }

    /// `cols` slots across, each as tall as a slide its width needs.
    fn grid_with(&self, cols: u16) -> Grid {
        let area = self.main;
        let font = self.picker.font_size();
        let (pw, ph) = self.deck.sizes[0];
        let slot_w = (area.width / cols).max(3);
        let inner_px = f32::from(slot_w - 2) * f32::from(font.width);
        let inner_h = (inner_px * ph / pw / f32::from(font.height)).ceil() as u16;
        let slot_h = (inner_h + 2).clamp(3, area.height.max(3));
        let spare = area.width.saturating_sub(slot_w * cols);
        Grid {
            cols: cols.into(),
            rows: usize::from(area.height / slot_h).max(1),
            slot: Size::new(slot_w, slot_h),
            origin: (area.x + spare / 2, area.y),
        }
    }

    /// The on-screen slots: slide index and where it goes.
    fn slots(&self, grid: &Grid) -> impl Iterator<Item = (usize, Rect)> {
        let first = self.top * grid.cols;
        let last = (first + grid.rows * grid.cols).min(self.deck.pages);
        let (x0, y0) = grid.origin;
        let slot = grid.slot;
        (first..last).map(move |i| {
            let (row, col) = ((i - first) / grid.cols, i % grid.cols);
            let x = x0 + col as u16 * slot.width;
            let y = y0 + row as u16 * slot.height;
            (i, Rect::new(x, y, slot.width, slot.height))
        })
    }

    /// Keep the current slide's grid row on screen, and no rows past the
    /// last below it.
    fn scroll(&mut self) {
        let grid = self.grid();
        let row = self.cur / grid.cols;
        let rows = self.deck.pages.div_ceil(grid.cols);
        self.top = self.top.min(rows.saturating_sub(grid.rows));
        if row < self.top {
            self.top = row;
        } else if row >= self.top + grid.rows {
            self.top = row + 1 - grid.rows;
        }
    }

    /// The slides on screen, the current one first.
    fn visible(&self) -> Vec<Key> {
        if let Some(preview) = self.preview() {
            return vec![preview.key];
        }
        match self.view {
            View::Present => vec![self.presented()],
            View::Grid => {
                let grid = self.grid();
                let thumb = |(i, r): (usize, Rect)| Key::new(i, Block::bordered().inner(r));
                let cur = self.slots(&grid).find(|&(i, _)| i == self.cur).map(thumb);
                cur.into_iter()
                    .chain(self.slots(&grid).map(thumb))
                    .collect()
            }
        }
    }

    /// The current slide as presented.
    fn presented(&self) -> Key {
        Key {
            zoom: self.zoom,
            ..Key::new(self.cur, self.main)
        }
    }

    /// The job that draws `key`: the presented slide being read aloud has
    /// the sentence being read lit, and every slide its key sentences
    /// marked, while summaries are shown.
    fn job(&self, key: Key) -> Job {
        let reading = self.view == View::Present
            && key == self.presented()
            && self.speaker.page() == Some(key.page);
        Job {
            key,
            look: self.look,
            lit: self.lit.filter(|_| reading),
            marked: self.marked(key.page),
            found: self.found(key.page),
        }
    }

    /// Queue what the view shows, then what it may show next: the slides
    /// either side fitted to the screen, and the presented slide from the
    /// grid. Kept are those, the slides either side fitted and zoomed, the
    /// slide last presented until its replacement arrives, and the grid's
    /// slides at its zoom.
    fn schedule(&mut self) {
        let area = self.main;
        let mut wanted = self.visible();
        let fitted = [self.cur + 1, self.cur.wrapping_sub(1), self.cur + 2];
        wanted.extend(fitted.map(|page| Key::new(page, area)));
        if self.view == View::Grid {
            wanted.push(self.presented());
        }
        wanted.retain(|k| k.page < self.deck.pages);

        for job in self.renderer.clear() {
            self.requested.remove(&job);
        }
        let mut jobs = Vec::new();
        let wanted: Vec<Job> = wanted.into_iter().map(|key| self.job(key)).collect();
        for job in wanted {
            if !self.slides.contains_key(&job) && self.requested.insert(job) {
                jobs.push(job);
            }
        }
        self.renderer.push(jobs);

        let previewed = self.preview().map(|p| p.key);
        // What a search finds on each slide now: a slide lit otherwise is
        // not kept, as each letter typed and each step lights anew.
        let found: Vec<Option<render::Found>> =
            (0..self.deck.pages).map(|p| self.found(p)).collect();
        let finding =
            |job: &Job| job.found.is_none() || found.get(job.key.page) == Some(&job.found);
        let (cur, zoom, shown, lit) = (self.cur, self.zoom, self.shown.get(), self.lit);
        let full = area.as_size();
        let grid = self.grid();
        let slot = Rect::new(0, 0, grid.slot.width, grid.slot.height);
        let thumb = Block::bordered().inner(slot).as_size();
        let (summarizer, panel) = (&self.summarizer, self.panel);
        // Unmarked, or marked as the slide's summary has it now.
        let marking = |job: &Job| {
            job.marked.is_empty() || job.marked == marked(summarizer, panel, job.key.page)
        };
        // Both looks are kept, so `t` back is immediate.
        self.slides.retain(|job, _| {
            let key = job.key;
            if Some(key) == previewed {
                true
            } else if key.size() == full {
                let kept = key.zoom == zoom || key.zoom == Zoom::FIT || Some(*job) == shown;
                let lighting = job.lit.is_none() || job.lit == lit || Some(*job) == shown;
                let marking = marking(job) || Some(*job) == shown;
                let finding = finding(job) || Some(*job) == shown;
                key.page.abs_diff(cur) <= KEEP && kept && lighting && marking && finding
            } else {
                key.size() == thumb && marking(job) && finding(job)
            }
        });
    }

    // ── drawing ─────────────────────────────────────────────────────────
}

/// What `work` returns, worked out on a thread of its own, off the
/// screen's: asked for with `try_recv` as the screen is redrawn.
fn background<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Receiver<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    rx
}

/// Built without signing, `A` says so, and nothing else is signing's.
#[cfg(not(feature = "sign"))]
impl App {
    fn sign(&mut self) {
        self.notice = Some("ohp was built without annotating: --features sign".into());
    }

    fn sign_key(&mut self, _: ratatui::crossterm::event::KeyEvent) -> bool {
        false
    }

    fn signing(&self) -> bool {
        false
    }

    fn sign_click(&mut self, _: ratatui::layout::Position) {}

    fn settle_wheel(&mut self, _: bool) {}

    fn annotated(&self) -> Option<Vec<u8>> {
        None
    }

    fn sign_drag(&mut self, _: ratatui::layout::Position) {}

    fn sign_release(&mut self) {}

    fn sign_wheel(&mut self, _: ratatui::layout::Position, _: bool) -> bool {
        false
    }

    fn dragged(&self) -> Option<(usize, [f32; 4])> {
        None
    }

    fn draw_palette(&self, _: &mut ratatui::Frame, _: Rect) {}

    fn sign_status(&self) -> Option<(String, &'static str)> {
        None
    }

    fn picking_signature(&self) -> bool {
        false
    }

    fn picked_signature(&mut self, _: Option<Vec<PathBuf>>) {}

    fn quit_signing(&mut self) {}
}

/// However ohp ends, even by a panic, a reload under way stops with it.
impl Drop for App {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
#[path = "tests/app.rs"]
mod tests;
