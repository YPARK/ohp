//! Drawing the screen: the slides presented or in a grid, the summary
//! beside them, the status line under them, and a list over them.

use super::{App, Panel, View};
use crate::render::{self, Job, Key, Look, Slide, Zoom};

use crate::summary::{self, Summarizer};
use crate::text::{self, Picked};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect, Size};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};
use ratatui_image::Image;
use ratatui_image::picker::ProtocolType;
use ratatui_image::picker::cap_parser::Parser;
use std::io::Write;

impl App {
    pub(super) fn draw(&self, f: &mut Frame) {
        let (main, side, status) = split(f.area(), self.panel.is_some());
        self.draw_summary(f, side);
        self.drawn.set(None);
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
        self.voices.draw(f, inner);
        #[cfg(feature = "summary")]
        self.models.draw(f, inner);
    }

    /// Whether a list is open over the slides.
    pub(super) fn listing(&self) -> bool {
        #[cfg(feature = "speech")]
        if self.voices.is_open() {
            return true;
        }
        #[cfg(feature = "summary")]
        if self.models.is_open() {
            return true;
        }
        self.browser.is_some()
    }

    /// The slide `job` renders, in `area`, which is its key's size; until it
    /// is rendered, the same slide as last presented or fitted. Returns the
    /// job whose slide is drawn, noted as `drawn` with its cells when
    /// presenting.
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
        let unmarked = Job {
            marked: Picked::default(),
            ..unlit
        };
        let found = [
            Some(job),
            self.shown.get().filter(same),
            Some(fitted),
            Some(unlit),
            Some(unmarked),
        ]
        .into_iter()
        .flatten()
        .filter(|j| j.key.size() == job.key.size())
        .find_map(|j| Some((j, self.slides.get(&j).or_else(|| self.stale.get(&j))?)));
        let drawn = match found {
            Some((j, Some(Slide::Image(proto)))) => {
                let area = centred(area, proto.size());
                f.render_widget(Image::new(proto), area);
                Some((j, area))
            }
            Some((j, Some(Slide::Text { cells, backdrop }))) => {
                let area = centred(area, backdrop.size());
                f.render_widget(Image::new(backdrop), area);
                text::overlay(cells, area, f.buffer_mut());
                Some((j, area))
            }
            Some((_, None)) => {
                note(f, area, "cannot render this slide");
                None
            }
            None => {
                note(f, area, "rendering…");
                None
            }
        };
        if self.view == View::Present {
            self.drawn.set(drawn);
        }
        found.map(|(j, _)| j)
    }

    /// The summary shown, in `area` beside the slides: what it says, the
    /// key sentences marked, each in the colour it is marked with, and what
    /// is being done towards it.
    fn draw_summary(&self, f: &mut Frame, area: Rect) {
        let Some(panel) = self.panel else { return };
        let title = match panel {
            Panel::Slide => format!(" summary · {} {} ", self.slide_word(self.cur), self.cur + 1),
            Panel::Whole => " summary · whole file ".into(),
        };
        let view = self.summary(panel);
        let dim = Style::new().fg(Color::DarkGray);
        let block = Block::bordered().border_style(dim).title(Span::styled(
            title,
            Style::new().add_modifier(Modifier::BOLD),
        ));
        let mut lines: Vec<Line> = view.text.lines().map(Line::raw).collect();
        if !view.key.is_empty() {
            let [r, g, b] = text::MARK;
            let mark = Style::new().fg(Color::Rgb(r, g, b));
            lines.push(Line::raw(""));
            lines.push(Line::styled(
                "Key takeaways",
                Style::new().add_modifier(Modifier::BOLD),
            ));
            for (page, sentence) in &view.key {
                let mut line = vec![Span::styled("▌ ", mark)];
                if panel == Panel::Whole {
                    let at = format!("{} {} · ", self.slide_word(*page), page + 1);
                    line.push(Span::styled(at, dim));
                }
                line.push(Span::raw(sentence.as_str()));
                lines.push(Line::from(line));
            }
        }
        if let Some(busy) = &view.busy {
            if !lines.is_empty() {
                lines.push(Line::raw(""));
            }
            let width = block.inner(area).width;
            if let Some(bar) = bar(busy.done, width) {
                lines.push(bar);
            }
            lines.push(Line::styled(
                busy.what.clone(),
                dim.add_modifier(Modifier::ITALIC),
            ));
        }
        let summary = Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(block);
        f.render_widget(summary, area);
    }

    /// What to show of `panel`'s summary.
    pub(super) fn summary(&self, panel: Panel) -> summary::View {
        match panel {
            Panel::Slide => self.summarizer.page(self.cur),
            Panel::Whole => self.summarizer.whole(),
        }
    }

    /// What page `page` of the deck is called, by its shape: a slide, or a
    /// page of a document. A deck of several files may have both.
    fn slide_word(&self, page: usize) -> &'static str {
        match self.deck.sizes.get(page) {
            Some((w, h)) if w > h => "slide",
            _ => "page",
        }
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

    pub(super) fn status(&self) -> Line<'_> {
        let dim = Style::new().fg(Color::DarkGray);
        let zoom = format!("present {}%", self.zoom.percent);
        let (view, keys) = match self.view {
            View::Present if self.zoomed() => (
                zoom.as_str(),
                "arrows pan · +/- zoom · 0 fit · n/p next/prev · t text · s speak · v voice · k/K summary · m model · r reload · ^O open · q quit",
            ),
            View::Present => (
                "present",
                "n/p ←/→ next/prev · +/- zoom · g grid · t text · s speak, then click to speak from · v voice · k/K summary · m model · S sign · r reload · ^O open · q quit",
            ),
            View::Grid => (
                "grid",
                "n/p arrows click move · g/enter/double-click present · +/- zoom · t text · s speak · v voice · k/K summary · m model · r reload · ^O open · q quit",
            ),
        };
        let signing = self.sign_status();
        let (view, keys) = match &signing {
            Some((doing, keys)) => (doing.as_str(), *keys),
            None => (view, keys),
        };
        let look = match self.look {
            Look::Image => "",
            Look::Text => " · text",
        };
        let mut speaking = String::new();
        if self.speaker.reading() {
            speaking.push_str(" · speaking");
        }
        #[cfg(feature = "speech")]
        if let Some(note) = self.voices.note() {
            speaking.push_str(&note);
        }
        let busy = self.panel.and_then(|panel| self.summary(panel).busy);
        if let Some(done) = busy.and_then(|b| b.done) {
            speaking.push_str(&format!(" · summing up {:.0}%", done * 100.));
        }
        let paths = &self.deck.paths;
        let file = self.deck.source(self.cur);
        let mut spans = vec![
            Span::styled(
                format!(" {} ", render::name(&paths[file])),
                Style::new().add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(" {}/{} ", self.cur + 1, self.deck.pages)),
        ];
        if paths.len() > 1 {
            spans.push(Span::styled(
                format!(" file {}/{} ·", file + 1, paths.len()),
                dim,
            ));
        }
        spans.push(Span::styled(format!(" {view}{look}{speaking}"), dim));
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
                Some(paths) => format!("opening {}…", render::shown(paths)),
                None if !render::pdf(&self.deck.paths) => "typesetting…".into(),
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
    pub(super) fn clear_images(&self) -> anyhow::Result<()> {
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

/// A bar `width` cells wide, its percent at its end, as full as `done`
/// says, if that is known.
fn bar(done: Option<f32>, width: u16) -> Option<Line<'static>> {
    let done = done?.clamp(0., 1.);
    let cells = usize::from(width.saturating_sub(5));
    let full = (done * cells as f32).round() as usize;
    let percent = format!(" {:>3.0}%", done * 100.);
    Some(Line::from(vec![
        Span::styled("█".repeat(full), Style::new().fg(Color::Yellow)),
        Span::styled("░".repeat(cells - full), Style::new().fg(Color::DarkGray)),
        Span::raw(percent),
    ]))
}

