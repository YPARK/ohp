use super::pages;

#[test]
fn a_documents_pages_are_drawn_as_wide_as_asked() {
    let dir = tempfile::tempdir().unwrap();
    let md = dir.path().join("note.md");
    std::fs::write(&md, "# A note\n\nIt says a little, on one page.\n").unwrap();
    let drawn = pages(&md, 600).unwrap();
    assert!(!drawn.is_empty());
    assert!(drawn.iter().all(|p| p.width() == 600 && p.height() > 0));
    // Not white all over: something is written on it.
    let page = &drawn[0];
    assert!(page.pixels().any(|p| p.0[0] < 128), "ink on the page");
}

#[test]
fn an_html_page_is_typeset_as_its_markdown() {
    let dir = tempfile::tempdir().unwrap();
    let html = dir.path().join("mail.html");
    std::fs::write(
        &html,
        "<html><body><h1>Seminar</h1><p>Thursday, room 4.</p></body></html>",
    )
    .unwrap();
    let drawn = pages(&html, 400).unwrap();
    assert!(!drawn.is_empty() && drawn[0].width() == 400);
}

#[test]
fn nothing_to_draw_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let gone = dir.path().join("gone.md");
    assert!(pages(&gone, 400).is_err());
}
