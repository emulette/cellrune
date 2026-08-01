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
- compares each declared array result's materialized range, shape, and cells;
- verifies the recorded classification and the exact CellRune-side result for reviewed
  multi-cell array divergences.

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
short note explaining the current state. A divergent array whose shape or non-anchor cells differ
also records `cellrune_result`; its range, shape, and every typed cell must remain exact. A sole
one-cell anchor difference remains owned by the scalar `cellrune_value`/`cellrune_type` signature.

## Maintaining the CellRune fixtures

The committed Online and desktop-2021 XLSX files, case manifest, observations, and suite identity
are stable reference fixtures. Feature work does not add formulas or cases, resave either workbook
in Excel, add host profiles, or create feature-specific conformance workbooks.

When CellRune implements an existing case:

1. Run `check_excel_oracle --report` for both profiles.
2. Review that every changed classification belongs to the implementation.
3. Update only the two `expectations.json` files.
4. Run the complete audit and standard test suite.

A correction to the fixture reader or schema may regenerate derived JSON for both profiles from
the same committed XLSX bytes. It must not change formulas, cached Excel observations, the case
manifest, or only one profile. Any intentional dataset revision is separate conformance
maintenance, not part of a feature implementation.

Coverage not represented by these reference workbooks belongs in an ordinary unit or integration
test, using a small generated-XLSX fixture outside `conformance/cellrune/`. Generated fixtures
verify CellRune behavior; they do not represent results saved by Excel.

## Optional local corpora

Two corpus tests are registered with `#[ignore]` because their third-party inputs are not
distributed in this repository. Developers who have supplied those inputs can run them
explicitly:

```bash
WORKBOOK_FORMULA_CORPUS=/path/to/formulas.xlsx \
  cargo test -p cellrune-integration-tests --test external_formula_corpus -- --ignored

CELLRUNE_WORKBOOK_CORPUS=/path/to/workbook-or-directory \
  cargo test -p cellrune-integration-tests --test external_workbook_corpus -- --ignored
```
