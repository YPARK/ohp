//! Choosing the voice slides are read in, on `v`, in a list over the slides.
//!
//! Listed are the voice given, Piper's voices kept where its voices go, and
//! other voices found, as eSpeak NG; then, once fetched, the voices in
//! Piper's catalog not yet kept, the user's language first. The catalog is
//! fetched once, with curl, off the UI thread, and kept for the next time
//! the list opens. Typing narrows the list as the file picker's does. A
//! voice not yet kept is downloaded first, its progress in the status line,
//! and read in once it is whole.

use crate::menu::{self, Menu, dim, rank};
use crate::process::child;
use crate::speak;
use ratatui::Frame;
use ratatui::crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::ListItem;
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::thread::JoinHandle;

/// Where Piper's voices are downloaded from.
const PIPER_VOICES: &str = "https://huggingface.co/rhasspy/piper-voices/resolve/main";

/// A voice to read slides in.
#[derive(Clone, Debug, PartialEq)]
pub struct Voice {
    pub name: String,
    /// What it is, as its language and quality.
    about: String,
    how: How,
}

#[derive(Clone, Debug, PartialEq)]
enum How {
    /// The command it speaks with.
    Ready(String),
    /// Piper's voice to download first.
    Fetch(Remote),
}

/// A voice in Piper's catalog: its model and config, each a path under the
/// catalog's root and its size.
#[derive(Clone, Debug, PartialEq)]
struct Remote {
    model: (String, u64),
    config: (String, u64),
}

/// The voice slides are read in unless one is picked: the one given, or
/// else the first at hand.
pub fn default(given: Option<String>) -> Option<String> {
    speak::given_or(given, || {
        let piper = speak::piper_dir().filter(|_| speak::installed("piper"));
        let voices = at_hand(None, piper.as_deref(), &speak::others());
        voices.into_iter().find_map(|v| match v.how {
            How::Ready(command) => Some(command),
            How::Fetch(_) => None,
        })
    })
}

/// The voices at hand: `given`, Piper's in `piper`, its voices' directory
/// if Piper is installed, and `others` found, as `speak::others` gives them.
fn at_hand(given: Option<&str>, piper: Option<&Path>, others: &[(&str, &str)]) -> Vec<Voice> {
    let ready = |name: &str, about: &str, command: String| Voice {
        name: name.into(),
        about: about.into(),
        how: How::Ready(command),
    };
    let mut voices = Vec::new();
    for model in piper.map(speak::piper_models).unwrap_or_default() {
        if let (Some(stem), Some(command)) = (
            model.file_stem().and_then(|s| s.to_str()),
            speak::piper_with(&model),
        ) {
            voices.push(ready(stem, "Piper", command));
        }
    }
    for &(name, command) in others {
        voices.push(ready(name, "installed", command.into()));
    }
    if let Some(given) = given.filter(|g| !g.trim().is_empty())
        && !voices.iter().any(|v| v.how == How::Ready(given.into()))
    {
        voices.insert(0, ready("given", given, given.into()));
    }
    voices
}

