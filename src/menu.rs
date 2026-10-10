//! A list to pick from over the slides, narrowed by typing, as the file
//! picker's and the voice picker's: what is typed, which entry is picked,
//! the keys they share and how the list is drawn. What is listed, and what
//! picking does, is theirs.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use std::cell::Cell;

/// What a key did to the list.
pub enum Key {
    /// A character was typed at the end of the query.
    Typed(char),
    /// The query changed otherwise.
    Retyped,
    /// Another entry was picked, or nothing changed.
    Moved,
    /// The entry picked is to be taken.
    Enter,
    /// The list is to be closed.
    Quit,
    /// A key the list leaves to its owner, as backspace on an empty query.
    Other,
}

#[derive(Default)]
pub struct Menu {
    pub query: String,
    /// Set as it is drawn, as is `rows`.
    list: Cell<ListState>,
    /// Rows the list shows, for page up and down.
    pub rows: Cell<u16>,
}

impl Menu {
    /// Take `key` for a list of `len` entries.
    pub fn key(&mut self, key: KeyEvent, len: usize) -> Key {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('c') if ctrl => Key::Quit,
            KeyCode::Char('u') if ctrl => {
                self.query.clear();
                Key::Retyped
            }
            KeyCode::Char('n') if ctrl => self.step(1, len),
            KeyCode::Char('p') if ctrl => self.step(-1, len),
            KeyCode::Char(c) if !ctrl => {
                self.query.push(c);
                Key::Typed(c)
            }
            KeyCode::Down => self.step(1, len),
            KeyCode::Up => self.step(-1, len),
            KeyCode::PageDown => self.step(self.page(), len),
            KeyCode::PageUp => self.step(-self.page(), len),
            KeyCode::Backspace if !self.query.is_empty() => {
                self.query.pop();
                Key::Retyped
            }
            KeyCode::Enter => Key::Enter,
            KeyCode::Esc if self.query.is_empty() => Key::Quit,
            KeyCode::Esc => {
                self.query.clear();
                Key::Retyped
            }
            _ => Key::Other,
        }
    }

    /// Pick entry `at`, or none.
    pub fn pick(&self, at: Option<usize>) {
        let mut list = self.list.get();
        list.select(at);
        self.list.set(list);
    }

    pub fn picked(&self) -> Option<usize> {
        self.list.get().selected()
    }

    fn step(&self, by: isize, len: usize) -> Key {
        if len > 0 {
            let at = self.picked().unwrap_or(0);
            self.pick(Some(at.saturating_add_signed(by).min(len - 1)));
        }
        Key::Moved
    }

    fn page(&self) -> isize {
        isize::try_from(self.rows.get().saturating_sub(1).max(1)).unwrap_or(1)
    }

    /// Draw the list in `area`: `prompt` and the query on its first line,
    /// then `items`, then `notes` and the `keys` it takes.
    pub fn draw<'a>(
        &self,
        f: &mut Frame,
        area: Rect,
        prompt: Vec<Span<'a>>,
        items: impl IntoIterator<Item = ListItem<'a>>,
        mut notes: Vec<Span<'a>>,
        keys: &'a str,
    ) {
        let [top, list, status] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(area);
        self.rows.set(list.height);

        let mut prompt = prompt;
        prompt.push(Span::raw(self.query.as_str()));
        prompt.push(Span::styled("▏", Style::new().fg(Color::Yellow)));
        f.render_widget(Paragraph::new(Line::from(prompt)), top);

        let picked = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);
        let mut state = self.list.get();
        f.render_stateful_widget(List::new(items).highlight_style(picked), list, &mut state);
        self.list.set(state);

        notes.push(Span::styled(format!("  {keys}"), dim()));
        f.render_widget(Paragraph::new(Line::from(notes)), status);
    }
}

/// How what matters less is shown.
pub fn dim() -> Style {
    Style::new().fg(Color::DarkGray)
}

/// A note in the status line of what is wrong.
pub fn trouble(why: &str) -> Span<'_> {
    Span::styled(
        format!(" {why} "),
        Style::new().add_modifier(Modifier::BOLD),
    )
}

/// How well `stem` matches `name`, both lowercase, best lowest: `name`
/// starts with it, has it in it, or has its letters in order.
pub fn rank(name: &str, stem: &str) -> Option<u8> {
    if name.starts_with(stem) {
        return Some(0);
    }
    if name.contains(stem) {
        return Some(1);
    }
    let mut letters = name.chars();
    stem.chars().all(|c| letters.any(|n| n == c)).then_some(2)
}
