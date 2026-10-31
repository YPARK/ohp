//! Printing what is shown: the pages, copies, sides and printer picked in
//! the status line, and the PDF handed to `lp`.

use super::*;

/// An app on a 3-page deck printing, to a fake `lp` in its own folder,
/// with two printers to pick from.
fn printing() -> (tempfile::TempDir, App) {
    let (dir, mut app) = app(3);
    press(&mut app, &[KeyCode::Char('P')]);
    let p = app.printing.as_mut().expect("the print line open");
    // Not what CUPS here says, as it says it.
    p.lookup = None;
    p.printers = vec!["Office".into(), "Label".into()];
    p.programs = vec![fixture::fake_lp(dir.path()).into()];
    (dir, app)
}

fn args(dir: &tempfile::TempDir) -> String {
    std::fs::read_to_string(dir.path().join("args")).unwrap_or_default()
}

#[test]
fn enter_prints_what_is_shown_on_the_default_printer() {
    let (dir, mut app) = printing();
    press(&mut app, &[KeyCode::Enter]);
    assert!(app.printing.is_none());
    assert_eq!(app.notice.as_deref(), Some("sent to the printer: Office-7"));
    let printed = std::fs::read(dir.path().join("printed.pdf")).unwrap();
    assert_eq!(printed, *app.deck.data);
    let args = args(&dir);
    assert!(args.starts_with("-d\nOffice\n"), "{args}");
    assert!(!args.contains("page-ranges"), "{args}");
    assert!(!args.contains("sides="), "the printer's own: {args}");
}

#[test]
fn pages_copies_sides_and_printer_are_picked_in_the_form() {
    let (dir, mut app) = printing();
    app.cur = 1;
    // "all" picked, typed over; then a row down each, changed with →.
    let typed = "this".chars().map(KeyCode::Char);
    let keys: Vec<KeyCode> = typed
        .chain([
            KeyCode::Down,
            KeyCode::Right,
            KeyCode::Right,
            KeyCode::Down,
            KeyCode::Right,
            KeyCode::Right,
            KeyCode::Down,
            KeyCode::Down,
            KeyCode::Right,
        ])
        .collect();
    press(&mut app, &keys);
    let p = app.printing.as_ref().unwrap();
    use crate::app::printing::Row;
    assert_eq!(p.value(Row::Copies), "3");
    assert_eq!(p.value(Row::Sides), "two-sided");
    assert_eq!(p.value(Row::Printer), "Label");
    press(&mut app, &[KeyCode::Enter]);
    let args = args(&dir);
    for want in [
        "-d\nLabel\n",
        "-n\n3\n",
        "page-ranges=2\n",
        "sides=two-sided-long-edge",
    ] {
        assert!(args.contains(want), "{want} not in {args}");
    }
}

#[test]
fn a_click_on_a_row_picks_it_and_again_changes_it() {
    let (_dir, mut app) = printing();
    app.side.set(Rect::new(110, 0, 30, 39));
    let click = |app: &mut App, y| {
        app.mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 115,
            row: y,
            modifiers: KeyModifiers::NONE,
        })
    };
    // Past the border and a blank line: the sides, the 3rd row.
    click(&mut app, 4);
    use crate::app::printing::Row;
    assert_eq!(app.printing.as_ref().unwrap().row, Row::Sides);
    click(&mut app, 4);
    assert_eq!(
        app.printing.as_ref().unwrap().value(Row::Sides),
        "one-sided"
    );
}

#[test]
fn ctrl_p_opens_the_form_as_p_does() {
    let (_dir, mut app) = app(2);
    app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    assert!(app.printing.is_some());
    assert!(app.side_open());
}

#[test]
fn pages_not_there_are_said_and_the_line_stays_open() {
    let (dir, mut app) = printing();
    press(&mut app, &[KeyCode::Char('9'), KeyCode::Enter]);
    assert!(app.printing.is_some());
    assert!(app.notice.as_deref().unwrap().contains("no page 9"));
    assert!(args(&dir).is_empty());
}

#[test]
fn esc_prints_nothing() {
    let (dir, mut app) = printing();
    press(&mut app, &[KeyCode::Esc]);
    assert!(app.printing.is_none());
    assert_eq!(app.notice.as_deref(), Some("not printed"));
    assert!(args(&dir).is_empty());
}

