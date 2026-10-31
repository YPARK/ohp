//! Signing the PDF presented: what a click puts, taken back, resized and
//! moved, and the copy named once done.

use super::*;
use crate::sign::{Signature, What};

/// The deck's one page, as drawn on the 140×39 screen: 139 by 39 cells.
const PAGE: Rect = Rect::new(0, 0, 139, 39);

/// An app signing a one-page deck, with a signature to put, keeping what
/// it keeps in its own folder.
fn signing() -> (tempfile::TempDir, App) {
    let (dir, mut app) = app(1);
    press(&mut app, &[KeyCode::Char('A')]);
    let s = app.signing.as_mut().unwrap();
    s.kept_in = Some(dir.path().join("kept"));
    // Not the sizes kept where ohp runs.
    s.sizes = crate::sign::Sizes::default();
    let ink = image::RgbImage::from_fn(40, 10, |x, _| {
        image::Rgb(if (5..35).contains(&x) {
            [0; 3]
        } else {
            [255; 3]
        })
    });
    let signature = Signature::of(&image::DynamicImage::ImageRgb8(ink)).unwrap();
    app.signing.as_mut().unwrap().signature = Some(signature);
    (dir, app)
}

/// A click at `(x, y)` on the page drawn.
fn click(app: &mut App, x: u16, y: u16) {
    app.drawn.set(Some((app.job(app.presented()), PAGE)));
    app.mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    });
    finish(app);
}

fn marks(app: &App) -> Vec<crate::sign::Mark> {
    app.signing.as_ref().unwrap().marks.clone()
}

#[test]
fn a_click_puts_the_signature_in_a_copy_shown_in_place() {
    let (dir, mut app) = signing();
    let original = std::fs::read(dir.path().join("deck.pdf")).unwrap();
    click(&mut app, 69, 19);
    let put = marks(&app);
    assert_eq!(put.len(), 1);
    assert_eq!(put[0].what, What::Signature);
    // The middle of the page, in points from its top left.
    let (x, y) = put[0].at;
    assert!((x - 227.).abs() < 3. && (y - 127.).abs() < 6., "{x} {y}");
    let copy = dir.path().join("deck-signed.pdf");
    assert_eq!(app.deck.paths, std::slice::from_ref(&copy));
    assert!(std::fs::read(&copy).unwrap().starts_with(&original));
    assert_eq!(
        std::fs::read(dir.path().join("deck.pdf")).unwrap(),
        original
    );
    // Read as it was written: no change on disk to read again.
    watch(&mut app);
    assert!(app.loading.is_none() && app.settling.is_none());
    // Nor is the next written, while it is read.
    click(&mut app, 30, 10);
    due(&mut app);
    app.watch();
    assert_eq!(
        app.notice.as_deref(),
        Some("click it to move or size it · u takes it back")
    );
}

#[test]
fn the_last_put_is_taken_back_resized_and_moved() {
    let (_dir, mut app) = signing();
    click(&mut app, 20, 10);
    press(&mut app, &[KeyCode::Char('4')]);
    click(&mut app, 100, 30);
    assert_eq!(marks(&app)[1].what, What::Check);
    let at = marks(&app)[1].at;
    press(&mut app, &[KeyCode::Char(']')]);
    app.key(KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT));
    finish(&mut app);
    let last = marks(&app)[1].clone();
    assert!(last.scale > 1.);
    assert_eq!(last.at, (at.0 + 2., at.1));
    press(&mut app, &[KeyCode::Char('u')]);
    finish(&mut app);
    assert_eq!(marks(&app).len(), 1);
}

#[test]
fn a_size_set_is_kept_for_the_next_of_its_kind_and_next_time() {
    let (dir, mut app) = signing();
    // Before anything is put: the size of what is picked.
    press(&mut app, &[KeyCode::Char(']')]);
    click(&mut app, 20, 10);
    let first = marks(&app)[0].scale;
    assert!(first > 1., "{first}");
    press(&mut app, &[KeyCode::Char(']')]);
    finish(&mut app);
    let larger = marks(&app)[0].scale;
    assert!(larger > first, "{larger}");
    // The next signature as large; a ✓ its own size.
    click(&mut app, 40, 25);
    assert_eq!(marks(&app)[1].scale, larger);
    press(&mut app, &[KeyCode::Char('4')]);
    click(&mut app, 120, 35);
    assert_eq!(marks(&app)[2].scale, 1.);
    let (doing, _) = app.sign_status().unwrap();
    assert!(doing.contains("100%"), "{doing}");
    // Signing again, the sizes are as they were left.
    let kept = crate::sign::sizes(Some(&dir.path().join("kept")));
    assert_eq!(kept.of(crate::sign::Kind::Signature), larger);
}

