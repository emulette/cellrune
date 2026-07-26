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
8. Each process had a 540-second timeout.

`values / all formulas` describes calculation coverage. `matched / compared`
describes agreement only for returned typed values with a saved reference.
High agreement with low coverage and complete coverage with low agreement are
different results.

### Adapter configuration

Settings are part of the observation. The adapters used the ordinary
load-and-calculate path for each library:

| engine | measured configuration |
|---|---|
| CellRune | `ReadOptions::default()` and `CalculationOptions::default()` |
| IronCalc | XLSX loader with `en` language and locale, `UTC` timezone, then `evaluate()` |
| Formualizer | eager load, `WorkbookConfig::ephemeral()`, then `evaluate_all()` |
| xlstream | `EvaluateOptions::default()` |
| LogiSheets | standard `Workbook::from_file` load-and-calculate path |
| recalc-engine | `open`, `Engine::load`, then `recalc()` with no overrides |
| HyperFormula | SheetJS formula/literal input, workbook-derived `nullDate`, `smartRounding: false`; version 3.3.0 defaults otherwise, including `useArrayArithmetic: false` |
| xlsx-calc | `continue_after_error: true`, diagnostic logging disabled |
| formulas, pycel, xlcalculator | documented default workbook loaders and evaluators, with no calculation-option overrides |

HyperFormula does not load XLSX itself, so its result also measures the
SheetJS-to-HyperFormula adapter. The other engines' loader boundaries likewise
remain part of their observed results.

### Coverage and agreement at a glance

This compact view prevents a high percentage over a small returned subset from
being confused with complete calculation coverage:

| profile on the large workbooks | observed examples |
|---|---|
| complete coverage, high saved-value agreement | CellRune on both; IronCalc, LogiSheets, recalc-engine, and pycel on NYMEX |
| complete coverage, lower saved-value agreement | Formualizer, recalc-engine, and HyperFormula on SWR |
| partial coverage, high agreement among returned values | xlsx-calc and pycel on SWR |
| no returned values because loading, parsing, evaluation, or timeout stopped the run | five engines on SWR; three on NYMEX |

## Large-workbook observations

### SWR Toolbox

The saved reference is a Google Sheets export, not an Excel oracle.

| engine | outcome | values / 251,164 | matched / compared | wall | peak RSS |
|---|---|---:|---:|---:|---:|
| CellRune | completed | 251,164 | 246,998 / 251,164 (98.34%) | 10 s | 1,090 MiB |
| IronCalc | load failure | 0 | — | <1 s | 39 MiB |
| Formualizer | completed | 251,161 | 180,845 / 251,161 (72.00%) | 19 s | 584 MiB |
| xlstream | evaluation failure | 0 | — | <1 s | 57 MiB |
| LogiSheets | load failure | 0 | — | <1 s | 41 MiB |
| recalc-engine | completed | 251,164 | 180,273 / 251,164 (71.78%) | 3 s | 1,028 MiB |
| HyperFormula | completed | 251,164 | 177,491 / 251,164 (70.67%) | 8 s | 983 MiB |
| xlsx-calc | completed | 48,382 | 48,379 / 48,382 (99.99%) | 6 s | 854 MiB |
| formulas | parse failure | 0 | — | 131 s | 5,039 MiB |
| pycel | completed | 167,187 | 167,184 / 167,187 (100.00%) | 93 s | 4,572 MiB |
| xlcalculator | parse failure | 0 | — | 19 s | 1,480 MiB |

CellRune, recalc-engine, and HyperFormula returned a typed outcome for every
formula. xlsx-calc and pycel had high agreement among their returned values but
lower calculation coverage. Five engines stopped during load, parsing, or
evaluation.

#### What the 71% cluster contains

The similar aggregate percentages do not mean that Formualizer,
recalc-engine, and HyperFormula produced the same workbook:

- Formualizer and recalc-engine shared 70,280 differing-cache cells. Their
  mismatch-set Jaccard similarity was 99.09%, so those two results genuinely
  clustered by cell location.
- HyperFormula shared 44,645 differing-cache cells with Formualizer and 45,232
  with recalc-engine. All three differed from the cache at 44,621 cells.
