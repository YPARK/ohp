//! Slides on another machine, named as scp names them: `[user@]host:path`.
//!
//! The PDF is copied over ssh into a local file and kept in step with the
//! original, so it reloads as a local one does. One ssh connection watches
//! the file and every copy goes through it, so a password is asked for once,
//! before the slides take over the terminal.

use anyhow::{Context, bail};
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{BufRead, BufReader, Lines};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use tempfile::TempDir;

/// Prints the file's modification time and length each second, or `gone`.
/// GNU stat takes `-c`, BSD stat `-f`.
const WATCH: &str = r#"while :; do stat -c %Y.%s "$f" 2>/dev/null || stat -f %m.%z "$f" 2>/dev/null || echo gone; sleep 1; done"#;

const GONE: &str = "gone";

/// How long to wait between attempts to reconnect a lost connection.
const RETRY: Duration = Duration::from_secs(3);

/// Options for every ssh: no terminal, which would end each line of output
/// with `\r`, and a dead link noticed within about 15 seconds rather than
/// never.
const SSH: [&str; 7] = [
    "-T",
    "-o",
    "ServerAliveInterval=5",
    "-o",
    "ServerAliveCountMax=3",
    "-o",
    "ConnectTimeout=10",
];

type Stamps = Lines<BufReader<ChildStdout>>;

pub struct Remote {
    path: PathBuf,
    master: Arc<Mutex<Master>>,
    /// Dropped to stop the watcher waiting to reconnect.
    stop: Option<Sender<()>>,
    watcher: Option<JoinHandle<()>>,
    link: Link,
    _dir: TempDir,
}

/// The ssh connection that watches the file and carries every copy.
struct Master {
    child: Option<Child>,
    /// Set as ohp closes, after which no new connection is made.
    stopped: bool,
}

/// What is wrong with the connection to the other machine, if anything.
#[derive(Clone, Default)]
pub struct Link(Arc<Mutex<Option<String>>>);

impl Link {
    pub fn trouble(&self) -> Option<String> {
        self.0.lock().unwrap().clone()
    }

    fn set(&self, trouble: Option<String>) {
        *self.0.lock().unwrap() = trouble;
    }
}

/// The host and path `arg` names, if it names a remote file: something
/// other than a directory before a colon, and no local file of that name.
pub fn split(arg: &Path) -> Option<(&str, &str)> {
    if arg.exists() {
        return None;
    }
    let (host, path) = arg.to_str()?.split_once(':')?;
    (!host.is_empty() && !host.contains('/') && !path.is_empty()).then_some((host, path))
}

impl Remote {
    /// Copy `host:path` here and keep the copy up to date, through `ssh`.
    pub fn open(ssh: &OsStr, host: &str, path: &str) -> anyhow::Result<Self> {
        // Not under $TMPDIR, which can be long: ssh binds the control socket
        // at its path plus a suffix, and a socket's path may be no longer
        // than 104 bytes on macOS.
        let dir = tempfile::Builder::new().prefix("ohp").tempdir_in("/tmp")?;
        // The copy keeps the remote file's name, so it is named on the status
        // line, in a directory of its own, so no name can clash with the
        // files ohp keeps beside it.
        let copies = dir.path().join("copy");
        std::fs::create_dir(&copies)?;
        let name = Path::new(path).file_name().unwrap_or("slides.pdf".as_ref());
        let fetch = Fetch {
            ssh: ssh.to_owned(),
            socket: dir.path().join("ssh"),
            log: dir.path().join("ssh.log"),
            host: host.into(),
            path: quote_path(path),
            part: dir.path().join("part"),
            local: copies.join(name),
        };
        let mut child = fetch
            .master(false)
            .with_context(|| format!("running {}", ssh.to_string_lossy()))?;
        let mut stamps = BufReader::new(child.stdout.take().expect("piped")).lines();
        let (stop, stopped) = mpsc::channel();
        let mut remote = Remote {
            path: fetch.local.clone(),
            master: Arc::new(Mutex::new(Master {
                child: Some(child),
                stopped: false,
            })),
            stop: Some(stop),
            watcher: None,
            link: Link::default(),
            _dir: dir,
        };
        let last = match stamps.next() {
            Some(Ok(stamp)) if stamp == GONE => bail!("{host}:{path} not found"),
            Some(Ok(stamp)) => stamp,
            _ => {
                let why = std::fs::read_to_string(&fetch.log).unwrap_or_default();
                bail!("ssh {host} failed: {}", why.trim());
            }
        };
        fetch
            .run()
            .with_context(|| format!("copying {host}:{path}"))?;
        let watcher = Watcher {
            fetch,
            master: remote.master.clone(),
            stopped,
            link: remote.link.clone(),
        };
        remote.watcher = Some(std::thread::spawn(move || watcher.run(stamps, last)));
        Ok(remote)
    }

