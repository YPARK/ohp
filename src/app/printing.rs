//! Printing what is shown, as `P` or ctrl-P begins it: the pages, copies,
//! sides and printer picked in a form beside the slides, and the PDF handed to `lp`, as
//! `print` has it. What is printed is the deck's PDF as it is shown: a
//! PDF being signed, the copy with all that is put.

use super::App;
use super::draw::{row_line, side_block};
use super::field::Field;
use crate::print::{self, Job, Scale, Sides};
use crate::render::{self, Key};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Margin, Position, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Paragraph};
use std::ffi::{OsStr, OsString};
use std::sync::mpsc::{Receiver, channel};

/// Most copies `→` asks for.
const MOST_COPIES: u32 = 99;

/// The form's rows, in order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Row {
    Pages,
    Copies,
    Sides,
    Scale,
    Printer,
}

impl Row {
    const ALL: [Row; 5] = [
        Row::Pages,
        Row::Copies,
        Row::Sides,
        Row::Scale,
        Row::Printer,
    ];

    fn name(self) -> &'static str {
        match self {
            Row::Pages => "pages",
            Row::Copies => "copies",
            Row::Sides => "sides",
            Row::Scale => "scale",
            Row::Printer => "printer",
        }
    }
}

/// The form's first row, below the top of the column: past its border and
/// a blank line.
const FORM_TOP: u16 = 2;

/// What is picked to print.
pub(super) struct Printing {
    /// The printers, the default first; none if `lpstat` lists none, as
    /// where it is not, and `lp`'s default is used.
    pub(super) printers: Vec<String>,
    printer: usize,
    pub(super) pages: Field,
    copies: u32,
    sides: Sides,
    scale: Scale,
    /// The paper the printer picked is set to, its name and size, if CUPS
    /// says.
    pub(super) paper: Option<(String, (f32, f32))>,
    /// Which of the pages picked the preview shows, from 0.
    pub(super) sheet: usize,
    /// The row the keys change.
    pub(super) row: Row,
    /// What prints, the first there is: `lp`, then `lpr`.
    pub(super) programs: Vec<OsString>,
    /// The printers, or the paper of the one picked, being asked of CUPS
    /// off the screen's thread: browsing the network for printers can
    /// take seconds.
    pub(super) lookup: Option<Receiver<Lookup>>,
}

/// What CUPS says: the printers, when they were asked for, and the paper
/// of the first, or of the one asked about.
pub(super) struct Lookup {
    pub(super) printers: Option<Vec<String>>,
    pub(super) paper: Option<(String, (f32, f32))>,
}

/// Ask CUPS, off the screen's thread, for the paper of `printer`; with
/// none, for the printers and the paper of the first.
fn look_up(printer: Option<String>) -> Receiver<Lookup> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let printers = printer.is_none().then(print::printers);
        let asked = printer.or_else(|| printers.as_ref()?.first().cloned());
        let paper = print::paper_of(asked.as_deref());
        let _ = tx.send(Lookup { printers, paper });
    });
    rx
}

impl App {
    /// Begin picking what to print.
    pub(super) fn print(&mut self) {
        // The copy as it is sized, though the wheel has not rested.
        self.settle_wheel(true);
        self.printing = Some(Printing {
            lookup: Some(look_up(None)),
            paper: None,
            printers: Vec::new(),
            printer: 0,
            pages: Field::offered("all"),
            copies: 1,
            sides: Sides::Printer,
            scale: Scale::Auto,
            sheet: 0,
            row: Row::Pages,
            programs: vec!["lp".into(), "lpr".into()],
        });
    }

    /// A key while picking what to print; whether it was printing's.
    pub(super) fn print_key(&mut self, key: KeyEvent) -> bool {
        let last = self.preview().map_or(0, |v| v.of.saturating_sub(1));
        let Some(p) = &mut self.printing else {
            return false;
        };
        match key.code {
            KeyCode::Enter => self.send(),
            KeyCode::Esc => {
                self.printing = None;
                self.notice = Some("not printed".into());
            }
            KeyCode::Down | KeyCode::Tab => p.row = cycle(&Row::ALL, p.row, true),
            KeyCode::Up | KeyCode::BackTab => p.row = cycle(&Row::ALL, p.row, false),
            KeyCode::Left | KeyCode::Right if p.row != Row::Pages => {
                p.change(key.code == KeyCode::Right);
            }
            KeyCode::Char(' ') if p.row != Row::Pages => p.change(true),
            // Not in a page range, so never typed in one.
            KeyCode::Char('n') | KeyCode::PageDown => p.sheet = (p.sheet + 1).min(last),
            KeyCode::Char('p') | KeyCode::PageUp => {
                p.sheet = p.sheet.min(last).saturating_sub(1);
            }
            _ if p.row == Row::Pages => p.pages.key(key),
            _ => {}
        }
        true
    }

