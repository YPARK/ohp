use super::*;
use crate::fixture::{self, FRAME};
use std::time::Duration;

#[test]
fn deck_reads_page_count_and_frame_size() {
    let dir = tempfile::tempdir().unwrap();
    let deck = Deck::open(&fixture::write(dir.path(), 5)).unwrap();
    assert_eq!(deck.pages, 5);
    assert_eq!(deck.name, "deck.pdf");
    let (w, h) = deck.page_size;
    assert!((w - FRAME.0).abs() < 0.01 && (h - FRAME.1).abs() < 0.01);
}

#[test]
fn a_file_that_is_not_a_pdf_does_not_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("deck.pdf");
    std::fs::write(&path, b"%PDF-1.5\ngarbage\n%%EOF\n").unwrap();
    assert!(Deck::open(&path).is_err());
    assert!(Deck::open(&dir.path().join("missing.pdf")).is_err());
}

#[test]
fn only_a_pdf_ending_in_eof_is_finished() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture::write(dir.path(), 3);
    assert!(finished(&path));

    let whole = fixture::pdf(3);
    std::fs::write(&path, &whole[..whole.len() / 2]).unwrap();
    assert!(!finished(&path));

    assert!(!finished(&dir.path().join("missing.pdf")));
}

#[test]
fn stamp_changes_when_the_file_is_rewritten() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture::write(dir.path(), 3);
    let before = stamp(&path);
    assert!(before.is_some());
    fixture::write(dir.path(), 4);
    assert_ne!(stamp(&path), before);
    assert_eq!(stamp(&dir.path().join("missing.pdf")), None);
}

#[test]
fn a_slide_fills_its_cell_box_without_overflowing() {
    let dir = tempfile::tempdir().unwrap();
    let deck = Deck::open(&fixture::write(dir.path(), 2)).unwrap();
    let pdf = Pdf::new(deck.data.clone()).unwrap();
    let cache = RenderCache::new();
    let picker = Picker::halfblocks();
    let font = picker.font_size();

    for (cols, rows) in [(40, 10), (20, 30), (140, 39)] {
        let key = key(1, cols, rows);
        let image = rasterise(
            &pdf,
            &cache,
            &InterpreterSettings::default(),
            &picker,
            key,
            &[],
            &Kept::default(),
        )
        .unwrap();
        let (bw, bh) = (u32::from(cols * font.width), u32::from(rows * font.height));
        assert!(image.width() <= bw && image.height() <= bh, "{cols}x{rows}");
        assert!(
            bw - image.width() <= 1 || bh - image.height() <= 1,
            "{cols}x{rows}: not fitted"
        );
    }
}

#[test]
fn a_page_past_the_end_renders_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let deck = Deck::open(&fixture::write(dir.path(), 2)).unwrap();
    let pdf = Pdf::new(deck.data.clone()).unwrap();
    let key = key(2, 40, 10);
    let settings = InterpreterSettings::default();
    assert!(
        rasterise(
            &pdf,
            &RenderCache::new(),
            &settings,
            &Picker::halfblocks(),
            key,
            &[],
            &Kept::default()
        )
        .is_none()
    );
}

/// Page `page` fitted to a `cols` × `rows` box.
fn key(page: usize, cols: u16, rows: u16) -> Key {
    Key::new(page, Rect::new(0, 0, cols, rows))
}

fn job(page: usize, cols: u16, rows: u16, look: Look) -> Job {
    Job {
        key: key(page, cols, rows),
        look,
        lit: None,
    }
}

#[test]
fn workers_deliver_every_requested_slide() {
    let dir = tempfile::tempdir().unwrap();
    let deck = Deck::open(&fixture::write(dir.path(), 4)).unwrap();
    let renderer = Renderer::spawn(&deck, &Picker::halfblocks(), 2).unwrap();
    let jobs: Vec<Job> = (0..4).map(|page| job(page, 30, 8, Look::Image)).collect();
    renderer.push(jobs.clone());

    let mut got: Vec<Job> = (0..jobs.len())
        .map(|_| {
            let done = fixture::next(&renderer);
            assert!(
                matches!(done.slide, Some(Slide::Image(_))),
                "{:?} failed",
                done.job
            );
            done.job
        })
        .collect();
    got.sort_by_key(|j| j.key.page);
    assert_eq!(got, jobs);
}

