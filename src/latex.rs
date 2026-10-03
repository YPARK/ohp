//! Formulas typst cannot set, rendered by LaTeX where it is installed.
//!
//! Each formula is set alone on a page cropped to it, and its height and
//! depth are read from LaTeX's log, so it can sit on the baseline of the
//! text around it. Formulas are rendered at once, a LaTeX run each, and
//! kept, the most recently asked for of them, so a reload renders only those
//! that changed. A run that takes too long, as a macro that expands for
//! ever, is ended; tried again once, as LaTeX making a font may be slow the
//! first time, it has failed.

use std::collections::{HashMap, HashSet};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use crate::process::child::{self, Ended};

/// Most LaTeX runs at once, well within the programs ohp may run.
const MAX_WORKERS: usize = 16;
const _: () = assert!(MAX_WORKERS < child::MOST);

/// Longest a formula's LaTeX run may take.
const LATEX_TIMEOUT: Duration = Duration::from_secs(10);

/// LaTeX's own size, which the rendered height and depth are measured in.
const LATEX_PT: f32 = 10.;

/// A formula as LaTeX set it.
#[derive(Clone, Debug)]
pub struct Rendered {
    pub pdf: Arc<[u8]>,
    /// Above and below the baseline, in ems of the text it is set in.
    pub height: f32,
    pub depth: f32,
}

/// A formula, and whether it is displayed apart from the text.
pub type Formula = (String, bool);

static AVAILABLE: LazyLock<bool> = LazyLock::new(|| {
    Command::new("pdflatex")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
});

/// Formulas kept rendered: past this, those asked for least recently go.
/// Every edit of a formula, or of the preamble, renders another.
const KEPT: usize = 512;

/// What a formula's LaTeX run came to, as kept: set, failed, or timed out
/// once, to be tried again.
#[derive(Clone, Debug)]
enum Kept {
    Set(Rendered),
    Failed,
    TimedOut,
}

/// Each formula LaTeX rendered before, or failed to, by preamble and
/// formula, with when it was last asked for.
type Done = HashMap<(String, Formula), (u64, Kept)>;

/// What was rendered, and how many times `render` has been asked.
static RENDERED: LazyLock<Mutex<(Done, u64)>> = LazyLock::new(Mutex::default);

pub fn available() -> bool {
    *AVAILABLE
}

/// The `formulas` LaTeX can render after `preamble`; those not yet rendered
/// once `stop` is set are left out.
pub fn render(
    formulas: &[Formula],
    preamble: &str,
    stop: &AtomicBool,
) -> HashMap<Formula, Rendered> {
    let workers = std::thread::available_parallelism().map_or(1, |n| n.get());
    render_on(formulas, preamble, stop, workers.min(MAX_WORKERS))
}

/// As `render`, with `workers` LaTeX runs at once.
fn render_on(
    formulas: &[Formula],
    preamble: &str,
    stop: &AtomicBool,
    workers: usize,
) -> HashMap<Formula, Rendered> {
    let key = |f: &Formula| (preamble.to_string(), f.clone());
    // What this call returns is taken as it goes, not read back, so
    // another call evicting meanwhile takes nothing from it. All it asks
    // for is stamped alike, and so kept or evicted together.
    let mut found = HashMap::new();
    // To be run, and whether each timed out before.
    let mut fresh: Vec<(&Formula, bool)> = Vec::new();
    let mut seen = HashSet::new();
    let now = {
        let (done, asked) = &mut *RENDERED.lock().expect("rendered formulas");
        *asked += 1;
        for f in formulas {
            if !seen.insert(f) {
                continue;
            }
            match done.get_mut(&key(f)) {
                Some((used, Kept::Set(rendered))) => {
                    *used = *asked;
                    found.insert(f.clone(), rendered.clone());
                }
                Some((used, Kept::Failed)) => *used = *asked,
                Some((_, Kept::TimedOut)) => fresh.push((f, true)),
                None => fresh.push((f, false)),
            }
        }
        *asked
    };
    let mut made = Vec::new();
    for batch in fresh.chunks(workers) {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        // A run stopped is neither set nor failed, and is not kept.
        let runs: Vec<Option<Kept>> = std::thread::scope(|s| {
            let runs: Vec<_> = batch
                .iter()
                .map(|(f, _)| s.spawn(|| run(&f.0, f.1, preamble, stop, LATEX_TIMEOUT)))
                .collect();
            runs.into_iter()
                .map(|r| r.join().unwrap_or(Some(Kept::Failed)))
                .collect()
        });
        for (&(f, timed_out_before), kept) in batch.iter().zip(runs) {
            let kept = match kept {
                Some(Kept::TimedOut) if timed_out_before => Kept::Failed,
                Some(kept) => kept,
                None => continue,
            };
            made.push((f, kept));
        }
    }
    let (done, _) = &mut *RENDERED.lock().expect("rendered formulas");
    for (f, kept) in made {
        if let Kept::Set(rendered) = &kept {
            found.insert(f.clone(), rendered.clone());
        }
        done.insert(key(f), (now, kept));
    }
    evict(done, KEPT);
    found
}

