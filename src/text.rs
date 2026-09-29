//! Slides as text, for terminals that cannot show images well.
//!
//! The interpreter reports every glyph it draws; each is kept with where it
//! sits, its size and its colour. Glyphs on one baseline form a line, and a
//! gap between glyphs starts a new word: TeX places words apart rather than
//! writing space characters. Lines are then set on the cell grid at the
//! height they have on the page, their words one space apart unless the page
//! puts them far apart, as in columns and tables.
//!
//! Text can be drawn over a coarse image of the slide, for its figures and
//! colours. The glyphs are painted out of that image first: the text is set
//! where it reads well, not exactly where the page has it, and would
//! otherwise show twice.

use hayro::hayro_interpret::font::Glyph;
use hayro::hayro_interpret::hayro_cmap::BfString;
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache,
    InterpreterSettings, Paint, PathDrawMode, SoftMask, TransformExt, interpret_page,
};
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::kurbo::{Affine, BezPath, Point, Rect, Shape};
use image::RgbaImage;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect as Area;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// A gap wider than this, in ems, starts a new word.
const WORD_GAP: f32 = 0.2;
/// A gap wider than this, in ems, is kept on screen, as between columns.
const WIDE_GAP: f32 = 1.5;
/// Baselines closer than this, in ems, are one line.
const SAME_LINE: f32 = 0.35;
/// Lines closer than this, in ems, are one paragraph: they go on consecutive
/// rows, where the page's proportions would scatter them.
const TIGHT: f32 = 1.6;
/// Text this much larger than the body is drawn bold.
const LARGE: f32 = 1.15;
/// Least difference in luma between text and what is behind it.
const CONTRAST: f32 = 0.4;

/// The text of one page, in page points with y growing down.
#[derive(Clone, Debug, Default)]
pub struct Page {
    pub width: f32,
    pub height: f32,
    glyphs: Vec<Mark>,
}

/// One drawn glyph.
#[derive(Clone, Debug)]
struct Mark {
    x0: f32,
    x1: f32,
    /// Top and bottom of the glyph's outline.
    y0: f32,
    y1: f32,
    /// Baseline.
    y: f32,
    /// Em size.
    size: f32,
    text: String,
    rgb: [u8; 3],
}

/// A run of glyphs with no word gap between them.
#[derive(Debug)]
struct Word {
    x0: f32,
    x1: f32,
    size: f32,
    text: String,
    rgb: [u8; 3],
}

/// A character set on the grid, in the colour the page gives it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cell {
    pub ch: char,
    pub rgb: [u8; 3],
    pub bold: bool,
}

/// Rows of cells, `None` where nothing is written.
pub type Cells = Vec<Vec<Option<Cell>>>;

pub fn extract(pdf: &Pdf, page: usize) -> Option<Page> {
    let page = pdf.pages().get(page)?;
    let (width, height) = page.render_dimensions();
    let cache = InterpreterCache::new();
    let mut ctx = Context::new(
        page.initial_transform(true).to_kurbo(),
        Rect::new(0., 0., width.into(), height.into()),
        &cache,
        page.xref(),
        InterpreterSettings::default(),
    );
    let mut marks = Marks::default();
    interpret_page(page, &mut ctx, &mut marks);
    // Text off the page is clipped away when rendered; beamer keeps some there.
    let on_page = |g: &Mark| g.x1 >= 0. && g.x0 <= width && g.y >= 0. && g.y <= height + g.size;
    marks.0.retain(on_page);
    Some(Page {
        width,
        height,
        glyphs: marks.0,
    })
}

impl Page {
    /// The page set on a `cols` × `rows` grid.
    pub fn layout(&self, cols: u16, rows: u16) -> Cells {
        let (cols, rows) = (usize::from(cols), usize::from(rows));
        let mut grid: Cells = vec![vec![None; cols]; rows];
        if cols == 0 || rows == 0 || self.width <= 0. || self.height <= 0. {
            return grid;
        }
        let body = self.body_size();
        let col_of =
            |x: f32| ((x / self.width * cols as f32).round().max(0.) as usize).min(cols - 1);
        // The last line placed: its baseline, size and row.
        let mut prev: Option<(f32, f32, usize)> = None;
        for (y, words) in self.text_lines() {
            let size = words.iter().map(|w| w.size).fold(0., f32::max);
            let want = (y / self.height * rows as f32) as usize;
            let mut row = match prev {
                Some((py, psize, prow)) if y - py <= TIGHT * size.max(psize) => prow + 1,
                Some((_, _, prow)) => want.max(prow + 1),
                None => want,
            };
            let spots = place(&words, &col_of, cols);
            while row < rows && !free(&grid, row, &spots) {
                row += 1;
            }
            if row >= rows {
                break;
            }
            for (word, &(down, col)) in words.iter().zip(&spots) {
                let bold = word.size > LARGE * body;
                for (i, ch) in word.text.chars().enumerate() {
                    if let Some(cell) = grid[row + down].get_mut(col + i) {
                        *cell = Some(Cell {
                            ch,
                            rgb: word.rgb,
                            bold,
                        });
                    }
                }
            }
            let last = spots.last().map_or(0, |&(down, _)| down);
            prev = Some((y, size, (row + last).min(rows - 1)));
        }
        grid
    }

