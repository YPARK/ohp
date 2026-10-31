//! Searching the slides for a word, as Emacs's isearch does in pdf-tools:
//! `ctrl-s` searches forward, `ctrl-r` back, each typed letter narrowing
//! what is found as it is typed, and turning to the first place it is
//! from where the search is. `ctrl-s` or `ctrl-r` again goes to the next
//! place, or the last, round the end to the other; on an empty query, they
//! search for the last searched again. What is found is lit, the place
//! gone to brighter. `enter` or `esc` stays there, and `ctrl-g` goes back
//! to where the search began; either way the query is kept, to search for
//! again. Any other key stays there and does its own.

use super::field::Field;
use super::{App, View};
use crate::render::{self, Found, Zoom};
use crate::text;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::sync::mpsc::channel;

/// A search under way.
pub(super) struct Search {
    pub(super) field: Field,
    /// The query as the renderer names it.
    query: u32,
    /// Where the search began, gone back to on `ctrl-g`: the slide, how it
    /// was zoomed, and the view.
    from: (usize, Zoom, View),
    /// The place gone to: the slide, and which of the places on it.
    pub(super) at: Option<(usize, usize)>,
    /// How many places each slide has the query in.
    counts: Vec<usize>,
    /// The last step went round the end to the other.
    wrapped: bool,
    /// Searching forward, or back.
    forward: bool,
}

impl App {
    /// Begin a search, `forward` or back, from the slide shown.
    fn begin_search(&mut self, forward: bool) {
        self.search = Some(Search {
            field: Field::default(),
            query: render::query_id(""),
            from: (self.cur, self.zoom, self.view),
            at: None,
            counts: Vec::new(),
            wrapped: false,
            forward,
        });
    }

