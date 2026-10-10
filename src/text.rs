//! Slides as text, for terminals that cannot show images well.
//!
//! The interpreter reports every glyph it draws; each is kept with where it
//! sits, its size and its colour. Glyphs on one baseline form a line, and a
//! gap between glyphs starts a new word: TeX places words apart rather than
//! writing space characters. Lines are then set on the cell grid at the
//! height they have on the page, their words one space apart unless the page
//! puts them far apart, as in columns and tables.
//!
//! The text is drawn over a coarse image of the slide, for its figures and
//! colours. Each glyph set on the grid is painted out of that image first:
//! the text is set where it reads well, not exactly where the page has it,
//! and would otherwise show twice. Glyphs that are not set, such as rotated
//! labels or lines past the bottom of the grid, stay in the image.
//!
//! Read aloud, the text leaves out formulas and tables, a pause in their
//! place. A glyph is a formula's when its font is a math font or its
//! character is a mathematical one: pdfTeX's Type1 fonts give no names, so
//! there a formula's plain letters are read. A line is a table's when rules
//! run above and below it and its words fall in columns, apart across a rule
//! or a wide gap.

use hayro::hayro_interpret::font::Glyph;
use hayro::hayro_interpret::hayro_cmap::BfString;
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache,
    InterpreterSettings, Paint, PathDrawMode, SoftMask, TransformExt, interpret_page,
};
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::kurbo::{Affine, BezPath, Point, Rect, Shape};
use image::{Rgba, RgbaImage};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect as Area;
use ratatui::style::{Color, Modifier, Style};
use std::collections::HashMap;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// A gap wider than this, in ems, from where the pen ends a glyph to where
/// it starts the next, starts a new word. A space a justified line squeezes
/// is about twice it.
const WORD_GAP: f32 = 0.1;
/// Where a glyph's advance is not known, the room assumed between its
/// outline and the pen either side, in ems.
const BEARING: f64 = 0.05;
/// A gap wider than this, in ems, is kept on screen, as between columns.
const WIDE_GAP: f32 = 1.5;
/// Baselines closer than this, in ems, are one line.
const SAME_LINE: f32 = 0.35;
/// Lines closer than this, in ems, are one paragraph: they go on consecutive
/// rows, where the page's proportions would scatter them.
const TIGHT: f32 = 1.6;
/// Text this much larger than the body is drawn bold.
const LARGE: f32 = 1.15;
/// Text this much smaller than the body, as a footline or a page number, is
/// not read aloud.
const SMALL: f32 = 0.8;
/// Words this far apart, in ems, are in columns of a table, where rules run
/// above and below them; prose is never spaced so wide.
const TABLE_GAP: f32 = 1.;
/// Thickest a drawn line is to be a rule, as a table's, in points.
const RULE: f64 = 3.;
/// How far a line being read is lit above and below its baseline, in ems.
const LIT_ASCENT: f64 = 0.8;
const LIT_DESCENT: f64 = 0.25;
/// The colour a sentence being read is lit with, behind its text.
pub const LIGHT: [u8; 3] = [255, 226, 110];
/// Least difference in luma between text and what is behind it. Text the
/// page itself draws fainter than this, as beamer does covered items, is
/// left faint.
const CONTRAST: f32 = 0.4;
/// Ascent, width and descent assumed for a Type3 glyph, in ems: its outline
/// is a content stream, not a shape with a known box.
const TYPE3: (f64, f64, f64) = (0.8, 0.5, 0.2);

/// The text of one page, in page points with y growing down.
#[derive(Debug, Default)]
pub struct Page {
    pub width: f32,
    pub height: f32,
    glyphs: Vec<Mark>,
    /// Rules the page draws, as a table's: across it and down it.
    across: Vec<Rect>,
    down: Vec<Rect>,
    /// Which glyphs are being read aloud; none if empty.
    lit: Vec<bool>,
}

