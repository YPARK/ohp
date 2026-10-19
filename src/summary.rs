//! Pages summed up by a language model run in ohp, and the sentences that
//! carry their key takeaways marked on them; and the whole document summed
//! up, its key sentences marked wherever they are.
//!
//! The model is one of the GGUF files on Hugging Face's Hub that `llm`
//! runs, Qwen2.5-3B-Instruct's on macOS and Qwen2.5-1.5B-Instruct's
//! elsewhere, unless another is given or picked with `m`. It is downloaded
//! with curl into Hugging Face's cache, as `hub` does, the first time it is
//! wanted, and loaded once, on a thread of its own that does all the
//! summing up, one thing at a time. The page on screen comes first, then
//! the document, then the pages after the one on screen, so they are
//! summed up before they are turned to. What is no longer wanted, as the
//! page turned from or a model picked over, is given up, its download too,
//! and taken up again if it is wanted again. How far each step has come,
//! downloading, loading, reading the prompt and writing the answer, is
//! shown as it goes.
//!
//! A page is given to the model as the sentences read aloud from it,
//! numbered; it answers with a summary and the numbers of the key
//! sentences, which are marked on the page. The whole of a paper would not
//! fit in the model's context, nor be read in good time, so the document is
//! summed up from the sentences most central to it, as `salient` picks
//! them by what they mean, as `embed` has it, or by their words where its
//! model cannot be had: retrieved, and then given to the model as a page's
//! are. Summaries are kept by the model and the sentences summed up, in
//! memory and in the user's cache directory, so a page, or a file, a
//! reload leaves as it was is not summed up again, nor one read before.

// Built without the model, it sums nothing up, but says why.
#![cfg_attr(not(feature = "summary"), allow(dead_code))]

#[cfg(feature = "summary")]
use crate::embed::{self, Embedder};
#[cfg(feature = "summary")]
use crate::hub;
#[cfg(feature = "summary")]
use crate::llm::Llm;
use crate::salient::{self, Vectors};
use crate::text::{self, Picked};
use hayro::hayro_interpret::{InterpreterCache, InterpreterSettings};
use hayro::hayro_syntax::Pdf;
use std::collections::HashMap;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Name of the thread summing up, so its panics can be told apart.
pub const THREAD: &str = "summary";

pub const NO_SUMMARY: &str = "ohp was built without its `summary` feature";

/// Most text given to the model at once, in bytes: about two thousand
/// tokens, read in a few seconds.
const TEXT: usize = 8000;
/// Most tokens of a page's summary, and of the document's.
const PAGE_ANSWER: usize = 240;
const WHOLE_ANSWER: usize = 400;
/// Most key sentences a page's summary picks, and the document's.
const PAGE_KEY: usize = 3;
const WHOLE_KEY: usize = 5;
/// Pages after the one on screen summed up while it is read.
const AHEAD: usize = 2;
/// Longest ohp waits, quitting, for summing up to stop.
const QUIT_WAIT: Duration = Duration::from_secs(1);
/// Fewest words of a key sentence: fewer are a piece of one, as the end of
/// a sentence broken across a column.
const KEY_WORDS: usize = 4;

const SYSTEM: &str = "You sum up slides and papers for a researcher reading them. \
                      Say only what the text says, plainly and briefly.";

/// How far the model's answer has come, as it is given.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step<'a> {
    /// Reading the prompt: tokens read, of how many.
    Reading(usize, usize),
    /// Writing the answer: what it says so far, of how many tokens.
    Writing(&'a str, usize),
}

/// A model ohp suggests: its name, its GGUF file on Hugging Face's Hub, how
/// large it is, and what it is like.
#[derive(Debug, PartialEq)]
pub struct Suggested {
    pub name: &'static str,
    pub repo: &'static str,
    pub file: &'static str,
    pub gb: f32,
    pub about: &'static str,
}

pub const SUGGESTED: [Suggested; 5] = [
    Suggested {
        name: "qwen2.5-3b",
        repo: "Qwen/Qwen2.5-3B-Instruct-GGUF",
        file: "qwen2.5-3b-instruct-q4_k_m.gguf",
        gb: 2.1,
        about: "apt and quick on a GPU",
    },
    Suggested {
        name: "qwen2.5-1.5b",
        repo: "Qwen/Qwen2.5-1.5B-Instruct-GGUF",
        file: "qwen2.5-1.5b-instruct-q4_k_m.gguf",
        gb: 1.1,
        about: "twice as quick, its picks less apt",
    },
    Suggested {
        name: "qwen3-4b",
        repo: "unsloth/Qwen3-4B-Instruct-2507-GGUF",
        file: "Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        gb: 2.5,
        about: "newer, and a little slower",
    },
    Suggested {
        name: "llama3.2-3b",
        repo: "bartowski/Llama-3.2-3B-Instruct-GGUF",
        file: "Llama-3.2-3B-Instruct-Q4_K_M.gguf",
        gb: 2.0,
        about: "Meta's",
    },
    Suggested {
        name: "qwen2.5-7b",
        repo: "bartowski/Qwen2.5-7B-Instruct-GGUF",
        file: "Qwen2.5-7B-Instruct-Q4_K_M.gguf",
        gb: 4.7,
        about: "the best of these, and the slowest",
    },
];

/// The model pages are summed up with unless one is given: the larger
/// where a GPU runs it, the quicker on a CPU.
fn default_model() -> &'static Suggested {
    if cfg!(target_os = "macos") {
        &SUGGESTED[0]
    } else {
        &SUGGESTED[1]
    }
}

