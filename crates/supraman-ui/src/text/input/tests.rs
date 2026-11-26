use super::*;

fn line(text: &str, cursor: usize) -> LineText {
    let mut l = LineText::new(text);
    l.move_to(cursor);
    l
}

fn typed(l: &mut LineText, text: &str) {
    for c in text.chars() {
        l.replace(l.selection(), &c.to_string(), EditKind::Typing);
    }
}

#[test]
fn the_text_has_room_inside_the_border() {
    assert!(PAD_X >= px(6.) && PAD_Y >= px(2.), "padding {PAD_X:?} × {PAD_Y:?}");
    assert_eq!(TEXT_LEFT, px(1.) + PAD_X);
    assert_eq!(SINGLE_HEIGHT, LINE_HEIGHT + PAD_Y * 2. + px(2.));
}

#[test]
fn typing_replaces_the_selection_and_moves_the_caret() {
    let mut l = line("acd", 1);
    typed(&mut l, "b");
    assert_eq!((l.text(), l.cursor()), ("abcd", 2));
    l.select(1..3, false);
    typed(&mut l, "X");
    assert_eq!((l.text(), l.selection()), ("aXd", 2..2));
}

#[test]
fn select_to_extends_from_the_anchor_on_either_side() {
    let mut l = line("abcdef", 3);
    l.select_to(5);
    assert_eq!((l.selection(), l.cursor()), (3..5, 5));
    l.select_to(1);
    assert_eq!((l.selection(), l.cursor()), (1..3, 1));
    l.select_to(4);
    assert_eq!((l.selection(), l.cursor()), (3..4, 4));
    l.select_all();
    assert_eq!(l.selection(), 0..6);
}

#[test]
fn arrows_step_over_whole_graphemes() {
    let l = LineText::new("ae\u{301}z");
    assert_eq!(l.next_grapheme(1), 4, "e + combining acute is one step");
    assert_eq!(l.prev_grapheme(4), 1);
    assert_eq!(l.prev_grapheme(0), 0);
    assert_eq!(l.next_grapheme(5), 5);
}

#[test]
fn ctrl_arrows_jump_words_over_separators() {
    let l = LineText::new("selftest-added  foo_bar.rs");
    assert_eq!(l.word_right(0), 8);
    assert_eq!(l.word_right(8), 14);
    assert_eq!(l.word_right(14), 23);
    assert_eq!(l.word_right(23), 26);
    assert_eq!(l.word_left(26), 24);
    assert_eq!(l.word_left(24), 16);
    assert_eq!(l.word_left(16), 9);
    assert_eq!(l.word_left(9), 0);
    assert_eq!(l.word_left(0), 0);
    let umlaut = LineText::new("grüße welt");
    assert_eq!(umlaut.word_right(0), "grüße".len());
}

#[test]
fn a_double_click_selects_the_word_or_the_separator_run() {
    let l = LineText::new("selftest-added  x");
    assert_eq!(l.word_at(3), 0..8);
    assert_eq!(l.word_at(10), 9..14);
    assert_eq!(l.word_at(8), 8..9);
    assert_eq!(l.word_at(15), 14..16);
    assert_eq!(l.word_at(17), 16..17, "past the end: the last word");
    assert_eq!(LineText::new("").word_at(0), 0..0);
}

#[test]
fn undo_takes_back_a_typed_run_at_once_and_redo_brings_it_back() {
    let mut l = LineText::new("");
    typed(&mut l, "abc");
    typed(&mut l, " de");
    assert_eq!(l.text(), "abc de");
    assert!(l.undo());
    assert_eq!(l.text(), "abc", "a space starts a new step");
    assert!(l.undo());
    assert_eq!((l.text(), l.cursor()), ("", 0));
    assert!(!l.undo());
    assert!(l.redo());
    assert!(l.redo());
    assert_eq!(l.text(), "abc de");
    l.move_to(1);
    typed(&mut l, "X");
    assert!(!l.redo(), "an edit drops the redo history");
    l.undo();
    assert_eq!(l.text(), "abc de", "moving the caret ends the typed run");
}

#[test]
fn undo_keeps_the_newest_steps_only() {
    let mut l = LineText::new("");
    for _ in 0..UNDO_STEPS + 20 {
        typed(&mut l, " x");
    }
    let mut steps = 0;
    while l.undo() {
        steps += 1;
    }
    assert_eq!(steps, UNDO_STEPS);
    assert_eq!(l.text().len(), 2 * 20, "the oldest steps went first");
}

#[test]
fn a_composition_stays_marked_and_commits_as_one_undo_step() {
    let mut l = LineText::new("A");
    l.replace_marked(1..1, "ka", Some(2..2));
    assert_eq!((l.text(), l.marked(), l.selection()), ("Aka", Some(1..3), 3..3));
    l.replace_marked(1..3, "か", Some(3..3));
    assert_eq!((l.text(), l.marked()), ("Aか", Some(1..4)));
    l.replace(l.marked().unwrap(), "仮", EditKind::Typing);
    assert_eq!((l.text(), l.marked(), l.cursor()), ("A仮", None, 4));
    l.undo();
    assert_eq!(l.text(), "A");
}

