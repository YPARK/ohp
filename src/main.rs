//! proj-beamer: present beamer slides in the terminal.
//!
//! Slides are rasterised in pure Rust by hayro and shown through
//! ratatui-image, over the kitty graphics protocol where the terminal has it,
//! so they can be presented from a remote machine over ssh.

mod app;
mod render;

use clap::Parser;
use std::path::PathBuf;

/// Present a beamer PDF in the terminal.
///
/// Keys: n/p next/previous slide, g or tab switches between the slide and
/// a grid of all slides, enter presents the slide picked in the grid, q quits.
///
/// The PDF is reloaded whenever it changes on disk, so recompiling the
/// slides updates them in place.
#[derive(Parser)]
#[command(version, about)]
struct Args {
    /// The slides.
    pdf: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let deck = render::Deck::open(&args.pdf)?;
    app::run(deck)
}
