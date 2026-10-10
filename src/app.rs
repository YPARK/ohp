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

use crate::browse::{self, Browser};
use crate::remote::Link;
use crate::render::{self, Deck, Done, Job, Key, Look, Options, Renderer, Slide, Stamp, Zoom};
use crate::speak::{self, Speaker};
use crate::text;
#[cfg(feature = "speech")]
use crate::voices::{self, How, Voice};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect, Size};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};
use ratatui::{DefaultTerminal, Frame};
use ratatui_image::Image;
use ratatui_image::picker::cap_parser::{Parser, QueryStdioOptions};
use ratatui_image::picker::{Picker, ProtocolType};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

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
/// `link` is the connection the slides come over, for slides on another
/// machine.
/// `local` is how files picked with Ctrl-O, which are here, are read.
/// `signaled` is set when a signal asks ohp to stop.
/// `speaker` reads the slides aloud.
pub fn run(
    deck: Deck,
    text: bool,
    link: Option<Link>,
    local: Options,
    speaker: Speaker,
    signaled: Arc<AtomicUsize>,
) -> anyhow::Result<()> {
    let mut terminal = ratatui::init();
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
        app.link = link;
        app.local = local;
        app.speaker = speaker;
        app.run(&mut terminal)?;
        app.clear_images()
    })();
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
    let mut picker =
        Picker::from_query_stdio_with_options(options).unwrap_or_else(|_| Picker::halfblocks());
    // Pads an image rounded a pixel short of its cells, as the slide's own white.
    picker.set_background_color(Some([255, 255, 255, 255]));
    picker
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
        if !matches!(thread.name(), Some(render::THREAD | speak::THREAD)) {
            prev(info);
        }
    }));
}