#[test]
fn text_is_typed_where_it_was_clicked_and_put_on_enter() {
    let (_dir, mut app) = signing();
    press(&mut app, &[KeyCode::Char('2')]);
    click(&mut app, 20, 10);
    assert!(marks(&app).is_empty(), "put only once typed");
    // Nothing typed before: begun empty, tab offering the date, picked.
    assert_eq!(typing_text(&app), "");
    press(&mut app, &[KeyCode::Tab]);
    assert_eq!(typing_text(&app), crate::sign::today());
    // Typing is the text's, not signing's: u is a letter, typed over
    // what is offered.
    press(&mut app, &[KeyCode::Char('u')]);
    assert_eq!(typing_text(&app), "u");
    press(&mut app, &[KeyCode::Backspace]);
    for c in "Yongjin Park".chars() {
        press(&mut app, &[KeyCode::Char(c)]);
    }
    press(&mut app, &[KeyCode::Enter]);
    finish(&mut app);
    assert_eq!(marks(&app)[0].what, What::Text("Yongjin Park".into()));
    // Next time it is offered first, picked, its initials after it.
    click(&mut app, 20, 14);
    assert_eq!(typing_text(&app), "Yongjin Park");
    press(&mut app, &[KeyCode::Tab]);
    assert_eq!(typing_text(&app), "YP");
    // Alt and a letter is no letter typed.
    app.key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT));
    assert_eq!(typing_text(&app), "YP");
}

fn typing_text(app: &App) -> String {
    let s = app.signing.as_ref().unwrap();
    s.typing.as_ref().unwrap().field.text.clone()
}

#[test]
fn done_asks_the_copy_s_name_its_signed_picked_to_type_over() {
    let (dir, mut app) = signing();
    click(&mut app, 20, 10);
    press(&mut app, &[KeyCode::Esc]);
    let naming = app.signing.as_ref().unwrap().naming.clone().unwrap();
    assert_eq!(naming.shown(), "deck-[signed].pdf");
    for c in "YP signed".chars() {
        press(&mut app, &[KeyCode::Char(c)]);
    }
    press(&mut app, &[KeyCode::Enter]);
    finish(&mut app);
    let kept = dir.path().join("deck-YP signed.pdf");
    assert!(app.signing.is_none());
    assert!(kept.is_file() && !dir.path().join("deck-signed.pdf").exists());
    assert_eq!(app.deck.paths, std::slice::from_ref(&kept));
    assert_eq!(app.notice, Some(format!("signed: {}", kept.display())));
}

#[test]
fn a_signature_picked_is_put_where_clicked_and_kept() {
    let (dir, mut app) = signing();
    app.signing.as_mut().unwrap().signature = None;
    let png = dir.path().join("mine.png");
    let ink = image::RgbImage::from_fn(40, 10, |x, _| {
        image::Rgb(if (5..35).contains(&x) {
            [0; 3]
        } else {
            [255; 3]
        })
    });
    ink.save(&png).unwrap();
    // None kept: a click asks for one, and waits for it.
    click(&mut app, 20, 10);
    assert!(app.browser.is_some() && app.picking_signature());
    app.picked_signature(Some(vec![png]));
    finish(&mut app);
    assert_eq!(marks(&app).len(), 1, "put where it was clicked");
    let notice = app.notice.clone().unwrap();
    assert!(
        notice.starts_with("signature kept for next time"),
        "{notice}"
    );
    assert!(dir.path().join("kept/signature.png").is_file());
}

#[test]
fn esc_with_nothing_put_stops_signing() {
    let (_dir, mut app) = signing();
    press(&mut app, &[KeyCode::Esc]);
    assert!(app.signing.is_none());
    assert_eq!(app.notice.as_deref(), Some("nothing put"));
}

#[test]
fn only_a_pdf_is_signed() {
    let (_dir, mut app) = app(1);
    app.links = vec![];
    app.deck.paths.push(PathBuf::from("more.pdf"));
    press(&mut app, &[KeyCode::Char('A')]);
    assert!(app.signing.is_none());
}

#[test]
fn p_while_signing_prints_the_copy_with_what_is_put() {
    let (dir, mut app) = signing();
    click(&mut app, 20, 10);
    press(&mut app, &[KeyCode::Char('P')]);
    let p = app.printing.as_mut().expect("the print line open");
    p.lookup = None;
    p.printers = crate::print::Printers::default();
    p.programs = vec![crate::fixture::fake_lp(dir.path()).into()];
    // As if the copy last written were not yet read again.
    let original = std::fs::read(dir.path().join("deck.pdf")).unwrap();
    app.deck.data = Arc::new(original.clone());
    press(&mut app, &[KeyCode::Enter]);
    super::printing::sent(&mut app);
    let printed = std::fs::read(dir.path().join("printed.pdf")).unwrap();
    assert_ne!(printed, original, "printed as annotated, not as last read");
    assert_eq!(
        printed,
        std::fs::read(dir.path().join("deck-signed.pdf")).unwrap()
    );
    // Still signing.
    assert!(app.signing());
}

