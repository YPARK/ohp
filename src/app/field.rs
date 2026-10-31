//! A line being typed in the status line: a signature's text, the copy's
//! name, the pages to print.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A line being typed: its text, the cursor, in chars, and what is picked,
/// typed over, if anything is.
#[derive(Clone, Debug, Default, PartialEq)]
pub(in crate::app) struct Field {
    pub(in crate::app) text: String,
    at: usize,
    picked: Option<(usize, usize)>,
}

impl Field {
    /// `text`, all of it picked, to be typed over or kept.
    pub(in crate::app) fn offered(text: &str) -> Self {
        Field::picking(text, 0, text.chars().count())
    }

    /// `text`, the chars from `from` to `to` picked.
    pub(in crate::app) fn picking(text: &str, from: usize, to: usize) -> Self {
        Field {
            text: text.to_string(),
            at: to,
            picked: Some((from, to)),
        }
    }

    fn byte(&self, c: usize) -> usize {
        self.text
            .char_indices()
            .nth(c)
            .map_or(self.text.len(), |(i, _)| i)
    }

    /// Cut what is picked; whether anything was.
    fn cut(&mut self) -> bool {
        let Some((a, b)) = self.picked.take() else {
            return false;
        };
        let (from, to) = (self.byte(a), self.byte(b));
        self.text.replace_range(from..to, "");
        self.at = a;
        true
    }

    pub(in crate::app) fn key(&mut self, key: KeyEvent) {
        let len = self.text.chars().count();
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Char('u') if ctrl => *self = Field::default(),
            KeyCode::Char(c) if !ctrl && !alt => {
                self.cut();
                let i = self.byte(self.at);
                self.text.insert(i, c);
                self.at += 1;
            }
            KeyCode::Backspace => {
                if !self.cut() && self.at > 0 {
                    self.at -= 1;
                    let i = self.byte(self.at);
                    self.text.remove(i);
                }
            }
            KeyCode::Delete => {
                if !self.cut() && self.at < len {
                    let i = self.byte(self.at);
                    self.text.remove(i);
                }
            }
            KeyCode::Left => {
                self.at = self
                    .picked
                    .take()
                    .map_or(self.at.saturating_sub(1), |p| p.0)
            }
            KeyCode::Right => self.at = self.picked.take().map_or((self.at + 1).min(len), |p| p.1),
            KeyCode::Home => (self.at, self.picked) = (0, None),
            KeyCode::End => (self.at, self.picked) = (len, None),
            _ => {}
        }
    }

    /// The text, what is picked in brackets, else the cursor shown.
    pub(in crate::app) fn shown(&self) -> String {
        let chars: Vec<char> = self.text.chars().collect();
        let part = |a: usize, b: usize| chars[a..b].iter().collect::<String>();
        match self.picked {
            Some((a, b)) => format!("{}[{}]{}", part(0, a), part(a, b), part(b, chars.len())),
            None => format!("{}▏{}", part(0, self.at), part(self.at, chars.len())),
        }
    }
}
