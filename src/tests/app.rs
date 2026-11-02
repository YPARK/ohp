use super::*;
use crate::fixture;
use draw::{SIDE, centred};
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Position;
use ratatui::style::Color;

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
    // Not what is kept here: what tests keep, kept with them.
    app.config = Some(dir.path().join("config"));
    (dir, app)
}

fn job(key: Key, look: Look) -> Job {
    Job {
        key,
        look,
        lit: None,
        marked: Picked::default(),
        found: None,
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
        let first = app.top;
        assert!(
            (first..first + grid.rows * grid.cols).contains(&cur),
            "slide {cur}"
        );
    }
}

#[test]
fn zooming_out_near_the_end_leaves_no_empty_slots() {
    let (_dir, mut app) = app(150);
    press(&mut app, &[KeyCode::Char('g')]);
    app.cur = 149;
    press(&mut app, &[KeyCode::Char('-'); 50]);
    app.scroll();
    let grid = app.grid();
    assert_eq!(app.top, 150usize.saturating_sub(grid.rows * grid.cols));
}

#[test]
fn the_grid_moves_a_slide_at_a_time() {
    let (_dir, mut app) = app(150);
    press(&mut app, &[KeyCode::Char('g')]);
    let grid = app.grid();
    let shown = grid.rows * grid.cols;
    app.cur = shown - 1;
    app.scroll();
    assert_eq!(app.top, 0);
    press(&mut app, &[KeyCode::Char('n')]);
    app.scroll();
    assert_eq!((app.cur, app.top), (shown, 1));
    let slots: Vec<usize> = app.slots(&app.grid()).map(|(i, _)| i).collect();
    assert_eq!(slots, (1..=shown).collect::<Vec<_>>());
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
    app.main = split(Rect::new(0, 0, 80, 24), false).0;
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
    app.stamp = app.deck.stamps();
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

    assert_eq!(app.deck.paths, [other.canonicalize().unwrap()]);
    assert_eq!(app.deck.pages, 5);
    assert_eq!(app.cur, 0);
    assert!(app.view == View::Present);
    assert_eq!(app.stamp, [render::stamp(&other)]);
}

#[cfg(feature = "markdown")]
#[test]
fn ctrl_o_opens_the_files_marked_together() {
    let (dir, mut app) = app(3);
    write_named(dir.path(), "other.pdf", 5);
    app.key(ctrl('o'));
    type_in(&mut app, "oth");
    press(&mut app, &[KeyCode::Tab]);
    press(&mut app, &[KeyCode::Esc]);
    type_in(&mut app, "deck");
    press(&mut app, &[KeyCode::Tab, KeyCode::Enter]);
    finish(&mut app);

    let root = dir.path().canonicalize().unwrap();
    assert_eq!(
        app.deck.paths,
        [root.join("other.pdf"), root.join("deck.pdf")]
    );
    assert_eq!(app.deck.pages, 8);
    // The status line names the file the slide is from.
    let status = app.status().to_string();
    assert!(status.starts_with(" other.pdf  1/8  file 1/2"), "{status}");
    press(&mut app, &[KeyCode::Char('n'); 5]);
    let status = app.status().to_string();
    assert!(status.starts_with(" deck.pdf  6/8  file 2/2"), "{status}");

    // A change to either is seen.
    write_named(dir.path(), "other.pdf", 2);
    watch(&mut app);
    assert_eq!(app.notice.as_deref(), Some("file changing…"));
    watch(&mut app);
    assert_eq!(app.deck.pages, 5);
}

#[cfg(feature = "markdown")]
#[test]
fn ctrl_o_over_several_files_starts_with_them_marked_to_add_to() {
    let (dir, mut app) = app(3);
    write_named(dir.path(), "other.pdf", 5);
    write_named(dir.path(), "third.pdf", 1);
    let root = dir.path().canonicalize().unwrap();
    let pair = vec![root.join("deck.pdf"), root.join("other.pdf")];
    app.open(pair.clone());
    finish(&mut app);
    assert_eq!(app.deck.pages, 8);

    // Enter on another adds it, and shows it first.
    app.key(ctrl('o'));
    type_in(&mut app, "thi");
    press(&mut app, &[KeyCode::Enter]);
    finish(&mut app);
    assert_eq!(
        app.deck.paths,
        [&[root.join("third.pdf")], &pair[..]].concat()
    );
    assert_eq!(app.cur, 0);
    let status = app.status().to_string();
    assert!(status.starts_with(" third.pdf"), "{status}");

    // Shift-tab unmarks them all, so enter opens just the one picked.
    app.key(ctrl('o'));
    press(&mut app, &[KeyCode::BackTab]);
    type_in(&mut app, "oth");
    press(&mut app, &[KeyCode::Enter]);
    finish(&mut app);
    assert_eq!(app.deck.paths, [root.join("other.pdf")]);
}

