//! Markdown and R Markdown typeset into a PDF, in memory, by typst.
//!
//! A file with code chunks, R's, python's, bash's or any other engine
//! knitr has, is first run through knitr when R is here, so its chunks show
//! their output and plots; otherwise, or if knitting fails, they show their
//! code. Only the fonts typst carries are used, so a file looks
//! the same on every machine.
//!
//! A formula typst cannot set is rendered by LaTeX where it is installed,
//! or else shown as its source, and an image it cannot read is shown as its
//! description. If the document still does not compile, its citations are
//! left as written, and then its source is shown as it is.

use crate::latex;
use crate::markdown::{self, Picture, Setting};
use crate::render::Options;
use anyhow::Context;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
use tempfile::TempDir;
use typst::diag::{FileError, FileResult, SourceDiagnostic};
use typst::foundations::{Bytes, Datetime, Duration};
use typst::layout::Paper;
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World, WorldExt};
use typst_layout::PagedDocument;
use typst_pdf::PdfOptions;

/// Times typst is tried again with the formulas it failed on shown as LaTeX,
/// the images it failed on left out, or citations left as written.
const RETRIES: usize = 5;

/// How often a knitr run is checked on, for whether it is done or to stop.
const KNIT_POLL: std::time::Duration = std::time::Duration::from_millis(50);

/// The PDF, and anything the status line should say about it.
pub struct Typeset {
    pub pdf: Vec<u8>,
    pub note: Option<String>,
}

/// `name` if typst knows it as a paper size.
pub fn paper(name: &str) -> Result<String, String> {
    Paper::from_str(name)
        .map(|_| name.to_string())
        .map_err(|_| format!("unknown paper size {name:?}; try us-letter, a4 or presentation-16-9"))
}

pub fn typeset(path: &Path, options: &Options) -> anyhow::Result<Typeset> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let base = path.parent().unwrap_or(Path::new("."));
    // Why chunks were not run, which holds however the text is shown.
    let mut knitting = None;
    let rmd = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("rmd"));
    // The directory is kept until typeset: knitted plots are in it.
    let (text, _knitted) = if options.knit && has_chunks(&text, rmd) {
        match knit(path, &options.stop) {
            Ok((dir, md)) => (md, Some(dir)),
            Err(why) => {
                knitting = Some(why);
                (text, None)
            }
        }
    } else {
        (text, None)
    };

    let mut raw_math = HashSet::new();
    let mut source_only = HashSet::new();
    let mut broken = HashSet::new();
    // What was left out to typeset the text.
    let mut dropped: Vec<String> = Vec::new();
    let mut rendered = HashMap::new();
    let mut loaded = HashMap::new();
    let mut citations = true;
    let mut failure = String::new();
    // Typesetting for a deck no longer wanted, as when ohp is quitting.
    let stopped = || {
        if options.stop.load(Ordering::Relaxed) {
            anyhow::bail!("stopped");
        }
        Ok(())
    };
    for _ in 0..=RETRIES {
        stopped()?;
        let convert = |rendered: &HashMap<latex::Formula, latex::Rendered>| {
            let setting = Setting {
                base,
                citations,
                paper: options.paper.as_deref(),
                raw_math: &raw_math,
                source_only: &source_only,
                latex: rendered,
                broken: &broken,
            };
            markdown::convert(&text, &setting)
        };
        let mut doc = convert(&rendered);
        if !doc.unset.is_empty() && latex::available() {
            let fresh = latex::render(&doc.unset, &doc.preamble, &options.stop);
            if !fresh.is_empty() {
                rendered.extend(fresh);
                doc = convert(&rendered);
            }
        }
        stopped()?;
        let world = Doc::new(doc.source, &doc.images, &doc.files, &mut loaded);
        match compile(&world) {
            Ok(pdf) => {
                let notes: Vec<String> = knitting
                    .into_iter()
                    .chain(dropped)
                    .chain(doc.warnings)
                    .collect();
                let note = (!notes.is_empty()).then(|| notes.join("; "));
                return Ok(Typeset { pdf, note });
            }
            Err(errors) => {
                failure = errors[0].message.to_string();
                let spans: Vec<_> = errors
                    .iter()
                    .map(|e| (world.range(e.span), e.message.as_str()))
                    .collect();
                // An error is a formula's, an image's or a citation's where
                // it starts within it: one over a link or emphasis around
                // it starts before it, and is not its.
                let starts_in = |s: &Option<Range<usize>>, r: &Range<usize>| {
                    s.as_ref().is_some_and(|s| r.contains(&s.start))
                };
                let bad: Vec<usize> = doc
                    .formulas
                    .iter()
                    .enumerate()
                    .filter(|(i, f)| {
                        !source_only.contains(i) && spans.iter().any(|(s, _)| starts_in(s, f))
                    })
                    .map(|(i, _)| i)
                    .collect();
                let unread: Vec<(usize, &str)> = doc
                    .images
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| !broken.contains(i))
                    .filter_map(|(i, p)| {
                        let (_, message) = spans.iter().find(|(s, _)| starts_in(s, &p.at))?;
                        Some((i, *message))
                    })
                    .collect();
                // A bibliography typst read but could not set from: an
                // error in a citation or the works cited, or in no place
                // in the text, as a file typst read.
                let uncite = spans.iter().find(|(s, _)| {
                    doc.listed && (s.is_none() || doc.citing.iter().any(|c| starts_in(s, c)))
                });
                for &(i, message) in &unread {
                    let name = doc.images[i].path.file_name().unwrap_or_default();
                    dropped.push(format!("cannot show {}: {message}", name.to_string_lossy()));
                    broken.insert(i);
                }
                // Failing as typst's math, a formula is tried as LaTeX
                // rendered it; failing as that, it is shown as its source.
                for &i in &bad {
                    if !raw_math.insert(i) {
                        source_only.insert(i);
                    }
                }
                if let Some((_, message)) = uncite {
                    citations = false;
                    dropped.push(format!("citations left as written: {message}"));
                }
                if bad.is_empty() && unread.is_empty() && uncite.is_none() {
                    break;
                }
            }
        }
    }
    stopped()?;
    let source = format!(
        "#set page(paper: {})\n#raw(block: true, {})",
        markdown::string(options.paper.as_deref().unwrap_or(markdown::PAGE_PAPER)),
        markdown::string(&text)
    );
    let pdf = compile(&Doc::new(source, &[], &[], &mut loaded))
        .map_err(|e| anyhow::anyhow!("cannot typeset {}: {}", path.display(), e[0].message))?;
    // First, so the status line shows it however long the rest; what was
    // left out of a typeset text is not on screen to say.
    let why = format!("shown as source: {failure}");
    let notes: Vec<String> = [why].into_iter().chain(knitting).collect();
    Ok(Typeset {
        pdf,
        note: Some(notes.join("; ")),
    })
}