/// A press, a drag or a let go, `kind`, at `(x, y)` on the page drawn.
fn mouse(app: &mut App, kind: MouseEventKind, x: u16, y: u16) {
    app.drawn.set(Some((app.job(app.presented()), PAGE)));
    app.mouse(MouseEvent {
        kind,
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    });
    finish(app);
}

#[test]
fn what_is_put_is_dragged_to_where_it_is_let_go() {
    let (dir, mut app) = signing();
    click(&mut app, 20, 10);
    let copy = dir.path().join("deck-signed.pdf");
    let written = std::fs::read(&copy).unwrap();
    let at = marks(&app)[0].at;
    let left = MouseEventKind::Down(MouseButton::Left);
    mouse(&mut app, left, 22, 10);
    // Picked, not another put.
    assert_eq!(marks(&app).len(), 1);
    mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), 32, 14);
    assert!(app.dragged().is_some(), "outlined while dragged");
    // Not written until let go.
    assert_eq!(std::fs::read(&copy).unwrap(), written);
    mouse(&mut app, MouseEventKind::Up(MouseButton::Left), 32, 14);
    let moved = marks(&app)[0].at;
    assert!(
        moved.0 > at.0 + 20. && moved.1 > at.1 + 10.,
        "{at:?} {moved:?}"
    );
    assert_ne!(std::fs::read(&copy).unwrap(), written);
    assert!(app.dragged().is_none());
    assert_eq!(marks(&app).len(), 1);
}

#[test]
fn the_wheel_sizes_what_it_is_over_and_turns_pages_elsewhere() {
    let (_dir, mut app) = signing();
    click(&mut app, 20, 10);
    press(&mut app, &[KeyCode::Char('4')]);
    click(&mut app, 100, 30);
    // Over the signature: it picked, and larger.
    mouse(&mut app, MouseEventKind::ScrollUp, 22, 10);
    let put = marks(&app);
    assert_eq!(put.last().unwrap().what, What::Signature);
    assert!(put.last().unwrap().scale > 1.);
    assert_eq!(put[0].scale, 1., "the ✓ as it was");
    mouse(&mut app, MouseEventKind::ScrollDown, 22, 10);
    mouse(&mut app, MouseEventKind::ScrollDown, 22, 10);
    assert!(marks(&app).last().unwrap().scale < 1.);
}

#[test]
fn a_tool_is_picked_by_its_number_or_its_row_in_the_palette() {
    let (_dir, mut app) = signing();
    press(&mut app, &[KeyCode::Char('5')]);
    assert_eq!(
        app.signing.as_ref().unwrap().tool,
        crate::app::signing::Tool::Cross
    );
    app.side.set(Rect::new(110, 0, 30, 39));
    // Past the border, the hint and a blank line: the 4th tool, ✓.
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), 115, 6);
    assert_eq!(
        app.signing.as_ref().unwrap().tool,
        crate::app::signing::Tool::Check
    );
    assert!(
        marks(&app).is_empty(),
        "a click on the palette puts nothing"
    );
    assert!(app.side_open());
}

#[test]
fn a_signature_put_in_a_form_field_fits_it() {
    use lopdf::dictionary;
    let (_, h) = fixture::FRAME;
    // A field across the middle of the page, 40 points tall.
    let mut doc = lopdf::Document::load_mem(&fixture::pdf(1)).unwrap();
    let rect = [100., h / 2. - 20., 350., h / 2. + 20.];
    let field = doc.add_object(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Widget",
        "FT" => "Sig",
        "Rect" => rect.iter().map(|&n| lopdf::Object::Real(n)).collect::<Vec<_>>(),
    });
    let page = *doc.get_pages().get(&1).unwrap();
    let page = doc.get_object_mut(page).unwrap().as_dict_mut().unwrap();
    page.set("Annots", vec![lopdf::Object::Reference(field)]);
    let mut pdf = Vec::new();
    doc.save_to(&mut pdf).unwrap();
    let (_dir, mut app) = app_with(&pdf, Look::Image);
    press(&mut app, &[KeyCode::Char('A')]);
    let ink = image::RgbImage::from_fn(200, 40, |x, y| {
        image::Rgb(if (4..196).contains(&x) && (4..36).contains(&y) {
            [0; 3]
        } else {
            [255; 3]
        })
    });
    let s = app.signing.as_mut().unwrap();
    s.signature = Some(Signature::of(&image::DynamicImage::ImageRgb8(ink)).unwrap());
    s.sizes = crate::sign::Sizes::default();
    s.kept_in = None;
    // The middle of the page: in the field.
    click(&mut app, 69, 19);
    let put = &marks(&app)[0];
    let [x0, y0, x1, y1] =
        crate::sign::bounds(put, app.signing.as_ref().unwrap().signature.as_ref());
    assert!(x0 >= 100. && x1 <= 350. && y0 >= h / 2. - 20. && y1 <= h / 2. + 20.);
    assert!(
        (x0 - 100.).abs() < 10.,
        "at the field's left edge, not the click: {x0}"
    );
}

