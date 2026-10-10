//! Choosing the voice slides are read in, on `v`, in a list over the slides.
//!
//! Listed are the voice given, Piper's voices kept where its voices go, and
//! other voices found, as eSpeak NG; then, once fetched, the voices in
//! Piper's catalog not yet kept, the user's language first. Typing narrows
//! the list as the file picker's does. A voice not yet kept is downloaded
//! with curl, off the UI thread, to where Piper's voices go, and is read in
//! once it is.

use crate::process::child::{self, Ended};
use crate::speak;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use serde_json::Value;
use std::cell::Cell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::thread::JoinHandle;

/// Where Piper's voices are downloaded from.
pub const PIPER_VOICES: &str = "https://huggingface.co/rhasspy/piper-voices/resolve/main";

/// A voice to read slides in.
#[derive(Clone, Debug, PartialEq)]
pub struct Voice {
    pub name: String,
    /// What it is, as its language and quality.
    pub about: String,
    pub how: How,
}

#[derive(Clone, Debug, PartialEq)]
pub enum How {
    /// The command it speaks with.
    Ready(String),
    /// Piper's voice to download first.
    Fetch(Remote),
}

/// A voice in Piper's catalog.
#[derive(Clone, Debug, PartialEq)]
pub struct Remote {
    /// Its name, as `en_US-amy-medium`.
    pub key: String,
    /// Its model and config: each path under the catalog's root, and size.
    files: Vec<(String, u64)>,
}

/// The voices at hand: `given`, Piper's in `dir` if Piper is installed, and
/// `others` found, as `speak::others` gives them.
pub fn at_hand(
    given: Option<&str>,
    dir: Option<&Path>,
    piper: bool,
    others: &[(&str, &str)],
) -> Vec<Voice> {
    let mut voices = Vec::new();
    if let Some(given) = given.filter(|g| !g.trim().is_empty())
        && !is_found(given, dir, others)
    {
        voices.push(Voice {
            name: "given".into(),
            about: given.into(),
            how: How::Ready(given.into()),
        });
    }
    if piper && let Some(dir) = dir {
        for model in speak::piper_models(dir) {
            let (Some(stem), Some(command)) = (
                model.file_stem().and_then(|s| s.to_str()),
                speak::piper_with(&model),
            ) else {
                continue;
            };
            voices.push(Voice {
                name: stem.into(),
                about: "Piper".into(),
                how: How::Ready(command),
            });
        }
    }
    for &(name, command) in others {
        voices.push(Voice {
            name: name.into(),
            about: "installed".into(),
            how: How::Ready(command.into()),
        });
    }
    voices
}

/// Whether `given` is one of the voices listed anyway.
fn is_found(given: &str, dir: Option<&Path>, others: &[(&str, &str)]) -> bool {
    others.iter().any(|&(_, c)| c == given)
        || dir.is_some_and(|dir| {
            speak::piper_models(dir)
                .iter()
                .any(|m| speak::piper_with(m).as_deref() == Some(given))
        })
}

/// Piper's voices in its `catalog`, `voices.json`, but those `kept`, the
/// voices in `language`'s family first, as `en` for `en_GB.UTF-8`.
pub fn catalog(
    catalog: &str,
    kept: &HashSet<String>,
    language: &str,
) -> Result<Vec<Voice>, String> {
    let all: serde_json::Map<String, Value> =
        serde_json::from_str(catalog).map_err(|e| format!("cannot read Piper's voices: {e}"))?;
    let family = language.split(['_', '.', '-']).next().unwrap_or_default();
    let mut voices: Vec<(bool, Voice)> = all
        .into_iter()
        .filter(|(key, _)| !kept.contains(key))
        .filter_map(|(key, v)| {
            let files = v["files"]
                .as_object()?
                .iter()
                .filter(|(path, _)| path.ends_with(".onnx") || path.ends_with(".onnx.json"))
                .map(|(path, f)| Some((path.clone(), f["size_bytes"].as_u64()?)))
                .collect::<Option<Vec<_>>>()?;
            if files.len() != 2 {
                return None;
            }
            let language = &v["language"];
            let size: u64 = files.iter().map(|(_, size)| size).sum();
            let about = format!(
                "{} ({}) · {} · {} MB",
                language["name_english"].as_str().unwrap_or("?"),
                language["country_english"].as_str().unwrap_or("?"),
                v["quality"].as_str().unwrap_or("?"),
                size.div_ceil(1_000_000),
            );
            let other = language["family"].as_str() != Some(family);
            let how = How::Fetch(Remote {
                key: key.clone(),
                files,
            });
            Some((
                other,
                Voice {
                    name: key,
                    about,
                    how,
                },
            ))
        })
        .collect();
    voices.sort_by(|(a, va), (b, vb)| (a, &va.name).cmp(&(b, &vb.name)));
    Ok(voices.into_iter().map(|(_, v)| v).collect())
}

