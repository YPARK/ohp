//! Choosing the voice slides are read in, on `v`, in a list over the slides.
//!
//! Listed are the voice given, Piper's voices kept where its voices go, and
//! other voices found, as eSpeak NG; on macOS, each of its own voices; then,
//! once fetched, the voices in Piper's catalog not yet kept, the user's
//! language first. Only voices that sound natural are listed, not macOS's
//! robotic ones, nor Piper's of low quality. The catalog is sherpa-onnx's
//! copies of Piper's voices, which sherpa-onnx speaks, and so does Piper.
//! It is fetched once, with curl, off the UI thread, and kept for the next
//! time the list opens; so are macOS's voices, which `say` takes a moment
//! to list. Typing narrows the list as the file picker's does. A voice not
//! yet kept is downloaded first, its progress in the status line, and read
//! in once it is whole.
//!
//! Without sherpa-onnx or Piper, the list offers to install sherpa-onnx:
//! its release for the machine, a program and the libraries it needs,
//! beside Piper's voices. Once it is, the list opens again, on the voices
//! to download.

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

/// sherpa-onnx's release of voices, as GitHub's API lists it, and where its
/// files are downloaded from.
const PIPER_VOICES_LISTED: &str =
    "https://api.github.com/repos/k2-fsa/sherpa-onnx/releases/tags/tts-models";
const PIPER_VOICES: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models";
/// The release of sherpa-onnx installed.
const SHERPA: &str = "1.13.8";

/// Where sherpa-onnx's release `build` is downloaded from.
fn sherpa_url(build: &str) -> String {
    format!(
        "https://github.com/k2-fsa/sherpa-onnx/releases/download/v{SHERPA}/\
         sherpa-onnx-v{SHERPA}-{build}-shared.tar.bz2"
    )
}

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
    /// sherpa-onnx, to install so Piper's voices can be downloaded.
    Install,
}

/// A voice in Piper's catalog: its archive, a file under the catalog's
/// root, and its size.
#[derive(Clone, Debug, PartialEq)]
struct Remote {
    file: String,
    size: u64,
}

/// The voice slides are read in unless one is picked: the one given, or
/// else the first at hand.
pub fn default(given: Option<String>) -> Option<String> {
    speak::given_or(given, || {
        let dir = speak::piper_dir();
        let engines = speak::Engines::found(dir.as_deref());
        let voices = at_hand(None, dir.as_deref(), &engines, &speak::others());
        voices.into_iter().find_map(|v| match v.how {
            How::Ready(command) => Some(command),
            How::Fetch(_) | How::Install => None,
        })
    })
}

/// The family of `language`, as `en` for `en_GB.UTF-8`.
fn family(language: &str) -> &str {
    (language.split(['_', '.', '-', '@']).next()).unwrap_or_default()
}

/// macOS's voices no one would read slides in: its sound effects, its
/// first voices, and Eloquence's, in each language, as `Eddy (English
/// (US))`, all robotic.
const ROBOTIC: &[&str] = &[
    "Albert",
    "Bad News",
    "Bahh",
    "Bells",
    "Boing",
    "Bubbles",
    "Cellos",
    "Eddy",
    "Flo",
    "Fred",
    "Good News",
    "Grandma",
    "Grandpa",
    "Jester",
    "Junior",
    "Kathy",
    "Organ",
    "Ralph",
    "Reed",
    "Rocko",
    "Sandy",
    "Shelley",
    "Superstar",
    "Trinoids",
    "Whisper",
    "Wobble",
    "Zarvox",
];

/// Piper's voices that sound least natural: the CMU Arctic sets', flat or
/// strongly accented, and a game's robot.
const UNNATURAL: &[&str] = &["arctic", "l2arctic", "glados"];

/// The English voices listed, of the many there are, the most natural:
/// Piper's speakers, and macOS's, besides its Premium and Enhanced ones.
const ENGLISH_PIPER: &[&str] = &["lessac", "ryan", "amy", "alan", "cori", "jenny_dioco"];
const ENGLISH_MACOS: &[&str] = &["Samantha", "Daniel"];

