//! Slides read aloud, a sentence at a time, the sentence being read lit.
//!
//! Two shell commands read: a voice, which writes the text on its input as
//! WAV audio on its output, as `piper -m voice.onnx -f -` or `espeak-ng
//! --stdout`, and a player, which plays the WAV on its input, as `aplay -q`.
//! Without them given, Piper is the voice if a voice for it is found where
//! its voices go, and otherwise the first of each found is used. Each sentence is
//! voiced ahead, while the one before it plays, so a voice slow to start
//! leaves no gap, and the sentence playing is known to the moment.
//!
//! A slide is read on a thread of its own, its text taken from the PDF there
//! too. The commands run as R does, in sessions of their own, so stopping
//! one stops all of its pipeline, and ohp quitting stops them. Where a
//! formula or a table is left out, the reading pauses.

#[cfg(feature = "speech")]
use crate::process::child::{self, Ended};
use crate::text::{self, Spoken};
use crate::{remote, render};
use hayro::hayro_interpret::{InterpreterCache, InterpreterSettings};
use hayro::hayro_syntax::Pdf;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
#[cfg(feature = "speech")]
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel, sync_channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Name of the threads reading, so their panics can be told apart.
pub const THREAD: &str = "speak";

/// How long a formula or a table left out is paused for, unless told.
const PAUSE: Duration = Duration::from_millis(500);
/// How often a wait looks whether to stop.
const STEP: Duration = Duration::from_millis(20);
/// No sentence is being read.
const NONE: usize = usize::MAX;

/// Voices that write what is on their input as WAV on their output: the
/// program looked for, and the command run.
const VOICES: &[(&str, &str)] = &[
    #[cfg(target_os = "macos")]
    ("say", SAY),
    ("espeak-ng", "espeak-ng --stdout"),
    ("espeak", "espeak --stdout"),
];
/// macOS's voice, the one chosen in its settings. It writes WAV only to a
/// file it can seek in.
#[cfg(target_os = "macos")]
const SAY: &str = "d=$(mktemp -d) || exit; \
    say -f - -o \"$d/s.wav\" --data-format=LEI16@22050 && cat \"$d/s.wav\"; \
    s=$?; rm -rf \"$d\"; exit $s";
/// Players of WAV on their input, which is a file: `afplay` cannot play
/// from a pipe.
const PLAYERS: &[(&str, &str)] = &[
    ("aplay", "aplay -q"),
    ("paplay", "paplay"),
    ("afplay", "afplay /dev/stdin"),
    ("play", "play -q -t wav -"),
];

/// The voice that reads slides: the one given, or else Piper with a voice
/// of its found, or else one found.
pub fn voice(given: Option<String>) -> Option<String> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let data = std::env::var_os("XDG_DATA_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    given_or(given, || {
        data.and_then(|data| piper(&path, &data.join("piper")))
            .or_else(|| found(&path, VOICES))
    })
}

/// The player of what the voice says: the one given, or else one found.
pub fn player(given: Option<String>) -> Option<String> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    given_or(given, || found(&path, PLAYERS))
}

fn given_or(given: Option<String>, or: impl FnOnce() -> Option<String>) -> Option<String> {
    given.filter(|c| !c.trim().is_empty()).or_else(or)
}

/// Whether `name` is a program on `path`.
fn on(path: &OsStr, name: &str) -> bool {
    std::env::split_paths(path).any(|dir| runnable(&dir.join(name)))
}

/// Whether `file` is a program: a file, and on unix, one allowed to run.
fn runnable(file: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(file) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    meta.is_file()
}

/// The command of the first of `known` on `path`.
fn found(path: &OsStr, known: &[(&str, &str)]) -> Option<String> {
    known
        .iter()
        .find(|(name, _)| on(path, name))
        .map(|(_, command)| command.to_string())
}

/// Piper speaking with the first voice in `voices`, a model with its
/// config beside it, if Piper is on `path`.
fn piper(path: &OsStr, voices: &Path) -> Option<String> {
    if !on(path, "piper") {
        return None;
    }
    let mut models: Vec<PathBuf> = std::fs::read_dir(voices)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|model| {
            model.extension().is_some_and(|e| e == "onnx")
                && model.with_extension("onnx.json").is_file()
        })
        .collect();
    models.sort();
    let model = remote::quote(models.first()?.to_str()?);
    Some(format!("piper -m {model} -f -"))
}

pub struct Speaker {
    voice: Option<String>,
    player: Option<String>,
    /// How long a formula or a table left out is paused for.
    pause: Duration,
    reading: Option<Reading>,
    /// The reader of a slide stopped, still ending its commands: the next
    /// waits for it, so two are never heard at once.
    ending: Option<JoinHandle<()>>,
}