/// A sentence read aloud, and the glyphs it is read from.
struct Sentence {
    text: String,
    glyphs: Vec<usize>,
}

/// What reading a page aloud says: a sentence, or a pause where a formula
/// or a table is left out.
#[derive(Debug, PartialEq)]
pub enum Spoken {
    Text(String),
    Pause,
}

/// One drawn glyph.
#[derive(Debug)]
struct Mark {
    x0: f32,
    x1: f32,
    /// Where the pen starts the glyph and ends it, past its advance: the
    /// outline of an f or a j reaches into the space beside it.
    start: f32,
    end: f32,
    /// Top and bottom of the glyph's outline.
    y0: f32,
    y1: f32,
    /// Baseline.
    y: f32,
    /// Em size.
    size: f32,
    text: String,
    rgb: [u8; 3],
    /// The colour around the glyph on the rendered page.
    back: [u8; 3],
    /// It is a formula's.
    math: bool,
    /// Its font is a bold one.
    heavy: bool,
}

/// A run of glyphs with no word gap between them.
struct Word {
    x0: f32,
    x1: f32,
    /// Where the pen ends its last glyph.
    end: f32,
    size: f32,
    text: String,
    /// Some of it is a formula.
    math: bool,
    /// All of it is set in bold.
    heavy: bool,
    /// Its glyphs, as indices into the page's.
    glyphs: Vec<usize>,
}

/// A character set on the grid, in the colour the page gives it.
#[derive(Clone, Copy, Debug)]
pub struct Cell {
    pub ch: char,
    pub rgb: [u8; 3],
    pub bold: bool,
    /// The page draws it faint against its background.
    pub faint: bool,
    /// It is being read aloud.
    pub lit: bool,
}

/// Rows of cells, `None` where nothing is written. A wide character's
/// second cell is `None`.
pub type Cells = Vec<Vec<Option<Cell>>>;

pub fn extract<'a>(
    pdf: &'a Pdf,
    page: usize,
    cache: &InterpreterCache<'a>,
    settings: &InterpreterSettings,
) -> Option<Page> {
    let page = pdf.pages().get(page)?;
    let (width, height) = page.render_dimensions();
    let mut ctx = Context::new(
        page.initial_transform(true).to_kurbo(),
        Rect::new(0., 0., width.into(), height.into()),
        cache,
        page.xref(),
        settings.clone(),
    );
    let mut marks = Marks::default();
    interpret_page(page, &mut ctx, &mut marks);
    // Text off the page is clipped away when rendered; beamer keeps some there.
    let on_page = |g: &Mark| g.x1 >= 0. && g.x0 <= width && g.y >= 0. && g.y <= height + g.size;
    marks.glyphs.retain(on_page);
    Some(Page {
        width,
        height,
        glyphs: marks.glyphs,
        across: marks.across,
        down: marks.down,
        lit: Vec::new(),
    })
}

impl Page {
    /// Set the page on a `cols` × `rows` grid over `image`, a render of it,
    /// painting each glyph set out of the image.
    pub fn set_over(&mut self, image: &mut RgbaImage, cols: u16, rows: u16) -> Cells {
        self.sample(image);
        let (cells, set) = self.layout(cols, rows);
        self.erase(image, &set);
        cells
    }

