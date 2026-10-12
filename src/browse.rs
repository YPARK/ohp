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
//! Tab marks the file picked, or unmarks it, in any directory, and
//! shift-tab unmarks them all; enter opens the files marked, in the order
//! they were, and then the one picked, as one deck.
//!
//! A directory is read once, when the query first names it, not on every
//! key: it may be large, or on a slow network disk.

use crate::menu::{self, Menu, dim, rank};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::ListItem;
use ratatui::{DefaultTerminal, Frame};
use std::path::{Path, PathBuf};

/// The files picked in `start`, or below or above it; `None` if none is.
pub fn choose(start: &Path) -> anyhow::Result<Option<Vec<PathBuf>>> {
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
    Open(Vec<PathBuf>),
    Quit,
}

pub struct Browser {
    /// The directory names are typed from.
    dir: PathBuf,
    /// What is typed, and the entry picked.
    menu: Menu,
    /// The directory last read, and what is in it or why it cannot be read.
    listing: Option<(PathBuf, Result<Vec<Entry>, String>)>,
    /// The directory last read, as `marked` names its files.
    here: PathBuf,
    /// What the query names in the directory it names, best match first.
    entries: Vec<Entry>,
    /// Files marked to open together, in the order they were.
    marked: Vec<PathBuf>,
}

impl Browser {
    pub fn new(start: &Path) -> anyhow::Result<Self> {
        let dir = start.canonicalize()?;
        let mut browser = Browser {
            here: dir.clone(),
            dir,
            menu: Menu::default(),
            listing: None,
            entries: Vec::new(),
            marked: Vec::new(),
        };
        browser.refresh();
        Ok(browser)
    }

    fn run(&mut self, terminal: &mut DefaultTerminal) -> anyhow::Result<Option<Vec<PathBuf>>> {
        loop {
            terminal.draw(|f| self.draw(f, f.area()))?;
            if let Event::Key(key) = event::read()?
                && key.kind != KeyEventKind::Release
            {
                match self.key(key) {
                    Outcome::Stay => {}
                    Outcome::Open(paths) => return Ok(Some(paths)),
                    Outcome::Quit => return Ok(None),
                }
            }
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('o') if ctrl => return Outcome::Quit,
            KeyCode::Backspace | KeyCode::Left if self.menu.query.is_empty() => self.up(),
            KeyCode::Right | KeyCode::Tab if self.picked().is_some_and(|e| e.dir) => self.enter(),
            KeyCode::Tab if self.picked().is_some() => self.mark(),
            KeyCode::BackTab => self.marked.clear(),
            _ => match self.menu.key(key, self.entries.len()) {
                menu::Key::Typed('/') => self.slash(),
                menu::Key::Typed(_) | menu::Key::Retyped => self.refresh(),
                menu::Key::Enter => return self.open(),
                menu::Key::Quit => return Outcome::Quit,
                menu::Key::Moved | menu::Key::Other => {}
            },
        }
        Outcome::Stay
    }

    /// A `/` typed: go to the directory typed, if it is one.
    fn slash(&mut self) {
        if let Ok(to) = resolve(&self.dir, &self.menu.query).canonicalize()
            && to.is_dir()
        {
            self.dir = to;
            self.menu.query.clear();
        }
        self.refresh();
    }

    fn retype(&mut self, query: String) {
        self.menu.query = query;
        self.refresh();
    }

    /// The directory the query names, and the name being typed in it.
    fn split(&self) -> (PathBuf, &str) {
        let query = &self.menu.query;
        match query.rfind('/') {
            Some(at) => (resolve(&self.dir, &query[..=at]), &query[at + 1..]),
            None => (self.dir.clone(), query),
        }
    }

