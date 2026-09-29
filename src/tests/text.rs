use super::*;
use crate::fixture::{self, FRAME};
use std::sync::Arc;

/// Page 0 of a one-page PDF drawn by `content`.
fn page(content: &str) -> Page {
    let pdf = Pdf::new(Arc::new(fixture::with(&[content]))).unwrap();
    extract(
        &pdf,
        0,
        &InterpreterCache::new(),
        &InterpreterSettings::default(),
    )
    .unwrap()
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

/// A white render of `page` at 2 pixels a point.
fn render(page: &Page) -> RgbaImage {
    let (w, h) = ((page.width * 2.) as u32, (page.height * 2.) as u32);
    RgbaImage::from_pixel(w, h, Rgba([255; 4]))
}

/// A black glyph drawn by hand, `text` one em wide per character.
fn mark(x0: f32, y: f32, size: f32, text: &str) -> Mark {
    Mark {
        x0,
        x1: x0 + size * text.chars().count() as f32,
        y0: y - 0.7 * size,
        y1: y,
        y,
        size,
        text: text.into(),
        rgb: [0; 3],
        back: [255; 3],
    }
}

#[test]
fn words_are_read_apart_and_set_one_space_apart() {
    let page = page(&fixture::text(40., 200., 24., "Hello World"));
    assert_eq!((page.width, page.height), FRAME);
    let rows = rows(&page.layout(60, 20).0);
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
    let (cells, _) = page(&content).layout(60, 20);
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
    let (cells, _) = page(&content).layout(60, 40);
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
    let (cells, _) = page(&content).layout(100, 20);
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
    let rows = rows(&page(&content).layout(16, 20).0);
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
    let rows = rows(&page(&content).layout(60, 20).0);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].trim(), "shown");
}

#[test]
fn filled_and_stroked_text_is_read_once() {
    let rows = rows(
        &page("BT /F1 24 Tf 2 Tr 40 150 Td (Hello) Tj ET")
            .layout(60, 20)
            .0,
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].trim(), "Hello");
}

#[test]
fn rotated_text_is_left_to_the_image() {
    let content = format!(
        "BT /F1 12 Tf 0 1 -1 0 30 50 Tm (Axis) Tj ET {}",
        fixture::text(60., 150., 12., "body")
    );
    let page = page(&content);
    assert_eq!(page.glyphs.len(), 4);
    let rows = rows(&page.layout(60, 20).0);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].trim(), "body");
}

#[test]
fn wide_characters_take_two_cells() {
    let page = Page {
        width: 100.,
        height: 100.,
        glyphs: vec![
            mark(10., 50., 5., "한"),
            mark(15., 50., 5., "글"),
            mark(30., 50., 5., "ok"),
        ],
    };
    let (cells, set) = page.layout(20, 10);
    let row = cells
        .iter()
        .find(|row| row.iter().any(Option::is_some))
        .unwrap();
    let text: String = row.iter().map(|c| c.map_or('_', |c| c.ch)).collect();
    assert_eq!(text.trim_matches('_'), "한_글_ ok".replace(' ', "_"));
    assert!(set.iter().all(|&s| s));
}

#[test]
fn colour_is_kept() {
    let content = format!("1 0 0 rg {}", fixture::text(40., 150., 12., "red"));
    let (cells, _) = page(&content).layout(60, 20);
    assert_eq!(find(&cells, 'r').rgb, [255, 0, 0]);
}

#[test]
fn text_the_page_draws_faint_stays_faint() {
    let content = format!(
        "0.87 g {}0 g {}",
        fixture::text(40., 150., 12., "covered"),
        fixture::text(40., 100., 12., "shown")
    );
    let mut page = page(&content);
    let cells = page.set_over(&mut render(&page), 60, 20);
    assert!(find(&cells, 'c').faint);
    assert!(!find(&cells, 's').faint);
}

#[test]
fn glyphs_are_painted_with_the_colour_around_them() {
    let mut page = page(&fixture::text(40., 40., 24., "Hello"));
    let mut image = render(&page);
    let (w, h) = image.dimensions();
    let (blue, green, red) = (
        Rgba([0, 0, 255, 255]),
        Rgba([0, 255, 0, 255]),
        Rgba([255, 0, 0, 255]),
    );
    for (_, y, p) in image.enumerate_pixels_mut() {
        *p = if y < h / 2 { blue } else { green };
    }
    // Ink within each glyph's box, short of the margin taken for its
    // antialiased edge; neighbours' ink reaches into each other's margins.
    for g in &page.glyphs {
        let (x0, y0, x1, y1) = page.pixels(g, &image);
        assert!(
            y0 > h / 2 && x0 > 0,
            "the text should sit in the green half"
        );
        for y in y0 + 1..y1 {
            for x in x0 + 1..x1 {
                image.put_pixel(x, y, red);
            }
        }
    }
    page.set_over(&mut image, 60, 20);
    assert!(image.pixels().all(|&p| p != red), "text left in the image");
    assert_eq!(
        image.pixels().filter(|&&p| p == blue).count() as u32,
        w * (h / 2)
    );
}