    /// The page set on a `cols` × `rows` grid, and which glyphs were set:
    /// lines past the bottom and words past the edge are not.
    fn layout(&self, cols: u16, rows: u16) -> (Cells, Vec<bool>) {
        let (cols, rows) = (usize::from(cols), usize::from(rows));
        let mut grid: Cells = vec![vec![None; cols]; rows];
        let mut set = vec![false; self.glyphs.len()];
        if cols == 0 || rows == 0 || self.width <= 0. || self.height <= 0. {
            return (grid, set);
        }
        let body = self.body_size();
        let col_of =
            |x: f32| ((x / self.width * cols as f32).round().max(0.) as usize).min(cols - 1);
        // The last line placed: its baseline, size and last row. Each line
        // goes below it, so rows already written are never written again.
        let mut prev: Option<(f32, f32, usize)> = None;
        for (y, words) in self.text_lines() {
            let size = words.iter().map(|w| w.size).fold(0., f32::max);
            let want = (y / self.height * rows as f32) as usize;
            let row = match prev {
                Some((py, psize, prow)) if y - py <= TIGHT * size.max(psize) => prow + 1,
                Some((_, _, prow)) => want.max(prow + 1),
                None => want,
            };
            let spots = place(&words, &col_of, cols);
            let last = row + spots.last().map_or(0, |&(down, _)| down);
            if last >= rows {
                break;
            }
            for (word, &(down, col)) in words.iter().zip(&spots) {
                let first = &self.glyphs[word.glyphs[0]];
                let lit = word.glyphs.iter().any(|&g| self.lit.get(g) == Some(&true));
                let cell = |ch| Cell {
                    ch,
                    rgb: first.rgb,
                    bold: word.heavy || word.size > LARGE * body,
                    faint: faint(first.rgb, first.back),
                    lit,
                };
                let mut at = col;
                for ch in word.text.chars() {
                    let w = ch.width().unwrap_or(0);
                    if w == 0 {
                        continue;
                    }
                    if at + w > cols {
                        break;
                    }
                    grid[row + down][at] = Some(cell(ch));
                    at += w;
                }
                if col + word.text.width() <= cols {
                    for &g in &word.glyphs {
                        set[g] = true;
                    }
                }
            }
            prev = Some((y, size, last));
        }
        (grid, set)
    }

    /// The page's text to be read aloud, a sentence at a time, and a pause
    /// where a formula or a table is left out.
    pub fn spoken(&self) -> Vec<Spoken> {
        self.sentences()
            .into_iter()
            .map(|s| s.map_or(Spoken::Pause, |s| Spoken::Text(s.text)))
            .collect()
    }

    /// Mark sentence `k` of what is read aloud as being read, so `set_over`
    /// lights its cells, and give boxes about it, one a line, in page points.
    pub fn light(&mut self, k: usize) -> Vec<Rect> {
        let lines = self.text_lines();
        let sentence = self.sentences_of(&lines).into_iter().flatten().nth(k);
        self.lit = vec![false; self.glyphs.len()];
        for g in sentence.map(|s| s.glyphs).unwrap_or_default() {
            self.lit[g] = true;
        }
        lines
            .iter()
            .filter_map(|(_, words)| {
                words
                    .iter()
                    .flat_map(|w| &w.glyphs)
                    .filter(|&&i| self.lit[i])
                    .map(|&i| {
                        let g = &self.glyphs[i];
                        let (y, size) = (f64::from(g.y), f64::from(g.size));
                        Rect::new(
                            f64::from(g.start),
                            y - LIT_ASCENT * size,
                            f64::from(g.end),
                            y + LIT_DESCENT * size,
                        )
                    })
                    .reduce(|a, b| a.union(b))
            })
            .collect()
    }

    /// Mark nothing as being read.
    pub fn unlight(&mut self) {
        self.lit.clear();
    }

    /// What is read aloud: sentences, and `None` where a pause goes. A
    /// title, an item or a paragraph ends a sentence too. Words of no
    /// letters nor digits, as bullets, are not read, nor a list's numbers,
    /// and an item one starts is a paragraph of its own. Small text, as a
    /// footline, and lines of no letters, as a page number, are left out.
    fn sentences(&self) -> Vec<Option<Sentence>> {
        self.sentences_of(&self.text_lines())
    }

