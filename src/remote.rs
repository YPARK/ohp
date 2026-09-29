//! Slides on another machine, named as scp names them: `[user@]host:path`.
//!
//! The PDF is copied over ssh into a local file and kept in step with the
//! original, so it reloads as a local one does. One ssh connection watches
//! the file and every copy goes through it, so a password is asked for once,
//! before the slides take over the terminal.

use anyhow::{Context, bail};
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use tempfile::TempDir;

/// Prints the file's modification time and length each second, or `gone`.
/// GNU stat takes `-c`, BSD stat `-f`.
const WATCH: &str = r#"while :; do stat -c %Y.%s "$f" 2>/dev/null || stat -f %m.%z "$f" 2>/dev/null || echo gone; sleep 1; done"#;

pub struct Remote {
    path: PathBuf,
    master: Child,
    _dir: TempDir,
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
        let dir = tempfile::Builder::new().prefix("ohp").tempdir()?;
        let name = Path::new(path).file_name().unwrap_or("slides.pdf".as_ref());
        let fetch = Fetch {
            ssh: ssh.to_owned(),
            socket: dir.path().join("ssh"),
            host: host.into(),
            path: quote_path(path),
            local: dir.path().join(name),
        };
        let log = dir.path().join("ssh.log");
        let mut master = Command::new(ssh)
            .args(["-o", "ControlMaster=yes", "-o", "ControlPersist=no", "-o"])
            .arg(fetch.control())
            .arg(host)
            .arg(remote(&format!("f={}; {WATCH}", fetch.path)))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(File::create(&log)?)
            .spawn()
            .with_context(|| format!("running {}", ssh.to_string_lossy()))?;
        let stamps = BufReader::new(master.stdout.take().expect("piped"));
        let remote = Remote {
            path: fetch.local.clone(),
            master,
            _dir: dir,
        };
        let mut stamps = stamps.lines().map_while(Result::ok);
        let mut last = match stamps.next() {
            None => {
                let why = std::fs::read_to_string(&log).unwrap_or_default();
                bail!("ssh {host} failed: {}", why.trim());
            }
            Some(stamp) if stamp == "gone" => bail!("{host}:{path} not found"),
            Some(stamp) => stamp,
        };
        fetch
            .run()
            .with_context(|| format!("copying {host}:{path}"))?;
        std::thread::spawn(move || {
            for now in stamps {
                if now != last && now != "gone" && fetch.run().is_ok() {
                    last = now;
                }
            }
        });
        Ok(remote)
    }

    /// The local copy.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Remote {
    fn drop(&mut self) {
        let _ = self.master.kill();
        let _ = self.master.wait();
    }
}

/// Copies the remote file over the master connection.
struct Fetch {
    ssh: OsString,
    socket: PathBuf,
    host: String,
    /// Quoted for the remote shell.
    path: String,
    local: PathBuf,
}

impl Fetch {
    fn control(&self) -> OsString {
        let mut option = OsString::from("ControlPath=");
        option.push(&self.socket);
        option
    }

    /// Copy to a file beside the local copy, then move it into place, so
    /// the copy is never seen half written.
    fn run(&self) -> anyhow::Result<()> {
        let part = self.local.with_extension("part");
        // Never a connection of its own: one would ask for a password over
        // the slides.
        let out = Command::new(&self.ssh)
            .args(["-o", "ControlMaster=no", "-o", "BatchMode=yes", "-o"])
            .arg(self.control())
            .arg(&self.host)
            .arg(remote(&format!("exec cat {}", self.path)))
            .stdin(Stdio::null())
            .stdout(File::create(&part)?)
            .output()?;
        if !out.status.success() {
            bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
        }
        std::fs::rename(&part, &self.local)?;
        Ok(())
    }
}

/// `script` run by sh, whatever the login shell on the other end is.
fn remote(script: &str) -> String {
    format!("exec sh -c {}", quote(script))
}

fn quote(s: &str) -> String {
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
