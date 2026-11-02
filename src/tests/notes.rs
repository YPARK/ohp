use super::*;

#[test]
fn numbered_headings_begin_each_pages_notes() {
    let text =
        "intro, ignored\n# 1\nWelcome.\n\n## Slide 3\nThe figure\nis new.\n# page 4\n\n# 5\n";
    assert_eq!(
        parse(text),
        vec![(0, "Welcome.".into()), (2, "The figure\nis new.".into())]
    );
}

#[test]
fn other_headings_are_part_of_the_note() {
    let text = "# 2\n## Why\nbecause\n";
    assert_eq!(parse(text), vec![(1, "## Why\nbecause".into())]);
}

#[test]
fn notes_for_one_page_are_joined() {
    let mut notes = Notes::new();
    add(&mut notes, 3, "one".into());
    add(&mut notes, 3, "  ".into());
    let notes = merge(notes, Notes::from([(3, "two".into()), (4, "x".into())]));
    assert_eq!(notes[&3], "one\n\ntwo");
    assert_eq!(notes[&4], "x");
}

#[test]
fn a_notes_file_beside_the_deck_is_read_from_its_first_page() {
    let dir = tempfile::tempdir().unwrap();
    let pdf = dir.path().join("talk.pdf");
    std::fs::write(dir.path().join("talk.notes.md"), "# 2\nsecond\n").unwrap();
    let other = dir.path().join("more.pdf");
    let mut beside = Beside::new(&[pdf.clone(), other.clone()], &[0, 10]);
    assert_eq!(beside.read(), Notes::from([(1, "second".into())]));
    assert!(!beside.changed(&[pdf.clone(), other.clone()]));
    std::fs::write(dir.path().join("more.notes"), "# 1\nfirst of more\n").unwrap();
    assert!(beside.changed(&[pdf, other]));
    assert_eq!(beside.read()[&10], "first of more");
}

#[test]
fn a_pages_notes_are_written_in_place_among_the_others() {
    let text = "Notes for the talk.\n\n# 1\nfirst\n\n## Slide 3\nthird\n";
    let out = rewrite(text, 1, "second\nline");
    assert_eq!(
        out,
        "Notes for the talk.\n\n# 1\nfirst\n\n# 2\nsecond\nline\n\n## Slide 3\nthird\n"
    );
    assert_eq!(parse(&out)[1], (1, "second\nline".into()));
    let out = rewrite(&out, 2, "THIRD");
    assert_eq!(parse(&out)[2], (2, "THIRD".into()));
    let out = rewrite(&out, 0, "  ");
    assert_eq!(parse(&out).len(), 2);
    assert!(out.starts_with("Notes for the talk.\n\n# 2"), "{out}");
    assert_eq!(rewrite("", 4, "hi"), "# 5\nhi\n");
}

#[test]
fn notes_are_saved_beside_the_deck() {
    let dir = tempfile::tempdir().unwrap();
    let pdf = dir.path().join("talk.pdf");
    let file = file_for(&pdf);
    assert_eq!(file, dir.path().join("talk.md"));
    save(&file, 2, "remember this").unwrap();
    assert_eq!(written(&file, 2), "remember this");
    assert_eq!(Beside::new(&[pdf], &[0]).read()[&2], "remember this");
}

#[test]
fn a_markdown_file_named_as_the_slides_is_notes_only_if_begun_by_a_page() {
    assert!(is_notes(""));
    assert!(is_notes("Notes for the talk\n\n# 1\nhi\n"));
    assert!(!is_notes("---\ntitle: Talk\n---\n# Introduction\n"));
    assert!(!is_notes("Some slides, split by rules\n\n---\n"));
    let dir = tempfile::tempdir().unwrap();
    let pdf = dir.path().join("talk.pdf");
    let md = dir.path().join("talk.md");
    // What the PDF was made from: never written into.
    std::fs::write(&md, "# Introduction\nhello\n").unwrap();
    assert_eq!(beside(&pdf), None);
    assert_eq!(file_for(&pdf), dir.path().join("talk.notes.md"));
    std::fs::write(&md, "# 2\nsecond\n").unwrap();
    assert_eq!(beside(&pdf), Some(md.clone()));
    // Markdown slides keep their notes apart from themselves.
    assert_eq!(file_for(&md), dir.path().join("talk.notes.md"));
}
