use super::*;
use ratatui::layout::Rect;

/// A row of a kitty image's placeholders, as ratatui-image writes it: its
/// first cell marked with row 3, column 0 and the id's byte 2.
fn row(buf: &mut Buffer, left: u16, width: u16) {
    for x in left..left + width {
        let symbol: String = if x == left {
            [PLACEHOLDER, DIACRITICS[3], DIACRITICS[0], DIACRITICS[2]]
                .iter()
                .collect()
        } else {
            PLACEHOLDER.to_string()
        };
        buf[(x, 0)].set_symbol(&symbol);
    }
}

#[test]
fn a_row_drawn_over_is_marked_where_the_image_goes_on() {
    let mut buf = Buffer::empty(Rect::new(0, 0, 20, 1));
    row(&mut buf, 2, 10);
    buf[(5, 0)].set_symbol("█");
    buf[(6, 0)].set_symbol("█");
    repair(&mut buf);
    let marked: Vec<char> = buf[(7, 0)].symbol().chars().collect();
    assert_eq!(
        marked,
        [PLACEHOLDER, DIACRITICS[3], DIACRITICS[5], DIACRITICS[2]]
    );
    // Those it follows on from are left as they were.
    assert_eq!(buf[(8, 0)].symbol(), PLACEHOLDER.to_string());
    assert_eq!(buf[(4, 0)].symbol(), PLACEHOLDER.to_string());
}
