use super::*;
use crate::fixture;
use std::sync::Arc;

/// A signature: a dark stroke on white paper, as photographed.
fn photo() -> DynamicImage {
    let mut image = image::RgbImage::from_pixel(200, 80, image::Rgb([250, 248, 245]));
    for x in 40..160 {
        for y in 35..45 {
            image.put_pixel(x, y, image::Rgb([20, 20, 60]));
        }
    }
    DynamicImage::ImageRgb8(image)
}

#[test]
fn a_photo_is_cut_to_its_ink_and_its_paper_made_clear() {
    let s = Signature::of(&photo()).unwrap();
    // 120 by 10 of ink, a little room round it.
    assert!((120..130).contains(&s.width), "{}", s.width);
    assert!((10..16).contains(&s.height), "{}", s.height);
    assert_eq!(s.alpha[0], 0, "a corner is paper");
    let middle = (s.height / 2 * s.width + s.width / 2) as usize;
    assert_eq!(s.alpha[middle], 255, "the middle is ink");
}

#[test]
fn all_paper_is_no_signature() {
    let white = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(20, 20, image::Rgb([255; 3])));
    assert!(Signature::of(&white).is_err());
}

#[test]
fn a_kept_signature_reads_back_the_same() {
    let s = Signature::of(&photo()).unwrap();
    let again = Signature::of(&image::load_from_memory(&s.png().unwrap()).unwrap()).unwrap();
    assert_eq!((again.width, again.height), (s.width, s.height));
}

#[test]
fn the_copy_is_named_with_what_is_added() {
    let path = Path::new("/mail/form.pdf");
    assert_eq!(copy_of(path, ""), Path::new("/mail/form-signed.pdf"));
    assert_eq!(
        copy_of(path, "-YP signed"),
        Path::new("/mail/form-YP signed.pdf")
    );
}

#[test]
fn initials_are_each_word_s_first_letter() {
    assert_eq!(initials("yongjin  park"), "YP");
}

#[test]
fn today_is_a_date_as_forms_write_it() {
    let today = today();
    assert_eq!(today.len(), 10, "{today}");
    assert!(
        jiff::civil::Date::strptime("%Y-%m-%d", &today).is_ok(),
        "{today}"
    );
}

#[test]
fn text_helvetica_lacks_is_said() {
    assert!(settable("Yongjin Park – 30/10/2026, café"));
    assert!(!settable("박용진"));
    assert_eq!(pdf_string("a(b)\\"), b"(a\\(b\\)\\\\)");
}

#[test]
fn a_turned_page_puts_what_is_shown_where_it_is_shown() {
    let crop = [0., 0., 600., 800.];
    // Unturned: the top left shown is the top left of the PDF.
    assert_eq!(View::new(crop, 0).at((0., 0.)), (0., 800.));
    // Turned a quarter clockwise, its bottom left comes to the top left,
    // shown 800 wide; the right shown is up the PDF.
    let turned = View::new(crop, 90);
    assert_eq!(turned.at((0., 0.)), (0., 0.));
    assert_eq!(turned.at((800., 0.)), (0., 800.));
    assert_eq!(turned.at((0., 600.)), (600., 0.));
    assert_eq!(View::new(crop, 180).at((0., 0.)), (600., 0.));
    assert_eq!(View::new(crop, -90).at((0., 0.)), (600., 800.));
}

#[test]
fn marks_are_appended_and_the_original_kept_whole() {
    let pdf = fixture::pdf(2);
    let signature = Signature::of(&photo()).unwrap();
    let marks = [
        Mark::new(1, (100., 100.), What::Signature),
        Mark::new(1, (100., 140.), What::Text("Yongjin Park".into())),
        Mark::new(0, (50., 50.), What::Check),
    ];
    let signed = stamp(&pdf, &marks, Some(&signature)).unwrap();
    assert!(signed.starts_with(&pdf), "an incremental update");
    let doc = Document::load_mem(&signed).unwrap();
    let pages = doc.get_pages();
    let page = doc.get_dictionary(pages[&2]).unwrap();
    // The page's own content, between what saves and restores the state.
    let contents = page.get(b"Contents").unwrap().as_array().unwrap();
    assert_eq!(contents.len(), 3);
    let resources = page.get(b"Resources").unwrap().as_dict().unwrap();
    assert!(
        resources
            .get(b"Font")
            .unwrap()
            .as_dict()
            .unwrap()
            .has(b"F1"),
        "its own font kept"
    );
    let content = doc.get_page_content(pages[&2]);
    let content = String::from_utf8_lossy(&content);
    assert!(content.contains("/OhpSignature Do"), "{content}");
    assert!(content.contains("(Yongjin Park) Tj"), "{content}");
    // Still read, and drawn, as a PDF.
    let pdf = hayro::hayro_syntax::Pdf::new(Arc::new(signed)).unwrap();
    assert_eq!(pdf.pages().len(), 2);
}

#[test]
fn a_signature_is_needed_to_put_one() {
    let marks = [Mark::new(0, (1., 1.), What::Signature)];
    assert!(stamp(&fixture::pdf(1), &marks, None).is_err());
}
