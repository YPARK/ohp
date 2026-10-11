use super::*;
use crate::fixture;
use ratatui::crossterm::event::KeyEvent;

/// Screen area above the status line, for a 140×40 terminal.
const MAIN: Rect = Rect::new(0, 0, 140, 39);

/// An app on a `pages`-page deck, with halfblocks' fixed 10×20 pixel cells.
fn app(pages: usize) -> (tempfile::TempDir, App) {
    app_with(&fixture::pdf(pages), Look::Image)
}

fn app_with(pdf: &[u8], look: Look) -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let deck = Deck::open(&fixture::write_pdf(dir.path(), pdf)).unwrap();
    let picker = Picker::halfblocks();
    let renderer = Renderer::spawn(&deck, &picker, 1).unwrap();
    let mut app = App::new(deck, picker, renderer, 1, look, Arc::default());
    app.main = MAIN;
    (dir, app)
}

fn job(key: Key, look: Look) -> Job {
    Job {
        key,
        look,
        lit: None,
    }
}

fn press(app: &mut App, keys: &[KeyCode]) {
    for &code in keys {
        app.key(KeyEvent::from(code));
    }
}

/// Let the next `watch` look at the file now.
fn due(app: &mut App) {
    app.checked = Instant::now() - WATCH_EVERY;
}

/// Look at the file now, and wait for any reload that starts.
fn watch(app: &mut App) {
    due(app);
    app.watch();
    finish(app);
}

/// Wait for a reload under way to finish.
fn finish(app: &mut App) {
    let start = Instant::now();
    while app.loading.is_some() {
        assert!(start.elapsed() < Duration::from_secs(30), "the reload hung");
        std::thread::sleep(Duration::from_millis(5));
        app.loaded();
    }
}

/// A reload under way that sends `deck` once done.
fn loading(deck: anyhow::Result<Deck>) -> (Receiver<anyhow::Result<Deck>>, JoinHandle<()>) {
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(deck).unwrap();
    (rx, std::thread::spawn(|| {}))
}

#[test]
fn auto_grid_takes_the_fewest_columns_that_show_a_dozen() {
    let (_dir, app) = app(40);
    let grid = app.grid();
    assert!(grid.cols * grid.rows >= GRID_TARGET);
    let narrower = app.grid_with(grid.cols as u16 - 1);
    assert!(narrower.cols * narrower.rows < GRID_TARGET);
}

#[test]
fn a_small_deck_gets_big_slides() {
    let (_dir, app) = app(2);
    assert!(app.grid().cols <= 2);
}

#[test]
fn zoom_steps_one_column_from_the_auto_grid() {
    let (_dir, mut app) = app(40);
    press(&mut app, &[KeyCode::Char('g')]);
    let auto = app.grid().cols;
    press(&mut app, &[KeyCode::Char('-')]);
    assert_eq!(app.grid().cols, auto + 1);
    press(&mut app, &[KeyCode::Char('+'), KeyCode::Char('+')]);
    assert_eq!(app.grid().cols, auto - 1);
}

#[test]
fn zoom_stops_at_one_column_and_at_the_narrowest_slot() {
    let (_dir, mut app) = app(40);
    press(&mut app, &[KeyCode::Char('g')]);
    press(&mut app, &[KeyCode::Char('+'); 50]);
    assert_eq!(app.grid().cols, 1);
    press(&mut app, &[KeyCode::Char('-'); 50]);
    assert_eq!(app.grid().cols, usize::from(MAIN.width / MIN_SLOT_WIDTH));
}

#[test]
fn zoom_is_for_the_grid_only() {
    let (_dir, mut app) = app(40);
    press(&mut app, &[KeyCode::Char('-'), KeyCode::Char('+')]);
    assert_eq!(app.grid_cols, None);
}

#[test]
fn n_p_and_arrows_stay_within_the_deck() {
    let (_dir, mut app) = app(5);
    press(&mut app, &[KeyCode::Char('p'), KeyCode::Left, KeyCode::Up]);
    assert_eq!(app.cur, 0);
    press(
        &mut app,
        &[KeyCode::Char('n'), KeyCode::Right, KeyCode::Down],
    );
    assert_eq!(app.cur, 3);
    press(&mut app, &[KeyCode::Char('n'); 10]);
    assert_eq!(app.cur, 4);
}

