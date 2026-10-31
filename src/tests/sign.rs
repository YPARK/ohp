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
    assert!((120..130).contains(&s.width()), "{}", s.width());
    assert!((10..16).contains(&s.height()), "{}", s.height());
    assert_eq!(
        s.image.get_pixel(0, 0).0,
        [0; 4],
        "a corner is paper, one colour"
    );
    let middle = s.image.get_pixel(s.width() / 2, s.height() / 2);
    assert_eq!(middle[3], 255, "the middle is ink");
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
    assert_eq!(again.image.dimensions(), s.image.dimensions());
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
    let signed = stamp(&Original::read(&pdf).unwrap(), &marks, Some(&signature)).unwrap();
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
    let original = Original::read(&fixture::pdf(1)).unwrap();
    assert!(stamp(&original, &marks, None).is_err());
}

#[test]
fn a_photo_s_paper_grain_is_not_kept() {
    // Paper with grain under the ink: once clear, one colour.
    let grain = image::RgbImage::from_fn(300, 100, |x, y| {
        let ink = (100..200).contains(&x) && (45..55).contains(&y);
        let n = ((x * 7 + y * 13) % 9) as u8;
        image::Rgb(if ink {
            [20, 20, 60]
        } else {
            [240 + n, 238 + n, 230 + n]
        })
    });
    let s = Signature::of(&DynamicImage::ImageRgb8(grain)).unwrap();
    // Compressed, the clear part is next to nothing.
    assert!(s.colour.content.len() < 2_000, "{}", s.colour.content.len());
}

#[test]
fn what_is_typed_and_the_signature_are_kept_where_asked() {
    let dir = tempfile::tempdir().unwrap();
    let at = Some(dir.path());
    assert!(typed(at).is_empty());
    remember(at, "Yongjin Park");
    remember(at, "Vancouver");
    remember(at, "Yongjin Park");
    assert_eq!(typed(at), ["Yongjin Park", "Vancouver"]);
    let s = Signature::of(&photo()).unwrap();
    let path = keep(at, &s).unwrap();
    assert_eq!(kept(at), Some(path));
    assert_eq!(kept(None), None);
}

#[test]
fn a_locked_pdf_is_not_read_to_sign() {
    assert!(Original::read(b"not a PDF").is_err());
}

#[test]
fn sizes_are_kept_each_kind_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let at = Some(dir.path());
    assert_eq!(sizes(at), Sizes::default());
    let mut s = Sizes::default();
    s.set(Kind::Signature, 1.5);
    s.set(Kind::Text, 0.8);
    keep_sizes(at, &s);
    let read = sizes(at);
    assert_eq!(read, s);
    assert_eq!(read.of(Kind::Signature), 1.5);
    assert_eq!(read.of(Kind::Text), 0.8);
    assert_eq!(read.of(Kind::Mark), 1.);
    assert_eq!(sizes(None), Sizes::default());
}

#[test]
fn sizes_are_kept_within_bounds_and_a_bad_line_is_passed_over() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("sizes"),
        "signature 100\ntext nonsense\nmark -1\nother 2\n",
    )
    .unwrap();
    let read = sizes(Some(dir.path()));
    assert_eq!(read.of(Kind::Signature), LARGEST);
    assert_eq!(read.of(Kind::Text), 1.);
    assert_eq!(read.of(Kind::Mark), SMALLEST);
}

#[test]
fn what_is_put_is_of_its_kind() {
    assert_eq!(What::Signature.kind(), Kind::Signature);
    assert_eq!(What::Text("a".into()).kind(), Kind::Text);
    assert_eq!(What::Check.kind(), Kind::Mark);
    assert_eq!(What::Dot.kind(), Kind::Mark);
}

#[test]
fn what_is_under_a_point_is_the_last_put_there() {
    let s = Signature::of(&photo()).unwrap();
    let sig = Mark::new(0, (100., 200.), What::Signature);
    let [x0, y0, x1, y1] = bounds(&sig, Some(&s));
    let w = s.width_at(1.);
    assert_eq!((x0, x1), (100., 100. + w));
    assert!(y0 < 200. && y1 > 200. && (y1 - y0 - s.height_at(w)).abs() < 0.01);
    let tick = Mark::new(0, (150., 200.), What::Check);
    let marks = [
        sig.clone(),
        tick.clone(),
        Mark::new(1, (150., 200.), What::Dot),
    ];
    // Both cover it: the last put on that page, the ✓.
    assert_eq!(under(&marks, 0, (150., 200.), Some(&s)), Some(1));
    assert_eq!(under(&marks, 0, (110., 200.), Some(&s)), Some(0));
    assert_eq!(under(&marks, 1, (110., 200.), Some(&s)), None);
    assert_eq!(under(&marks, 0, (400., 400.), Some(&s)), None);
    // Larger, it covers more.
    let big = Mark { scale: 3., ..tick };
    assert!(bounds(&big, None)[2] - bounds(&big, None)[0] > 3. * MARK_SIZE - 0.01);
}