    /// The local copy.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The state of the connection, for the status line.
    pub fn link(&self) -> Link {
        self.link.clone()
    }
}

impl Drop for Remote {
    /// Close the connection and wait for the watcher, so nothing is being
    /// written to the copy's directory as it is removed.
    fn drop(&mut self) {
        {
            let mut master = self.master.lock().unwrap();
            master.stopped = true;
            if let Some(mut child) = master.child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        self.stop.take();
        if let Some(watcher) = self.watcher.take() {
            let _ = watcher.join();
        }
    }
}

/// Copies the file whenever it changes, and reconnects when the connection
/// is lost.
struct Watcher {
    fetch: Fetch,
    master: Arc<Mutex<Master>>,
    stopped: mpsc::Receiver<()>,
    link: Link,
}

impl Watcher {
    fn run(self, mut stamps: Stamps, mut last: String) {
        loop {
            for now in stamps.by_ref().map_while(Result::ok) {
                self.link.set(None);
                if now != last && now != GONE && self.fetch.run().is_ok() {
                    last = now;
                }
            }
            self.link.set(Some(format!(
                "lost connection to {}, reconnecting…",
                self.fetch.host
            )));
            stamps = loop {
                match self.stopped.recv_timeout(RETRY) {
                    Err(RecvTimeoutError::Timeout) => {}
                    _ => return,
                }
                match self.reconnect() {
                    Ok(Some(stamps)) => break stamps,
                    Ok(None) => return,
                    Err(_) => {}
                }
            };
        }
    }

    /// A new master connection, or `None` once ohp is closing.
    fn reconnect(&self) -> std::io::Result<Option<Stamps>> {
        let mut master = self.master.lock().unwrap();
        if master.stopped {
            return Ok(None);
        }
        if let Some(mut old) = master.child.take() {
            let _ = old.kill();
            let _ = old.wait();
        }
        // A socket left by a master that did not exit cleanly would stop
        // the new one listening.
        let _ = std::fs::remove_file(&self.fetch.socket);
        let mut child = self.fetch.master(true)?;
        let stdout = child.stdout.take().expect("piped");
        master.child = Some(child);
        Ok(Some(BufReader::new(stdout).lines()))
    }
}

/// Copies the remote file over the master connection.
struct Fetch {
    ssh: OsString,
    socket: PathBuf,
    /// What the master says on stderr.
    log: PathBuf,
    host: String,
    /// Quoted for the remote shell.
    path: String,
    /// Where a copy is written before it is moved into place.
    part: PathBuf,
    local: PathBuf,
}

impl Fetch {
    fn control(&self) -> OsString {
        let mut option = OsString::from("ControlPath=");
        option.push(&self.socket);
        option
    }

    /// Start the master connection, printing the file's stamp each second.
    /// `batch` never asks for a password: once the slides are up it would
    /// be asked over them.
    fn master(&self, batch: bool) -> std::io::Result<Child> {
        let mut ssh = Command::new(&self.ssh);
        ssh.args(SSH)
            .args(["-o", "ControlMaster=yes", "-o", "ControlPersist=no", "-o"])
            .arg(self.control());
        if batch {
            ssh.args(["-o", "BatchMode=yes"]);
        }
        ssh.arg(&self.host)
            .arg(remote(&format!("f={}; {WATCH}", self.path)))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(File::create(&self.log)?)
            .spawn()
    }

    /// Copy to a file beside the local copy, then move it into place, so
    /// the copy is never seen half written.
    fn run(&self) -> anyhow::Result<()> {
        // Never a connection of its own: one would ask for a password over
        // the slides.
        let out = Command::new(&self.ssh)
            .args(SSH)
            .args(["-o", "ControlMaster=no", "-o", "BatchMode=yes", "-o"])
            .arg(self.control())
            .arg(&self.host)
            .arg(remote(&format!("exec cat {}", self.path)))
            .stdin(Stdio::null())
            .stdout(File::create(&self.part)?)
            .output()?;
        if !out.status.success() {
            bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
        }
        std::fs::rename(&self.part, &self.local)?;
        Ok(())
    }
}

/// `script` run by sh, whatever the login shell on the other end is.
fn remote(script: &str) -> String {
    format!("exec sh -c {}", quote(script))
}

pub fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// `path` quoted for sh, leaving a leading `~/` for it to expand.
fn quote_path(path: &str) -> String {
    match path.strip_prefix("~/") {
        Some(rest) => format!("~/{}", quote(rest)),
        None => quote(path),
    }
}

#[cfg(test)]
#[path = "tests/remote.rs"]
mod tests;
