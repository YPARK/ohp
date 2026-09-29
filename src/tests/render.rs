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
        let key = Key { page: 1, cols, rows };
        let image = rasterise(&pdf, &cache, &InterpreterSettings::default(), &picker, key).unwrap();
        let (bw, bh) = (u32::from(cols * font.width), u32::from(rows * font.height));
        assert!(image.width() <= bw && image.height() <= bh, "{cols}x{rows}");
        assert!(bw - image.width() <= 1 || bh - image.height() <= 1, "{cols}x{rows}: not fitted");
    }
}

#[test]
fn a_page_past_the_end_renders_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let deck = Deck::open(&fixture::write(dir.path(), 2)).unwrap();
    let pdf = Pdf::new(deck.data.clone()).unwrap();
    let key = Key { page: 2, cols: 40, rows: 10 };
    let settings = InterpreterSettings::default();
    assert!(rasterise(&pdf, &RenderCache::new(), &settings, &Picker::halfblocks(), key).is_none());
}

#[test]
fn workers_deliver_every_requested_slide() {
    let dir = tempfile::tempdir().unwrap();
    let deck = Deck::open(&fixture::write(dir.path(), 4)).unwrap();
    let renderer = Renderer::spawn(&deck, &Picker::halfblocks(), 2).unwrap();
    let keys: Vec<Key> = (0..4).map(|page| Key { page, cols: 30, rows: 8 }).collect();
    renderer.push(keys.clone());

    let mut got: Vec<Key> = (0..keys.len())
        .map(|_| {
            let done = renderer.done.recv_timeout(Duration::from_secs(10)).expect("a render");
            assert!(done.slide.is_some(), "page {} failed", done.key.page);
            done.key
        })
        .collect();
    got.sort_by_key(|k| k.page);
    assert_eq!(got, keys);
}

#[test]
fn cleared_jobs_are_not_rendered() {
    let dir = tempfile::tempdir().unwrap();
    let deck = Deck::open(&fixture::write(dir.path(), 3)).unwrap();
    let renderer = Renderer::spawn(&deck, &Picker::halfblocks(), 1).unwrap();
    // Let the worker reach its wait; jobs queued without waking it then stay queued.
    std::thread::sleep(Duration::from_millis(100));
    let (lock, _) = &*renderer.queue;
    lock.lock().unwrap().keys.extend((0..3).map(|page| Key { page, cols: 30, rows: 8 }));
    assert_eq!(renderer.clear().len(), 3);
    assert!(renderer.done.recv_timeout(Duration::from_millis(200)).is_err());
}
