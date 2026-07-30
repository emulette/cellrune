# Excel conformance workbooks

This directory contains Excel-saved workbooks and the expected CellRune result for each selected
formula. The audit runs as part of the standard `cargo test` suite. The command below runs the same
audit by itself:

```bash
cargo run \
  --package cellrune-integration-tests \
  --bin check_excel_oracle \
  --locked
```

The checker:

- verifies suite, manifest, workbook, metadata, observation, feature-set, profile, count, and
  SHA-256 provenance;
- exact-matches every formula anchor and observed array-result cell against the raw XLSX cache;
- opens each workbook and selects the active manifest formula cells;
- compares Excel's saved value with `expectations.json`;
- calculates the same workbook with CellRune;
- verifies the recorded classification.

## Layout

```text
conformance/
  apache-poi/
    formula-eval-test-data/
      workbook.xlsx
      metadata.json
      expectations.json
    matrix-formula-eval-test-data/
      workbook.xlsx
      metadata.json
      expectations.json
  cellrune/
    suite.json
    case-manifest.json
    online/
      formula-oracle.xlsx
      metadata.json
      observations.json
      expectations.json
    desktop-2021/
      formula-oracle.xlsx
      metadata.json
      observations.json
      expectations.json
```

`suite.json` requires the two audited host profiles and binds them to one case manifest.
`case-manifest.json` gives formulas stable names so expectations do not depend on row numbers.

## Classifications

- `match`: CellRune matches Excel.
- `divergent`: CellRune intentionally differs and the current difference is recorded.
- `not_implemented`: CellRune reports an unsupported feature.
- `host_unsupported`: that Excel workbook has no usable value for the formula.
- `excluded` or `unreadable`: the case cannot be compared.

`host_unsupported` is a valid final observation even when every recorded Excel workbook lacks a
value. It does not require regenerating the workbook or removing the formula.

Finite numbers use the case's comparator; other values compare exactly. Non-match entries carry a
short note explaining the current state.

## Updating

1. Generate the workbook.
2. Recalculate and save it in Excel.
3. Run `verify_excel_oracle.mjs saved` with the profile's declared output directory
   (`online/` or `desktop-2021/`). It stages `suite.json` and `case-manifest.json` in the parent.
4. Run `check_excel_oracle --report` against that staged profile directory to generate
   expectations. Suite-bound metadata is rejected when the parent suite contract is absent.
5. Copy the complete staged suite tree into this directory.
6. Run the audit command above.

No separate CI step or release-only oracle gate is required.