/// The key sentences `summarizer` marks on `page` with `panel` shown.
pub(super) fn marked(summarizer: &Summarizer, panel: Option<Panel>, page: usize) -> Picked {
    match panel {
        Some(Panel::Slide) => summarizer.picked(page),
        Some(Panel::Whole) => summarizer.picked_whole(page),
        None => Picked::default(),
    }
}

/// Columns the summary beside the slides takes, at least and at most.
pub(super) const SIDE: (u16, u16) = (32, 64);

/// The slide area, the summary's column beside it if there is a summary,
/// and the status line under them.
pub(super) fn split(area: Rect, summary: bool) -> (Rect, Rect, Rect) {
    let [body, status] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);
    if !summary {
        return (body, Rect::default(), status);
    }
    let width = (body.width / 3).clamp(SIDE.0, SIDE.1).min(body.width / 2);
    let [main, side] =
        Layout::horizontal([Constraint::Min(1), Constraint::Length(width)]).areas(body);
    (main, side, status)
}

pub(super) fn centred(area: Rect, size: Size) -> Rect {
    let (w, h) = (size.width.min(area.width), size.height.min(area.height));
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

pub(super) fn note(f: &mut Frame, area: Rect, text: &str) {
    let mid = Rect::new(
        area.x,
        area.y + area.height / 2,
        area.width,
        1.min(area.height),
    );
    let text = Span::styled(text, Style::new().fg(Color::DarkGray));
    f.render_widget(Paragraph::new(text).centered(), mid);
}
