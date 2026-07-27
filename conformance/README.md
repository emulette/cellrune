# Excel-saved conformance workbooks

This tree keeps the binary workbooks that Excel actually calculated. They are inputs to an
explicit local audit, not the default Rust test suite, CI, or a publication gate.

```bash
cargo run \
  --package cellrune-integration-tests \
  --bin check_excel_oracle \
  --locked
```

The checker validates each workbook hash and metadata, including the effective XLSX iteration
setting, derives the complete selected case set, compares the saved cache with
`expectations.json`, calculates the workbook with CellRune, and enforces every reviewed
classification in both directions. Case selection is capped at 100,000 results, and declared
array ranges above that limit are rejected before expansion.

## Layout and provenance

```text
conformance/
  apache-poi/
    LICENSE-APACHE-2.0
    NOTICE
    formula-eval-test-data/
      workbook.xlsx
      metadata.json
      expectations.json
    matrix-formula-eval-test-data/
      workbook.xlsx
      metadata.json
      expectations.json
  cellrune/
    formula-oracle.xlsx
    metadata.json
    expectations.json
```

`apache-poi/` contains unmodified Apache POI fixtures under Apache-2.0. Each metadata file records
the exact upstream revision and SHA-256. `cellrune/` contains a workbook authored by the CellRune
project under the repository license and recalculated in Excel. The directory name identifies
workbook authorship; it does not claim that CellRune produced the expected values.

The two sources must remain separate. Their Excel versions, dates, locales, and licensing differ.

## Files

`metadata.json` uses `cellrune_excel_oracle_metadata_v1` and records:

- workbook filename and SHA-256;
- formula-cell count and selected primary-case rule;
- source name, license, URL, and revision;
- generator revision when CellRune authored the workbook;
- Excel application, version, channel, OS, locale, saved time, date system, and iteration state.

Unknown historical host fields remain `null`; they must not be guessed. Populate them when a
workbook is regenerated on a known host.

`expectations.json` is keyed as `Sheet!A1`. Every selected case is explicit; missing and extra
keys fail the audit. Classifications are:

- `match` — CellRune must reproduce the saved Excel value;
- `divergent` — CellRune must remain different and reproduce the recorded CellRune value;
- `not_implemented` — CellRune must return a structured unavailable result;
- `host_unsupported` — the saving Excel host could not evaluate a valid newer or host-specific
  function, so its cache is an observation rather than a semantic oracle;
- `excluded` — the workbook has no comparable saved value, with a required explanation;
- `unreadable` — reserved for a reviewed source limitation;
- `unclassified` — forbidden in committed data.

Every non-match classification requires a note. Finite numbers default to a scale-relative
`1e-8` comparison; other values compare exactly. A case can explicitly request `exact`,
`exact_bits`, or an absolute-plus-relative tolerance. Cancellation and signed-zero probes use
`exact_bits` so a unit-scaled tolerance cannot hide a near-zero mismatch. Report regeneration
uses and preserves each case's existing comparator.

Excel rich errors can store `#VALUE!` in `<v>` while `vm` points to the actual modern error such
as `#SPILL!`. Such cases set `excel_rich_error: true`; the checker treats the typed expectation as
authoritative and requires an error fallback in the workbook.

## Updating the CellRune workbook

The tracked generator and pre-Excel validation tools live in the private planning repository
because regeneration includes a manual Excel step. The public, reproducible boundary is:

1. generate the pre-Excel workbook and verify its formula inventory;
2. open it in the recorded Excel host, force recalculation, and save;
3. verify that Excel did not remove or downgrade formulas;
4. copy the saved workbook, metadata, and reviewed expectations into `conformance/cellrune/`;
5. update the workbook SHA-256, host fields, formula count, and every changed classification;
6. run the explicit checker above.

Do not overwrite an older host baseline when the host lacked functions needed by the comparison.
Add a separately identified baseline or update the metadata and expectations through review.