/// A slide being read.
struct Reading {
    page: usize,
    /// The sentence being read, or `NONE`.
    now: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    done: Receiver<Result<(), String>>,
    thread: JoinHandle<()>,
}

impl Speaker {
    /// A speaker with `voice` and `player`, which reads nothing if either
    /// is `None`.
    pub fn new(voice: Option<String>, player: Option<String>) -> Self {
        Speaker {
            voice,
            player,
            pause: PAUSE,
            reading: None,
            ending: None,
        }
    }

    /// The speaker, pausing for `pause` where a formula or a table is left
    /// out.
    #[cfg(feature = "speech")]
    pub fn with_pause(mut self, pause: Duration) -> Self {
        self.pause = pause;
        self
    }

    /// The slide being read, if one is.
    pub fn page(&self) -> Option<usize> {
        self.reading.as_ref().map(|r| r.page)
    }

    /// The sentence of the slide being read, as `text::Page::light` counts
    /// them, if one is.
    pub fn sentence(&self) -> Option<usize> {
        let now = self.reading.as_ref()?.now.load(Ordering::Relaxed);
        (now != NONE).then_some(now)
    }

    /// Read `page` of the PDF in `data` aloud, in place of any slide being
    /// read.
    pub fn read(&mut self, data: Arc<Vec<u8>>, page: usize) -> Result<(), String> {
        if !cfg!(feature = "speech") {
            return Err(NO_SPEECH.into());
        }
        self.stop();
        let voice = self.voice.clone().ok_or(
            "nothing to read aloud with: give --voice or OHP_VOICE, or install Piper or espeak-ng",
        )?;
        let player = self
            .player
            .clone()
            .ok_or("nothing to play speech with: give --play or OHP_PLAY")?;
        let reader = Reader {
            voice,
            player,
            pause: self.pause,
            now: Arc::new(AtomicUsize::new(NONE)),
            stop: Arc::new(AtomicBool::new(false)),
        };
        let (now, stop) = (reader.now.clone(), reader.stop.clone());
        let (tx, done) = channel();
        let before = self.ending.take();
        let thread = std::thread::Builder::new()
            .name(THREAD.into())
            .spawn(move || {
                if let Some(before) = before {
                    let _ = before.join();
                }
                // Nothing to do if the app is gone.
                let _ = tx.send(reader.read(&data, page));
            })
            .map_err(|e| format!("cannot read aloud: {e}"))?;
        self.reading = Some(Reading {
            page,
            now,
            stop,
            done,
            thread,
        });
        Ok(())
    }

    /// Stop reading. Its commands end soon after, not waited for here, so
    /// the screen does not wait on them.
    pub fn stop(&mut self) {
        if let Some(reading) = self.reading.take() {
            reading.stop.store(true, Ordering::Relaxed);
            self.ending = Some(reading.thread);
        }
    }

    /// The slide just read to its end, or why it could not be, once it is.
    pub fn ended(&mut self) -> Option<Result<usize, String>> {
        let reading = self.reading.as_ref()?;
        let ended = match reading.done.try_recv() {
            Ok(ended) => ended.map(|()| reading.page),
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err("the reader stopped".into()),
        };
        self.stop();
        Some(ended)
    }
}

/// However ohp ends, the slide being read stops with it.
impl Drop for Speaker {
    fn drop(&mut self) {
        self.stop();
        if let Some(ending) = self.ending.take() {
            let _ = ending.join();
        }
    }
}

/// What reads one slide, on its own thread.
struct Reader {
    voice: String,
    player: String,
    pause: Duration,
    now: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
}