/// The weights of the model to sum up with.
#[derive(Clone, Debug, PartialEq)]
pub enum Weights {
    /// A GGUF file of a repo on Hugging Face's Hub, or else the one `gguf`
    /// picks of its files.
    Hub { repo: String, file: Option<String> },
    /// A GGUF file here.
    File(PathBuf),
}

impl Default for Weights {
    fn default() -> Self {
        Weights::from(default_model())
    }
}

impl From<&Suggested> for Weights {
    fn from(model: &Suggested) -> Self {
        Weights::Hub {
            repo: model.repo.into(),
            file: Some(model.file.into()),
        }
    }
}

impl Weights {
    /// What the model is called: its name if ohp suggests it, else its file,
    /// or its repo.
    pub fn name(&self) -> String {
        if let Some(model) = SUGGESTED.iter().find(|m| Weights::from(*m) == *self) {
            return model.name.into();
        }
        match self {
            Weights::Hub {
                file: Some(file), ..
            } => file.trim_end_matches(".gguf").into(),
            Weights::Hub { repo, file: None } => repo.clone(),
            Weights::File(path) => crate::render::name(path),
        }
    }
}

/// The weights `s` names: a model ohp suggests, by name, a GGUF file here,
/// or a repo on Hugging Face's Hub, as `hub_named` reads it.
pub fn weights(s: &str) -> Result<Weights, String> {
    if let Some(model) = SUGGESTED.iter().find(|m| m.name.eq_ignore_ascii_case(s)) {
        return Ok(Weights::from(model));
    }
    let path = PathBuf::from(s);
    if path.is_file() {
        return Ok(Weights::File(path));
    }
    hub_named(s).ok_or_else(|| {
        let names: Vec<_> = SUGGESTED.iter().map(|m| m.name).collect();
        format!(
            "`{s}` is neither a GGUF file, a repo on Hugging Face as `owner/repo`, nor one of {}",
            names.join(", ")
        )
    })
}

/// The repo on Hugging Face's Hub, and its GGUF file, `s` names: as
/// `owner/repo` or `owner/repo/file.gguf`, or as the Hub's page for either.
pub fn hub_named(s: &str) -> Option<Weights> {
    let s = s.trim();
    let s = ["https://", "http://"]
        .iter()
        .fold(s, |s, scheme| s.strip_prefix(scheme).unwrap_or(s));
    let s = ["huggingface.co/", "hf.co/"]
        .iter()
        .fold(s, |s, host| s.strip_prefix(host).unwrap_or(s));
    let named = |part: &str| {
        !part.is_empty()
            && !part.starts_with('.')
            && (part.chars()).all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    };
    let mut parts = s.splitn(3, '/');
    let (owner, name) = (parts.next()?, parts.next()?);
    if !named(owner) || !named(name) {
        return None;
    }
    let file = match parts.next() {
        None => None,
        Some(rest) => {
            let rest = ["resolve/main/", "blob/main/", "tree/main/"]
                .iter()
                .fold(rest, |rest, at| rest.strip_prefix(at).unwrap_or(rest));
            if !is_gguf(rest) {
                return None;
            }
            Some(rest.to_string())
        }
    };
    Some(Weights::Hub {
        repo: format!("{owner}/{name}"),
        file,
    })
}

/// Whether `file` is a GGUF file, by its name, in any case.
pub fn is_gguf(file: &str) -> bool {
    file.to_ascii_lowercase().ends_with(".gguf")
}

/// Quantisations to take, most liked first: 4 bits a weight is apt and
/// quick enough.
const QUANTS: [&str; 6] = ["q4_k_m", "q4_k_s", "q5_k_m", "q4_0", "q6_k", "q8_0"];

/// The GGUF file of a repo's `files` to sum up with: of one piece, and
/// quantised as most liked.
pub fn gguf<'a>(files: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let files: Vec<&str> = (files.into_iter())
        .filter(|f| {
            let f = f.to_ascii_lowercase();
            f.ends_with(".gguf") && !f.contains("-of-0") && !f.contains("mmproj")
        })
        .collect();
    let quantised = |q: &str| (files.iter()).find(|f| f.to_ascii_lowercase().contains(q));
    QUANTS
        .iter()
        .find_map(|q| quantised(q))
        .or(files.first())
        .copied()
}

/// A key sentence: the page it is on, which of the page's it is, as
/// `text::Page::light` counts them, and what it says.
#[derive(Clone, Debug, PartialEq)]
pub struct Key {
    pub page: usize,
    pub sentence: usize,
    pub text: String,
}

/// A page, or the document, summed up.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Summary {
    /// What it says, in a few sentences.
    pub text: String,
    pub key: Vec<Key>,
}

impl Summary {
    /// The key sentences on `page`.
    fn picked(&self, page: usize) -> Picked {
        Picked::of(
            self.key
                .iter()
                .filter(|k| k.page == page)
                .map(|k| k.sentence),
        )
    }

    /// The summary, its key sentences on `page`, as a page's summary kept
    /// for another is.
    fn on(&self, page: usize) -> Summary {
        let key = self.key.iter().map(|k| Key { page, ..k.clone() });
        Summary {
            text: self.text.clone(),
            key: key.collect(),
        }
    }
}

/// How far a summary has come: what is being done, and how much of it is,
/// if that is known.
#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    pub what: String,
    pub done: Option<f32>,
}

impl Progress {
    fn of(what: String, done: usize, of: usize) -> Progress {
        Progress {
            what,
            done: Some(done as f32 / of.max(1) as f32),
        }
    }

    fn unknown(what: String) -> Progress {
        Progress { what, done: None }
    }
}

