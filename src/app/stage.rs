//! Presenting to a second window: the presenter's view, with the slide,
//! the next, the notes, a strip of slides to pick from and the time, and
//! the audience's, with the slide alone. See [`crate::link`].

use super::draw::centred;
use super::{App, View};
use crate::link::{self, Said, Stage, Turn};
use crate::notes::{self, Beside};
use crate::render::{Key, Zoom};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect, Size};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long the audience's window says what to do with it.
const HINT: Duration = Duration::from_secs(12);

/// What a key that used to quit says now.
pub(super) const QUIT_HINT: &str = "C-x C-c quits";
/// What printing or annotating, presenting, says.
const NOT_PRESENTING: &str = "w ends presenting, to print or annotate";

/// The presenter's colours.
const LIT: Style = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);
const DIM: Style = Style::new().fg(Color::DarkGray);
const RED: Style = Style::new().fg(Color::Red).add_modifier(Modifier::BOLD);
const CAP: Style = Style::new()
    .fg(Color::Black)
    .bg(Color::Rgb(255, 214, 10))
    .add_modifier(Modifier::BOLD);

/// Time since the talk began, but for while it was paused.
pub(super) struct Clock {
    pub(super) began: Instant,
    paused: Option<Instant>,
    /// When the time was last clicked, to see a double click.
    clicked: Option<Instant>,
}

/// Two clicks this close together are a double click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

impl Clock {
    pub(super) fn new() -> Self {
        Clock {
            began: Instant::now(),
            paused: None,
            clicked: None,
        }
    }

    pub(super) fn elapsed(&self) -> Duration {
        self.paused.unwrap_or_else(Instant::now) - self.began
    }

    pub(super) fn paused(&self) -> bool {
        self.paused.is_some()
    }

    fn pause(&mut self) {
        match self.paused.take() {
            Some(at) => self.began += at.elapsed(),
            None => self.paused = Some(Instant::now()),
        }
    }

    fn reset(&mut self) {
        self.began = Instant::now();
        self.paused = self.paused.map(|_| self.began);
    }

    /// A click pauses the time, or goes on; a double click starts it
    /// again from nought, going.
    fn click(&mut self) {
        let double = self
            .clicked
            .take()
            .is_some_and(|at| at.elapsed() < DOUBLE_CLICK);
        if double {
            self.paused = None;
            self.reset();
        } else {
            self.pause();
            self.clicked = Some(Instant::now());
        }
    }
}

/// `d` as `m:ss`, or `h:mm:ss` from an hour.
pub(super) fn clock_face(d: Duration) -> String {
    let s = d.as_secs();
    match s / 3600 {
        0 => format!("{}:{:02}", s / 60, s % 60),
        h => format!("{h}:{:02}:{:02}", s / 60 % 60, s % 60),
    }
}

/// Where the presenter's view puts things, each a bordered box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Boxes {
    /// The row of tools along the top.
    pub bar: Rect,
    pub current: Rect,
    pub clock: Rect,
    pub next: Rect,
    pub notes: Rect,
    /// The strip of slides along the bottom, and its slots' size.
    pub strip: Rect,
    pub slot: Size,
}

/// The audience's end, in its window.
pub(super) struct Seat {
    pub link: link::Audience,
    /// The slide told, which may be past the end of the deck until the
    /// deck it is of is read.
    pub page: usize,
    /// Until when the window says what to do with it.
    pub hint: Instant,
}

impl App {
    /// Whether this is the presenter's window of two.
    pub(super) fn presenting(&self) -> bool {
        self.stage.is_some()
    }

    /// Whether the presenter's view is on screen: presenting, not picking
    /// from the grid.
    pub(super) fn stage_view(&self) -> bool {
        self.presenting() && self.view == View::Present
    }

    /// `w`: begin presenting to a second window, opening it; open it again
    /// if it was closed; or, with it open, stop.
    pub(super) fn toggle_stage(&mut self) {
        match &self.stage {
            Some(_) => {
                self.stage = None;
                self.black = false;
                self.laser = None;
                if self.hover {
                    self.toggle_hover();
                }
                self.uncut_notes();
                self.notice = Some("presenting to one window again".into());
            }
            None => match Stage::new() {
                Ok(stage) => {
                    self.stage = Some(stage);
                    self.clock = Clock::new();
                    self.zoom = Zoom::FIT;
                    self.view = View::Present;
                    self.restage();
                    if self.stage.is_none() {
                        return;
                    }
                    self.open_audience();
                }
                Err(e) => self.cannot_present(&e),
            },
        }
        self.forget();
    }

    fn cannot_present(&mut self, e: &dyn std::fmt::Display) {
        self.notice = Some(format!("cannot present to a second window: {e}"));
    }

    /// The audience's window is gone.
    fn lost_audience(&mut self) {
        if let Some(stage) = &mut self.stage {
            stage.peer = None;
        }
        self.notice = Some("the audience's window closed: W opens it again".into());
        self.dirty = true;
    }