#[cfg(feature = "markdown")]
#[test]
fn ctrl_o_opens_the_directory_of_the_file_on_screen() {
    let (dir, mut app) = app(1);
    let root = dir.path().canonicalize().unwrap();
    let slides = root.join("slides");
    let figs = root.join("figs");
    std::fs::create_dir(&slides).unwrap();
    std::fs::create_dir(&figs).unwrap();
    let talk = write_named(&slides, "talk.pdf", 2);
    let plot = write_named(&figs, "plot.pdf", 4);
    write_named(&figs, "extra.pdf", 1);
    app.open(vec![talk, plot]);
    finish(&mut app);

    // On plot.pdf, which is not the deck's first file, nor in its folder.
    press(&mut app, &[KeyCode::Char('n'); 2]);
    app.key(ctrl('o'));
    type_in(&mut app, "ext");
    press(&mut app, &[KeyCode::Enter]);
    finish(&mut app);
    assert_eq!(
        app.deck.paths,
        [
            figs.join("extra.pdf"),
            slides.join("talk.pdf"),
            figs.join("plot.pdf")
        ]
    );
    let status = app.status().to_string();
    assert!(status.starts_with(" extra.pdf"), "{status}");
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
fn c_x_c_c_quits_with_the_list_open() {
    let (_dir, mut app) = app(3);
    app.key(ctrl('o'));
    app.key(ctrl('c'));
    assert!(!app.quit);
    app.key(ctrl('x'));
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

fn click(app: &mut App, at: Position) {
    app.mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: at.x,
        row: at.y,
        modifiers: KeyModifiers::NONE,
    });
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

#[cfg(feature = "summary")]
#[test]
fn k_shows_the_slide_s_summary_beside_it_and_k_again_hides_it() {
    let (_dir, mut app) = app(2);
    app.summarizer = Summarizer::new(Some(summary::Weights::File("/no/model.gguf".into())));
    press(&mut app, &[KeyCode::Char('k')]);
    assert_eq!(app.panel, Some(Panel::Slide));
    press(&mut app, &[KeyCode::Char('K')]);
    assert_eq!(app.panel, Some(Panel::Whole));
    press(&mut app, &[KeyCode::Char('k')]);
    assert_eq!(app.panel, Some(Panel::Slide));
    press(&mut app, &[KeyCode::Char('k')]);
    assert_eq!(app.panel, None);
    assert!(app.job(app.presented()).marked.is_empty());
}

#[test]
fn a_summary_takes_a_column_beside_the_slides() {
    let screen = Rect::new(0, 0, 140, 40);
    let (main, side, status) = split(screen, true);
    assert_eq!((main.width + side.width, side.x), (140, main.width));
    assert!((SIDE.0..=SIDE.1).contains(&side.width));
    assert_eq!((main.height, side.height, status.y), (39, 39, 39));
    let (main, side, _) = split(screen, false);
    assert_eq!((main.width, side.area()), (140, 0));
}

#[cfg(feature = "speech")]
#[path = "app/speaking.rs"]
mod speaking;

#[cfg(feature = "sign")]
#[path = "app/signing.rs"]
mod signing;

#[path = "app/printing.rs"]
mod printing;

#[path = "app/search.rs"]
mod search;

/// `app` presenting to a second window, none of which is opened.
fn staged(app: &mut App) {
    app.stage = Some(crate::link::Stage::new().unwrap());
    app.view = View::Present;
}

#[test]
fn the_presenters_strip_keeps_the_current_slide_in_its_middle() {
    let (_dir, mut app) = app(40);
    staged(&mut app);
    let boxes = app.boxes();
    let strip = app.strip(&boxes);
    assert!(strip.len() > 2 && strip.len() < 40);
    assert_eq!(strip[0].0, 0);
    app.cur = 20;
    let strip = app.strip(&boxes);
    let at = strip.iter().position(|&(i, _)| i == 20).unwrap();
    assert!(at.abs_diff(strip.len() / 2) <= 1, "{at} of {}", strip.len());
    app.cur = 39;
    assert_eq!(app.strip(&boxes).last().unwrap().0, 39);
}

#[test]
fn clicking_the_strip_or_the_next_slide_turns_to_it() {
    let (_dir, mut app) = app(10);
    staged(&mut app);
    let boxes = app.boxes();
    let (i, slot) = app.strip(&boxes)[3];
    assert!(app.stage_click(Position::new(slot.x + 1, slot.y + 1)));
    assert_eq!(app.cur, i);
    assert!(app.stage_click(Position::new(boxes.next.x + 2, boxes.next.y + 2)));
    assert_eq!(app.cur, i + 1);
    // The time pauses, and goes on; double-clicked, starts again.
    let time = Position::new(boxes.clock.x + 2, boxes.clock.y + 1);
    assert!(app.stage_click(time));
    assert!(app.clock.paused());
    std::thread::sleep(Duration::from_millis(450));
    assert!(app.stage_click(time));
    assert!(!app.clock.paused());
    app.clock.began -= Duration::from_secs(60);
    std::thread::sleep(Duration::from_millis(450));
    app.stage_click(time);
    app.stage_click(time);
    assert!(!app.clock.paused());
    assert!(app.clock.elapsed() < Duration::from_secs(1));
    press(&mut app, &[KeyCode::Char('T')]);
    app.clock.began -= Duration::from_secs(60);
    press(&mut app, &[KeyCode::Char('Z')]);
    assert!(app.clock.elapsed() < Duration::from_secs(1));
    // The notes are written.
    assert!(app.stage_click(Position::new(boxes.notes.x + 2, boxes.notes.y + 2)));
    assert!(app.writing.is_some());
}

#[test]
fn presenting_shows_the_slide_the_next_and_the_strip() {
    let (_dir, mut app) = app(10);
    staged(&mut app);
    app.cur = 4;
    let keys = app.visible();
    assert_eq!(keys[0].page, 4);
    assert_eq!(keys[1].page, 5);
    assert!(keys[2..].iter().any(|k| k.page == 4));
    let sizes = app.stage_sizes();
    assert_eq!(keys[0].size(), sizes[0]);
    assert_eq!(keys[1].size(), sizes[1]);
}

#[test]
fn b_blacks_out_the_audience_and_the_timer_pauses() {
    let (_dir, mut app) = app(3);
    staged(&mut app);
    press(&mut app, &[KeyCode::Char('b')]);
    assert!(app.black);
    press(&mut app, &[KeyCode::Char('T')]);
    let paused = app.clock.elapsed();
    std::thread::sleep(Duration::from_millis(30));
    assert_eq!(app.clock.elapsed(), paused);
    press(&mut app, &[KeyCode::Char('T'), KeyCode::Char('b')]);
    assert!(!app.black);
    // Alone, b is not taken.
    app.stage = None;
    press(&mut app, &[KeyCode::Char('b')]);
    assert!(!app.black);
}

#[test]
fn space_and_page_keys_turn_the_slides_as_clickers_send_them() {
    let (_dir, mut app) = app(5);
    press(&mut app, &[KeyCode::Char(' '), KeyCode::PageDown]);
    assert_eq!(app.cur, 2);
    press(&mut app, &[KeyCode::PageUp]);
    assert_eq!(app.cur, 1);
    press(&mut app, &[KeyCode::End]);
    assert_eq!(app.cur, 4);
    press(&mut app, &[KeyCode::Home]);
    assert_eq!(app.cur, 0);
}

fn press_ctrl(app: &mut App, c: char) {
    app.key(ctrl(c));
}

#[test]
fn only_c_x_c_c_quits() {
    let (_dir, mut app) = app(3);
    press(&mut app, &[KeyCode::Char('q')]);
    press_ctrl(&mut app, 'c');
    assert!(!app.quit);
    press_ctrl(&mut app, 'x');
    press_ctrl(&mut app, 'g');
    press_ctrl(&mut app, 'c');
    assert!(!app.quit, "C-g gives up the C-x");
    press_ctrl(&mut app, 'x');
    press_ctrl(&mut app, 'c');
    assert!(app.quit);
}

#[test]
fn picking_in_the_grid_shows_the_audience_nothing_until_enter() {
    let (_dir, mut app) = app(10);
    staged(&mut app);
    app.cur = 2;
    app.tend();
    assert_eq!(app.live, 2);
    press(
        &mut app,
        &[KeyCode::Char('g'), KeyCode::Right, KeyCode::Right],
    );
    app.tend();
    assert_eq!((app.cur, app.live), (4, 2));
    // Esc, g or C-g go back to the slide shown.
    press(&mut app, &[KeyCode::Esc]);
    assert_eq!((app.view, app.cur), (View::Present, 2));
    press(&mut app, &[KeyCode::Char('g'), KeyCode::Right]);
    press_ctrl(&mut app, 'g');
    assert_eq!((app.view, app.cur), (View::Present, 2));
    press(
        &mut app,
        &[KeyCode::Char('g'), KeyCode::Right, KeyCode::Char('g')],
    );
    assert_eq!(app.cur, 2);
    // Enter shows the one picked.
    press(
        &mut app,
        &[KeyCode::Char('g'), KeyCode::Right, KeyCode::Enter],
    );
    app.tend();
    assert_eq!((app.view, app.cur, app.live), (View::Present, 3, 3));
}

#[test]
fn notes_written_while_practising_are_saved_beside_the_deck() {
    let (dir, mut app) = app(4);
    staged(&mut app);
    app.cur = 1;
    press(&mut app, &[KeyCode::Char('N')]);
    assert!(app.writing.is_some());
    for c in "hello".chars() {
        press(&mut app, &[KeyCode::Char(c)]);
    }
    // Keys are typed, not acted on: q neither quits nor turns.
    press(
        &mut app,
        &[KeyCode::Enter, KeyCode::Char('q'), KeyCode::Char('n')],
    );
    press(&mut app, &[KeyCode::Up, KeyCode::End, KeyCode::Backspace]);
    assert_eq!(app.cur, 1);
    press(&mut app, &[KeyCode::Esc]);
    assert!(app.writing.is_none());
    let file = dir.path().join("deck.md");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "# 2\nhell\nqn\n");
    assert_eq!(app.notes[&1], "hell\nqn");
    // Written again, it starts from what was saved; C-g drops a change.
    press(&mut app, &[KeyCode::Char('N'), KeyCode::Char('!')]);
    press_ctrl(&mut app, 'g');
    assert!(app.writing.is_none());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "# 2\nhell\nqn\n");
}

