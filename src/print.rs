//! Printing what is shown, as the PDF it is, with CUPS's `lp`, which macOS
//! and Linux both have, and BSD's `lpr` where there is no `lp`.
//!
//! Every file ohp shows is a PDF by then, markdown and images typeset into
//! one, and a PDF being signed is the copy with all that is put, so what
//! is printed is what is seen. It is handed to `lp` on its input, never
//! written where another could read or swap it first. The printers are
//! the queues `lpstat` lists, the default first, then those it finds on
//! the network with no queue.

use anyhow::Context;
use std::ffi::{OsStr, OsString};
use std::io::{ErrorKind, Write};
use std::path::Path;
use std::process::{Command, Stdio};

/// Which sides of the paper are printed on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sides {
    /// As the printer is set to.
    Printer,
    One,
    Two,
}

impl Sides {
    pub const ALL: [Sides; 3] = [Sides::Printer, Sides::One, Sides::Two];
}

/// How a page is made the paper's size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scale {
    /// As it is if it fits the paper, else shrunk to fit: CUPS's own.
    Auto,
    /// Made as large as the paper takes.
    Fit,
    /// As it is, what is off the paper cut off.
    Actual,
}

impl Scale {
    pub const ALL: [Scale; 3] = [Scale::Auto, Scale::Fit, Scale::Actual];
}

/// What to print, and how.
#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    /// The printer, by its CUPS name; the default if `None`.
    pub printer: Option<String>,
    pub copies: u32,
    /// The pages, as CUPS reads them, `3-7,9`; all if `None`.
    pub pages: Option<String>,
    pub sides: Sides,
    pub scale: Scale,
    /// Its pages are wider than tall, as slides: printed across the sheet,
    /// so turned on its short edge, two-sided.
    pub wide: bool,
    /// What the printer's queue calls it.
    pub title: String,
}

impl Job {
    /// One copy of all of it, on the default printer, as it is set.
    pub fn new(title: &str) -> Self {
        Job {
            printer: None,
            copies: 1,
            pages: None,
            sides: Sides::Printer,
            scale: Scale::Auto,
            wide: false,
            title: title.to_string(),
        }
    }
}

/// The pages `text` names, `current` the page shown, from 0, of `total`,
/// as CUPS reads them: `None` for all, as `all` or nothing; `this` or `.`
/// for the page shown; else pages and ranges, `3-7, 9`, a range open at
/// either end running to the first or last page, put in order and those
/// that overlap or meet made one.
pub fn pages(text: &str, current: usize, total: usize) -> Result<Option<Ranges>, String> {
    let text = text.trim();
    if text.is_empty() || text.eq_ignore_ascii_case("all") {
        return Ok(None);
    }
    if text == "." || text.eq_ignore_ascii_case("this") {
        return Ok(Some(Ranges(vec![(current + 1, current + 1)])));
    }
    let page = |s: &str, or: usize| -> Result<usize, String> {
        let s = s.trim();
        if s.is_empty() {
            return Ok(or);
        }
        match s.parse::<usize>() {
            Ok(n) if (1..=total).contains(&n) => Ok(n),
            Ok(n) => Err(format!("there is no page {n}: 1 to {total}")),
            Err(_) => Err(format!("{s} is not a page: as 3-7, 9, or all")),
        }
    };
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for part in text.split(',') {
        if part.trim().is_empty() {
            return Err(format!("{text} is not pages: as 3-7, 9, or all"));
        }
        let (from, to) = match part.split_once('-') {
            Some((a, b)) => (page(a, 1)?, page(b, total)?),
            None => {
                let n = page(part, 0)?;
                (n, n)
            }
        };
        if from > to {
            return Err(format!("{} runs backwards", part.trim()));
        }
        ranges.push((from, to));
    }
    // In order, those that overlap or meet made one: CUPS takes no other.
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (from, to) in ranges {
        match merged.last_mut() {
            Some(last) if from <= last.1 + 1 => last.1 = last.1.max(to),
            _ => merged.push((from, to)),
        }
    }
    Ok(Some(Ranges(merged)))
}

/// Pages picked, as ranges of them from 1, each `(first, last)`, in order,
/// none overlapping or meeting another: as CUPS takes them.
#[derive(Clone, Debug, PartialEq)]
pub struct Ranges(Vec<(usize, usize)>);

impl Ranges {
    /// The pages, from 0.
    pub fn pages(&self) -> impl Iterator<Item = usize> + '_ {
        self.0.iter().flat_map(|&(from, to)| from - 1..to)
    }
}

/// As CUPS's `page-ranges` reads them: `3-7,9`.
impl std::fmt::Display for Ranges {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let each: Vec<String> = (self.0.iter())
            .map(|&(from, to)| match from == to {
                true => from.to_string(),
                false => format!("{from}-{to}"),
            })
            .collect();
        f.write_str(&each.join(","))
    }
}