    /// `W`: open the audience's window, again: off this thread, as the
    /// terminal may take a while to say it did.
    pub(super) fn open_audience(&mut self) {
        let Some(stage) = &self.stage else {
            return;
        };
        let socket = stage.socket();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || tx.send(link::open_window(&socket)));
        self.opening_window = Some(rx);
        self.notice = Some("opening the audience's window…".into());
    }

    /// Say whether the audience's window was opened, once it is known.
    fn opened_audience(&mut self) {
        let Some(rx) = &self.opening_window else {
            return;
        };
        let Ok(opened) = rx.try_recv() else {
            return;
        };
        self.opening_window = None;
        self.dirty = true;
        self.notice = Some(match opened {
            Ok(terminal) => format!(
                "opened the audience's window in {terminal}: drag it to the projector and make it full screen"
            ),
            Err(command) if copy(&command) => {
                "no window can be opened from here: run the command copied to the clipboard in one on the projector".into()
            }
            Err(command) => {
                format!("for the audience, run in a window on the projector: {command}")
            }
        });
    }

    /// The deck, as read again, written for the audience's window to read:
    /// beamer's slides with their notes beside them, the slides alone,
    /// while the presenter's shows both.
    pub(super) fn restage(&mut self) {
        // The deck is new: the one cut before is not to come back.
        self.note_pages = false;
        self.uncut = None;
        if self.stage.is_some() {
            let audience = self.cut_notes();
            // The slide told again, after the deck it is of.
            self.told = None;
            if let Some(stage) = &self.stage
                && let Err(e) = stage.write(&audience)
            {
                self.stage = None;
                self.cannot_present(&e);
            }
        }
        self.read_notes();
    }

    /// The deck as the audience is shown it: beamer's slides cut from
    /// their notes, which the presenter's window is shown as well.
    #[cfg(feature = "sign")]
    fn cut_notes(&mut self) -> Arc<Vec<u8>> {
        if !crate::split::noted(&self.deck.sizes) {
            return self.deck.data.clone();
        }
        match crate::split::split(&self.deck.data) {
            Ok(Some(cut)) => {
                self.show_notes(cut.both);
                Arc::new(cut.slides)
            }
            Ok(None) => self.deck.data.clone(),
            Err(e) => {
                self.notice = Some(format!("cannot cut the notes off the slides: {e:#}"));
                self.deck.data.clone()
            }
        }
    }

    /// The deck as the audience is shown it: ohp cuts beamer's notes off
    /// with lopdf, which it is built without here.
    #[cfg(not(feature = "sign"))]
    fn cut_notes(&mut self) -> Arc<Vec<u8>> {
        self.deck.data.clone()
    }

    /// Show `both`, the deck's slides followed by their notes, the notes
    /// past its last page.
    #[cfg(feature = "sign")]
    fn show_notes(&mut self, both: Vec<u8>) {
        let data = Arc::new(both);
        let Ok(pdf) = hayro::hayro_syntax::Pdf::new(data.clone()) else {
            return;
        };
        let sizes = crate::render::sizes(&pdf);
        if sizes.len() != 2 * self.deck.pages {
            return;
        }
        let deck = crate::render::Deck {
            data: data.clone(),
            sizes,
            ..self.deck.clone()
        };
        match crate::render::Renderer::spawn(&deck, &self.picker, self.workers) {
            Ok(renderer) => {
                self.summarizer.set_deck(data, deck.pages);
                self.uncut = Some(std::mem::replace(&mut self.deck, deck));
                self.renderer = renderer;
                self.note_pages = true;
                self.shown.set(None);
                self.forget();
            }
            Err(e) => self.notice = Some(format!("cannot show the notes: {e:#}")),
        }
    }

    /// The deck as it was before its notes were cut off, back, as
    /// presenting ends.
    fn uncut_notes(&mut self) {
        self.note_pages = false;
        let Some(deck) = self.uncut.take() else {
            return;
        };
        match crate::render::Renderer::spawn(&deck, &self.picker, self.workers) {
            Ok(renderer) => {
                self.summarizer.set_deck(deck.data.clone(), deck.pages);
                self.deck = deck;
                self.renderer = renderer;
                self.shown.set(None);
            }
            // Read again from its files, as it cannot be shown as kept.
            Err(_) => self.reload(),
        }
    }

    /// The speaker notes, from the deck and the files beside it, read again.
    pub(super) fn read_notes(&mut self) {
        self.beside = Beside::new(&self.deck.paths, &self.deck.starts());
        self.notes = notes::merge(self.deck.speaker.clone(), self.beside.read());
        self.dirty = true;
    }

    /// Hear the other window, tell it what changed, and keep the clock
    /// going: called each turn of the loop.
    pub(super) fn tend(&mut self) {
        if let Some(seat) = &mut self.seat {
            let mut quit = false;
            for said in seat.link.peer.heard() {
                match said {
                    // Kept as told, as it may be past the end of the deck
                    // until the deck it is of is read.
                    Said::Page(n) => {
                        seat.page = n;
                        self.cur = n.min(self.deck.pages - 1);
                    }
                    Said::Black(on) => self.black = on,
                    Said::Laser(at) => self.laser = at,
                    Said::Bye => quit = true,
                    Said::Deck(_) | Said::Key(_) => continue,
                }
                self.dirty = true;
            }
            self.quit |= quit;
            if seat.hint.elapsed() < HINT + Duration::from_secs(1) && seat.hint.elapsed() >= HINT {
                self.dirty = true;
            }
            return;
        }
        self.opened_audience();
        let Some(stage) = &mut self.stage else {
            return;
        };
        if stage.arrived() {
            self.told = None;
            self.told_laser = None;
            self.notice = Some(
                "the audience's window is connected: drag it to the projector, then full screen"
                    .into(),
            );
            let pdf = stage.pdf();
            if let Some(peer) = stage.peer.as_mut() {
                peer.say(&Said::Deck(pdf));
            }
            self.dirty = true;
        }
        let mut heard = Vec::new();
        if let Some(peer) = &stage.peer {
            heard = peer.heard();
        }
        for said in heard {
            match said {
                Said::Key(Turn::Next) => self.turn(1),
                Said::Key(Turn::Prev) => self.turn(-1),
                Said::Key(Turn::Black) => self.black = !self.black,
                Said::Bye => self.lost_audience(),
                Said::Deck(_) | Said::Page(_) | Said::Black(_) | Said::Laser(_) => continue,
            }
            self.dirty = true;
        }
        // Picking in the grid, the audience goes on seeing the slide shown,
        // and no laser on it.
        if self.view == View::Grid {
            self.laser = None;
        } else {
            self.live = self.cur;
        }
        // What changed, told the audience; a window that cannot hear it is gone.
        let now = (self.live, self.black);
        if let Some(stage) = &mut self.stage
            && let Some(peer) = stage.peer.as_mut()
        {
            let mut heard = true;
            if self.told != Some(now) {
                heard = peer.say(&Said::Page(now.0)) && peer.say(&Said::Black(now.1));
                self.told = Some(now);
            }
            if heard && self.told_laser != self.laser {
                heard = peer.say(&Said::Laser(self.laser));
                self.told_laser = self.laser;
            }
            if !heard {
                self.lost_audience();
            }
        }
        // Each second, the clock is drawn again, and the notes files
        // looked at, paused or not.
        let second = self.clock.elapsed().as_secs();
        if self.ticked != second {
            self.ticked = second;
            self.dirty |= self.stage_view();
        }
        if self.notes_looked.elapsed() >= Duration::from_secs(1) {
            self.notes_looked = Instant::now();
            if self.beside.changed(&self.deck.paths) {
                self.read_notes();
            }
        }
    }

    /// Keys in the audience's window: those that turn the slides are the
    /// presenter's to act on.
    pub(super) fn seat_key(&mut self, key: ratatui::crossterm::event::KeyEvent) {
        use ratatui::crossterm::event::KeyCode;
        let Some(seat) = &mut self.seat else {
            return;
        };
        let turn = match key.code {
            KeyCode::Char('n' | ' ' | 'j' | 'l')
            | KeyCode::Right
            | KeyCode::Down
            | KeyCode::PageDown
            | KeyCode::Enter => Turn::Next,
            KeyCode::Char('p' | 'k' | 'h') | KeyCode::Left | KeyCode::Up | KeyCode::PageUp => {
                Turn::Prev
            }
            KeyCode::Char('b' | '.') => Turn::Black,
            KeyCode::Char('q') | KeyCode::Esc => {
                self.notice = Some(QUIT_HINT.into());
                self.dirty = true;
                return;
            }
            _ => return,
        };
        if !seat.link.peer.say(&Said::Key(turn)) {
            self.quit = true;
        }
    }

    /// A slide turned to by a key pressed in the audience's window: while
    /// picking in the grid, the one shown, and the pick with it.
    fn turn(&mut self, by: isize) {
        if self.view == View::Grid {
            self.back_to_shown();
        }
        self.step(by);
    }

    /// Picking privately in the grid, given up: back to the slide shown.
    pub(super) fn back_to_shown(&mut self) {
        self.cur = self.live;
        self.view = View::Present;
    }

    /// The file slide `slide` is from, by its place among the deck's, and
    /// its page there, from 0.
    fn page_in_file(&self, slide: usize) -> (usize, usize) {
        let source = self.deck.source(slide);
        (source, slide - self.deck.starts()[source])
    }

    /// `N`, or a click on the notes: write the current slide's notes, into
    /// the notes file beside the file it is from.
    fn begin_writing(&mut self) {
        let (source, page) = self.page_in_file(self.cur);
        let file = notes::file_for(&self.deck.paths[source]);
        let text = notes::written(&file, page);
        self.writing = Some(Writing {
            slide: self.cur,
            page,
            file,
            at: text.chars().count(),
            text,
        });
    }

    /// Save the notes being written, and stop writing; if they cannot be
    /// saved, go on writing them, and say so. True if saved, or none were
    /// being written.
    pub(super) fn finish_writing(&mut self) -> bool {
        let Some(writing) = self.writing.take() else {
            return true;
        };
        let name = writing.file.display().to_string();
        match notes::save(&writing.file, writing.page, &writing.text) {
            Ok(()) => {
                self.notice = Some(format!("notes saved to {name}"));
                self.read_notes();
                true
            }
            Err(e) => {
                self.notice = Some(format!(
                    "cannot save the notes to {name}: {e} · C-g drops them"
                ));
                self.writing = Some(writing);
                false
            }
        }
    }

    /// A click while writing notes: in them, the cursor goes there;
    /// elsewhere, they are saved, as esc saves them.
    pub(super) fn writing_click(&mut self, at: Position) {
        let inner = Block::bordered().inner(self.boxes().notes);
        match &mut self.writing {
            Some(writing) if inner.contains(at) => {
                writing.click(at.x - inner.x, at.y - inner.y, inner.width);
            }
            Some(_) => {
                self.finish_writing();
            }
            None => {}
        }
    }

    /// A key while writing notes: esc saves them, C-g drops them.
    pub(super) fn write_key(&mut self, key: ratatui::crossterm::event::KeyEvent) {
        use ratatui::crossterm::event::{KeyCode, KeyModifiers};
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Some(writing) = &mut self.writing else {
            return;
        };
        match key.code {
            KeyCode::Char('g') if ctrl => {
                self.writing = None;
                self.notice = Some("notes left as they were".into());
            }
            KeyCode::Esc => {
                self.finish_writing();
            }
            KeyCode::Char('a') if ctrl => writing.home(),
            KeyCode::Char('e') if ctrl => writing.end(),
            KeyCode::Char('b') if ctrl => writing.left(),
            KeyCode::Char('f') if ctrl => writing.right(),
            KeyCode::Char('p') if ctrl => writing.up(),
            KeyCode::Char('n') if ctrl => writing.down(),
            KeyCode::Char('d') if ctrl => writing.delete(),
            KeyCode::Char('k') if ctrl => writing.kill(),
            KeyCode::Char(c) if !ctrl => writing.insert(c),
            KeyCode::Enter => writing.insert('\n'),
            KeyCode::Backspace => {
                if writing.at > 0 {
                    writing.left();
                    writing.delete();
                }
            }
            KeyCode::Delete => writing.delete(),
            KeyCode::Left => writing.left(),
            KeyCode::Right => writing.right(),
            KeyCode::Up => writing.up(),
            KeyCode::Down => writing.down(),
            KeyCode::Home => writing.home(),
            KeyCode::End => writing.end(),
            _ => {}
        }
    }

    /// Timer and screen keys while presenting; true if the key was one.
    pub(super) fn stage_key(&mut self, key: ratatui::crossterm::event::KeyEvent) -> bool {
        use ratatui::crossterm::event::{KeyCode, KeyModifiers};
        if !self.presenting() {
            return false;
        }
        match key.code {
            KeyCode::Char('b' | '.') => self.black = !self.black,
            // Annotating and printing want the slide alone on the screen,
            // and the deck as it is, not cut from its notes.
            KeyCode::Char(c)
                if matches!(c, 'A' | 'P')
                    || (c == 'p' && key.modifiers.contains(KeyModifiers::CONTROL)) =>
            {
                self.notice = Some(NOT_PRESENTING.into());
            }
            KeyCode::Char('W') => self.open_audience(),
            KeyCode::Char('T') => self.clock.pause(),
            KeyCode::Char('N') if self.stage_view() => self.begin_writing(),
            KeyCode::Char('Z') => self.clock.reset(),
            KeyCode::Char('L') if self.stage_view() => self.toggle_hover(),
            // The presenter's slides are fitted to their boxes.
            KeyCode::Char('+' | '=' | '-' | '_') if self.stage_view() => {}
            _ => return false,
        }
        true
    }

    /// Where the presenter's view puts things.
    pub(super) fn boxes(&self) -> Boxes {
        let area = self.main;
        let font = self.picker.font_size();
        let (fw, fh) = (f32::from(font.width), f32::from(font.height));
        let (pw, ph) = self
            .deck
            .sizes
            .get(self.cur)
            .copied()
            .unwrap_or(self.deck.sizes[0]);
        let [bar, area] = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);
        let strip_h = (area.height / 5).clamp(4, 12).min(area.height / 2);
        let [top, strip] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(strip_h)]).areas(area);
        let right_w = (top.width * 2 / 5).max(10).min(top.width / 2);
        let [current, right] =
            Layout::horizontal([Constraint::Min(1), Constraint::Length(right_w)]).areas(top);
        let inner_w = f32::from(right_w.saturating_sub(2));
        let next_h = ((inner_w * fw * ph / pw / fh).ceil() as u16 + 2).min(right.height / 2);
        let [clock, next, notes] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(next_h),
            Constraint::Min(1),
        ])
        .areas(right);
        let inner_h = f32::from(strip_h.saturating_sub(2));
        let slot_w = (inner_h * fh * pw / ph / fw).ceil() as u16 + 2;
        Boxes {
            bar,
            current,
            clock,
            next,
            notes,
            strip,
            slot: Size::new(slot_w.clamp(3, strip.width.max(3)), strip_h),
        }
    }

    /// The strip's slots: slide and where, the current slide as near the
    /// middle as the ends of the deck let it be.
    pub(super) fn strip(&self, boxes: &Boxes) -> Vec<(usize, Rect)> {
        let slot = boxes.slot;
        let cols = usize::from((boxes.strip.width / slot.width).max(1)).min(self.deck.pages);
        let first = self
            .cur
            .saturating_sub(cols / 2)
            .min(self.deck.pages - cols);
        let used = slot.width * cols as u16;
        let x0 = boxes.strip.x + boxes.strip.width.saturating_sub(used) / 2;
        (0..cols)
            .map(|k| {
                let x = x0 + k as u16 * slot.width;
                (
                    first + k,
                    Rect::new(x, boxes.strip.y, slot.width, slot.height),
                )
            })
            .collect()
    }

    /// The slides the presenter's view shows, the current one first.
    pub(super) fn stage_keys(&self, boxes: &Boxes) -> Vec<Key> {
        let boxes = *boxes;
        let inner = |r: Rect| Block::bordered().inner(r);
        let mut keys = vec![Key::new(self.cur, inner(boxes.current))];
        if self.cur + 1 < self.deck.pages {
            keys.push(Key::new(self.cur + 1, inner(boxes.next)));
        }
        if self.note_pages {
            keys.push(Key::new(self.deck.pages + self.cur, self.note_box(&boxes)));
        }
        keys.extend(
            self.strip(&boxes)
                .into_iter()
                .map(|(i, r)| Key::new(i, inner(r))),
        );
        keys
    }

    /// The sizes of the presenter's view's boxes, whose slides are kept.
    pub(super) fn stage_sizes(&self, boxes: &Boxes) -> [Size; 4] {
        let boxes = *boxes;
        let inner = |r: Rect| Block::bordered().inner(r).as_size();
        let slot = Rect::new(0, 0, boxes.slot.width, boxes.slot.height);
        [
            inner(boxes.current),
            inner(boxes.next),
            inner(slot),
            self.note_box(&boxes).as_size(),
        ]
    }

    /// Where beamer's note for the slide is drawn: the notes box, but for
    /// the lower half where notes are written for it too.
    fn note_box(&self, boxes: &Boxes) -> Rect {
        let inner = Block::bordered().inner(boxes.notes);
        if self.notes.contains_key(&self.cur) {
            Rect {
                height: inner.height / 2,
                ..inner
            }
        } else {
            inner
        }
    }

    /// The laser pointer at `at` on the screen, if that is on the current
    /// slide as drawn; off if not.
    pub(super) fn point(&mut self, at: Position) {
        let laser = self.drawn.get().and_then(|(job, area)| {
            (job.key.page == self.cur && area.contains(at)).then(|| {
                let frac =
                    |at: u16, from: u16, len: u16| (f32::from(at - from) + 0.5) / f32::from(len);
                (
                    frac(at.x, area.x, area.width),
                    frac(at.y, area.y, area.height),
                )
            })
        });
        if laser != self.laser {
            self.laser = laser;
            self.dirty = true;
        }
    }

    /// `L`: the laser follows the mouse with no button held, or no longer.
    fn toggle_hover(&mut self) {
        self.hover = !self.hover;
        super::mouse_modes(self.hover);
        let notice = if self.hover {
            "the laser follows the mouse over the slide: L again stops it"
        } else {
            self.laser = None;
            "the laser shows while the mouse is held down on the slide"
        };
        self.notice = Some(notice.into());
    }

    /// A click in the presenter's view: on the strip, turn to that slide;
    /// on the next slide, turn to it; on the time, pause it or go on; on
    /// the notes, write them. True if it was one of those.
    pub(super) fn stage_click(&mut self, at: Position) -> bool {
        let boxes = self.boxes();
        if let Some((tool, _)) = self.tools(&boxes).into_iter().find(|(_, r)| r.contains(at)) {
            self.use_tool(tool);
            return true;
        }
        if boxes.clock.contains(at) {
            self.clock.click();
            return true;
        }
        if boxes.notes.contains(at) {
            self.begin_writing();
            return true;
        }
        if boxes.next.contains(at) && self.cur + 1 < self.deck.pages {
            self.step(1);
            return true;
        }
        if let Some((i, _)) = self.strip(&boxes).into_iter().find(|(_, r)| r.contains(at)) {
            self.cur = i;
            return true;
        }
        false
    }

    /// The tools along the top, and where each is, as many as fit.
    pub(super) fn tools(&self, boxes: &Boxes) -> Vec<(Tool, Rect)> {
        let bar = boxes.bar;
        // How to quit is kept, at the right end, whatever else fits.
        let quit = Tool::Quit.width().min(bar.width);
        let end = bar.right() - quit;
        let mut x = bar.x;
        let mut out = Vec::new();
        for tool in Tool::ALL.into_iter().filter(|&t| t != Tool::Quit) {
            let width = tool.width();
            if x + width > end {
                break;
            }
            out.push((tool, Rect::new(x, bar.y, width, 1)));
            x += width + 1;
        }
        out.push((Tool::Quit, Rect::new(end, bar.y, quit, 1)));
        out
    }

    /// What a tool does, clicked: as its key does.
    fn use_tool(&mut self, tool: Tool) {
        match tool {
            Tool::Prev => self.step(-1),
            Tool::Next => self.step(1),
            Tool::Black => self.black = !self.black,
            Tool::Laser => self.toggle_hover(),
            Tool::Notes => self.begin_writing(),
            Tool::Pause => self.clock.pause(),
            Tool::Restart => self.clock.reset(),
            Tool::Grid => self.view = View::Grid,
            Tool::Window => self.open_audience(),
            Tool::End => self.toggle_stage(),
            Tool::Quit => self.notice = Some(format!("{QUIT_HINT}: press C-x, then C-c")),
        }
        self.dirty = true;
    }

    /// Whether a tool's mode is on, and the colour it is lit in then.
    fn lit(&self, tool: Tool) -> Option<Color> {
        match tool {
            Tool::Black if self.black => Some(Color::Red),
            Tool::Laser if self.hover || self.laser.is_some() => Some(Color::Red),
            Tool::Pause if self.clock.paused() => Some(Color::Yellow),
            // No window for the audience: the way to open it, lit.
            Tool::Window if self.stage.as_ref().is_some_and(|s| s.peer.is_none()) => {
                Some(Color::Red)
            }
            Tool::Notes if self.writing.is_some() => Some(Color::Yellow),
            _ => None,
        }
    }

    fn draw_tools(&self, f: &mut Frame, boxes: &Boxes) {
        let buf = f.buffer_mut();
        buf.set_style(boxes.bar, Style::new().bg(Color::Rgb(30, 30, 46)));
        for (tool, at) in self.tools(boxes) {
            let (key, icon, word) = tool.label();
            let body = match self.lit(tool) {
                Some(on) => Style::new()
                    .fg(Color::Black)
                    .bg(on)
                    .add_modifier(Modifier::BOLD),
                None if tool == Tool::Quit => {
                    Style::new().fg(Color::Gray).bg(Color::Rgb(60, 60, 80))
                }
                None => Style::new()
                    .fg(Color::White)
                    .bg(Color::Rgb(70, 70, 110))
                    .add_modifier(Modifier::BOLD),
            };
            let line = Line::from(vec![
                Span::styled(format!(" {key} "), CAP),
                Span::styled(format!(" {icon} {word} "), body),
            ]);
            f.render_widget(Paragraph::new(line), at);
        }
    }

    pub(super) fn draw_stage(&self, f: &mut Frame) {
        let boxes = self.boxes();
        self.draw_tools(f, &boxes);
        let (lit, dim, red) = (LIT, DIM, RED);

        let paused = self.clock.paused();
        let face = clock_face(self.clock.elapsed());
        let (face_style, word) = if paused {
            (dim.add_modifier(Modifier::BOLD), "  paused (T)")
        } else {
            (Style::new().add_modifier(Modifier::BOLD), "")
        };
        let audience = match self.stage.as_ref().map(|s| s.peer.is_some()) {
            Some(true) => Span::styled("  audience ●", Style::new().fg(Color::Green)),
            _ => Span::styled("  no audience window: W opens it", red),
        };
        let line = Line::from(vec![
            Span::styled(format!(" {face}"), face_style),
            Span::styled(word, dim),
            audience,
        ]);
        let block = Block::bordered()
            .border_style(dim)
            .title(Span::styled(" time ", dim));
        let inner = block.inner(boxes.clock);
        f.render_widget(block, boxes.clock);
        f.render_widget(Paragraph::new(line), inner);

        let next = self.cur + 1;
        let block = Block::bordered().border_style(dim);
        let block = if next < self.deck.pages {
            block.title(Span::styled(format!(" next · {} ", next + 1), dim))
        } else {
            block.title(Span::styled(" the end ", dim))
        };
        let inner = block.inner(boxes.next);
        f.render_widget(block, boxes.next);
        if next < self.deck.pages {
            self.draw_slide(f, self.job(Key::new(next, inner)), inner);
        }

        if let Some(writing) = &self.writing {
            let file = crate::render::name(&writing.file);
            let block = Block::bordered()
                .border_type(BorderType::Thick)
                .border_style(lit)
                .title(Span::styled(
                    format!(" notes · slide {} → {file} ", writing.slide + 1),
                    lit,
                ))
                .title_bottom(Span::styled(" esc saves · C-g drops ", dim));
            let inner = block.inner(boxes.notes);
            f.render_widget(block, boxes.notes);
            // Scrolled, so the cursor's row is in the box.
            let row = writing.cursor_row(inner.width);
            let scroll = row.saturating_sub(usize::from(inner.height.saturating_sub(1)));
            let scroll = u16::try_from(scroll).unwrap_or(u16::MAX);
            f.render_widget(
                Paragraph::new(writing.lines(inner.width)).scroll((scroll, 0)),
                inner,
            );
        } else {
            self.draw_notes(f, &boxes);
        }
        for (i, rect) in self.strip(&boxes) {
            self.draw_cell(f, i, rect);
        }

        // Last, so it is the slide noted as drawn, which clicks read from.
        let title = format!(" {} / {} ", self.cur + 1, self.deck.pages);
        let mut spans = vec![Span::styled(title, lit)];
        if self.black {
            spans.push(Span::styled(" audience sees black (b) ", red));
        }
        let block = Block::bordered()
            .border_type(BorderType::Thick)
            .border_style(if self.black { red } else { lit })
            .title(Line::from(spans));
        let inner = block.inner(boxes.current);
        f.render_widget(block, boxes.current);
        let job = self.job(Key::new(self.cur, inner));
        if let Some(drawn) = self.draw_slide(f, job, inner) {
            self.shown.set(Some(drawn));
        }
        self.draw_laser(f);
    }

    /// The notes box: beamer's note for the slide, and the notes written.
    fn draw_notes(&self, f: &mut Frame, boxes: &Boxes) {
        let block = Block::bordered()
            .border_style(DIM)
            .title(Span::styled(" notes · N or a click writes ", DIM));
        let mut inner = block.inner(boxes.notes);
        f.render_widget(block, boxes.notes);
        if self.note_pages {
            let at = self.note_box(boxes);
            self.draw_slide(f, self.job(Key::new(self.deck.pages + self.cur, at)), at);
            inner.y += at.height;
            inner.height -= at.height;
        }
        let text = match self.notes.get(&self.cur) {
            Some(note) => Paragraph::new(shown_markdown(note)),
            None if self.note_pages => Paragraph::new(""),
            None => {
                let (source, page) = self.page_in_file(self.cur);
                let file = crate::render::name(&notes::file_for(&self.deck.paths[source]));
                Paragraph::new(format!(
                    "No notes for this slide. N writes them, into {file} under a heading # {}; or write ::: notes in markdown.",
                    page + 1
                ))
                .style(DIM)
            }
        };
        f.render_widget(text.wrap(Wrap { trim: false }), inner);
    }

    /// The laser pointer, where it is, over the current slide as drawn.
    fn draw_laser(&self, f: &mut Frame) {
        if let Some(at) = self.laser
            && let Some((job, area)) = self.drawn.get()
            && job.key.page == self.cur
        {
            laser(f, area, at);
        }
    }

    /// The audience's window: the slide alone, or black.
    pub(super) fn draw_seat(&self, f: &mut Frame) {
        let area = f.area();
        f.render_widget(Block::new().style(Style::new().bg(Color::Black)), area);
        if !self.black {
            let job = self.job(self.presented());
            let shown = self.draw_slide(f, job, self.main);
            self.shown.set(shown.or(self.shown.get()));
            self.draw_laser(f);
        }
        let Some(seat) = &self.seat else {
            return;
        };
        if let Some(notice) = &self.notice {
            let width = (notice.chars().count() as u16 + 2).min(area.width);
            let at = Rect::new(area.x, area.bottom().saturating_sub(1), width, 1);
            f.render_widget(Clear, at);
            f.render_widget(
                Paragraph::new(format!(" {notice} ")).style(Style::new().fg(Color::Yellow)),
                at,
            );
        }
        if seat.hint.elapsed() < HINT {
            let text = "ohp, the audience's window · drag it to the projector and make it full screen · keys here turn the slides too · C-x C-c closes it";
            let width = (text.chars().count() as u16 + 4).min(area.width);
            let at = centred(
                Rect::new(area.x, area.bottom().saturating_sub(4), area.width, 3),
                Size::new(width, 3),
            );
            let block = Block::bordered().border_style(Style::new().fg(Color::Yellow));
            f.render_widget(Clear, at);
            f.render_widget(Paragraph::new(text).centered().block(block), at);
        }
    }
}

