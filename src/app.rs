//! The two views, one slide or a grid of them, and the loop that drives them.
//!
//! A slide is rendered for the exact cell box it is drawn in, so each box
//! size is its own cache entry. Input is drained before each redraw, so a
//! burst of key presses costs one frame.
//!
//! The PDF is reloaded when it changes on disk, as after a LaTeX run. The
//! slides already on screen stay until their new renders arrive, so a reload
//! does not flash the screen empty.

use crate::render::{self, Deck, Done, Key, Renderer, Stamp};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect, Size};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};
use ratatui::{DefaultTerminal, Frame};
use ratatui_image::Image;
use ratatui_image::picker::cap_parser::{Parser, QueryStdioOptions};
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::Protocol;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::time::{Duration, Instant};

/// Slides the grid tries to show at once.
const GRID_TARGET: usize = 12;
const GRID_MAX_COLS: u16 = 8;
/// Narrowest grid slot +/- zooms out to, in cells.
const MIN_SLOT_WIDTH: u16 = 10;
/// Full-size slides kept either side of the current one.
const KEEP: usize = 2;
const MAX_WORKERS: usize = 4;
/// How often the PDF is checked for changes.
const WATCH_EVERY: Duration = Duration::from_millis(250);

pub fn run(deck: Deck) -> anyhow::Result<()> {
    let mut terminal = ratatui::init();
    let result = (|| {
        let picker = pick();
        quiet_render_panics();
        let workers = std::thread::available_parallelism().map_or(1, |n| n.get().min(MAX_WORKERS));
        let renderer = Renderer::spawn(&deck, &picker, workers)?;
        let mut app = App::new(deck, picker, renderer, workers);
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
    Picker::from_query_stdio_with_options(options).unwrap_or_else(|_| Picker::halfblocks())
}

fn over_ssh() -> bool {
    ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"]
        .iter()
        .any(|v| std::env::var_os(v).is_some())
}

/// A slide hayro cannot render shows as failed; its panic must not reach
/// ratatui's hook, which would tear down the terminal.
fn quiet_render_panics() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if std::thread::current().name() != Some(render::THREAD) {
            prev(info);
        }
    }));
}

#[derive(Clone, Copy, PartialEq)]
enum View {
    Present,
    Grid,
}

