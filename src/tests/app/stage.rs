use super::*;

fn writing(text: &str, at: usize) -> Writing {
    Writing {
        slide: 0,
        page: 0,
        file: "notes.md".into(),
        text: text.into(),
        at,
    }
}

fn shown(lines: &[Line]) -> Vec<String> {
    lines.iter().map(|l| l.to_string()).collect()
}

#[test]
fn a_long_line_is_broken_between_words() {
    let w = writing("one two three\nfour", 0);
    let rows: Vec<String> = w
        .rows(8)
        .into_iter()
        .map(|r| w.text.chars().collect::<Vec<_>>()[r].iter().collect())
        .collect();
    assert_eq!(rows, ["one two ", "three", "four"]);
    // A word longer than a row is cut where the row ends.
    assert_eq!(writing("abcdefghij", 0).rows(4), [0..4, 4..8, 8..10]);
}

#[test]
fn a_click_puts_the_cursor_on_the_row_and_column_clicked() {
    let mut w = writing("one two three\nfour", 0);
    w.click(2, 1, 8);
    assert_eq!(w.at, 10);
    // Past a row's end, at its end; past the last row, on it.
    w.click(30, 0, 8);
    assert_eq!(w.at, 8);
    w.click(1, 9, 8);
    assert_eq!(w.at, 15);
}

#[test]
fn the_cursor_is_shown_on_one_row_alone() {
    let w = writing("ab\ncd", 2);
    assert_eq!(shown(&w.lines(10)), ["ab ", "cd"]);
    let w = writing("ab\ncd", 5);
    assert_eq!(shown(&w.lines(10)), ["ab", "cd "]);
    let w = writing("", 0);
    assert_eq!(shown(&w.lines(10)), [" "]);
}

#[test]
fn up_and_down_keep_the_column() {
    let mut w = writing("abcd\nef\nghij", 3);
    w.down();
    assert_eq!(w.at, 7);
    w.down();
    assert_eq!(w.at, 10);
    w.up();
    w.up();
    assert_eq!(w.at, 2);
    w.kill();
    assert_eq!(w.text, "ab\nef\nghij");
}

#[cfg(feature = "markdown")]
#[test]
fn a_note_is_shown_as_markdown() {
    let lines = shown_markdown(
        "# Say\n\nIt is **bold** and `code`.\n\n- one\n- two\n\n1. first\n2. second\n",
    );
    let text = shown(&lines);
    assert_eq!(
        text,
        [
            "Say",
            "",
            "It is bold and code.",
            "",
            "• one",
            "• two",
            "",
            "1. first",
            "2. second"
        ]
    );
    let bold = lines[2].spans.iter().find(|s| s.content == "bold").unwrap();
    assert!(bold.style.add_modifier.contains(Modifier::BOLD));
}

#[test]
fn the_cursors_row_is_found_to_scroll_to() {
    let w = writing("one two three\nfour", 18);
    assert_eq!(w.cursor_row(8), 2);
    let w = writing("one two three\nfour", 2);
    assert_eq!(w.cursor_row(8), 0);
    let w = writing("one two three\nfour", 8);
    assert_eq!(w.cursor_row(8), 1);
}