#[test]
fn up_and_down_move_a_row_in_the_grid() {
    let (_dir, mut app) = app(40);
    press(&mut app, &[KeyCode::Tab]);
    let cols = app.grid().cols;
    press(&mut app, &[KeyCode::Down, KeyCode::Down, KeyCode::Right]);
    assert_eq!(app.cur, 2 * cols + 1);
    press(&mut app, &[KeyCode::Up]);
    assert_eq!(app.cur, cols + 1);
}

#[test]
fn enter_and_esc_leave_the_grid() {
    let (_dir, mut app) = app(5);
    for leave in [KeyCode::Enter, KeyCode::Esc, KeyCode::Char('g')] {
        press(&mut app, &[KeyCode::Char('g')]);
        assert!(app.view == View::Grid);
        press(&mut app, &[leave]);
        assert!(app.view == View::Present);
    }
}

#[test]
fn the_grid_scrolls_to_keep_the_current_slide_on_screen() {
    let (_dir, mut app) = app(150);
    press(&mut app, &[KeyCode::Char('g')]);
    let grid = app.grid();
    for cur in [149, 0, 75] {
        app.cur = cur;
        app.scroll();
        let first = app.top * grid.cols;
        assert!(
            (first..first + grid.rows * grid.cols).contains(&cur),
            "slide {cur}"
        );
    }
}

#[test]
fn zooming_out_near_the_end_leaves_no_empty_rows() {
    let (_dir, mut app) = app(150);
    press(&mut app, &[KeyCode::Char('g')]);
    app.cur = 149;
    press(&mut app, &[KeyCode::Char('-'); 50]);
    app.scroll();
    let grid = app.grid();
    let rows = 150usize.div_ceil(grid.cols);
    assert_eq!(app.top, rows.saturating_sub(grid.rows));
}

#[test]
fn only_the_current_zoom_and_nearby_full_slides_are_kept() {
    let (_dir, mut app) = app(40);
    press(&mut app, &[KeyCode::Char('g')]);
    let grid = app.grid();
    let inner = Block::bordered().inner(Rect::new(0, 0, grid.slot.width, grid.slot.height));
    let at = |page, cols, rows| Key::new(page, Rect::new(0, 0, cols, rows));
    let thumb = |page| job(at(page, inner.width, inner.height), Look::Image);
    let full = |page| job(Key::new(page, MAIN), Look::Text);
    let old_zoom = job(at(0, inner.width + 5, inner.height + 3), Look::Image);
    for job in [thumb(7), full(1), full(20), old_zoom] {
        app.slides.insert(job, None);
    }
    app.schedule();
    assert!(app.slides.contains_key(&thumb(7)));
    assert!(app.slides.contains_key(&full(1)));
    assert!(!app.slides.contains_key(&full(20)));
    assert!(!app.slides.contains_key(&old_zoom));
}

#[test]
fn plus_zooms_the_slide_about_the_middle_and_arrows_pan() {
    let (_dir, mut app) = app(5);
    press(&mut app, &[KeyCode::Char('+')]);
    // 150% of a slide 140 cells wide: the middle 70 stays in the middle.
    assert_eq!((app.zoom.percent, app.zoom.x), (150, 35));
    press(&mut app, &[KeyCode::Right]);
    assert_eq!(app.cur, 0);
    assert!(app.zoom.x > 35);
    // The slide is 1.5 × 138.6 cells wide: 208, 68 more than the screen.
    press(&mut app, &[KeyCode::Right, KeyCode::Down]);
    assert_eq!(app.zoom.x, 68, "stops at the slide's right edge");
    assert!(app.zoom.y > 0);
    press(&mut app, &[KeyCode::Char('n')]);
    assert_eq!(app.cur, 1);
    assert_eq!(app.zoom.percent, 150, "zoom kept from slide to slide");
    press(&mut app, &[KeyCode::Char('0'), KeyCode::Right]);
    assert_eq!((app.zoom, app.cur), (Zoom::FIT, 2));
}

#[test]
fn slide_zoom_stops_at_its_steps() {
    let (_dir, mut app) = app(5);
    press(&mut app, &[KeyCode::Char('+'); 10]);
    assert_eq!(app.zoom.percent, *ZOOMS.last().unwrap());
    press(&mut app, &[KeyCode::Char('-'); 10]);
    assert_eq!(app.zoom, Zoom::FIT);
}

#[test]
fn a_resize_keeps_the_pan_within_the_slide() {
    let (_dir, mut app) = app(5);
    press(
        &mut app,
        &[KeyCode::Char('+'), KeyCode::Right, KeyCode::Right],
    );
    app.main = Rect::new(0, 0, 200, 60);
    app.clamp_pan();
    let most = (f32::from(app.main.width) * 1.5).round() as u16 - app.main.width;
    assert!(app.zoom.x <= most);
}

