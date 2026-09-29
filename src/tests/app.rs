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
    let mut app = App::new(deck, picker, renderer, 1, look);
    app.main = MAIN;
    (dir, app)
}

fn job(key: Key, look: Look) -> Job {
    Job { key, look }
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

    due(&mut app);
    app.watch();
    assert_eq!(app.deck.pages, 3, "reloaded before the file settled");
    assert_eq!(app.notice.as_deref(), Some("PDF changing…"));

    due(&mut app);
    app.watch();
    assert_eq!(app.deck.pages, 5);
    assert_eq!(app.notice.as_deref(), Some("reloaded"));
}

#[test]
fn a_half_written_pdf_is_not_reloaded() {
    let (dir, mut app) = app(3);
    let whole = fixture::pdf(5);
    std::fs::write(dir.path().join("deck.pdf"), &whole[..whole.len() / 2]).unwrap();
    for _ in 0..3 {
        due(&mut app);
        app.watch();
    }
    assert_eq!(app.deck.pages, 3);
    assert_eq!(app.notice.as_deref(), Some("PDF changing…"));
}

#[test]
fn a_broken_pdf_keeps_the_old_slides() {
    let (dir, mut app) = app(3);
    std::fs::write(dir.path().join("deck.pdf"), b"%PDF-1.5\ngarbage\n%%EOF\n").unwrap();
    for _ in 0..2 {
        due(&mut app);
        app.watch();
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
        due(&mut app);
        app.watch();
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
        due(&mut app);
        app.watch();
    }
    assert!(app.slides.is_empty());
    assert!(app.stale.contains_key(&key));
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
