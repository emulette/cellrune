# Observed workbook engine comparison

This report records an observational comparison measured on 2026-07-26. It
describes what eleven workbook calculation engines returned for two large
workbooks and four upstream test corpora under one typed comparison protocol.

The report is not a composite score, quality ranking, compatibility guarantee,
CI threshold, or release gate. Workbook acceptance, calculation coverage,
agreement with saved values, failure handling, runtime, and memory are separate
observations.

## Targets

| engine | version | runtime |
|---|---:|---|
| CellRune | 0.1.1 | Rust, macOS arm64 |
| IronCalc | 0.7.1 | Rust, macOS arm64 |
| Formualizer | 0.7.1 | Rust, macOS arm64 |
| xlstream | 0.4.0 | Rust, macOS arm64 |
| LogiSheets | 1.8.0 | Rust, macOS arm64 |
| recalc-engine | 0.1.0 | Rust, macOS arm64 |
| HyperFormula | 3.3.0 | Node 26.5.0 |
| xlsx-calc | 0.9.2 | Node 26.5.0 |
| formulas | 1.3.4 | Python 3.14.4 |
| pycel | 1.0b30 | Python 3.14.4 |
| xlcalculator | 0.5.0 | Python 3.14.4 |

Only libraries that load a workbook and attempt dependency-aware formula
calculation were included. Read/write-only libraries and single-formula
evaluators were outside the comparison.

## Inputs and sources

### Large workbooks