/// macOS's voices in `listing`, as `say -v '?'` gives it, each speaking with
/// `command` of its name, those in `language`'s family first; the robotic
/// left out, and in English, all but `ENGLISH_MACOS` and those downloaded
/// as Premium or Enhanced.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn macos(listing: &str, language: &str, command: impl Fn(&str) -> String) -> Vec<Voice> {
    let voices: Vec<(bool, Voice)> = (listing.lines())
        .filter_map(|line| {
            // Its name, which may have spaces, its locale, and `#` a sample.
            let (named, _) = line.split_once('#')?;
            let (name, locale) = named.trim_end().rsplit_once(char::is_whitespace)?;
            let name = name.trim();
            let base = name.split(" (").next().unwrap_or(name);
            let downloaded = name.ends_with("(Premium)") || name.ends_with("(Enhanced)");
            let picked = family(locale) != "en" || ENGLISH_MACOS.contains(&base) || downloaded;
            if ROBOTIC.contains(&base) || !picked {
                return None;
            }
            let voice = Voice {
                name: name.into(),
                about: format!("macOS · {locale}"),
                how: How::Ready(command(name)),
            };
            (!name.is_empty()).then(|| (family(locale) != family(language), voice))
        })
        .collect();
    language_first(voices)
}

/// `voices`, each noted whether in another language than the user's, those
/// in the user's first, each by name.
fn language_first(mut voices: Vec<(bool, Voice)>) -> Vec<Voice> {
    voices.sort_by(|(a, va), (b, vb)| (a, &va.name).cmp(&(b, &vb.name)));
    voices.into_iter().map(|(_, v)| v).collect()
}