/// The user's language, as `LANG` gives it.
pub fn language() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(std::env::var_os)
        .map(|l| l.to_string_lossy().into_owned())
        .find(|l| !l.is_empty())
        .unwrap_or_default()
}

/// Fetch `url` to `to` with curl, until it is done or `stop` is set.
fn fetch(url: &str, to: &Path, stop: &AtomicBool) -> Result<(), String> {
    let log = to.with_extension("log");
    let stderr = std::fs::File::create(&log).map_err(|e| e.to_string())?;
    let mut curl = Command::new("curl");
    curl.args(["-fsSL", "-o"])
        .arg(to)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr);
    let ended = child::run(curl, stop, None);
    let why = child::last_line(&log, |_| true);
    let _ = std::fs::remove_file(&log);
    match ended {
        Ok(Ended::Exited(status)) if status.success() => Ok(()),
        Ok(Ended::Exited(_)) => Err(why.unwrap_or_else(|| "curl failed".into())),
        Ok(Ended::Stopped) => Err("stopped".into()),
        Ok(Ended::Paused | Ended::TimedOut) => Err("curl paused".into()),
        Err(e) => Err(format!("cannot run curl: {e}")),
    }
}

/// Piper's catalog being fetched, to list its voices once it is.
pub struct Catalog {
    stop: Arc<AtomicBool>,
    done: Receiver<Result<String, String>>,
    thread: Option<JoinHandle<()>>,
}

impl Catalog {
    /// Fetch the catalog at `root`.
    pub fn fetch(root: &str) -> Catalog {
        let url = format!("{root}/voices.json");
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, done) = channel();
        let halt = stop.clone();
        let thread = std::thread::Builder::new()
            .name(speak::THREAD.into())
            .spawn(move || {
                let read = tempfile::tempdir()
                    .map_err(|e| e.to_string())
                    .and_then(|dir| {
                        let to = dir.path().join("voices.json");
                        fetch(&url, &to, &halt)?;
                        std::fs::read_to_string(&to).map_err(|e| e.to_string())
                    });
                let _ = tx.send(read);
            })
            .ok();
        Catalog { stop, done, thread }
    }

    /// The catalog, or why it could not be fetched, once it is.
    pub fn fetched(&self) -> Option<Result<String, String>> {
        match self.done.try_recv() {
            Ok(read) => Some(read),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("cannot fetch Piper's voices".into())),
        }
    }
}

impl Drop for Catalog {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// A voice of Piper's being downloaded.
pub struct Download {
    pub name: String,
    /// Each file as it is downloaded, and its size when done.
    parts: Vec<(PathBuf, u64)>,
    stop: Arc<AtomicBool>,
    done: Receiver<Result<String, String>>,
    thread: Option<JoinHandle<()>>,
}

impl Download {
    /// Download `remote` from `root` into `dir`.
    pub fn start(remote: &Remote, root: &str, dir: &Path) -> Download {
        let parts: Vec<(PathBuf, u64)> = remote
            .files
            .iter()
            .map(|(path, size)| {
                let name = Path::new(path).file_name().unwrap_or_default();
                let mut part = dir.join(name).into_os_string();
                part.push(".part");
                (PathBuf::from(part), *size)
            })
            .collect();
        let jobs: Vec<(String, PathBuf, u64)> = (remote.files.iter().zip(&parts))
            .map(|((path, size), (part, _))| (format!("{root}/{path}"), part.clone(), *size))
            .collect();
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, done) = channel();
        let (halt, dir) = (stop.clone(), dir.to_path_buf());
        let thread = std::thread::Builder::new()
            .name(speak::THREAD.into())
            .spawn(move || {
                let _ = tx.send(download(&jobs, &dir, &halt));
            })
            .ok();
        Download {
            name: remote.key.clone(),
            parts,
            stop,
            done,
            thread,
        }
    }

