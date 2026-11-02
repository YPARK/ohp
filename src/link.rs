//! Two windows presenting one deck: the presenter's, with the slide, the
//! next, the notes, the slides to pick from and the time, and the
//! audience's, with the slide alone, on the projector.
//!
//! The presenter's ohp writes the deck as a PDF into a directory of its
//! own, beside a socket, and opens a terminal window that runs
//! `ohp --audience <socket>`. That ohp shows the PDF, reloads it as it is
//! written again, and turns to the slide it is told to. The two say lines
//! to each other over the socket:
//!
//! - presenter to audience: `deck <path>` once, then `page <n>`,
//!   `black <0|1>`, `laser <x> <y>` or `laser off`, and `bye` as it stops;
//! - audience to presenter: `key next`, `key prev`, `key black`, and `bye`
//!   as its window closes, so keys pressed in either window turn the
//!   slides.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// What one window says to the other.
#[derive(Clone, Debug, PartialEq)]
pub enum Said {
    /// The PDF the audience shows.
    Deck(PathBuf),
    Page(usize),
    /// Whether the audience's screen is blacked out.
    Black(bool),
    /// A key pressed in the audience's window, for the presenter to act on.
    Key(Turn),
    /// Where the laser pointer is on the slide, as fractions of its width
    /// and height from its top left, or that it is off.
    Laser(Option<(f32, f32)>),
    /// The other window is gone, or going.
    Bye,
}

/// What a key pressed in the audience's window does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Turn {
    Next,
    Prev,
    Black,
}

impl Said {
    fn line(&self) -> String {
        match self {
            Said::Deck(path) => format!("deck {}", path.display()),
            Said::Page(n) => format!("page {n}"),
            Said::Black(on) => format!("black {}", u8::from(*on)),
            Said::Key(Turn::Next) => "key next".into(),
            Said::Key(Turn::Prev) => "key prev".into(),
            Said::Key(Turn::Black) => "key black".into(),
            Said::Laser(Some((x, y))) => format!("laser {x:.4} {y:.4}"),
            Said::Laser(None) => "laser off".into(),
            Said::Bye => "bye".into(),
        }
    }

    fn parse(line: &str) -> Option<Said> {
        let (word, rest) = line
            .trim_end_matches(['\r', '\n'])
            .split_once(' ')
            .unwrap_or((line.trim(), ""));
        Some(match (word, rest) {
            ("deck", path) if !path.is_empty() => Said::Deck(path.into()),
            ("page", n) => Said::Page(n.parse().ok()?),
            ("black", on) => Said::Black(on == "1"),
            ("key", "next") => Said::Key(Turn::Next),
            ("key", "prev") => Said::Key(Turn::Prev),
            ("key", "black") => Said::Key(Turn::Black),
            ("laser", "off") => Said::Laser(None),
            ("laser", at) => {
                let (x, y) = at.split_once(' ')?;
                Said::Laser(Some((x.parse().ok()?, y.parse().ok()?)))
            }
            ("bye", _) => Said::Bye,
            _ => return None,
        })
    }
}

/// One end of the socket: what the other says, as it says it, and a way to
/// speak to it.
pub struct Peer {
    out: UnixStream,
    heard: Receiver<Said>,
}

impl Peer {
    fn new(stream: UnixStream) -> std::io::Result<Self> {
        let out = stream.try_clone()?;
        let (tx, heard) = mpsc::channel();
        std::thread::Builder::new()
            .name("ohp-link".into())
            .spawn(move || listen(stream, &tx))?;
        Ok(Peer { out, heard })
    }

    /// Say `said`; false if the other end is gone.
    pub fn say(&mut self, said: &Said) -> bool {
        writeln!(self.out, "{}", said.line()).is_ok() && self.out.flush().is_ok()
    }