    /// `sentences` of the page's `lines`, as `text_lines` gives them.
    fn sentences_of(&self, lines: &[(f32, Vec<Word>)]) -> Vec<Option<Sentence>> {
        let body = self.body_size();
        let mut said: Vec<Option<Sentence>> = Vec::new();
        let pause = |said: &mut Vec<Option<Sentence>>| {
            if said.last().is_none_or(Option::is_some) {
                said.push(None);
            }
        };
        // Whether the last sentence goes on: it has not ended, nor its
        // paragraph.
        let mut open = false;
        // The last line read: its baseline, size and whether it is bold.
        let mut prev: Option<(f32, f32, bool)> = None;
        for &(y, ref words) in lines {
            let size = words.iter().map(|w| w.size).fold(0., f32::max);
            if size < SMALL * body {
                continue;
            }
            if self.tabular(y, words) {
                pause(&mut said);
                continue;
            }
            let worded = |w: &Word| !w.math && w.text.chars().any(char::is_alphanumeric);
            let lettered = words
                .iter()
                .any(|w| !w.math && w.text.chars().any(char::is_alphabetic));
            // A bullet, or a list's number, starts an item, and is not read.
            let item = words
                .first()
                .is_some_and(|w| !w.math && (!worded(w) || label(&w.text)));
            let read = |i: usize, w: &Word| worded(w) && !(item && i == 0);
            let heavy = (words.iter().enumerate())
                .filter(|&(i, w)| read(i, w))
                .all(|(_, w)| w.heavy);
            let apart = prev.is_none_or(|(py, psize, pheavy)| {
                item || y - py > TIGHT * size.max(psize)
                    || size > LARGE * psize
                    || psize > LARGE * size
                    || heavy != pheavy
            });
            if apart && lettered {
                open = false;
            }
            let mut first = true;
            for (i, w) in words.iter().enumerate() {
                if w.math {
                    pause(&mut said);
                    continue;
                }
                if !lettered || !read(i, w) {
                    continue;
                }
                match said.last_mut() {
                    Some(Some(s)) if open => {
                        // A word hyphenated across the break is joined again
                        // with its hyphen: it may be a compound's.
                        if !(first && s.text.ends_with('-')) {
                            s.text.push(' ');
                        }
                        s.text.push_str(&w.text);
                        s.glyphs.extend(&w.glyphs);
                    }
                    _ => said.push(Some(Sentence {
                        text: w.text.clone(),
                        glyphs: w.glyphs.clone(),
                    })),
                }
                open = !ends_sentence(&w.text);
                first = false;
            }
            if !first {
                prev = Some((y, size, heavy));
            }
        }
        said
    }

    /// Whether a line is a table's: rules run above and below its words,
    /// and they fall in columns, apart across a rule or a wide gap.
    fn tabular(&self, y: f32, words: &[Word]) -> bool {
        let y = f64::from(y);
        let ruled = words.iter().any(|w| {
            let x = f64::from(w.x0 + w.x1) / 2.;
            let top = y - f64::from(w.size) / 2.;
            let over = |r: &&Rect| r.x0 <= x && x <= r.x1;
            let mut rules = self.across.iter().filter(over);
            rules.clone().any(|r| r.y1 <= top) && rules.any(|r| r.y0 >= y)
        });
        ruled
            && words.windows(2).any(|pair| {
                let [a, b] = pair else { return false };
                let top = y - f64::from(a.size.max(b.size)) / 2.;
                let (left, right) = (f64::from(a.x1), f64::from(b.x0));
                b.x0 - a.x1 > TABLE_GAP * a.size.max(b.size)
                    || self.down.iter().any(|r| {
                        let x = (r.x0 + r.x1) / 2.;
                        r.y0 <= top && r.y1 >= y && left <= x && x <= right
                    })
            })
    }