    /// How much is downloaded, from 0 to 1.
    pub fn progress(&self) -> f64 {
        let total: u64 = self.parts.iter().map(|(_, size)| size).sum();
        let got: u64 = (self.parts.iter())
            .map(|(part, size)| {
                let len = std::fs::metadata(part).map_or(0, |m| m.len());
                // A part renamed when done is no longer there.
                if len == 0 && !part.exists() && part.with_extension("").exists() {
                    *size
                } else {
                    len
                }
            })
            .sum();
        if total == 0 {
            1.
        } else {
            got as f64 / total as f64
        }
    }

    /// The command to speak in the voice, or why it could not be
    /// downloaded, once it is.
    pub fn ended(&self) -> Option<Result<String, String>> {
        match self.done.try_recv() {
            Ok(ended) => Some(ended),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("the download stopped".into())),
        }
    }
}

/// However ohp ends, a download stops with it, and leaves no part behind.
impl Drop for Download {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Download each of `jobs`, a URL, the part it goes to and its size, then
/// put them in place, the model before its config, so no voice is half
/// there. The command to speak in it.
fn download(
    jobs: &[(String, PathBuf, u64)],
    dir: &Path,
    stop: &AtomicBool,
) -> Result<String, String> {
    let fetched = std::fs::create_dir_all(dir)
        .map_err(|e| format!("cannot make {}: {e}", dir.display()))
        .and_then(|()| {
            for (url, part, size) in jobs {
                fetch(url, part, stop)?;
                let len = std::fs::metadata(part).map_or(0, |m| m.len());
                if len != *size {
                    return Err(format!("{url} came short: {len} of {size} bytes"));
                }
            }
            Ok(())
        });
    if let Err(e) = fetched {
        for (_, part, _) in jobs {
            let _ = std::fs::remove_file(part);
        }
        return Err(e);
    }
    let mut placed: Vec<&PathBuf> = jobs.iter().map(|(_, part, _)| part).collect();
    // The config last: a model is a voice once its config is beside it.
    placed.sort_by_key(|part| part.to_string_lossy().ends_with(".json.part"));
    let mut model = None;
    for part in placed {
        let to = part.with_extension("");
        std::fs::rename(part, &to).map_err(|e| e.to_string())?;
        if to.extension().is_some_and(|e| e == "onnx") {
            model = Some(to);
        }
    }
    model
        .as_deref()
        .and_then(speak::piper_with)
        .ok_or_else(|| "the voice has no model".into())
}

pub enum Outcome {
    Stay,
    Pick(Voice),
    Quit,
}

/// The list voices are picked from.
pub struct Menu {
    query: String,
    voices: Vec<Voice>,
    /// The voices the query names, best match first.
    shown: Vec<usize>,
    /// The command of the voice in use, marked in the list.
    current: Option<String>,
    catalog: Option<Catalog>,
    /// Why there are no more voices to download.
    trouble: Option<String>,
    list: Cell<ListState>,
    rows: Cell<u16>,
}

impl Menu {
    /// A list of `voices`, `current` the command of the one in use, which
    /// adds Piper's catalog from `catalog` once it is fetched.
    pub fn new(
        voices: Vec<Voice>,
        current: Option<&str>,
        catalog: Result<Catalog, String>,
    ) -> Self {
        let (catalog, trouble) = match catalog {
            Ok(catalog) => (Some(catalog), None),
            Err(why) => (None, Some(why)),
        };
        let mut menu = Menu {
            query: String::new(),
            voices,
            shown: Vec::new(),
            current: current.map(String::from),
            catalog,
            trouble,
            list: Cell::default(),
            rows: Cell::new(1),
        };
        menu.refresh();
        let at = (menu.shown.iter()).position(|&i| menu.is_current(&menu.voices[i]));
        menu.list
            .set(ListState::default().with_selected(at.or(Some(0))));
        menu
    }

    /// Add Piper's catalog once it is fetched. Whether the list changed.
    pub fn poll(&mut self) -> bool {
        let Some(fetched) = self.catalog.as_ref().and_then(Catalog::fetched) else {
            return false;
        };
        self.catalog = None;
        let kept: HashSet<String> = self.voices.iter().map(|v| v.name.clone()).collect();
        match fetched.and_then(|json| catalog(&json, &kept, &language())) {
            Ok(more) => self.voices.extend(more),
            Err(e) => self.trouble = Some(e),
        }
        let picked = self.picked().map(|v| v.name.clone());
        self.refresh();
        if let Some(name) = picked {
            self.select(&name);
        }
        true
    }

