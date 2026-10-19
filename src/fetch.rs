//! Files downloaded with curl, as Piper's voices and the language model
//! are, and the programs that unpack them, run until done or stopped.

use crate::process::child;
use std::path::Path;
#[cfg(feature = "speech")]
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;

#[cfg(feature = "speech")]
/// `path` as it is while being made.
pub fn part(path: &Path) -> PathBuf {
    let mut part = path.as_os_str().to_owned();
    part.push(".part");
    PathBuf::from(part)
}

/// Run `command` on no input, its output dropped, until it is done or
/// `stop` is set; what it says when it fails is logged beside `at` until
/// then.
pub fn run(mut command: Command, at: &Path, stop: &AtomicBool) -> Result<(), String> {
    command.stdin(Stdio::null()).stdout(Stdio::null());
    let log = at.with_extension("log");
    let ran = child::run_logged(command, &log, stop);
    let _ = std::fs::remove_file(&log);
    if ran? { Ok(()) } else { Err("stopped".into()) }
}

#[cfg(feature = "speech")]
/// Fetch `url` to `to` with curl, until it is done or `stop` is set. A
/// fetch that cannot connect, or stalls, fails rather than waits for ever.
pub fn fetch(url: &str, to: &Path, stop: &AtomicBool) -> Result<(), String> {
    fetch_with(url, to, &Fetch::default(), stop)
}

/// How a file is fetched, past `fetch`'s way.
#[derive(Default)]
pub struct Fetch<'a> {
    /// A header to send, as `Authorization: Bearer …`. It is handed to
    /// curl in a file, not on its command line, where others could read it.
    pub header: Option<&'a str>,
    /// Take up a fetch cut short where it stopped.
    pub resume: bool,
}

/// Fetch `url` to `to` as `how` says, as `fetch` does.
pub fn fetch_with(url: &str, to: &Path, how: &Fetch, stop: &AtomicBool) -> Result<(), String> {
    let mut curl = Command::new("curl");
    curl.args(["-fsSL", "--connect-timeout", "30"]).args([
        "--speed-limit",
        "1000",
        "--speed-time",
        "30",
    ]);
    if how.resume {
        curl.args(["-C", "-"]);
    }
    let headers = match how.header {
        Some(header) => {
            let mut file = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
            std::io::Write::write_all(&mut file, header.as_bytes()).map_err(|e| e.to_string())?;
            let mut at = std::ffi::OsString::from("@");
            at.push(file.path());
            curl.arg("-H").arg(at);
            Some(file)
        }
        None => None,
    };
    curl.arg("-o").arg(to).arg(url);
    let fetched = run(curl, to, stop);
    drop(headers);
    fetched
}
