//! The text set on the cell grid, written over a coarse image of the slide.

use super::{CONTRAST, Cells, LIGHT, MARK};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect as Area;
use ratatui::style::{Color, Modifier, Style};
use unicode_width::UnicodeWidthChar;

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
            let back = match (cell.lit, cell.marked) {
                (true, _) => LIGHT,
                (false, true) => MARK,
                (false, false) => mix(top, bottom),
            };
            let fg = if cell.faint && !cell.lit && !cell.marked {
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

pub(super) fn faint(fg: [u8; 3], back: [u8; 3]) -> bool {
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