/// Put `text` on the clipboard, where there is a program here to; true if
/// it is there.
fn copy(text: &str) -> bool {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let programs: [(&str, &[&str]); 3] = [
        ("pbcopy", &[]),
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
    ];
    programs.iter().any(|(program, args)| {
        let Ok(mut child) = Command::new(program)
            .args(*args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            return false;
        };
        let written = child
            .stdin
            .take()
            .is_some_and(|mut input| input.write_all(text.as_bytes()).is_ok());
        child.wait().is_ok_and(|status| status.success()) && written
    })
}

/// A slide's notes being written, as the presenter practises.
pub(super) struct Writing {
    /// The deck's slide they are for, and its page in its own file.
    pub slide: usize,
    page: usize,
    /// The notes file they are saved into.
    pub file: std::path::PathBuf,
    text: String,
    /// The cursor, in chars.
    at: usize,
}

impl Writing {
    fn chars(&self) -> Vec<char> {
        self.text.chars().collect()
    }

    fn byte(&self, c: usize) -> usize {
        self.text
            .char_indices()
            .nth(c)
            .map_or(self.text.len(), |(i, _)| i)
    }

    fn insert(&mut self, c: char) {
        let at = self.byte(self.at);
        self.text.insert(at, c);
        self.at += 1;
    }