    /// Note the colour around each glyph in `image`, a render of this page.
    fn sample(&mut self, image: &RgbaImage) {
        if image.width() == 0 || image.height() == 0 {
            return;
        }
        for i in 0..self.glyphs.len() {
            let (x0, y0, x1, y1) = self.pixels(&self.glyphs[i], image);
            // The most common colour on the box's edge: letters beside it
            // touch the edge, but the flat background fills most of it.
            let rows = (x0..=x1).flat_map(|x| [(x, y0), (x, y1)]);
            let cols = (y0..=y1).flat_map(|y| [(x0, y), (x1, y)]);
            let mut edge: Vec<[u8; 3]> = rows
                .chain(cols)
                .map(|(x, y)| {
                    let [r, g, b, _] = image.get_pixel(x, y).0;
                    [r, g, b]
                })
                .collect();
            edge.sort_unstable();
            if let Some(run) = edge.chunk_by(|a, b| a == b).max_by_key(|run| run.len()) {
                self.glyphs[i].back = run[0];
            }
        }
    }

    /// Paint the glyphs marked in `set` out of `image` with their colour around.
    fn erase(&self, image: &mut RgbaImage, set: &[bool]) {
        if image.width() == 0 || image.height() == 0 {
            return;
        }
        for (g, _) in self.glyphs.iter().zip(set).filter(|(_, set)| **set) {
            let (x0, y0, x1, y1) = self.pixels(g, image);
            let [r, gr, b] = g.back;
            for y in y0..=y1 {
                for x in x0..=x1 {
                    image.put_pixel(x, y, Rgba([r, gr, b, 255]));
                }
            }
        }
    }

    /// The corners of the pixels `g` touches in `image`, a render of this
    /// page, and a pixel more for its antialiased edge.
    fn pixels(&self, g: &Mark, image: &RgbaImage) -> (u32, u32, u32, u32) {
        let (w, h) = image.dimensions();
        let (sx, sy) = (w as f32 / self.width, h as f32 / self.height);
        let px = |v: f32, max: u32| (v.max(0.) as u32).min(max - 1);
        let (x0, x1) = ((g.x0 * sx).floor() - 1., (g.x1 * sx).ceil() + 1.);
        let (y0, y1) = ((g.y0 * sy).floor() - 1., (g.y1 * sy).ceil() + 1.);
        (px(x0, w), px(y0, h), px(x1, w), px(y1, h))
    }

    /// The most common glyph size: the body text's.
    fn body_size(&self) -> f32 {
        let mut sizes: Vec<i32> = self
            .glyphs
            .iter()
            .map(|g| (g.size * 2.).round() as i32)
            .collect();
        sizes.sort_unstable();
        sizes
            .chunk_by(|a, b| a == b)
            .max_by_key(|run| run.len())
            .map_or(0., |run| run[0] as f32 / 2.)
    }

    /// Lines top to bottom, each its baseline and its words left to right.
    fn text_lines(&self) -> Vec<(f32, Vec<Word>)> {
        let g = &self.glyphs;
        let mut order: Vec<usize> = (0..g.len()).collect();
        order.sort_by(|&a, &b| g[a].y.total_cmp(&g[b].y));
        let mut lines: Vec<Vec<usize>> = Vec::new();
        for i in order {
            match lines.last_mut() {
                Some(line)
                    if g[i].y - g[line[0]].y <= SAME_LINE * g[line[0]].size.max(g[i].size) =>
                {
                    line.push(i);
                }
                _ => lines.push(vec![i]),
            }
        }
        lines
            .into_iter()
            .map(|mut line| {
                line.sort_by(|&a, &b| g[a].x0.total_cmp(&g[b].x0));
                (g[line[0]].y, self.words(&line))
            })
            .collect()
    }