/// What to show of a summary: what it says, its key sentences, each with
/// the page it is on, and how far it has come, if it is not done.
#[derive(Debug, Default, PartialEq)]
pub struct View {
    pub text: String,
    pub key: Vec<(usize, String)>,
    pub busy: Option<Progress>,
}

impl View {
    fn of(summary: &Summary, empty: &str) -> View {
        let text = if summary.text.is_empty() {
            empty.into()
        } else {
            summary.text.clone()
        };
        View {
            text,
            key: (summary.key.iter())
                .map(|k| (k.page, k.text.clone()))
                .collect(),
            busy: None,
        }
    }

    fn says(text: &str) -> View {
        View {
            text: text.into(),
            ..View::default()
        }
    }
}

/// What the model is set to do.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Task {
    Page(usize),
    Whole,
}

impl Task {
    /// Most tokens of its answer.
    fn most(self) -> usize {
        match self {
            Task::Page(_) => PAGE_ANSWER,
            Task::Whole => WHOLE_ANSWER,
        }
    }
}

/// What is being done towards a task, and how far it has come.
#[derive(Debug)]
enum Doing {
    /// Reading the document's pages, to pick from: how many, of how many.
    Gathering(usize, usize),
    /// Weighing what the sentences picked from mean: how many, of how many.
    Weighing(usize, usize),
    /// The model reading the prompt: how many tokens, of how many.
    Reading(Task, usize, usize),
    /// The model writing: what it says so far, in how many tokens, since
    /// when.
    Writing(Task, String, usize, Instant),
}

impl Doing {
    fn task(&self) -> Task {
        match self {
            Doing::Gathering(..) | Doing::Weighing(..) => Task::Whole,
            Doing::Reading(task, ..) | Doing::Writing(task, ..) => *task,
        }
    }

    fn progress(&self) -> Progress {
        match self {
            Doing::Gathering(done, of) => Progress::of(
                format!("reading the file · page {} of {of}", done + 1),
                *done,
                *of,
            ),
            Doing::Weighing(done, of) => Progress::of(
                format!("weighing its sentences · {done} of {of}"),
                *done,
                *of,
            ),
            Doing::Reading(_, done, of) => {
                Progress::of(format!("reading · {done} of {of} tokens"), *done, *of)
            }
            Doing::Writing(task, _, tokens, since) => {
                let secs = since.elapsed().as_secs_f32();
                let rate = if secs > 0.5 {
                    format!(" · {:.0} a second", *tokens as f32 / secs)
                } else {
                    String::new()
                };
                Progress::of(
                    format!("writing · {tokens} tokens{rate}"),
                    *tokens,
                    task.most(),
                )
            }
        }
    }
}

/// Where the model is.
#[derive(Default)]
enum Weighed {
    #[default]
    Unloaded,
    /// Its repo's files being listed by the Hub.
    Asking(String),
    /// Loading, by name.
    Loading(String),
    Loaded,
    /// It cannot be had, and why.
    Broken(String),
}

/// A file being downloaded: what it is, where it is kept while it is, and
/// its length, whole.
struct Fetching {
    what: String,
    part: PathBuf,
    size: u64,
}

impl Fetching {
    fn progress(&self) -> Progress {
        let got = std::fs::metadata(&self.part).map_or(0, |m| m.len());
        let gb = |bytes: u64| bytes as f64 / 1e9;
        Progress::of(
            format!(
                "downloading {} · {:.2} of {:.2} GB, once only",
                self.what,
                gb(got),
                gb(self.size)
            ),
            got as usize,
            self.size as usize,
        )
    }
}

type Done = Result<Arc<Summary>, String>;

#[derive(Default)]
struct State {
    /// The deck's PDF and how many pages it has, anew with each reload.
    deck: Option<(Arc<Vec<u8>>, usize)>,
    /// The model to sum up with.
    weights: Weights,
    /// Counts decks and models, so what was done for one is not kept for
    /// the next.
    round: u64,
    /// The page to sum up first, as the one on screen.
    wanted: Option<usize>,
    /// The whole document is to be summed up.
    whole_wanted: bool,
    pages: HashMap<usize, Done>,
    whole: Option<Done>,
    /// Pages' summaries by the sentences summed up, kept from deck to deck.
    known: HashMap<Vec<String>, Arc<Summary>>,
    doing: Option<Doing>,
    running: Option<Running>,
    model: Weighed,
    fetching: Option<Fetching>,
    /// Counts changes, so the screen is drawn again for each.
    version: u64,
    closed: bool,
}

/// The task under way, the round it was begun in, and what tells it to
/// give up.
struct Running {
    task: Task,
    round: u64,
    abandon: Arc<AtomicBool>,
}

impl State {
    /// What the model is to do next, if anything: the page wanted, then
    /// the document, then the pages after the one wanted, to be summed up
    /// before they are turned to.
    fn next(&self) -> Option<Task> {
        let (_, pages) = self.deck.as_ref()?;
        if matches!(self.model, Weighed::Broken(_)) {
            return None;
        }
        let fresh = |p: &usize| p < pages && !self.pages.contains_key(p);
        if let Some(page) = self.wanted.filter(fresh) {
            return Some(Task::Page(page));
        }
        if self.whole_wanted && self.whole.is_none() {
            return Some(Task::Whole);
        }
        let ahead = (self.wanted.into_iter()).flat_map(|w| w + 1..=w + AHEAD);
        ahead.filter(fresh).map(Task::Page).next()
    }

    /// Whether `task`, begun in `round`, is still to be done.
    fn going(&self, task: Task, round: u64) -> bool {
        !self.closed && self.round == round && self.next() == Some(task)
    }

