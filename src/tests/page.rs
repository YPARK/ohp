use super::pages;

#[cfg(feature = "markdown")]
#[test]
fn a_documents_pages_are_drawn_as_wide_as_asked() {
    let dir = tempfile::tempdir().unwrap();
    let md = crate::fixture::write_text(
        dir.path(),
        "note.md",
        "# A note\n\nIt says a little, on one page.\n",
    );
    let drawn = pages(&md, 600).unwrap();
    assert!(!drawn.is_empty());
    assert!(drawn.iter().all(|p| p.width() == 600 && p.height() > 0));
    // Not white all over: something is written on it.
    let page = &drawn[0];
    assert!(page.pixels().any(|p| p.0[0] < 128), "ink on the page");
}

#[cfg(feature = "markdown")]
#[test]
fn an_html_page_is_typeset_as_its_markdown() {
    let dir = tempfile::tempdir().unwrap();
    let html = crate::fixture::write_text(
        dir.path(),
        "mail.html",
        "<html><body><h1>Seminar</h1><p>Thursday, room 4.</p></body></html>",
    );
    let drawn = pages(&html, 400).unwrap();
    assert!(!drawn.is_empty() && drawn[0].width() == 400);
}

#[test]
fn nothing_to_draw_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let gone = dir.path().join("gone.md");
    assert!(pages(&gone, 400).is_err());
}

#[cfg(not(feature = "markdown"))]
#[test]
fn without_markdown_a_document_says_it_cannot_be_drawn() {
    let dir = tempfile::tempdir().unwrap();
    let md = crate::fixture::write_text(dir.path(), "note.md", "# A note\n");
    let err = pages(&md, 600).unwrap_err();
    assert!(format!("{err:#}").contains("`markdown` feature"), "{err:#}");
}
