# ohp

An overhead projector: present beamer PDF slides, or markdown and R
Markdown, in the terminal.

Slides are rasterised in pure Rust by [hayro](https://crates.io/crates/hayro)
and shown through [ratatui-image](https://crates.io/crates/ratatui-image),
over the kitty graphics protocol where the terminal has it, so they can be
presented from a remote machine over ssh. Elsewhere they are shown as their
text, over a coarse image of the slide.

The file is reloaded whenever it changes, so recompiling the slides, or
saving the markdown, updates them in place.

## Install

Rust 1.92 or later:

```sh
cargo install --git https://github.com/YPARK/ohp
```

No C libraries, no poppler, no LaTeX needed. Markdown is typeset by
[typst](https://typst.app), whose evaluator assembles one small
stack-switching file with the system C toolchain; without markdown, nothing
but Rust is compiled:

```sh
cargo install --git https://github.com/YPARK/ohp --no-default-features --features speech
```

Reading slides aloud, the `speech` feature, is in the default build, and
reads with speech programs it runs, none built in: on macOS, its own `say`
and `afplay`, with nothing to install; on Linux, `espeak-ng` and ALSA's
`aplay`, as `sudo apt install espeak-ng alsa-utils`, or, far better, Piper,
set up as [Reading aloud](#reading-aloud) shows.

## Use

```sh
ohp                                  # pick the file from a list
ohp ~/talks                          # pick it from a list there
ohp talk.pdf
ohp --text talk.pdf                  # slides as text, even where images can be shown
ohp user@host:~/talks/talk.pdf       # slides on another machine
ohp talk.Rmd                         # R Markdown slides, chunks run by knitr
ohp --paper a4 notes.md              # a markdown document, on A4 pages
```

In the list, typing narrows it to the names that match, and a path typed
up to a `/`, as `../talks/` or `~/`, goes to that directory. Up and down
move, enter opens a file or goes into a folder, left or backspace on an
empty line goes up, and esc quits. While presenting, ctrl-o brings the
same list up over the slides.

| Key | |
|---|---|
| `n` / `p`, arrows | next / previous slide; up and down move a row in the grid |
| `g`, `tab` | switch between the slide and a grid of all slides |
| `enter` | present the slide picked in the grid |
| `+` / `-` | zoom the slide, or the grid |
| arrows, zoomed in | pan the slide |
| `0` | fit the slide back to the screen |
| `t` | switch between images and text |
| `r` | reload the file now, as after fixing what made a reload fail |
| `ctrl-o` | pick another file to present, from the list above |
| `s` | read the slides aloud from this one on; again to stop |
| `q` | quit |

## Terminals

Slides are shown as images in terminals with a graphics protocol, such as
kitty, Ghostty and WezTerm. In others, and with `--text`, each slide is
shown as its text over a coarse image of it.

Over ssh, images are sent compressed: slides are flat colour, and the link,
not the CPU, limits how fast a slide appears.

## Markdown and R Markdown

`.md` and `.Rmd` files are typeset into a PDF in memory, with the fonts
typst carries, so they look the same everywhere.

A file whose front matter names a slide format (`beamer_presentation`,
`ioslides_presentation`, `revealjs`, xaringan, …) is set as 16:9 slides,
broken as pandoc breaks them: at `---` rules and at headings of the slide
level, with higher headings as section slides. With `marp: true` slides
break at rules only. Any other file is a document, set on US letter pages
and broken where a page fills or at `\newpage`. `--paper` takes any paper
size typst knows: `a4`, `a5`, `presentation-4-3`, ….

Hugo's and Zola's TOML front matter, between `+++` lines, is not shown; its
`title` and `date` head the document.

Code chunks, ```` ```{r} ````, ```` ```{python} ````, ```` ```{bash} ````
or any other knitr engine, are run by knitr where R is installed, in the
file's directory, so their output and plots are shown; a plain
```` ```python ```` block only shows code. Without R, with `--no-knit`, or
for a file on another machine, chunks show their code. Knitting runs again
on every save.

Formulas are set by typst from their LaTeX. One using what the conversion
does not know, such as a macro of your own, is rendered by `pdflatex` where
it is installed, with the macros from `header-includes` and from
`\newcommand` lines in the file; otherwise it is shown as its source.

Citations are set as pandoc's citeproc sets them, by typst, where front
matter names a bibliography:

```yaml
bibliography: [refs.bib, refs.json]   # BibLaTeX, BibTeX, CSL JSON, CSL YAML
references: [...]                      # or CSL items here
csl: apa.csl                           # a CSL file, or a style typst knows
nocite: '@*'
reference-section-title: References
suppress-bibliography: false
lang: en-GB
```

`[see @doe99, p. 33; @roe]`, `@doe99 [p. 33]` in the text and `[-@doe99]`
for the year alone are read as pandoc reads them, in Chicago author-date
unless `csl:` names another style. A style that is not a file here, such as
`https://www.zotero.org/styles/apa`, is used if typst knows it by name. The
works cited are listed at the end, on a slide of their own in slides, or in
a `::: {#refs}` div. A key not in the bibliography is shown as **key?**, as
pandoc shows it, and named on the status line. One difference: typst's
citations have no place for a prefix, so the `see` of `[see @doe99]` is
written before the parentheses rather than in them. Only the markdown file
is watched, so after changing the bibliography, save the markdown again.

## Reading aloud

`s` reads the current slide aloud, then turns to the next and reads that,
to the end of the deck or until `s` again; turning to another slide while
reading reads that one. Each slide's text is taken from the PDF as `--text`
takes it, a sentence at a time, leaving out bullets and list numbers,
footlines and page numbers, and the sentence being read is lit on the
slide, in yellow. Formulas
and tables are not read: a pause takes the place of each, half a second
unless `--pause` gives another, as `--pause 1`.

Two shell commands do the reading. The voice, `--voice` or `OHP_VOICE`,
turns the text of a sentence on its input into WAV audio on its output; the
player, `--play` or `OHP_PLAY`, plays the WAV on its input. Without them,
the voice is [Piper](https://github.com/rhasspy/piper) if it is installed
with a voice in `~/.local/share/piper` (or `$XDG_DATA_HOME/piper`); else,
on macOS, `say`, in the voice chosen under System Settings > Accessibility >
Spoken Content; else `espeak-ng --stdout` or `espeak --stdout`. The player
is the first of `aplay`, `paplay`, `afplay` and sox's `play` found. Piper
runs on the CPU with voices from
[rhasspy/piper-voices](https://huggingface.co/rhasspy/piper-voices) on
Hugging Face, and sounds far better than espeak. To set it up on Linux, so
that `s` uses it with nothing more to set:

```sh
# Piper's C++ release: one program, with ONNX Runtime and espeak-ng beside it.
mkdir -p ~/.local/opt ~/.local/bin && cd ~/.local/opt
curl -L https://github.com/rhasspy/piper/releases/download/2023.11.14-2/piper_linux_x86_64.tar.gz | tar xz
ln -sf ~/.local/opt/piper/piper ~/.local/bin/piper

mkdir -p ~/.local/share/piper && cd ~/.local/share/piper
for f in en_US-lessac-medium.onnx en_US-lessac-medium.onnx.json; do
  curl -LO https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/$f
done
```

With several voices there, the first by name is used; to choose one, give
it, as `OHP_VOICE='piper -m ~/.local/share/piper/en_GB-alan-medium.onnx -f -'`.
On ARM Linux, take `piper_linux_aarch64.tar.gz` in its place. Piper's
`-f -` matters: without it, the WAV goes to a file rather than to ohp.
Piper's macOS releases do not run on today's Macs; there, download a
Premium voice for `say`, under Spoken Content > System voice > Manage
Voices, which sounds nearly as good. The sound comes out of the machine
ohp runs on: for slides on another machine, run ohp here with `host:path`,
not over ssh there.

A formula is known by its math font, as typst's and unicode-math's are, or
by its mathematical characters; pdfLaTeX's math fonts give no names, so
there its plain letters are still read. A table is known by rules above and
below its rows and its columns. Columns of prose side by side are read in
the order their lines run across the page. A sentence ends at a full stop,
question or exclamation mark, but not at an initial or an abbreviation such
as `e.g.` or `Fig.`. The voice works a sentence ahead of the player, so a
voice slow to start, as Piper's, mostly delays just the first sentence
of a slide.

## Slides on another machine

`ohp [user@]host:path` names the slides as scp does. The file is copied over
the system `ssh` into a temporary file and kept in step with the original,
so running LaTeX, or saving the markdown, there updates the slides here.
Markdown from another machine is not knitted, and images it links to by a
relative path are not shown. One ssh connection watches
the file and every copy goes through it, so a password is asked for once,
before the slides take over the terminal.

If the connection is lost, the status line says so and ohp reconnects by
itself. It never asks for a password once the slides are up, so this needs
a key or an ssh agent.

## License

MIT; see [LICENSE](LICENSE).