    /// Tell the task under way to give up, if it is no longer to be done:
    /// so a download or a reading stops, not only an answer.
    fn recheck(&self) {
        if let Some(running) = &self.running
            && !self.going(running.task, running.round)
        {
            running.abandon.store(true, Ordering::Relaxed);
        }
    }

    fn changed(&mut self) {
        self.version = self.version.wrapping_add(1);
    }

    /// Begin again: what was done is not what is now wanted.
    fn anew(&mut self) {
        self.round += 1;
        self.pages.clear();
        self.whole = None;
        self.changed();
    }

    /// How far `task` has come, if it is under way, or what it waits for.
    fn progress(&self, task: Task) -> Progress {
        if let Some(fetching) = &self.fetching {
            return fetching.progress();
        }
        match (&self.model, &self.doing) {
            (Weighed::Asking(repo), _) => {
                Progress::unknown(format!("asking Hugging Face for {repo}…"))
            }
            (Weighed::Loading(name), _) => Progress::unknown(format!("loading {name}…")),
            (_, Some(doing)) if doing.task() == task => doing.progress(),
            _ => Progress::unknown("waiting…".into()),
        }
    }

    /// What to show of `task`'s summary, `done` if it is done, `empty` if
    /// it is of nothing.
    fn view(&self, task: Task, done: Option<&Done>, empty: &str) -> View {
        match (done, &self.model) {
            (Some(Ok(summary)), _) => View::of(summary, empty),
            (Some(Err(e)), _) | (None, Weighed::Broken(e)) => View::says(e),
            (None, _) => {
                let text = match &self.doing {
                    Some(Doing::Writing(t, so_far, ..)) if *t == task => so_far.clone(),
                    _ => String::new(),
                };
                View {
                    text,
                    key: Vec::new(),
                    busy: Some(self.progress(task)),
                }
            }
        }
    }
}

type Shared = Arc<(Mutex<State>, Condvar)>;

fn lock(shared: &Shared) -> MutexGuard<'_, State> {
    // A panic summing up is caught before it could poison the lock.
    shared.0.lock().unwrap_or_else(|e| e.into_inner())
}

/// Sums up the deck's pages, and the deck, on a thread started the first
/// time it is asked to.
pub struct Summarizer {
    shared: Shared,
    thread: Option<JoinHandle<()>>,
    /// The page last wanted, the version last seen, and how much of a file
    /// being downloaded was, in thousandths.
    wanted: Option<usize>,
    seen: u64,
    fetched: Option<u16>,
}

impl Summarizer {
    /// A summarizer with `weights`, or else the model ohp picks.
    pub fn new(weights: Option<Weights>) -> Self {
        let state = State {
            weights: weights.unwrap_or_default(),
            ..State::default()
        };
        Summarizer {
            shared: Arc::new((Mutex::new(state), Condvar::new())),
            thread: None,
            wanted: None,
            seen: 0,
            fetched: None,
        }
    }

    /// The model summed up with.
    pub fn weights(&self) -> Weights {
        lock(&self.shared).weights.clone()
    }

    /// Sum up with `weights` from now on, all again.
    pub fn set_weights(&mut self, weights: Weights) {
        let mut state = lock(&self.shared);
        if state.weights == weights {
            return;
        }
        state.weights = weights;
        state.model = Weighed::Unloaded;
        state.known.clear();
        state.anew();
        state.recheck();
        self.shared.1.notify_all();
    }

    /// Sum up the pages of the PDF in `data`, `pages` of them, from now on.
    pub fn set_deck(&mut self, data: Arc<Vec<u8>>, pages: usize) {
        let mut state = lock(&self.shared);
        state.deck = Some((data, pages));
        state.anew();
        state.recheck();
        self.shared.1.notify_all();
    }

    /// Sum up `page` first, or none.
    pub fn want(&mut self, page: Option<usize>) {
        if page == self.wanted {
            return;
        }
        self.wanted = page;
        self.start();
        let mut state = lock(&self.shared);
        state.wanted = page;
        state.recheck();
        self.shared.1.notify_all();
    }

    /// Sum up the whole document, or stop doing so.
    pub fn want_whole(&mut self, on: bool) {
        self.start();
        let mut state = lock(&self.shared);
        if state.whole_wanted != on {
            state.whole_wanted = on;
            state.changed();
            state.recheck();
            self.shared.1.notify_all();
        }
    }

    /// Try again what failed, as the model's download.
    pub fn retry(&mut self) {
        let mut state = lock(&self.shared);
        if matches!(state.model, Weighed::Broken(_)) {
            state.model = Weighed::Unloaded;
        }
        state.pages.retain(|_, done| done.is_ok());
        if matches!(state.whole, Some(Err(_))) {
            state.whole = None;
        }
        state.changed();
        self.shared.1.notify_all();
    }

    fn start(&mut self) {
        if self.thread.is_some() {
            return;
        }
        let shared = self.shared.clone();
        self.thread = std::thread::Builder::new()
            .name(THREAD.into())
            .spawn(move || work(&shared))
            .ok();
    }

    /// Whether anything shown has changed since last asked.
    pub fn changed(&mut self) -> bool {
        let state = lock(&self.shared);
        let fetched = (state.fetching.as_ref())
            .and_then(|f| f.progress().done)
            .map(|done| (done * 1000.) as u16);
        let changed = state.version != self.seen || fetched != self.fetched;
        (self.seen, self.fetched) = (state.version, fetched);
        changed
    }

    /// The key sentences of `page`, once it is summed up.
    pub fn picked(&self, page: usize) -> Picked {
        match lock(&self.shared).pages.get(&page) {
            Some(Ok(summary)) => summary.picked(page),
            _ => Picked::default(),
        }
    }