    /// What the other end has said since last asked; `Bye` once it is gone.
    pub fn heard(&self) -> Vec<Said> {
        let mut all = Vec::new();
        loop {
            match self.heard.try_recv() {
                Ok(said) => all.push(said),
                Err(TryRecvError::Empty) => return all,
                Err(TryRecvError::Disconnected) => {
                    all.push(Said::Bye);
                    return all;
                }
            }
        }
    }
}

/// Pass on each line `stream` says, until it ends.
fn listen(stream: UnixStream, tx: &Sender<Said>) {
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else {
            return;
        };
        if let Some(said) = Said::parse(&line)
            && tx.send(said).is_err()
        {
            return;
        }
    }
}

/// The presenter's end: the directory the deck is written into, the
/// socket, and the audience's window once it has connected.
pub struct Stage {
    dir: TempDir,
    peers: Receiver<UnixStream>,
    pub peer: Option<Peer>,
}

impl Stage {
    pub fn new() -> std::io::Result<Self> {
        let dir = tempfile::Builder::new().prefix("ohp-").tempdir()?;
        let listener = UnixListener::bind(dir.path().join("ohp.sock"))?;
        let (tx, peers) = mpsc::channel();
        std::thread::Builder::new()
            .name("ohp-stage".into())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    // Gone, the stage knocks once more to end this.
                    if tx.send(stream).is_err() {
                        return;
                    }
                }
            })?;
        Ok(Stage {
            dir,
            peers,
            peer: None,
        })
    }

    pub fn socket(&self) -> PathBuf {
        self.dir.path().join("ohp.sock")
    }

    /// Where the audience's PDF is written.
    pub fn pdf(&self) -> PathBuf {
        self.dir.path().join("deck.pdf")
    }

    /// Write `data` as the audience's PDF, all at once, so its window
    /// never reads half of it.
    pub fn write(&self, data: &[u8]) -> std::io::Result<()> {
        let part = self.dir.path().join("deck.pdf.part");
        std::fs::write(&part, data)?;
        std::fs::rename(part, self.pdf())
    }

    /// An audience window that has connected since last asked, replacing
    /// any before it.
    pub fn arrived(&mut self) -> bool {
        let mut came = false;
        while let Ok(stream) = self.peers.try_recv() {
            if let Ok(peer) = Peer::new(stream) {
                if let Some(old) = self.peer.as_mut() {
                    old.say(&Said::Bye);
                }
                self.peer = Some(peer);
                came = true;
            }
        }
        came
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        if let Some(peer) = self.peer.as_mut() {
            peer.say(&Said::Bye);
        }
        // The thread waiting for a window to connect, woken to see no one
        // waits for it any longer.
        let (_, gone) = mpsc::channel();
        drop(std::mem::replace(&mut self.peers, gone));
        let _ = UnixStream::connect(self.socket());
    }
}

/// The audience's end, connected, and the PDF it is to show.
pub struct Audience {
    pub peer: Peer,
    pub pdf: PathBuf,
    /// The slide to show first.
    pub page: usize,
    pub black: bool,
}

/// Connect to the presenter at `socket` and hear which PDF to show.
pub fn join(socket: &Path) -> anyhow::Result<Audience> {
    let stream = UnixStream::connect(socket)
        .map_err(|e| anyhow::anyhow!("cannot reach the presenter at {}: {e}", socket.display()))?;
    let peer = Peer::new(stream)?;
    let (mut pdf, mut page, mut black) = (None, 0, false);
    let until = Instant::now() + Duration::from_secs(10);
    while pdf.is_none() {
        if Instant::now() > until {
            anyhow::bail!("the presenter did not say what to show");
        }
        match peer.heard.recv_timeout(Duration::from_millis(100)) {
            Ok(Said::Deck(path)) => pdf = Some(path),
            Ok(Said::Page(n)) => page = n,
            Ok(Said::Black(on)) => black = on,
            Ok(Said::Bye) => anyhow::bail!("the presenter stopped"),
            Ok(Said::Key(_) | Said::Laser(_)) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => anyhow::bail!("the presenter stopped"),
        }
    }
    // What came after the deck, as its page, is heard by the window.
    Ok(Audience {
        peer,
        pdf: pdf.unwrap_or_default(),
        page,
        black,
    })
}