    /// A click while picking what to print: on a row of the form, that row
    /// picked; whether it was on the form.
    pub(super) fn print_click(&mut self, at: Position) -> bool {
        let side = self.side.get();
        let Some(p) = &mut self.printing else {
            return false;
        };
        if !side.contains(at) {
            return false;
        }
        let row = at.y.checked_sub(side.y + FORM_TOP).map(usize::from);
        if let Some(&row) = row.and_then(|r| Row::ALL.get(r)) {
            match p.row == row && row != Row::Pages {
                true => p.change(true),
                false => p.row = row,
            }
        }
        true
    }

    /// What CUPS has said since asked, taken in.
    pub(super) fn found_printers(&mut self) {
        let Some(p) = &mut self.printing else {
            return;
        };
        let Some(found) = p.lookup.as_ref().and_then(|rx| rx.try_recv().ok()) else {
            return;
        };
        p.lookup = None;
        if let Some(printers) = found.printers {
            p.printers = printers;
        }
        p.paper = found.paper;
        self.dirty = true;
    }

    /// Print what is picked; with pages not there, say so and stay.
    fn send(&mut self) {
        let Some(p) = &self.printing else {
            return;
        };
        let pages = match print::pages(&p.pages.text, self.cur, self.deck.pages) {
            Ok(pages) => pages,
            Err(e) => {
                self.notice = Some(e);
                return;
            }
        };
        let job = Job {
            printer: p.printers.get(p.printer).cloned(),
            copies: p.copies,
            pages,
            sides: p.sides,
            scale: p.scale,
            title: render::name(&self.deck.paths[0]),
        };
        let programs: Vec<&OsStr> = p.programs.iter().map(OsString::as_os_str).collect();
        self.notice = Some(match print::print_with(&programs, &self.deck.data, &job) {
            Ok(said) => said,
            Err(e) => format!("{e:#}"),
        });
        self.printing = None;
    }

    /// What the preview shows, while picking what to print: the page, as
    /// it is printed on the paper, in the slides' area.
    pub(super) fn preview(&self) -> Option<Preview> {
        let p = self.printing.as_ref()?;
        let total = self.deck.pages;
        let pages = match print::pages(&p.pages.text, self.cur, total) {
            Ok(pages) => print::picked(pages.as_deref(), total),
            Err(_) => vec![self.cur],
        };
        let at = p.sheet.min(pages.len().saturating_sub(1));
        let page = *pages.get(at)?;
        let size = *self.deck.sizes.get(page)?;
        let paper = p.paper.as_ref().map_or(size, |(_, size)| *size);
        let (sheet, [x, y, w, h]) = print::placed(size, paper, p.scale);
        // The sheet and the page on it, all of each, fitted in the area,
        // past a border round the sheet.
        let area = self.main;
        let font = self.picker.font_size();
        let (fw, fh) = (f32::from(font.width), f32::from(font.height));
        let (left, top) = (x.min(0.), y.min(0.));
        let (wide, tall) = (sheet.0.max(x + w) - left, sheet.1.max(y + h) - top);
        let room = |cells: u16, size: f32| f32::from(cells.saturating_sub(2)) * size;
        let k = (room(area.width, fw) / wide).min(room(area.height, fh) / tall);
        let (ox, oy) = (
            f32::from(area.x) + (f32::from(area.width) - wide * k / fw) / 2.,
            f32::from(area.y) + (f32::from(area.height) - tall * k / fh) / 2.,
        );
        let cells = |x: f32, y: f32, w: f32, h: f32| {
            let (x0, y0) = (
                (ox + (x - left) * k / fw).round(),
                (oy + (y - top) * k / fh).round(),
            );
            let (x1, y1) = (
                (ox + (x + w - left) * k / fw).round(),
                (oy + (y + h - top) * k / fh).round(),
            );
            Rect::new(
                x0 as u16,
                y0 as u16,
                (x1 - x0).max(1.) as u16,
                (y1 - y0).max(1.) as u16,
            )
        };
        let paper = cells(0., 0., sheet.0, sheet.1);
        let on = cells(x, y, w, h);
        Some(Preview {
            key: Key::new(page, on),
            page: on,
            sheet: paper.outer(Margin::new(1, 1)),
            at,
            of: pages.len(),
        })
    }

    /// The preview, in `area`: the sheet, its paper named, the page on it.
    pub(super) fn draw_preview(&self, f: &mut Frame, area: Rect) {
        let (Some(p), Some(preview)) = (&self.printing, self.preview()) else {
            return;
        };
        let paper = p
            .paper
            .as_ref()
            .map_or("paper not known".into(), |(name, _)| name.clone());
        let title = format!(
            " {paper} · {} · page {} · {} of {} ",
            p.value(Row::Scale),
            preview.key.page + 1,
            preview.at + 1,
            preview.of
        );
        let sheet = preview.sheet.intersection(area);
        f.render_widget(Block::bordered().title(title), sheet);
        // The page's edge, just outside it, where it is not the sheet's.
        let edge = preview.page.outer(Margin::new(1, 1));
        if edge != preview.sheet {
            let dim = Style::new().fg(Color::DarkGray);
            f.render_widget(Block::bordered().border_style(dim), edge.intersection(area));
        }
        self.draw_slide(f, self.job(preview.key), preview.page);
    }