/// Piper's voices in its `catalog`, `voices.json`, those in `language`'s
/// family first, as `en` for `en_GB.UTF-8`.
fn catalog(catalog: &str, language: &str) -> Result<Vec<Voice>, String> {
    let all: serde_json::Map<String, Value> =
        serde_json::from_str(catalog).map_err(|e| format!("cannot read Piper's voices: {e}"))?;
    let family = language
        .split(['_', '.', '-', '@'])
        .next()
        .unwrap_or_default();
    let mut voices: Vec<(bool, Voice)> = all
        .into_iter()
        .filter_map(|(key, v)| {
            let files = v["files"].as_object()?;
            let file = |suffix: &str| {
                let (path, f) = files.iter().find(|(path, _)| path.ends_with(suffix))?;
                Some((path.clone(), f["size_bytes"].as_u64()?))
            };
            let remote = Remote {
                model: file(".onnx")?,
                config: file(".onnx.json")?,
            };
            let language = &v["language"];
            let size = remote.model.1 + remote.config.1;
            let about = format!(
                "{} ({}) · {} · {} MB",
                language["name_english"].as_str().unwrap_or("?"),
                language["country_english"].as_str().unwrap_or("?"),
                v["quality"].as_str().unwrap_or("?"),
                size.div_ceil(1_000_000),
            );
            let other = language["family"].as_str() != Some(family);
            let how = How::Fetch(remote);
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

/// The user's language, as the locale gives it.
fn language() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(std::env::var_os)
        .map(|l| l.to_string_lossy().into_owned())
        .find(|l| !l.is_empty())
        .unwrap_or_default()
}

/// Fetch `url` to `to` with curl, until it is done or `stop` is set. A
/// fetch that cannot connect, or stalls, fails rather than waits for ever.
fn fetch(url: &str, to: &Path, stop: &AtomicBool) -> Result<(), String> {
    let mut curl = Command::new("curl");
    curl.args(["-fsSL", "--connect-timeout", "30"])
        .args(["--speed-limit", "1000", "--speed-time", "30", "-o"])
        .arg(to)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    let log = to.with_extension("log");
    let ran = child::run_logged(curl, &log, stop);
    let _ = std::fs::remove_file(&log);
    match ran {
        Ok(true) => Ok(()),
        Ok(false) => Err("stopped".into()),
        Err(why) => Err(why),
    }
}

/// Work on a thread of its own, stopped and waited for when dropped.
struct Task<T> {
    stop: Arc<AtomicBool>,
    done: Receiver<Result<T, String>>,
    thread: Option<JoinHandle<()>>,
}

impl<T: Send + 'static> Task<T> {
    /// Do `work`, which is to stop once its flag is set.
    fn spawn(work: impl FnOnce(&AtomicBool) -> Result<T, String> + Send + 'static) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, done) = channel();
        let flag = stop.clone();
        let thread = std::thread::Builder::new()
            .name(speak::VOICES_THREAD.into())
            .spawn(move || {
                // Nothing to do if no one waits for it.
                let _ = tx.send(work(&flag));
            })
            .ok();
        Task { stop, done, thread }
    }

    /// What the work came to, once it is done.
    fn done(&self) -> Option<Result<T, String>> {
        match self.done.try_recv() {
            Ok(done) => Some(done),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("it stopped".into())),
        }
    }
}

impl<T> Drop for Task<T> {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Fetch Piper's catalog at `root`, and read its voices.
fn fetch_catalog(root: &str) -> Task<Vec<Voice>> {
    let url = format!("{root}/voices.json");
    Task::spawn(move |stop| {
        let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
        let to = dir.path().join("voices.json");
        fetch(&url, &to, stop)?;
        let json = std::fs::read_to_string(&to).map_err(|e| e.to_string())?;
        catalog(&json, &language())
    })
}

/// A voice of Piper's being downloaded.
struct Download {
    name: String,
    /// The model and config as they are downloaded.
    parts: [PathBuf; 2],
    /// Their size, whole.
    total: u64,
    /// How much of it was last shown, in percent.
    shown: u8,
    task: Task<String>,
}

impl Download {
    /// Download `voice` from `root` into `dir`.
    fn start(name: &str, remote: &Remote, root: &str, dir: &Path) -> Download {
        let part = |path: &str| {
            let mut part = dir.join(Path::new(path).file_name().unwrap_or_default());
            part.as_mut_os_string().push(".part");
            part
        };
        let parts = [part(&remote.model.0), part(&remote.config.0)];
        let total = remote.model.1 + remote.config.1;
        let jobs = [
            (
                format!("{root}/{}", remote.model.0),
                parts[0].clone(),
                remote.model.1,
            ),
            (
                format!("{root}/{}", remote.config.0),
                parts[1].clone(),
                remote.config.1,
            ),
        ];
        let dir = dir.to_path_buf();
        let task = Task::spawn(move |stop| download(&jobs, &dir, stop));
        Download {
            name: name.into(),
            parts,
            total,
            shown: 0,
            task,
        }
    }

