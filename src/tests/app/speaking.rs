//! Reading slides aloud: on `s`, as slides turn and reload, and from a
//! sentence clicked while reading.

use super::*;
use crate::speak::Speaker;
use crate::text;
use std::time::{Duration, Instant};

/// An app on a deck whose pages say `texts`, read aloud by `cat` as the
/// voice and `command` as the player, which gets the text.
fn speaking(texts: &[&str], command: &str) -> (tempfile::TempDir, App) {
    let (dir, mut app) = app_with(&fixture::saying(texts), Look::Image);
    app.speaker = Speaker::new(Some("cat".into()), Some(command.into()));
    (dir, app)
}

/// Keep reading along until `done` holds.
fn read_until(app: &mut App, done: impl Fn(&App) -> bool) {
    let start = Instant::now();
    while !done(app) {
        assert!(start.elapsed() < Duration::from_secs(10), "reading hung");
        std::thread::sleep(Duration::from_millis(5));
        app.read_along();
    }
}

#[test]
fn s_reads_from_this_slide_to_the_end() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("read.txt");
    let (_deck, mut app) = speaking(
        &["one", "two", "three"],
        &format!("cat >> '{}'", out.display()),
    );
    press(&mut app, &[KeyCode::Char('n'), KeyCode::Char('s')]);
    read_until(&mut app, |app| app.speaker.page().is_none());
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "two\nthree\n");
    assert_eq!(app.cur, 2);
    assert_eq!(app.notice.as_deref(), Some("read to the end"));
}

#[test]
fn s_again_stops_reading() {
    let (_deck, mut app) = speaking(&["one", "two"], "sleep 30");
    press(&mut app, &[KeyCode::Char('s')]);
    assert_eq!(app.speaker.page(), Some(0));
    press(&mut app, &[KeyCode::Char('s')]);
    assert_eq!(app.speaker.page(), None);
    assert_eq!(app.cur, 0);
}

#[test]
fn turning_the_slide_while_reading_reads_that_one() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("read.txt");
    let command = format!("cat >> '{}'; sleep 30", out.display());
    let (_deck, mut app) = speaking(&["one", "two", "three"], &command);
    press(
        &mut app,
        &[KeyCode::Char('s'), KeyCode::Char('n'), KeyCode::Char('n')],
    );
    app.read_along();
    assert_eq!(app.speaker.page(), Some(2));
    read_until(&mut app, |_| {
        std::fs::read_to_string(&out).is_ok_and(|t| t.ends_with("three\n"))
    });
    assert!(!std::fs::read_to_string(&out).unwrap().contains("two"));
}

#[test]
fn a_reload_while_reading_reads_the_slide_again() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("read.txt");
    let command = format!("cat >> '{}'; sleep 30", out.display());
    let (deck, mut app) = speaking(&["old", "two"], &command);
    press(&mut app, &[KeyCode::Char('s')]);
    read_until(&mut app, |_| {
        std::fs::read_to_string(&out).is_ok_and(|t| t == "old\n")
    });
    fixture::write_pdf(deck.path(), &fixture::saying(&["new", "two"]));
    app.reload();
    finish(&mut app);
    assert_eq!(app.speaker.page(), Some(0));
    read_until(&mut app, |_| {
        std::fs::read_to_string(&out).is_ok_and(|t| t == "old\nnew\n")
    });
}

#[test]
fn a_failed_reading_stops_and_says_why() {
    let (_deck, mut app) = speaking(&["one", "two"], "echo 'no voice' >&2; exit 1");
    press(&mut app, &[KeyCode::Char('s')]);
    read_until(&mut app, |app| app.speaker.page().is_none());
    assert_eq!(app.cur, 0);
    let notice = app.notice.take().unwrap_or_default();
    assert!(notice.contains("no voice"), "{notice}");
}

