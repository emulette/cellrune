use super::{CopyOffset, GridChange};
use crate::CellAddress;
use crate::calculation::formula_rewrite::{
    FormulaRewriteBudget, FormulaRewriteError, FormulaRewriteLimits, FormulaRewriteRequest,
    SheetChanges, rewrite_formula,
};
use crate::calculation::sheet_changes::{TabEntry, TabStep};
use crate::grid::{GridAxis, GridEdit};

fn rewrite(
    source: &str,
    request: &FormulaRewriteRequest<'_>,
) -> Result<String, FormulaRewriteError> {
    let mut budget = FormulaRewriteBudget::new(FormulaRewriteLimits::UNBOUNDED, &|| false);
    rewrite_formula(source, request, &mut budget)
        .map(|rewritten| rewritten.unwrap_or_else(|| source.to_owned()))
}

fn grid(edit: GridEdit, host_is_sheet: bool) -> FormulaRewriteRequest<'static> {
    FormulaRewriteRequest::Grid(GridChange {
        sheet: "S1",
        host_is_sheet,
        edit,
    })
}

fn check(request: &FormulaRewriteRequest<'_>, cases: &[(&str, &str)]) {
    for (source, expected) in cases {
        assert_eq!(
            rewrite(source, request).expect(source),
            *expected,
            "{source}"
        );
    }
}

const INSERT_ROWS: GridEdit = GridEdit::Insert {
    axis: GridAxis::Rows,
    at: 3,
    count: 2,
};
const DELETE_ROWS: GridEdit = GridEdit::Delete {
    axis: GridAxis::Rows,
    first: 3,
    last: 4,
};

#[test]
fn inserted_rows_move_absolute_and_relative_references_by_the_range_rule() {
    check(
        &grid(INSERT_ROWS, true),
        &[
            ("A1+A3", "A1+A5"),
            ("SUM(A1:A3)", "SUM(A1:A5)"),
            ("SUM(A3:A5)", "SUM(A5:A7)"),
            ("SUM(A1:A2)", "SUM(A1:A2)"),
            ("$A$3+A$4+$B5", "$A$5+A$6+$B7"),
            ("SUM(3:3)+SUM($5:$5)", "SUM(5:5)+SUM($7:$7)"),
            ("SUM(A:A)+SUM(1:2)", "SUM(A:A)+SUM(1:2)"),
            ("SUM('S1:S3'!A3)", "SUM('S1:S3'!A3)"),
            ("INDIRECT(\"A3\")", "INDIRECT(\"A3\")"),
            ("[Book.xlsx]S1!A3", "[Book.xlsx]S1!A3"),
            ("S2!A3+S1!A3+'s1'!A3", "S2!A3+S1!A5+'s1'!A5"),
            ("A3#+OFFSET(A3,1,1)", "A5#+OFFSET(A5,1,1)"),
            ("SUM('S1'!A1:'S1'!A3)", "SUM('S1'!A1:'S1'!A5)"),
            ("SUM(Table1[col])+Rate", "SUM(Table1[col])+Rate"),
            ("SUM( a3 , A1 )", "SUM( A5 , A1 )"),
        ],
    );
    // Another sheet's unqualified references name that sheet.
    check(
        &grid(INSERT_ROWS, false),
        &[("A3+S1!A3", "A3+S1!A5"), ("S1!$A$4:$B$9", "S1!$A$6:$B$11")],
    );
}

#[test]
fn deleted_rows_turn_references_into_ref_errors_and_shrink_ranges() {
    check(
        &grid(DELETE_ROWS, true),
        &[
            ("A3", "#REF!"),
            ("SUM(A3:A4)", "SUM(#REF!)"),
            ("SUM(A1:A5)", "SUM(A1:A3)"),
            ("SUM(A4:A6)", "SUM(A3:A4)"),
            ("A3#", "#REF!"),
            ("SUM(3:4)+SUM(2:5)", "SUM(#REF!)+SUM(2:3)"),
            ("A6+1", "A4+1"),
            ("SUM('S1'!A3:'S1'!A4)", "SUM('S1'!#REF!)"),
        ],
    );
    check(
        &grid(DELETE_ROWS, false),
        &[
            ("S1!A3", "S1!#REF!"),
            ("'S1'!A4*2", "'S1'!#REF!*2"),
            ("SUM('S1'!A3:A4)", "SUM('S1'!#REF!)"),
        ],
    );
}

