//! What keys and the mouse do: turning, zooming and panning the slides,
//! and the lists over them taking the keys while open.

use super::{App, Panel, View, ZOOMS};
use crate::browse;
use crate::render::{self, Look, Slide, Zoom};

use crate::text;
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Position;

impl App {
    pub(super) fn handle(&mut self, ev: Event) {
        match ev {
            Event::Key(key) if key.kind != KeyEventKind::Release => self.key(key),
            Event::Mouse(mouse) => self.mouse(mouse),
            Event::Resize(..) => self.dirty = true,
            _ => {}
        }
    }

    /// A click, while reading aloud, reads the presented slide from the
    /// sentence clicked, or picks the slide clicked in the grid, and presents it clicked
    /// again, so a double click presents it. The wheel is the arrow keys,
    /// as the terminal makes it when the mouse is not taken.
    pub(super) fn mouse(&mut self, mouse: MouseEvent) {
        if self.listing() {
            return;
        }
        let at = Position::new(mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::ScrollDown => self.key(KeyCode::Down.into()),
            MouseEventKind::ScrollUp => self.key(KeyCode::Up.into()),
            MouseEventKind::Down(MouseButton::Left) => match self.view {
                #[cfg(feature = "sign")]
                View::Present if self.signing.is_some() => {
                    self.sign_click(at);
                    self.dirty = true;
                }
                View::Present if self.speaker.reading() => {
                    if let Some(k) = self.sentence_at(at) {
                        self.notice = None;
                        self.speak_from(k);
                    }
                }
                View::Present => {}
                View::Grid => {
                    let grid = self.grid();
                    let clicked = self.slots(&grid).find(|(_, slot)| slot.contains(at));
                    if let Some((i, _)) = clicked {
                        self.notice = None;
                        if i == self.cur {
                            self.view = View::Present;
                        }
                        self.cur = i;
                        self.dirty = true;
                    }
                }
            },
            _ => {}
        }
    }

    /// The sentence of the presented slide shown at `at` on the screen, as
    /// `text::Page::light` counts them: the one a word of the slide's text
    /// set there is read in, or else the one nearest that point of it.
    fn sentence_at(&self, at: Position) -> Option<usize> {
        let (job, area) = self.drawn.get()?;
        if !area.contains(at) {
            return None;
        }
        let page = text::of(self.deck.data.clone(), job.key.page)?;
        let (dx, dy) = (at.x - area.x, at.y - area.y);
        if let Some(Some(Slide::Text { cells, .. })) = self.slides.get(&job)
            && let Some(Some(cell)) = cells
                .get(usize::from(dy))
                .and_then(|row| row.get(usize::from(dx)))
        {
            return page.sentence_of(cell.glyph);
        }
        let font = self.picker.font_size();
        let (fw, fh) = (f32::from(font.width), f32::from(font.height));
        let key = job.key;
        let scale = render::fit(key, (fw, fh), (page.width, page.height)) * key.zoom.scale();
        // The middle of the cell, past the part of the slide panned off.
        let point = |pan: u16, cell: u16, size: f32| {
            (f32::from(pan) + f32::from(cell) + 0.5) * size / scale
        };
        page.sentence_near(point(key.zoom.x, dx, fw), point(key.zoom.y, dy, fh))
    }

    pub(super) fn key(&mut self, key: KeyEvent) {
        self.notice = None;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if let Some(browser) = &mut self.browser {
            match browser.key(key) {
                browse::Outcome::Stay => {}
                #[cfg(feature = "sign")]
                browse::Outcome::Open(paths) if self.picking_signature() => {
                    self.browser = None;
                    self.picked_signature(Some(paths));
                }
                browse::Outcome::Open(paths) => {
                    self.browser = None;
                    self.open(self.added_first(paths));
                }
                browse::Outcome::Quit => {
                    self.browser = None;
                    #[cfg(feature = "sign")]
                    self.picked_signature(None);
                }
            }
            self.dirty = true;
            return;
        }
        #[cfg(feature = "speech")]
        if self.voices.is_open() {
            if let Some(event) = self.voices.key(key) {
                self.voiced(event);
            }
            self.dirty = true;
            return;
        }
        #[cfg(feature = "summary")]
        if self.models.is_open() {
            if let Some(weights) = self.models.key(key) {
                self.summarize_with(weights);
            }
            self.dirty = true;
            return;
        }
        #[cfg(feature = "sign")]
        if self.sign_key(key) {
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
            KeyCode::Char('r') => {
                self.deck.options.forget();
                self.reload();
            }
            KeyCode::Char('s') if self.speaker.reading() => self.speaker.stop(),
            KeyCode::Char('s') => self.speak(),
            KeyCode::Char('v') => self.choose_voice(),
            KeyCode::Char('k') => self.summarize(Panel::Slide),
            KeyCode::Char('K') => self.summarize(Panel::Whole),
            KeyCode::Char('m') => self.choose_model(),
            #[cfg(feature = "sign")]
            KeyCode::Char('S') => self.sign(),
            #[cfg(not(feature = "sign"))]
            KeyCode::Char('S') => {
                self.notice = Some("ohp was built without signing: --features sign".into())
            }
            _ => return,
        }
        self.dirty = true;
    }

    pub(super) fn zoomed(&self) -> bool {
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
    pub(super) fn pan(&mut self, across: i32, down: i32) {
        let by = |at: u16, screen: u16, steps: i32| {
            let step = i32::from((screen / 4).max(1));
            (i32::from(at) + steps * step).clamp(0, u16::MAX.into()) as u16
        };
        self.zoom.x = by(self.zoom.x, self.main.width, across);
        self.zoom.y = by(self.zoom.y, self.main.height, down);
        self.clamp_pan();
    }

    /// Keep the zoomed slide covering the screen where it is large enough to.
    pub(super) fn clamp_pan(&mut self) {
        let font = self.picker.font_size();
        let (fw, fh) = (f32::from(font.width), f32::from(font.height));
        let (pw, ph) = self.deck.sizes[self.cur];
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
}