fn compile(world: &Doc) -> Result<Vec<u8>, Vec<SourceDiagnostic>> {
    let compiled = typst::compile::<PagedDocument>(world).output;
    // typst memoises across compiles in a global cache; what reloads stop
    // using would otherwise be kept for as long as ohp runs.
    typst::comemo::evict(10);
    let options = PdfOptions {
        tagged: false,
        ..PdfOptions::default()
    };
    typst_pdf::pdf(&compiled.map_err(|e| e.to_vec())?, &options).map_err(|e| e.to_vec())
}

/// Whether a file has anything for knitr to run: a chunk in any of its
/// engines, as ```` ```{python} ````, or in R Markdown, inline R. A plain
/// ```` ```python ```` block only shows code.
fn has_chunks(text: &str, rmd: bool) -> bool {
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
/// knitting there would run them, R's and those of knitr's other engines:
/// python, through reticulate, bash and the rest.
fn knit(rmd: &Path, stop: &AtomicBool) -> Result<(TempDir, String), String> {
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
    // A group of its own, so what its chunks start can be stopped with it.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let child = command.spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err("R not found: chunks shown as code".into());
        }
        Err(e) => return Err(format!("cannot run R: {e}")),
    };
    let status = loop {
        if stop.load(Ordering::Relaxed) {
            let _ = end(&mut child);
            return Err("knitting stopped".into());
        }
        match exited(&mut child, false) {
            Ok(true) => break end(&mut child),
            Ok(false) => std::thread::sleep(KNIT_POLL),
            Err(e) => {
                let _ = end(&mut child);
                return Err(format!("cannot run R: {e}"));
            }
        }
    };
    let status = status.map_err(|e| format!("cannot run R: {e}"))?;
    if !status.success() {
        let stderr = std::fs::read_to_string(&log).unwrap_or_default();
        let why = stderr
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty() && !l.starts_with("Execution halted"))
            .unwrap_or("knitr failed");
        return Err(format!(
            "knitting failed, chunks shown as code: {}",
            why.trim()
        ));
    }
    let md = std::fs::read_to_string(&out).map_err(|e| format!("knitr wrote nothing: {e}"))?;
    Ok((dir, md))
}