    fn delete(&mut self) {
        if self.at < self.chars().len() {
            let at = self.byte(self.at);
            self.text.remove(at);
        }
    }

    /// Delete to the end of the line, or the line's end itself if there.
    fn kill(&mut self) {
        let chars = self.chars();
        let end = (self.at..chars.len())
            .find(|&i| chars[i] == '\n')
            .unwrap_or(chars.len());
        let end = if end == self.at {
            (end + 1).min(chars.len())
        } else {
            end
        };
        let (a, b) = (self.byte(self.at), self.byte(end));
        self.text.replace_range(a..b, "");
    }

    fn left(&mut self) {
        self.at = self.at.saturating_sub(1);
    }

    fn right(&mut self) {
        self.at = (self.at + 1).min(self.chars().len());
    }

    /// Where the cursor's line begins, and how far into it the cursor is.
    fn line(&self) -> (usize, usize) {
        let chars = self.chars();
        let start = chars[..self.at]
            .iter()
            .rposition(|&c| c == '\n')
            .map_or(0, |i| i + 1);
        (start, self.at - start)
    }

    fn home(&mut self) {
        self.at = self.line().0;
    }

    fn end(&mut self) {
        let chars = self.chars();
        self.at = (self.at..chars.len())
            .find(|&i| chars[i] == '\n')
            .unwrap_or(chars.len());
    }

