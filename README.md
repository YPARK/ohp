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
cargo install --git https://github.com/YPARK/ohp --no-default-features
```

## Use

```sh
ohp talk.pdf
ohp --text talk.pdf                  # slides as text, even where images can be shown
ohp user@host:~/talks/talk.pdf       # slides on another machine
ohp talk.Rmd                         # R Markdown slides, chunks run by knitr
ohp --paper a4 notes.md              # a markdown document, on A4 pages
```

| Key | |
|---|---|
| `n` / `p`, arrows | next / previous slide; up and down move a row in the grid |
| `g`, `tab` | switch between the slide and a grid of all slides |
| `enter` | present the slide picked in the grid |
| `+` / `-` | zoom the slide, or the grid |
| arrows, zoomed in | pan the slide |
| `0` | fit the slide back to the screen |
| `t` | switch between images and text |
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