- All three agreed with one another under the published typed comparator at
  151,821 of the 251,161 cells they could all compare (60.45%). Aggregate
  agreement near 71% therefore does not establish one shared calculation
  semantics.

The adapters also differed: Formualizer and recalc-engine loaded the workbook
directly, while HyperFormula received a SheetJS-translated sheet matrix and
used the explicit settings above.

#### CellRune's 4,166 cache differences

Post-processing the retained result streams, without rerunning an engine,
separated the differences as follows:

| observed difference | cells |
|---|---:|
| numeric date-serial offsets of +28, +29, +30, or +31 days | 3,514 |
| other numeric differences concentrated in the CAPE-based calculation area | 604 |
| ±0.001 differences in a percentile-derived result area | 26 |
| saved/result type differences | 19 |
| differing text values | 3 |

For all 3,514 month-length offsets, CellRune, Formualizer, and recalc-engine
returned exactly the same formula-derived number while HyperFormula returned
the saved cache number. Inspection of the retained XLSX showed a shared
`EOMONTH` chain whose formula and saved cache were one month apart. A positive
`months` argument means a future month in both the
[Microsoft](https://support.microsoft.com/en-us/office/eomonth-function-7314ffa1-2bc9-4005-9d66-f49db127d628)
and [Google Sheets](https://support.google.com/docs/answer/3093044?hl=en-GB)
definitions. This identifies an export formula/cache inconsistency or adapter
semantic difference, not a CellRune-only failure.

The remaining 652 differences are observable groups, not established causes:
the retained comparison alone does not prove whether each is a
Google-versus-Excel semantic difference, a stale saved value, an adapter
effect, or an engine defect. None of the 4,166 should be quoted as an Excel
conformance failure.

#### Tolerance sensitivity

The published tolerance was applied centrally and equally. Re-comparing the
same retained numeric outputs, without recalculation, gives:

| engine | `rel=1e-8`, `abs=1e-10` | `rel=1e-10`, `abs=1e-12` | exact |
|---|---:|---:|---:|
| CellRune | 246,998 (98.34%) | 216,416 (86.17%) | 141,632 (56.39%) |
| Formualizer | 180,845 (72.00%) | 167,920 (66.86%) | 132,371 (52.70%) |
| recalc-engine | 180,273 (71.78%) | 167,339 (66.63%) | 131,786 (52.47%) |
| HyperFormula | 177,491 (70.67%) | 161,759 (64.40%) | 120,223 (47.87%) |

The tighter and exact columns are sensitivity checks, not replacement scores.
Their common drop shows why floating-point spreadsheet results should not be
described with exact equality alone.

### Enron NYMEX

The saved reference was produced by Microsoft Excel AppVersion `15.0300`.

| engine | outcome | values / 34,583 | matched / compared | wall | peak RSS |
|---|---|---:|---:|---:|---:|
| CellRune | completed | 34,583 | 34,583 / 34,583 (100.00%) | 1 s | 282 MiB |
| IronCalc | completed | 34,583 | 34,583 / 34,583 (100.00%) | 22 s | 198 MiB |
| Formualizer | completed | 34,583 | 32,324 / 34,583 (93.47%) | 186 s | 195 MiB |
| xlstream | evaluation failure | 0 | — | <1 s | 15 MiB |
| LogiSheets | completed | 34,583 | 34,583 / 34,583 (100.00%) | 69 s | 1,171 MiB |
| recalc-engine | completed | 34,583 | 34,583 / 34,583 (100.00%) | 1 s | 290 MiB |
| HyperFormula | completed | 34,583 | 34,582 / 34,583 (100.00%) | 3 s | 555 MiB |
| xlsx-calc | completed | 14,493 | 7,351 / 14,493 (50.72%) | 5 s | 432 MiB |
| formulas | timeout | 0 | — | 540 s | not recorded |
| pycel | completed | 34,583 | 34,583 / 34,583 (100.00%) | 246 s | 1,207 MiB |
| xlcalculator | timeout | 0 | — | 540 s | not recorded |

CellRune, IronCalc, LogiSheets, recalc-engine, and pycel agreed with every saved
value. HyperFormula differed at one cell. The engines with identical agreement
still differed substantially in runtime and memory on this host.

Wall time and peak RSS in both public tables are rounded to whole seconds and
MiB because these were single observations, not repeated performance samples.

### Observed stops

The retained stderr logs distinguish the large-workbook stops instead of
assigning them all to generic lack of support:

- On SWR, IronCalc rejected legacy array formulas during XLSX loading.
- xlstream rejected an expression outside its documented streaming model:
  SWR used a structural reference form outside an aggregate, and NYMEX used a
  cross-row reference. See xlstream's
  [streaming-model documentation](https://github.com/cilladev/xlstream/blob/main/docs/architecture/streaming-model.md).
- On SWR, LogiSheets panicked on an internal `Option::unwrap()` in its OOXML
  complex-type loader.
- On SWR, formulas rejected an `OFFSET`-containing formula as invalid syntax,
  and xlcalculator ended in its tokenizer with an `IndexError`.
- On NYMEX, formulas and xlcalculator crossed the 540-second process limit.

These are terminal observations from the named versions and inputs. The report
does not promote an adapter traceback to a confirmed upstream root cause, and
it does not link an issue that the relevant maintainer has not confirmed.

### Reduced inputs for three of those stops

Three of the stops above were narrowed to a minimal input, each with a control
input differing only in the trigger. This records what the reduced inputs
demonstrate on the measured versions. It remains a local observation: no
maintainer has confirmed any of it, and none of these engines is claimed to be
defective on any version other than the one measured.

- **LogiSheets 1.8.0.** The trigger is a `<calcPr>` element that omits the
  optional `calcId` attribute. A 2 KB workbook with `<calcPr/>` panics; the same
  workbook with `<calcPr calcId="152511"/>` loads, and so does one with no
  `calcPr` element at all. The SWR workbook is a Google Sheets export and
  carries a bare `<calcPr/>`, while the NYMEX workbook carries `calcId` and
  loaded with full agreement. A separate input showed that omitting the
  optional `xl/styles.xml` part panics at a different site.
- **xlcalculator 0.5.0.** The tokenizer raises `IndexError` for any formula
  ending in a space or newline, with no workbook involved:
  `ExcelParser().getTokens("=A1*3 ")`.
- **formulas 1.3.4.** `OFFSET` is not the trigger. `=SUM(A1:OFFSET(A1,2,0))`
  parses and so does `=SUM(A1:OFFSET($A$1,2,0))`; what fails is a `$`-anchored
  left operand of the range operator combined with a sheet-qualified or
  function-valued right operand, as in `=SUM($A$1:OFFSET(A1,2,0))`. The SWR
  formula matched that shape. A related parse gap for a function-valued left
  operand at the top level is the subject of the still-open upstream
  [issue #101](https://github.com/vinci1it2000/formulas/issues/101) from 2022,
  whose own reported case parses on 1.3.4.

Reduced inputs were prepared so the observations can be reported upstream in a
form a maintainer can act on. Where an upstream report already exists, the
intent is to add to it rather than duplicate it.

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
authoritative cross-corpus results:

- The xlcalculator adapter quoted worksheet names even though its model keys
  are unquoted, causing valid lookups to appear blank.
- The xlsx-calc adapter could serialize an invalid JavaScript date as a null
  numeric value and stop a file on an unsupported array result.

All 283 files were repeated for each affected engine. The original runs were
retained and marked as superseded rather than overwritten.

The authoritative result set contains 3,113 cross-corpus run records and 44
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
- The large-workbook sample is two files: one Google Sheets export and one
  workbook with an Excel 2013-era saved cache. It contains no large workbook
  authored and saved by a current Excel release.
- Exact historical acquisition commits for the four upstream corpora were not
  retained.
- Results depend on the measured engine versions, adapters, host, runtime
  versions, and timeout policy.
- Wall time and peak RSS are single-host observations, not stable performance
  rankings.
- The comparison harness, third-party workbook copies, and per-cell result
  streams are retained development evidence but are not distributed in this
  repository. The public report provides source links, hashes, configuration,
  protocol, and aggregates; it is not independently reproducible from this
  repository alone.
- This report measures CellRune 0.1.1 and does not change the current release's
  documented XLSX or formula support.