    /// A key, searching, or `ctrl-s` or `ctrl-r` beginning a search;
    /// whether it was the search's.
    pub(super) fn search_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let step = match key.code {
            KeyCode::Char('s') if ctrl => Some(true),
            KeyCode::Char('r') if ctrl => Some(false),
            _ => None,
        };
        if self.search.is_none() {
            let Some(forward) = step else {
                return false;
            };
            self.begin_search(forward);
            self.read_slides();
            self.notice = Some(match forward {
                true => "search forward: type what to find".into(),
                false => "search back: type what to find".into(),
            });
            return true;
        }
        match (key.code, step) {
            (_, Some(forward)) => {
                let empty = self
                    .search
                    .as_ref()
                    .is_some_and(|s| s.field.text.is_empty());
                if empty && !self.last_query.is_empty() {
                    let last = self.last_query.clone();
                    self.retype(Field::offered(&last), forward);
                } else {
                    self.step_search(forward);
                }
            }
            (KeyCode::Char('g'), _) if ctrl => {
                if let Some(s) = &self.search {
                    (self.cur, self.zoom, self.view) = s.from;
                }
                self.end_search();
                self.notice = Some("search stopped".into());
            }
            (KeyCode::Enter | KeyCode::Esc, _) => self.end_search(),
            (KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Delete, _)
                if !ctrl || matches!(key.code, KeyCode::Char('u')) =>
            {
                let Some(s) = &self.search else {
                    return true;
                };
                let (mut field, forward) = (s.field.clone(), s.forward);
                field.key(key);
                self.retype(field, forward);
            }
            // Any other key ends the search where it is, and does its own.
            _ => {
                self.end_search();
                return false;
            }
        }
        true
    }

    /// The search done, staying where it went; its query kept, to search
    /// for again.
    fn end_search(&mut self) {
        if let Some(s) = self.search.take()
            && !s.field.text.trim().is_empty()
        {
            self.last_query = s.field.text;
        }
    }

    /// The query typed as `field` has it: what is found counted again, and
    /// the first place it is gone to, from the place gone to last, or where
    /// the search began, `forward` or back.
    fn retype(&mut self, field: Field, forward: bool) {
        let counts = self.counts(&field.text).unwrap_or_default();
        let Some(s) = &mut self.search else {
            return;
        };
        // The slide gone to, from its first place, or its last back: which
        // of the old query's places it was does not count the new one's.
        let page = s.at.map_or(s.from.0, |(page, _)| page);
        let from = if forward {
            (page, 0)
        } else {
            (page, usize::MAX)
        };
        s.query = render::query_id(&field.text);
        s.field = field;
        s.counts = counts;
        s.wrapped = false;
        s.at = place(&s.counts, from, forward, true);
        self.go_to_found();
    }

    /// Go to the next place, `forward`, or the last, round the end.
    fn step_search(&mut self, forward: bool) {
        let Some(s) = &mut self.search else {
            return;
        };
        let from = s.at.unwrap_or((s.from.0, 0));
        let next = place(&s.counts, from, forward, s.at.is_none());
        s.forward = forward;
        s.wrapped = match (s.at, next) {
            (Some(was), Some(now)) => (now < was) == forward || now == was,
            _ => false,
        };
        s.at = next.or(s.at);
        self.go_to_found();
    }

    /// Turn to the slide gone to: presented, or picked in the grid.
    fn go_to_found(&mut self) {
        if let Some((page, _)) = self.search.as_ref().and_then(|s| s.at) {
            self.cur = page;
        }
    }

    /// How many places each slide has `query` in; none while the slides'
    /// text is being read.
    fn counts(&self, query: &str) -> Option<Vec<usize>> {
        let texts = self.texts.as_ref()?;
        Some(texts.iter().map(|t| t.find(query).len()).collect())
    }

    /// Read the slides' text to search, off the screen's thread, once until
    /// the deck is read again: a long deck takes a while.
    pub(super) fn read_slides(&mut self) {
        if self.texts.is_some() || self.reading.is_some() {
            return;
        }
        let (tx, rx) = channel();
        let (data, pages) = (self.deck.data.clone(), self.deck.pages);
        std::thread::spawn(move || {
            let all: Vec<usize> = (0..pages).collect();
            let texts = text::of_each(data, &all).into_iter();
            let _ = tx.send(
                texts
                    .map(|t| t.map(|t| t.searchable()).unwrap_or_default())
                    .collect(),
            );
        });
        self.reading = Some(rx);
    }

    /// The slides' text, once read, taken in, and what is typed searched
    /// for in it.
    pub(super) fn read_texts(&mut self) {
        let Some(texts) = self.reading.as_ref().and_then(|rx| rx.try_recv().ok()) else {
            return;
        };
        (self.texts, self.reading) = (Some(texts), None);
        if let Some(s) = &self.search {
            let (field, forward) = (s.field.clone(), s.forward);
            self.retype(field, forward);
        }
        self.dirty = true;
    }

    /// What is found on slide `page`, to be lit, while searching.
    pub(super) fn found(&self, page: usize) -> Option<Found> {
        let s = self.search.as_ref()?;
        if s.counts.get(page).is_none_or(|&n| n == 0) {
            return None;
        }
        let current = s.at.filter(|&(p, _)| p == page).map(|(_, k)| k as u16);
        Some(Found {
            query: s.query,
            current,
        })
    }

    /// What the status line says while searching, and the keys.
    pub(super) fn search_status(&self) -> Option<(String, &'static str)> {
        let s = self.search.as_ref()?;
        let total: usize = s.counts.iter().sum();
        let typed = s.field.shown();
        let said = match s.at {
            _ if s.field.text.is_empty() => format!("search: {typed}"),
            _ if self.texts.is_none() => format!("search: {typed} · reading the slides…"),
            None => format!("failing search: {typed}"),
            Some((page, k)) => {
                let before: usize = s.counts[..page].iter().sum();
                let wrapped = if s.wrapped { "wrapped " } else { "" };
                format!("{wrapped}search: {typed} · {} of {total}", before + k + 1)
            }
        };
        Some((said, "^S next · ^R previous · enter stay · ^G go back"))
    }
}

/// The place, a slide and which of the places on it, `counts` having how
/// many each slide has, next after `from`, `forward`, or before it, round
/// the end to the other; `from` itself too if `inclusive`.
pub(super) fn place(
    counts: &[usize],
    from: (usize, usize),
    forward: bool,
    inclusive: bool,
) -> Option<(usize, usize)> {
    let all: Vec<(usize, usize)> = counts
        .iter()
        .enumerate()
        .flat_map(|(page, &n)| (0..n).map(move |k| (page, k)))
        .collect();
    let (first, last) = (*all.first()?, *all.last()?);
    let found = match (forward, inclusive) {
        (true, true) => all.iter().find(|&&p| p >= from),
        (true, false) => all.iter().find(|&&p| p > from),
        (false, true) => all.iter().rev().find(|&&p| p <= from),
        (false, false) => all.iter().rev().find(|&&p| p < from),
    };
    Some(found.copied().unwrap_or(if forward { first } else { last }))
}