impl Reader {
    /// Read `page` of `data` aloud, a sentence at a time, each voiced while
    /// the one before plays, until it is read or `stop` is set. A slide with
    /// no text is read at once.
    fn read(&self, data: &Arc<Vec<u8>>, page: usize) -> Result<(), String> {
        // Stopped while the reader before it ended, as slides turned fast.
        if self.stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        let said = words(data, page)?;
        let sentences: Vec<&String> = said
            .iter()
            .filter_map(|s| match s {
                Spoken::Text(t) => Some(t),
                Spoken::Pause => None,
            })
            .collect();
        if sentences.is_empty() {
            return Ok(());
        }
        let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
        // One sentence voiced ahead, handed over as the one before it ends,
        // is enough, and no more is wasted when the slide is turned.
        let (tx, voiced) = sync_channel(0);
        let read = std::thread::scope(|scope| {
            let voicing = std::thread::Builder::new().name(THREAD.into());
            let (dir, stop) = (dir.path(), &self.stop);
            voicing
                .spawn_scoped(scope, move || {
                    for (k, text) in sentences.iter().enumerate() {
                        let (input, wav) =
                            (dir.join(format!("{k}.txt")), dir.join(format!("{k}.wav")));
                        let made = std::fs::write(&input, format!("{text}\n"))
                            .map_err(|e| e.to_string())
                            .and_then(|()| run(&self.voice, &input, Some(&wav), stop))
                            .map(|()| wav);
                        let failed = made.is_err();
                        if tx.send(made).is_err() || failed || stop.load(Ordering::Relaxed) {
                            return;
                        }
                    }
                })
                .map_err(|e| e.to_string())?;
            // Taking the voiced, so voicing stops when they are no longer
            // played.
            let read = self.play(&said, voiced);
            // Voicing what is left is of no use now.
            self.stop.store(true, Ordering::Relaxed);
            read
        });
        self.now.store(NONE, Ordering::Relaxed);
        read
    }

    /// Play each sentence of `said` as it is voiced, and pause where it
    /// says to.
    fn play(
        &self,
        said: &[Spoken],
        voiced: Receiver<Result<PathBuf, String>>,
    ) -> Result<(), String> {
        let mut k = 0;
        for part in said {
            if self.stop.load(Ordering::Relaxed) {
                break;
            }
            let Spoken::Text(_) = part else {
                wait(self.pause, &self.stop);
                continue;
            };
            // Voicing ends early only when stopped.
            let Ok(wav) = voiced.recv() else { break };
            // Lit until the next is, so a pause does not flicker it.
            self.now.store(k, Ordering::Relaxed);
            run(&self.player, &wav?, None, &self.stop)?;
            k += 1;
        }
        Ok(())
    }
}

/// Wait for `pause`, unless `stop` is set.
fn wait(pause: Duration, stop: &AtomicBool) {
    let start = Instant::now();
    while start.elapsed() < pause && !stop.load(Ordering::Relaxed) {
        std::thread::sleep(STEP);
    }
}

/// What reading `page` of `data` aloud says.
fn words(data: &Arc<Vec<u8>>, page: usize) -> Result<Vec<Spoken>, String> {
    let pdf = Pdf::new(data.clone()).map_err(|e| format!("cannot read the PDF: {e:?}"))?;
    let cache = InterpreterCache::new();
    let settings = InterpreterSettings::default();
    // Past the last page, there is nothing to say.
    let said =
        || Some(text::extract(&pdf, page, &cache, &settings).map_or_else(Vec::new, |p| p.spoken()));
    render::caught(said).ok_or_else(|| "cannot read this slide's text".into())
}

const NO_SPEECH: &str = "ohp was built without its `speech` feature";

#[cfg(not(feature = "speech"))]
fn run(_: &str, _: &Path, _: Option<&Path>, _: &AtomicBool) -> Result<(), String> {
    Err(NO_SPEECH.into())
}

/// Run `command` on `input` until it is done or `stop` is set, its output
/// to `output` if given. From and to files, not pipes, so it is waited on
/// without feeding or reading it.
#[cfg(feature = "speech")]
fn run(
    command: &str,
    input: &Path,
    output: Option<&Path>,
    stop: &AtomicBool,
) -> Result<(), String> {
    let stdin = std::fs::File::open(input).map_err(|e| e.to_string())?;
    let stdout = match output {
        Some(path) => Stdio::from(std::fs::File::create(path).map_err(|e| e.to_string())?),
        None => Stdio::null(),
    };
    let mut log = input.as_os_str().to_owned();
    log.push(".log");
    let stderr = std::fs::File::create(&log).map_err(|e| e.to_string())?;
    let mut sh = Command::new("sh");
    sh.args(["-c", command])
        .stdin(stdin)
        .stdout(stdout)
        .stderr(stderr);
    let status = match child::run(sh, stop, None) {
        Ok(Ended::Exited(status)) => status,
        Ok(Ended::Stopped) => return Ok(()),
        // Given no time limit, it cannot run past one.
        Ok(Ended::Paused | Ended::TimedOut) => return Err("reading aloud paused".into()),
        Err(e) => return Err(format!("cannot read aloud: {e}")),
    };
    if status.success() {
        return Ok(());
    }
    let why = child::last_line(Path::new(&log), |_| true);
    Err(format!(
        "cannot read aloud: {}",
        why.as_deref().unwrap_or("it failed")
    ))
}

#[cfg(all(test, feature = "speech"))]
#[path = "tests/speak.rs"]
mod tests;