/// The printers CUPS lists, and whether the first is its default: it
/// names none, else.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Printers {
    pub names: Vec<String>,
    pub default: bool,
}

/// The printers: the queues `lpstat -p -d` lists, `queues`, the default
/// it names first; then those `lpstat -e` lists, `found`, with no queue,
/// as a printer found on the network.
pub fn destinations(queues: &str, found: &str) -> Printers {
    let mut all: Vec<String> = queues
        .lines()
        .filter_map(|l| l.strip_prefix("printer "))
        .filter_map(|l| l.split_whitespace().next())
        .map(String::from)
        .collect();
    let default = queues
        .lines()
        .find_map(|l| l.strip_prefix("system default destination:"))
        .map(str::trim);
    let named = default.and_then(|d| all.iter().position(|p| p == d));
    if let Some(i) = named {
        let first = all.remove(i);
        all.insert(0, first);
    }
    for name in found.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if !all.iter().any(|p| p == name) {
            all.push(name.to_string());
        }
    }
    Printers {
        names: all,
        default: named.is_some(),
    }
}

/// The printers here, the default first; none where CUPS is not.
pub fn printers() -> Printers {
    let lpstat = |flags: &[&str]| {
        // In English, whatever the language here: macOS's heeds the
        // locale only with SOFTWARE set.
        Command::new("lpstat")
            .args(flags)
            .env("LC_ALL", "C")
            .env("LANG", "C")
            .env("SOFTWARE", "ohp")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    };
    destinations(&lpstat(&["-p", "-d"]), &lpstat(&["-e"]))
}

/// The flags a printing program names the printer, the copies and the
/// title by; the rest, `-o`, `lp` and `lpr` share.
pub struct Flags {
    printer: &'static str,
    copies: &'static str,
    title: &'static str,
}

/// CUPS's `lp`'s.
pub const LP: Flags = Flags {
    printer: "-d",
    copies: "-n",
    title: "-t",
};

/// BSD's `lpr`'s.
pub const LPR: Flags = Flags {
    printer: "-P",
    copies: "-#",
    title: "-T",
};

/// What `lp`, or `lpr`, as `flags` say, is asked, to print what it reads
/// as `job` has it.
pub fn args(job: &Job, flags: &Flags) -> Vec<OsString> {
    let mut args: Vec<OsString> = Vec::new();
    let mut push = |a: &str, b: &str| {
        args.push(a.into());
        args.push(b.into());
    };
    if let Some(printer) = &job.printer {
        push(flags.printer, printer);
    }
    if job.copies > 1 {
        push(flags.copies, &job.copies.to_string());
    }
    if let Some(pages) = &job.pages {
        push("-o", &format!("page-ranges={pages}"));
    }
    match job.sides {
        Sides::Printer => {}
        Sides::One => push("-o", "sides=one-sided"),
        Sides::Two if job.wide => push("-o", "sides=two-sided-short-edge"),
        Sides::Two => push("-o", "sides=two-sided-long-edge"),
    }
    match job.scale {
        Scale::Auto => {}
        Scale::Fit => push("-o", "print-scaling=fit"),
        Scale::Actual => push("-o", "print-scaling=none"),
    }
    push(flags.title, &job.title);
    args
}

/// Whether the pages `pages` picks, of these sizes, are, most of them,
/// wider than tall: all if `None`.
pub fn wide(sizes: &[(f32, f32)], pages: Option<&Ranges>) -> bool {
    let picked = picked(pages, sizes.len());
    let wide = picked.iter().filter(|&&i| sizes[i].0 > sizes[i].1).count();
    2 * wide > picked.len()
}

/// The pages `pages` picks, of `total`, from 0, as CUPS prints them: in
/// the document's order, each once; all if `None`.
pub fn picked(pages: Option<&Ranges>, total: usize) -> Vec<usize> {
    match pages {
        Some(pages) => pages.pages().filter(|&p| p < total).collect(),
        None => (0..total).collect(),
    }
}