    /// The document's key sentences on `page`, once it is summed up.
    pub fn picked_whole(&self, page: usize) -> Picked {
        match &lock(&self.shared).whole {
            Some(Ok(summary)) => summary.picked(page),
            _ => Picked::default(),
        }
    }

    /// What to show of `page`'s summary.
    pub fn page(&self, page: usize) -> View {
        let state = lock(&self.shared);
        let done = state.pages.get(&page);
        state.view(Task::Page(page), done, "Nothing on this page to sum up.")
    }

    /// What to show of the whole document's summary.
    pub fn whole(&self) -> View {
        let state = lock(&self.shared);
        let done = state.whole.as_ref();
        state.view(Task::Whole, done, "Nothing in this file to sum up.")
    }
}

/// However ohp ends, summing up stops with it.
impl Drop for Summarizer {
    fn drop(&mut self) {
        {
            let mut state = lock(&self.shared);
            state.closed = true;
            state.recheck();
        }
        self.shared.1.notify_all();
        let Some(thread) = self.thread.take() else {
            return;
        };
        // A download, or an answer, stops at once; loading a model does
        // not, and is left to end with ohp rather than keep it waiting.
        let since = Instant::now();
        while !thread.is_finished() && since.elapsed() < QUIT_WAIT {
            std::thread::sleep(Duration::from_millis(10));
        }
        if thread.is_finished() {
            let _ = thread.join();
        }
    }
}

/// What the model is asked to sum up, and whether it is still to be, as
/// what the screen wants changes.
struct Asked<'a> {
    shared: &'a Shared,
    data: Arc<Vec<u8>>,
    task: Task,
    round: u64,
    /// Set once it is no longer to be, for what cannot ask, as a download.
    abandon: &'a AtomicBool,
}

impl Asked<'_> {
    /// Whether it is still to be summed up; `doing` is what is being done,
    /// to be shown.
    fn still(&self, doing: Doing) -> bool {
        let mut state = lock(self.shared);
        state.doing = Some(doing);
        state.changed();
        state.going(self.task, self.round)
    }

    /// The model summing up, which it was begun with.
    fn weights(&self) -> Weights {
        lock(self.shared).weights.clone()
    }

    /// Whether `task` is still to be done, as the model, at `step`, says
    /// how far it has come; when it begins writing, it is so `since`.
    fn step(&self, task: Task, step: Step, since: &mut Option<Instant>) -> bool {
        self.still(match step {
            Step::Reading(done, of) => Doing::Reading(task, done, of),
            Step::Writing(so_far, tokens) => {
                let since = *since.get_or_insert_with(Instant::now);
                Doing::Writing(task, shown(so_far), tokens, since)
            }
        })
    }

    /// `llm`'s answer to `prompt`, towards `task`, as long as it is still
    /// to be; `None` if given up.
    fn answer(&self, llm: &mut Llm, task: Task, prompt: &str) -> Option<Result<String, String>> {
        let (mut given_up, mut since) = (false, None);
        let answer = llm.answer(SYSTEM, prompt, task.most(), |step| {
            given_up = !self.step(task, step, &mut since);
            !given_up
        });
        (!given_up).then_some(answer)
    }
}

/// The models, once loaded.
#[derive(Default)]
struct Loader {
    llm: Option<(Weights, Llm)>,
    /// The embedding model, or why it cannot be had.
    embedder: Option<Result<Embedder, String>>,
}

/// Sum up what `shared` asks for until it is closed.
fn work(shared: &Shared) {
    let mut loader = Loader::default();
    loop {
        let (task, data, round, abandon) = {
            let mut state = lock(shared);
            loop {
                if state.closed {
                    return;
                }
                if let (Some(task), Some((data, _))) = (state.next(), &state.deck) {
                    let (data, round) = (data.clone(), state.round);
                    let abandon = Arc::new(AtomicBool::new(false));
                    state.running = Some(Running {
                        task,
                        round,
                        abandon: abandon.clone(),
                    });
                    break (task, data, round, abandon);
                }
                state = shared.1.wait(state).unwrap_or_else(|e| e.into_inner());
            }
        };
        let asked = Asked {
            shared,
            data,
            task,
            round,
            abandon: &abandon,
        };
        let done = panic::catch_unwind(AssertUnwindSafe(|| match task {
            Task::Page(page) => sum_page(&asked, &mut loader, page),
            Task::Whole => sum_whole(&asked, &mut loader),
        }));
        let mut state = lock(shared);
        state.running = None;
        state.doing = None;
        state.fetching = None;
        state.changed();
        let done = match done {
            // Given up, to be done later if still wanted.
            _ if abandon.load(Ordering::Relaxed) => continue,
            Ok(Some(done)) => done.map(Arc::new),
            Ok(None) => continue,
            Err(_) => {
                loader.llm = None;
                if matches!(state.model, Weighed::Loaded) {
                    state.model = Weighed::Unloaded;
                }
                Err("the model failed on this".into())
            }
        };
        if state.round != round {
            continue;
        }
        match task {
            Task::Page(page) => {
                state.pages.insert(page, done);
            }
            Task::Whole => state.whole = Some(done),
        }
    }
}