#[test]
fn a_click_in_the_notes_moves_the_cursor_and_outside_saves_them() {
    let (dir, mut app) = app(4);
    staged(&mut app);
    let notes = app.boxes().notes;
    click(&mut app, Position::new(notes.x + 3, notes.y + 3));
    assert!(app.writing.is_some());
    for c in "ab\ncd".chars() {
        app.key(KeyEvent::from(if c == '\n' {
            KeyCode::Enter
        } else {
            KeyCode::Char(c)
        }));
    }
    // Before the b, on the first row of the box's inside.
    click(&mut app, Position::new(notes.x + 2, notes.y + 1));
    press(&mut app, &[KeyCode::Char('X')]);
    let current = app.boxes().current;
    click(&mut app, Position::new(current.x + 5, current.y + 5));
    assert!(app.writing.is_none());
    let file = dir.path().join("deck.md");
    assert_eq!(std::fs::read_to_string(file).unwrap(), "# 1\naXb\ncd\n");
}

#[test]
fn the_tools_are_drawn_along_the_top_and_clicked_as_keys() {
    let (_dir, mut app) = app(5);
    staged(&mut app);
    let top: String = screen(&app).chars().take(140).collect();
    for word in [
        "next", "laser", "black", "notes", "pause", "restart", "quit",
    ] {
        assert!(top.contains(word), "{word} is not on the top row: {top}");
    }
    let tools = app.tools(&app.boxes());
    let at = |tool| {
        let (_, r): (stage::Tool, Rect) = *tools.iter().find(|(t, _)| *t == tool).unwrap();
        Position::new(r.x + 1, r.y)
    };
    click(&mut app, at(stage::Tool::Next));
    assert_eq!(app.cur, 1);
    click(&mut app, at(stage::Tool::Black));
    assert!(app.black);
    click(&mut app, at(stage::Tool::Quit));
    assert!(!app.quit, "a click never quits");
    click(&mut app, at(stage::Tool::Grid));
    assert_eq!(app.view, View::Grid);
}