enum Slide {
    Ready(Protocol),
    Failed,
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

struct App {
    deck: Deck,
    picker: Picker,
    renderer: Renderer,
    workers: usize,
    view: View,
    cur: usize,
    /// First grid row on screen.
    top: usize,
    /// Grid columns chosen with +/-; `None` fits the grid to the screen.
    grid_cols: Option<u16>,
    /// Where slides are drawn: the screen above the status line.
    main: Rect,
    slides: HashMap<Key, Slide>,
    /// Slides of the deck before the last reload, shown until replaced.
    stale: HashMap<Key, Slide>,
    /// Keys queued or being rendered.
    requested: HashSet<Key>,
    /// The PDF as last loaded.
    stamp: Stamp,
    /// A change seen on disk, waiting for the file to stop changing.
    settling: Option<Stamp>,
    checked: Instant,
    /// Shown in the status line until the next key press.
    notice: Option<String>,
    dirty: bool,
    quit: bool,
}

impl App {
    fn new(deck: Deck, picker: Picker, renderer: Renderer, workers: usize) -> Self {
        App {
            stamp: render::stamp(&deck.path),
            deck,
            picker,
            renderer,
            workers,
            view: View::Present,
            cur: 0,
            top: 0,
            grid_cols: None,
            main: Rect::default(),
            slides: HashMap::new(),
            stale: HashMap::new(),
            requested: HashSet::new(),
            settling: None,
            checked: Instant::now(),
            notice: None,
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
        if !self.requested.remove(&done.key) {
            return;
        }
        let slide = done.slide.map_or(Slide::Failed, Slide::Ready);
        self.slides.insert(done.key, slide);
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
        let now = render::stamp(&self.deck.path);
        if now == self.stamp {
            self.settling = None;
        } else if self.settling == Some(now) && render::finished(&self.deck.path) {
            self.settling = None;
            self.stamp = now;
            self.reload();
        } else if self.settling != Some(now) {
            self.settling = Some(now);
            self.notice = Some("PDF changing…".into());
            self.dirty = true;
        }
    }

    fn reload(&mut self) {
        let fresh = Deck::open(&self.deck.path).and_then(|deck| {
            let renderer = Renderer::spawn(&deck, &self.picker, self.workers)?;
            Ok((deck, renderer))
        });
        match fresh {
            Ok((deck, renderer)) => {
                self.cur = self.cur.min(deck.pages - 1);
                self.deck = deck;
                self.renderer = renderer;
                self.requested.clear();
                self.stale = std::mem::take(&mut self.slides);
                self.notice = Some("reloaded".into());
            }
            Err(e) => self.notice = Some(format!("reload failed: {e:#}")),
        }
        self.dirty = true;
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
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => self.quit = true,
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
            KeyCode::Enter | KeyCode::Esc if self.view == View::Grid => self.view = View::Present,
            KeyCode::Char('+' | '=') if self.view == View::Grid => self.zoom(-1),
            KeyCode::Char('-' | '_') if self.view == View::Grid => self.zoom(1),
            _ => return,
        }
        self.dirty = true;
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
    fn zoom(&mut self, by: i32) {
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

    /// Queue what the view shows, most wanted first. Kept are the full-size
    /// slides near the current one and the grid's slides at its zoom.
    fn schedule(&mut self) {
        let area = self.main;
        let grid = self.grid();
        let mut wanted = Vec::new();
        let full = |page| Key::new(page, area);
        let near = [self.cur, self.cur + 1, self.cur.wrapping_sub(1), self.cur + 2];
        match self.view {
            View::Present => wanted.extend(near.map(full)),
            View::Grid => {
                let thumb = |(i, r): (usize, Rect)| Key::new(i, Block::bordered().inner(r));
                wanted.extend(self.slots(&grid).find(|&(i, _)| i == self.cur).map(thumb));
                wanted.extend(self.slots(&grid).map(thumb));
                // So leaving the grid shows the slide at once.
                wanted.push(full(self.cur));
            }
        }
        wanted.retain(|k| k.page < self.deck.pages);

        for key in self.renderer.clear() {
            self.requested.remove(&key);
        }
        let mut jobs = Vec::new();
        for key in wanted {
            if !self.slides.contains_key(&key) && self.requested.insert(key) {
                jobs.push(key);
            }
        }
        self.renderer.push(jobs);

        let cur = self.cur;
        let full = (area.width, area.height);
        let inner = Block::bordered().inner(Rect::new(0, 0, grid.slot.width, grid.slot.height));
        let thumb = (inner.width, inner.height);
        self.slides.retain(|k, _| {
            let size = (k.cols, k.rows);
            if size == full {
                k.page.abs_diff(cur) <= KEEP
            } else {
                size == thumb
            }
        });
    }

    // ── drawing ─────────────────────────────────────────────────────────

    fn draw(&self, f: &mut Frame) {
        let (main, status) = split(f.area());
        match self.view {
            View::Present => self.draw_slide(f, self.cur, main),
            View::Grid => self.draw_grid(f),
        }
        f.render_widget(Paragraph::new(self.status()), status);
    }

    fn draw_slide(&self, f: &mut Frame, page: usize, area: Rect) {
        let key = Key::new(page, area);
        match self.slides.get(&key).or_else(|| self.stale.get(&key)) {
            Some(Slide::Ready(proto)) => f.render_widget(Image::new(proto), centred(area, proto.size())),
            Some(Slide::Failed) => note(f, area, "cannot render this slide"),
            None => note(f, area, "rendering…"),
        }
    }

    fn draw_grid(&self, f: &mut Frame) {
        let grid = self.grid();
        for (i, rect) in self.slots(&grid) {
            let on = i == self.cur;
            let (style, border) = if on {
                (Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD), BorderType::Thick)
            } else {
                (Style::new().fg(Color::DarkGray), BorderType::Plain)
            };
            let block = Block::bordered()
                .border_type(border)
                .border_style(style)
                .title(Span::styled(format!(" {} ", i + 1), style));
            let inner = block.inner(rect);
            f.render_widget(block, rect);
            self.draw_slide(f, i, inner);
        }
    }

    fn status(&self) -> Line<'_> {
        let dim = Style::new().fg(Color::DarkGray);
        let (view, keys) = match self.view {
            View::Present => ("present", "n/p ←/→ next/prev · g grid · q quit"),
            View::Grid => ("grid", "n/p arrows move · +/- zoom · g/enter present · q quit"),
        };
        let mut spans = vec![
            Span::styled(format!(" {} ", self.deck.name), Style::new().add_modifier(Modifier::BOLD)),
            Span::raw(format!(" {}/{} ", self.cur + 1, self.deck.pages)),
            Span::styled(format!(" {view}"), dim),
        ];
        if !self.requested.is_empty() {
            spans.push(Span::styled(format!("  rendering {}", self.requested.len()), dim));
        }
        if let Some(notice) = &self.notice {
            spans.push(Span::styled(format!("  {notice}"), Style::new().add_modifier(Modifier::BOLD)));
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

/// The slide area and the status line under it.
fn split(area: Rect) -> (Rect, Rect) {
    let [main, status] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);
    (main, status)
}

fn centred(area: Rect, size: Size) -> Rect {
    let (w, h) = (size.width.min(area.width), size.height.min(area.height));
    Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h)
}

fn note(f: &mut Frame, area: Rect, text: &str) {
    let mid = Rect::new(area.x, area.y + area.height / 2, area.width, 1.min(area.height));
    let text = Span::styled(text, Style::new().fg(Color::DarkGray));
    f.render_widget(Paragraph::new(text).centered(), mid);
}

#[cfg(test)]
#[path = "tests/app.rs"]
mod tests;