#[test]
fn column_edits_change_column_parts_and_leave_rows_alone() {
    check(
        &grid(
            GridEdit::Insert {
                axis: GridAxis::Columns,
                at: 2,
                count: 1,
            },
            true,
        ),
        &[
            ("A1+B1+C1", "A1+C1+D1"),
            ("SUM(A:B)", "SUM(A:C)"),
            ("SUM(1:1)", "SUM(1:1)"),
        ],
    );
    check(
        &grid(
            GridEdit::Delete {
                axis: GridAxis::Columns,
                first: 2,
                last: 2,
            },
            true,
        ),
        &[
            ("SUM(A1:C1)", "SUM(A1:B1)"),
            ("B1", "#REF!"),
            ("SUM(B:B)", "SUM(#REF!)"),
        ],
    );
}

#[test]
fn moved_rows_carry_references_and_each_range_end() {
    check(
        &grid(
            GridEdit::Move {
                axis: GridAxis::Rows,
                first: 3,
                last: 3,
                destination: 8,
            },
            true,
        ),
        &[
            ("SUM(A1:A3)", "SUM(A1:A8)"),
            ("SUM(A3:A4)", "SUM(A3:A3)"),
            ("A3+A4", "A8+A3"),
        ],
    );
}

#[test]
fn a_range_move_carries_references_wholly_inside_it() {
    let request = grid(
        GridEdit::MoveRange {
            source: crate::CellRange::new(
                CellAddress::from_a1("A2").expect("A2"),
                CellAddress::from_a1("A5").expect("A5"),
            )
            .expect("range"),
            destination: CellAddress::from_a1("M2").expect("M2"),
        },
        true,
    );
    check(
        &request,
        &[
            ("A5", "M5"),
            ("SUM(A2:A5)", "SUM(M2:M5)"),
            ("SUM(A1:A5)", "SUM(A1:A5)"),
            ("SUM(M:M)", "SUM(M:M)"),
        ],
    );
    assert!(matches!(
        rewrite("M3+1", &request),
        Err(FormulaRewriteError::OverwrittenReference { .. })
    ));
}

#[test]
fn copies_shift_only_relative_parts_and_refuse_to_leave_the_grid() {
    let request = FormulaRewriteRequest::Copy(CopyOffset {
        rows: 4,
        columns: 1,
    });
    check(
        &request,
        &[
            ("SUM(A2:A5)", "SUM(B6:B9)"),
            ("$A$1+A$1+$A1", "$A$1+B$1+$A5"),
            ("SUM(T[c])", "SUM(T[c])"),
            ("S2!A1+SUM(A:A)+SUM(1:2)", "S2!B5+SUM(B:B)+SUM(5:6)"),
            ("\"A1\"&Rate", "\"A1\"&Rate"),
        ],
    );
    let upward = FormulaRewriteRequest::Copy(CopyOffset {
        rows: -1,
        columns: 0,
    });
    check(&upward, &[("A2+$A$1", "A1+$A$1"), ("A1+B2", "#REF!")]);
}

#[test]
fn qualified_ref_errors_follow_sheet_renames_and_deletions() {
    check(
        &FormulaRewriteRequest::Sheet {
            old_name: "S1",
            new_name: "Data Sheet",
        },
        &[("S1!#REF!+1", "'Data Sheet'!#REF!+1")],
    );
    let id = |value| crate::SheetId::new(value).expect("sheet ID");
    let tabs = [id(1), id(2)]
        .into_iter()
        .map(|id| TabEntry {
            id,
            worksheet: true,
        })
        .collect::<std::sync::Arc<[_]>>();
    let steps = [TabStep::Delete {
        tabs,
        deleted: id(1),
    }];
    check(
        &FormulaRewriteRequest::Sheets(SheetChanges {
            before: &[("S1", id(1)), ("S2", id(2))],
            after: &[(id(2), "S2")],
            steps: &steps,
            retarget: None,
            deleted_tables: &[],
        }),
        &[("'S1'!#REF!+S2!#REF!", "#REF!+S2!#REF!")],
    );
}

#[test]
fn unreadable_formulas_fail_closed_only_when_they_may_name_the_edited_cells() {
    assert!(matches!(
        rewrite("A3+", &grid(INSERT_ROWS, true)),
        Err(FormulaRewriteError::Parse { .. })
    ));
    assert!(matches!(
        rewrite("S1!A3+", &grid(INSERT_ROWS, false)),
        Err(FormulaRewriteError::Parse { .. })
    ));
    assert_eq!(
        rewrite("A3+", &grid(INSERT_ROWS, false)).expect("another sheet"),
        "A3+"
    );
}
