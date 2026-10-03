//! Formulas typst cannot set, rendered by LaTeX where it is installed.
//!
//! Each formula is set alone on a page cropped to it, and its height and
//! depth are read from LaTeX's log, so it can sit on the baseline of the
//! text around it. Formulas are rendered at once, a LaTeX run each, and
//! kept, the most recently asked for of them, so a reload renders only those
//! that changed.

use std::collections::{HashMap, HashSet};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

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

/// Each formula LaTeX rendered before, or failed to, by preamble and
/// formula, with when it was last asked for.
type Done = HashMap<(String, Formula), (u64, Option<Rendered>)>;

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
    let key = |f: &Formula| (preamble.to_string(), f.clone());
    // What this call returns is taken as it goes, not read back, so
    // another call evicting meanwhile takes nothing from it.
    let mut found = HashMap::new();
    let fresh: Vec<&Formula> = {
        let (done, asked) = &mut *RENDERED.lock().expect("rendered formulas");
        *asked += 1;
        let mut seen = HashSet::new();
        formulas
            .iter()
            .filter(|f| match done.get_mut(&key(f)) {
                Some((used, rendered)) => {
                    *used = *asked;
                    found.extend(rendered.clone().map(|r| ((*f).clone(), r)));
                    false
                }
                None => seen.insert(*f),
            })
            .collect()
    };
    let workers = std::thread::available_parallelism().map_or(1, |n| n.get());
    for batch in fresh.chunks(workers) {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let results: Vec<Option<Rendered>> = std::thread::scope(|s| {
            let runs: Vec<_> = batch
                .iter()
                .map(|f| s.spawn(|| run(&f.0, f.1, preamble)))
                .collect();
            runs.into_iter().map(|r| r.join().ok().flatten()).collect()
        });
        let (done, asked) = &mut *RENDERED.lock().expect("rendered formulas");
        *asked += 1;
        for (f, result) in batch.iter().zip(results) {
            found.extend(result.clone().map(|r| ((*f).clone(), r)));
            done.insert(key(f), (*asked, result));
        }
        evict(done, KEPT);
    }
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

/// `formula` rendered by one LaTeX run, in a directory of its own.
fn run(formula: &str, display: bool, preamble: &str) -> Option<Rendered> {
    let dir = tempfile::tempdir().ok()?;
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
    std::fs::write(dir.path().join("f.tex"), tex).ok()?;
    let status = Command::new("pdflatex")
        .args([
            "-interaction=nonstopmode",
            "-halt-on-error",
            "-no-shell-escape",
            "f.tex",
        ])
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    if !status.success() {
        return None;
    }
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