    /// How much is downloaded, in percent.
    fn percent(&self) -> u8 {
        let got: u64 = (self.parts.iter())
            .map(|part| {
                // Once whole, a part is renamed to what it was downloading.
                std::fs::metadata(part)
                    .or_else(|_| std::fs::metadata(part.with_extension("")))
                    .map_or(0, |m| m.len())
            })
            .sum();
        (got * 100 / self.total.max(1)).min(100) as u8
    }
}

/// Download the model, then the config, each a URL, the part it goes to
/// and its size, then put them in place, the config last: a model is a
/// voice once its config is beside it. The command to speak in it.
fn download(
    jobs: &[(String, PathBuf, u64); 2],
    dir: &Path,
    stop: &AtomicBool,
) -> Result<String, String> {
    let fetched = std::fs::create_dir_all(dir)
        .map_err(|e| format!("cannot make {}: {e}", dir.display()))
        .and_then(|()| {
            jobs.iter().try_for_each(|(url, part, size)| {
                fetch(url, part, stop)?;
                let len = std::fs::metadata(part).map_or(0, |m| m.len());
                if len == *size {
                    Ok(())
                } else {
                    Err(format!("{url} came short: {len} of {size} bytes"))
                }
            })
        });
    let placed = fetched.and_then(|()| {
        let [(_, model, _), (_, config, _)] = jobs;
        std::fs::rename(model, model.with_extension("")).map_err(|e| e.to_string())?;
        std::fs::rename(config, config.with_extension("")).map_err(|e| {
            // Without its config, the model is no voice: not to be left.
            let _ = std::fs::remove_file(model.with_extension(""));
            e.to_string()
        })
    });
    if let Err(e) = placed {
        for (_, part, _) in jobs {
            let _ = std::fs::remove_file(part);
        }
        return Err(e);
    }
    speak::piper_with(&jobs[0].1.with_extension(""))
        .ok_or_else(|| "the model's path is not text".into())
}

/// What picking voices calls for.
pub enum Event {
    /// Read in voice `name`, which speaks with `command`.
    Use(String, String),
    Notice(String),
}

/// Piper's catalog: not yet fetched, being fetched, or its voices.
enum Catalog {
    Unfetched,
    Fetching(Task<Vec<Voice>>),
    Fetched(Vec<Voice>),
}

/// The voice picker: the list, when open, Piper's catalog, and a voice
/// being downloaded.
pub struct Voices {
    root: String,
    list: Option<List>,
    catalog: Catalog,
    /// Where Piper's voices go, with Piper and curl installed to fetch them.
    piper: Result<PathBuf, &'static str>,
    download: Option<Download>,
}

/// The list voices are picked from.
struct List {
    menu: Menu,
    voices: Vec<Voice>,
    /// The voices the query names, best match first.
    shown: Vec<usize>,
    /// The command of the voice in use, marked.
    current: Option<String>,
    /// Why there are no more voices to download.
    trouble: Option<String>,
}

impl Default for Voices {
    fn default() -> Self {
        Voices::new(PIPER_VOICES)
    }
}

impl Voices {
    /// A picker downloading Piper's voices from `root`.
    fn new(root: &str) -> Self {
        Voices {
            root: root.into(),
            list: None,
            catalog: Catalog::Unfetched,
            piper: Err(""),
            download: None,
        }
    }

    /// Open the list, `current` the command of the voice in use.
    pub fn open(&mut self, current: Option<&str>) {
        let piper = speak::installed("piper");
        let dir = speak::piper_dir().filter(|_| piper);
        let at_hand = at_hand(current, dir.as_deref(), &speak::others());
        self.piper = match dir {
            _ if !piper => Err("install Piper to download its voices"),
            _ if !speak::installed("curl") => Err("install curl to download Piper's voices"),
            None => Err("no place for Piper's voices: HOME is not set"),
            Some(dir) => Ok(dir),
        };
        let mut list = List::new(at_hand, current);
        match (&self.piper, &self.catalog) {
            (Err(why), _) => list.trouble = Some((*why).into()),
            (Ok(_), Catalog::Fetched(more)) => list.add(more),
            (Ok(_), Catalog::Fetching(_)) => {}
            (Ok(_), Catalog::Unfetched) => {
                self.catalog = Catalog::Fetching(fetch_catalog(&self.root))
            }
        }
        self.list = Some(list);
    }

    pub fn is_open(&self) -> bool {
        self.list.is_some()
    }

    pub fn key(&mut self, key: KeyEvent) -> Option<Event> {
        let list = self.list.as_mut()?;
        match list.menu.key(key, list.shown.len()) {
            menu::Key::Typed(_) | menu::Key::Retyped => list.refresh(),
            menu::Key::Moved | menu::Key::Other => {}
            menu::Key::Quit => self.list = None,
            menu::Key::Enter => {
                let voice = list.picked()?.clone();
                self.list = None;
                return self.pick(voice);
            }
        }
        None
    }

    /// Read in `voice`, once it is downloaded if it must be.
    fn pick(&mut self, voice: Voice) -> Option<Event> {
        let remote = match voice.how {
            How::Ready(command) => return Some(Event::Use(voice.name, command)),
            How::Fetch(remote) => remote,
        };
        if self.download.is_some() {
            return Some(Event::Notice("one voice is downloaded at a time".into()));
        }
        let dir = match &self.piper {
            Ok(dir) => dir,
            Err(why) => return Some(Event::Notice((*why).into())),
        };
        self.download = Some(Download::start(&voice.name, &remote, &self.root, dir));
        None
    }