/// What is read once the slide is clicked where `at` finds, while reading
/// aloud, the slide drawn first in `look` at `zoom`, as it shows.
fn read_from_click(
    look: Look,
    zoom: Zoom,
    at: impl Fn(&App, &ratatui::buffer::Buffer) -> Position,
) -> String {
    use ratatui::backend::TestBackend;
    let dir = tempfile::tempdir().unwrap();
    let (out, started) = (dir.path().join("read.txt"), dir.path().join("started"));
    // The reading `s` starts says nothing, and lasts until the click.
    let (_deck, mut app) = speaking(
        &["One here. Two here. Three here."],
        &format!(
            "if [ -e '{0}' ]; then cat >> '{1}'; else touch '{0}'; sleep 30; fi",
            started.display(),
            out.display()
        ),
    );
    (app.look, app.zoom) = (look, zoom);
    app.schedule();
    app.receive(fixture::next(&app.renderer));
    let mut terminal = ratatui::Terminal::new(TestBackend::new(140, 40)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let at = at(&app, terminal.backend().buffer());
    press(&mut app, &[KeyCode::Char('s')]);
    read_until(&mut app, |_| started.exists());
    click(&mut app, at);
    assert_eq!(app.speaker.page(), Some(0), "nothing read from the click");
    read_until(&mut app, |app| app.speaker.page().is_none());
    std::fs::read_to_string(&out).unwrap()
}

/// Where sentence 1 of the slide shows, the slide as it is drawn.
fn second_sentence(app: &App) -> Position {
    let (job, area) = app.drawn.get().unwrap();
    let page = text::of(app.deck.data.clone(), 0).unwrap();
    let b = page.light(1).boxes[0];
    // Halfblocks' cells are 10×20 pixels; the slide fits the screen, and is
    // enlarged from that.
    let fit = (f64::from(MAIN.width) * 10. / f64::from(page.width))
        .min(f64::from(MAIN.height) * 20. / f64::from(page.height));
    let scale = fit * f64::from(job.key.zoom.percent) / 100.;
    let cell = |at: f64, size: f64, pan: u16| (at * scale / size) as u16 - pan;
    Position::new(
        area.x + cell(b.center().x, 10., job.key.zoom.x),
        area.y + cell(b.center().y, 20., job.key.zoom.y),
    )
}

#[test]
fn clicking_a_sentence_on_the_slide_reads_from_it() {
    let read = read_from_click(Look::Image, Zoom::FIT, |app, _| second_sentence(app));
    assert_eq!(read, "Two here.\nThree here.\n");
}

#[test]
fn clicking_a_sentence_on_the_slide_zoomed_and_panned_reads_from_it() {
    let zoom = Zoom {
        percent: 200,
        x: 20,
        y: 15,
    };
    let read = read_from_click(Look::Image, zoom, |app, _| second_sentence(app));
    assert_eq!(read, "Two here.\nThree here.\n");
}

#[test]
fn clicking_a_word_of_the_slides_text_reads_from_its_sentence() {
    let read = read_from_click(Look::Text, Zoom::FIT, |_, buf| {
        let i = (buf.content().iter())
            .position(|c| c.symbol() == "T")
            .unwrap();
        buf.pos_of(i).into()
    });
    assert_eq!(read, "Two here.\nThree here.\n");
}

#[test]
fn clicking_a_sentence_reads_nothing_unless_reading_aloud() {
    use ratatui::backend::TestBackend;
    let (_deck, mut app) = speaking(&["One here. Two here."], "cat > /dev/null");
    app.schedule();
    app.receive(fixture::next(&app.renderer));
    let mut terminal = ratatui::Terminal::new(TestBackend::new(140, 40)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let at = second_sentence(&app);
    click(&mut app, at);
    assert_eq!(app.speaker.page(), None);
}

#[test]
fn clicking_where_no_sentence_is_reads_nothing() {
    let (_deck, mut app) = speaking(&["One here."], "cat > /dev/null");
    app.schedule();
    app.receive(fixture::next(&app.renderer));
    screen(&app);
    click(&mut app, Position::new(1, 1));
    assert_eq!(app.speaker.page(), None);
}

#[test]
fn the_sentence_being_read_is_lit_on_the_slide_read() {
    let (_deck, mut app) = speaking(
        &["Only one. Then two.", "next"],
        "cat > /dev/null; sleep 30",
    );
    assert_eq!(app.job(app.presented()).lit, None);
    press(&mut app, &[KeyCode::Char('s')]);
    read_until(&mut app, |app| app.lit == Some(0));
    assert_eq!(app.job(app.presented()).lit, Some(0));
    let other = Key {
        page: 1,
        ..app.presented()
    };
    assert_eq!(app.job(other).lit, None);
    press(&mut app, &[KeyCode::Char('s')]);
    app.read_along();
    assert_eq!(app.lit, None);
    assert_eq!(app.job(app.presented()).lit, None);
}