/// List macOS's own voices, with `say`.
#[cfg(target_os = "macos")]
fn list_macos() -> Task<Vec<Voice>> {
    Task::spawn(|_| {
        let listed = Command::new("say")
            .args(["-v", "?"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|e| e.to_string())?;
        let listing = String::from_utf8_lossy(&listed.stdout);
        Ok(macos(&listing, &language(), speak::say_in))
    })
}

/// `path` as it is while being made.
fn part(path: &Path) -> PathBuf {
    let mut part = path.as_os_str().to_owned();
    part.push(".part");
    PathBuf::from(part)
}

/// Unpack `archive`, a `.tar.bz2` of one directory, as `into`.
fn untar(archive: &Path, into: &Path, stop: &AtomicBool) -> Result<(), String> {
    std::fs::create_dir_all(into).map_err(|e| format!("cannot make {}: {e}", into.display()))?;
    let mut tar = Command::new("tar");
    tar.arg("-xjf")
        .arg(archive)
        .arg("-C")
        .arg(into)
        .args(["--strip-components", "1"]);
    run(tar, into, stop)
}

/// Run `command` on no input, its output dropped, until it is done or
/// `stop` is set; what it says when it fails is logged beside `at` until
/// then.
fn run(mut command: Command, at: &Path, stop: &AtomicBool) -> Result<(), String> {
    command.stdin(Stdio::null()).stdout(Stdio::null());
    let log = at.with_extension("log");
    let ran = child::run_logged(command, &log, stop);
    let _ = std::fs::remove_file(&log);
    if ran? { Ok(()) } else { Err("stopped".into()) }
}

/// Install sherpa-onnx's release at `url` beside Piper's voices in `dir`:
/// its program speaking, and the libraries it needs. Stopped or failed,
/// nothing is left half made.
fn install_sherpa(dir: &Path, url: &str) -> Task<()> {
    let (dir, url) = (dir.to_path_buf(), url.to_string());
    Task::spawn(move |stop| {
        let home = speak::sherpa_home(&dir);
        let (archive, staged) = (part(&home.with_extension("tar.bz2")), part(&home));
        let installed = std::fs::create_dir_all(&dir)
            .map_err(|e| format!("cannot make {}: {e}", dir.display()))
            .and_then(|()| fetch(&url, &archive, stop))
            .and_then(|()| untar(&archive, &staged, stop))
            .and_then(|()| {
                let program = speak::sherpa_program(&staged);
                if !program.is_file() {
                    return Err(format!("no {} in it", program.display()));
                }
                // Only the program speaking is wanted, of its many.
                let _ = std::fs::remove_dir_all(staged.join("include"));
                for other in std::fs::read_dir(staged.join("bin"))
                    .into_iter()
                    .flatten()
                    .flatten()
                {
                    if other.path() != program {
                        let _ = std::fs::remove_file(other.path());
                    }
                }
                let _ = std::fs::remove_dir_all(&home);
                std::fs::rename(&staged, &home).map_err(|e| e.to_string())
            });
        let _ = std::fs::remove_file(&archive);
        if installed.is_err() {
            let _ = std::fs::remove_dir_all(&staged);
        }
        installed
    })
}

/// The voices at hand: `given`, Piper's in `dir`, its voices' directory,
/// if `engines` speak them, and `others` found, as `speak::others` gives
/// them.
fn at_hand(
    given: Option<&str>,
    dir: Option<&Path>,
    engines: &speak::Engines,
    others: &[(&str, &str)],
) -> Vec<Voice> {
    let ready = |name: &str, about: &str, command: String| Voice {
        name: name.into(),
        about: about.into(),
        how: How::Ready(command),
    };
    let mut voices = Vec::new();
    for model in dir.map(speak::piper_models).unwrap_or_default() {
        if let (Some(stem), Some(command)) = (
            model.file_stem().and_then(|s| s.to_str()),
            engines.voice(&model),
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

/// Piper's voices in sherpa-onnx's `release` of them, as GitHub's API gives
/// it, those in `language`'s family first, as `en` for `en_GB.UTF-8`. Each
/// is an archive named for it, as `vits-piper-en_GB-alan-medium.tar.bz2`.
/// Only the natural sounding are listed: of medium or high quality, not
/// made smaller, not of `UNNATURAL`, and in English, of `ENGLISH_PIPER`.
fn catalog(release: &str, language: &str) -> Result<Vec<Voice>, String> {
    let release: Value =
        serde_json::from_str(release).map_err(|e| format!("cannot read Piper's voices: {e}"))?;
    // Refused, as when asked too often, GitHub says why.
    let assets = (release["assets"].as_array()).ok_or_else(|| {
        let why = release["message"].as_str().unwrap_or("none listed");
        format!("cannot read Piper's voices: {why}")
    })?;
    let mine = family(language);
    let voices: Vec<(bool, Voice)> = (assets.iter())
        .filter_map(|asset| {
            let file = asset["name"].as_str()?;
            let name = file.strip_prefix("vits-piper-")?.strip_suffix(".tar.bz2")?;
            let mut parts = name.split('-');
            let (locale, speaker, quality) = (parts.next()?, parts.next()?, parts.next()?);
            let natural = ["medium", "high"].contains(&quality)
                && !UNNATURAL.contains(&speaker)
                && (family(locale) != "en" || ENGLISH_PIPER.contains(&speaker));
            if !natural || parts.next().is_some() {
                return None;
            }
            let size = asset["size"].as_u64()?;
            let voice = Voice {
                name: name.into(),
                about: format!("{locale} · {quality} · {} MB", size.div_ceil(1_000_000)),
                how: How::Fetch(Remote {
                    file: file.into(),
                    size,
                }),
            };
            Some((family(locale) != mine, voice))
        })
        .collect();
    Ok(language_first(voices))
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
        .arg(url);
    run(curl, to, stop)
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

/// Fetch Piper's catalog, listed at `url`, and read its voices.
fn fetch_catalog(url: &str) -> Task<Vec<Voice>> {
    let url = url.to_string();
    Task::spawn(move |stop| {
        let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
        let to = dir.path().join("release.json");
        fetch(&url, &to, stop)?;
        let json = std::fs::read_to_string(&to).map_err(|e| e.to_string())?;
        catalog(&json, &language())
    })
}

/// A voice of Piper's being downloaded.
struct Download {
    name: String,
    /// Its archive as it is downloaded, and its directory once unpacked.
    archive: PathBuf,
    voice: PathBuf,
    /// The archive's size, whole.
    total: u64,
    /// How much of it was last shown, in percent.
    shown: u8,
    task: Task<String>,
}

impl Download {
    /// Download `voice` from `root` into a directory of its own in `dir`,
    /// to be spoken by `engines`.
    fn start(
        name: &str,
        remote: &Remote,
        root: &str,
        dir: &Path,
        engines: &speak::Engines,
    ) -> Download {
        let archive = part(&dir.join(&remote.file));
        let voice = dir.join(remote.file.strip_suffix(".tar.bz2").unwrap_or(&remote.file));
        let (url, size, engines) = (
            format!("{root}/{}", remote.file),
            remote.size,
            engines.clone(),
        );
        let (to, into) = (archive.clone(), voice.clone());
        let task = Task::spawn(move |stop| download(&url, size, &to, &into, &engines, stop));
        Download {
            name: name.into(),
            archive,
            voice,
            total: remote.size,
            shown: 0,
            task,
        }
    }

    /// How much is downloaded, in percent.
    fn percent(&self) -> u8 {
        if self.voice.is_dir() {
            return 100;
        }
        let got = std::fs::metadata(&self.archive).map_or(0, |m| m.len());
        (got * 100 / self.total.max(1)).min(100) as u8
    }
}

/// Download the archive at `url`, `size` long, to `archive`, and unpack it
/// as `voice`, to be spoken by `engines`: the command to speak in it. The
/// archive is not kept, nor a voice half unpacked.
fn download(
    url: &str,
    size: u64,
    archive: &Path,
    voice: &Path,
    engines: &speak::Engines,
    stop: &AtomicBool,
) -> Result<String, String> {
    let dir = voice.parent().unwrap_or(Path::new("."));
    let staged = part(voice);
    let placed = std::fs::create_dir_all(dir)
        .map_err(|e| format!("cannot make {}: {e}", dir.display()))
        .and_then(|()| fetch(url, archive, stop))
        .and_then(|()| {
            let len = std::fs::metadata(archive).map_or(0, |m| m.len());
            if len == size {
                Ok(())
            } else {
                Err(format!("{url} came short: {len} of {size} bytes"))
            }
        })
        .and_then(|()| untar(archive, &staged, stop))
        .and_then(|()| {
            let _ = std::fs::remove_dir_all(voice);
            std::fs::rename(&staged, voice).map_err(|e| e.to_string())
        });
    let _ = std::fs::remove_file(archive);
    if let Err(e) = placed {
        let _ = std::fs::remove_dir_all(&staged);
        return Err(e);
    }
    let model = speak::piper_models(voice).into_iter().next();
    let Some(model) = model else {
        let _ = std::fs::remove_dir_all(voice);
        return Err(format!("no voice in {url}"));
    };
    engines
        .voice(&model)
        .ok_or_else(|| "nothing to speak it".into())
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
    /// Where Piper's catalog is listed, and its voices downloaded from.
    listed: String,
    root: String,
    /// Where sherpa-onnx's release for this machine is, if it has one.
    sherpa: Option<String>,
    list: Option<List>,
    catalog: Catalog,
    /// macOS's own voices.
    system: Catalog,
    /// Where Piper's voices go, with what speaks them and curl installed to
    /// fetch them.
    piper: Result<PathBuf, &'static str>,
    /// What speaks Piper's voices.
    engines: speak::Engines,
    /// Where sherpa-onnx is to be installed, where nothing speaks Piper's
    /// voices and it could be.
    installable: Option<PathBuf>,
    install: Option<Task<()>>,
    download: Option<Download>,
    /// The command of the voice in use when the list last opened.
    current: Option<String>,
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
        let sherpa = speak::sherpa_build().map(sherpa_url);
        Voices::new(PIPER_VOICES_LISTED, PIPER_VOICES, sherpa)
    }
}

impl Voices {
    /// A picker of Piper's voices `listed`, downloaded from `root`, and of
    /// sherpa-onnx's release at `sherpa`, to speak them.
    fn new(listed: &str, root: &str, sherpa: Option<String>) -> Self {
        Voices {
            listed: listed.into(),
            root: root.into(),
            sherpa,
            list: None,
            catalog: Catalog::Unfetched,
            system: Catalog::Unfetched,
            piper: Err(""),
            engines: speak::Engines::default(),
            installable: None,
            install: None,
            download: None,
            current: None,
        }
    }

    /// Open the list, `current` the command of the voice in use.
    pub fn open(&mut self, current: Option<&str>) {
        self.current = current.map(String::from);
        let dir = speak::piper_dir();
        self.engines = speak::Engines::found(dir.as_deref());
        let at_hand = at_hand(current, dir.as_deref(), &self.engines, &speak::others());
        let curl = speak::installed("curl");
        self.piper = match &dir {
            _ if !self.engines.any() => {
                Err("install sherpa-onnx or Piper to download Piper's voices")
            }
            _ if !curl => Err("install curl to download Piper's voices"),
            None => Err("no place for Piper's voices: HOME is not set"),
            Some(dir) => Ok(dir.clone()),
        };
        let can = curl && self.sherpa.is_some() && speak::installed("tar");
        self.installable = dir.filter(|_| !self.engines.any() && can);
        let mut list = List::new(at_hand, current);
        if self.installable.is_some() {
            list.add(&[Voice {
                name: "sherpa-onnx".into(),
                about: "install, 20 MB, to download Piper's natural voices in many languages"
                    .into(),
                how: How::Install,
            }]);
        }
        match &self.system {
            Catalog::Fetched(voices) => list.add(voices),
            Catalog::Fetching(_) => {}
            #[cfg(target_os = "macos")]
            Catalog::Unfetched if speak::installed("say") => {
                self.system = Catalog::Fetching(list_macos());
            }
            Catalog::Unfetched => self.system = Catalog::Fetched(Vec::new()),
        }
        match (&self.piper, &self.catalog) {
            (Err(_), _) if self.install.is_some() => {
                list.trouble = Some("installing sherpa-onnx…".into())
            }
            (Err(_), _) if self.installable.is_some() => {}
            (Err(why), _) => list.trouble = Some((*why).into()),
            (Ok(_), Catalog::Fetched(more)) => list.add(more),
            (Ok(_), Catalog::Fetching(_)) => {}
            (Ok(_), Catalog::Unfetched) => {
                self.catalog = Catalog::Fetching(fetch_catalog(&self.listed))
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
            How::Install if self.install.is_some() => {
                return Some(Event::Notice("sherpa-onnx is being installed".into()));
            }
            How::Install => {
                let (dir, url) = (self.installable.as_ref()?, self.sherpa.as_ref()?);
                self.install = Some(install_sherpa(dir, url));
                return None;
            }
        };
        if self.download.is_some() {
            return Some(Event::Notice("one voice is downloaded at a time".into()));
        }
        let dir = match &self.piper {
            Ok(dir) => dir,
            Err(why) => return Some(Event::Notice((*why).into())),
        };
        self.download = Some(Download::start(
            &voice.name,
            &remote,
            &self.root,
            dir,
            &self.engines,
        ));
        None
    }

    /// Note the catalog fetched, and a voice downloaded. Whether to draw
    /// again, and what is called for.
    pub fn poll(&mut self) -> (bool, Option<Event>) {
        let mut redraw = false;
        if let Catalog::Fetching(task) = &self.system
            && let Some(listed) = task.done()
        {
            // Without them, the voice chosen in macOS's settings is still listed.
            let voices = listed.unwrap_or_default();
            if let Some(list) = &mut self.list {
                list.add(&voices);
            }
            self.system = Catalog::Fetched(voices);
            redraw = true;
        }
        if let Some(task) = &self.install
            && let Some(installed) = task.done()
        {
            self.install = None;
            let event = match installed {
                Ok(()) => {
                    // Open again, on Piper's voices to download.
                    let current = self.current.take();
                    self.open(current.as_deref());
                    Event::Notice(
                        "sherpa-onnx is installed: pick one of Piper's voices to download".into(),
                    )
                }
                Err(why) => Event::Notice(format!("cannot install sherpa-onnx: {why}")),
            };
            return (true, Some(event));
        }
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
        if self.install.is_some() {
            return Some(" · installing sherpa-onnx…".into());
        }
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
                How::Install => (voice.about.clone(), Style::new()),
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