#[test]
fn with_no_printer_listed_lp_s_default_is_used() {
    let (dir, mut app) = printing();
    app.printing.as_mut().unwrap().printers.clear();
    let p = app.printing.as_ref().unwrap();
    assert_eq!(
        p.value(crate::app::printing::Row::Printer),
        "the default printer"
    );
    press(&mut app, &[KeyCode::Enter]);
    assert!(!args(&dir).contains("-d\n"), "{}", args(&dir));
}

#[test]
fn a_zoomed_slide_is_dragged_to_pan() {
    let (_dir, mut app) = app(1);
    press(&mut app, &[KeyCode::Char('+'), KeyCode::Char('+')]);
    let was = app.zoom;
    let at = |kind, x, y| MouseEvent {
        kind,
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    };
    app.mouse(at(MouseEventKind::Down(MouseButton::Left), 60, 20));
    app.mouse(at(MouseEventKind::Drag(MouseButton::Left), 50, 15));
    assert_eq!((app.zoom.x, app.zoom.y), (was.x + 10, was.y + 5));
    app.mouse(at(MouseEventKind::Up(MouseButton::Left), 50, 15));
    app.mouse(at(MouseEventKind::Drag(MouseButton::Left), 0, 0));
    assert_eq!(
        (app.zoom.x, app.zoom.y),
        (was.x + 10, was.y + 5),
        "let go, no more"
    );
}

#[test]
fn the_preview_shows_each_page_picked_on_the_paper_as_it_prints() {
    let (_dir, mut app) = printing();
    app.main = MAIN;
    let p = app.printing.as_mut().unwrap();
    p.paper = Some(("Letter".into(), (612., 792.)));
    p.pages = super::field::Field::offered("2-3");
    let first = app.preview().unwrap();
    assert_eq!((first.key.page, first.at, first.of), (1, 0, 2));
    // 16:9 pages, across a Letter sheet, shrunk to it.
    assert!(first.sheet.width > first.sheet.height);
    assert!(first.page.width < first.sheet.width);
    assert!(first.sheet.contains(first.page.as_position()));
    press(&mut app, &[KeyCode::Char('n')]);
    assert_eq!(app.preview().unwrap().key.page, 2);
    // No further than the last.
    press(&mut app, &[KeyCode::Char('n'), KeyCode::Char('n')]);
    assert_eq!(app.preview().unwrap().key.page, 2);
    press(&mut app, &[KeyCode::Char('p')]);
    assert_eq!(app.preview().unwrap().key.page, 1);
    // What is rendered is the page as previewed.
    assert_eq!(app.visible(), [app.preview().unwrap().key]);
}

#[test]
fn fitting_to_the_paper_previews_the_page_larger() {
    let (_dir, mut app) = printing();
    app.main = MAIN;
    let p = app.printing.as_mut().unwrap();
    // A small card: as it is, unless fitted.
    p.paper = Some(("Tabloid".into(), (792., 1224.)));
    let auto = app.preview().unwrap().page;
    let p = app.printing.as_mut().unwrap();
    p.row = crate::app::printing::Row::Scale;
    press(&mut app, &[KeyCode::Right]);
    assert_eq!(
        app.printing
            .as_ref()
            .unwrap()
            .value(crate::app::printing::Row::Scale),
        "fit to paper"
    );
    let fit = app.preview().unwrap().page;
    assert!(fit.width > auto.width, "{auto:?} {fit:?}");
    press(&mut app, &[KeyCode::Esc]);
    assert!(app.preview().is_none());
}

#[test]
fn printers_and_paper_found_off_the_screen_s_thread_are_taken_in() {
    let (_dir, mut app) = printing();
    let (tx, rx) = std::sync::mpsc::channel();
    app.printing.as_mut().unwrap().lookup = Some(rx);
    use crate::app::printing::Row;
    assert_eq!(
        app.printing.as_ref().unwrap().value(Row::Printer),
        "Office (default)"
    );
    app.found_printers();
    // Nothing said yet: as it was.
    assert!(app.printing.as_ref().unwrap().lookup.is_some());
    tx.send(crate::app::printing::Lookup {
        printers: Some(vec!["Lab".into()]),
        paper: Some(("A4".into(), (595., 842.))),
    })
    .unwrap();
    app.found_printers();
    let p = app.printing.as_ref().unwrap();
    assert!(p.lookup.is_none());
    assert_eq!(p.value(Row::Printer), "Lab (default)");
    assert_eq!(p.paper.as_ref().unwrap().0, "A4");
}