#[test]
fn the_zoomed_slide_is_asked_for_and_other_zooms_dropped() {
    let (_dir, mut app) = app(5);
    let at = |zoom| {
        job(
            Key {
                zoom,
                ..Key::new(0, MAIN)
            },
            Look::Image,
        )
    };
    let old = at(Zoom {
        percent: 300,
        x: 5,
        y: 5,
    });
    app.slides.insert(at(Zoom::FIT), None);
    app.slides.insert(old, None);
    press(&mut app, &[KeyCode::Char('+')]);
    app.schedule();
    assert!(app.requested.contains(&at(app.zoom)));
    assert!(
        app.slides.contains_key(&at(Zoom::FIT)),
        "fitted slides are kept"
    );
    assert!(!app.slides.contains_key(&old));
}

/// Draw the whole screen, 140×40 as `MAIN` is part of, and return its text.
fn screen(app: &App) -> String {
    use ratatui::backend::TestBackend;
    let mut terminal = ratatui::Terminal::new(TestBackend::new(140, 40)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let buf = terminal.backend().buffer();
    buf.content().iter().map(|c| c.symbol()).collect()
}

#[test]
fn the_last_view_stays_on_screen_while_a_new_zoom_renders() {
    let (_dir, mut app) = app_with(&fixture::hello(), Look::Image);
    app.schedule();
    app.receive(fixture::next(&app.renderer));
    let fitted = job(Key::new(0, MAIN), Look::Image);
    assert!(app.slides.contains_key(&fitted));
    screen(&app);
    assert_eq!(app.shown.get(), Some(fitted));

    press(&mut app, &[KeyCode::Char('+')]);
    app.schedule();
    let text = screen(&app);
    assert!(
        !text.contains("rendering…"),
        "the slide blanked while zooming"
    );
    assert!(
        text.contains("rendering 1"),
        "the zoomed slide is waited for"
    );
    assert_eq!(app.shown.get(), Some(fitted));
    assert!(app.slides.contains_key(&fitted));
}

#[test]
fn slides_either_side_are_fetched_fitted_even_when_zoomed() {
    let (_dir, mut app) = app(5);
    press(&mut app, &[KeyCode::Char('+')]);
    app.schedule();
    assert!(app.requested.contains(&job(Key::new(1, MAIN), Look::Image)));
    assert!(
        !app.requested
            .iter()
            .any(|j| j.key.page == 1 && j.key.zoom != Zoom::FIT)
    );
}

#[test]
fn slides_fetched_ahead_are_not_counted_as_rendering() {
    let (_dir, mut app) = app(5);
    app.schedule();
    app.slides.insert(job(Key::new(0, MAIN), Look::Image), None);
    assert!(
        !app.requested.is_empty(),
        "the slides either side are on their way"
    );
    assert!(!app.status().to_string().contains("rendering"));
}

#[test]
fn t_switches_between_images_and_text() {
    let (_dir, mut app) = app(3);
    press(&mut app, &[KeyCode::Char('t')]);
    assert_eq!(app.look, Look::Text);
    press(&mut app, &[KeyCode::Char('g'), KeyCode::Char('t')]);
    assert_eq!(app.look, Look::Image);
}

#[test]
fn the_text_look_asks_for_text() {
    let (_dir, mut app) = app(3);
    press(&mut app, &[KeyCode::Char('t')]);
    app.schedule();
    assert!(app.requested.contains(&job(Key::new(0, MAIN), Look::Text)));
    assert!(app.requested.iter().all(|job| job.look == Look::Text));
}

#[test]
fn a_text_slide_is_drawn_over_its_backdrop() {
    use ratatui::backend::TestBackend;
    let (_dir, mut app) = app_with(&fixture::hello(), Look::Text);
    let mut terminal = ratatui::Terminal::new(TestBackend::new(80, 24)).unwrap();
    app.main = split(Rect::new(0, 0, 80, 24)).0;
    app.schedule();
    app.receive(fixture::next(&app.renderer));
    terminal.draw(|f| app.draw(f)).unwrap();

    let buf = terminal.backend().buffer();
    let screen: String = buf.content().iter().map(|c| c.symbol()).collect();
    assert!(screen.contains("Hello"));
    // Black on the white of the slide, whatever the terminal's colours.
    let h = buf.content().iter().find(|c| c.symbol() == "H").unwrap();
    assert_eq!(
        (h.fg, h.bg),
        (Color::Rgb(0, 0, 0), Color::Rgb(255, 255, 255))
    );
}

#[test]
fn a_rewritten_pdf_reloads_once_it_settles() {
    let (dir, mut app) = app(3);
    fixture::write(dir.path(), 5);

    watch(&mut app);
    assert_eq!(app.deck.pages, 3, "reloaded before the file settled");
    assert_eq!(app.notice.as_deref(), Some("PDF changing…"));

    watch(&mut app);
    assert_eq!(app.deck.pages, 5);
    assert_eq!(app.notice.as_deref(), Some("reloaded"));
}

#[test]
fn a_half_written_pdf_is_not_reloaded() {
    let (dir, mut app) = app(3);
    let whole = fixture::pdf(5);
    std::fs::write(dir.path().join("deck.pdf"), &whole[..whole.len() / 2]).unwrap();
    for _ in 0..3 {
        watch(&mut app);
    }
    assert_eq!(app.deck.pages, 3);
    assert_eq!(app.notice.as_deref(), Some("PDF changing…"));
}

#[test]
fn a_broken_pdf_keeps_the_old_slides() {
    let (dir, mut app) = app(3);
    std::fs::write(dir.path().join("deck.pdf"), b"%PDF-1.5\ngarbage\n%%EOF\n").unwrap();
    for _ in 0..2 {
        watch(&mut app);
    }
    assert_eq!(app.deck.pages, 3);
    assert!(
        app.notice
            .as_deref()
            .is_some_and(|n| n.starts_with("reload failed"))
    );
}

#[test]
fn a_shorter_deck_moves_the_current_slide_back() {
    let (dir, mut app) = app(10);
    app.cur = 8;
    fixture::write(dir.path(), 4);
    for _ in 0..2 {
        watch(&mut app);
    }
    assert_eq!(app.deck.pages, 4);
    assert_eq!(app.cur, 3);
}

#[test]
fn a_reload_keeps_old_slides_on_screen_until_replaced() {
    let (dir, mut app) = app(3);
    let key = job(Key::new(0, MAIN), Look::Image);
    app.slides.insert(key, None);
    fixture::write(dir.path(), 4);
    for _ in 0..2 {
        watch(&mut app);
    }
    assert!(app.slides.is_empty());
    assert!(app.stale.contains_key(&key));
}

#[test]
fn a_change_while_reloading_reloads_again() {
    let (dir, mut app) = app(3);
    fixture::write(dir.path(), 6);
    app.loading = Some(loading(Deck::open(&dir.path().join("deck.pdf"))));
    app.reload();
    assert!(app.then == Some(Then::Reload));
    app.loaded();
    assert_eq!(app.deck.pages, 6);
    assert!(app.loading.is_some(), "read once more");
    app.stop();
    assert!(app.loading.is_none());
}

#[test]
fn r_reloads_a_file_the_watch_has_already_seen() {
    let (dir, mut app) = app(3);
    fixture::write(dir.path(), 5);
    // As after a reload that failed: the change is taken, the old deck kept.
    app.stamp = render::stamp(&app.deck.path);
    watch(&mut app);
    assert_eq!(app.deck.pages, 3);

    press(&mut app, &[KeyCode::Char('r')]);
    finish(&mut app);
    assert_eq!(app.deck.pages, 5);
    assert_eq!(app.notice.as_deref(), Some("reloaded"));
}

#[test]
fn a_key_press_clears_the_notice() {
    let (_dir, mut app) = app(3);
    app.notice = Some("reloaded".into());
    press(&mut app, &[KeyCode::Char('n')]);
    assert_eq!(app.notice, None);
}

#[test]
fn centred_keeps_the_image_inside_its_area() {
    let area = Rect::new(10, 5, 100, 30);
    assert_eq!(centred(area, Size::new(60, 30)), Rect::new(30, 5, 60, 30));
    assert_eq!(centred(area, Size::new(200, 50)), area);
}

#[test]
fn dropping_the_app_stops_a_reload_and_waits_for_it() {
    let (_dir, mut app) = app(3);
    let stop = app.deck.options.stop.clone();
    let heard = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let told = heard.clone();
    let reader = std::thread::spawn(move || {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(10) {
            if stop.load(Ordering::Relaxed) {
                told.store(true, Ordering::Relaxed);
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    });
    app.loading = Some((std::sync::mpsc::channel().1, reader));
    drop(app);
    assert!(heard.load(Ordering::Relaxed));
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn type_in(app: &mut App, text: &str) {
    press(app, &text.chars().map(KeyCode::Char).collect::<Vec<_>>());
}

/// A `pages`-page PDF written to `name` in `dir`.
fn write_named(dir: &Path, name: &str, pages: usize) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, fixture::pdf(pages)).unwrap();
    path
}

#[test]
fn ctrl_o_opens_another_file_in_place_of_this_one() {
    let (dir, mut app) = app(3);
    let other = write_named(dir.path(), "other.pdf", 5);
    press(&mut app, &[KeyCode::Right, KeyCode::Char('g')]);
    app.key(ctrl('o'));
    assert!(app.browser.is_some());
    // Keys go to the list, not the slides.
    type_in(&mut app, "oth");
    assert_eq!(app.cur, 1);
    press(&mut app, &[KeyCode::Enter]);
    assert!(app.browser.is_none());
    assert!(app.status().to_string().contains("opening"));
    finish(&mut app);

    assert_eq!(app.deck.path, other.canonicalize().unwrap());
    assert_eq!(app.deck.pages, 5);
    assert_eq!(app.cur, 0);
    assert!(app.view == View::Present);
    assert_eq!(app.stamp, render::stamp(&other));
}

#[test]
fn the_list_starts_on_the_file_presented() {
    let (dir, mut app) = app(3);
    write_named(dir.path(), "another.pdf", 2);
    app.key(ctrl('o'));
    press(&mut app, &[KeyCode::Enter]);
    finish(&mut app);
    assert_eq!(app.deck.pages, 3);
}

#[test]
fn esc_or_ctrl_o_closes_the_list() {
    let (_dir, mut app) = app(3);
    app.key(ctrl('o'));
    press(&mut app, &[KeyCode::Esc]);
    assert!(app.browser.is_none());
    app.key(ctrl('o'));
    app.key(ctrl('o'));
    assert!(app.browser.is_none());
    assert!(app.loading.is_none());
}

#[test]
fn a_file_that_cannot_be_read_keeps_the_slides() {
    let (dir, mut app) = app(3);
    std::fs::write(dir.path().join("broken.pdf"), b"not a pdf").unwrap();
    app.key(ctrl('o'));
    type_in(&mut app, "broken");
    press(&mut app, &[KeyCode::Enter]);
    finish(&mut app);
    assert_eq!(app.deck.pages, 3);
    assert!(app.notice.as_deref().unwrap().starts_with("cannot open"));
}

#[test]
fn a_file_picked_while_reloading_is_read_after() {
    let (dir, mut app) = app(3);
    write_named(dir.path(), "other.pdf", 5);
    app.loading = Some(loading(Deck::open(&dir.path().join("deck.pdf"))));
    app.then = Some(Then::Reload);
    app.key(ctrl('o'));
    type_in(&mut app, "oth");
    press(&mut app, &[KeyCode::Enter]);
    finish(&mut app);
    assert_eq!(app.deck.pages, 5);
    assert!(app.then.is_none());
}

#[test]
fn a_reload_asked_for_while_opening_is_dropped() {
    let (dir, mut app) = app(3);
    write_named(dir.path(), "other.pdf", 5);
    app.key(ctrl('o'));
    type_in(&mut app, "oth");
    press(&mut app, &[KeyCode::Enter]);
    // As when the file opened before changes while the other is read.
    app.reload();
    assert!(app.then == Some(Then::Reload));
    finish(&mut app);
    assert_eq!(app.deck.pages, 5);
    assert!(app.then.is_none());
    assert!(app.loading.is_none(), "the file opened is not read again");
}

#[test]
fn ctrl_c_quits_with_the_list_open() {
    let (_dir, mut app) = app(3);
    app.key(ctrl('o'));
    app.key(ctrl('c'));
    assert!(app.quit);
}

#[test]
fn the_list_is_drawn_over_the_slides() {
    let (_dir, mut app) = app(3);
    app.key(ctrl('o'));
    let text = screen(&app);
    assert!(text.contains(" open "));
    assert!(text.contains("deck.pdf"));
}

#[cfg(feature = "speech")]
/// An app on a deck whose pages say `texts`, read aloud by `cat` as the
/// voice and `command` as the player, which gets the text.
fn speaking(texts: &[&str], command: &str) -> (tempfile::TempDir, App) {
    let (dir, mut app) = app_with(&fixture::saying(texts), Look::Image);
    app.speaker = Speaker::new(Some("cat".into()), Some(command.into()));
    (dir, app)
}

#[cfg(feature = "speech")]
/// Keep reading along until `done` holds.
fn read_until(app: &mut App, done: impl Fn(&App) -> bool) {
    let start = Instant::now();
    while !done(app) {
        assert!(start.elapsed() < Duration::from_secs(10), "reading hung");
        std::thread::sleep(Duration::from_millis(5));
        app.read_along();
    }
}

#[cfg(feature = "speech")]
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

#[cfg(feature = "speech")]
#[test]
fn s_again_stops_reading() {
    let (_deck, mut app) = speaking(&["one", "two"], "sleep 30");
    press(&mut app, &[KeyCode::Char('s')]);
    assert_eq!(app.speaker.page(), Some(0));
    press(&mut app, &[KeyCode::Char('s')]);
    assert_eq!(app.speaker.page(), None);
    assert_eq!(app.cur, 0);
}

#[cfg(feature = "speech")]
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

#[cfg(feature = "speech")]
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

#[cfg(feature = "speech")]
#[test]
fn a_failed_reading_stops_and_says_why() {
    let (_deck, mut app) = speaking(&["one", "two"], "echo 'no voice' >&2; exit 1");
    press(&mut app, &[KeyCode::Char('s')]);
    read_until(&mut app, |app| app.speaker.page().is_none());
    assert_eq!(app.cur, 0);
    let notice = app.notice.take().unwrap_or_default();
    assert!(notice.contains("no voice"), "{notice}");
}

fn click(app: &mut App, at: Position) {
    app.mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: at.x,
        row: at.y,
        modifiers: KeyModifiers::NONE,
    });
}