#[test]
fn a_text_job_sets_the_text_on_its_backdrop() {
    let dir = tempfile::tempdir().unwrap();
    let deck = Deck::open(&fixture::write_pdf(dir.path(), &fixture::hello())).unwrap();
    let renderer = Renderer::spawn(&deck, &Picker::halfblocks(), 1).unwrap();
    renderer.push([job(0, 60, 30, Look::Text)]);

    let Some(Slide::Text { cells, backdrop }) = fixture::next(&renderer).slide else {
        panic!("no text slide");
    };
    let size = backdrop.size();
    assert_eq!(
        (cells.len(), cells[0].len()),
        (size.height.into(), size.width.into())
    );
    // 16:9 in 60x30 cells of 10x20 pixels: the full width, and as tall as that makes it.
    assert_eq!(size.width, 60);
    assert_eq!(
        size.height,
        (60. * 10. * FRAME.1 / FRAME.0 / 20.).round() as u16
    );
    let text: String = cells.iter().flatten().flatten().map(|c| c.ch).collect();
    assert_eq!(text, "Hello");
}

#[test]
fn cleared_jobs_are_not_rendered() {
    let dir = tempfile::tempdir().unwrap();
    let deck = Deck::open(&fixture::write(dir.path(), 3)).unwrap();
    let renderer = Renderer::spawn(&deck, &Picker::halfblocks(), 1).unwrap();
    // Let the worker reach its wait; jobs queued without waking it then stay queued.
    std::thread::sleep(Duration::from_millis(100));
    let (lock, _) = &*renderer.queue;
    let jobs = (0..3).map(|page| job(page, 30, 8, Look::Image));
    lock.lock().unwrap().waiting.extend(jobs);
    assert_eq!(renderer.clear().len(), 3);
    assert!(
        renderer
            .done
            .recv_timeout(Duration::from_millis(200))
            .is_err()
    );
}

/// A zoomed rasterisation of page 1 of a two-page deck, whose square sits
/// low on the left, in a 40×10 box of 10×20 pixel cells.
fn zoomed(zoom: Zoom) -> RgbaImage {
    let dir = tempfile::tempdir().unwrap();
    let deck = Deck::open(&fixture::write(dir.path(), 2)).unwrap();
    let pdf = Pdf::new(deck.data.clone()).unwrap();
    let key = Key {
        zoom,
        ..key(1, 40, 10)
    };
    let settings = InterpreterSettings::default();
    rasterise(
        &pdf,
        &RenderCache::new(),
        &settings,
        &Picker::halfblocks(),
        key,
        &[],
        &Kept::default(),
    )
    .unwrap()
}

fn has_blue(image: &RgbaImage) -> bool {
    image.pixels().any(|p| p.0[2] > 200 && p.0[0] < 50)
}

#[test]
fn a_zoomed_slide_fills_its_box_with_the_part_panned_to() {
    let top = zoomed(Zoom {
        percent: 200,
        x: 0,
        y: 0,
    });
    assert_eq!(top.dimensions(), (400, 200));
    assert!(!has_blue(&top), "the square is in the bottom half");
    let bottom = zoomed(Zoom {
        percent: 200,
        x: 0,
        y: 10,
    });
    assert_eq!(bottom.dimensions(), (400, 200));
    assert!(has_blue(&bottom));
}

#[test]
fn panning_past_the_edge_still_fills_the_box() {
    let far = zoomed(Zoom {
        percent: 200,
        x: 1000,
        y: 1000,
    });
    assert_eq!(far.dimensions(), (400, 200));
}

