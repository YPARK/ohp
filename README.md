# ohp

An overhead projector: present beamer PDF slides, or markdown, R
Markdown and HTML, in the terminal.

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
stack-switching file with the system C toolchain, and summaries' tokenizer
builds Oniguruma's C with it; without either, nothing but Rust is compiled:

```sh
cargo install --git https://github.com/YPARK/ohp --no-default-features --features speech
```

Summing slides up, the `summary` feature, is in the default build too: a
language model runs in ohp itself, with
[candle](https://github.com/huggingface/candle), on the GPU through Metal
on macOS, as [Summaries](#summaries) shows.

Reading slides aloud, the `speech` feature, is in the default build, and
reads with speech programs it runs, none built in: on macOS, its own `say`
and `afplay`, with nothing to install, and natural voices a key away, as
[On macOS](#on-macos) shows; on Linux, `espeak-ng` and ALSA's `aplay`, as
`sudo apt install espeak-ng alsa-utils`, or, far better, Piper's voices,
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
ohp notes.html                       # an HTML document
ohp plot.png                         # an image
ohp intro.pdf figs/*.png photo.jpg   # several files, mixed, one after another
ohp host:talk.pdf host:fig.png       # several on another machine
```

In the list, typing narrows it to the names that match, and a path typed
up to a `/`, as `../talks/` or `~/`, goes to that directory. Up and down
move, enter opens a file or goes into a folder, left or backspace on an
empty line goes up, and esc quits. Tab marks a file, or unmarks it, in
any folder, and shift-tab unmarks them all; enter opens the files marked
and the one picked together. While presenting, ctrl-o brings the same
list up over the slides, in the directory of the file on screen, with a
deck of several files marked, so enter on another adds it, and that
file is shown first.

Several files given, or marked, are shown as one deck, in the order
given: each page of a PDF as a slide, each image, PNG, JPEG, GIF, WebP or
SVG, as a slide its own shape, and markdown or HTML typeset as it is
alone. The status line names the file the slide is from. A photo larger than a
screen shows, past 4096 pixels across or down, is made that small first.
A file that cannot be read is left out, and named in the status line. A
change to any of them reloads the deck, reading again only the files
changed, and `r` all of them. Images, and several files together, are
set by typst, so need the `markdown` feature, as in the default build.

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
| click | read aloud from the sentence clicked; in the grid, pick the slide clicked, and present it clicked again or double-clicked |
| `v` | pick the voice to read in, or one of Piper's to download |
| `k` | sum up the slide in a column beside it, and mark its key sentences; again to close |
| `K` | sum up the whole file there, and mark its key sentences wherever they are; again to close |
| `m` | pick the model that sums up, or one of Hugging Face's to download |
| `q` | quit |

ohp takes the mouse, so selecting text in the terminal needs Shift held,
or Option in macOS terminals; the wheel turns slides as the arrows do.

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

## HTML

`.html` and `.htm` files are typeset as markdown is. What is set is the
first `<article>`, or else `<main>`, or else the body. Headings, lists,
tables, code, links and images are set; an image whose data is in the
file is shown too. Scripts, styles, navigation, footers and a table of
contents are left out.

A formula in `\(...\)` or `\[...\]`, in a MathML annotation, or in the
TeX KaTeX or MathJax keep beside their drawing, is set as one written
in markdown. A picture drawn in the page is shown, and so is a mermaid
flowchart drawn from its source. A reveal.js deck, the `<section>` slides in
`.slides`, is set as slides, and so is remark's or xaringan's markdown,
kept in `<textarea id="source">` and typeset as written.
Anything else is a document, on the same paper as markdown, and `--paper`
sets it.

## Reading aloud

`s` reads the current slide aloud, then turns to the next and reads that,
to the end of the deck or until `s` again; turning to another slide while
reading reads that one. Clicking a sentence on the slide, or near one,
reads from there on the same way, whether or not the slide was being read.
Each slide's text is taken from the PDF as `--text`
takes it, a sentence at a time, leaving out bullets and list numbers,
footlines and page numbers, and the sentence being read is lit on the
slide, in yellow. Formulas
and tables are not read: a pause takes the place of each, half a second
unless `--pause` gives another, as `--pause 1`.

### On macOS

Nothing needs installing: `s` reads at once, in macOS's own voice. For a
better one, with no Python, Homebrew or anything else to set up:

1. Open slides, as `ohp talk.pdf`, and press `v`. The list shows macOS's
   natural voices, as `Daniel` or `Samantha`, and `sherpa-onnx`.
2. Pick `sherpa-onnx`, Enter: it downloads, 20 MB, in seconds, to
   `~/.local/share/piper/sherpa-onnx`, `installing sherpa-onnx…` in the
   status line meanwhile.
3. The list opens again, now with Piper's natural voices, yours in your
   language first. Type to narrow it, as `en_GB` or `alan`, and Enter: the
   voice downloads, 20 to 140 MB, its progress in the status line, and
   reading goes on in it. `en_US-lessac-medium` and `en_GB-alan-medium` are
   good to start with; `high` voices are fuller, `medium` ones quicker.
4. Press `s`, or click a sentence on the slide to read from there.

The voices downloaded stay in `~/.local/share/piper`: the next time, the
first of them by name is the voice without asking, and `v` switches among
them, or back to a macOS voice. A macOS voice sounds better once its
Premium or Enhanced version is downloaded, under System Settings >
Accessibility > Spoken Content > System voice > Manage Voices; choosing it
there makes it the voice ohp reads in when no Piper voice is downloaded.
To remove everything, `rm -rf ~/.local/share/piper`.

Two shell commands do the reading. The voice, `--voice` or `OHP_VOICE`,
turns the text of a sentence on its input into WAV audio on its output; the
player, `--play` or `OHP_PLAY`, plays the WAV on its input. Without them,
the voice is one of [Piper](https://github.com/rhasspy/piper)'s in
`~/.local/share/piper` (or `$XDG_DATA_HOME/piper`), if there is one, with
Piper or sherpa-onnx installed to speak it; else,
on macOS, `say`, in the voice chosen under System Settings > Accessibility >
Spoken Content; else `espeak-ng --stdout` or `espeak --stdout`. The player
is the first of `aplay`, `paplay`, `afplay` and sox's `play` found. Piper's
voices run on the CPU, and sound far better than espeak. `v`, below, sets
them up with no more than `curl`; or, to set Piper up by hand on Linux,
with a voice from
[rhasspy/piper-voices](https://huggingface.co/rhasspy/piper-voices) on
Hugging Face, so
that `s` uses it with nothing more to set (on ARM Linux, with
`piper_linux_aarch64.tar.gz`; Piper's `-f -` matters: without it, the WAV
goes to a file rather than to ohp):

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
it, as `OHP_VOICE='piper -m ~/.local/share/piper/en_GB-alan-medium.onnx -f -'`,
or press `v`. It lists the voices at hand, on macOS `say`'s, then, with
`curl` and Piper or sherpa-onnx installed, the voices in Piper's catalog,
your language's first: type to narrow the list, and Enter to read in the
one picked. Only voices that sound natural are listed: not macOS's
robotic ones and sound effects, as `Fred`, `Zarvox` or `Grandpa`, nor
Piper's of `low` quality, nor a few others of its that sound flat. Of the
many in English, only the best are listed: Piper's `lessac`, `ryan`,
`amy`, `alan`, `cori` and `jenny_dioco`, and macOS's `Samantha`, `Daniel`
and any downloaded as Premium or Enhanced. The catalog is
[sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx)'s copies of Piper's
voices, which Piper speaks too. A voice not yet there is downloaded to a
directory of its own in `~/.local/share/piper` first, 20 to 140 MB, its
progress in the status line, and is read in once it is; it stays there for
next time. The voice picked is used until ohp quits.

Without Piper, `v` lists sherpa-onnx itself: picking it downloads its
release for the machine, 20 MB, a program and the libraries it needs, to
`~/.local/share/piper/sherpa-onnx`, in seconds, with no Python or anything
else to install; the list then opens again on Piper's voices to download.
sherpa-onnx speaks them as Piper does, as quickly. This is the way on
macOS, where Piper's own releases do not run on today's Macs; on ARM
Linux, sherpa-onnx has no such release, and Piper is set up by hand.
Better voices for `say` can be downloaded too, under Spoken Content >
System voice > Manage Voices. The sound comes out of the machine
ohp runs on: for slides on another machine, run ohp here with `host:path`,
not over ssh there.

A formula is known by its math font, as typst's and unicode-math's are, or
by its mathematical characters; pdfLaTeX's math fonts give no names, so
there its plain letters are still read. A table is known by rules above and
below its rows and its columns. Columns of prose side by side, as beamer's
`columns` or a two-column paper's, are read one after the other, each from
its top to its foot; columns closer than an em apart are not told apart,
and are read a line across the page at a time. A sentence ends at a full stop,
question or exclamation mark, but not at an initial or an abbreviation such
as `e.g.` or `Fig.`. The voice works a sentence ahead of the player, so a
voice slow to start, as Piper's, mostly delays just the first sentence
of a slide.

## Summaries

`k` sums up the slide or page on screen, in a column beside it, and marks
on it, in green, the one to three sentences that carry its key takeaways,
listed under the summary too. The column follows the slides as they turn,
summing up each one turned to; while one is read, the two after it are
summed up too, so turning to them shows theirs at once. `K` sums up the
whole file there instead,
and marks its three to five key sentences on whatever pages they are.
Either key again, or the other, closes the column or switches it.

The summary is written by a language model run in ohp, offline, which
[candle](https://github.com/huggingface/candle) runs on the GPU through
Metal on macOS and on the CPU elsewhere: a GGUF file of its weights,
quantised to 4 bits, from [Hugging Face](https://huggingface.co)'s Hub.
The first `k` downloads it, once, with `curl`, into Hugging Face's own
cache, `~/.cache/huggingface/hub` (or `$HF_HUB_CACHE`, or `hub` in
`$HF_HOME`, or in `$XDG_CACHE_HOME/huggingface`), as its libraries lay it out, so a model one of them
downloaded is used without downloading it again, and one ohp downloaded is
theirs too. A download cut short, or given up as another model is picked
or the column closed, is taken up where it stopped. While it downloads,
reads the page and writes, a bar in the column shows how far it has come,
and the status line its percent; while it loads, the column says so.

`m` picks the model, in a list over the slides: those below, then any GGUF
file in the cache. Type to narrow the list, or type a repo of the Hub's,
as `bartowski/Qwen2.5-7B-Instruct-GGUF`, and Enter to sum up with it; its
4-bit file, `Q4_K_M`, is taken, unless the file is named too, as
`owner/repo/model-Q5_K_M.gguf`. `--model`, or `OHP_MODEL`, gives the model
to start with in the same ways, or as a `.gguf` file here. A gated repo
needs its terms accepted on the Hub, and a token, as `HF_TOKEN` gives it
or `hf auth login` keeps it.

| `m`, `--model` | | |
|---|---|---|
| `qwen2.5-3b` | 2.1 GB | default on macOS: a slide in a few seconds on an M4, a dense page of a paper in under ten |
| `qwen2.5-1.5b` | 1.1 GB | default elsewhere: twice as quick, its picks less apt |
| `qwen3-4b` | 2.5 GB | Qwen3-4B-Instruct-2507: newer, and a little slower |
| `llama3.2-3b` | 2.0 GB | Meta's Llama 3.2 3B Instruct |
| `qwen2.5-7b` | 4.7 GB | the best of these, and the slowest |
| `owner/repo` | | any GGUF of a Qwen2, Qwen3 or Llama 3 model on the Hub |

A page is given to the model as the sentences `s` reads aloud from it,
numbered, up to about two thousand tokens' worth, and the model answers
with its summary and the numbers of its key sentences, so what is marked
is what the page says, word for word. A paper is too long to give the
model whole, and would take minutes to read a page at a time; so for `K`,
the sentences most central to the file are retrieved first: each is
embedded by a small model of what sentences mean,
[all-MiniLM-L6-v2](https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2),
90 MB, downloaded from the Hub as the language model is, and scored by how
near it is to the whole file, more on its first page and where it says
what was found, as `we show`, and less where it points to a figure,
leaving out references, running heads and sentences that say what one
picked already does. Where that model cannot be had, as offline, the
sentences' words are weighed instead, by TF-IDF. About two thousand tokens
of them, in order and with their pages, are summed up as a page is: in
about twenty seconds on an M4, however long the file.

Summaries are kept, by the model and the text summed up, in
`~/.cache/ohp/summaries` (or `$XDG_CACHE_HOME/ohp/summaries`), so a page,
or a file, summed up before, as one a reload leaves as it was, or a paper
opened again, is shown at once, with no model loaded. To forget them,
`rm -rf ~/.cache/ohp/summaries`.

## Slides on another machine

`ohp [user@]host:path` names the slides as scp does. The file is copied over
the system `ssh` into a temporary file and kept in step with the original,
so running LaTeX, or saving the markdown, there updates the slides here.
Markdown from another machine is not knitted, and images it links to by a
relative path are not shown. Several files may be on other machines, and
mixed with files here, as `ohp host:talk.pdf host:figs/a.png notes.md`.
One ssh connection a machine watches its files and every copy goes
through it, so a password is asked for once a machine, before the slides
take over the terminal.

If the connection is lost, the status line says so and ohp reconnects by
itself. It never asks for a password once the slides are up, so this needs
a key or an ssh agent.

## As a library

ohp is a library too: `ohp::page::pages` draws the pages of a PDF, a
markdown, R Markdown or HTML file, or an image, as ohp typesets them,
each as wide as asked, as letterbox shows a message:

```rust
let pages = ohp::page::pages("note.html".as_ref(), 800)?;
```

## License

MIT; see [LICENSE](LICENSE).