| input | source and saved-value producer | formulas | SHA-256 |
|---|---|---:|---|
| EarlyRetirementNow SWR Toolbox v2.0 XLSX export | [official public workbook article](https://earlyretirementnow.com/2018/08/29/google-sheet-updates-swr-series-part-28/); Google Sheets saved values | 251,164 | `2f69abd87af7d534cd6e3db727a962116614fba705467c09cc205139df3bdcef` |
| `Enron_Nymex_cal_Spreads.xlsx` local copy | Enron spreadsheet corpus-derived; Microsoft Excel AppVersion `15.0300` saved values | 34,583 | `68d62ed6713b61f8534b9d18a55cd1159b051bbbd39ffba08f58296a51da729c` |

The exact acquisition URL for the NYMEX copy was not retained, so this report
does not claim one. The hash and package metadata identify the measured file.

### Upstream engine corpora

| corpus | upstream directory | files | formulas |
|---|---|---:|---:|
| formulas | [`test/test_files`](https://github.com/vinci1it2000/formulas/tree/master/test/test_files) | 5 | 15,042 |
| IronCalc | [`xlsx/tests/calc_tests`](https://github.com/ironcalc/IronCalc/tree/main/xlsx/tests/calc_tests) | 201 | 67,806 |
| pycel | [`tests/fixtures`](https://github.com/dgorissen/pycel/tree/master/tests/fixtures) | 13 | 5,080 |
| xlcalculator | [`tests/resources`](https://github.com/bradbase/xlcalculator/tree/master/tests/resources) | 64 | 3,400 |

The source repositories and directories are confirmed, and every local
workbook is identified by a retained hash. The exact upstream commits used to
assemble the historical copies were not retained and are therefore not
asserted.

No third-party workbook binary, formula text, sheet name, or per-cell value is
distributed in this repository.

## Method

The comparison used a central, engine-independent OOXML inventory:

1. Every formula cell was inventoried in workbook order, including normal,
   shared, and legacy array formulas.
2. Saved values were decoded as typed numbers, text, logical values, errors, or
   blanks.
3. Every engine received the same workbook and immutable inventory and emitted
   one ordered outcome for each formula: a typed `value`, `unavailable`, or
   `exception`.
4. Engine adapters could neither select the denominator nor decide whether a
   result matched.
5. The central comparison required identical types. Finite numbers used
   relative tolerance `1e-8` and absolute tolerance `1e-10`; other values
   required exact equality.
6. A rejected file, process failure, timeout, missing result, or unsupported
   formula remained in the canonical formula denominator.
7. Formula caches were cleared where an adapter could otherwise read a stale
   saved value instead of a recalculated result.

`values / all formulas` describes calculation coverage. `matched / compared`
describes agreement only for returned typed values with a saved reference.
High agreement with low coverage and complete coverage with low agreement are
different results.

## Large-workbook observations

### SWR Toolbox

The saved reference is a Google Sheets export, not an Excel oracle.

| engine | outcome | values / 251,164 | matched / compared | wall | peak RSS |
|---|---|---:|---:|---:|---:|
| CellRune | completed | 251,164 | 246,998 / 251,164 (98.34%) | 9.8 s | 1,089.7 MiB |
| IronCalc | load failure | 0 | — | 0.5 s | 38.5 MiB |
| Formualizer | completed | 251,161 | 180,845 / 251,161 (72.00%) | 18.8 s | 584.3 MiB |
| xlstream | evaluation failure | 0 | — | 0.8 s | 56.7 MiB |
| LogiSheets | load failure | 0 | — | 0.5 s | 40.8 MiB |
| recalc-engine | completed | 251,164 | 180,273 / 251,164 (71.78%) | 3.2 s | 1,028.4 MiB |
| HyperFormula | completed | 251,164 | 177,491 / 251,164 (70.67%) | 7.9 s | 982.6 MiB |
| xlsx-calc | completed | 48,382 | 48,379 / 48,382 (99.99%) | 6.1 s | 853.5 MiB |
| formulas | parse failure | 0 | — | 130.9 s | 5,039.0 MiB |
| pycel | completed | 167,187 | 167,184 / 167,187 (100.00%) | 93.2 s | 4,572.3 MiB |
| xlcalculator | parse failure | 0 | — | 19.0 s | 1,479.7 MiB |

CellRune, recalc-engine, and HyperFormula returned a typed outcome for every
formula, with different saved-cache agreement. xlsx-calc and pycel had high
agreement among their returned values but lower calculation coverage. Five
engines stopped during load, parsing, or evaluation.

CellRune's 4,166 differing values are differences from the Google-produced
cache. They must not be quoted as failures to match Excel.

### Enron NYMEX

The saved reference was produced by Microsoft Excel AppVersion `15.0300`.

| engine | outcome | values / 34,583 | matched / compared | wall | peak RSS |
|---|---|---:|---:|---:|---:|
| CellRune | completed | 34,583 | 34,583 / 34,583 (100.00%) | 1.4 s | 282.1 MiB |
| IronCalc | completed | 34,583 | 34,583 / 34,583 (100.00%) | 22.4 s | 197.5 MiB |
| Formualizer | completed | 34,583 | 32,324 / 34,583 (93.47%) | 186.3 s | 194.6 MiB |
| xlstream | evaluation failure | 0 | — | 0.2 s | 15.1 MiB |
| LogiSheets | completed | 34,583 | 34,583 / 34,583 (100.00%) | 69.3 s | 1,170.5 MiB |
| recalc-engine | completed | 34,583 | 34,583 / 34,583 (100.00%) | 1.4 s | 289.7 MiB |
| HyperFormula | completed | 34,583 | 34,582 / 34,583 (100.00%) | 3.4 s | 554.5 MiB |
| xlsx-calc | completed | 14,493 | 7,351 / 14,493 (50.72%) | 4.5 s | 432.3 MiB |
| formulas | timeout | 0 | — | 540.1 s | not recorded |
| pycel | completed | 34,583 | 34,583 / 34,583 (100.00%) | 245.8 s | 1,206.7 MiB |
| xlcalculator | timeout | 0 | — | 541.9 s | not recorded |

CellRune, IronCalc, LogiSheets, recalc-engine, and pycel agreed with every saved
value. HyperFormula differed at one cell. The engines with identical agreement
still differed substantially in runtime and memory on this host.

## Upstream-corpus observations

The following table keeps CellRune's results next to the engine that owns each
corpus. It is not a head-to-head score: package releases can differ from the
repository revision that supplied their fixtures.

| corpus | formulas | CellRune values | CellRune matched / compared | corpus engine values | corpus engine matched / compared |
|---|---:|---:|---:|---:|---:|
| formulas | 15,042 | 7,178 | 5,905 / 7,178 | 13,996 | 13,781 / 13,996 |
| IronCalc | 67,806 | 21,785 | 20,449 / 21,785 | 67,796 | 65,860 / 67,796 |
| pycel | 5,080 | 4,739 | 3,528 / 4,739 | 5,045 | 5,045 / 5,045 |
| xlcalculator | 3,400 | 3,352 | 3,313 / 3,352 | 637 | 546 / 637 |

These corpora expose different behavior than the two large workbooks:

- CellRune returned explicit `unavailable` outcomes for 46,021 of the 67,806
  IronCalc-corpus formulas, identifying a breadth gap without removing those
  formulas from the denominator.
- CellRune returned values for 3,352 of 3,400 formulas in the xlcalculator
  corpus and agreed with 3,313 of those saved values.
- A corpus-owning engine was not necessarily perfect on the package version
  tested. Fixture evolution, unsupported cases, and cell-level exceptions
  remained visible.
- Across all engines, file refusal, unavailable results, cell exceptions,
  calculated errors, and differing typed values formed distinct failure
  profiles.

## Harness corrections and evidence integrity

The final audit found and corrected two adapter defects before selecting the
authoritative Away results:

- The xlcalculator adapter quoted worksheet names even though its model keys
  are unquoted, causing valid lookups to appear blank.
- The xlsx-calc adapter could serialize an invalid JavaScript date as a null
  numeric value and stop a file on an unsupported array result.

All 283 files were repeated for each affected engine. The original runs were
retained and marked as superseded rather than overwritten.

The authoritative result set contains 3,113 Away run records and 44
corpus-engine aggregates. A read-only audit also covered 22 large-workbook
runs, parsed 2,547,015 JSONL records, checked retained hashes and aggregate
partitions, and found no remaining result-integrity error.

Historical runs retained runner-source hashes but not full source snapshots.
The harness now snapshots each unique runner source once per new session in
addition to recording hashes.

## Interpretation limits

- Saved values are empirical references, not universal correctness oracles.
- Google Sheets agreement and Microsoft Excel agreement are not interchangeable.
- The NYMEX observation applies to the exact file hash and saved state listed
  above.
- Exact historical acquisition commits for the four upstream corpora were not
  retained.
- Results depend on the measured engine versions, adapters, host, runtime
  versions, and timeout policy.
- Wall time and peak RSS are single-host observations, not stable performance
  rankings.
- This report does not expand CellRune's stated XLSX or formula support scope.
  Use `scan_formula_capabilities` on each workbook before relying on calculated
  values.