    fn words(&self, line: &[usize]) -> Vec<Word> {
        let mut words: Vec<Word> = Vec::new();
        for &i in line {
            let g = &self.glyphs[i];
            match words.last_mut() {
                Some(w) if g.start - w.end <= WORD_GAP * w.size.max(g.size) => {
                    w.text.push_str(&g.text);
                    w.x1 = w.x1.max(g.x1);
                    // A mark set over the letter before it ends short of it.
                    w.end = w.end.max(g.end);
                    w.math |= g.math;
                    w.heavy &= g.heavy;
                    w.glyphs.push(i);
                }
                _ => words.push(Word {
                    x0: g.x0,
                    x1: g.x1,
                    end: g.end,
                    size: g.size,
                    text: g.text.clone(),
                    math: g.math,
                    heavy: g.heavy,
                    glyphs: vec![i],
                }),
            }
        }
        words
    }
}

/// Where each word of a line goes, as rows down from the line's first and a
/// column: one space after the word before it, or at its place on the page
/// when the page leaves a wide gap. A word that would run past `cols` wraps
/// to the next row, under the line's first word.
fn place(words: &[Word], col_of: &impl Fn(f32) -> usize, cols: usize) -> Vec<(usize, usize)> {
    let Some((first, rest)) = words.split_first() else {
        return Vec::new();
    };
    let start = col_of(first.x0).min(cols.saturating_sub(first.text.width()));
    let mut spots = vec![(0, start)];
    let (mut down, mut end, mut x1) = (0, start + first.text.width(), first.x1);
    for w in rest {
        let len = w.text.width();
        let mut col = if w.x0 - x1 > WIDE_GAP * w.size {
            col_of(w.x0).max(end + 1)
        } else {
            end + 1
        };
        if col + len > cols && col > start {
            down += 1;
            col = start;
        }
        spots.push((down, col));
        (end, x1) = (col + len, w.x1);
    }
    spots
}

/// Whether `word` ends a sentence: it ends as one does, and is not an
/// abbreviation nor an initial.
fn ends_sentence(word: &str) -> bool {
    const ABBREVIATIONS: [&str; 12] = [
        "e.g.", "i.e.", "vs.", "cf.", "al.", "Fig.", "Eq.", "Dr.", "Mr.", "Ms.", "Mrs.", "No.",
    ];
    let word = word.trim_start_matches(['(', '[', '"', '\'', '“', '‘']);
    let end = word.trim_end_matches([')', ']', '"', '\'', '”', '’']);
    let initial = end.chars().count() == 2 && end.starts_with(char::is_uppercase);
    end.ends_with(['.', '!', '?']) && !initial && !ABBREVIATIONS.contains(&end)
}

/// Whether `word` numbers an item of a list, as `1.`, `b)`, `(iv)` do.
fn label(word: &str) -> bool {
    let word = word.strip_prefix('(').unwrap_or(word);
    let Some(n) = word.strip_suffix(['.', ')']) else {
        return false;
    };
    let all = |f: fn(char) -> bool| !n.is_empty() && n.chars().all(f);
    let roman = |c| matches!(c, 'i' | 'v' | 'x');
    (n.len() <= 3 && all(|c| c.is_ascii_digit()))
        || (n.len() == 1 && all(|c| c.is_ascii_lowercase()))
        || (n.len() <= 4 && all(roman))
}

/// Write the cells over what `area` of `buf` already shows, a coarse image
/// of the slide, each on the colour behind it.
pub fn overlay(cells: &Cells, area: Area, buf: &mut Buffer) {
    for (dy, row) in cells.iter().enumerate().take(area.height.into()) {
        for (dx, cell) in row.iter().enumerate().take(area.width.into()) {
            let Some(cell) = cell else { continue };
            let wide = cell.ch.width() == Some(2);
            // Half of it would show past the area.
            if wide && dx + 1 >= usize::from(area.width) {
                continue;
            }
            let (x, y) = (area.x + dx as u16, area.y + dy as u16);
            let Some(target) = buf.cell_mut((x, y)) else {
                continue;
            };
            // A block cell shows its top half in fg and bottom half in bg.
            let [top, bottom] = [target.fg, target.bg].map(|c| rgb_of(c).unwrap_or([255; 3]));
            let back = if cell.lit { LIGHT } else { mix(top, bottom) };
            let fg = if cell.faint && !cell.lit {
                cell.rgb
            } else {
                readable(cell.rgb, back)
            };
            let mut style = Style::new().fg(fg.into()).bg(back.into());
            if cell.bold {
                style = style.add_modifier(Modifier::BOLD);
            }
            target.set_char(cell.ch).set_style(style);
            if wide && let Some(next) = buf.cell_mut((x + 1, y)) {
                next.set_char(' ').set_style(style);
            }
        }
    }
}