#[test]
fn a_neighbours_ink_does_not_colour_a_glyph() {
    // A tall glyph, and a short one kerned into it.
    let tall = Mark {
        y0: 30.,
        ..mark(10., 50., 10., "l")
    };
    let short = Mark {
        y0: 40.,
        ..mark(19., 50., 10., "e")
    };
    let mut page = Page {
        width: 100.,
        height: 100.,
        glyphs: vec![tall, short],
    };
    let mut image = RgbaImage::from_pixel(200, 200, Rgba([255; 4]));
    let black = Rgba([0, 0, 0, 255]);
    for g in &page.glyphs {
        let (x0, y0, x1, y1) = page.pixels(g, &image);
        for y in y0 + 1..y1 {
            for x in x0 + 1..x1 {
                image.put_pixel(x, y, black);
            }
        }
    }
    page.set_over(&mut image, 60, 20);
    assert!(
        image.pixels().all(|&p| p != black),
        "text left in the image"
    );
}

#[test]
fn text_that_is_not_set_stays_in_the_image() {
    let content = [
        fixture::text(40., 200., 12., "first"),
        fixture::text(40., 120., 12., "second"),
        fixture::text(40., 40., 12., "third"),
    ]
    .concat();
    let mut page = page(&content);
    let mut image = render(&page);
    let black = Rgba([0, 0, 0, 255]);
    let centre = |page: &Page, g: &Mark, image: &RgbaImage| {
        let (x0, y0, x1, y1) = page.pixels(g, image);
        ((x0 + x1) / 2, (y0 + y1) / 2)
    };
    for g in &page.glyphs {
        let (x, y) = centre(&page, g, &image);
        image.put_pixel(x, y, black);
    }
    // One row: only the first line is set.
    let cells = page.set_over(&mut image, 60, 1);
    assert_eq!(
        rows(&cells),
        [format!("{:>w$}", "first", w = rows(&cells)[0].len())]
    );
    let top = page.glyphs.iter().map(|g| g.y).fold(f32::MAX, f32::min);
    for g in &page.glyphs {
        let (x, y) = centre(&page, g, &image);
        assert_eq!(*image.get_pixel(x, y) == black, g.y > top, "{}", g.text);
    }
}

#[test]
fn overlay_writes_text_that_reads_on_the_backdrop() {
    let area = Area::new(0, 0, 6, 2);
    let white = Color::Rgb(255, 255, 255);
    let black = Color::Rgb(0, 0, 0);
    let mut buf = Buffer::empty(area);
    for (x, back) in [(0, white), (1, black), (2, white)] {
        buf[(x, 0)].set_char('▀').set_fg(back).set_bg(back);
    }
    let dark = Cell {
        ch: 'a',
        rgb: [0; 3],
        bold: true,
        faint: false,
    };
    let pale = Cell {
        ch: 'b',
        rgb: [220; 3],
        bold: false,
        faint: true,
    };
    let cells: Cells = vec![
        vec![Some(dark), Some(dark), Some(pale), None, None, None],
        vec![None; 6],
    ];
    overlay(&cells, area, &mut buf);

    assert_eq!(
        (buf[(0, 0)].symbol(), buf[(0, 0)].fg, buf[(0, 0)].bg),
        ("a", black, white)
    );
    assert!(buf[(0, 0)].modifier.contains(Modifier::BOLD));
    // Black text on black would vanish: it turns white.
    assert_eq!((buf[(1, 0)].fg, buf[(1, 0)].bg), (white, black));
    // Text the page draws faint is not made to stand out.
    assert_eq!(buf[(2, 0)].fg, Color::Rgb(220, 220, 220));
    assert_eq!(buf[(3, 0)].symbol(), " ");
}

#[test]
fn overlay_blanks_the_cell_under_a_wide_character() {
    let area = Area::new(0, 0, 4, 1);
    let mut buf = Buffer::empty(area);
    for x in 0..4 {
        buf[(x, 0)]
            .set_char('▀')
            .set_fg(Color::Rgb(255, 255, 255))
            .set_bg(Color::Rgb(255, 255, 255));
    }
    let wide = Cell {
        ch: '한',
        rgb: [0; 3],
        bold: false,
        faint: false,
    };
    overlay(&vec![vec![Some(wide), None, None, None]], area, &mut buf);
    assert_eq!(buf[(0, 0)].symbol(), "한");
    assert_eq!(buf[(1, 0)].symbol(), " ");
    assert_eq!(buf[(2, 0)].symbol(), "▀");
}
