//! Searching the slides, as Emacs's isearch does: typed, stepped through
//! with ctrl-s and ctrl-r round the end, stayed at or gone back from.

use super::*;
use crate::app::search::place;

fn ctrl(app: &mut App, c: char) {
    app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
}

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        app.key(KeyEvent::from(KeyCode::Char(c)));
    }
}

/// Read the slides' text for searching, and wait for it.
fn read(app: &mut App) {
    app.read_slides();
    let start = Instant::now();
    while app.texts.is_none() {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "the slides were not read"
        );
        std::thread::sleep(Duration::from_millis(5));
        app.read_texts();
    }
}

/// An app on four slides, saying what each does: "beta" on the 2nd twice
/// and the 4th once, their text read to search.
fn slides() -> (tempfile::TempDir, App) {
    let pdf = fixture::with(
        &["alpha", "beta and Beta", "gamma", "beta"].map(|t| fixture::text(40., 150., 12., t)),
    );
    let (dir, mut app) = app_with(&pdf, Look::Text);
    read(&mut app);
    (dir, app)
}

fn status(app: &App) -> String {
    app.search_status().unwrap().0
}

#[test]
fn places_are_stepped_through_round_the_end() {
    let counts = [0, 2, 0, 1];
    assert_eq!(place(&counts, (0, 0), true, true), Some((1, 0)));
    assert_eq!(place(&counts, (1, 0), true, false), Some((1, 1)));
    assert_eq!(place(&counts, (3, 0), true, false), Some((1, 0)));
    assert_eq!(place(&counts, (1, 0), false, false), Some((3, 0)));
    assert_eq!(place(&counts, (2, 0), false, true), Some((1, 1)));
    assert_eq!(place(&[0, 0], (0, 0), true, true), None);
}

#[test]
fn typing_goes_to_the_first_place_and_ctrl_s_to_the_next() {
    let (_dir, mut app) = slides();
    ctrl(&mut app, 's');
    assert!(app.search.is_some());
    typed(&mut app, "beta");
    assert_eq!(app.cur, 1);
    assert!(status(&app).ends_with("1 of 3"), "{}", status(&app));
    // Lit on the slide gone to, the place gone to brighter.
    assert_eq!(app.job(app.presented()).found.unwrap().current, Some(0));
    assert!(app.found(0).is_none() && app.found(2).is_none());
    ctrl(&mut app, 's');
    assert_eq!(
        (app.cur, app.search.as_ref().unwrap().at),
        (1, Some((1, 1)))
    );
    ctrl(&mut app, 's');
    assert_eq!(app.cur, 3);
    // Round the end, as said.
    ctrl(&mut app, 's');
    assert_eq!(app.cur, 1);
    assert!(status(&app).starts_with("wrapped"), "{}", status(&app));
    ctrl(&mut app, 'r');
    assert_eq!(app.cur, 3);
}

#[test]
fn ctrl_g_goes_back_and_enter_stays() {
    let (_dir, mut app) = slides();
    ctrl(&mut app, 's');
    typed(&mut app, "gamma");
    assert_eq!(app.cur, 2);
    ctrl(&mut app, 'g');
    assert!(app.search.is_none());
    assert_eq!(app.cur, 0);
    // Gone back from, it is kept to search for again.
    assert_eq!(app.last_query, "gamma");
    ctrl(&mut app, 's');
    typed(&mut app, "gamma");
    press(&mut app, &[KeyCode::Enter]);
    assert!(app.search.is_none());
    assert_eq!(app.cur, 2);
    assert!(app.job(app.presented()).found.is_none(), "unlit once done");
}

#[test]
fn a_query_not_there_fails_and_stays_put() {
    let (_dir, mut app) = slides();
    app.cur = 2;
    ctrl(&mut app, 's');
    typed(&mut app, "zeta");
    assert_eq!(app.cur, 2);
    assert!(status(&app).starts_with("failing"), "{}", status(&app));
    // Taken back to what is there, it is found again.
    for _ in 0..4 {
        press(&mut app, &[KeyCode::Backspace]);
    }
    typed(&mut app, "alp");
    assert_eq!(app.cur, 0);
}