    /// Note the catalog fetched, and a voice downloaded. Whether to draw
    /// again, and what is called for.
    pub fn poll(&mut self) -> (bool, Option<Event>) {
        let mut redraw = false;
        if let Catalog::Fetching(task) = &self.catalog
            && let Some(fetched) = task.done()
        {
            match fetched {
                Ok(voices) => {
                    // Not to be offered where they cannot be downloaded.
                    if let Some(list) = &mut self.list
                        && self.piper.is_ok()
                    {
                        list.add(&voices);
                    }
                    self.catalog = Catalog::Fetched(voices);
                }
                Err(why) => {
                    if let Some(list) = &mut self.list {
                        list.trouble = Some(format!("cannot fetch Piper's voices: {why}"));
                    }
                    // Fetched again the next time the list opens.
                    self.catalog = Catalog::Unfetched;
                }
            }
            redraw = true;
        }
        let Some(download) = &mut self.download else {
            return (redraw, None);
        };
        let Some(done) = download.task.done() else {
            let now = download.percent();
            redraw |= now != download.shown;
            download.shown = now;
            return (redraw, None);
        };
        let name = std::mem::take(&mut download.name);
        self.download = None;
        let event = match done {
            Ok(command) => Event::Use(name, command),
            Err(why) => Event::Notice(format!("cannot download {name}: {why}")),
        };
        (true, Some(event))
    }

    /// How far the voice being downloaded is, for the status line.
    pub fn note(&self) -> Option<String> {
        let download = self.download.as_ref()?;
        Some(format!(
            " · downloading {} {}%",
            download.name, download.shown
        ))
    }

    pub fn draw(&self, f: &mut Frame, area: Rect) {
        if let Some(list) = &self.list {
            let fetching = matches!(self.catalog, Catalog::Fetching(_));
            list.draw(f, area, fetching);
        }
    }
}

impl List {
    fn new(voices: Vec<Voice>, current: Option<&str>) -> Self {
        let mut list = List {
            menu: Menu::default(),
            voices,
            shown: Vec::new(),
            current: current.map(String::from),
            trouble: None,
        };
        list.refresh();
        let at = (list.shown.iter()).position(|&i| list.is_current(&list.voices[i]));
        list.menu.pick(at.or(Some(0)));
        list
    }

    /// Add the voices of Piper's catalog not already listed, keeping the one
    /// picked.
    fn add(&mut self, catalog: &[Voice]) {
        let listed: HashSet<&str> = self.voices.iter().map(|v| v.name.as_str()).collect();
        let more: Vec<Voice> = (catalog.iter())
            .filter(|v| !listed.contains(v.name.as_str()))
            .cloned()
            .collect();
        let picked = self.picked().map(|v| v.name.clone());
        self.voices.extend(more);
        self.refresh();
        let at =
            picked.and_then(|name| self.shown.iter().position(|&i| self.voices[i].name == name));
        self.menu.pick(at.or(Some(0)));
    }

    fn is_current(&self, voice: &Voice) -> bool {
        matches!(&voice.how, How::Ready(c) if Some(c) == self.current.as_ref())
    }

    fn picked(&self) -> Option<&Voice> {
        Some(&self.voices[*self.shown.get(self.menu.picked()?)?])
    }

    /// List the voices the query names, best first: by name, then by what
    /// they are.
    fn refresh(&mut self) {
        let query = self.menu.query.to_lowercase();
        let mut ranked: Vec<(u8, usize)> = (self.voices.iter().enumerate())
            .filter_map(|(i, v)| {
                let by_name = rank(&v.name.to_lowercase(), &query);
                let by_about = || rank(&v.about.to_lowercase(), &query).map(|r| r + 3);
                Some((by_name.or_else(by_about)?, i))
            })
            .collect();
        ranked.sort_by_key(|&(rank, _)| rank);
        self.shown = ranked.into_iter().map(|(_, i)| i).collect();
        self.menu.pick((!self.shown.is_empty()).then_some(0));
    }

    fn draw(&self, f: &mut Frame, area: Rect, fetching: bool) {
        let prompt = vec![Span::styled(
            " voice ",
            Style::new().add_modifier(Modifier::BOLD),
        )];
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
                How::Fetch(_) => (format!("{} · download", voice.about), dim()),
            };
            ListItem::new(Line::from(vec![
                Span::raw(format!("{mark}{:width$}  ", voice.name)),
                Span::styled(about, dim()),
            ]))
            .style(style)
        });
        let mut notes = Vec::new();
        if fetching {
            notes.push(Span::styled(" fetching Piper's voices… ", dim()));
        } else if let Some(trouble) = &self.trouble {
            notes.push(menu::trouble(trouble));
        }
        if self.shown.is_empty() {
            notes.push(Span::styled(" no match ", dim()));
        }
        let keys = "type to narrow · ↑/↓ move · enter read in it · esc cancel";
        self.menu.draw(f, area, prompt, items, notes, keys);
    }
}

#[cfg(test)]
#[path = "tests/voices.rs"]
mod tests;
