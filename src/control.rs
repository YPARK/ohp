//! How another program reaches the ohp running here: each ohp showing
//! slides listens on a socket of its own, in a directory only its user can
//! enter, and `ohp mcp` speaks to the one started last.
//!
//! A request is a line, and so is each line of the answer, ended by `end`:
//!
//! - `status`: `file <path>`, one a file of the deck, `page <n>` from 1,
//!   `pages <n>`, `view <present|grid>`, `presenting <0|1>`;
//! - `goto <n>`: turn to slide `n`, from 1, and show it;
//! - `open <n> <path>`: show the file, from slide `n`.
//!
//! An answer to `goto` or `open` is `ok <what was done>` or `err <why>`.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

/// What is asked of the running ohp.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    Status,
    /// A slide, from 0.
    Goto(usize),
    /// A file, and the slide, from 0, to show first.
    Open(PathBuf, usize),
}

impl Request {
    pub fn line(&self) -> String {
        match self {
            Request::Status => "status".into(),
            Request::Goto(n) => format!("goto {}", n + 1),
            Request::Open(path, n) => format!("open {} {}", n + 1, path.display()),
        }
    }

    pub fn parse(line: &str) -> Option<Request> {
        let line = line.trim_end_matches(['\r', '\n']);
        let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
        let from_one = |n: &str| n.parse::<usize>().ok()?.checked_sub(1);
        Some(match word {
            "status" => Request::Status,
            "goto" => Request::Goto(from_one(rest)?),
            "open" => {
                let (n, path) = rest.split_once(' ')?;
                Request::Open(path.into(), from_one(n)?)
            }
            _ => return None,
        })
    }
}

/// A request, and where its answer goes, a line at a time.
pub type Asked = (Request, Sender<Vec<String>>);

/// The directory every ohp of this user listens in.
pub fn dir() -> PathBuf {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_else(|_| "me".into());
    std::env::temp_dir().join(format!("ohp-{user}"))
}

/// The running ohp's end: requests, as they come.
pub struct Listener {
    path: PathBuf,
    pub asked: Receiver<Asked>,
}

impl Listener {
    /// Listen on a socket of this ohp's own.
    pub fn new() -> std::io::Result<Self> {
        Self::in_dir(&dir())
    }

    pub fn in_dir(dir: &Path) -> std::io::Result<Self> {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        // Only its user may reach an ohp, which opens what it is told to.
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        let path = dir.join(format!("{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        let (tx, asked) = mpsc::channel();
        std::thread::Builder::new()
            .name("ohp-control".into())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    let tx = tx.clone();
                    std::thread::spawn(move || serve(stream, &tx));
                }
            })?;
        Ok(Listener { path, asked })
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Answer each request `stream` makes, as the running ohp answers it.
fn serve(stream: UnixStream, tx: &Sender<Asked>) {
    let Ok(mut out) = stream.try_clone() else {
        return;
    };
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else {
            return;
        };
        let answer = match Request::parse(&line) {
            Some(request) => {
                let (reply, answer) = mpsc::channel();
                if tx.send((request, reply)).is_err() {
                    return;
                }
                answer
                    .recv_timeout(Duration::from_secs(10))
                    .unwrap_or_else(|_| vec!["err ohp did not answer".into()])
            }
            None => vec![format!("err cannot do {line:?}")],
        };
        for line in answer.iter().chain(std::iter::once(&"end".to_string())) {
            if writeln!(out, "{line}").is_err() {
                return;
            }
        }
    }
}

/// Ask the ohp started last, of those running here, and hear its answer.
/// None if no ohp is running.
pub fn ask(request: &Request) -> Option<Vec<String>> {
    ask_in(&dir(), request)
}

pub fn ask_in(dir: &Path, request: &Request) -> Option<Vec<String>> {
    let mut sockets: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "sock"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    sockets.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
    for (_, path) in sockets {
        let Ok(mut stream) = UnixStream::connect(&path) else {
            // Left by an ohp that did not end as it should.
            let _ = std::fs::remove_file(&path);
            continue;
        };
        let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
        if writeln!(stream, "{}", request.line()).is_err() {
            continue;
        }
        let answer: Vec<String> = BufReader::new(stream)
            .lines()
            .map_while(Result::ok)
            .take_while(|l| l != "end")
            .collect();
        return Some(answer);
    }
    None
}

#[cfg(test)]
#[path = "tests/control.rs"]
mod tests;