/// Leave about `kept` formulas, those asked for most recently: those asked
/// for at once go together.
fn evict(done: &mut Done, kept: usize) {
    if done.len() <= kept {
        return;
    }
    let mut used: Vec<u64> = done.values().map(|(used, _)| *used).collect();
    used.sort_unstable();
    let oldest = used[done.len() - kept];
    done.retain(|_, (used, _)| *used >= oldest);
}

/// `formula` rendered by one LaTeX run, in a directory of its own, unless it
/// runs past `timeout`; `None` where it was stopped or paused, as it is
/// neither set nor failed and is not kept.
fn run(
    formula: &str,
    display: bool,
    preamble: &str,
    stop: &AtomicBool,
    timeout: Duration,
) -> Option<Kept> {
    let Ok(dir) = tempfile::tempdir() else {
        return Some(Kept::Failed);
    };
    let style = if display { "\\displaystyle" } else { "" };
    let tex = format!(
        "\\documentclass{{article}}\n\
         \\usepackage{{amsmath,amssymb,bm}}\n\
         {preamble}\n\
         \\usepackage[active,tightpage]{{preview}}\n\
         \\setlength\\PreviewBorder{{0pt}}\n\
         \\newsavebox\\ohpbox\n\
         \\begin{{document}}\n\
         \\sbox\\ohpbox{{${style} {formula}$}}\n\
         \\typeout{{OHP:\\the\\ht\\ohpbox:\\the\\dp\\ohpbox}}\n\
         \\begin{{preview}}\\usebox\\ohpbox\\end{{preview}}\n\
         \\end{{document}}\n"
    );
    if std::fs::write(dir.path().join("f.tex"), tex).is_err() {
        return Some(Kept::Failed);
    }
    let mut command = Command::new("pdflatex");
    command
        .args([
            "-interaction=nonstopmode",
            "-halt-on-error",
            "-no-shell-escape",
            "f.tex",
        ])
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match child::run(command, stop, Some(timeout)) {
        Ok(Ended::Exited(status)) if status.success() => {
            Some(read(&dir).map_or(Kept::Failed, Kept::Set))
        }
        // Paused from outside, as stopped, it is no fault of the formula.
        Ok(Ended::Stopped | Ended::Paused) => None,
        Ok(Ended::TimedOut) => Some(Kept::TimedOut),
        Ok(Ended::Exited(_)) | Err(_) => Some(Kept::Failed),
    }
}

/// The formula pdflatex set in `dir`, measured.
fn read(dir: &tempfile::TempDir) -> Option<Rendered> {
    let (height, depth) = measure(&std::fs::read_to_string(dir.path().join("f.log")).ok()?)?;
    let pdf = std::fs::read(dir.path().join("f.pdf")).ok()?;
    Some(Rendered {
        pdf: pdf.into(),
        height: height / LATEX_PT,
        depth: depth / LATEX_PT,
    })
}

/// The height and depth the log reports, in points.
fn measure(log: &str) -> Option<(f32, f32)> {
    let line = log.lines().find_map(|l| l.strip_prefix("OHP:"))?;
    let (height, depth) = line.split_once(':')?;
    let pt = |s: &str| s.trim().strip_suffix("pt")?.parse::<f32>().ok();
    let (height, depth) = (pt(height)?, pt(depth)?);
    (height + depth > 0.).then_some((height, depth))
}

#[cfg(test)]
#[path = "tests/latex.rs"]
mod tests;