/// Open a terminal window running `ohp --audience <socket>`. Returns the
/// terminal's name, or, where no window can be opened from here, the
/// command to run in one.
pub fn open_window(socket: &Path) -> Result<&'static str, String> {
    open_terminal(
        &["--audience".into(), socket.as_os_str().to_owned()],
        "ohp audience",
    )
}

/// Open a terminal window running this ohp with `args`, titled `title`.
/// Returns the terminal's name, or, where no window can be opened from
/// here, the command to run in one.
pub fn open_terminal(args: &[OsString], title: &str) -> Result<&'static str, String> {
    let exe = std::env::current_exe().unwrap_or_else(|_| "ohp".into());
    let args: Vec<OsString> = std::iter::once(exe.into_os_string())
        .chain(args.iter().cloned())
        .collect();
    let shell = args
        .iter()
        .map(|a| crate::remote::quote(&a.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(" ");
    let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
    // Over ssh, or in tmux, a window opened here is not where the
    // presenter sees it.
    if var("SSH_CONNECTION").is_some() || var("SSH_TTY").is_some() || var("TMUX").is_some() {
        return Err(shell);
    }
    let (name, mut command) = if var("KITTY_WINDOW_ID").is_some() || var("KITTY_PID").is_some() {
        ("kitty", kitty(&args, title))
    } else if var("GHOSTTY_RESOURCES_DIR").is_some() {
        ("Ghostty", ghostty(&args))
    } else if let Some(wezterm) = var("WEZTERM_EXECUTABLE") {
        let mut c = Command::new(wezterm);
        c.args(["cli", "spawn", "--new-window", "--"]).args(&args);
        ("WezTerm", c)
    } else if var("ITERM_SESSION_ID").is_some() {
        let script = format!(
            "tell application \"iTerm2\" to create window with default profile command \"{}\"",
            applescript(&shell)
        );
        ("iTerm2", osascript(&script))
    } else if std::env::var("TERM_PROGRAM").is_ok_and(|t| t == "Apple_Terminal") {
        let script = format!(
            "tell application \"Terminal\"\nactivate\ndo script \"exec {}\"\nend tell",
            applescript(&shell)
        );
        ("Terminal", osascript(&script))
    } else {
        return Err(shell);
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match command.spawn() {
        Ok(mut child) => {
            // Reaped, as it ends, by a thread of its own.
            std::thread::spawn(move || child.wait());
            Ok(name)
        }
        Err(_) => Err(shell),
    }
}

/// kitty: a window of this kitty's, where it lets itself be controlled,
/// else a kitty of its own.
fn kitty(args: &[OsString], title: &str) -> Command {
    if let Some(to) = std::env::var_os("KITTY_LISTEN_ON").filter(|v| !v.is_empty()) {
        let mut c = Command::new("kitty");
        c.arg("@")
            .arg("--to")
            .arg(to)
            .args(["launch", "--type=os-window", "--title", title])
            .args(args);
        return c;
    }
    let bundled = std::env::var_os("KITTY_INSTALLATION_DIR")
        .map(|dir| Path::new(&dir).join("../../MacOS/kitty"))
        .filter(|p| p.is_file());
    let mut c = Command::new(bundled.map_or_else(|| "kitty".into(), PathBuf::into_os_string));
    c.args(["--title", title]).args(args);
    c
}

fn ghostty(args: &[OsString]) -> Command {
    if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.args(["-na", "Ghostty", "--args", "-e"]).args(args);
        c
    } else {
        let mut c = Command::new("ghostty");
        c.arg("-e").args(args);
        c
    }
}

fn osascript(script: &str) -> Command {
    let mut c = Command::new("osascript");
    c.arg("-e").arg(script);
    c
}

/// `s` in an AppleScript string.
fn applescript(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
#[path = "tests/link.rs"]
mod tests;
