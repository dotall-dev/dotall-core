use dotall_xlsx::edits::transform::{
    Axis, AxisChange, TransformResult, parse_cell, parse_range, transform_cell, transform_formula,
    transform_range, transform_sqref,
};

fn rows_inserted(at: u32, count: u32) -> AxisChange {
    AxisChange::Insert {
        axis: Axis::Row,
        at,
        count,
    }
}

fn rows_deleted(at: u32, count: u32) -> AxisChange {
    AxisChange::Delete {
        axis: Axis::Row,
        at,
        count,
    }
}

#[test]
fn transforms_cells_before_within_and_after_structural_changes() {
    let cases = [
        ("A4", rows_inserted(5, 2), TransformResult::Kept("A4")),
        ("$B$5", rows_inserted(5, 2), TransformResult::Kept("$B$7")),
        ("C8", rows_deleted(5, 2), TransformResult::Kept("C6")),
        ("D5", rows_deleted(5, 2), TransformResult::Removed),
        ("E4", rows_deleted(5, 2), TransformResult::Kept("E4")),
    ];

    for (address, change, expected) in cases {
        let actual =
            transform_cell(&parse_cell(address).unwrap(), change).map(|cell| cell.to_string());
        assert_eq!(
            actual,
            expected.map(str::to_owned),
            "{address} with {change:?}"
        );
    }
}

#[test]
fn transforms_ranges_across_insertions_and_deletions() {
    let cases = [
        ("A1:B4", rows_inserted(5, 2), TransformResult::Kept("A1:B4")),
        (
            "$A$5:B8",
            rows_inserted(5, 2),
            TransformResult::Kept("$A$7:B10"),
        ),
        ("A1:B10", rows_deleted(5, 2), TransformResult::Kept("A1:B8")),
        ("A5:B10", rows_deleted(5, 2), TransformResult::Kept("A5:B8")),
        ("A1:B6", rows_deleted(5, 2), TransformResult::Kept("A1:B4")),
        ("A5:B6", rows_deleted(5, 2), TransformResult::Removed),
    ];

    for (address, change, expected) in cases {
        let actual =
            transform_range(&parse_range(address).unwrap(), change).map(|range| range.to_string());
        assert_eq!(
            actual,
            expected.map(str::to_owned),
            "{address} with {change:?}"
        );
    }
}

#[test]
fn reports_excel_limit_overflows_as_ref_errors() {
    let cell = parse_cell("XFD1048576").unwrap();
    assert_eq!(
        transform_cell(&cell, rows_inserted(1, 1)),
        TransformResult::RefError
    );

    let range = parse_range("XFC1:XFD2").unwrap();
    let columns = AxisChange::Insert {
        axis: Axis::Column,
        at: 16_383,
        count: 2,
    };
    assert_eq!(transform_range(&range, columns), TransformResult::RefError);
}

#[test]
fn rewrites_only_matching_formula_references() {
    assert_eq!(
        transform_formula(
            r#"=SUM(A4,$B$5,'Edited Sheet'!C5:C6,Other!D5,"A5 ""Edited!B5""",TaxRate)"#,
            "Edited Sheet",
            "Edited Sheet",
            rows_inserted(5, 2),
        ),
        r#"=SUM(A4,$B$7,'Edited Sheet'!C7:C8,Other!D5,"A5 ""Edited!B5""",TaxRate)"#
    );
    assert_eq!(
        transform_formula("=A5+Edited!B5", "Elsewhere", "Edited", rows_deleted(5, 1)),
        "=A5+#REF!"
    );
}

#[test]
fn transforms_whitespace_separated_sqref_lists_and_drops_removed_ranges() {
    assert_eq!(
        transform_sqref("A1:B4  C5:C6\t$D$8", rows_inserted(5, 2)).unwrap(),
        TransformResult::Kept("A1:B4 C7:C8 $D$10".into())
    );
    assert_eq!(
        transform_sqref("A5:A6 B1:B2", rows_deleted(5, 2)).unwrap(),
        TransformResult::Kept("B1:B2".into())
    );
    assert!(transform_sqref("A1 invalid", rows_inserted(1, 1)).is_err());
}