    /// Paint each glyph out of `image`, a render of this page, with the
    /// colour just above and left of it.
    pub fn erase(&self, image: &mut RgbaImage) {
        let (w, h) = (image.width(), image.height());
        if w == 0 || h == 0 || self.width <= 0. {
            return;
        }
        let scale = w as f32 / self.width;
        let px = |v: f32, max: u32| ((v * scale).max(0.) as u32).min(max - 1);
        for g in &self.glyphs {
            let (x0, x1) = (px(g.x0, w), px(g.x1, w));
            let (y0, y1) = (px(g.y0, h), px(g.y1, h));
            let fill = *image.get_pixel(x0.saturating_sub(1), y0.saturating_sub(1));
            for y in y0..=y1 {
                for x in x0..=x1 {
                    image.put_pixel(x, y, fill);
                }
            }
        }
    }

    /// The most common glyph size: the body text's.
    fn body_size(&self) -> f32 {
        let mut sizes: Vec<i32> = self
            .glyphs
            .iter()
            .map(|g| (g.size * 2.).round() as i32)
            .collect();
        sizes.sort_unstable();
        let mut best = (0, 0);
        for run in sizes.chunk_by(|a, b| a == b) {
            if run.len() > best.1 {
                best = (run[0], run.len());
            }
        }
        best.0 as f32 / 2.
    }

    /// Lines top to bottom, each its baseline and its words left to right.
    fn text_lines(&self) -> Vec<(f32, Vec<Word>)> {
        let mut glyphs: Vec<&Mark> = self.glyphs.iter().collect();
        glyphs.sort_by(|a, b| a.y.total_cmp(&b.y));
        let mut lines: Vec<Vec<&Mark>> = Vec::new();
        for g in glyphs {
            match lines.last_mut() {
                Some(line) if g.y - line[0].y <= SAME_LINE * line[0].size.max(g.size) => {
                    line.push(g)
                }
                _ => lines.push(vec![g]),
            }
        }
        lines
            .into_iter()
            .map(|mut line| {
                line.sort_by(|a, b| a.x0.total_cmp(&b.x0));
                (line[0].y, words(&line))
            })
            .collect()
    }
}

fn words(line: &[&Mark]) -> Vec<Word> {
    let mut words: Vec<Word> = Vec::new();
    for g in line {
        match words.last_mut() {
            Some(w) if g.x0 - w.x1 <= WORD_GAP * w.size.max(g.size) => {
                w.text.push_str(&g.text);
                w.x1 = w.x1.max(g.x1);
            }
            _ => words.push(Word {
                x0: g.x0,
                x1: g.x1,
                size: g.size,
                text: g.text.clone(),
                rgb: g.rgb,
            }),
        }
    }
    words
}

/// Where each word of a line goes, as rows down from the line's first and a
/// column: one space after the word before it, or at its place on the page
/// when the page leaves a wide gap. A word that would run past `cols` wraps
/// to the next row, under the line's first word.
fn place(words: &[Word], col_of: &impl Fn(f32) -> usize, cols: usize) -> Vec<(usize, usize)> {
    let mut spots = Vec::with_capacity(words.len());
    let mut down = 0;
    let mut start = 0;
    let mut end: Option<(usize, f32)> = None;
    for w in words {
        let len = w.text.chars().count();
        let mut col = match end {
            None => {
                start = col_of(w.x0).min(cols.saturating_sub(len));
                start
            }
            Some((c, x1)) if w.x0 - x1 > WIDE_GAP * w.size => col_of(w.x0).max(c + 1),
            Some((c, _)) => c + 1,
        };
        if col + len > cols && col > start {
            down += 1;
            col = start;
        }
        spots.push((down, col));
        end = Some((col + len, w.x1));
    }
    spots
}