#[cfg(feature = "speech")]
/// What is read once the slide is clicked where `at` finds, the slide
/// drawn first in `look` at `zoom`, as it shows.
fn read_from_click(
    look: Look,
    zoom: Zoom,
    at: impl Fn(&App, &ratatui::buffer::Buffer) -> Position,
) -> String {
    use ratatui::backend::TestBackend;
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("read.txt");
    let (_deck, mut app) = speaking(
        &["One here. Two here. Three here."],
        &format!("cat >> '{}'", out.display()),
    );
    (app.look, app.zoom) = (look, zoom);
    app.schedule();
    app.receive(fixture::next(&app.renderer));
    let mut terminal = ratatui::Terminal::new(TestBackend::new(140, 40)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let at = at(&app, terminal.backend().buffer());
    click(&mut app, at);
    assert_eq!(app.speaker.page(), Some(0), "nothing read from the click");
    read_until(&mut app, |app| app.speaker.page().is_none());
    std::fs::read_to_string(&out).unwrap()
}

#[cfg(feature = "speech")]
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

#[cfg(feature = "speech")]
#[test]
fn clicking_a_sentence_on_the_slide_reads_from_it() {
    let read = read_from_click(Look::Image, Zoom::FIT, |app, _| second_sentence(app));
    assert_eq!(read, "Two here.\nThree here.\n");
}

#[cfg(feature = "speech")]
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

#[cfg(feature = "speech")]
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

#[cfg(feature = "speech")]
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
fn clicking_a_slide_in_the_grid_picks_it_and_again_presents_it() {
    let (_dir, mut app) = app(6);
    press(&mut app, &[KeyCode::Char('g')]);
    let slot = |app: &App, i: usize| {
        let (_, slot) = app.slots(&app.grid()).nth(i).unwrap();
        // Its far corner, inside its border.
        Position::new(slot.right() - 1, slot.bottom() - 1)
    };
    let at = slot(&app, 4);
    click(&mut app, at);
    assert_eq!((app.cur, app.view == View::Grid), (4, true));
    let at = slot(&app, 2);
    click(&mut app, at);
    assert_eq!((app.cur, app.view == View::Grid), (2, true));
    click(&mut app, at);
    assert_eq!((app.cur, app.view == View::Present), (2, true));
}

#[test]
fn clicking_between_the_grid_s_slots_does_nothing() {
    let (_dir, mut app) = app(3);
    press(&mut app, &[KeyCode::Char('g')]);
    let grid = app.grid();
    let below = app.slots(&grid).map(|(_, s)| s.bottom()).max().unwrap();
    click(&mut app, Position::new(grid.origin.0, below));
    assert_eq!((app.cur, app.view == View::Grid), (0, true));
}

#[cfg(feature = "speech")]
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