fn faint(fg: [u8; 3], back: [u8; 3]) -> bool {
    (luma(fg) - luma(back)).abs() < CONTRAST
}

/// `fg`, or black or white when `fg` would not stand out from `back`.
fn readable(fg: [u8; 3], back: [u8; 3]) -> [u8; 3] {
    if !faint(fg, back) {
        fg
    } else if luma(back) > 0.5 {
        [0, 0, 0]
    } else {
        [255, 255, 255]
    }
}

fn rgb_of(c: Color) -> Option<[u8; 3]> {
    match c {
        Color::Rgb(r, g, b) => Some([r, g, b]),
        _ => None,
    }
}

fn mix(a: [u8; 3], b: [u8; 3]) -> [u8; 3] {
    [0, 1, 2].map(|i| ((u16::from(a[i]) + u16::from(b[i])) / 2) as u8)
}

fn luma([r, g, b]: [u8; 3]) -> f32 {
    (0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b)) / 255.
}

/// The letters of a typographic ligature, which fonts for terminals rarely have.
fn unligate(c: char) -> Option<&'static str> {
    Some(match c {
        'ﬀ' => "ff",
        'ﬁ' => "fi",
        'ﬂ' => "fl",
        'ﬃ' => "ffi",
        'ﬄ' => "ffl",
        _ => return None,
    })
}

/// Whether a font, by its PostScript name, is one formulas are set in.
fn math_font(name: &str) -> bool {
    const TEX: [&str; 8] = [
        "CMMI", "CMSY", "CMEX", "CMBSY", "MSAM", "MSBM", "EUFM", "RSFS",
    ];
    name.contains("Math") || TEX.iter().any(|tex| name.starts_with(tex))
}

/// Whether a font, by its PostScript name, is a bold one.
fn bold_font(name: &str) -> bool {
    const WEIGHTS: [&str; 4] = ["bold", "black", "heavy", "demi"];
    let lower = name.to_ascii_lowercase();
    name.starts_with("CMBX")
        || name.starts_with("CMB10")
        || WEIGHTS.iter().any(|w| lower.contains(w))
}

/// Whether a character is a formula's: Greek, a mathematical operator or
/// symbol, or a mathematical letter.
fn math_char(c: char) -> bool {
    matches!(c,
        '\u{0391}'..='\u{03C9}'
        | '\u{2200}'..='\u{22FF}'
        | '\u{27C0}'..='\u{27EF}'
        | '\u{2980}'..='\u{2AFF}'
        | '\u{1D400}'..='\u{1D7FF}')
}

/// Collects the glyphs a page draws, and the rules; everything else is
/// ignored.
#[derive(Default)]
struct Marks {
    glyphs: Vec<Mark>,
    across: Vec<Rect>,
    down: Vec<Rect>,
    /// Whether each font, by its cache key, is a math font and a bold one.
    fonts: HashMap<u128, (bool, bool)>,
}

impl<'a> Device<'a> for Marks {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'a>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}

