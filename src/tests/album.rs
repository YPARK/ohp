use super::*;
use crate::fixture::{self, FRAME};
use crate::render::Deck;
use crate::text::{self, Spoken};
use image::{Rgb, RgbImage};

/// A `w` × `h` image of one colour, written to `name` in `dir` in the
/// format its extension names.
fn picture(dir: &Path, name: &str, w: u32, h: u32) -> PathBuf {
    let path = dir.join(name);
    RgbImage::from_pixel(w, h, Rgb([200, 30, 30]))
        .save(&path)
        .unwrap();
    path
}

fn shape((w, h): (f32, f32)) -> f32 {
    w / h
}

#[test]
fn pdfs_and_images_mixed_are_one_deck_each_page_its_shape() {
    let dir = tempfile::tempdir().unwrap();
    let paths = [
        picture(dir.path(), "tall.png", 30, 60),
        fixture::write(dir.path(), 3),
        picture(dir.path(), "wide.jpg", 80, 40),
    ];
    let deck = Deck::open_with(&paths, Options::default()).unwrap();
    assert_eq!(deck.pages, 5);
    assert_eq!(deck.from, [0, 1, 1, 1, 2]);
    assert!(deck.note.is_none(), "{:?}", deck.note);
    let shapes: Vec<f32> = deck.sizes.iter().copied().map(shape).collect();
    let want = [0.5, shape(FRAME), shape(FRAME), shape(FRAME), 2.];
    for (got, want) in shapes.iter().zip(want) {
        assert!((got - want).abs() < 0.01, "{shapes:?}");
    }
}

#[test]
fn an_image_fills_its_page() {
    let dir = tempfile::tempdir().unwrap();
    let paths = [
        fixture::write(dir.path(), 1),
        picture(dir.path(), "red.png", 40, 20),
    ];
    let deck = Deck::open_with(&paths, Options::default()).unwrap();
    let pdf = Pdf::new(deck.data.clone()).unwrap();
    let cache = hayro::RenderCache::new();
    let settings = hayro::hayro_interpret::InterpreterSettings::default();
    let image = render::raster(&pdf.pages()[1], &cache, &settings, 0.1, 0.1).unwrap();
    let (w, h) = image.dimensions();
    for (x, y) in [(1, 1), (w / 2, h / 2), (w - 2, h - 2)] {
        let [r, g, b, _] = image.get_pixel(x, y).0;
        assert!(r > 180 && g < 60 && b < 60, "({x}, {y}) is {r},{g},{b}");
    }
}

#[test]
fn one_image_is_a_deck_of_a_slide() {
    let dir = tempfile::tempdir().unwrap();
    let deck = Deck::open(&picture(dir.path(), "photo.png", 40, 30)).unwrap();
    assert_eq!(deck.pages, 1);
    assert!((shape(deck.sizes[0]) - 4. / 3.).abs() < 0.01);
}

#[test]
fn a_photo_larger_than_a_screen_is_made_smaller_keeping_its_shape() {
    let dir = tempfile::tempdir().unwrap();
    let path = picture(dir.path(), "huge.png", 5000, 100);
    let shown = show(&path, &Options::default()).unwrap();
    let Kind::Pixels { width, height, .. } = shown.kind else {
        panic!("not made smaller");
    };
    assert_eq!(width, IMAGE_PIXELS);
    assert!(height.abs_diff(82) <= 1, "{height}");
    let deck = Deck::open(&path).unwrap();
    assert!((shape(deck.sizes[0]) - 50.).abs() < 1., "{:?}", deck.sizes);
}

#[test]
fn a_drawing_is_set_its_own_shape() {
    let dir = tempfile::tempdir().unwrap();
    let svg = fixture::write_text(
        dir.path(),
        "plot.svg",
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="300" height="100"><rect width="300" height="100" fill="red"/></svg>"#,
    );
    let paths = [svg, fixture::write(dir.path(), 1)];
    let deck = Deck::open_with(&paths, Options::default()).unwrap();
    assert_eq!(deck.from, [0, 1]);
    assert!((shape(deck.sizes[0]) - 3.).abs() < 0.01, "{:?}", deck.sizes);
}

