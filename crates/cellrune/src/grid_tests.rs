use super::{GridAxis, GridEdit, GridHistory, RangeRule, intersect, subtract};
use crate::{CellAddress, CellRange};

fn range(text: &str) -> CellRange {
    let (start, end) = text.split_once(':').unwrap_or((text, text));
    CellRange::new(
        CellAddress::from_a1(start).expect("start"),
        CellAddress::from_a1(end).expect("end"),
    )
    .expect("range")
}

fn spelled(range: CellRange) -> String {
    if range.start() == range.end() {
        range.start().to_string()
    } else {
        format!("{}:{}", range.start(), range.end())
    }
}

fn mapped(edit: GridEdit, text: &str, rule: RangeRule) -> Option<String> {
    edit.map_range(range(text), rule).map(spelled)
}

const fn insert_rows(at: u32, count: u32) -> GridEdit {
    GridEdit::Insert {
        axis: GridAxis::Rows,
        at,
        count,
    }
}

const fn delete_rows(first: u32, last: u32) -> GridEdit {
    GridEdit::Delete {
        axis: GridAxis::Rows,
        first,
        last,
    }
}

const fn move_rows(first: u32, last: u32, destination: u32) -> GridEdit {
    GridEdit::Move {
        axis: GridAxis::Rows,
        first,
        last,
        destination,
    }
}

#[test]
fn inserted_rows_grow_ranges_they_fall_inside_and_move_later_ones() {
    let edit = insert_rows(3, 2);
    for (before, after) in [
        ("A1:A2", "A1:A2"),
        ("A1:A3", "A1:A5"),
        ("A2:A5", "A2:A7"),
        ("A3:A5", "A5:A7"),
        ("B4", "B6"),
        ("A1:A1048576", "A1:A1048576"),
        ("A1:A1048575", "A1:A1048576"),
    ] {
        assert_eq!(
            mapped(edit, before, RangeRule::Reference).as_deref(),
            Some(after),
            "{before}"
        );
    }
    // Formats grow over the lines inserted right after them; references do not.
    assert_eq!(
        mapped(edit, "A1:A2", RangeRule::Format).as_deref(),
        Some("A1:A4")
    );
    assert_eq!(
        mapped(insert_rows(1, 1), "A1:A2", RangeRule::Format).as_deref(),
        Some("A2:A3")
    );
    assert_eq!(insert_rows(3, 2).map_line(1_048_575), None);
}

#[test]
fn deleted_rows_shrink_ranges_and_remove_the_ones_they_cover() {
    let edit = delete_rows(3, 4);
    for (before, after) in [
        ("A1:A2", Some("A1:A2")),
        ("A1:A3", Some("A1:A2")),
        ("A1:A10", Some("A1:A8")),
        ("A3:A10", Some("A3:A8")),
        ("A4:A6", Some("A3:A4")),
        ("A3:A4", None),
        ("A3", None),
        ("A5", Some("A3")),
        ("A1:A1048576", Some("A1:A1048576")),
    ] {
        assert_eq!(
            mapped(edit, before, RangeRule::Reference).as_deref(),
            after,
            "{before}"
        );
    }
}

#[test]
fn moved_rows_carry_each_range_end_and_shrink_crossed_ranges() {
    let edit = move_rows(3, 3, 8);
    for (before, after) in [
        ("A1:A3", "A1:A8"),
        ("A3:A4", "A3"),
        ("A3", "A8"),
        ("A4", "A3"),
        ("A8", "A7"),
        ("A9", "A9"),
        ("A1:A10", "A1:A10"),
        ("A1:A5", "A1:A4"),
    ] {
        assert_eq!(
            mapped(edit, before, RangeRule::Reference).as_deref(),
            Some(after),
            "{before}"
        );
    }
    let upward = move_rows(7, 8, 2);
    assert_eq!(upward.map_line(7), Some(2));
    assert_eq!(upward.map_line(8), Some(3));
    assert_eq!(upward.map_line(2), Some(4));
    assert_eq!(upward.map_line(6), Some(8));
    assert_eq!(upward.map_line(9), Some(9));
    for line in 1..=12 {
        let moved = upward.map_line(line).expect("a move keeps every line");
        assert_eq!(upward.unmap_line(moved), Some(line), "{line}");
        assert_eq!(upward.inverse().map_line(moved), Some(line), "{line}");
    }
}

#[test]
fn a_range_move_carries_cells_and_ranges_wholly_inside_it() {
    let edit = GridEdit::MoveRange {
        source: range("A2:A5"),
        destination: CellAddress::from_a1("M2").expect("M2"),
    };
    let address = |text: &str| CellAddress::from_a1(text).expect("address");
    assert_eq!(edit.map_address(address("A5")), Some(address("M5")));
    assert_eq!(edit.map_address(address("M6")), Some(address("M6")));
    assert_eq!(edit.map_address(address("M3")), None);
    assert_eq!(edit.unmap_address(address("M3")), Some(address("A3")));
    assert_eq!(edit.unmap_address(address("A3")), None);
    assert_eq!(
        mapped(edit, "A2:A5", RangeRule::Reference).as_deref(),
        Some("M2:M5")
    );
    assert_eq!(
        mapped(edit, "A1:A5", RangeRule::Reference).as_deref(),
        Some("A1:A5")
    );
    // A conditional format splits into the part that stays and the part that moves.
    let ranges = edit
        .map_ranges(&[range("A1:B6")], RangeRule::Format)
        .into_iter()
        .map(spelled)
        .collect::<Vec<_>>();
    assert_eq!(ranges, ["A1:B1", "B2:B5", "A6:B6", "M2:M5"]);
    assert!(edit.overwrites(range("M1:M3")));
    assert!(!edit.overwrites(range("A1:A3")));
    assert_eq!(
        edit.inverse().map_address(address("M2")),
        Some(address("A2"))
    );
}

#[test]
fn split_ranges_are_those_an_edit_changes_in_part() {
    let array = range("B3:B5");
    assert!(insert_rows(4, 1).splits(array));
    assert!(!insert_rows(3, 1).splits(array));
    assert!(!insert_rows(6, 1).splits(array));
    assert!(delete_rows(5, 6).splits(array));
    assert!(!delete_rows(3, 5).splits(array));
    assert!(move_rows(4, 4, 9).splits(array));
    assert!(!move_rows(3, 5, 9).splits(array));
    assert!(move_rows(9, 9, 4).splits(array));
    assert!(!move_rows(9, 9, 3).splits(array));
    assert!(move_rows(1, 1, 4).splits(array));
}

#[test]
fn history_maps_current_cells_back_through_every_edit() {
    let mut history = GridHistory::default();
    history.push(insert_rows(2, 1));
    history.push(delete_rows(5, 5));
    let address = |text: &str| CellAddress::from_a1(text).expect("address");
    assert_eq!(history.to_source(address("A1")), Some(address("A1")));
    assert_eq!(history.to_source(address("A4")), Some(address("A3")));
    assert_eq!(history.to_source(address("A2")), None);
    assert_eq!(history.to_source(address("A5")), Some(address("A5")));
}

#[test]
fn rectangle_subtraction_covers_the_cells_outside_the_removed_part() {
    let pieces = subtract(range("A1:C3"), range("B2"))
        .into_iter()
        .map(spelled)
        .collect::<Vec<_>>();
    assert_eq!(pieces, ["A1:C1", "A2", "C2", "A3:C3"]);
    assert_eq!(subtract(range("A1:A3"), range("A1:A3")), Vec::new());
    assert_eq!(intersect(range("A1:B2"), range("C3")), None);
}
