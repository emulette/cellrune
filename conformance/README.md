# Conformance expectation matrices

Each JSON file in this tree is a **non-binary description of one supported workbook shape** —
every literal, every formula, and the value a recorded oracle saved for each formula. The single
data-driven test in `release-tests/integration/tests/conformance_matrix.rs` reconstructs every
matrix, calculates it, and compares CellRune's values with the recorded oracle during the normal
`cargo test` run. Adding functions or syntax adds cases to this suite, not new CI jobs.

Schema v1 deliberately supports only ordinary formulas and resolved shared formulas. The
extractor rejects array, dynamic-array, data-table, recalculate-always, and sheet-scoped-name
metadata instead of silently reconstructing a workbook with different calculation semantics.

## Layout

Matrices are grouped by the oracle that recorded them, named `<product>-<version>`:

- `excel-2013-15.0300/formula-eval-test-data.json` — the Apache POI `FormulaEvalTestData`
  corpus (Apache-2.0) in XLSX form: 4 sheets, 2,882 literals, 1,295 formula cases whose
  expectations are the workbook's saved calculation cache, last written by Microsoft Excel
  2013 (`AppVersion 15.0300`, saved 2016-02-15). The Apache license and modification notice are
  distributed as `LICENSE-APACHE-2.0` and `NOTICE` in this directory.

## Schema (`cellrune_conformance_matrix_v1`)

Defined in `release-tests/integration/src/conformance.rs`, shared by the extractor and the
test. The load-bearing fields:

- **`oracle`** — the exact build the expectations came from. "Matches Excel" is not a checkable
  claim; "matches Microsoft Excel 2013 (AppVersion 15.0300)" is.
- **`tolerance.mode`** — `exact`, or `scaled` with a finite epsilon in `0..=1`:
  `|actual - expected| <= epsilon * max(|actual|, |expected|, 1)`, the same rule the
  external-corpus audit applies.
- **`cellrune_status`** — `match`, `divergent`, `intentionally_more_accurate`, or
  `not_implemented`. The test enforces statuses in both directions: a `match` case must match,
  and a divergent case must still diverge, must still produce the recorded `cellrune_value`,
  and must carry a `note` explaining why the divergence is expected. A documented divergence
  can therefore not silently become a pass, and neither side of it can drift unnoticed.

## Regenerating a matrix

```
cargo run -p cellrune-integration-tests --bin extract_conformance_matrix -- \
  <workbook.xlsx> <metadata.json> <output.json>
```

`metadata.json` supplies the `oracle` and `source` blocks the workbook cannot describe about
itself. The extractor classifies every case by running CellRune over the supported workbook
shape; divergences come out as `divergent` with CellRune's actual value recorded and **no note**.
Writing the note is the reviewed, human step, and the normal conformance test refuses matrices
that skip it.
