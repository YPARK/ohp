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
        let key = Key {
            page: 1,
            cols,
            rows,
        };
        let image = rasterise(&pdf, &cache, &InterpreterSettings::default(), &picker, key).unwrap();
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
    let key = Key {
        page: 2,
        cols: 40,
        rows: 10,
    };
    let settings = InterpreterSettings::default();
    assert!(
        rasterise(
            &pdf,
            &RenderCache::new(),
            &settings,
            &Picker::halfblocks(),
            key
        )
        .is_none()
    );
}

/// A job for page `page` in a `cols` × `rows` box.
fn job(page: usize, cols: u16, rows: u16, look: Look) -> Job {
    let key = Key { page, cols, rows };
    Job { key, look }
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
