//! ohp, an overhead projector: present beamer slides in the terminal.
//!
//! Slides are rasterised in pure Rust by hayro and shown through
//! ratatui-image, over the kitty graphics protocol where the terminal has it,
//! so they can be presented from a remote machine over ssh. Elsewhere they
//! are shown as their text, over a coarse image of the slide.
//!
//! Markdown and R Markdown are typeset into a PDF first, by typst.

mod app;
#[cfg(feature = "markdown")]
mod cite;
#[cfg(feature = "markdown")]
mod csl;
#[cfg(feature = "markdown")]
mod latex;
#[cfg(feature = "markdown")]
mod markdown;
#[cfg(feature = "markdown")]
mod math;
mod remote;
mod render;
mod text;
#[cfg(feature = "markdown")]
mod typeset;

#[cfg(test)]
#[path = "tests/fixture.rs"]
mod fixture;

use clap::Parser;
use std::path::PathBuf;

#[cfg_attr(
    feature = "markdown",
    doc = "Present a beamer PDF, or markdown or R Markdown, in the terminal."
)]
#[cfg_attr(
    not(feature = "markdown"),
    doc = "Present a beamer PDF in the terminal."
)]
///
/// Keys: n/p or arrows next/previous slide (up/down move a row in the grid),
/// g or tab switches between the slide and a grid of all slides, enter
/// presents the slide picked in the grid, t switches between images and
/// text, q quits.
///
/// +/- zoom the slide, or the grid. Zoomed in, arrows pan the slide and 0
/// fits it back to the screen.
///
/// The file is reloaded whenever it changes on disk, so recompiling or
/// saving it updates the slides in place.
#[derive(Parser)]
#[command(version)]
struct Args {
    #[cfg_attr(
        feature = "markdown",
        doc = "The slides: a PDF, `.md` or `.Rmd` file here, or on another machine"
    )]
    #[cfg_attr(
        not(feature = "markdown"),
        doc = "The slides: a PDF here, or on another machine"
    )]
    /// as `[user@]host:path`, copied over ssh and reloaded when it changes
    /// there.
    file: PathBuf,
    /// Start with slides as text, even where the terminal shows images.
    #[arg(long)]
    text: bool,
    /// Paper to set markdown on, as typst names it: us-letter, a4,
    /// presentation-16-9, … [default: presentation-16-9 for slides,
    /// us-letter otherwise]
    #[cfg(feature = "markdown")]
    #[arg(long, value_parser = typeset::paper)]
    paper: Option<String>,
    /// Show code chunks, ```{r}, ```{python}, ```{bash} and the like, as
    /// code, without running them through knitr.
    #[cfg(feature = "markdown")]
    #[arg(long)]
    no_knit: bool,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let remote = match remote::split(&args.file) {
        Some((host, path)) => Some(remote::Remote::open("ssh".as_ref(), host, path)?),
        None => None,
    };
    let path = remote.as_ref().map_or(args.file.as_path(), |r| r.path());
    let options = options(&args, remote.is_some());
    #[cfg(all(unix, feature = "markdown"))]
    stop_on_signals(&options.stop)?;
    let deck = render::Deck::open_with(path, options)?;
    app::run(deck, args.text, remote.as_ref().map(remote::Remote::link))
}

/// Ask ohp to stop on Ctrl-C, hang-up or termination, so a knitr run, in a
/// process group of its own that the signal does not reach, is stopped with
/// it. Another signal asks again, rather than ending ohp before it has
/// stopped R and restored the terminal.
#[cfg(all(unix, feature = "markdown"))]
fn stop_on_signals(stop: &std::sync::Arc<std::sync::atomic::AtomicBool>) -> std::io::Result<()> {
    use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
    use signal_hook::flag;
    for signal in [SIGINT, SIGTERM, SIGHUP] {
        flag::register(signal, stop.clone())?;
    }
    Ok(())
}

#[cfg(feature = "markdown")]
fn options(args: &Args, remote: bool) -> render::Options {
    render::Options {
        paper: args.paper.clone(),
        // The data a remote file's chunks read is on the other machine.
        knit: !args.no_knit && !remote,
        ..render::Options::default()
    }
}

#[cfg(not(feature = "markdown"))]
fn options(_: &Args, _: bool) -> render::Options {
    render::Options::default()
}