#[test]
fn text_is_as_wide_as_its_letters() {
    let short = Mark::new(0, (0., 0.), What::Text("ab".into()));
    let long = Mark::new(0, (0., 0.), What::Text("abcdefgh".into()));
    assert!(bounds(&long, None)[2] > 3. * bounds(&short, None)[2]);
}

/// A signature `w` by `h` pixels, all ink.
fn inked(w: u32, h: u32) -> Signature {
    let ink = image::RgbImage::from_fn(w, h, |x, y| {
        let inside = (2..w - 2).contains(&x) && (2..h - 2).contains(&y);
        image::Rgb(if inside { [0; 3] } else { [255; 3] })
    });
    Signature::of(&DynamicImage::ImageRgb8(ink)).unwrap()
}

#[test]
fn a_signature_fits_a_box_its_own_shape_kept() {
    // Wide: as wide as the box; tall: as tall as it.
    let wide = inked(400, 40);
    assert!((wide.width_at(1.) - SIGNATURE_WIDTH).abs() < 0.01);
    assert!(wide.height_at(wide.width_at(1.)) <= SIGNATURE_HEIGHT + 0.01);
    let tall = inked(100, 100);
    let w = tall.width_at(1.);
    assert!(
        (tall.height_at(w) - SIGNATURE_HEIGHT).abs() < 0.5,
        "{}",
        tall.height_at(w)
    );
    assert!(tall.width_at(2.) > 1.99 * w);
}

/// A one-page PDF with a form field on it, `kind` (`Sig`, `Tx`, `Btn`), at
/// `rect` in PDF points, y up.
fn with_field(kind: &str, rect: [f32; 4]) -> Vec<u8> {
    let mut doc = Document::load_mem(&fixture::pdf(1)).unwrap();
    let field = doc.add_object(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Widget",
        "FT" => kind,
        "T" => Object::string_literal("f"),
        "Rect" => rect.iter().map(|&n| Object::Real(n)).collect::<Vec<_>>(),
    });
    let page = *doc.get_pages().get(&1).unwrap();
    doc.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Annots", vec![Object::Reference(field)]);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

#[test]
fn a_field_clicked_is_found_as_it_is_shown() {
    let (_, h) = fixture::FRAME;
    let pdf = with_field("Sig", [100., 50., 300., 90.]);
    let original = Original::read(&pdf).unwrap();
    // Shown from the top: 90 up is h - 90 down.
    let shown = field_at(&original, 0, (150., h - 70.)).unwrap();
    let want = [100., h - 90., 300., h - 50.];
    for (a, b) in shown.iter().zip(want) {
        assert!((a - b).abs() < 0.01, "{shown:?} {want:?}");
    }
    // On its line, just under it: as good as in it.
    assert!(field_at(&original, 0, (150., h - 45.)).is_some());
    assert_eq!(field_at(&original, 0, (150., h - 30.)), None);
    assert_eq!(field_at(&original, 0, (10., 10.)), None);
    assert_eq!(field_at(&original, 1, (150., h - 70.)), None);
    // A checkbox is not a field to sign or type in.
    let boxed = Original::read(&with_field("Btn", [100., 50., 300., 90.])).unwrap();
    assert_eq!(field_at(&boxed, 0, (150., h - 70.)), None);
}

#[test]
fn a_signature_put_in_a_field_fits_it() {
    let s = inked(300, 60);
    let field = [100., 50., 300., 90.];
    let (at, scale) = fit(&s, field);
    let mark = Mark {
        scale,
        ..Mark::new(0, at, What::Signature)
    };
    let [x0, y0, x1, y1] = bounds(&mark, Some(&s));
    assert!(
        x0 >= 100. && x1 <= 300. && y0 >= 50. && y1 <= 90.,
        "{x0} {y0} {x1} {y1}"
    );
    // Filling the field one way or the other.
    assert!(
        x1 - x0 > 0.8 * 200. || y1 - y0 > 0.8 * 40.,
        "{x0} {y0} {x1} {y1}"
    );
}

#[test]
fn of_fields_near_a_click_the_one_it_is_in_is_taken() {
    let (_, h) = fixture::FRAME;
    let mut doc = Document::load_mem(&with_field("Tx", [100., 120., 300., 134.])).unwrap();
    // Another, 6 points below the first, listed after it.
    let below = doc.add_object(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Widget",
        "FT" => "Tx",
        "Rect" => [100., 100., 300., 114.].iter().map(|&n| Object::Real(n)).collect::<Vec<_>>(),
    });
    let page = *doc.get_pages().get(&1).unwrap();
    let page = doc.get_object_mut(page).unwrap().as_dict_mut().unwrap();
    let mut annots = page.get(b"Annots").unwrap().as_array().unwrap().clone();
    annots.push(Object::Reference(below));
    page.set("Annots", annots);
    let mut pdf = Vec::new();
    doc.save_to(&mut pdf).unwrap();
    let original = Original::read(&pdf).unwrap();
    // In the lower one, within 8 points of the upper.
    let got = field_at(&original, 0, (150., h - 112.)).unwrap();
    assert!((got[3] - (h - 100.)).abs() < 0.01, "{got:?}");
}