    fn is_current(&self, voice: &Voice) -> bool {
        matches!(&voice.how, How::Ready(c) if Some(c) == self.current.as_ref())
    }

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('c') if ctrl => return Outcome::Quit,
            KeyCode::Char('u') if ctrl => self.retype(String::new()),
            KeyCode::Char('n') if ctrl => self.step(1),
            KeyCode::Char('p') if ctrl => self.step(-1),
            KeyCode::Char(c) if !ctrl => self.retype(format!("{}{c}", self.query)),
            KeyCode::Down => self.step(1),
            KeyCode::Up => self.step(-1),
            KeyCode::PageDown => self.step(self.page()),
            KeyCode::PageUp => self.step(-self.page()),
            KeyCode::Backspace => {
                let mut query = self.query.clone();
                query.pop();
                self.retype(query);
            }
            KeyCode::Enter => {
                if let Some(voice) = self.picked() {
                    return Outcome::Pick(voice.clone());
                }
            }
            KeyCode::Esc if self.query.is_empty() => return Outcome::Quit,
            KeyCode::Esc => self.retype(String::new()),
            _ => {}
        }
        Outcome::Stay
    }

    fn retype(&mut self, query: String) {
        self.query = query;
        self.refresh();
        self.list
            .set(ListState::default().with_selected((!self.shown.is_empty()).then_some(0)));
    }

    /// List the voices the query names, best first; all, as they are, if
    /// it names none in particular.
    fn refresh(&mut self) {
        let query = self.query.to_lowercase();
        let mut ranked: Vec<(u8, usize)> = (self.voices.iter().enumerate())
            .filter_map(|(i, v)| {
                let name = v.name.to_lowercase();
                let about = v.about.to_lowercase();
                let rank = crate::browse::rank(&name, &query)
                    .or_else(|| crate::browse::rank(&about, &query).map(|r| r + 3))?;
                Some((rank, i))
            })
            .collect();
        ranked.sort_by_key(|&(rank, i)| (rank, i));
        self.shown = ranked.into_iter().map(|(_, i)| i).collect();
    }

    fn select(&mut self, name: &str) {
        let at = (self.shown.iter()).position(|&i| self.voices[i].name == name);
        if at.is_some() {
            self.list.set(ListState::default().with_selected(at));
        }
    }

    fn picked(&self) -> Option<&Voice> {
        Some(&self.voices[*self.shown.get(self.list.get().selected()?)?])
    }

    fn step(&mut self, by: isize) {
        if self.shown.is_empty() {
            return;
        }
        let at = self.list.get().selected().unwrap_or(0);
        let at = at.saturating_add_signed(by).min(self.shown.len() - 1);
        self.list.set(ListState::default().with_selected(Some(at)));
    }

    fn page(&self) -> isize {
        isize::try_from(self.rows.get().saturating_sub(1).max(1)).unwrap_or(1)
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
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" voice ", Style::new().add_modifier(Modifier::BOLD)),
                Span::raw(self.query.as_str()),
                Span::styled("▏", Style::new().fg(Color::Yellow)),
            ])),
            prompt,
        );

        let width = (self.shown.iter())
            .map(|&i| self.voices[i].name.chars().count())
            .max()
            .unwrap_or(0);
        let items = self.shown.iter().map(|&i| {
            let voice = &self.voices[i];
            let mark = if self.is_current(voice) {
                " ● "
            } else {
                "   "
            };
            let (about, style) = match voice.how {
                How::Ready(_) => (voice.about.clone(), Style::new()),
                How::Fetch(_) => (format!("{} · download", voice.about), dim),
            };
            ListItem::new(Line::from(vec![
                Span::raw(format!("{mark}{:width$}  ", voice.name)),
                Span::styled(about, dim),
            ]))
            .style(style)
        });
        let picked = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);
        let mut state = self.list.get();
        f.render_stateful_widget(List::new(items).highlight_style(picked), list, &mut state);
        self.list.set(state);

        let mut spans = Vec::new();
        if self.catalog.is_some() {
            spans.push(Span::styled(" fetching Piper's voices… ", dim));
        } else if let Some(trouble) = &self.trouble {
            spans.push(Span::styled(
                format!(" {trouble} "),
                Style::new().add_modifier(Modifier::BOLD),
            ));
        }
        if self.shown.is_empty() {
            spans.push(Span::styled(" no match ", dim));
        }
        spans.push(Span::styled(
            "  type to narrow · ↑/↓ move · enter read in it · esc cancel",
            dim,
        ));
        f.render_widget(Paragraph::new(Line::from(spans)), status);
    }
}

#[cfg(test)]
#[path = "tests/voices.rs"]
mod tests;
