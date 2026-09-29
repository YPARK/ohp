//! ohp, an overhead projector: present beamer slides in the terminal.
//!
//! Slides are rasterised in pure Rust by hayro and shown through
//! ratatui-image, over the kitty graphics protocol where the terminal has it,
//! so they can be presented from a remote machine over ssh. Elsewhere they
//! are shown as their text, over a coarse image of the slide.

mod app;
mod remote;
mod render;
mod text;

#[cfg(test)]
#[path = "tests/fixture.rs"]
mod fixture;

use clap::Parser;
use std::path::PathBuf;

/// Present a beamer PDF in the terminal.
///
/// Keys: n/p or arrows next/previous slide (up/down move a row in the grid),
/// g or tab switches between the slide and a grid of all slides, enter
/// presents the slide picked in the grid, t switches between images and
/// text, q quits.
///
/// +/- zoom the slide, or the grid. Zoomed in, arrows pan the slide and 0
/// fits it back to the screen.
///
/// The PDF is reloaded whenever it changes on disk, so recompiling the
/// slides updates them in place.
#[derive(Parser)]
#[command(version, about)]
struct Args {
    /// The slides: a file here, or on another machine as `[user@]host:path`,
    /// copied over ssh and reloaded when it changes there.
    pdf: PathBuf,
    /// Start with slides as text, even where the terminal shows images.
    #[arg(long)]
    text: bool,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let remote = match remote::split(&args.pdf) {
        Some((host, path)) => Some(remote::Remote::open("ssh".as_ref(), host, path)?),
        None => None,
    };
    let path = remote.as_ref().map_or(args.pdf.as_path(), |r| r.path());
    let deck = render::Deck::open(path)?;
    app::run(deck, args.text)
}