#[test]
fn an_image_typst_cannot_decode_is_left_out_and_the_rest_set() {
    let dir = tempfile::tempdir().unwrap();
    let whole = picture(dir.path(), "whole.png", 300, 300);
    let bytes = std::fs::read(&whole).unwrap();
    // Its header is whole, so it is measured, but not its pixels.
    let cut = dir.path().join("cut.png");
    std::fs::write(&cut, &bytes[..bytes.len() / 2]).unwrap();
    let paths = [cut, fixture::write(dir.path(), 2)];
    let deck = Deck::open_with(&paths, Options::default()).unwrap();
    assert_eq!(deck.from, [1, 1]);
    let note = deck.note.unwrap();
    assert!(note.starts_with("cannot show cut.png"), "{note}");
}

#[test]
fn a_file_unchanged_is_not_read_again() {
    let dir = tempfile::tempdir().unwrap();
    let path = picture(dir.path(), "a.png", 10, 10);
    let options = Options::default();
    let first = options.cache.show(&path, &options).unwrap();
    assert!(Arc::ptr_eq(
        &first,
        &options.cache.show(&path, &options).unwrap()
    ));

    picture(dir.path(), "a.png", 20, 10);
    let changed = options.cache.show(&path, &options).unwrap();
    assert!(!Arc::ptr_eq(&first, &changed));

    options.forget();
    assert!(!Arc::ptr_eq(
        &changed,
        &options.cache.show(&path, &options).unwrap()
    ));

    options.cache.keep(&[]);
    assert!(options.cache.0.lock().unwrap().is_empty());
}

#[test]
fn a_pdf_set_among_others_is_still_read_aloud() {
    let dir = tempfile::tempdir().unwrap();
    let paths = [
        picture(dir.path(), "a.png", 10, 10),
        fixture::write_pdf(dir.path(), &fixture::hello()),
    ];
    let deck = Deck::open_with(&paths, Options::default()).unwrap();
    let page = text::of(deck.data.clone(), 1).unwrap();
    assert_eq!(page.spoken(), [Spoken::Text("Hello".into())]);
    assert!(text::of(deck.data.clone(), 0).unwrap().spoken().is_empty());
}

#[test]
fn a_file_that_cannot_be_read_is_left_out_and_named() {
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad.png");
    std::fs::write(&bad, b"not a png").unwrap();
    let paths = [bad, fixture::write(dir.path(), 2)];
    let deck = Deck::open_with(&paths, Options::default()).unwrap();
    assert_eq!(deck.pages, 2);
    let note = deck.note.unwrap();
    assert!(note.starts_with("cannot show bad.png"), "{note}");
}

#[test]
fn no_file_read_is_no_deck() {
    let dir = tempfile::tempdir().unwrap();
    let paths = [dir.path().join("gone.png"), dir.path().join("gone.pdf")];
    let why = Deck::open_with(&paths, Options::default()).err().unwrap();
    assert!(why.to_string().contains("cannot show gone.pdf"), "{why}");
}

#[test]
fn markdown_is_typeset_among_the_others() {
    let dir = tempfile::tempdir().unwrap();
    let md = fixture::write_text(dir.path(), "notes.md", "Some notes.\n");
    let paths = [md, picture(dir.path(), "fig.gif", 20, 20)];
    let deck = Deck::open_with(&paths, Options::default()).unwrap();
    assert_eq!(deck.pages, 2);
    let (w, h) = deck.sizes[0];
    assert!((w - 612.).abs() < 1. && (h - 792.).abs() < 1., "{w}×{h}");
}

#[test]
fn a_stopped_gathering_ends_without_a_pdf() {
    let dir = tempfile::tempdir().unwrap();
    let paths = [picture(dir.path(), "a.png", 10, 10)];
    let stopped = Options::default();
    stopped.stop.store(true, Ordering::Relaxed);
    let why = gather(&paths, &stopped).err().unwrap();
    assert!(why.is::<Stopped>(), "{why}");
}