#[test]
fn a_capital_searches_in_its_own_case() {
    let (_dir, mut app) = slides();
    ctrl(&mut app, 's');
    typed(&mut app, "Beta");
    assert!(status(&app).ends_with("1 of 1"), "{}", status(&app));
}

#[test]
fn ctrl_s_on_nothing_typed_searches_again_for_the_last() {
    let (_dir, mut app) = slides();
    ctrl(&mut app, 's');
    typed(&mut app, "gamma");
    press(&mut app, &[KeyCode::Enter]);
    app.cur = 0;
    ctrl(&mut app, 's');
    ctrl(&mut app, 's');
    assert_eq!(app.cur, 2);
    assert_eq!(app.search.as_ref().unwrap().field.text, "gamma");
}

#[test]
fn another_key_ends_the_search_and_does_its_own() {
    let (_dir, mut app) = slides();
    ctrl(&mut app, 's');
    typed(&mut app, "beta");
    press(&mut app, &[KeyCode::Right]);
    assert!(app.search.is_none());
    assert_eq!(app.cur, 2);
}

#[test]
fn backward_typing_finds_the_last_place_before() {
    let (_dir, mut app) = slides();
    app.cur = 2;
    ctrl(&mut app, 'r');
    typed(&mut app, "beta");
    assert_eq!(app.cur, 1);
    assert_eq!(app.search.as_ref().unwrap().at, Some((1, 1)));
}

#[test]
fn typing_more_stays_on_the_slide_gone_to_where_it_still_finds() {
    let pdf = fixture::with(&["x", "ab a a a ab", "ab"].map(|t| fixture::text(40., 150., 12., t)));
    let (_dir, mut app) = app_with(&pdf, Look::Text);
    read(&mut app);
    ctrl(&mut app, 's');
    typed(&mut app, "a");
    for _ in 0..3 {
        ctrl(&mut app, 's');
    }
    assert_eq!(app.search.as_ref().unwrap().at, Some((1, 3)));
    // "ab" is on this slide still: not the next slide's.
    typed(&mut app, "b");
    assert_eq!(app.cur, 1);
}

#[test]
fn ctrl_g_goes_back_to_the_grid_a_search_began_in() {
    let (_dir, mut app) = slides();
    press(&mut app, &[KeyCode::Char('g')]);
    ctrl(&mut app, 's');
    typed(&mut app, "gamma");
    assert!(app.cur == 2 && app.view == View::Grid);
    ctrl(&mut app, 'g');
    assert!(app.cur == 0 && app.view == View::Grid);
}

#[test]
fn slides_lit_for_what_was_found_before_are_not_kept() {
    let (_dir, mut app) = slides();
    ctrl(&mut app, 's');
    typed(&mut app, "beta");
    let now = app.job(app.presented());
    let before = Job {
        found: Some(render::Found {
            query: render::query_id("bet"),
            current: Some(0),
        }),
        ..now
    };
    app.slides.insert(now, None);
    app.slides.insert(before, None);
    app.schedule();
    assert!(app.slides.contains_key(&now));
    assert!(!app.slides.contains_key(&before));
}

#[test]
fn what_is_typed_before_the_slides_are_read_is_searched_for_once_they_are() {
    let (_dir, mut app) = slides();
    (app.texts, app.reading) = (None, None);
    ctrl(&mut app, 's');
    typed(&mut app, "gamma");
    assert!(
        status(&app).contains("reading the slides"),
        "{}",
        status(&app)
    );
    let start = Instant::now();
    while app.texts.is_none() {
        assert!(start.elapsed() < Duration::from_secs(30));
        std::thread::sleep(Duration::from_millis(5));
        app.read_texts();
    }
    assert_eq!(app.cur, 2);
}