/// Rows as a box `width` characters wide would wrap them: at the last space that fits, else mid-word.
fn wrapped(text: &str, width: usize) -> LineRows {
    let mut rows = Vec::new();
    let mut start = 0;
    for line in text.split('\n') {
        let mut from = 0;
        while line.len() - from > width {
            let cut = line[from..from + width].rfind(' ').map_or(from + width, |i| from + i + 1);
            rows.push(start + from..start + cut);
            from = cut;
        }
        rows.push(start + from..start + line.len());
        start += line.len() + 1;
    }
    LineRows { rows, text: text.to_string() }
}

#[test]
fn up_and_down_keep_the_column_across_shorter_lines() {
    let text = "abcdef\nxy\nlonger line";
    let rows = LineRows::new(text);
    let mut l = line(text, 4);
    l.move_vertically(&rows, 1, false);
    assert_eq!(l.cursor(), 9, "line 2 is shorter: its end");
    l.move_vertically(&rows, 1, false);
    assert_eq!(l.cursor(), 14, "back at column 4 on line 3");
    l.move_vertically(&rows, -1, false);
    l.move_vertically(&rows, -1, false);
    assert_eq!(l.cursor(), 4);
    l.move_vertically(&rows, -1, false);
    assert_eq!(l.cursor(), 0, "Up on the first row: the start");
    l.move_to(12);
    l.move_vertically(&rows, 1, false);
    assert_eq!(l.cursor(), text.len(), "Down on the last row: the end");
    l.move_to(14);
    l.move_vertically(&rows, -1, false);
    l.move_to(l.cursor());
    l.move_vertically(&rows, 1, false);
    assert_eq!(l.cursor(), 12, "a horizontal move resets the column: 2 on line 2 is 2 on line 3");
}

#[test]
fn home_end_and_vertical_moves_follow_wrapped_rows() {
    let text = "abcd efgh ij\nxyz";
    let rows = wrapped(text, 5);
    assert_eq!(rows.rows(), &[0..5, 5..10, 10..12, 13..16]);
    assert_eq!(row_of(rows.rows(), 5), 1, "a wrap offset is the next row's start");
    assert_eq!(row_of(rows.rows(), 12), 2, "a line's end is its own row's");
    let mut l = line(text, 7);
    assert_eq!((l.row_start(rows.rows()), l.row_end(rows.rows())), (5, 9), "End stops before the wrap's space");
    l.move_vertically(&rows, -1, false);
    assert_eq!(l.cursor(), 2);
    l.move_vertically(&rows, 1, false);
    l.move_vertically(&rows, 1, false);
    assert_eq!(l.cursor(), 12, "row 3 is two characters: its end");
    l.move_vertically(&rows, 1, false);
    assert_eq!(l.cursor(), 15, "column 2 again on the next line");
    assert_eq!((l.row_start(rows.rows()), l.row_end(rows.rows())), (13, 16));
}

#[test]
fn shift_up_and_down_select_across_lines() {
    let text = "first line\nsecond";
    let rows = LineRows::new(text);
    let mut l = line(text, 3);
    l.move_vertically(&rows, 1, true);
    assert_eq!((l.selection(), l.cursor()), (3..14, 14));
    assert_eq!(&l.text()[l.selection()], "st line\nsec");
    l.move_vertically(&rows, -1, true);
    assert_eq!(l.selection(), 3..3, "back to the anchor");
    l.move_vertically(&rows, -1, true);
    assert_eq!((l.selection(), l.cursor()), (0..3, 0), "above the first row: to the start");
    l.select_all();
    l.replace(l.selection(), "X", EditKind::Typing);
    assert_eq!(l.text(), "X", "typing replaces a selection over both lines");
}

#[test]
fn a_paste_keeps_its_lines_on_several_and_joins_them_on_one() {
    assert_eq!(pasted("a\r\nb\rc\n", true), "a\nb\nc\n");
    assert_eq!(pasted("1\r\n2\n", false), "1 2");
    let mut l = line("ab\ncd", 1);
    l.select(1..4, false);
    l.replace(l.selection(), &pasted("X\r\nY", true), EditKind::Other);
    assert_eq!((l.text(), l.cursor()), ("aX\nYd", 4));
    let rows = LineRows::new(l.text());
    assert_eq!((l.row_start(rows.rows()), l.row_end(rows.rows())), (3, 5), "the caret is on the pasted second line");
    l.undo();
    assert_eq!(l.text(), "ab\ncd", "one undo step");
}

#[test]
fn word_moves_cross_line_breaks() {
    let l = LineText::new("fix the\nbuild now");
    assert_eq!(l.word_right(7), 13, "over the newline to the end of `build`");
    assert_eq!(l.word_left(8), 4, "from the start of line 2 to `the`");
    assert_eq!(l.word_at(7), 7..8, "a double-click on the newline selects only it");
}

#[test]
fn utf16_offsets_map_to_bytes_and_back() {
    let l = LineText::new("e🙂漢");
    assert_eq!(l.range_from_utf16(&(1..3)), 1..5);
    assert_eq!(l.range_to_utf16(&(1..8)), 1..4);
    assert_eq!(l.offset_from_utf16(99), 8);
}

#[test]
fn the_filter_maps_and_reports_the_first_refused_character() {
    let lower = |c: char| c.is_ascii_alphabetic().then(|| c.to_ascii_lowercase());
    assert_eq!(filtered("AbC", lower), ("abc".to_string(), None));
    assert_eq!(filtered("a1ä", lower), ("a".to_string(), Some('1')));
}