    fn up(&mut self) {
        let (start, col) = self.line();
        if start == 0 {
            self.at = 0;
            return;
        }
        self.at = start - 1;
        let (above, _) = self.line();
        self.at = (above + col).min(start - 1);
    }

    fn down(&mut self) {
        let (_, col) = self.line();
        let chars = self.chars();
        let Some(end) = (self.at..chars.len()).find(|&i| chars[i] == '\n') else {
            self.at = chars.len();
            return;
        };
        let below = end + 1;
        let below_end = (below..chars.len())
            .find(|&i| chars[i] == '\n')
            .unwrap_or(chars.len());
        self.at = (below + col).min(below_end);
    }

    /// The text as rows `width` cells wide, broken between words where a
    /// line is too long: each the chars, by index, it shows.
    fn rows(&self, width: u16) -> Vec<std::ops::Range<usize>> {
        let width = usize::from(width.max(1));
        let chars = self.chars();
        let mut rows = Vec::new();
        let mut start = 0;
        loop {
            let end = (start..chars.len())
                .find(|&i| chars[i] == '\n')
                .unwrap_or(chars.len());
            // One line, as rows.
            let mut from = start;
            while end - from > width {
                let cut = (from + 1..=from + width)
                    .rev()
                    .find(|&i| chars[i - 1] == ' ')
                    .unwrap_or(from + width);
                rows.push(from..cut);
                from = cut;
            }
            rows.push(from..end);
            if end == chars.len() {
                return rows;
            }
            start = end + 1;
        }
    }