/// The size of the paper CUPS names `name`, in points, wide by tall: as
/// `Letter`, `A4`, `w288h432` or `Custom.200x300`, a variant of one, as
/// `Letter.Fullbleed`, as large as it.
pub fn paper(name: &str) -> Option<(f32, f32)> {
    if let Some(custom) = name.strip_prefix("Custom.") {
        let (w, h) = custom.split_once('x')?;
        return Some((w.parse().ok()?, h.parse().ok()?));
    }
    let name = name.split('.').next()?;
    if let Some((w, h)) = name.strip_prefix('w').and_then(|n| n.split_once('h')) {
        return Some((w.parse().ok()?, h.parse().ok()?));
    }
    let size = match name.to_ascii_lowercase().as_str() {
        "letter" => (612., 792.),
        "legal" => (612., 1008.),
        "tabloid" => (792., 1224.),
        "ledger" => (1224., 792.),
        "executive" => (522., 756.),
        "statement" => (396., 612.),
        "a3" => (842., 1191.),
        "a4" => (595., 842.),
        "a5" => (420., 595.),
        "a6" => (298., 420.),
        "b5" => (499., 709.),
        "4x6" => (288., 432.),
        "5x7" => (360., 504.),
        "5x8" => (360., 576.),
        "env10" => (297., 684.),
        "envdl" => (312., 624.),
        "envc5" => (459., 649.),
        _ => return None,
    };
    Some(size)
}

/// The paper `lpoptions -l` says a printer is set to: its page size marked
/// `*`, the default.
pub fn default_paper(options: &str) -> Option<String> {
    let line = options.lines().find(|l| l.starts_with("PageSize"))?;
    let (_, choices) = line.split_once(':')?;
    let marked = choices
        .split_whitespace()
        .find_map(|c| c.strip_prefix('*'))?;
    Some(marked.to_string())
}

/// The name and size of the paper `printer` is set to, the default
/// printer's if `None`; none where CUPS does not say.
pub fn paper_of(printer: Option<&str>) -> Option<(String, (f32, f32))> {
    let mut lpoptions = Command::new("lpoptions");
    if let Some(printer) = printer {
        lpoptions.args(["-p", printer]);
    }
    let out = lpoptions.arg("-l").env("LC_ALL", "C").output().ok()?;
    let name = default_paper(&String::from_utf8_lossy(&out.stdout))?;
    let size = paper(&name)?;
    Some((name, size))
}

/// Where a page `page` big, wide by tall in points, is printed on paper
/// `paper` big, as `scale` has it: the sheet, turned to lie as the page
/// does, as CUPS turns it, and the page's box on it, `[x, y, w, h]` from
/// its top left; off the sheet where the page is larger and not shrunk.
pub fn placed(page: (f32, f32), paper: (f32, f32), scale: Scale) -> ((f32, f32), [f32; 4]) {
    let (pw, ph) = page;
    let (a, b) = (paper.0.min(paper.1), paper.0.max(paper.1));
    let sheet = if pw > ph { (b, a) } else { (a, b) };
    let fit = (sheet.0 / pw).min(sheet.1 / ph);
    let by = match scale {
        Scale::Fit => fit,
        Scale::Actual => 1.,
        Scale::Auto => fit.min(1.),
    };
    let (w, h) = (pw * by, ph * by);
    (sheet, [(sheet.0 - w) / 2., (sheet.1 - h) / 2., w, h])
}

/// The request `lp` says it made: `Office-12` of `request id is Office-12
/// (1 file(s))`.
pub fn request(said: &str) -> Option<String> {
    let rest = said.split("request id is ").nth(1)?;
    rest.split_whitespace().next().map(String::from)
}

/// Print `pdf` as `job` has it; what to say of it.
pub fn print(pdf: &[u8], job: &Job) -> anyhow::Result<String> {
    print_with(&["lp".as_ref(), "lpr".as_ref()], pdf, job)
}

/// Print `pdf` with the first of `programs` there is, `lp` or `lpr`, as
/// its name says, handing it `pdf` on its input.
pub fn print_with(programs: &[&OsStr], pdf: &[u8], job: &Job) -> anyhow::Result<String> {
    for &program in programs {
        let lpr = Path::new(program).file_name() == Some(OsStr::new("lpr"));
        let args = args(job, if lpr { &LPR } else { &LP });
        let shown = program.to_string_lossy();
        let child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match child {
            Err(e) if e.kind() == ErrorKind::NotFound => continue,
            child => child.with_context(|| format!("cannot run {shown}"))?,
        };
        let mut input = child.stdin.take().expect("piped");
        // Written while what it says is read, so neither waits on the other.
        let out = std::thread::scope(|scope| {
            let writing = scope.spawn(move || input.write_all(pdf));
            let out = child.wait_with_output();
            (out, writing.join())
        });
        let (out, written) = out;
        let out = out.with_context(|| format!("cannot run {shown}"))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            anyhow::bail!("{shown} could not print it: {}", err.trim());
        }
        if let Ok(Err(e)) = written {
            anyhow::bail!("{shown} did not take it all: {e}");
        }
        let said = String::from_utf8_lossy(&out.stdout);
        return Ok(match request(&said) {
            Some(id) => format!("sent to the printer: {id}"),
            None => "sent to the printer".into(),
        });
    }
    anyhow::bail!("nothing to print with: CUPS's lp, or lpr, is needed")
}

#[cfg(test)]
#[path = "tests/print.rs"]
mod tests;