/// Whether R has exited, waiting for it if `block`. On unix it is left
/// unreaped, so its process group's id stays its own, and no other group
/// can be given it, until `end`.
fn exited(child: &mut Child, block: bool) -> std::io::Result<bool> {
    #[cfg(unix)]
    {
        let pid = libc::id_t::from(child.id());
        // SAFETY: a zeroed siginfo_t is valid, and waitid only fills it in.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let mut flags = libc::WEXITED | libc::WNOWAIT;
        if !block {
            flags |= libc::WNOHANG;
        }
        // SAFETY: `info` is a valid siginfo_t to write to.
        if unsafe { libc::waitid(libc::P_PID, pid, &mut info, flags) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: waitid filled in a child's pid, or left it zero.
        Ok(unsafe { info.si_pid() } != 0)
    }
    #[cfg(not(unix))]
    if block {
        child.wait().map(|_| true)
    } else {
        child.try_wait().map(|s| s.is_some())
    }
}

/// Kill R, if it still runs, and what its chunks started, its process
/// group, and reap it. The group is killed while R, unreaped, holds its id,
/// and again once R is gone, for a process forked as it was first killed.
fn end(child: &mut Child) -> std::io::Result<ExitStatus> {
    #[cfg(unix)]
    if let Ok(group) = libc::pid_t::try_from(child.id()) {
        let kill = || {
            // SAFETY: killpg only sends a signal, to R's own group.
            unsafe { libc::killpg(group, libc::SIGKILL) };
        };
        kill();
        exited(child, true)?;
        kill();
    }
    #[cfg(not(unix))]
    let _ = child.kill();
    child.wait()
}

static LIBRARY: LazyLock<LazyHash<Library>> = LazyLock::new(|| LazyHash::new(Library::default()));

static FONTS: LazyLock<(LazyHash<FontBook>, Vec<Font>)> = LazyLock::new(|| {
    let fonts: Vec<Font> = typst_assets::fonts()
        .flat_map(|data| Font::iter(Bytes::new(data)))
        .collect();
    (LazyHash::new(FontBook::from_fonts(&fonts)), fonts)
});

/// One typst document, the images it shows and the formulas LaTeX rendered
/// for it; it reads nothing else.
struct Doc {
    main: Source,
    files: HashMap<FileId, FileResult<Bytes>>,
}

impl Doc {
    /// `loaded` keeps the images read, for the next try at the document.
    fn new(
        source: String,
        images: &[Picture],
        rendered: &[(String, Arc<[u8]>)],
        loaded: &mut HashMap<PathBuf, FileResult<Bytes>>,
    ) -> Self {
        let id = |path: &str| {
            let vpath = VirtualPath::new(path).expect("a valid virtual path");
            RootedPath::new(VirtualRoot::Project, vpath).intern()
        };
        let mut files: HashMap<_, _> = images
            .iter()
            .map(|Picture { name, path, .. }| {
                let data = loaded.entry(path.clone()).or_insert_with(|| {
                    std::fs::read(path)
                        .map(Bytes::new)
                        .map_err(|e| FileError::from_io(e, path))
                });
                (id(name), data.clone())
            })
            .collect();
        for (name, pdf) in rendered {
            files.insert(id(name), Ok(Bytes::new(pdf.clone())));
        }
        Doc {
            main: Source::new(id("/main.typ"), source),
            files,
        }
    }
}

impl World for Doc {
    fn library(&self) -> &LazyHash<Library> {
        &LIBRARY
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &FONTS.0
    }

    fn main(&self) -> FileId {
        self.main.id()
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main.id() {
            Ok(self.main.clone())
        } else {
            Err(FileError::NotSource)
        }
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        match self.files.get(&id) {
            Some(file) => file.clone(),
            None => Err(FileError::NotFound(id.vpath().get_with_slash().into())),
        }
    }

    fn font(&self, index: usize) -> Option<Font> {
        FONTS.1.get(index).cloned()
    }

    fn today(&self, _: Option<Duration>) -> Option<Datetime> {
        None
    }
}

#[cfg(test)]
#[path = "tests/typeset.rs"]
mod tests;