    /// Put the cursor at column `x` of row `y` of the rows `width` wide.
    fn click(&mut self, x: u16, y: u16, width: u16) {
        let rows = self.rows(width);
        let Some(row) = rows.get(usize::from(y)).or(rows.last()) else {
            return;
        };
        self.at = (row.start + usize::from(x)).min(row.end);
    }

    /// The text as rows `width` wide, the cursor shown on the char it is
    /// before.
    /// The row, of those `width` wide, the cursor is on.
    pub(super) fn cursor_row(&self, width: u16) -> usize {
        let chars = self.chars();
        let rows = self.rows(width);
        let last = rows.len() - 1;
        rows.iter()
            .position(|row| {
                let ends_line = row.end == chars.len() || chars[row.end] == '\n';
                row.contains(&self.at) || (self.at == row.end && ends_line)
            })
            .unwrap_or(last)
    }

    pub(super) fn lines(&self, width: u16) -> Vec<Line<'static>> {
        let cursor = Style::new().add_modifier(Modifier::REVERSED);
        let chars = self.chars();
        let rows = self.rows(width);
        let last = rows.len() - 1;
        rows.into_iter()
            .enumerate()
            .map(|(n, row)| {
                let text = |r: std::ops::Range<usize>| chars[r].iter().collect::<String>();
                // On this row: before its end, or at it, on its last row, or
                // on a row a line ends after.
                let ends_line = row.end == chars.len() || chars[row.end] == '\n';
                let here =
                    row.contains(&self.at) || (self.at == row.end && (ends_line || n == last));
                if !here {
                    return Line::raw(text(row));
                }
                let mut spans = vec![Span::raw(text(row.start..self.at))];
                if self.at < row.end {
                    spans.push(Span::styled(chars[self.at].to_string(), cursor));
                    spans.push(Span::raw(text(self.at + 1..row.end)));
                } else {
                    spans.push(Span::styled(" ", cursor));
                }
                Line::from(spans)
            })
            .collect()
    }
}