/// Page `page` summed up, as long as it is still to be; `None` if given
/// up.
fn sum_page(asked: &Asked, loader: &mut Loader, page: usize) -> Option<Result<Summary, String>> {
    let Some(text) = text::of(asked.data.clone(), page) else {
        return Some(Err("cannot read this page's text".into()));
    };
    let sentences = text.said();
    if sentences.is_empty() {
        return Some(Ok(Summary::default()));
    }
    if let Some(known) = lock(asked.shared).known.get(&sentences) {
        return Some(Ok(known.on(page)));
    }
    let name = kept_name(&asked.weights(), "page", [sentences.as_slice()]);
    let kept = kept_dir().and_then(|dir| recall(&dir, &name));
    let summary = match kept {
        Some(kept) => Ok(kept.on(page)),
        None => {
            let llm = match loader.llm(asked.shared, asked.abandon) {
                Ok(llm) => llm,
                Err(e) => return Some(Err(e)),
            };
            let numbered: Vec<(usize, &str)> =
                sentences.iter().map(|s| (page, s.as_str())).collect();
            let answer = asked.answer(llm, Task::Page(page), &prompt(&numbered, false))?;
            let summary = answer.map(|answer| summary(&answer, &numbered, &|k| k, PAGE_KEY));
            if let (Ok(summary), Some(dir)) = (&summary, kept_dir()) {
                keep(&dir, &name, summary);
            }
            summary
        }
    };
    if let Ok(summary) = &summary {
        let kept = Arc::new(summary.clone());
        lock(asked.shared).known.insert(sentences, kept);
    }
    Some(summary)
}

/// The document summed up from the sentences most central to it, as long
/// as it is still to be; `None` if given up.
fn sum_whole(asked: &Asked, loader: &mut Loader) -> Option<Result<Summary, String>> {
    let Ok(pdf) = Pdf::new(asked.data.clone()) else {
        return Some(Err("cannot read the file".into()));
    };
    let (cache, settings) = (InterpreterCache::new(), InterpreterSettings::default());
    let mut pages = Vec::new();
    let count = pdf.pages().len();
    for page in 0..count {
        if !asked.still(Doing::Gathering(page, count)) {
            return None;
        }
        let text = crate::render::caught(|| text::extract(&pdf, page, &cache, &settings));
        pages.push(text.map_or_else(Vec::new, |t| t.said()));
    }
    let name = kept_name(&asked.weights(), "whole", pages.iter().map(Vec::as_slice));
    if let Some(kept) = kept_dir().and_then(|dir| recall(&dir, &name)) {
        return Some(Ok(kept));
    }
    let summary = sum_picked(asked, loader, &pages)?;
    if let (Ok(summary), Some(dir)) = (&summary, kept_dir()) {
        keep(&dir, &name, summary);
    }
    Some(summary)
}

/// The document of `pages`, each the sentences read aloud from it, summed
/// up from the sentences most central to it; `None` if given up.
fn sum_picked(
    asked: &Asked,
    loader: &mut Loader,
    pages: &[Vec<String>],
) -> Option<Result<Summary, String>> {
    let candidates = salient::candidates(pages);
    if candidates.is_empty() {
        return Some(Ok(Summary::default()));
    }
    let said: Vec<&str> = (candidates.iter())
        .map(|&(p, k)| pages[p][k].as_str())
        .collect();
    let vectors = match loader.embedder(asked.shared, asked.abandon) {
        Some(embedder) => {
            let weighed =
                embedder.embed(&said, |done| asked.still(Doing::Weighing(done, said.len())));
            match weighed {
                Ok(Some(meanings)) => Vectors::Meanings(meanings),
                Ok(None) => return None,
                Err(_) => salient::tfidf(&said),
            }
        }
        None => salient::tfidf(&said),
    };
    let picked = salient::pick_from(pages, &candidates, &vectors, TEXT);
    let llm = match loader.llm(asked.shared, asked.abandon) {
        Ok(llm) => llm,
        Err(e) => return Some(Err(e)),
    };
    let numbered: Vec<(usize, &str)> = (picked.iter())
        .map(|&(p, k)| (p, pages[p][k].as_str()))
        .collect();
    let answer = asked.answer(llm, Task::Whole, &prompt(&numbered, true))?;
    let sentence = |i: usize| picked[i].1;
    Some(answer.map(|answer| summary(&answer, &numbered, &sentence, WHOLE_KEY)))
}

#[cfg(feature = "summary")]
impl Loader {
    /// The language model wanted now, loaded, and downloaded first if it is
    /// not yet kept; another loaded before is let go first. One given up,
    /// as `stop` says, is not taken for one that cannot be had.
    fn llm(&mut self, shared: &Shared, stop: &AtomicBool) -> Result<&mut Llm, String> {
        let weights = lock(shared).weights.clone();
        if self.llm.as_ref().is_none_or(|(w, _)| *w != weights) {
            self.llm = None;
            let loaded = locate(shared, &weights, stop).and_then(|path| {
                set(shared, Weighed::Loading(weights.name()));
                Llm::load(&path)
            });
            match loaded {
                Ok(llm) => {
                    set(shared, Weighed::Loaded);
                    self.llm = Some((weights, llm));
                }
                Err(e) if stop.load(Ordering::Relaxed) => {
                    set(shared, Weighed::Unloaded);
                    return Err(e);
                }
                Err(e) => {
                    set(shared, Weighed::Broken(e.clone()));
                    return Err(e);
                }
            }
        }
        (self.llm.as_mut().map(|(_, llm)| llm)).ok_or_else(|| "no model".into())
    }

    /// The embedding model, loaded, and downloaded first if it is not yet
    /// kept; none if it cannot be had, as offline, or is given up, as
    /// `stop` says, and so tried again next time.
    fn embedder(&mut self, shared: &Shared, stop: &AtomicBool) -> Option<&Embedder> {
        if self.embedder.is_none() {
            let loaded = fetch_all(shared, embed::REPO, &embed::FILES, stop)
                .and_then(|dir| Embedder::load(&dir));
            if loaded.is_ok() || !stop.load(Ordering::Relaxed) {
                self.embedder = Some(loaded);
            }
        }
        self.embedder.as_ref()?.as_ref().ok()
    }
}

