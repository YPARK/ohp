//! Choosing the file to present: when ohp is started without one, and over
//! the slides on Ctrl-O.
//!
//! A directory's folders and slides are listed. Typing narrows the list to
//! the names that match, best first: those the typing starts, then those it
//! is in, then those with its letters in order. A path typed up to a `/`,
//! as `../talks/` or `~/`, goes to that directory. Enter opens the file
//! picked, or goes into the folder picked; backspace or left on an empty
//! line goes up.
//!
//! A directory is read once, when the query first names it, not on every
//! key: it may be large, or on a slow network disk.

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use ratatui::{DefaultTerminal, Frame};
use std::cell::Cell;
use std::path::{Path, PathBuf};

/// The file picked in `start`, or below or above it; `None` if none is.
pub fn choose(start: &Path) -> anyhow::Result<Option<PathBuf>> {
    let mut browser = Browser::new(start)?;
    let mut terminal = ratatui::init();
    let chosen = browser.run(&mut terminal);
    ratatui::restore();
    chosen
}

#[derive(Clone)]
struct Entry {
    name: String,
    /// `name` lowercase, as it is matched.
    lower: String,
    dir: bool,
}

pub enum Outcome {
    Stay,
    Open(PathBuf),
    Quit,
}

pub struct Browser {
    /// The directory names are typed from.
    dir: PathBuf,
    query: String,
    /// The directory last read, and what is in it or why it cannot be read.
    listing: Option<(PathBuf, Result<Vec<Entry>, String>)>,
    /// What the query names in the directory it names, best match first.
    entries: Vec<Entry>,
    /// Set as it is drawn, as is `rows`.
    list: Cell<ListState>,
    /// Rows the list shows, for page up and down.
    rows: Cell<u16>,
}

impl Browser {
    pub fn new(start: &Path) -> anyhow::Result<Self> {
        let mut browser = Browser {
            dir: start.canonicalize()?,
            query: String::new(),
            listing: None,
            entries: Vec::new(),
            list: Cell::default(),
            rows: Cell::new(1),
        };
        browser.refresh();
        Ok(browser)
    }

    fn run(&mut self, terminal: &mut DefaultTerminal) -> anyhow::Result<Option<PathBuf>> {
        loop {
            terminal.draw(|f| self.draw(f, f.area()))?;
            if let Event::Key(key) = event::read()?
                && key.kind != KeyEventKind::Release
            {
                match self.key(key) {
                    Outcome::Stay => {}
                    Outcome::Open(path) => return Ok(Some(path)),
                    Outcome::Quit => return Ok(None),
                }
            }
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('c' | 'o') if ctrl => return Outcome::Quit,
            KeyCode::Char('u') if ctrl => self.retype(String::new()),
            KeyCode::Char('n') if ctrl => self.step(1),
            KeyCode::Char('p') if ctrl => self.step(-1),
            KeyCode::Char(c) if !ctrl => self.type_char(c),
            KeyCode::Down => self.step(1),
            KeyCode::Up => self.step(-1),
            KeyCode::PageDown => self.step(self.page()),
            KeyCode::PageUp => self.step(-self.page()),
            KeyCode::Backspace | KeyCode::Left if self.query.is_empty() => self.up(),
            KeyCode::Backspace => {
                let mut query = self.query.clone();
                query.pop();
                self.retype(query);
            }
            KeyCode::Right | KeyCode::Tab if self.picked().is_some_and(|e| e.dir) => self.enter(),
            KeyCode::Enter => return self.open(),
            KeyCode::Esc if self.query.is_empty() => return Outcome::Quit,
            KeyCode::Esc => self.retype(String::new()),
            _ => {}
        }
        Outcome::Stay
    }

    fn type_char(&mut self, c: char) {
        let query = format!("{}{c}", self.query);
        if c == '/'
            && let Ok(to) = resolve(&self.dir, &query).canonicalize()
            && to.is_dir()
        {
            self.dir = to;
            return self.retype(String::new());
        }
        self.retype(query);
    }

    fn retype(&mut self, query: String) {
        self.query = query;
        self.refresh();
    }

    /// The directory the query names, and the name being typed in it.
    fn split(&self) -> (PathBuf, &str) {
        match self.query.rfind('/') {
            Some(at) => (
                resolve(&self.dir, &self.query[..=at]),
                &self.query[at + 1..],
            ),
            None => (self.dir.clone(), &self.query),
        }
    }

    fn refresh(&mut self) {
        let (dir, stem) = self.split();
        let stem = stem.to_lowercase();
        if self.listing.as_ref().is_none_or(|(at, _)| *at != dir) {
            let listed = list(&dir).map_err(|e| format!("{}: {e}", dir.display()));
            self.listing = Some((dir.clone(), listed));
        }
        self.entries.clear();
        if let Some((_, Ok(listed))) = &self.listing {
            let hidden = stem.starts_with('.');
            let mut ranked: Vec<(u8, &Entry)> = listed
                .iter()
                .filter(|e| hidden || !e.name.starts_with('.'))
                .filter_map(|e| Some((rank(&e.lower, &stem)?, e)))
                .collect();
            ranked.sort_by_key(|&(r, _)| r);
            if stem.is_empty() && dir.parent().is_some() {
                self.entries.push(Entry {
                    name: "..".into(),
                    lower: "..".into(),
                    dir: true,
                });
            }
            self.entries
                .extend(ranked.into_iter().map(|(_, e)| e.clone()));
        }
        self.pick((!self.entries.is_empty()).then_some(0));
    }

