use super::*;
use crate::fixture::{self, FRAME};
use std::sync::Arc;

/// Page 0 of a one-page PDF drawn by `content`.
fn page(content: &str) -> Page {
    of(Arc::new(fixture::with(&[content])), 0).unwrap()
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
        start: x0,
        end: x0 + size * text.chars().count() as f32,
        y0: y - 0.7 * size,
        y1: y,
        y,
        size,
        text: text.into(),
        rgb: [0; 3],
        back: [255; 3],
        math: false,
        heavy: false,
    }
}

/// What reading `page` aloud says, a pause written `|`.
fn said(page: &Page) -> String {
    page.spoken()
        .iter()
        .map(|s| match s {
            Spoken::Text(t) => t.as_str(),
            Spoken::Pause => "|",
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn words_are_read_apart_and_set_one_space_apart() {
    let page = page(&fixture::text(40., 200., 24., "Hello World"));
    assert_eq!((page.width, page.height), FRAME);
    let rows = rows(&page.layout(60, 20, &[]).0);
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
    let (cells, _) = page(&content).layout(60, 20, &[]);
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
    let (cells, _) = page(&content).layout(60, 40, &[]);
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
    let (cells, _) = page(&content).layout(100, 20, &[]);
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
    let rows = rows(&page(&content).layout(16, 20, &[]).0);
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
    let rows = rows(&page(&content).layout(60, 20, &[]).0);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].trim(), "shown");
}

#[test]
fn filled_and_stroked_text_is_read_once() {
    let rows = rows(
        &page("BT /F1 24 Tf 2 Tr 40 150 Td (Hello) Tj ET")
            .layout(60, 20, &[])
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
    let rows = rows(&page.layout(60, 20, &[]).0);
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
        ..Page::default()
    };
    let (cells, set) = page.layout(20, 10, &[]);
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
    let (cells, _) = page(&content).layout(60, 20, &[]);
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
    let cells = page.set_over(&mut render(&page), 60, 20, &[]);
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
    page.set_over(&mut image, 60, 20, &[]);
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
        ..Page::default()
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
    page.set_over(&mut image, 60, 20, &[]);
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
    let cells = page.set_over(&mut image, 60, 1, &[]);
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
fn a_paragraph_is_read_as_one_line_its_broken_word_whole() {
    let content = [
        fixture::text(40., 150., 12., "Slides are read by easily-"),
        fixture::text(40., 136., 12., "downloadable voices"),
    ]
    .concat();
    assert_eq!(
        said(&page(&content)),
        "Slides are read by easily-downloadable voices"
    );
}

#[test]
fn a_title_and_items_are_read_apart_and_bullets_not_at_all() {
    let content = [
        fixture::text(20., 220., 24., "Results"),
        fixture::text(40., 150., 12., "* first item"),
        fixture::text(40., 136., 12., "* second item!"),
        fixture::text(40., 122., 12., "and its end"),
    ]
    .concat();
    assert_eq!(
        said(&page(&content)),
        "Results\nfirst item\nsecond item!\nand its end"
    );
}

#[test]
fn numbered_items_are_read_apart_and_their_numbers_not_at_all() {
    let content = [
        fixture::text(40., 150., 12., "1. First point"),
        fixture::text(40., 136., 12., "2. Second point"),
        fixture::text(40., 122., 12., "(iv) and the fourth"),
    ]
    .concat();
    assert_eq!(
        said(&page(&content)),
        "First point\nSecond point\nand the fourth"
    );
    assert!(label("12.") && label("b)") && label("(iv)") && label("(3)"));
    assert!(!label("2024.") && !label("Fig.") && !label("A.") && !label("one"));
}

#[test]
fn a_mark_set_over_its_letter_does_not_split_the_word() {
    // A grave accent, its pen not moved, set over the n before it.
    let grave = Mark {
        x1: 13.,
        start: 13.,
        end: 13.,
        ..mark(11., 50., 5., "\u{300}")
    };
    let page = Page {
        width: 100.,
        height: 100.,
        glyphs: vec![mark(10., 50., 5., "n"), grave, mark(15., 50., 5., "ext")],
        ..Page::default()
    };
    assert_eq!(said(&page), "n\u{300}ext");
}

#[test]
fn a_footline_is_not_read() {
    let content = [
        fixture::text(40., 150., 12., "Body text"),
        fixture::text(40., 136., 12., "more body"),
        fixture::text(40., 10., 6., "Author 3 / 20"),
        fixture::text(200., 20., 12., "7"),
    ]
    .concat();
    assert_eq!(said(&page(&content)), "Body text more body");
}

#[test]
fn letters_reaching_into_a_space_do_not_join_words() {
    // An italic f, its outline reaching 0.15 em into the 0.25 em space
    // after it.
    let f = Mark {
        x1: 31.5,
        ..mark(20., 50., 10., "f")
    };
    let page = Page {
        width: 100.,
        height: 100.,
        glyphs: vec![mark(10., 50., 10., "o"), f, mark(32.5, 50., 10., "raw")],
        ..Page::default()
    };
    assert_eq!(said(&page), "of raw");
}

#[test]
fn a_formula_is_a_pause_once_however_long() {
    let math = |x0, y, text| Mark {
        math: true,
        ..mark(x0, y, 5., text)
    };
    let page = Page {
        width: 200.,
        height: 100.,
        glyphs: vec![
            // Take x + y first, / so on. / α
            mark(10., 30., 5., "Take"),
            math(35., 30., "x"),
            math(41., 30., "+"),
            math(47., 30., "y"),
            mark(60., 30., 5., "first"),
            mark(10., 37., 5., "so"),
            mark(25., 37., 5., "on"),
            math(10., 60., "α"),
        ],
        ..Page::default()
    };
    assert_eq!(said(&page), "Take\n|\nfirst so on\n|");
}

#[test]
fn greek_and_math_letters_are_a_formulas() {
    assert!(math_char('α') && math_char('𝑍') && math_char('∑'));
    assert!(!math_char('a') && !math_char('é'));
    assert!(math_font("NewCMMath-Regular") && math_font("CMMI10"));
    assert!(!math_font("LibertinusSerif-Regular") && !math_font("CMR10"));
}

/// A rule across from `x0` to `x1` at `y`, or down from `y0` to `y1` at `x`.
fn across(x0: f64, x1: f64, y: f64) -> Rect {
    Rect::new(x0, y - 0.25, x1, y + 0.25)
}

fn down(x: f64, y0: f64, y1: f64) -> Rect {
    Rect::new(x - 0.25, y0, x + 0.25, y1)
}

#[test]
fn a_ruled_table_is_a_pause_and_ruled_prose_is_read() {
    let page = Page {
        width: 200.,
        height: 100.,
        glyphs: vec![
            mark(10., 10., 5., "Before"),
            // Columns closer than a wide gap, a rule between them.
            mark(10., 30., 5., "aaa"),
            mark(28., 30., 5., "b"),
            mark(10., 40., 5., "one"),
            mark(28., 40., 5., "two"),
            mark(10., 60., 5., "After"),
            // As a block quote, its rule down its left.
            mark(10., 80., 5., "quoted"),
            mark(42., 80., 5., "text"),
        ],
        across: vec![
            across(5., 50., 24.),
            across(5., 50., 34.),
            across(5., 50., 44.),
            across(5., 90., 70.),
            across(5., 90., 90.),
        ],
        down: vec![down(26.5, 24., 44.), down(5., 75., 85.)],
    };
    assert_eq!(said(&page), "Before\n|\nAfter\nquoted text");
}

#[test]
fn columns_far_apart_under_booktabs_rules_are_a_table() {
    let page = Page {
        width: 200.,
        height: 100.,
        glyphs: vec![
            mark(10., 30., 5., "Gene"),
            mark(60., 30., 5., "Score"),
            mark(10., 60., 5., "Prose"),
            mark(40., 60., 5., "here"),
        ],
        across: vec![across(5., 100., 20.), across(5., 100., 40.)],
        ..Page::default()
    };
    assert_eq!(said(&page), "|\nProse here");
}

#[test]
fn columns_are_read_one_after_the_other_under_a_title_across_them() {
    let page = Page {
        width: 200.,
        height: 100.,
        glyphs: vec![
            mark(10., 10., 5., "Two"),
            mark(28., 10., 5., "columns"),
            mark(66., 10., 5., "side"),
            mark(89., 10., 5., "by"),
            mark(102., 10., 5., "side"),
            mark(10., 30., 5., "Left"),
            mark(35., 30., 5., "one"),
            mark(110., 30., 5., "Right"),
            mark(140., 30., 5., "one"),
            mark(10., 37., 5., "goes"),
            mark(35., 37., 5., "on."),
            mark(110., 37., 5., "ends"),
            mark(135., 37., 5., "here."),
        ],
        ..Page::default()
    };
    assert_eq!(
        said(&page),
        "Two columns side by side\nLeft one goes on.\nRight one ends here."
    );
    let lit = page.light(1);
    assert_eq!(lit.boxes.len(), 2);
    assert!(lit.boxes.iter().all(|b| b.x1 < 100.));
}

#[test]
fn a_sentence_goes_on_from_a_columns_foot_to_the_next_columns_head() {
    let page = Page {
        width: 200.,
        height: 100.,
        glyphs: vec![
            mark(10., 30., 5., "It"),
            mark(110., 30., 5., "the"),
            mark(10., 37., 5., "runs"),
            mark(35., 37., 5., "into"),
            mark(110., 37., 5., "next."),
        ],
        ..Page::default()
    };
    assert_eq!(said(&page), "It runs into the next.");
}

#[test]
fn a_lists_labels_are_not_a_column() {
    let page = Page {
        width: 200.,
        height: 100.,
        glyphs: vec![
            mark(10., 20., 10., "*"),
            mark(25., 20., 10., "first"),
            mark(80., 20., 10., "item"),
            mark(25., 32., 10., "wrapped"),
            mark(10., 44., 10., "*"),
            mark(25., 44., 10., "second"),
        ],
        ..Page::default()
    };
    assert_eq!(said(&page), "first item wrapped\nsecond");
}

#[test]
fn prose_between_the_pages_own_rules_is_read_though_a_line_has_a_wide_gap() {
    let page = Page {
        width: 200.,
        height: 100.,
        glyphs: vec![
            mark(10., 30., 5., "Prose"),
            mark(37., 30., 5., "with"),
            // A superscript's room left in the line.
            mark(10., 37., 5., "gap"),
            mark(40., 37., 5., "here."),
            mark(10., 44., 5., "more"),
            mark(32., 44., 5., "prose."),
        ],
        across: vec![across(5., 190., 5.), across(5., 190., 95.)],
        ..Page::default()
    };
    assert_eq!(said(&page), "Prose with gap here.\nmore prose.");
}

#[test]
fn a_centred_title_and_display_beside_short_lines_are_no_column() {
    let page = Page {
        width: 200.,
        height: 100.,
        glyphs: vec![
            mark(120., 10., 10., "Notes"),
            mark(10., 30., 10., "Short"),
            mark(10., 45., 10., "one."),
            Mark {
                math: true,
                ..mark(120., 60., 10., "x")
            },
            mark(10., 75., 10., "More."),
        ],
        ..Page::default()
    };
    assert_eq!(said(&page), "Notes\nShort one.\n|\nMore.");
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
        lit: false,
        glyph: 0,
    };
    let pale = Cell {
        ch: 'b',
        rgb: [220; 3],
        bold: false,
        faint: true,
        lit: false,
        glyph: 0,
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
        lit: false,
        glyph: 0,
    };
    overlay(&vec![vec![Some(wide), None, None, None]], area, &mut buf);
    assert_eq!(buf[(0, 0)].symbol(), "한");
    assert_eq!(buf[(1, 0)].symbol(), " ");
    assert_eq!(buf[(2, 0)].symbol(), "▀");
}

#[test]
fn a_sentence_ends_at_its_stop_but_not_at_an_initial_or_abbreviation() {
    for word in ["end.", "why?", "so!", "(this).", "“quoted.”"] {
        assert!(ends_sentence(word), "{word}");
    }
    for word in ["e.g.", "Fig.", "J.", "(i.e.", "word", "3.5"] {
        assert!(!ends_sentence(word), "{word}");
    }
}

/// A paragraph of two sentences, the second over two lines.
fn two_sentences() -> Page {
    page(
        &[
            fixture::text(40., 150., 12., "One thing, e.g. this. Another"),
            fixture::text(40., 136., 12., "thing here."),
        ]
        .concat(),
    )
}

#[test]
fn sentences_are_read_apart_across_lines() {
    assert_eq!(
        said(&two_sentences()),
        "One thing, e.g. this.\nAnother thing here."
    );
}

#[test]
fn a_sentence_lit_is_boxed_a_line_at_a_time() {
    let page = two_sentences();
    let lit = page.light(1);
    let boxes = lit.boxes;
    let lit: String = page
        .glyphs
        .iter()
        .zip(&lit.glyphs)
        .filter(|(_, lit)| **lit)
        .map(|(g, _)| g.text.as_str())
        .collect();
    assert_eq!(lit, "Anotherthinghere.");
    assert_eq!(boxes.len(), 2);
    // The first line's box starts at the sentence, not the line.
    let start = page
        .glyphs
        .iter()
        .find(|g| g.text.starts_with('A'))
        .unwrap();
    assert!((boxes[0].x0 - f64::from(start.start)).abs() < 0.01);
    assert!(boxes[1].x0 < boxes[0].x0);
    assert!(boxes.iter().all(|b| b.height() > 10. && b.height() < 14.));

    assert!(page.light(5).boxes.is_empty());
}

#[test]
fn the_cells_of_a_sentence_lit_are_lit() {
    let page = two_sentences();
    let (cells, _) = page.layout(80, 20, &page.light(0).glyphs);
    let lit: String = cells
        .iter()
        .flatten()
        .flatten()
        .filter(|c| c.lit)
        .map(|c| c.ch)
        .collect();
    assert_eq!(lit, "Onething,e.g.this.");
}

#[test]
fn a_sentence_is_found_from_a_glyph_set_on_the_grid_or_a_point_near_it() {
    let page = two_sentences();
    let (cells, _) = page.layout(80, 20, &[]);
    let at = |ch| page.sentence_of(find(&cells, ch).glyph);
    assert_eq!((at('O'), at('A'), at('r')), (Some(0), Some(1), Some(1)));
    let start = |text: &str| page.glyphs.iter().find(|g| g.text == text).unwrap();
    let a = start("A");
    assert_eq!(page.sentence_near(a.x0 + 1., a.y - 2.), Some(1));
    let o = start("O");
    // Left of the first word, a little way into the margin.
    assert_eq!(page.sentence_near(o.x0 - 5., o.y), Some(0));
    assert_eq!(page.sentence_near(o.x0, o.y + 100.), None);
}

#[test]
fn a_bold_font_is_known_by_its_name() {
    for name in [
        "LibertinusSerif-Bold",
        "Helvetica-BoldOblique",
        "SourceSans3-Semibold",
        "CMBX10",
    ] {
        assert!(bold_font(name), "{name}");
    }
    for name in ["LibertinusSerif-Regular", "Helvetica", "CMR10", "CMMI10"] {
        assert!(!bold_font(name), "{name}");
    }
}

#[test]
fn a_bold_heading_ends_its_sentence_and_is_bold() {
    let heavy = |m: Mark| Mark { heavy: true, ..m };
    let page = Page {
        width: 200.,
        height: 100.,
        glyphs: vec![
            heavy(mark(5., 20., 10., "Heading")),
            mark(5., 32., 10., "Body"),
            mark(60., 32., 10., "text."),
        ],
        ..Page::default()
    };
    assert_eq!(said(&page), "Heading\nBody text.");
    let (cells, _) = page.layout(60, 20, &[]);
    assert!(find(&cells, 'H').bold);
    assert!(!find(&cells, 'B').bold);
}
