//! Code chunks run by knitr, where R is here: R's, and those of knitr's
//! other engines, python through reticulate, bash and the rest.

use crate::process::child::{self, Ended};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use tempfile::TempDir;

/// What came of knitting a file.
pub enum Knitted {
    /// The markdown knitr wrote, in a directory with its plots, which is to
    /// be kept while they are read.
    Done(TempDir, String),
    /// Why the chunks show as their code.
    Failed(String),
    /// Stopped, as ohp quits.
    Stopped,
}

/// Whether a file has anything for knitr to run: a chunk in any of its
/// engines, as ```` ```{python} ````, or in R Markdown, inline R. A plain
/// ```` ```python ```` block only shows code.
pub fn has_chunks(text: &str, rmd: bool) -> bool {
    let chunk = |line: &str| {
        line.trim_start()
            .strip_prefix("```")
            .and_then(|rest| rest.trim_start().strip_prefix('{'))
            .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_alphabetic()))
    };
    text.lines().any(chunk) || (rmd && text.contains("`r "))
}

/// Run `rmd`'s chunks with knitr, into a markdown file in a directory of its
/// own, with the plots beside it. Chunks run in `rmd`'s directory, as
/// knitting there would run them, until they end or `stop` is set.
pub fn knit(rmd: &Path, stop: &AtomicBool) -> Knitted {
    run(rmd, stop).unwrap_or_else(Knitted::Failed)
}

/// As `knit`, with why it failed as an error, to be given with `?`.
fn run(rmd: &Path, stop: &AtomicBool) -> Result<Knitted, String> {
    let rmd = std::path::absolute(rmd).map_err(|e| e.to_string())?;
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let out = dir.path().join("knitted.md");
    let figures = format!("{}/figure/", dir.path().display());
    const SCRIPT: &str = "a <- commandArgs(TRUE); \
        knitr::opts_chunk$set(fig.path = a[3]); \
        invisible(knitr::knit(a[1], a[2], quiet = TRUE, envir = new.env()))";
    // To a file, not a pipe, so R is waited on without reading it.
    let log = dir.path().join("stderr.txt");
    let stderr = std::fs::File::create(&log).map_err(|e| e.to_string())?;
    // R's own temporary files, and its chunks', go with the directory.
    let tmp = dir.path().join("tmp");
    std::fs::create_dir(&tmp).map_err(|e| e.to_string())?;
    let mut command = Command::new("Rscript");
    command
        .args(["-e", SCRIPT])
        .arg(&rmd)
        .arg(&out)
        .arg(&figures)
        .current_dir(dir.path())
        .env("TMPDIR", &tmp)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr);
    let status = match child::run(command, stop, None) {
        Ok(Ended::Exited(status)) => status,
        Ok(Ended::Stopped) => return Ok(Knitted::Stopped),
        Ok(Ended::Paused) => return Err("knitting paused, chunks shown as code".into()),
        // Given no time limit, it cannot run past one; were it, the chunks
        // still show.
        Ok(Ended::TimedOut) => return Err("knitting timed out, chunks shown as code".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err("R not found: chunks shown as code".into());
        }
        Err(e) => return Err(format!("cannot run R: {e}")),
    };
    if !status.success() {
        let why = child::last_line(&log, |l| !l.starts_with("Execution halted"));
        return Err(format!(
            "knitting failed, chunks shown as code: {}",
            why.as_deref().unwrap_or("knitr failed")
        ));
    }
    let md = std::fs::read_to_string(&out).map_err(|e| format!("knitr wrote nothing: {e}"))?;
    Ok(Knitted::Done(dir, md))
}

#[cfg(test)]
#[path = "tests/knit.rs"]
mod tests;
