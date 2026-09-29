# ohp

An overhead projector: present beamer PDF slides in the terminal.

Slides are rasterised in pure Rust by [hayro](https://crates.io/crates/hayro)
and shown through [ratatui-image](https://crates.io/crates/ratatui-image),
over the kitty graphics protocol where the terminal has it, so they can be
presented from a remote machine over ssh. Elsewhere they are shown as their
text, over a coarse image of the slide.

The PDF is reloaded whenever it changes, so recompiling the slides updates
them in place.

## Install

Rust 1.92 or later:

```sh
cargo install --git https://github.com/YPARK/ohp
```

Nothing but Rust is compiled: no C libraries, no poppler.

## Use

```sh
ohp talk.pdf
ohp --text talk.pdf                  # slides as text, even where images can be shown
ohp user@host:~/talks/talk.pdf       # slides on another machine
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

## Slides on another machine

`ohp [user@]host:path` names the slides as scp does. The PDF is copied over
the system `ssh` into a temporary file and kept in step with the original,
so running LaTeX there updates the slides here. One ssh connection watches
the file and every copy goes through it, so a password is asked for once,
before the slides take over the terminal.

If the connection is lost, the status line says so and ohp reconnects by
itself. It never asks for a password once the slides are up, so this needs
a key or an ssh agent.
