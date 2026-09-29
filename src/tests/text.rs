use super::*;
use crate::fixture::{self, FRAME};
use image::Rgba;
use std::sync::Arc;

/// Page 0 of a one-page PDF drawn by `content`.
fn page(content: &str) -> Page {
    let pdf = Pdf::new(Arc::new(fixture::with(&[content]))).unwrap();
    extract(&pdf, 0).unwrap()
}

/// Each row's text, trailing blanks trimmed; blank rows dropped.
fn rows(cells: &Cells) -> Vec<String> {
    cells
        .iter()
        .map(|row| {
            row.iter()
                .map(|c| c.map_or(' ', |c| c.ch))
                .collect::<String>()
        })
        .map(|row| row.trim_end().to_string())
        .filter(|row| !row.is_empty())
        .collect()
}

fn find(cells: &Cells, ch: char) -> Cell {
    cells
        .iter()
        .flatten()
        .flatten()
        .copied()
        .find(|c| c.ch == ch)
        .unwrap()
}

#[test]
fn words_are_read_apart_and_set_one_space_apart() {
    let page = page(&fixture::text(40., 200., 24., "Hello World"));
    assert_eq!((page.width, page.height), FRAME);
    let rows = rows(&page.layout(60, 20));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].trim_start(), "Hello World");
}

#[test]
fn lines_keep_their_order_and_a_title_is_bold() {
    let content = [
        fixture::text(20., 220., 24., "Title"),
        fixture::text(40., 150., 12., "first line"),
        fixture::text(40., 136., 12., "second line"),
    ]
    .concat();
    let cells = page(&content).layout(60, 20);
    let rows: Vec<String> = rows(&cells).iter().map(|r| r.trim().to_string()).collect();
    assert_eq!(rows, ["Title", "first line", "second line"]);
    assert!(find(&cells, 'T').bold);
    assert!(!find(&cells, 'f').bold);
}

#[test]
fn a_paragraph_goes_on_consecutive_rows() {
    let content = [
        fixture::text(40., 150., 12., "first"),
        fixture::text(40., 136., 12., "second"),
    ]
    .concat();
    let cells = page(&content).layout(60, 40);
    let at: Vec<usize> = cells
        .iter()
        .enumerate()
        .filter(|(_, row)| row.iter().any(Option::is_some))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(at.len(), 2);
    assert_eq!(at[1], at[0] + 1);
}

#[test]
fn columns_far_apart_keep_their_place() {
    let content = [
        fixture::text(40., 150., 12., "Left"),
        fixture::text(300., 150., 12., "Right"),
    ]
    .concat();
    let cells = page(&content).layout(100, 20);
    let row = cells
        .iter()
        .find(|row| row.iter().any(Option::is_some))
        .unwrap();
    let col = row
        .iter()
        .position(|c| c.is_some_and(|c| c.ch == 'R'))
        .unwrap();
    let want = (300. / FRAME.0 * 100.) as usize;
    assert!(
        col.abs_diff(want) <= 2,
        "Right at {col}, page puts it at {want}"
    );
}

#[test]
fn a_line_too_wide_wraps_and_the_rest_still_shows() {
    let content = [
        fixture::text(10., 150., 12., "one two three four five six seven"),
        fixture::text(10., 100., 12., "after"),
    ]
    .concat();
    let rows = rows(&page(&content).layout(16, 20));
    assert!(rows.len() >= 3, "{rows:?}");
    assert!(rows.iter().all(|r| r.chars().count() <= 16));
    assert_eq!(rows.last().unwrap().trim(), "after");
}

#[test]
fn text_off_the_page_is_dropped() {
    let content = [
        fixture::text(-500., 150., 12., "hidden"),
        fixture::text(40., 150., 12., "shown"),
    ]
    .concat();
    let rows = rows(&page(&content).layout(60, 20));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].trim(), "shown");
}

#[test]
fn colour_is_kept_and_black_takes_the_terminal_colour() {
    let content = format!(
        "1 0 0 rg {}0 g {}",
        fixture::text(40., 150., 12., "red"),
        fixture::text(40., 100., 12., "black")
    );
    let cells = page(&content).layout(60, 20);
    let red = find(&cells, 'r');
    assert_eq!(red.rgb, [255, 0, 0]);
    assert_eq!(plain(red).fg, Some(Color::Rgb(255, 0, 0)));
    assert_eq!(plain(find(&cells, 'b')).fg, None);
}

#[test]
fn lines_trim_trailing_blanks() {
    let lines = lines(&page(&fixture::text(40., 150., 12., "word")).layout(60, 20));
    assert_eq!(lines.len(), 20);
    let text: Vec<String> = lines
        .iter()
        .map(ToString::to_string)
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(text.len(), 1);
    assert_eq!(text[0].trim_start(), "word");
}

#[test]
fn erasing_paints_each_glyph_with_the_colour_around_it() {
    let page = page(&fixture::text(40., 150., 24., "Hello"));
    let scale = 2.;
    let (w, h) = ((page.width * scale) as u32, (page.height * scale) as u32);
    let blue = Rgba([0, 0, 255, 255]);
    let mut image = RgbaImage::from_pixel(w, h, blue);
    for g in &page.glyphs {
        let (x, y) = (
            ((g.x0 + g.x1) / 2. * scale) as u32,
            ((g.y0 + g.y1) / 2. * scale) as u32,
        );
        image.put_pixel(x, y, Rgba([0, 0, 0, 255]));
    }
    page.erase(&mut image);
    assert!(image.pixels().all(|&p| p == blue));
}

#[test]
fn overlay_writes_text_that_reads_on_the_backdrop() {
    let area = Area::new(0, 0, 4, 2);
    let white = Color::Rgb(255, 255, 255);
    let black = Color::Rgb(0, 0, 0);
    let mut buf = Buffer::empty(area);
    for (x, back) in [(0, white), (1, black)] {
        buf[(x, 0)].set_char('▀').set_fg(back).set_bg(back);
    }
    let dark = Some(Cell {
        ch: 'a',
        rgb: [0, 0, 0],
        bold: true,
    });
    let cells: Cells = vec![vec![dark, dark, None, None], vec![None; 4]];
    overlay(&cells, area, &mut buf);

    assert_eq!(
        (buf[(0, 0)].symbol(), buf[(0, 0)].fg, buf[(0, 0)].bg),
        ("a", black, white)
    );
    assert!(buf[(0, 0)].modifier.contains(Modifier::BOLD));
    // Black text on black would vanish: it turns white.
    assert_eq!((buf[(1, 0)].fg, buf[(1, 0)].bg), (white, black));
    assert_eq!(buf[(2, 0)].symbol(), " ");
}