    fn refresh(&mut self) {
        let (dir, stem) = self.split();
        let stem = stem.to_lowercase();
        if self.listing.as_ref().is_none_or(|(at, _)| *at != dir) {
            let listed = list(&dir).map_err(|e| format!("{}: {e}", dir.display()));
            self.here = dir.canonicalize().unwrap_or_else(|_| dir.clone());
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
        self.menu.pick((!self.entries.is_empty()).then_some(0));
    }

    /// Why the directory the query names cannot be read.
    fn trouble(&self) -> Option<&str> {
        match &self.listing {
            Some((_, Err(e))) => Some(e),
            _ => None,
        }
    }

    fn picked(&self) -> Option<&Entry> {
        self.entries.get(self.menu.picked()?)
    }

    /// Go into the folder picked.
    fn enter(&mut self) {
        let Some(name) = self.picked().map(|e| e.name.clone()) else {
            return;
        };
        if name == ".." && self.menu.query.is_empty() {
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
            self.menu.pick(Some(at));
        }
    }

    /// The path of `entry`, as listed.
    fn path(&self, entry: &Entry) -> PathBuf {
        self.here.join(&entry.name)
    }

    /// Mark the file picked, or unmark it, and pick the next.
    fn mark(&mut self) {
        let Some(path) = self.picked().map(|e| self.path(e)) else {
            return;
        };
        match self.marked.iter().position(|m| *m == path) {
            Some(at) => {
                self.marked.remove(at);
            }
            None => self.marked.push(path),
        }
        self.menu.key(KeyCode::Down.into(), self.entries.len());
    }

    /// Mark `paths`, as the files of the deck presented, in place of any
    /// marked.
    pub fn mark_all(&mut self, paths: &[PathBuf]) {
        // As `path` names them: a link to a file is marked, not the file.
        let listed = |p: &PathBuf| {
            let dir = p.parent().filter(|d| !d.as_os_str().is_empty());
            let dir = dir.unwrap_or(Path::new(".")).canonicalize().ok()?;
            let path = dir.join(p.file_name()?);
            path.exists().then_some(path)
        };
        self.marked = paths.iter().filter_map(listed).collect();
    }

    /// Go into the folder picked, or open the files marked and the one
    /// picked after them.
    fn open(&mut self) -> Outcome {
        let picked = match self.picked() {
            Some(entry) if entry.dir => {
                self.enter();
                return Outcome::Stay;
            }
            Some(entry) => Some(self.path(entry)),
            // A file typed out in full, even one not listed.
            None => Some(resolve(&self.dir, &self.menu.query)).filter(|p| p.is_file()),
        };
        let mut paths = self.marked.clone();
        if let Some(path) = picked
            && !paths.contains(&path)
        {
            paths.push(path);
        }
        if paths.is_empty() {
            Outcome::Stay
        } else {
            Outcome::Open(paths)
        }
    }

    pub fn draw(&self, f: &mut Frame, area: Rect) {
        let mut dir = self.dir.display().to_string();
        if !dir.ends_with('/') {
            dir.push('/');
        }
        let prompt = vec![
            Span::styled(" open ", Style::new().add_modifier(Modifier::BOLD)),
            Span::styled(dir, dim()),
        ];
        let items = self.entries.iter().map(|e| {
            if e.dir {
                ListItem::new(format!("   {}/", e.name)).style(Style::new().fg(Color::Blue))
            } else if !self.marked.is_empty() && self.marked.contains(&self.path(e)) {
                ListItem::new(format!(" + {}", e.name)).style(Style::new().fg(Color::Green))
            } else {
                ListItem::new(format!("   {}", e.name))
            }
        });
        let mut notes = match self.trouble() {
            Some(trouble) => vec![menu::trouble(trouble)],
            None if self.entries.is_empty() => vec![Span::styled(" no match ", dim())],
            None => Vec::new(),
        };
        if !self.marked.is_empty() {
            let marked = format!(" {} marked ", self.marked.len());
            notes.push(Span::styled(marked, Style::new().fg(Color::Green)));
        }
        let keys = "type to narrow · a/b/ go to a path · ↑/↓ move · tab mark · shift-tab unmark all · enter open · ←/→ up/into · esc cancel";
        self.menu.draw(f, area, prompt, items, notes, keys);
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
    let typeset = crate::render::markdown(path) || crate::render::image(path);
    pdf || cfg!(feature = "markdown") && typeset
}

#[cfg(test)]
#[path = "tests/browse.rs"]
mod tests;