#[test]
fn the_laser_is_where_the_mouse_is_held_on_the_slide() {
    let (_dir, mut app) = app(3);
    staged(&mut app);
    let area = Rect::new(10, 5, 40, 20);
    app.drawn.set(Some((app.job(Key::new(0, area)), area)));
    app.mouse(MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column: 20,
        row: 15,
        modifiers: KeyModifiers::NONE,
    });
    let (x, y) = app.laser.unwrap();
    assert!((x - 10.5 / 40.).abs() < 1e-3 && (y - 10.5 / 20.).abs() < 1e-3);
    // Off the slide, it is off; let go, too.
    app.point(Position::new(2, 2));
    assert!(app.laser.is_none());
    app.point(Position::new(20, 15));
    app.mouse(MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column: 20,
        row: 15,
        modifiers: KeyModifiers::NONE,
    });
    assert!(app.laser.is_none());
}

#[test]
fn the_laser_is_a_red_dot_kept_on_the_slide() {
    use ratatui::backend::TestBackend;
    let mut terminal = ratatui::Terminal::new(TestBackend::new(30, 10)).unwrap();
    let area = Rect::new(5, 2, 20, 6);
    terminal
        .draw(|f| {
            stage::laser(f, area, (0.5, 0.5));
            stage::laser(f, area, (0.99, 0.0));
        })
        .unwrap();
    let buf = terminal.backend().buffer();
    let red = |x, y| buf[(x, y)].fg == Color::Rgb(255, 30, 30);
    assert!(red(15, 5) && red(13, 5) && red(16, 5) && red(14, 4) && red(15, 6));
    assert!(!red(12, 5) && !red(17, 5));
    // At the edge, nothing outside the slide.
    assert!(red(24, 2) && !red(25, 2) && !red(24, 1));
}