/// A note, written in markdown, as lines to show: headings and bold in
/// bold, emphasis in italics, code in colour, lists with their bullets.
#[cfg(feature = "markdown")]
pub(super) fn shown_markdown(text: &str) -> Vec<Line<'static>> {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut style = Style::new();
    // The lists the text is in: the next number of each ordered one.
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut quoted = 0usize;
    let flush = |lines: &mut Vec<Line<'static>>, spans: &mut Vec<Span<'static>>| {
        if !spans.is_empty() {
            lines.push(Line::from(std::mem::take(spans)));
        }
    };
    let gap = |lines: &mut Vec<Line<'static>>| {
        if lines.last().is_some_and(|l| l.width() > 0) {
            lines.push(Line::raw(""));
        }
    };
    let lead = |quoted: usize| "│ ".repeat(quoted);
    for event in Parser::new(text) {
        match event {
            Event::Start(Tag::Heading { .. }) => {
                gap(&mut lines);
                style = style.add_modifier(Modifier::BOLD).fg(Color::Yellow);
            }
            Event::End(TagEnd::Heading(_)) => {
                flush(&mut lines, &mut spans);
                style = Style::new();
            }
            Event::Start(Tag::Paragraph) if lists.is_empty() => gap(&mut lines),
            Event::End(TagEnd::Paragraph) => flush(&mut lines, &mut spans),
            Event::Start(Tag::Strong) => style = style.add_modifier(Modifier::BOLD),
            Event::End(TagEnd::Strong) => style = style.remove_modifier(Modifier::BOLD),
            Event::Start(Tag::Emphasis) => style = style.add_modifier(Modifier::ITALIC),
            Event::End(TagEnd::Emphasis) => style = style.remove_modifier(Modifier::ITALIC),
            Event::Start(Tag::Strikethrough) => style = style.add_modifier(Modifier::CROSSED_OUT),
            Event::End(TagEnd::Strikethrough) => {
                style = style.remove_modifier(Modifier::CROSSED_OUT)
            }
            Event::Start(Tag::Link { .. }) => style = style.add_modifier(Modifier::UNDERLINED),
            Event::End(TagEnd::Link) => style = style.remove_modifier(Modifier::UNDERLINED),
            Event::Start(Tag::BlockQuote(_)) => quoted += 1,
            Event::End(TagEnd::BlockQuote(_)) => quoted = quoted.saturating_sub(1),
            Event::Start(Tag::List(first)) => {
                if lists.is_empty() {
                    gap(&mut lines);
                }
                flush(&mut lines, &mut spans);
                lists.push(first);
            }
            Event::End(TagEnd::List(_)) => {
                flush(&mut lines, &mut spans);
                lists.pop();
            }
            Event::Start(Tag::Item) => {
                flush(&mut lines, &mut spans);
                let indent = "  ".repeat(lists.len().saturating_sub(1));
                let bullet = match lists.last_mut() {
                    Some(Some(n)) => {
                        *n += 1;
                        format!("{}. ", *n - 1)
                    }
                    _ => "• ".into(),
                };
                spans.push(Span::raw(format!("{}{indent}{bullet}", lead(quoted))));
            }
            Event::End(TagEnd::Item) => flush(&mut lines, &mut spans),
            Event::Code(code) => {
                spans.push(Span::styled(code.to_string(), style.fg(Color::Cyan)));
            }
            Event::Start(Tag::CodeBlock(_)) => gap(&mut lines),
            Event::Text(t) if spans.is_empty() && quoted > 0 => {
                spans.push(Span::styled(lead(quoted), Style::new().fg(Color::DarkGray)));
                spans.push(Span::styled(
                    t.to_string(),
                    style.add_modifier(Modifier::ITALIC),
                ));
            }
            Event::Text(t) => {
                // A code block's lines, each a line.
                let mut parts = t.split('\n').peekable();
                while let Some(part) = parts.next() {
                    if !part.is_empty() {
                        spans.push(Span::styled(part.to_string(), style));
                    }
                    if parts.peek().is_some() {
                        flush(&mut lines, &mut spans);
                    }
                }
            }
            Event::SoftBreak => spans.push(Span::raw(" ")),
            Event::HardBreak => flush(&mut lines, &mut spans),
            Event::Rule => {
                flush(&mut lines, &mut spans);
                lines.push(Line::styled("────────", Style::new().fg(Color::DarkGray)));
            }
            Event::TaskListMarker(done) => {
                spans.push(Span::raw(if done { "[x] " } else { "[ ] " }));
            }
            _ => {}
        }
    }
    flush(&mut lines, &mut spans);
    lines
}

