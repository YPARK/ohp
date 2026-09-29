use super::*;
use crate::fixture;
use ratatui::crossterm::event::KeyEvent;

/// Screen area above the status line, for a 140×40 terminal.
const MAIN: Rect = Rect::new(0, 0, 140, 39);

/// An app on a `pages`-page deck, with halfblocks' fixed 10×20 pixel cells.
fn app(pages: usize) -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let deck = Deck::open(&fixture::write(dir.path(), pages)).unwrap();
    let picker = Picker::halfblocks();
    let renderer = Renderer::spawn(&deck, &picker, 1).unwrap();
    let mut app = App::new(deck, picker, renderer, 1);
    app.main = MAIN;
    (dir, app)
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
    press(&mut app, &[KeyCode::Char('n'), KeyCode::Right, KeyCode::Down]);
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
        assert!((first..first + grid.rows * grid.cols).contains(&cur), "slide {cur}");
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
    let thumb = |page| Key { page, cols: inner.width, rows: inner.height };
    let full = |page| Key::new(page, MAIN);
    let old_zoom = Key { page: 0, cols: inner.width + 5, rows: inner.height + 3 };
    for key in [thumb(7), full(1), full(20), old_zoom] {
        app.slides.insert(key, Slide::Failed);
    }
    app.schedule();
    assert!(app.slides.contains_key(&thumb(7)));
    assert!(app.slides.contains_key(&full(1)));
    assert!(!app.slides.contains_key(&full(20)));
    assert!(!app.slides.contains_key(&old_zoom));
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
    assert!(app.notice.as_deref().is_some_and(|n| n.starts_with("reload failed")));
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
    let key = Key::new(0, MAIN);
    app.slides.insert(key, Slide::Failed);
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