#[test]
fn another_program_is_told_what_is_shown_and_turns_the_slides() {
    use crate::control::Request;
    let (dir, mut app) = app(5);
    let status = app.answer_one(Request::Status);
    assert!(status.contains(&format!("file {}", dir.path().join("deck.pdf").display())));
    assert!(status.contains(&"page 1".to_string()) && status.contains(&"pages 5".to_string()));
    app.view = View::Grid;
    assert_eq!(
        app.answer_one(Request::Goto(3)),
        ["ok showing slide 4 of 5"]
    );
    assert_eq!((app.cur, app.view), (3, View::Present));
    assert!(app.answer_one(Request::Goto(9))[0].starts_with("err"));
    // The file shown, opened again, is turned to.
    let same = app.answer_one(Request::Open(dir.path().join("deck.pdf"), 1));
    assert_eq!(same, ["ok showing slide 2 of 5"]);
    assert!(app.answer_one(Request::Open(dir.path().join("nope.pdf"), 0))[0].starts_with("err"));
}

#[test]
fn w_ends_presenting_though_no_audience_window_came() {
    let (_dir, mut app) = app(3);
    staged(&mut app);
    press(&mut app, &[KeyCode::Char('w')]);
    assert!(!app.presenting());
}

#[test]
fn presenting_neither_prints_nor_annotates() {
    let (_dir, mut app) = app(3);
    staged(&mut app);
    press(&mut app, &[KeyCode::Char('P'), KeyCode::Char('A')]);
    app.key(ctrl('p'));
    assert!(app.printing.is_none());
    assert!(!app.signing());
    assert!(app.notice.as_deref().unwrap().contains("w ends presenting"));
}

#[test]
fn quitting_saves_the_notes_being_written_and_stays_if_it_cannot() {
    let (dir, mut app) = app(3);
    staged(&mut app);
    press(
        &mut app,
        &[KeyCode::Char('N'), KeyCode::Char('h'), KeyCode::Char('i')],
    );
    app.key(ctrl('x'));
    app.key(ctrl('c'));
    assert!(app.quit);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("deck.md")).unwrap(),
        "# 1\nhi\n"
    );
}

#[test]
fn notes_that_cannot_be_saved_are_kept_and_ohp_stays() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, mut app) = app(3);
    staged(&mut app);
    let mode = |m| std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(m));
    mode(0o555).unwrap();
    press(
        &mut app,
        &[KeyCode::Char('N'), KeyCode::Char('x'), KeyCode::Esc],
    );
    let kept = app.writing.is_some();
    app.key(ctrl('x'));
    app.key(ctrl('c'));
    let stayed = !app.quit;
    mode(0o755).unwrap();
    assert!(kept, "the notes are kept to save again");
    assert!(stayed);
    app.key(ctrl('g'));
    assert!(app.writing.is_none());
}

#[test]
fn notes_written_elsewhere_are_shown_while_the_time_is_paused() {
    let (dir, mut app) = app(3);
    staged(&mut app);
    app.read_notes();
    press(&mut app, &[KeyCode::Char('T')]);
    std::fs::write(dir.path().join("deck.md"), "# 1\nfrom an editor\n").unwrap();
    app.notes_looked -= Duration::from_secs(2);
    app.tend();
    assert_eq!(app.notes[&0], "from an editor");
}

#[test]
fn a_slide_asked_of_a_file_that_cannot_be_read_is_forgotten() {
    let (dir, mut app) = app(3);
    let bad = dir.path().join("bad.pdf");
    std::fs::write(&bad, b"not a pdf").unwrap();
    app.answer_one(crate::control::Request::Open(bad, 2));
    assert_eq!(app.landing, Some(2));
    finish(&mut app);
    assert_eq!(app.landing, None);
}