#[cfg(feature = "summary")]
fn set(shared: &Shared, model: Weighed) {
    let mut state = lock(shared);
    state.model = model;
    state.changed();
}

/// Where the model in `weights` is, downloading it into Hugging Face's
/// cache first if it is not kept there.
#[cfg(feature = "summary")]
fn locate(shared: &Shared, weights: &Weights, stop: &AtomicBool) -> Result<PathBuf, String> {
    let (repo, file) = match weights {
        Weights::File(path) => return Ok(path.clone()),
        Weights::Hub { repo, file } => (repo, file),
    };
    let cache = cache()?;
    let kept = (hub::kept(&cache).into_iter())
        .filter(|(r, _)| r == repo)
        .map(|(_, f)| f)
        .collect::<Vec<_>>();
    let wanted = match file {
        Some(file) => Some(file.as_str()),
        None => gguf(kept.iter().map(String::as_str)),
    };
    if let Some(path) = wanted.and_then(|file| hub::cached(&cache, repo, file)) {
        return Ok(path);
    }
    set(shared, Weighed::Asking(repo.clone()));
    let listing = hub::list(repo, stop)?;
    let file = match file {
        Some(file) => file.as_str(),
        None => gguf(listing.files.iter().map(|f| f.path.as_str()))
            .ok_or_else(|| format!("{repo} has no GGUF file"))?,
    };
    let remote = remote(&listing, repo, file)?;
    fetch(
        shared,
        &cache,
        repo,
        &listing.commit,
        remote,
        &weights.name(),
        stop,
    )
}

/// Hugging Face's cache, where models are kept.
#[cfg(feature = "summary")]
fn cache() -> Result<PathBuf, String> {
    hub::cache().ok_or_else(|| "no home directory for Hugging Face's cache".into())
}

/// `file` of `repo`, as `listing` has it.
#[cfg(feature = "summary")]
fn remote<'a>(
    listing: &'a hub::Listing,
    repo: &str,
    file: &str,
) -> Result<&'a hub::Remote, String> {
    (listing.files.iter())
        .find(|f| f.path == file)
        .ok_or_else(|| format!("{repo} has no {file}"))
}

/// The directory of a snapshot of `repo` with all of `files`, downloading
/// them into Hugging Face's cache first if they are not kept there.
#[cfg(feature = "summary")]
fn fetch_all(
    shared: &Shared,
    repo: &str,
    files: &[&str],
    stop: &AtomicBool,
) -> Result<PathBuf, String> {
    let cache = cache()?;
    let dir = (hub::cached(&cache, repo, files[0]))
        .and_then(|path| Some(path.parent()?.to_path_buf()))
        .filter(|dir| files.iter().all(|f| dir.join(f).is_file()));
    if let Some(dir) = dir {
        return Ok(dir);
    }
    let listing = hub::list(repo, stop)?;
    let name = repo.rsplit('/').next().unwrap_or(repo);
    let mut dir = None;
    for file in files {
        let remote = remote(&listing, repo, file)?;
        let path = fetch(shared, &cache, repo, &listing.commit, remote, name, stop)?;
        dir = path.parent().map(Path::to_path_buf);
    }
    dir.ok_or_else(|| format!("nothing of {repo} to fetch"))
}

/// Download `remote` of `repo` at `commit` into `cache`, shown as `what`.
#[cfg(feature = "summary")]
fn fetch(
    shared: &Shared,
    cache: &Path,
    repo: &str,
    commit: &str,
    remote: &hub::Remote,
    what: &str,
    stop: &AtomicBool,
) -> Result<PathBuf, String> {
    {
        let mut state = lock(shared);
        state.fetching = Some(Fetching {
            what: what.into(),
            part: hub::incomplete(&hub::blob(cache, repo, remote)),
            size: remote.size,
        });
        state.changed();
    }
    let fetched = hub::download(cache, repo, commit, remote, stop);
    let mut state = lock(shared);
    state.fetching = None;
    state.changed();
    fetched
}

#[cfg(not(feature = "summary"))]
struct Llm;

#[cfg(not(feature = "summary"))]
impl Llm {
    fn answer(
        &mut self,
        _: &str,
        _: &str,
        _: usize,
        _: impl FnMut(Step) -> bool,
    ) -> Result<String, String> {
        Err(NO_SUMMARY.into())
    }
}

#[cfg(not(feature = "summary"))]
struct Embedder;

#[cfg(not(feature = "summary"))]
impl Embedder {
    fn embed(
        &self,
        _: &[&str],
        _: impl FnMut(usize) -> bool,
    ) -> Result<Option<Vec<Vec<f32>>>, String> {
        Err(NO_SUMMARY.into())
    }
}

#[cfg(not(feature = "summary"))]
impl Loader {
    fn llm(&mut self, _: &Shared, _: &AtomicBool) -> Result<&mut Llm, String> {
        Err(NO_SUMMARY.into())
    }

    fn embedder(&mut self, _: &Shared, _: &AtomicBool) -> Option<&Embedder> {
        None
    }
}

/// Counts the ways summaries have been asked for and read: those kept from
/// another are not used.
const KEPT: u32 = 1;

/// Where summaries are kept from one run of ohp to the next: `ohp/summaries`
/// in the user's cache directory.
fn kept_dir() -> Option<PathBuf> {
    let var = |name| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let cache = var("XDG_CACHE_HOME").or_else(|| var("HOME").map(|home| home.join(".cache")))?;
    Some(cache.join("ohp/summaries"))
}