    /// Why the directory the query names cannot be read.
    fn trouble(&self) -> Option<&str> {
        match &self.listing {
            Some((_, Err(e))) => Some(e),
            _ => None,
        }
    }

    fn pick(&self, at: Option<usize>) {
        let mut list = self.list.get();
        list.select(at);
        self.list.set(list);
    }

    fn picked(&self) -> Option<&Entry> {
        self.entries.get(self.list.get().selected()?)
    }

    fn step(&mut self, by: isize) {
        if self.entries.is_empty() {
            return;
        }
        let at = self.list.get().selected().unwrap_or(0);
        let last = self.entries.len() - 1;
        self.pick(Some(at.saturating_add_signed(by).min(last)));
    }

    fn page(&self) -> isize {
        isize::try_from(self.rows.get().saturating_sub(1).max(1)).unwrap_or(1)
    }

    /// Go into the folder picked.
    fn enter(&mut self) {
        let Some(name) = self.picked().map(|e| e.name.clone()) else {
            return;
        };
        if name == ".." && self.query.is_empty() {
            return self.up();
        }
        let to = self.split().0.join(&name);
        match to.canonicalize() {
            Ok(to) => {
                self.dir = to;
                self.retype(String::new());
            }
            // Gone since it was listed: read the directory again.
            Err(_) => {
                self.listing = None;
                self.refresh();
            }
        }
    }

    /// Go to the parent directory, with the one left picked.
    fn up(&mut self) {
        let Some(parent) = self.dir.parent().map(Path::to_path_buf) else {
            return;
        };
        let left = self
            .dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned());
        self.dir = parent;
        self.retype(String::new());
        if let Some(left) = left {
            self.select(&left);
        }
    }

    /// Pick the entry `name`, if it is listed.
    pub fn select(&mut self, name: &str) {
        if let Some(at) = self.entries.iter().position(|e| e.name == name) {
            self.pick(Some(at));
        }
    }

    fn open(&mut self) -> Outcome {
        match self.picked() {
            Some(entry) if entry.dir => {
                self.enter();
                Outcome::Stay
            }
            Some(entry) => Outcome::Open(self.split().0.join(&entry.name)),
            // A file typed out in full, even one not listed.
            None => {
                let path = resolve(&self.dir, &self.query);
                if path.is_file() {
                    Outcome::Open(path)
                } else {
                    Outcome::Stay
                }
            }
        }
    }

    pub fn draw(&self, f: &mut Frame, area: Rect) {
        let [prompt, list, status] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(area);
        self.rows.set(list.height);

        let dim = Style::new().fg(Color::DarkGray);
        let mut dir = self.dir.display().to_string();
        if !dir.ends_with('/') {
            dir.push('/');
        }
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" open ", Style::new().add_modifier(Modifier::BOLD)),
                Span::styled(dir, dim),
                Span::raw(self.query.as_str()),
                Span::styled("▏", Style::new().fg(Color::Yellow)),
            ])),
            prompt,
        );

        let items = self.entries.iter().map(|e| {
            if e.dir {
                ListItem::new(format!("   {}/", e.name)).style(Style::new().fg(Color::Blue))
            } else {
                ListItem::new(format!("   {}", e.name))
            }
        });
        let picked = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);
        let mut state = self.list.get();
        f.render_stateful_widget(List::new(items).highlight_style(picked), list, &mut state);
        self.list.set(state);

        let mut spans = Vec::new();
        if let Some(trouble) = self.trouble() {
            spans.push(Span::styled(
                format!(" {trouble} "),
                Style::new().add_modifier(Modifier::BOLD),
            ));
        } else if self.entries.is_empty() {
            spans.push(Span::styled(" no match ", dim));
        }
        spans.push(Span::styled(
            "  type to narrow · a/b/ go to a path · ↑/↓ move · enter open · ←/→ up/into · esc cancel",
            dim,
        ));
        f.render_widget(Paragraph::new(Line::from(spans)), status);
    }
}

/// `typed` as a path from `dir`, with a leading `~` as the home directory.
fn resolve(dir: &Path, typed: &str) -> PathBuf {
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    if typed == "~"
        && let Some(home) = home()
    {
        return home;
    }
    if let Some(rest) = typed.strip_prefix("~/")
        && let Some(home) = home()
    {
        return home.join(rest);
    }
    dir.join(typed)
}

/// The folders and slides in `dir`, hidden ones too, folders first, each
/// by name.
fn list(dir: &Path) -> std::io::Result<Vec<Entry>> {
    let mut entries: Vec<Entry> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let kind = e.file_type().ok()?;
            // Through a symlink, as a link to a folder is a folder.
            let dir = kind.is_dir() || kind.is_symlink() && e.path().is_dir();
            (dir || slides(Path::new(&name))).then(|| Entry {
                lower: name.to_lowercase(),
                name,
                dir,
            })
        })
        .collect();
    entries.sort_by(|a, b| (!a.dir, &a.lower).cmp(&(!b.dir, &b.lower)));
    Ok(entries)
}

/// Whether ohp can present `path`.
fn slides(path: &Path) -> bool {
    let pdf = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
    pdf || cfg!(feature = "markdown") && crate::render::markdown(path)
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

#[cfg(test)]
#[path = "tests/browse.rs"]
mod tests;
