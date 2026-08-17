use cellrune::{
    CalculationOptions, CellAddress, CellValue, EditBatch, FiniteNumber, SheetId,
    WorkbookCalculationSession, WorkbookChange, WorkbookFingerprint, calculate_workbook,
};

fn set_a1(session: &mut WorkbookCalculationSession, value: f64) {
    let revision = session.workbook().semantic_revision();
    session
        .apply_changes(
            revision,
            EditBatch::new([WorkbookChange::set_cell_value(
                SheetId::new(1).expect("constant sheet ID"),
                CellAddress::from_indices(1, 1).expect("constant cell address"),
                CellValue::Number(FiniteNumber::new(value).expect("finite test value")),
            )]),
        )
        .expect("test edit should install");
}

#[test]
fn public_fingerprint_is_versioned_stable_and_history_independent() {
    let unchanged = WorkbookCalculationSession::create();
    let mut direct = WorkbookCalculationSession::create();
    let mut rewritten = WorkbookCalculationSession::create();

    set_a1(&mut direct, 2.0);
    set_a1(&mut rewritten, 1.0);
    set_a1(&mut rewritten, 2.0);

    let direct_fingerprint = direct.workbook().fingerprint();
    let rewritten_fingerprint = rewritten.workbook().fingerprint();

    assert_ne!(
        direct.workbook().semantic_revision(),
        rewritten.workbook().semantic_revision()
    );
    assert_eq!(direct_fingerprint, rewritten_fingerprint);
    assert_ne!(direct_fingerprint, unchanged.workbook().fingerprint());
    assert_eq!(
        direct_fingerprint.schema_version(),
        WorkbookFingerprint::CURRENT_SCHEMA_VERSION
    );
    assert_eq!(direct_fingerprint.as_bytes().len(), 32);
    assert_eq!(direct_fingerprint.to_hex().len(), 64);
    assert_eq!(direct_fingerprint.to_string(), direct_fingerprint.to_hex());
    assert_eq!(direct_fingerprint.schema_version(), 7);
    assert_eq!(
        direct_fingerprint.to_hex(),
        "a116541edc0fd3068602fdefb5fba8559c56f187fc66c4d5dfce2434cfe68ee4"
    );

    let calculation = calculate_workbook(direct.workbook(), CalculationOptions::default());
    assert_eq!(calculation.source_fingerprint(), direct_fingerprint);
}