/// The name the summary of `pages`, each its sentences, `what` it is of, by
/// the model in `weights`, is kept by: a hash of them all, FNV-1a's, the
/// same from one build of ohp to the next.
fn kept_name<'a>(
    weights: &Weights,
    what: &str,
    pages: impl IntoIterator<Item = &'a [String]>,
) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for &b in bytes {
            hash = (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
    };
    eat(format!("{KEPT}\0{weights:?}\0{what}\0").as_bytes());
    for page in pages {
        for sentence in page {
            eat(sentence.as_bytes());
            eat(b"\0");
        }
        eat(b"\x01");
    }
    format!("{hash:016x}.json")
}

/// The summary kept as `name` in `dir`, if there is one.
#[cfg(feature = "summary")]
fn recall(dir: &Path, name: &str) -> Option<Summary> {
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join(name)).ok()?).ok()?;
    let index = |v: &serde_json::Value| usize::try_from(v.as_u64()?).ok();
    let key = (json["key"].as_array()?.iter())
        .map(|k| {
            Some(Key {
                page: index(&k[0])?,
                sentence: index(&k[1])?,
                text: k[2].as_str()?.into(),
            })
        })
        .collect::<Option<_>>()?;
    Some(Summary {
        text: json["text"].as_str()?.into(),
        key,
    })
}

/// Keep `summary` as `name` in `dir`, whole or not at all.
#[cfg(feature = "summary")]
fn keep(dir: &Path, name: &str, summary: &Summary) {
    let key: Vec<_> = (summary.key.iter())
        .map(|k| serde_json::json!([k.page, k.sentence, k.text]))
        .collect();
    let json = serde_json::json!({ "text": summary.text, "key": key });
    let part = dir.join(format!("{name}.part"));
    let kept = std::fs::create_dir_all(dir)
        .and_then(|()| std::fs::write(&part, json.to_string()))
        .and_then(|()| std::fs::rename(&part, dir.join(name)));
    if kept.is_err() {
        let _ = std::fs::remove_file(&part);
    }
}

#[cfg(not(feature = "summary"))]
fn recall(_: &Path, _: &str) -> Option<Summary> {
    None
}

#[cfg(not(feature = "summary"))]
fn keep(_: &Path, _: &str, _: &Summary) {}

/// What the model is asked of `sentences`, each the page it is on and what
/// it says: as many as fit, each on a line of its own, numbered from 1, and
/// for the `whole` document, with its page.
fn prompt(sentences: &[(usize, &str)], whole: bool) -> String {
    let mut prompt = String::from(if whole {
        "Sentences picked from a document as the most central to it, in order, \
         a sentence a line, numbered, each with the page it is on:\n\n"
    } else {
        "The text of a page, a sentence a line, numbered:\n\n"
    });
    for (k, (page, sentence)) in sentences.iter().enumerate() {
        let line = if whole {
            format!("[{}] (page {}) {sentence}\n", k + 1, page + 1)
        } else {
            format!("[{}] {sentence}\n", k + 1)
        };
        if prompt.len() + line.len() > TEXT {
            break;
        }
        prompt.push_str(&line);
    }
    prompt.push_str(if whole {
        "\nSum up the whole document in one paragraph of at most five sentences. \
         Then give the numbers of the three to five sentences that carry its key \
         takeaway messages.\n"
    } else {
        "\nSum up the page in at most three sentences. Then give the numbers of \
         the one to three sentences that carry its key takeaway messages.\n"
    });
    prompt.push_str(
        "Answer in exactly this form:\n\
         SUMMARY: <the summary>\n\
         KEY: <the numbers, separated by commas>",
    );
    prompt
}

/// Where `label`, as `KEY:`, is in `answer`, in any case, and where what
/// follows it starts.
fn find(answer: &str, label: &str) -> Option<(usize, usize)> {
    let at = answer.to_ascii_uppercase().rfind(label)?;
    Some((at, at + label.len()))
}

/// The summary in an `answer`, so far as it has come: what follows
/// `SUMMARY:` and comes before `KEY:`.
fn shown(answer: &str) -> String {
    let before = find(answer, "KEY:").map_or(answer, |(at, _)| &answer[..at]);
    let summary = find(before, "SUMMARY:").map_or(before, |(_, from)| &before[from..]);
    summary.trim().into()
}

/// The summary in the model's `answer` about `sentences`, each the page it
/// is on and what it says, and the key sentences it numbers, at most
/// `most`, in their order; `sentence` gives which of its page's each is.
fn summary(
    answer: &str,
    sentences: &[(usize, &str)],
    sentence: &dyn Fn(usize) -> usize,
    most: usize,
) -> Summary {
    let numbers = find(answer, "KEY:").map_or("", |(_, from)| &answer[from..]);
    let mut key: Vec<usize> = Vec::new();
    for n in numbers.split(|c: char| !c.is_ascii_digit()) {
        let Ok(n) = n.parse::<usize>() else { continue };
        let whole = |i: usize| sentences[i].1.split_whitespace().count() >= KEY_WORDS;
        if (1..=sentences.len()).contains(&n)
            && whole(n - 1)
            && !key.contains(&(n - 1))
            && key.len() < most
        {
            key.push(n - 1);
        }
    }
    key.sort_unstable();
    let key = key.into_iter().map(|i| Key {
        page: sentences[i].0,
        sentence: sentence(i),
        text: sentences[i].1.into(),
    });
    Summary {
        text: shown(answer),
        key: key.collect(),
    }
}

#[cfg(test)]
#[path = "tests/summary.rs"]
mod tests;