#[test]
fn the_wheel_sizes_at_once_and_writes_the_copy_once_it_rests() {
    let (dir, mut app) = signing();
    click(&mut app, 20, 10);
    let copy = dir.path().join("deck-signed.pdf");
    let written = std::fs::read(&copy).unwrap();
    for _ in 0..3 {
        mouse(&mut app, MouseEventKind::ScrollUp, 22, 10);
    }
    assert!(marks(&app)[0].scale > 1.4);
    assert_eq!(std::fs::read(&copy).unwrap(), written, "not yet written");
    app.settle_wheel(true);
    finish(&mut app);
    assert_ne!(std::fs::read(&copy).unwrap(), written);
}

#[test]
fn the_preview_is_not_a_page_to_put_on() {
    let (_dir, mut app) = signing();
    press(&mut app, &[KeyCode::Char('P')]);
    app.printing.as_mut().unwrap().lookup = None;
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), 69, 19);
    mouse(&mut app, MouseEventKind::ScrollUp, 69, 19);
    assert!(marks(&app).is_empty());
}

#[test]
fn ctrl_s_while_typing_text_is_typing_s_place() {
    let (_dir, mut app) = signing();
    press(&mut app, &[KeyCode::Char('2')]);
    click(&mut app, 20, 10);
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    press(&mut app, &[KeyCode::Char('x')]);
    assert!(app.search.is_none());
    assert_eq!(typing_text(&app), "x");
}

#[test]
fn a_mark_a_cell_tall_is_outlined_as_it_is_dragged() {
    let (_dir, app) = signing();
    app.drawn.set(Some((app.job(app.presented()), PAGE)));
    // Two points square: within one cell each way.
    assert!(app.cells_of(0, [100., 100., 102., 102.]).is_some());
}

#[test]
fn a_again_while_annotating_keeps_what_is_put() {
    let (dir, mut app) = signing();
    click(&mut app, 20, 10);
    press(&mut app, &[KeyCode::Char('A')]);
    let s = app.signing.as_ref().unwrap();
    assert_eq!(s.marks.len(), 1, "still the one put, to move or take back");
    assert_eq!(s.copy, dir.path().join("deck-signed.pdf"));
}

#[test]
fn a_box_off_what_is_shown_is_not_outlined() {
    let (_dir, app) = signing();
    app.drawn.set(Some((app.job(app.presented()), PAGE)));
    assert!(app.cells_of(0, [-50., 10., -20., 20.]).is_none());
    assert!(app.cells_of(0, [100., 2000., 120., 2010.]).is_none());
}

#[test]
fn done_just_after_the_wheel_keeps_the_copy_as_sized() {
    let (dir, mut app) = signing();
    click(&mut app, 20, 10);
    mouse(&mut app, MouseEventKind::ScrollUp, 22, 10);
    let s = app.signing.as_ref().unwrap();
    let want = crate::sign::stamp(&s.original, &s.marks, s.signature.as_ref()).unwrap();
    press(&mut app, &[KeyCode::Enter, KeyCode::Enter]);
    finish(&mut app);
    let kept = dir.path().join("deck-signed.pdf");
    assert_eq!(std::fs::read(&kept).unwrap(), want);
    // Shown as written, and its size kept for next time.
    assert_eq!(*app.deck.data, want);
    let sizes = crate::sign::sizes(Some(&dir.path().join("kept")));
    assert!(sizes.of(crate::sign::Kind::Signature) > 1.);
    assert!(
        app.notice.as_deref().unwrap_or("").starts_with("signed:"),
        "{:?}",
        app.notice
    );
}

#[test]
fn another_file_opened_while_annotating_ends_annotating() {
    let (dir, mut app) = signing();
    click(&mut app, 20, 10);
    std::fs::create_dir(dir.path().join("other")).unwrap();
    let other = fixture::write_pdf(&dir.path().join("other"), &fixture::pdf(2));
    app.open(vec![other]);
    assert!(app.signing.is_none());
    finish(&mut app);
    assert_eq!(app.deck.pages, 2);
}