#[derive(Clone, Copy, PartialEq)]
enum View {
    Present,
    Grid,
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
    /// A file picked while another was being read.
    Open(PathBuf),
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
    /// Where slides are drawn: the screen above the status line.
    main: Rect,
    /// Finished slides, `None` where one could not be rendered.
    slides: HashMap<Job, Option<Slide>>,
    /// Slides of the deck before the last reload, shown until replaced.
    stale: HashMap<Job, Option<Slide>>,
    /// Jobs queued or being done.
    requested: HashSet<Job>,
    /// The PDF as last loaded.
    stamp: Stamp,
    /// A change seen on disk, waiting for the file to stop changing.
    settling: Option<Stamp>,
    checked: Instant,
    /// The deck being read again, off this thread, and that thread.
    loading: Option<(Receiver<anyhow::Result<Deck>>, JoinHandle<()>)>,
    /// The file being read in place of this one, if one is.
    opening: Option<PathBuf>,
    /// What to read once the read under way is done.
    then: Option<Then>,
    /// How files picked with Ctrl-O are read.
    local: Options,
    /// The list another file is picked from, open over the slides.
    browser: Option<Browser>,
    /// Set when a signal asks ohp to stop.
    signaled: Arc<AtomicUsize>,
    /// Shown in the status line until the next key press.
    notice: Option<String>,
    link: Option<Link>,
    /// What is wrong with `link`, shown in the status line until it is not.
    trouble: Option<String>,
    speaker: Speaker,
    /// The sentence being read aloud, lit on the slide, as last seen: the
    /// speaker moves on under the screen, and a frame's jobs must agree.
    lit: Option<usize>,
    /// The list a voice is picked from, open over the slides.
    #[cfg(feature = "speech")]
    voices: Option<voices::Menu>,
    /// A voice being downloaded, and the percent of it last shown.
    #[cfg(feature = "speech")]
    download: Option<(voices::Download, u8)>,
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
            stamp: render::stamp(&deck.path),
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
            main: Rect::default(),
            slides: HashMap::new(),
            stale: HashMap::new(),
            requested: HashSet::new(),
            settling: None,
            checked: Instant::now(),
            loading: None,
            opening: None,
            then: None,
            browser: None,
            signaled,
            notice: None,
            link: None,
            trouble: None,
            speaker: Speaker::new(None, None),
            lit: None,
            #[cfg(feature = "speech")]
            voices: None,
            #[cfg(feature = "speech")]
            download: None,
            dirty: true,
            quit: false,
        }
    }

    fn run(&mut self, terminal: &mut DefaultTerminal) -> anyhow::Result<()> {
        let mut screen = Rect::default();
        while !self.quit {
            let size = terminal.size()?;
            let now = Rect::new(0, 0, size.width, size.height);
            if now != screen {
                screen = now;
                self.main = split(screen).0;
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
            self.watch();
            self.loaded();
            self.read_along();
            #[cfg(feature = "speech")]
            self.downloading();
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
        let trouble = self.link.as_ref().and_then(Link::trouble);
        if trouble != self.trouble {
            self.trouble = trouble;
            self.dirty = true;
        }
        let now = render::stamp(&self.deck.path);
        if now == self.stamp {
            self.settling = None;
        } else if self.settling == Some(now) && render::finished(&self.deck.path) {
            self.settling = None;
            self.stamp = now;
            self.reload();
        } else if self.settling != Some(now) {
            self.settling = Some(now);
            let what = if render::markdown(&self.deck.path) {
                "file"
            } else {
                "PDF"
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
        self.read(self.deck.path.clone(), self.deck.options.clone());
    }

    /// Read `path` on a thread of its own, to present in place of the deck.
    fn open(&mut self, path: PathBuf) {
        if self.loading.is_some() {
            self.then = Some(Then::Open(path));
            return;
        }
        self.opening = Some(path.clone());
        self.read(path, self.local.clone());
    }

    fn read(&mut self, path: PathBuf, options: Options) {
        let (tx, rx) = std::sync::mpsc::channel();
        // Nothing to do if the app is gone.
        let reader = std::thread::spawn(move || {
            let _ = tx.send(Deck::open_with(&path, options));
        });
        self.loading = Some((rx, reader));
        self.dirty = true;
    }

    /// Open the list to pick another file from, in the deck's directory.
    fn browse(&mut self) {
        let path = &self.deck.path;
        // A file on another machine is copied to a temporary directory.
        let here = self.link.is_none();
        let dir = path
            .parent()
            .filter(|dir| here && !dir.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        match Browser::new(dir) {
            Ok(mut browser) => {
                if here && let Some(name) = path.file_name() {
                    browser.select(&name.to_string_lossy());
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
        match fresh {
            Ok((deck, renderer)) => {
                if opening.is_some() {
                    self.speaker.stop();
                    self.stamp = render::stamp(&deck.path);
                    self.settling = None;
                    // Slides of another file: not to be shown for this one.
                    self.slides.clear();
                    self.shown.set(None);
                    (self.cur, self.top) = (0, 0);
                    self.zoom = Zoom::FIT;
                    self.view = View::Present;
                    self.link = None;
                    self.trouble = None;
                    // A reload asked for was of the file this replaces.
                    self.then = self.then.take().filter(|then| *then != Then::Reload);
                    self.notice = deck.note.clone();
                } else {
                    self.notice = Some(match &deck.note {
                        Some(note) => format!("reloaded; {note}"),
                        None => "reloaded".into(),
                    });
                }
                self.cur = self.cur.min(deck.pages - 1);
                self.deck = deck;
                self.renderer = renderer;
                self.requested.clear();
                self.stale = std::mem::take(&mut self.slides);
                // The slide being read is read again from what it says now.
                if self.speaker.reading() {
                    self.speak();
                }
            }
            Err(e) => {
                self.notice = Some(match opening {
                    Some(path) => format!("cannot open {}: {e:#}", path.display()),
                    None => format!("reload failed: {e:#}"),
                });
            }
        }
        self.dirty = true;
        match self.then.take() {
            Some(Then::Open(path)) => self.open(path),
            Some(Then::Reload) => self.reload(),
            None => {}
        }
    }

    /// Read the current slide aloud, in place of any being read.
    fn speak(&mut self) {
        if let Err(e) = self.speaker.read(self.deck.data.clone(), self.cur) {
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
        let dir = speak::piper_dir();
        let piper = speak::installed("piper");
        let current = self.speaker.voice();
        let at_hand = voices::at_hand(current, dir.as_deref(), piper, &speak::others());
        let catalog = if !piper {
            Err("install Piper to download its voices".into())
        } else if !speak::installed("curl") {
            Err("install curl to download Piper's voices".into())
        } else if dir.is_none() {
            Err("no place for Piper's voices: HOME is not set".into())
        } else {
            Ok(voices::Catalog::fetch(voices::PIPER_VOICES))
        };
        self.voices = Some(voices::Menu::new(at_hand, current, catalog));
    }

    #[cfg(not(feature = "speech"))]
    fn choose_voice(&mut self) {
        self.notice = Some(speak::NO_SPEECH.into());
    }

    /// Read in `voice`, once it is downloaded if it must be.
    #[cfg(feature = "speech")]
    fn use_voice(&mut self, voice: Voice) {
        match voice.how {
            How::Ready(command) => self.read_in(&voice.name, command),
            How::Fetch(_) if self.download.is_some() => {
                self.notice = Some("one voice is downloaded at a time".into());
            }
            How::Fetch(remote) => match speak::piper_dir() {
                Some(dir) => {
                    let download = voices::Download::start(&remote, voices::PIPER_VOICES, &dir);
                    self.download = Some((download, 0));
                }
                None => self.notice = Some("no place for Piper's voices: HOME is not set".into()),
            },
        }
    }

    /// Read in the voice `name`, which speaks with `command`; the slide
    /// being read is read again in it.
    #[cfg(feature = "speech")]
    fn read_in(&mut self, name: &str, command: String) {
        self.speaker.set_voice(command);
        self.notice = Some(format!("reading in {name}"));
        if self.speaker.reading() {
            self.speak();
        }
    }

    /// List Piper's voices once they are fetched, and read in a voice
    /// downloaded once it is.
    #[cfg(feature = "speech")]
    fn downloading(&mut self) {
        if let Some(menu) = &mut self.voices
            && menu.poll()
        {
            self.dirty = true;
        }
        let Some((download, shown)) = &mut self.download else {
            return;
        };
        let Some(ended) = download.ended() else {
            let now = (download.progress() * 100.) as u8;
            if now != *shown {
                *shown = now;
                self.dirty = true;
            }
            return;
        };
        let name = download.name.clone();
        self.download = None;
        match ended {
            Ok(command) => self.read_in(&name, command),
            Err(e) => self.notice = Some(format!("cannot download {name}: {e}")),
        }
        self.dirty = true;
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

    fn handle(&mut self, ev: Event) {
        match ev {
            Event::Key(key) if key.kind != KeyEventKind::Release => self.key(key),
            Event::Resize(..) => self.dirty = true,
            _ => {}
        }
    }

    fn key(&mut self, key: KeyEvent) {
        self.notice = None;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if let Some(browser) = &mut self.browser {
            match browser.key(key) {
                browse::Outcome::Stay => {}
                browse::Outcome::Open(path) => {
                    self.browser = None;
                    self.open(path);
                }
                browse::Outcome::Quit => self.browser = None,
            }
            self.dirty = true;
            return;
        }
        #[cfg(feature = "speech")]
        if let Some(menu) = &mut self.voices {
            match menu.key(key) {
                voices::Outcome::Stay => {}
                voices::Outcome::Pick(voice) => {
                    self.voices = None;
                    self.use_voice(voice);
                }
                voices::Outcome::Quit => self.voices = None,
            }
            self.dirty = true;
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('o') if ctrl => self.browse(),
            KeyCode::Right if self.zoomed() => self.pan(1, 0),
            KeyCode::Left if self.zoomed() => self.pan(-1, 0),
            KeyCode::Down if self.zoomed() => self.pan(0, 1),
            KeyCode::Up if self.zoomed() => self.pan(0, -1),
            KeyCode::Char('n') | KeyCode::Right => self.step(1),
            KeyCode::Char('p') | KeyCode::Left => self.step(-1),
            KeyCode::Down => self.step(self.row_step()),
            KeyCode::Up => self.step(-self.row_step()),
            KeyCode::Char('g') | KeyCode::Tab => {
                self.view = match self.view {
                    View::Present => View::Grid,
                    View::Grid => View::Present,
                }
            }
            KeyCode::Char('t') => {
                self.look = match self.look {
                    Look::Image => Look::Text,
                    Look::Text => Look::Image,
                }
            }
            KeyCode::Enter | KeyCode::Esc if self.view == View::Grid => self.view = View::Present,
            KeyCode::Char('+' | '=') if self.view == View::Grid => self.zoom_grid(-1),
            KeyCode::Char('-' | '_') if self.view == View::Grid => self.zoom_grid(1),
            KeyCode::Char('+' | '=') => self.zoom_slide(1),
            KeyCode::Char('-' | '_') => self.zoom_slide(-1),
            KeyCode::Char('0') if self.view == View::Present => self.zoom = Zoom::FIT,
            // Even unchanged: a chunk may read files ohp does not watch.
            KeyCode::Char('r') => self.reload(),
            KeyCode::Char('s') if self.speaker.reading() => self.speaker.stop(),
            KeyCode::Char('s') => self.speak(),
            KeyCode::Char('v') => self.choose_voice(),
            _ => return,
        }
        self.dirty = true;
    }

    fn zoomed(&self) -> bool {
        self.view == View::Present && self.zoom != Zoom::FIT
    }

    /// Zoom the presented slide `by` steps, keeping the middle of the screen
    /// on the same part of it.
    fn zoom_slide(&mut self, by: isize) {
        let at = ZOOMS
            .iter()
            .position(|&z| z == self.zoom.percent)
            .unwrap_or(0);
        let percent = ZOOMS[at.saturating_add_signed(by).min(ZOOMS.len() - 1)];
        let k = f32::from(percent) / f32::from(self.zoom.percent);
        let keep = |at: u16, screen: u16| {
            let half = f32::from(screen) / 2.;
            ((f32::from(at) + half) * k - half).max(0.) as u16
        };
        self.zoom = Zoom {
            percent,
            x: keep(self.zoom.x, self.main.width),
            y: keep(self.zoom.y, self.main.height),
        };
        self.clamp_pan();
    }

    /// Move the zoomed slide a quarter of the screen across and down.
    fn pan(&mut self, across: i32, down: i32) {
        let by = |at: u16, screen: u16, steps: i32| {
            let step = i32::from((screen / 4).max(1));
            (i32::from(at) + steps * step).clamp(0, u16::MAX.into()) as u16
        };
        self.zoom.x = by(self.zoom.x, self.main.width, across);
        self.zoom.y = by(self.zoom.y, self.main.height, down);
        self.clamp_pan();
    }

    /// Keep the zoomed slide covering the screen where it is large enough to.
    fn clamp_pan(&mut self) {
        let font = self.picker.font_size();
        let (fw, fh) = (f32::from(font.width), f32::from(font.height));
        let (pw, ph) = self.deck.page_size;
        let (cols, rows) = (self.main.width, self.main.height);
        let fit = (f32::from(cols) * fw / pw).min(f32::from(rows) * fh / ph);
        let scale = fit * f32::from(self.zoom.percent) / 100.;
        let most = |len: f32, screen: u16| (len.round() as u16).saturating_sub(screen);
        self.zoom.x = self.zoom.x.min(most(pw * scale / fw, cols));
        self.zoom.y = self.zoom.y.min(most(ph * scale / fh, rows));
    }

    fn step(&mut self, by: isize) {
        let last = self.deck.pages - 1;
        self.cur = self.cur.saturating_add_signed(by).min(last);
    }

    /// Slides up/down moves by: a grid row, or one slide when presenting.
    fn row_step(&self) -> isize {
        match self.view {
            View::Present => 1,
            View::Grid => isize::try_from(self.grid().cols).unwrap_or(1),
        }
    }

    /// Change the grid by `by` columns: fewer columns, bigger slides.
    fn zoom_grid(&mut self, by: i32) {
        let cols = i32::try_from(self.grid().cols).unwrap_or(i32::MAX);
        let cols = (cols + by).clamp(1, self.widest().into());
        self.grid_cols = Some(cols as u16);
    }

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
        let (pw, ph) = self.deck.page_size;
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
    /// the sentence being read lit.
    fn job(&self, key: Key) -> Job {
        let reading = self.view == View::Present
            && key == self.presented()
            && self.speaker.page() == Some(key.page);
        Job {
            key,
            look: self.look,
            lit: self.lit.filter(|_| reading),
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

        let (cur, zoom, shown, lit) = (self.cur, self.zoom, self.shown.get(), self.lit);
        let full = area.as_size();
        let grid = self.grid();
        let slot = Rect::new(0, 0, grid.slot.width, grid.slot.height);
        let thumb = Block::bordered().inner(slot).as_size();
        // Both looks are kept, so `t` back is immediate.
        self.slides.retain(|job, _| {
            let key = job.key;
            if key.size() == full {
                let kept = key.zoom == zoom || key.zoom == Zoom::FIT || Some(*job) == shown;
                let lighting = job.lit.is_none() || job.lit == lit || Some(*job) == shown;
                key.page.abs_diff(cur) <= KEEP && kept && lighting
            } else {
                key.size() == thumb
            }
        });
    }

    // ── drawing ─────────────────────────────────────────────────────────

    fn draw(&self, f: &mut Frame) {
        let (main, status) = split(f.area());
        // Kitty's images are cells, which the list covers; others' are not.
        let covered = self.listing()
            && self.look == Look::Image
            && !matches!(
                self.picker.protocol_type(),
                ProtocolType::Kitty | ProtocolType::Halfblocks
            );
        if !covered {
            match self.view {
                View::Present => {
                    let job = self.job(self.presented());
                    let shown = self.draw_slide(f, job, main);
                    self.shown.set(shown.or(self.shown.get()));
                }
                View::Grid => self.draw_grid(f),
            }
        }
        f.render_widget(Paragraph::new(self.status()), status);
        if !self.listing() {
            return;
        }
        let size = Size::new((main.width * 3 / 4).max(40), (main.height * 3 / 4).max(10));
        let area = centred(main, size);
        let block = Block::bordered().border_style(Style::new().fg(Color::Yellow));
        let inner = block.inner(area);
        f.render_widget(Clear, area);
        f.render_widget(block, area);
        if let Some(browser) = &self.browser {
            browser.draw(f, inner);
        }
        #[cfg(feature = "speech")]
        if let Some(menu) = &self.voices {
            menu.draw(f, inner);
        }
    }

    /// How far the voice being downloaded is, if one is.
    #[cfg(feature = "speech")]
    fn download_note(&self) -> Option<String> {
        let (download, shown) = self.download.as_ref()?;
        Some(format!(" · downloading {} {shown}%", download.name))
    }

    #[cfg(not(feature = "speech"))]
    fn download_note(&self) -> Option<String> {
        None
    }

    /// Whether a list is open over the slides.
    fn listing(&self) -> bool {
        #[cfg(feature = "speech")]
        if self.voices.is_some() {
            return true;
        }
        self.browser.is_some()
    }

    /// The slide `job` renders, in `area`, which is its key's size; until it
    /// is rendered, the same slide as last presented or fitted. Returns the
    /// job whose slide is drawn.
    fn draw_slide(&self, f: &mut Frame, job: Job, area: Rect) -> Option<Job> {
        let fitted = Job {
            key: Key {
                zoom: Zoom::FIT,
                ..job.key
            },
            ..job
        };
        let same = |other: &Job| other.key.page == job.key.page && other.look == job.look;
        let unlit = Job { lit: None, ..job };
        let found = [
            Some(job),
            self.shown.get().filter(same),
            Some(fitted),
            Some(unlit),
        ]
        .into_iter()
        .flatten()
        .filter(|j| j.key.size() == job.key.size())
        .find_map(|j| Some((j, self.slides.get(&j).or_else(|| self.stale.get(&j))?)));
        match found.map(|(_, slide)| slide) {
            Some(Some(Slide::Image(proto))) => {
                f.render_widget(Image::new(proto), centred(area, proto.size()));
            }
            Some(Some(Slide::Text { cells, backdrop })) => {
                let area = centred(area, backdrop.size());
                f.render_widget(Image::new(backdrop), area);
                text::overlay(cells, area, f.buffer_mut());
            }
            Some(None) => note(f, area, "cannot render this slide"),
            None => note(f, area, "rendering…"),
        }
        found.map(|(j, _)| j)
    }

    fn draw_grid(&self, f: &mut Frame) {
        let grid = self.grid();
        for (i, rect) in self.slots(&grid) {
            let on = i == self.cur;
            let (style, border) = if on {
                (
                    Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                    BorderType::Thick,
                )
            } else {
                (Style::new().fg(Color::DarkGray), BorderType::Plain)
            };
            let block = Block::bordered()
                .border_type(border)
                .border_style(style)
                .title(Span::styled(format!(" {} ", i + 1), style));
            let inner = block.inner(rect);
            f.render_widget(block, rect);
            self.draw_slide(f, self.job(Key::new(i, inner)), inner);
        }
    }

    fn status(&self) -> Line<'_> {
        let dim = Style::new().fg(Color::DarkGray);
        let zoom = format!("present {}%", self.zoom.percent);
        let (view, keys) = match self.view {
            View::Present if self.zoomed() => (
                zoom.as_str(),
                "arrows pan · +/- zoom · 0 fit · n/p next/prev · t text · s speak · v voice · r reload · ^O open · q quit",
            ),
            View::Present => (
                "present",
                "n/p ←/→ next/prev · +/- zoom · g grid · t text · s speak · v voice · r reload · ^O open · q quit",
            ),
            View::Grid => (
                "grid",
                "n/p arrows move · +/- zoom · g/enter present · t text · s speak · v voice · r reload · ^O open · q quit",
            ),
        };
        let look = match self.look {
            Look::Image => "",
            Look::Text => " · text",
        };
        let mut speaking = String::new();
        if self.speaker.reading() {
            speaking.push_str(" · speaking");
        }
        if let Some(download) = self.download_note() {
            speaking.push_str(&download);
        }
        let mut spans = vec![
            Span::styled(
                format!(" {} ", self.deck.name),
                Style::new().add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(" {}/{} ", self.cur + 1, self.deck.pages)),
            Span::styled(format!(" {view}{look}{speaking}"), dim),
        ];
        let waiting = self
            .visible()
            .into_iter()
            .filter(|&key| !self.slides.contains_key(&self.job(key)))
            .count();
        if waiting > 0 {
            spans.push(Span::styled(format!("  rendering {waiting}"), dim));
        }
        if self.loading.is_some() {
            let what = match &self.opening {
                Some(path) => format!("opening {}…", path.display()),
                None if render::markdown(&self.deck.path) => "typesetting…".into(),
                None => "reloading…".into(),
            };
            spans.push(Span::styled(format!("  {what}"), dim));
        }
        if let Some(trouble) = &self.trouble {
            spans.push(Span::styled(
                format!("  {trouble}"),
                Style::new().add_modifier(Modifier::BOLD),
            ));
        }
        if let Some(notice) = &self.notice {
            spans.push(Span::styled(
                format!("  {notice}"),
                Style::new().add_modifier(Modifier::BOLD),
            ));
        }
        spans.push(Span::styled(format!("   {keys}"), dim));
        Line::from(spans)
    }

    /// Free the images kitty holds for us; it keeps them after we exit.
    fn clear_images(&self) -> anyhow::Result<()> {
        if self.picker.protocol_type() != ProtocolType::Kitty {
            return Ok(());
        }
        let (start, esc, end) = Parser::tmux_start_escape_end(self.picker.tmux_detected());
        let mut out = std::io::stdout();
        write!(out, "{start}{esc}_Ga=d,d=A,q=2{esc}\\{end}")?;
        out.flush()?;
        Ok(())
    }
}

/// However ohp ends, even by a panic, a reload under way stops with it.
impl Drop for App {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The slide area and the status line under it.
fn split(area: Rect) -> (Rect, Rect) {
    let [main, status] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);
    (main, status)
}

fn centred(area: Rect, size: Size) -> Rect {
    let (w, h) = (size.width.min(area.width), size.height.min(area.height));
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

fn note(f: &mut Frame, area: Rect, text: &str) {
    let mid = Rect::new(
        area.x,
        area.y + area.height / 2,
        area.width,
        1.min(area.height),
    );
    let text = Span::styled(text, Style::new().fg(Color::DarkGray));
    f.render_widget(Paragraph::new(text).centered(), mid);
}

#[cfg(test)]
#[path = "tests/app.rs"]
mod tests;
