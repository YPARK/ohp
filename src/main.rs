//! ohp, an overhead projector: present beamer slides in the terminal.
//!
//! Slides are rasterised in pure Rust by hayro and shown through
//! ratatui-image, over the kitty graphics protocol where the terminal has it,
//! so they can be presented from a remote machine over ssh. Elsewhere they
//! are shown as their text, over a coarse image of the slide.
//!
//! Markdown, R Markdown and HTML are typeset into a PDF first, by typst.

use clap::Parser;
#[cfg(any(feature = "markdown", feature = "speech", feature = "summary"))]
use ohp::process;
#[cfg(feature = "markdown")]
use ohp::typeset;
#[cfg(feature = "speech")]
use ohp::voices;
use ohp::{app, browse, print, remote, render, shell_status, speak, summary};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(feature = "speech")]
use std::time::Duration;

#[cfg_attr(
    feature = "markdown",
    doc = "Present a beamer PDF, or markdown, R Markdown or HTML, in the terminal."
)]
#[cfg_attr(
    not(feature = "markdown"),
    doc = "Present a beamer PDF in the terminal."
)]
///
/// Keys: n/p or arrows next/previous slide (up/down move a row in the grid),
/// g or tab switches between the slide and a grid of all slides, enter
/// presents the slide picked in the grid, t switches between images and
/// text, s reads the slides aloud from this one on, k sums up the slide on
/// screen and marks its key sentences, K sums up the whole file, m picks
/// the model that sums up, A annotates and signs the PDF, P prints, q
/// quits.
///
/// +/- zoom the slide, or the grid. Zoomed in, arrows pan the slide and 0
/// fits it back to the screen.
///
/// The file is reloaded whenever it changes on disk, so recompiling or
/// saving it updates the slides in place; r reloads it at once. Ctrl-O
/// picks another file to present, or several, marked with tab, or adds one
/// to a deck of several.
#[derive(Parser)]
#[command(version)]
struct Args {
    #[cfg_attr(
        feature = "markdown",
        doc = "The slides: a PDF, `.md`, `.Rmd` or `.html` file, or an image, here, or on another machine"
    )]
    #[cfg_attr(
        not(feature = "markdown"),
        doc = "The slides: a PDF here, or on another machine"
    )]
    /// as `[user@]host:path`, copied over ssh and reloaded when it changes
    /// there. Without one, or given a directory, the file is picked from a
    /// list.
    #[cfg_attr(
        feature = "markdown",
        doc = "Several files, here or not, PDFs, images, markdown and HTML mixed, are shown one after another as one deck."
    )]
    files: Vec<PathBuf>,
    /// Start with slides as text, even where the terminal shows images.
    #[arg(long)]
    text: bool,
    /// Print the file, as P does, with CUPS's lp, and quit, showing
    /// nothing.
    #[arg(long)]
    print: bool,
    /// Printer to print on, as `lpstat -e` names it. [default: the
    /// system's default]
    #[arg(long, requires = "print", value_name = "NAME")]
    printer: Option<String>,
    /// Pages to print: all, or as 3-7, 9.
    #[arg(long, requires = "print", default_value = "all")]
    pages: String,
    /// Copies to print.
    #[arg(long, requires = "print", default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=99))]
    copies: u32,
    /// Print on both sides of the paper.
    #[arg(long, requires = "print")]
    two_sided: bool,
    /// How pages are made the paper's size: auto, as they are if they fit
    /// and else shrunk; fit, as large as the paper takes; or actual, as
    /// they are.
    #[arg(long, requires = "print", default_value = "auto", value_parser = ["auto", "fit", "actual"])]
    scale: String,
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
    /// [default: the first of Piper's voices in ~/.local/share/piper, else
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
    /// Language model k sums pages up with: one ohp suggests, as
    /// qwen2.5-3b, qwen2.5-1.5b, qwen3-4b, llama3.2-3b or qwen2.5-7b, a
    /// repo on Hugging Face as owner/repo or owner/repo/file.gguf,
    /// downloaded once into its cache, or a GGUF file here; m picks
    /// another. [default: qwen2.5-3b on macOS, qwen2.5-1.5b elsewhere]
    #[cfg(feature = "summary")]
    #[arg(long, env = "OHP_MODEL", value_name = "MODEL", value_parser = summary::weights)]
    model: Option<summary::Weights>,
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

fn run(signaled: &Arc<AtomicUsize>) -> anyhow::Result<()> {
    let args = Args::parse();
    let files = match args.files.as_slice() {
        [] => browse::choose(".".as_ref())?,
        [dir] if dir.is_dir() => browse::choose(dir)?,
        files => Some(files.to_vec()),
    };
    let Some(files) = files else {
        return Ok(());
    };
    let (remotes, files) = remote::open_all("ssh".as_ref(), &files)?;
    // Files picked later are here. Clones share one stop, which a signal sets.
    let local = options(&args);
    let options = render::Options {
        // The data a remote file's chunks read is on the other machine.
        knit: local.knit && remotes.is_empty(),
        ..local.clone()
    };
    #[cfg(all(
        unix,
        any(feature = "markdown", feature = "speech", feature = "summary")
    ))]
    process::signals::install(&options.stop, signaled)?;
    let deck = render::Deck::open_with(&files, options);
    // Stopped while the deck was read: no screen to show.
    if signaled.load(Ordering::Relaxed) != 0 {
        return Ok(());
    }
    if args.print {
        return print(&deck?, &args);
    }
    let links = remotes.iter().map(remote::Remote::link).collect();
    app::run(
        deck?,
        args.text,
        links,
        local,
        speaker(&args),
        summarizer(&args),
        signaled.clone(),
    )
}

/// Print `deck` as `args` ask, and say what became of it.
fn print(deck: &render::Deck, args: &Args) -> anyhow::Result<()> {
    let pages = print::pages(&args.pages, 0, deck.pages).map_err(anyhow::Error::msg)?;
    let job = print::Job {
        printer: args.printer.clone(),
        copies: args.copies,
        pages,
        scale: match args.scale.as_str() {
            "fit" => print::Scale::Fit,
            "actual" => print::Scale::Actual,
            _ => print::Scale::Auto,
        },
        sides: match args.two_sided {
            true => print::Sides::Two,
            false => print::Sides::Printer,
        },
        title: render::name(&deck.paths[0]),
    };
    println!("{}", print::print(&deck.data, &job)?);
    Ok(())
}

#[cfg(feature = "summary")]
fn summarizer(args: &Args) -> summary::Summarizer {
    summary::Summarizer::new(args.model.clone())
}

#[cfg(not(feature = "summary"))]
fn summarizer(_: &Args) -> summary::Summarizer {
    summary::Summarizer::new(None)
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
