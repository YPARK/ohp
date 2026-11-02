use super::*;
use crate::fixture::{FRAME, pdf};
use hayro::hayro_syntax::Pdf;
use std::sync::Arc;

/// A PDF of `pages` pages each a 16:9 slide with its note beside it.
fn noted_pdf(pages: usize) -> Vec<u8> {
    let (w, h) = FRAME;
    let text = String::from_utf8(pdf(pages)).unwrap();
    let wide = text.replace(
        &format!("/MediaBox [0 0 {w} {h}]"),
        &format!("/MediaBox [0 0 {} {h}]", 2. * w),
    );
    // The xref's offsets hold: the box is written as long as before, or
    // lopdf reads past them by its own repair.
    wide.into_bytes()
}

fn sizes(data: Vec<u8>) -> Vec<(f32, f32)> {
    crate::render::sizes(&Pdf::new(Arc::new(data)).unwrap())
}

#[test]
fn a_page_twice_a_slide_wide_is_noted() {
    let (w, h) = FRAME;
    assert!(noted(&[(2. * w, h), (2. * w, h)]));
    assert!(noted(&[(2. * 400., 300.)]));
    assert!(!noted(&[(w, h)]));
    assert!(!noted(&[(2. * w, h), (w, h)]));
    assert!(!noted(&[]));
}

#[test]
fn each_page_is_cut_into_its_slide_and_its_note() {
    let (w, h) = FRAME;
    let data = noted_pdf(3);
    assert!(noted(&sizes(data.clone())));
    let cut = split(&data).unwrap();
    let close = |(a, b): (f32, f32)| (a - w).abs() < 0.1 && (b - h).abs() < 0.1;
    let slides = sizes(cut.slides);
    assert_eq!(slides.len(), 3);
    assert!(slides.iter().copied().all(close), "{slides:?}");
    let both = sizes(cut.both);
    assert_eq!(both.len(), 6);
    assert!(both.iter().copied().all(close), "{both:?}");
}