    /// What the status line says while picking what to print, and the keys.
    pub(super) fn print_status(&self) -> Option<(String, &'static str)> {
        self.printing.as_ref()?;
        Some((
            "print".into(),
            "↑/↓ row · ←/→ change · type pages · n/p preview · enter print · esc cancel",
        ))
    }

    /// The form, in the column beside the slides: a row each for the
    /// pages, copies, sides and printer, the one the keys change lit.
    pub(super) fn draw_print(&self, f: &mut Frame, area: Rect) {
        let Some(p) = &self.printing else {
            return;
        };
        let dim = Style::new().fg(Color::DarkGray);
        let block = side_block(" print ");
        // A row each, cut to the column: past the border, the marker and
        // the row's name, and the arrows.
        let room = usize::from(block.inner(area).width).saturating_sub(15);
        let mut lines = vec![Line::raw("")];
        for row in Row::ALL {
            let value = match row {
                Row::Pages => format!("{} of {}", p.value(row), self.deck.pages),
                _ => format!("◀ {} ▶", cut(&p.value(row), room)),
            };
            lines.push(row_line(
                row == p.row,
                &format!("{:<8} {value}", row.name()),
            ));
        }
        lines.push(Line::raw(""));
        for help in [
            "pages: all, this, 3-7, 9",
            "",
            "↑/↓ row · ←/→ change",
            "n/p turn the preview",
            "enter print · esc cancel",
        ] {
            lines.push(Line::styled(help, dim));
        }
        let form = Paragraph::new(lines).block(block);
        f.render_widget(form, area);
    }
}

impl Printing {
    /// Change the value of the row picked: the next, `up`, or the last.
    fn change(&mut self, up: bool) {
        match self.row {
            Row::Pages => {}
            Row::Copies if up => self.copies = (self.copies + 1).min(MOST_COPIES),
            Row::Copies => self.copies = (self.copies - 1).max(1),
            Row::Sides => self.sides = cycle(&Sides::ALL, self.sides, up),
            Row::Scale => self.scale = cycle(&Scale::ALL, self.scale, up),
            Row::Printer => {
                self.printer = step(self.printer, self.printers.len(), up);
                let printer = self.printers.get(self.printer).cloned();
                self.paper = None;
                self.lookup = Some(look_up(printer));
            }
        }
    }

    /// What `row` is set to, as the form shows it.
    pub(super) fn value(&self, row: Row) -> String {
        match row {
            Row::Pages => self.pages.shown(),
            Row::Copies => self.copies.to_string(),
            Row::Sides => match self.sides {
                Sides::Printer => "as the printer is set",
                Sides::One => "one-sided",
                Sides::Two => "two-sided",
            }
            .into(),
            Row::Scale => match self.scale {
                Scale::Auto => "shrink to fit",
                Scale::Fit => "fit to paper",
                Scale::Actual => "actual size",
            }
            .into(),
            Row::Printer => match self.printers.get(self.printer) {
                Some(name) if self.printer == 0 => format!("{name} (default)"),
                Some(name) => name.clone(),
                None if self.lookup.is_some() => "finding printers…".into(),
                None => "the default printer".into(),
            },
        }
    }
}

/// `text`, cut to `most` chars, `…` ending it if it is cut.
fn cut(text: &str, most: usize) -> String {
    match text.chars().count() > most {
        true => text
            .chars()
            .take(most.saturating_sub(1))
            .chain(['…'])
            .collect(),
        false => text.to_string(),
    }
}

/// The preview: the page, at the size of its box on the sheet, and where
/// that box is; the sheet, its border round it; and which of how many
/// pages picked it is.
pub(super) struct Preview {
    pub(super) key: Key,
    pub(super) page: Rect,
    pub(super) sheet: Rect,
    pub(super) at: usize,
    pub(super) of: usize,
}

/// The index after `at` of `len`, `up`, or before it, round the end.
fn step(at: usize, len: usize, up: bool) -> usize {
    let len = len.max(1);
    match up {
        true => (at + 1) % len,
        false => (at + len - 1) % len,
    }
}

/// The one of `all` after `now`, `up`, or before it, round the end.
fn cycle<T: Copy + PartialEq>(all: &[T], now: T, up: bool) -> T {
    let at = all.iter().position(|&t| t == now).unwrap_or(0);
    all[step(at, all.len(), up)]
}
