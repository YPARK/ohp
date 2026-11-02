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
            Some(stage) if stage.peer.is_some() => {
                self.stage = None;
                self.black = false;
                // Its pages whole again, notes and all.
                if self.note_pages {
                    self.note_pages = false;
                    self.reload();
                }
                self.notice = Some("presenting to one window again".into());
            }
            Some(_) => self.open_audience(),
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
                Err(e) => self.notice = Some(format!("cannot present to a second window: {e}")),
            },
        }
        self.forget();
    }

    fn open_audience(&mut self) {
        let Some(stage) = &self.stage else {
            return;
        };
        self.notice = Some(match link::open_window(&stage.socket()) {
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
        self.note_pages = false;
        if self.stage.is_some() {
            let audience = self.cut_notes();
            if let Some(stage) = &self.stage
                && let Err(e) = stage.write(&audience)
            {
                self.stage = None;
                self.notice = Some(format!("cannot present to a second window: {e}"));
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
            Ok(cut) => {
                self.show_notes(cut.both);
                Arc::new(cut.slides)
            }
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
                self.deck = deck;
                self.renderer = renderer;
                self.note_pages = true;
                self.shown.set(None);
                self.forget();
            }
            Err(e) => self.notice = Some(format!("cannot show the notes: {e:#}")),
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
                    Said::Page(n) => self.cur = n.min(self.deck.pages - 1),
                    Said::Black(on) => self.black = on,
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
        let Some(stage) = &mut self.stage else {
            return;
        };
        if stage.arrived() {
            self.told = None;
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
                Said::Bye => {
                    if let Some(stage) = &mut self.stage {
                        stage.peer = None;
                    }
                    self.notice = Some("the audience's window closed: w opens it again".into());
                }
                Said::Deck(_) | Said::Page(_) | Said::Black(_) => continue,
            }
            self.dirty = true;
        }
        // Picking in the grid, the audience goes on seeing the slide shown.
        if self.view != View::Grid {
            self.live = self.cur;
        }
        let now = (self.live, self.black);
        if let Some(stage) = &mut self.stage
            && let Some(peer) = stage.peer.as_mut()
            && self.told != Some(now)
        {
            if peer.say(&Said::Page(now.0)) && peer.say(&Said::Black(now.1)) {
                self.told = Some(now);
            } else {
                stage.peer = None;
                self.notice = Some("the audience's window closed: w opens it again".into());
                self.dirty = true;
            }
        }
        // Each second, the clock is drawn again, and the notes files looked at.
        let second = self.clock.elapsed().as_secs();
        if self.ticked != second {
            self.ticked = second;
            self.dirty |= self.stage_view();
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
                self.notice = Some("C-x C-c closes this window".into());
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
            self.cur = self.live;
            self.step(by);
            self.live = self.cur;
        } else {
            self.step(by);
        }
    }

    /// `N`, or a click on the notes: write the current slide's notes, into the notes file beside
    /// the file it is from.
    fn begin_writing(&mut self) {
        let source = self.deck.source(self.cur);
        let file = notes::file_for(&self.deck.paths[source]);
        let page = self.cur - self.deck.starts()[source];
        let text = notes::written(&file, page);
        self.writing = Some(Writing {
            slide: self.cur,
            page,
            file,
            at: text.chars().count(),
            text,
        });
    }

    /// Save the notes being written, and stop writing.
    fn finish_writing(&mut self) {
        let Some(writing) = self.writing.take() else {
            return;
        };
        let saved = notes::save(&writing.file, writing.page, &writing.text);
        let name = writing.file.display().to_string();
        self.notice = Some(match saved {
            Ok(()) => format!("notes saved to {name}"),
            Err(e) => format!("cannot save the notes to {name}: {e}"),
        });
        self.read_notes();
    }

    /// A click while writing notes: in them, the cursor goes there;
    /// elsewhere, they are saved, as esc saves them.
    pub(super) fn writing_click(&mut self, at: Position) {
        let inner = Block::bordered().inner(self.boxes().notes);
        match &mut self.writing {
            Some(writing) if inner.contains(at) => {
                writing.click(at.x - inner.x, at.y - inner.y, inner.width);
            }
            Some(_) => self.finish_writing(),
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
            KeyCode::Esc => self.finish_writing(),
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
        use ratatui::crossterm::event::KeyCode;
        if !self.presenting() {
            return false;
        }
        match key.code {
            KeyCode::Char('b' | '.') => self.black = !self.black,
            KeyCode::Char('A' | 'P') if self.note_pages => {
                self.notice = Some(
                    "the notes are cut off the slides: w ends presenting, to print or annotate"
                        .into(),
                );
            }
            KeyCode::Char('T') => self.clock.pause(),
            KeyCode::Char('N') if self.stage_view() => self.begin_writing(),
            KeyCode::Char('Z') => self.clock.reset(),
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
    pub(super) fn stage_keys(&self) -> Vec<Key> {
        let boxes = self.boxes();
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
    pub(super) fn stage_sizes(&self) -> [Size; 4] {
        let boxes = self.boxes();
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

    /// A click in the presenter's view: on the strip, turn to that slide;
    /// on the next slide, turn to it; on the time, pause it or go on; on
    /// the notes, write them. True if it was one of those.
    pub(super) fn stage_click(&mut self, at: Position) -> bool {
        let boxes = self.boxes();
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

    pub(super) fn draw_stage(&self, f: &mut Frame) {
        let boxes = self.boxes();
        let lit = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);
        let dim = Style::new().fg(Color::DarkGray);
        let red = Style::new().fg(Color::Red).add_modifier(Modifier::BOLD);

        let paused = self.clock.paused();
        let face = clock_face(self.clock.elapsed());
        let (face_style, word) = if paused {
            (dim.add_modifier(Modifier::BOLD), "  paused (T)")
        } else {
            (Style::new().add_modifier(Modifier::BOLD), "")
        };
        let audience = match self.stage.as_ref().map(|s| s.peer.is_some()) {
            Some(true) => Span::styled("  audience ●", Style::new().fg(Color::Green)),
            _ => Span::styled("  no audience window (w)", red),
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
            let file = writing
                .file
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into());
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
            f.render_widget(Paragraph::new(writing.lines(inner.width)), inner);
            return self.draw_strip_and_current(f, &boxes);
        }
        let block = Block::bordered()
            .border_style(dim)
            .title(Span::styled(" notes · N or a click writes ", dim));
        let inner = block.inner(boxes.notes);
        f.render_widget(block, boxes.notes);
        let mut inner = inner;
        if self.note_pages {
            let at = self.note_box(&boxes);
            self.draw_slide(f, self.job(Key::new(self.deck.pages + self.cur, at)), at);
            inner.y += at.height;
            inner.height -= at.height;
        }
        let text = match self.notes.get(&self.cur) {
            Some(note) => Paragraph::new(shown_markdown(note)),
            None if self.note_pages => Paragraph::new(""),
            None => {
                let file = &self.deck.paths[self.deck.source(self.cur)];
                let stem = file
                    .file_stem()
                    .map_or_else(String::new, |s| s.to_string_lossy().into());
                let page = self.cur + 1 - self.deck.starts()[self.deck.source(self.cur)];
                Paragraph::new(format!(
                    "No notes for this slide. Write them in {stem}.notes.md, under a heading # {page}, or in ::: notes in markdown."
                ))
                .style(dim)
            }
        };
        f.render_widget(text.wrap(Wrap { trim: false }), inner);
        self.draw_strip_and_current(f, &boxes);
    }

    fn draw_strip_and_current(&self, f: &mut Frame, boxes: &Boxes) {
        let boxes = *boxes;
        let lit = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);
        let dim = Style::new().fg(Color::DarkGray);
        let red = Style::new().fg(Color::Red).add_modifier(Modifier::BOLD);
        for (i, rect) in self.strip(&boxes) {
            let on = i == self.cur;
            let style = if on { lit } else { dim };
            let block = Block::bordered()
                .border_type(if on {
                    BorderType::Thick
                } else {
                    BorderType::Plain
                })
                .border_style(style)
                .title(Span::styled(format!(" {} ", i + 1), style));
            let inner = block.inner(rect);
            f.render_widget(block, rect);
            self.draw_slide(f, self.job(Key::new(i, inner)), inner);
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
    }

    /// The audience's window: the slide alone, or black.
    pub(super) fn draw_seat(&self, f: &mut Frame) {
        let area = f.area();
        f.render_widget(Block::new().style(Style::new().bg(Color::Black)), area);
        if !self.black {
            let job = self.job(self.presented());
            let shown = self.draw_slide(f, job, self.main);
            self.shown.set(shown.or(self.shown.get()));
        }
        let Some(seat) = &self.seat else {
            return;
        };
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