/// Whether words placed from `row` fit above the grid's bottom, with the
/// cell before each and its first two empty.
fn free(grid: &Cells, row: usize, spots: &[(usize, usize)]) -> bool {
    spots.iter().all(|&(down, col)| {
        grid.get(row + down).is_some_and(|r| {
            let span = col.saturating_sub(1).min(r.len())..(col + 2).min(r.len());
            r[span].iter().all(Option::is_none)
        })
    })
}

/// The cells as lines of text in the terminal's own colours, for drawing
/// on the terminal's background.
pub fn lines(cells: &Cells) -> Vec<Line<'static>> {
    cells
        .iter()
        .map(|row| {
            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut run = String::new();
            let mut run_style = Style::new();
            for cell in row {
                let (ch, style) = cell.map_or((' ', Style::new()), |c| (c.ch, plain(c)));
                if style != run_style && !run.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut run), run_style));
                }
                run_style = style;
                run.push(ch);
            }
            let run = run.trim_end().to_string();
            if !run.is_empty() {
                spans.push(Span::styled(run, run_style));
            }
            Line::from(spans)
        })
        .collect()
}

/// Write the cells over what `area` of `buf` already shows, a coarse image
/// of the slide, each on the colour behind it.
pub fn overlay(cells: &Cells, area: Area, buf: &mut Buffer) {
    for (dy, row) in cells.iter().enumerate().take(area.height.into()) {
        for (dx, cell) in row.iter().enumerate().take(area.width.into()) {
            let Some(cell) = cell else { continue };
            let (x, y) = (area.x + dx as u16, area.y + dy as u16);
            let Some(target) = buf.cell_mut((x, y)) else {
                continue;
            };
            // A block cell shows its top half in fg and bottom half in bg.
            let back = match (rgb_of(target.fg), rgb_of(target.bg)) {
                (Some(a), Some(b)) => mix(a, b),
                (Some(c), None) | (None, Some(c)) => c,
                (None, None) => [255, 255, 255],
            };
            let [r, g, b] = readable(cell.rgb, back);
            let [br, bg, bb] = back;
            let mut style = Style::new()
                .fg(Color::Rgb(r, g, b))
                .bg(Color::Rgb(br, bg, bb));
            if cell.bold {
                style = style.add_modifier(Modifier::BOLD);
            }
            target.set_char(cell.ch).set_style(style);
        }
    }
}

/// A cell's style on the terminal's background: black and white text take
/// the terminal's own colour, so it reads on dark and light terminals alike.
fn plain(cell: Cell) -> Style {
    let [r, g, b] = cell.rgb;
    let mut style = if (0.25..0.85).contains(&luma(cell.rgb)) || saturated(cell.rgb) {
        Style::new().fg(Color::Rgb(r, g, b))
    } else {
        Style::new()
    };
    if cell.bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    style
}

/// `fg`, or black or white when `fg` would not stand out from `back`.
fn readable(fg: [u8; 3], back: [u8; 3]) -> [u8; 3] {
    if (luma(fg) - luma(back)).abs() >= CONTRAST {
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

fn saturated([r, g, b]: [u8; 3]) -> bool {
    let (hi, lo) = (r.max(g).max(b), r.min(g).min(b));
    hi > 60 && f32::from(hi - lo) / f32::from(hi) > 0.4
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

/// Collects the glyphs a page draws; everything else is ignored.
#[derive(Default)]
struct Marks(Vec<Mark>);

impl<'a> Device<'a> for Marks {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'a>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}
    fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'a>, _: &PathDrawMode) {}
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
        // Outlines are in units of 1000 per em.
        let m = transform * glyph_transform;
        let origin = m * Point::ZERO;
        let size = (m * Point::new(0., 1000.) - origin).hypot() as f32;
        let bbox = match glyph {
            Glyph::Outline(o) => m.transform_rect_bbox(o.outline().bounding_box()),
            Glyph::Type3(_) => {
                let s = f64::from(size);
                Rect::new(
                    origin.x,
                    origin.y - 0.8 * s,
                    origin.x + 0.5 * s,
                    origin.y + 0.2 * s,
                )
            }
        };
        let rgb = match paint {
            Paint::Color(c) => {
                let [r, g, b, _] = c.to_rgba().to_rgba8();
                [r, g, b]
            }
            Paint::Pattern(_) => [0, 0, 0],
        };
        self.0.push(Mark {
            x0: bbox.x0 as f32,
            x1: bbox.x1 as f32,
            y0: bbox.y0 as f32,
            y1: bbox.y1 as f32,
            y: origin.y as f32,
            size,
            text,
            rgb,
        });
    }
}

#[cfg(test)]
#[path = "tests/text.rs"]
mod tests;