#[test]
fn a_zoom_too_large_to_render_is_enlarged_to_fill_the_box() {
    let dir = tempfile::tempdir().unwrap();
    let deck = Deck::open(&fixture::write(dir.path(), 1)).unwrap();
    let pdf = Pdf::new(deck.data.clone()).unwrap();
    // 400% of a slide fitted to 1400×780 pixels is over 17 million pixels.
    let zoom = Zoom {
        percent: 400,
        x: 0,
        y: 0,
    };
    let key = Key {
        zoom,
        ..key(0, 140, 39)
    };
    let settings = InterpreterSettings::default();
    let image = rasterise(
        &pdf,
        &RenderCache::new(),
        &settings,
        &Picker::halfblocks(),
        key,
        &[],
        &Kept::default(),
    )
    .unwrap();
    assert_eq!(image.dimensions(), (1400, 780));
}

#[test]
fn a_zoomed_text_job_shows_the_part_panned_to() {
    let dir = tempfile::tempdir().unwrap();
    let deck = Deck::open(&fixture::write_pdf(dir.path(), &fixture::hello())).unwrap();
    let renderer = Renderer::spawn(&deck, &Picker::halfblocks(), 1).unwrap();
    let text = |x| {
        let key = Key {
            zoom: Zoom {
                percent: 200,
                x,
                y: 0,
            },
            ..key(0, 60, 30)
        };
        renderer.push([Job {
            key,
            look: Look::Text,
            lit: None,
        }]);
        let Some(Slide::Text { cells, backdrop }) = fixture::next(&renderer).slide else {
            panic!("no text slide");
        };
        assert_eq!(backdrop.size(), Size::new(60, 30));
        assert_eq!((cells.len(), cells[0].len()), (30, 60));
        cells
            .iter()
            .flatten()
            .flatten()
            .map(|c| c.ch)
            .collect::<String>()
    };
    assert_eq!(text(0), "Hello");
    assert_eq!(text(60), "");
}

#[test]
fn a_slide_lit_again_is_lit_on_the_page_kept_unlit() {
    let dir = tempfile::tempdir().unwrap();
    let deck = Deck::open(&fixture::write(dir.path(), 2)).unwrap();
    let pdf = Pdf::new(deck.data.clone()).unwrap();
    let (cache, settings, picker) = (
        RenderCache::new(),
        InterpreterSettings::default(),
        Picker::halfblocks(),
    );
    let key = key(1, 40, 10);
    let lit = |boxes: &[kurbo::Rect], kept: &Kept| {
        rasterise(&pdf, &cache, &settings, &picker, key, boxes, kept).unwrap()
    };
    let (first, second) = (
        [kurbo::Rect::new(0., 0., 50., 50.)],
        [kurbo::Rect::new(100., 100., 150., 150.)],
    );
    let kept = Kept::default();
    // Slides drawn with nothing lit, as those either side, are not kept.
    let unlit = lit(&[], &kept);
    assert!(kept.lock().unwrap().is_none());
    lit(&first, &kept);
    assert!(
        (kept.lock().unwrap().as_ref())
            .is_some_and(|((page, ..), image)| *page == 1 && **image == unlit)
    );
    assert_eq!(lit(&second, &kept), lit(&second, &Kept::default()));
}

#[test]
fn a_zoomed_slide_panned_is_lit_on_the_page_kept() {
    let dir = tempfile::tempdir().unwrap();
    let deck = Deck::open(&fixture::write(dir.path(), 2)).unwrap();
    let pdf = Pdf::new(deck.data.clone()).unwrap();
    let (cache, settings, picker) = (
        RenderCache::new(),
        InterpreterSettings::default(),
        Picker::halfblocks(),
    );
    let at = |y| Key {
        zoom: Zoom {
            percent: 200,
            x: 0,
            y,
        },
        ..key(1, 40, 10)
    };
    let boxes = [kurbo::Rect::new(0., 100., 200., 200.)];
    let lit =
        |key, kept: &Kept| rasterise(&pdf, &cache, &settings, &picker, key, &boxes, kept).unwrap();
    let kept = Kept::default();
    lit(at(0), &kept);
    let page = kept.lock().unwrap().as_ref().unwrap().1.clone();
    let panned = lit(at(10), &kept);
    let same = kept.lock().unwrap().as_ref().unwrap().1.clone();
    assert!(Arc::ptr_eq(&page, &same), "the page was drawn again");
    assert_eq!(panned, lit(at(10), &Kept::default()));
}
