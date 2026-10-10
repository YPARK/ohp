//! ohp, an overhead projector: present beamer slides in the terminal.
//!
//! Slides are rasterised in pure Rust by hayro and shown through
//! ratatui-image, over the kitty graphics protocol where the terminal has it,
//! so they can be presented from a remote machine over ssh. Elsewhere they
//! are shown as their text, over a coarse image of the slide.
//!
//! Markdown and R Markdown are typeset into a PDF first, by typst.

mod app;
mod browse;
#[cfg(feature = "markdown")]
mod cite;
#[cfg(feature = "markdown")]
mod csl;
#[cfg(feature = "markdown")]
mod knit;
#[cfg(feature = "markdown")]
mod latex;
#[cfg(feature = "markdown")]
mod markdown;
#[cfg(feature = "markdown")]
mod math;
mod menu;
#[cfg(any(feature = "markdown", feature = "speech"))]
mod process;
mod remote;
mod render;
mod speak;
mod text;
#[cfg(feature = "markdown")]
mod typeset;
#[cfg(feature = "speech")]
mod voices;

#[cfg(test)]
#[path = "tests/fixture.rs"]
mod fixture;

use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(feature = "speech")]
use std::time::Duration;

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
/// text, s reads the slides aloud from this one on, q quits.
///
/// +/- zoom the slide, or the grid. Zoomed in, arrows pan the slide and 0
/// fits it back to the screen.
///
/// The file is reloaded whenever it changes on disk, so recompiling or
/// saving it updates the slides in place; r reloads it at once. Ctrl-O
/// picks another file to present.
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
    /// there. Without one, or given a directory, the file is picked from a
    /// list.
    file: Option<PathBuf>,
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
    /// Shell command that writes the text on its input as WAV audio on its
    /// output, to read slides aloud with s, as `piper -m voice.onnx -f -`.
    /// [default: Piper with the first voice in ~/.local/share/piper, else
    /// macOS's say, else espeak-ng --stdout, or espeak's]
    #[cfg(feature = "speech")]
    #[arg(long, env = "OHP_VOICE", value_name = "COMMAND")]
    voice: Option<String>,
    /// Shell command that plays the WAV audio on its input. [default: aplay
    /// -q, paplay, afplay /dev/stdin or sox's play, the first found]
    #[cfg(feature = "speech")]
    #[arg(long, env = "OHP_PLAY", value_name = "COMMAND")]
    play: Option<String>,
    /// Seconds paused, reading aloud, where a formula or a table is left
    /// out.
    #[cfg(feature = "speech")]
    #[arg(long, value_name = "SECONDS", default_value = "0.5", value_parser = seconds)]
    pause: Duration,
}

/// A time in seconds, as `0.5`.
#[cfg(feature = "speech")]
fn seconds(s: &str) -> Result<Duration, String> {
    let secs: f64 = s
        .parse()
        .map_err(|_| format!("`{s}` is not a number of seconds"))?;
    Duration::try_from_secs_f64(secs).map_err(|_| format!("`{s}` is not a time to pause"))
}

fn main() -> anyhow::Result<ExitCode> {
    // The signal that stopped ohp, if one did.
    let signaled = Arc::new(AtomicUsize::new(0));
    let ran = run(&signaled);
    match signaled.load(Ordering::Relaxed) {
        0 => ran.map(|()| ExitCode::SUCCESS),
        // As a shell reports a signal. The error stopping made is not
        // worth saying; another is.
        signal => {
            if let Err(e) = ran
                && !e.chain().any(|c| c.is::<render::Stopped>())
            {
                eprintln!("Error: {e:?}");
            }
            Ok(ExitCode::from(shell_status(signal)))
        }
    }
}

/// The status a shell gives a process a signal ended.
pub fn shell_status(signal: usize) -> u8 {
    u8::try_from(128 + signal).unwrap_or(u8::MAX)
}

fn run(signaled: &Arc<AtomicUsize>) -> anyhow::Result<()> {
    let args = Args::parse();
    let file = match &args.file {
        Some(file) if !file.is_dir() => file.clone(),
        given => match browse::choose(given.as_deref().unwrap_or(".".as_ref()))? {
            Some(file) => file,
            None => return Ok(()),
        },
    };
    let remote = match remote::split(&file) {
        Some((host, path)) => Some(remote::Remote::open("ssh".as_ref(), host, path)?),
        None => None,
    };
    let path = remote.as_ref().map_or(file.as_path(), |r| r.path());
    // Files picked later are here. Clones share one stop, which a signal sets.
    let local = options(&args);
    let options = render::Options {
        // The data a remote file's chunks read is on the other machine.
        knit: local.knit && remote.is_none(),
        ..local.clone()
    };
    #[cfg(all(unix, any(feature = "markdown", feature = "speech")))]
    process::signals::install(&options.stop, signaled)?;
    let deck = render::Deck::open_with(path, options);
    // Stopped while the deck was read: no screen to show.
    if signaled.load(Ordering::Relaxed) != 0 {
        return Ok(());
    }
    let link = remote.as_ref().map(remote::Remote::link);
    app::run(
        deck?,
        args.text,
        link,
        local,
        speaker(&args),
        signaled.clone(),
    )
}

#[cfg(feature = "speech")]
fn speaker(args: &Args) -> speak::Speaker {
    let (voice, play) = (args.voice.clone(), args.play.clone());
    speak::Speaker::new(voices::default(voice), speak::player(play)).with_pause(args.pause)
}

#[cfg(not(feature = "speech"))]
fn speaker(_: &Args) -> speak::Speaker {
    speak::Speaker::new(None, None)
}

#[cfg(feature = "markdown")]
fn options(args: &Args) -> render::Options {
    render::Options {
        paper: args.paper.clone(),
        knit: !args.no_knit,
        ..render::Options::default()
    }
}

#[cfg(not(feature = "markdown"))]
fn options(_: &Args) -> render::Options {
    render::Options::default()
}