/// A note as it is written, ohp being built without markdown.
#[cfg(not(feature = "markdown"))]
pub(super) fn shown_markdown(text: &str) -> Vec<Line<'static>> {
    text.lines().map(|l| Line::raw(l.to_string())).collect()
}

#[cfg(test)]
#[path = "../tests/app/stage.rs"]
mod tests;

/// A laser pointer's dot at `at`, fractions of `area`, over what is drawn
/// there: a red blob two cells high and four wide, about round, as cells
/// are twice as high as wide, kept within `area`.
pub(super) fn laser(f: &mut Frame, area: Rect, (x, y): (f32, f32)) {
    if area.width < 4 || area.height < 2 {
        return;
    }
    let red = Color::Rgb(255, 30, 30);
    let cx = area.x + ((x * f32::from(area.width)) as u16).min(area.width - 1);
    let cy = area.y + ((y * f32::from(area.height)) as u16).min(area.height - 1);
    // The middle row, and halves of the rows above and below, rounding it.
    let rows: [(i32, [&str; 4]); 3] = [
        (-1, [" ", "▄", "▄", " "]),
        (0, ["█", "█", "█", "█"]),
        (1, [" ", "▀", "▀", " "]),
    ];
    let buf = f.buffer_mut();
    for (dy, cells) in rows {
        for (dx, symbol) in (-2i32..2).zip(cells) {
            if symbol == " " {
                continue;
            }
            let (px, py) = (i32::from(cx) + dx, i32::from(cy) + dy);
            let inside = px >= i32::from(area.x)
                && px < i32::from(area.right())
                && py >= i32::from(area.y)
                && py < i32::from(area.bottom());
            if inside && let Some(cell) = buf.cell_mut((px as u16, py as u16)) {
                cell.set_symbol(symbol).set_fg(red);
            }
        }
    }
}

/// A tool along the top of the presenter's view: a key, and a click.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Tool {
    Prev,
    Next,
    Black,
    Laser,
    Notes,
    Pause,
    Restart,
    Grid,
    Window,
    End,
    Quit,
}

impl Tool {
    const ALL: [Tool; 11] = [
        Tool::Prev,
        Tool::Next,
        Tool::Laser,
        Tool::Black,
        Tool::Notes,
        Tool::Pause,
        Tool::Restart,
        Tool::Grid,
        Tool::Window,
        Tool::End,
        Tool::Quit,
    ];

    /// Its key, an icon, and what it does.
    fn label(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Tool::Prev => ("p", "◀", "back"),
            Tool::Next => ("n", "▶", "next"),
            Tool::Laser => ("L", "●", "laser"),
            Tool::Black => ("b", "■", "black"),
            Tool::Notes => ("N", "✎", "notes"),
            Tool::Pause => ("T", "⏸", "pause"),
            Tool::Restart => ("Z", "↺", "restart"),
            Tool::Grid => ("g", "▦", "pick"),
            Tool::Window => ("W", "⧉", "window"),
            Tool::End => ("w", "✕", "end"),
            Tool::Quit => ("C-x C-c", "⏻", "quit"),
        }
    }

    fn width(self) -> u16 {
        let (key, icon, word) = self.label();
        let text = format!(" {key}  {icon} {word} ");
        unicode_width::UnicodeWidthStr::width(text.as_str()) as u16
    }
}
