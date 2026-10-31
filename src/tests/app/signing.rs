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
    press(&mut app, &[KeyCode::Char('S')]);
    app.signing.as_mut().unwrap().kept_in = Some(dir.path().join("kept"));
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
    assert_eq!(app.notice.as_deref(), Some("2 put · u takes it back"));
}

#[test]
fn the_last_put_is_taken_back_resized_and_moved() {
    let (_dir, mut app) = signing();
    click(&mut app, 20, 10);
    press(&mut app, &[KeyCode::Char('C')]);
    click(&mut app, 30, 10);
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
fn text_is_typed_where_it_was_clicked_and_put_on_enter() {
    let (_dir, mut app) = signing();
    press(&mut app, &[KeyCode::Char('T')]);
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
    assert_eq!(app.notice.as_deref(), Some("nothing signed"));
}

#[test]
fn only_a_pdf_is_signed() {
    let (_dir, mut app) = app(1);
    app.links = vec![];
    app.deck.paths.push(PathBuf::from("more.pdf"));
    press(&mut app, &[KeyCode::Char('S')]);
    assert!(app.signing.is_none());
}
