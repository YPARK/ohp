//! Choosing the model summaries are written by, on `m`, in a list over the
//! slides: the models ohp suggests, then the GGUF files kept in Hugging
//! Face's cache, then, as `owner/repo` or `owner/repo/file.gguf` is typed,
//! that repo of the Hub's. Typing narrows the list as the file picker's
//! does. A model not yet kept is downloaded the next time a summary is
//! wanted, its progress beside it.

use crate::hub;
use crate::menu::{self, Menu, dim, rank};
use crate::summary::{self, SUGGESTED, Weights};
use ratatui::Frame;
use ratatui::crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::ListItem;

/// A model to pick: what it is called, what it is, and whether it is kept,
/// so needs no downloading.
#[derive(Clone, Debug, PartialEq)]
struct Entry {
    name: String,
    about: String,
    weights: Weights,
    kept: bool,
}

/// The model picker, its list open over the slides when picking.
#[derive(Default)]
pub struct Models {
    list: Option<List>,
}

struct List {
    menu: Menu,
    entries: Vec<Entry>,
    /// The entries the query names, best match first.
    shown: Vec<usize>,
    /// The repo the query names, if it names one not listed.
    typed: Option<Entry>,
    current: Weights,
}

impl Models {
    /// Open the list, `current` the model in use.
    pub fn open(&mut self, current: &Weights) {
        let cache = hub::cache();
        let kept = cache.as_deref().map(hub::kept).unwrap_or_default();
        self.list = Some(List::new(entries(&kept, current), current.clone()));
    }

    pub fn is_open(&self) -> bool {
        self.list.is_some()
    }

    /// Take `key`: the model picked, once one is.
    pub fn key(&mut self, key: KeyEvent) -> Option<Weights> {
        let list = self.list.as_mut()?;
        match list.menu.key(key, list.len()) {
            menu::Key::Typed(_) | menu::Key::Retyped => list.refresh(),
            menu::Key::Moved | menu::Key::Other => {}
            menu::Key::Quit => self.list = None,
            menu::Key::Enter => {
                let picked = list.picked()?.weights.clone();
                self.list = None;
                return Some(picked);
            }
        }
        None
    }

    pub fn draw(&self, f: &mut Frame, area: Rect) {
        if let Some(list) = &self.list {
            list.draw(f, area);
        }
    }
}

/// The models to pick from: those ohp suggests, then those in `kept`,
/// GGUF files of Hugging Face's cache as `hub::kept` gives them, then
/// `current`, if none of them is.
fn entries(kept: &[(String, String)], current: &Weights) -> Vec<Entry> {
    let is_kept = |repo: &str, file: &str| kept.iter().any(|(r, f)| r == repo && f == file);
    let mut entries: Vec<Entry> = (SUGGESTED.iter())
        .map(|model| Entry {
            name: model.name.into(),
            about: format!("{} · {:.1} GB · {}", model.repo, model.gb, model.about),
            weights: Weights::from(model),
            kept: is_kept(model.repo, model.file),
        })
        .collect();
    for (repo, file) in kept {
        let weights = Weights::Hub {
            repo: repo.clone(),
            file: Some(file.clone()),
        };
        let gguf = file.to_ascii_lowercase().ends_with(".gguf");
        if gguf && !entries.iter().any(|e| e.weights == weights) {
            entries.push(Entry {
                name: weights.name(),
                about: repo.clone(),
                weights,
                kept: true,
            });
        }
    }
    if !entries.iter().any(|e| e.weights == *current) {
        let about = match current {
            Weights::Hub { repo, .. } => repo.clone(),
            Weights::File(path) => path.display().to_string(),
        };
        entries.push(Entry {
            name: current.name(),
            about,
            weights: current.clone(),
            kept: matches!(current, Weights::File(_)),
        });
    }
    entries
}

impl List {
    fn new(entries: Vec<Entry>, current: Weights) -> Self {
        let mut list = List {
            menu: Menu::default(),
            entries,
            shown: Vec::new(),
            typed: None,
            current,
        };
        list.refresh();
        let at = (list.shown.iter()).position(|&i| list.entries[i].weights == list.current);
        list.menu.pick(at.or(Some(0)));
        list
    }

    fn len(&self) -> usize {
        self.shown.len() + usize::from(self.typed.is_some())
    }

    /// The entry picked: the repo typed comes first, if one is.
    fn picked(&self) -> Option<&Entry> {
        let at = self.menu.picked()?;
        match &self.typed {
            Some(typed) if at == 0 => Some(typed),
            Some(_) => Some(&self.entries[*self.shown.get(at - 1)?]),
            None => Some(&self.entries[*self.shown.get(at)?]),
        }
    }

    /// List the models the query names, best first, by name, then by what
    /// they are; and a repo of the Hub's, if the query names one not
    /// listed.
    fn refresh(&mut self) {
        let query = self.menu.query.to_lowercase();
        let mut ranked: Vec<(u8, usize)> = (self.entries.iter().enumerate())
            .filter_map(|(i, e)| {
                let by_name = rank(&e.name.to_lowercase(), &query);
                let by_about = || rank(&e.about.to_lowercase(), &query).map(|r| r + 3);
                Some((by_name.or_else(by_about)?, i))
            })
            .collect();
        ranked.sort_by_key(|&(rank, _)| rank);
        self.shown = ranked.into_iter().map(|(_, i)| i).collect();
        self.typed = summary::hub_named(&self.menu.query)
            .filter(|w| !self.entries.iter().any(|e| e.weights == *w))
            .map(|weights| Entry {
                name: self.menu.query.trim().into(),
                about: "from Hugging Face".into(),
                weights,
                kept: false,
            });
        self.menu.pick((self.len() > 0).then_some(0));
    }

    fn draw(&self, f: &mut Frame, area: Rect) {
        let prompt = vec![Span::styled(
            " model ",
            Style::new().add_modifier(Modifier::BOLD),
        )];
        let entries: Vec<&Entry> = (self.typed.iter())
            .chain(self.shown.iter().map(|&i| &self.entries[i]))
            .collect();
        let width = (entries.iter())
            .map(|e| e.name.chars().count())
            .max()
            .unwrap_or(0);
        let items = entries.into_iter().map(|entry| {
            let mark = if entry.weights == self.current {
                " ● "
            } else {
                "   "
            };
            let (about, style) = if entry.kept {
                (entry.about.clone(), Style::new())
            } else {
                (format!("{} · download", entry.about), dim())
            };
            ListItem::new(Line::from(vec![
                Span::raw(format!("{mark}{:width$}  ", entry.name)),
                Span::styled(about, dim()),
            ]))
            .style(style)
        });
        let mut notes = Vec::new();
        if self.len() == 0 {
            notes.push(Span::styled(
                " no match: type a repo of Hugging Face's as owner/repo ",
                dim(),
            ));
        }
        let keys = "type to narrow, or owner/repo · ↑/↓ move · enter sum up with it · esc cancel";
        self.menu.draw(f, area, prompt, items, notes, keys);
    }
}

#[cfg(test)]
#[path = "tests/models.rs"]
mod tests;