    fn draw_path(&mut self, path: &BezPath, transform: Affine, _: &Paint<'a>, mode: &PathDrawMode) {
        let mut bbox = transform.transform_rect_bbox(path.bounding_box());
        if let PathDrawMode::Stroke(stroke) = mode {
            let half = f64::from(stroke.line_width) * transform.determinant().abs().sqrt() / 2.;
            bbox = bbox.inflate(half, half);
        }
        let (w, h) = (bbox.width(), bbox.height());
        if h <= RULE && w >= 2. * RULE {
            self.across.push(bbox);
        } else if w <= RULE && h >= 2. * RULE {
            self.down.push(bbox);
        }
    }

    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_image(&mut self, _: Image<'a, '_>, _: Affine) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}

    fn draw_glyph(
        &mut self,
        glyph: &Glyph<'a>,
        transform: Affine,
        glyph_transform: Affine,
        paint: &Paint<'a>,
        draw_mode: &GlyphDrawMode,
    ) {
        if matches!(draw_mode, GlyphDrawMode::Invisible) {
            return;
        }
        let text = match glyph.as_unicode() {
            Some(BfString::Char(c)) => unligate(c).map_or_else(|| c.to_string(), str::to_string),
            Some(BfString::String(s)) => s,
            None => return,
        };
        if text.trim().is_empty() {
            return;
        }
        let m = transform * glyph_transform;
        // Rotated text, as on a plot's axis, cannot be set in rows.
        let [a, b, ..] = m.as_coeffs();
        if a <= 0. || b.abs() > 0.1 * a {
            return;
        }
        let rgb = match paint {
            Paint::Color(c) => {
                let [r, g, b, alpha] = c.to_rgba().to_rgba8();
                if alpha < 128 {
                    return;
                }
                [r, g, b]
            }
            Paint::Pattern(_) => [0, 0, 0],
        };
        // Outlines are in units of 1000 per em.
        let origin = m * Point::ZERO;
        let size = (m * Point::new(0., 1000.) - origin).hypot() as f32;
        let bbox = match glyph {
            Glyph::Outline(o) => m.transform_rect_bbox(o.outline().bounding_box()),
            Glyph::Type3(_) => {
                let (ascent, width, descent) = TYPE3;
                let s = f64::from(size);
                Rect::new(
                    origin.x,
                    origin.y - ascent * s,
                    origin.x + width * s,
                    origin.y + descent * s,
                )
            }
        };
        let advance = match glyph {
            Glyph::Outline(o) => o.advance_width(),
            Glyph::Type3(_) => None,
        };
        let pen = |at: f64| (m * Point::new(at, 0.)).x;
        // A width past the outline by more than an em is in other units.
        let (start, end) = match advance.map(|a| pen(a.into())) {
            Some(end) if end >= origin.x && end <= bbox.x1 + f64::from(size) => (origin.x, end),
            _ => {
                let bearing = BEARING * f64::from(size);
                (bbox.x0 - bearing, bbox.x1 + bearing)
            }
        };
        // Filled and stroked text, as in fake bold, is drawn twice.
        let again = |l: &Mark| l.text == text && l.x0 == bbox.x0 as f32 && l.y == origin.y as f32;
        if self.glyphs.last().is_some_and(again) {
            return;
        }
        let (math_face, heavy) = match glyph {
            Glyph::Outline(o) => *self.fonts.entry(o.font_cache_key()).or_insert_with(|| {
                o.font_data()
                    .and_then(|f| f.postscript_name)
                    .map_or((false, false), |name| {
                        // Less a subset's tag, as `ABCDEF+`.
                        let name = name.split_once('+').map_or(&*name, |(_, base)| base);
                        (math_font(name), bold_font(name))
                    })
            }),
            Glyph::Type3(_) => (false, false),
        };
        let math = math_face || text.chars().any(math_char);
        self.glyphs.push(Mark {
            x0: bbox.x0 as f32,
            x1: bbox.x1 as f32,
            start: start as f32,
            end: end as f32,
            y0: bbox.y0 as f32,
            y1: bbox.y1 as f32,
            y: origin.y as f32,
            size,
            text,
            rgb,
            back: [255; 3],
            math,
            heavy,
        });
    }
}

#[cfg(test)]
#[path = "tests/text.rs"]
mod tests;
