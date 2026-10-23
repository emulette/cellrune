# CellRune

CellRune is a headless Rust backend for bounded XLSX/XLSM reading and deterministic formula
calculation. It keeps the workbook read from disk immutable, returns recalculated values in a
separate snapshot, and can retain an exact package backing for explicit round-trip writing. Its
MCP integration is a local stdio workflow server, not a browser, UI, hosted service, or remote
transport.

## Rust installation

The CellRune Rust crate 0.1.26 requires Rust 1.88 or newer.

```bash
cargo add cellrune@0.1.26
```

Or add the dependency directly:

```toml
[dependencies]
cellrune = "0.1.26"
```

## Features

- reads `.xlsx` files from paths, byte slices, or `Read + Seek` streams;
- opens package-backed `.xlsx` and `.xlsm` documents with exact SHA-256 identity, borrowed access
  to the retained input bytes, and bounded round-trip preservation, including from an owned byte
  vector that it keeps without copying and can hand back;
- preserves sheet order, sparse cells, formulas, saved results, defined names, and relevant
  number-format metadata;
- exposes merged ranges, hidden rows, worksheet-level autoFilter ranges and their filtered
  columns, and validated worksheet-owned Excel
  tables, including stable table and column IDs, complete filter/sort/formula/style metadata, and
  case-insensitive lookup indexes;
- reads every worksheet data validation rule, including Excel 2010 extension rules, and worksheet
  hyperlinks and cell comments as read-only metadata;
- imports and exports one sheet as RFC 4180 CSV under a documented, lossless value rule;
- reads and writes a minimal cell formatting model (font, fill, border, alignment) with column
  widths and row heights, patching only the style records an edit needs into existing packages;
- resolves typed multi-area, 3-D, structured table, current-row, and spill references under
  cumulative reference and dependency budgets;
- inspects workbook and sheet-local defined names without running a calculation session, preserving
  rectangular, 3-D, ordered multi-area, empty, dynamic, external, invalid, and unsupported results;
- expands shared formulas while preserving absolute and relative references;
- returns typed formula values and stable per-cell calculation issues in one result snapshot;
- reports normalized per-workbook function demand and exposes the implemented function catalog;
- calculates average-tie ranks, linear forecasts, matrix determinants, dated cash-flow present
  values, declining-balance depreciation, normal/lognormal inverses, and deterministic
  text-to-number conversions;
- evaluates first-class and defined-name `LAMBDA` callables, including immediate invocation,
  `ISOMITTED`, `MAP`, `BYROW`, `BYCOL`, `REDUCE`, `SCAN`, and `MAKEARRAY`, under explicit
  recursion and iteration limits;
- evaluates audited 3-D aggregate references across workbook tab order, including hidden sheets and
  reverse-written endpoints, without expanding the sheet span into dependency edges;
- applies configurable limits to ZIP, XML, workbook, formula, dependency, text, and array work;
- never executes macros, never follows external links, never reads the host clock for
  `TODAY()` or `NOW()`, and draws `RAND()`, `RANDBETWEEN()`, and `RANDARRAY()` from an explicit
  seed instead of host randomness;
- returns stable error and issue codes for programmatic handling, listed in
  [`docs/ERROR_CODES.md`](https://github.com/emulette/cellrune/blob/main/docs/ERROR_CODES.md);
  read, formula, and targeted-calculation limit failures also name the exceeded limit or report
  the observed value;
- materializes recalculated typed results into existing `.xlsx`/`.xlsm` packages with strict or
  explicit cache-invalidation policies and reports a verified output SHA-256 identity;
- writes `WorkbookDraft` edits into an existing package without calculating, clearing only the
  saved results the edits can affect and asking the host to recalculate on open only when needed;
- creates canonical `.xlsx` workbooks and applies typed cell, formula, sheet (including sheet
  deletion), name, table rename, table-column rename, table-row resize, table and autoFilter column
  widening, number-format, date-system, and calculation-property edits through `WorkbookDraft`;
- reads, queries, preserves, and explicitly authors SpreadsheetML phonetic annotations and default
  frozen panes without mixing presentation state into formula calculation;
- exposes the versioned read/edit/calculate/write contract through typed Python and
  Node.js/TypeScript native packages, including saves of edits without calculating, adjustable
  read and calculation limits, and capability scans; the table under
  [Language bindings](#language-bindings) lists what each surface offers;
- reports the cells and ranges a formula reads and the formulas that read a cell, from the
  dependency graph of the installed calculation, marking references that depend on values;
- supports atomic typed edit batches, persistent parsed/dependency state, safe incremental
  recalculation, bounded result deltas, cooperative cancellation, stale-result rejection, and
  retained immutable change previews;
- provides a local stdio MCP server with high-level open, inspect, edit, preview, recalculate,
  range-read, delta, and verified Save As tools over the same interop session; and
- raw-copies unchanged package entries without exposing ZIP or XML implementation types.

## Usage

```rust
use cellrune::{
    CalculationCellResult, CalculationOptions, ReadOptions, calculate_workbook, read_xlsx_path,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let workbook = read_xlsx_path("input.xlsx", ReadOptions::default())?;
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());

    for (cell, result) in calculation.cells() {
        match result {
            CalculationCellResult::Value(value) => println!("{cell:?}: {value:?}"),
            CalculationCellResult::Unavailable(issue) => {
                eprintln!("{cell:?}: {}", issue.code().as_str());
            }
        }
    }

    Ok(())
}
```

Reading and calculation are separate operations. `calculate_workbook` does not modify the source
`WorkbookSnapshot` or its saved XLSX results. It attempts every formula in the workbook:
successfully calculated cells contain a typed `Value`, while a cell that cannot be calculated
contains a structured `Unavailable(CalculationIssue)`. One unavailable formula does not suppress
independent results; dependent formulas report `BlockedByUpstream` when applicable.
Volatile functions require deterministic inputs through `CalculationOptions`:
`with_today_serial` for `TODAY()`, `with_now_serial` for `NOW()`, and `with_random_seed` for
`RAND()`, `RANDBETWEEN()`, and `RANDARRAY()`. A draw depends only on the seed, the cell, and the
call's position in its formula, so wherever full calculation produces a value the other
calculation modes produce the same one; the generator is not cryptographically secure. Use
`with_locale_profile` to select the built-in English (United States) or Korean (Korea)
conventions that `DOLLAR`, the names `TEXT` shows, the double-byte text functions, and the
`GROUPBY` and `PIVOTBY` total labels follow; CellRune never reads the host locale. Use
`with_arithmetic_semantics` and `with_financial_solver_semantics` to opt into the raw IEEE-754 and
extended-search behavior shipped through 0.1.2; the defaults select Excel-compatible cancellation
and Microsoft's function-specific solver budgets. Use
`supported_function_catalog` for the build's exact function surface and `scan_function_usage` to
summarize the functions used by a workbook. `scan_formula_capabilities` remains available as an
optional static inventory for migration planning and user-interface reporting; calculation does
not require it.

CellRune 0.1.14 adds exactly 19 official Excel-facing engineering names: `CONVERT`, `BESSELI`,
`BESSELJ`, `BESSELK`, `BESSELY`, `COMPLEX`, `IMABS`, `IMAGINARY`, `IMARGUMENT`, `IMCONJUGATE`,
`IMREAL`, `IMDIV`, `IMPOWER`, `IMPRODUCT`, `IMSUB`, `IMSUM`, `IMEXP`, `IMLN`, and `IMSQRT`.
`CONVERT` uses a typed, case-sensitive unit registry; the complex family shares one finite parser
and canonical formatter; and the four Bessel functions use budgeted first-party kernels.

The fixed-income wave on top of 0.1.14 adds exactly 26 official names: `ACCRINT`, `ACCRINTM`,
`COUPDAYBS`, `COUPDAYS`, `COUPDAYSNC`, `COUPNCD`, `COUPNUM`, `COUPPCD`, `DISC`, `DURATION`,
`INTRATE`, `MDURATION`, `ODDFPRICE`, `ODDFYIELD`, `ODDLPRICE`, `ODDLYIELD`, `PRICE`, `PRICEDISC`,
`PRICEMAT`, `RECEIVED`, `TBILLEQ`, `TBILLPRICE`, `TBILLYIELD`, `YIELD`, `YIELDDISC`, and
`YIELDMAT`. They share a typed day-count and coupon-schedule model and a safeguarded yield root
solver that charges the calculation budget and observes cancellation.

CellRune 0.1.22 adds 22 official statistical, distribution, and complex-logarithm names,
including exclusive percentiles, `TRIMMEAN`, the `A` variances, `STEYX`, `PROB`, chi-square,
Weibull, and confidence-interval functions, and `IMLOG10`/`IMLOG2`; the exact list is in
[llms.txt](https://github.com/emulette/cellrune/blob/main/llms.txt).

CellRune 0.1.26 adds the seeded random functions `RAND`, `RANDBETWEEN`, and `RANDARRAY`.

CellRune 0.1.27 adds the shape statistics `SKEW`, `SKEW.P`, and `KURT`, the array statistics
`FREQUENCY` and `MODE.MULT`, the chi-square test and quantiles `CHISQ.TEST`, `CHISQ.INV`, and
`CHISQ.INV.RT` with their legacy names, the financial functions `CUMIPMT`, `CUMPRINC`, `VDB`,
`AMORLINC`, and `AMORDEGRC`,
the complex trigonometric functions `IMSIN`, `IMCOS`, `IMTAN`, `IMCOT`, `IMSEC`, `IMCSC`,
`IMSINH`, `IMSECH`, and `IMCSCH`, `ENCODEURL`, `BAHTTEXT`, and `CELL` for the information that
depends only on the workbook; the exact list is in
[llms.txt](https://github.com/emulette/cellrune/blob/main/llms.txt).

CellRune 0.1.27 adds `AGGREGATE`, `FIXED`, the byte-counting text functions `LENB`, `LEFTB`,
`RIGHTB`, `MIDB`, `FINDB`, `SEARCHB`, and `REPLACEB`, and `ASC` and `DBCS`, and formats `TEXT`
with Excel number-format codes: sections and conditions, dates and times in both date systems,
elapsed time, scientific notation, fractions, and thousands scaling. A construct whose Excel output
has not been established, such as a fixed fraction denominator, stays an explicit `Unavailable`
result rather than a guess; the details are in
[llms.txt](https://github.com/emulette/cellrune/blob/main/llms.txt).

CellRune 0.1.16 added `XLOOKUP`, `DATEVALUE`, `TIMEVALUE`,
`NETWORKDAYS.INTL`, and `WORKDAY.INTL`. Their fixed grammar, lookup modes, calendar rules,
wildcard behavior, and array-result boundaries are documented alongside the current function
catalog in [llms.txt](https://github.com/emulette/cellrune/blob/main/llms.txt).

The regex functions target PCRE2 semantics with bounded compile, matching, capture, and output
work. CellRune's prebuilt Python, Node.js, and MCP artifacts pin the bundled PCRE2 10.46 engine;
Rust consumers use `pcre2-sys` linkage policy and can set `PCRE2_SYS_STATIC=1` to select its bundled
source build.

`INDEX` follows Excel's zero-index reference behavior: a zero row or column selects the complete
corresponding column or row, and zero for both selects the complete input range. Scalar formulas
apply legacy implicit intersection, while array formulas can materialize the selected rectangle.
Unary and binary array operators that combine whole-column references use one common extent: the
greatest populated row among the columns those operands reference, with a one-row minimum for an
otherwise empty sheet. Cells in other columns of the same sheet do not widen it, so the value
depends only on the cells the expression's own dependency rectangles cover and a full and an
incremental recalculation agree. Missing cells within that extent are blanks, and directly
resolved source arrays plus operator outputs are charged to its cumulative array-cell budget. Function calls keep function-defined
argument boundaries: each array argument is evaluated under its own bounded context, and a
function's return does not establish a whole-column operator extent by itself. When another direct
whole-column operand has established such an enclosing context, the returned array is charged to
that context before the operator consumes it.

Direct 3-D references are accepted by `SUM`, `AVERAGE`, `AVERAGEA`, `COUNT`, `COUNTA`, `MAX`,
`MAXA`, `MIN`, `MINA`, `PRODUCT`, `STDEV.P`, `STDEV.S`, `VAR.P`, and `VAR.S` (including the
catalogued legacy aliases). The span follows workbook tab order, so hidden sheets participate and
reversed endpoint spelling produces the same set. Excel-defined direct 3-D error contexts remain
values: `INDEX` and `VLOOKUP` return `#VALUE!`, while `OFFSET` returns `#REF!`. Other direct 3-D
consumers remain an explicit `UnsupportedSheetRange` capability issue. A bare 3-D reference or one
composed through an array operator returns `#VALUE!`; it does not silently inherit an enclosing
aggregate's collection policy. The static capability scan and evaluator share this policy.

For repeated programmatic edits, use `WorkbookCalculationSession` instead of rebuilding stateless
calculation state after every cell:

```rust
use cellrune::{
    CalculationOptions, CancellationToken, CellAddress, CellValue, EditBatch, FiniteNumber,
    RecalculationMode, SheetId, WorkbookCalculationSession, WorkbookChange,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut session = WorkbookCalculationSession::create();
    let sheet = SheetId::new(1)?;
    let receipt = session.apply_changes(
        0,
        EditBatch::new([WorkbookChange::set_cell_value(
            sheet,
            CellAddress::from_a1("A1")?,
            CellValue::Number(FiniteNumber::new(42.0)?),
        )]),
    )?;
    let delta = session.recalculate(
        RecalculationMode::Auto,
        CalculationOptions::default(),
        CancellationToken::new(),
    )?;
    assert_eq!(delta.result_revision(), receipt.result_revision());

    Ok(())
}
```

`Auto` evaluates a proven dirty subset and falls back to the same full-workbook calculation
semantics when name, sheet, option, dynamic-reference, or spill topology is uncertain; an edited
cell formula is parsed and wired into the dependency graph on its own.
Forced incremental mode fails closed instead of guessing. Sessions use optimistic semantic
revisions for atomic edits, retain bounded result-delta history, and reject stale calculations.
A batch whose accepted operations make no semantic change keeps the current revision, topology,
and installed calculation instead of forcing a redundant recalculation.
Long-running work can be prepared outside the session lock with `prepare_recalculation`, cancelled
through a request-owned `CancellationToken`, and installed only if its source revision is still
current.

Use targeted calculation when a headless service needs only a few outputs. Rust exposes
`calculate_targets` and `WorkbookCalculationSession::calculate_targets`; Python and Node expose
the same operation without requiring a prior full calculation:

```python
result = workbook.calculate_targets([
    {"sheet": "Sheet1", "start": "B1"},
    {"sheet": "Sheet1", "start": "D1", "end": "D10"},
])
```

```javascript
const result = await workbook.calculateTargets([
  { sheet: "Sheet1", start: "B1" },
  { sheet: "Sheet1", start: "D1", end: "D10" },
]);
```

The separate partial result contains only requested cells, deduplicated in sheet-ID and row-major
order, with typed values/issues, revision, fingerprint, provenance, options, and work counts.
Only target formulas and required precedents are parsed/evaluated; layout metadata is inspected
across the source workbook. A current complete cache with matching options can supply results.
Partial requests preserve dirty state, full results, delta history, previews, and save requirements.
Defaults bound input targets to 1,024, returned cells to 10,000, and evaluator invocations to
100,000; dynamic retries count toward the work budget. Request limits can be set with `limits`.
Known array followers calculate their entire anchor. For an undeclared spill whose owner has
never been calculated, include its anchor in the request; an unknown follower alone cannot locate
an arbitrary formula elsewhere in the workbook. Saved formula results are never used as current
calculation values. See [llms.txt](https://github.com/emulette/cellrune/blob/main/llms.txt) for
the complete Rust types and transport contract.

Undeclared spills outside the dependency scope are not discovered. Use declared spill ranges or
full calculation when independently anchored spills may overlap. A reproducible comparison of
small and complete scopes is recorded in the
[targeted calculation benchmark](https://github.com/emulette/cellrune/blob/main/release-tests/integration/benches/targeted_calculation.md).

The 0.1.16 change-preview workflow captures an immutable base/candidate transaction
without mutating the live workbook. Retention belongs to `cellrune-interop::WorkbookSession`, which
allows one active preview calculation and one published preview. Publication is two phase: a failed,
cancelled, stale, or oversized replacement leaves the previous published preview available. A
successful semantic mutation, commit, discard, or session close drops the published preview.
Preview details use core-owned opaque cursors; a cancelled or resource-limited pre-commit remains
retryable, while a stale or successful commit consumes its preview. The complete Rust and binding
call shapes are documented in
[llms.txt](https://github.com/emulette/cellrune/blob/main/llms.txt).

Table authoring uses stable identities rather than positional names. Construct
`WorkbookChange::rename_table`, `rename_table_column`, or `resize_table_rows` and include it in the
same atomic `EditBatch` as other workbook changes. Renames update cell formulas, defined names,
calculated-column formulas, and totals-row formulas through one typed source-span rewrite path.
Resize preserves table identity and filter/sort orientation, materializes calculated and totals
cells, and supports expanding a header-only empty table. Through
`WorkbookCalculationSession`, formula rewriting and table materialization are bounded by
`SessionLimits`. A successful `EditReceipt` lists the affected stable table IDs; any invalid
target, collision, rewrite error, resource limit, or cancellation rolls the whole batch back.

`WorkbookChange::delete_sheet` and `WorkbookDraft::delete_sheet` delete a sheet with the names
scoped to it and its tables. References to them in the remaining formulas, defined names, and table
formulas become `#REF!` as Excel saves them, a deleted end of a 3-D reference moves to the nearest
remaining worksheet, and `INDIRECT` text is left as it is. Deleting the last visible sheet fails with
`validation.last_visible_sheet`. A sheet added later in the same draft or session never takes a
deleted sheet's ID; a saved file does not record deleted IDs, so after reopening, an added sheet can
take one. Saving into the source
package removes the parts only the deleted sheet reached and rewrites references in conditional
formats, data validations, and chart series to `#REF!`; a remaining pivot cache or pivot chart that
reads the deleted sheet, a form control, VML control, linked shape, or data consolidation that names
it, or a slicer or timeline cache while it holds a pivot table or table, fails with
`xlsx.write.unsupported_preservation`. See
[llms.txt](https://github.com/emulette/cellrune/blob/main/llms.txt) for the complete rules.

Use `analyze_defined_name`, `analyze_defined_name_with_options`, or
`analyze_defined_name_cancellable` to inspect a defined name against an immutable
`WorkbookSnapshot`. The typed result distinguishes a single rectangle, a 3-D span, ordered
multi-area geometry, a valid empty reference, dynamic formulas, constants, external targets,
invalid definitions, unsupported expressions, and missing names.

`open_xlsx_document_*` retains the exact input package for writing. `open_xlsx_document_vec`
keeps an owned `Vec<u8>` as that package without copying it, and
`XlsxDocument::into_input_bytes` hands the bytes back, copying them only while a draft or clone
still shares them.
`write_recalculated_xlsx_bytes`, `write_recalculated_xlsx`, and
`write_recalculated_xlsx_path` bind a calculation to that exact input, update typed formula
caches, remove stale calculation chains, preserve unrelated package content, and reopen the output
before reporting success. Their `WriteReport::output_hash()` returns an `OutputHash`: the SHA-256
of the exact verified output archive bytes, deliberately distinct from the input identity. Strict
mode rejects incomplete calculations without producing an artifact; cache invalidation is an
explicit opt-in policy. `write_preserved_xlsx_bytes` remains available for an unchanged
preservation copy.

`WorkbookDraft::new` creates a canonical workbook, while
`WorkbookDraft::from_document` retains the source package for preservation-aware edits. Calculate
the draft's current `workbook()` and pass both objects to `write_xlsx_draft_bytes`,
`write_xlsx_draft`, or `write_xlsx_draft_path`. A mutation that changes workbook semantics advances
the semantic revision, so a calculation made before the latest effective edit is rejected. An
accepted no-op keeps the revision unchanged. Path writes are Save As operations and never replace
an existing destination unless replacement is explicitly enabled. Canonical drafts can author
dynamic-array formulas with `WorkbookDraft::set_cell_dynamic_formula`; calculation resolves their
spill region, detects occupied targets, and materializes followers for writing. Existing
document-backed dynamic formulas can be recalculated without changing their metadata, while adding
or replacing one is rejected until source metadata-index merging is implemented.

`write_xlsx_draft_edits_bytes`, `write_xlsx_draft_edits`, and `write_xlsx_draft_edits_path` write
the edits of a document draft without a calculation. They remove the saved results of the formulas
the edits can affect, listed by `EditWriteReport::affected_formulas()`, and only then set
`fullCalcOnLoad` so the host recalculates on open. Every other saved result, `calcChain.xml` while
the formula layout is unchanged, `calcPr`, and untouched parts keep their source bytes, and an
uncalculable formula elsewhere does not block the write. Formulas whose references are known only
after evaluation, such as `INDIRECT` or an `OFFSET` whose position reads another formula, always
count as affected when anything changed.
Cells the draft creates take the style of their row or column, as in Excel.

Package-backed documents expose phonetic annotations and frozen panes through
`XlsxDocument::presentation()`. `WorkbookDraft` provides atomic `set_annotated_text`,
`set_phonetics`, `clear_phonetics`, `set_frozen_pane`, and `clear_frozen_pane` mutations.
Phonetic base ranges are zero-based half-open UTF-16 code-unit ranges. Presentation-only changes
have a separate revision and reuse an otherwise current calculation. Source rich-text phonetic
editing, RTL pane authoring, and `PHONETIC()` calculation remain explicit unsupported boundaries.

Runnable examples are shipped in the crate package under `examples/` and live at
`crates/cellrune/examples/` in this repository. From the repository root, run one with
`cargo run -p cellrune --example <name> -- [arguments]`; from an extracted crate package, omit
`-p cellrune`. See the public
[llms.txt reference](https://github.com/emulette/cellrune/blob/main/llms.txt) for the complete
example inventory and a condensed public API reference.

## Language bindings

Python uses the mainstream PyO3 + maturin native-extension path. Node.js and TypeScript use napi-rs
over stable Node-API with Promise-backed native work and exact-version platform packages. Neither
binding requires a consumer Rust toolchain when installed from a wheel or prebuilt npm artifact.

The 0.1.26 release line targets Python 3.10 through 3.14 and Node.js 22 or newer. Install the
bindings with:

```bash
python -m pip install "cellrune==0.1.26"
npm install "@cellrune/node@0.1.26"
```

The bindings and the MCP server share one versioned interop contract, but not every Rust capability
reaches every surface:

| Capability | Rust | Python | Node.js | MCP |
| --- | --- | --- | --- | --- |
| Open, summary, range read, v1/v2 edits, calculate, recalculate, targeted calculation, previews, function usage, capability scan | yes | yes | yes | yes |
| Save calculated results | yes | yes | yes | yes |
| Save edits without calculating | yes | yes | yes | yes |
| Cell number formats, hidden rows, sheet autoFilter ranges and filtered columns, `fullCalcOnLoad` | yes | yes | yes | yes |
| Sheet deletion (`delete_sheet`, Node.js `deleteSheet`) | yes | yes | yes | yes |
| Sheet move and copy, defined-name rename, sheet autoFilter set and removal, table creation, range change, and conversion to a range | yes | yes | yes | yes |
| Row and column insertion, deletion, and moves, range moves and copies, and the inverse changes in the receipt of these and of sheet, defined-name, table, and autoFilter edits | yes | yes | yes | yes |
| Locale profile and number-format display text | yes | yes | yes | yes |
| Data validation rules, hyperlinks, and comments (read-only) | yes | yes | yes | yes |
| Cell formats, column widths, and row heights (read and edit) | yes | yes | yes | yes |
| CSV import and export of one sheet | yes | yes | yes | yes |
| Adjustable read and calculation limits | yes | yes | yes | defaults only |
| Defined-name inspection | yes | yes | yes | no |
| Sheet autoFilter and table column widening edits | yes | yes | yes | yes |
| Phonetic annotations and frozen panes (read and edit) | yes | yes | yes | yes |
| Typed limit diagnostics (`observed`, limit kinds, evaluation counts) | yes | yes | yes | yes |
| Rectangle value read and write as rows (interop `read_values`/`write_values`) | yes | yes | yes | no |
| Precedent and dependent queries | yes | yes | yes | yes |

Native package availability remains platform-specific; package managers must select a wheel or
exact-version npm platform package compatible with the current runtime.
The random-function seed is a calculation option on every surface: Python `random_seed` and
Node.js `randomSeed` on calculate, recalculate, targeted calculation, and previews, and MCP
`random_seed` on `workbook_recalculate`, `workbook_calculate_targets` options, and
`workbook_preview_changes`. It accepts an integer from 0 through 4294967295.
The locale profile travels the same way: Python `locale_profile={"locale": "ko-KR"}` and Node.js
`localeProfile: { locale: "ko-KR" }`, each with an optional display language, and MCP
`locale_profile` on the same tools; `en-US` and `ko-KR` are the built-in locales.
`format_display_text` (Rust and Python), `formatDisplayText` (Node.js), and the MCP
`format_display_text` tool return the text a cell shows for a value and a number-format code, so
an application that draws its own grid does not rebuild Excel's format engine.
Python `inspect_defined_name` and Node.js `inspectDefinedName` expose the typed defined-name query.
The existing `apply_changes`/`applyChanges` v1 shapes are unchanged; the separate
`apply_changes_v2`/`applyChangesV2` methods add stable-ID table rename, table-column rename, and
table-row resize plus `changed_table_ids`/`changedTableIds` receipts.

Node.js `calculate`, `recalculate`, `calculateTargets`, and `previewChanges` take an optional
`signal` (`AbortSignal`). Aborting it cancels that call's calculation through the session's
cooperative cancellation, so the Promise rejects with `session.cancelled`, or
`calculation.target.cancelled` for `calculateTargets`, and nothing is installed or published; an
already aborted signal cancels the call before it calculates.

In the current bindings, Python exposes synchronous
`preview_changes`, `preview_changes_page`, `commit_preview`, and `discard_preview`; the long
native preview operation releases the GIL. Node.js exposes Promise-backed `previewChanges`, then
synchronous `previewChangesPage`, `commitPreview`, and `discardPreview`. Python DTO fields use
snake case and integer IDs; Node DTO fields use camel case and `bigint` IDs. A successful Python
write report has `output_sha256`; the Node `WriteReport` has `outputSha256`.
[llms.txt](https://github.com/emulette/cellrune/blob/main/llms.txt)
defines the shared lifecycle and pagination semantics.

Python workbooks are context managers:

```python
from cellrune import Workbook

with Workbook.create() as workbook:
    workbook.set_number("Sheet1", "A1", 41.0)
    workbook.set_formula("Sheet1", "B1", "=A1+1")
    workbook.calculate()
    # 0.1.2-compatible calculation remains available when required:
    workbook.calculate(
        arithmetic_semantics="ieee_754",
        financial_solver_semantics="extended_search",
    )
    workbook.save("output.xlsx", replace_existing=True)
```

In a Node.js ES module, close the workbook in `finally`:

```js
import { Workbook } from "@cellrune/node";

const workbook = Workbook.create();
try {
  workbook.setNumber("Sheet1", "A1", 41);
  workbook.setFormula("Sheet1", "B1", "=A1+1");
  await workbook.calculate();
  await workbook.calculate({
    arithmeticSemantics: "ieee_754",
    financialSolverSemantics: "extended_search",
  });
  await workbook.save("output.xlsx", { replaceExisting: true });
} finally {
  workbook.close();
}
```

`save`/`toBytes` write CellRune's calculated results and, unless `invalidate_unavailable` is set,
fail with `xlsx.write.incomplete_calculation` when any result is unavailable; the error's
`details.cells` names the first 64 such formulas. To change an existing file without calculating,
save only the edits instead. Formulas the edits can affect lose their saved results and the file
asks the spreadsheet app to recalculate them on open; every other saved result and untouched part
keeps its source bytes, and formulas CellRune cannot calculate do not block the save:

```python
from cellrune import Workbook

with Workbook.open_path("input.xlsx") as workbook:
    workbook.set_number("Sheet1", "A1", 42.0)
    report = workbook.save_edits("edited.xlsx", replace_existing=True)
    print(report["affected_formulas"], report["requests_host_recalculation"])
```

```js
import { Workbook } from "@cellrune/node";

const workbook = await Workbook.openPath("input.xlsx");
try {
  workbook.setNumber("Sheet1", "A1", 42);
  const report = await workbook.saveEdits("edited.xlsx", { replaceExisting: true });
  console.log(report.affectedFormulas, report.requestsHostRecalculation);
} finally {
  workbook.close();
}
```

`edits_to_bytes`/`editsToBytes` return the same package in memory. A workbook created rather than
opened has no package to edit and fails with `xlsx.write.invalid_package_plan`. Node.js `save`,
`toBytes`, `saveEdits`, and `editsToBytes` read the workbook when their worker runs, not when they
are called, so do not edit the workbook until the returned Promise settles: an edit made meanwhile
may or may not be saved, or may fail with `interop.session.unavailable`.

Opening takes `read_limits`/`readLimits` and calculating takes
`calculation_limits`/`calculationLimits` to change individual default limits, for example
`Workbook.open_path(path, read_limits={"max_entry_uncompressed_bytes": 256 << 20})`. Omitted limits
keep their defaults; a zero fails with `interop.limit.invalid`, and limits that are not an object
(a `dict` in Python), an unknown name, or a value that is not a positive integer with
`interop.limit.payload_invalid`. `scan_capabilities`/
`scanCapabilities` page through the static per-formula support report. Range cells carry a
`number_format` with its ID, code, and kind, and the summary reports each sheet's `auto_filter`
range and filtered columns, its hidden rows, and the package's `full_calculation_on_load`
declaration: xlsxwriter and similar producers
set it and may store placeholder results, so read such saved results only after calculating.
`data_validations`/`dataValidations` page through a sheet's data validation rules with `offset` and
`limit`; each rule carries its ranges, OOXML type, operator, and error-style tokens, its formulas as
stored, and its flags and messages. `hyperlinks` and `comments` page the same way through a sheet's
hyperlinks and cell comments. `from_csv`/`fromCsv` open RFC 4180 UTF-8 CSV as a one-sheet
workbook and `to_csv`/`toCsv` export one sheet, formulas as their values in the current calculated
revision; a sheet without formulas needs no calculation. Reading never
creates a formula: unquoted decimal numbers, `TRUE`/`FALSE` in any case, and Excel error spellings
become numbers, logicals, and errors, quoted fields and everything else become text, and dates are
not parsed. Writing quotes text that would read back as anything else, so a write and a read keep
every value at its address. Text is written as it is: text starting with `=`, `+`, `-`, or `@`
opens as a formula in Excel and other spreadsheets, so neutralize untrusted text before exporting
a CSV that a spreadsheet will open. `read_formats`/`readFormats` page through the font, fill,
border, and alignment of a range's cells with its column widths and the page's row heights, and
`apply_changes_v2`/`applyChangesV2` take `set_cell_format`/`setCellFormat`,
`set_column_width`/`setColumnWidth`, and `set_row_height`/`setRowHeight` changes, which keep the
semantic revision. A cell format change merges into the cell's format: its `font`, `border`, and
`alignment` set only the properties they give, so `{"font": {"bold": true}}` keeps the font's name,
size, and color, and its `fill` replaces the whole fill.
The v2 batch also widens a table (`widen_table_columns`/`widenTableColumns`
with a `table_id` and the new `range`) or a sheet autoFilter
(`widen_sheet_auto_filter`/`widenSheetAutoFilter` with a `sheet` and the new `range`), and sets a
sheet's frozen pane (`set_frozen_pane`/`setFrozenPane` with `rows` and `columns`; zero clears it)
or a text cell's phonetic runs (`set_phonetics`/`setPhonetics`, `clear_phonetics`/
`clearPhonetics`), which keep the semantic revision. The summary reports a sheet's `frozen_pane`
when it has one, and Python and Node.js `phonetics` page through a sheet's annotated cells. A
failure that exceeded a resource limit carries its figures in `details.limit` (`name`,
`observed`, `requested_cell_count`/`requestedCellCount`, `evaluated_count`/`evaluatedCount`), and
an unavailable result that exceeded a calculation limit names it in `limit`; both are absent
otherwise.

Python `read_values(sheet, start, end)` and Node.js `readValues(sheet, start, end)` read a
rectangle of up to 1,048,576 cells as rows of typed values without numpy or Arrow: each cell holds
its current calculated result, or an `unavailable` result, once the workbook has calculated the
current revision, and otherwise its literal or saved value. `write_values(sheet, start, rows)` and
`writeValues(sheet, start, rows)` write equal-length rows of values from `start` as one atomic
edit batch, where a blank value clears its cell, and return the edit receipt.

Python `precedents(sheet, address)` and `dependents(sheet, address)` and the Node.js methods of
the same names, with `offset` and `limit`, page through the cells and ranges a cell's formula
reads and the formulas that read the cell directly. They use the dependency graph of the
calculation installed for the current revision, so they fail with `interop.calculation.required`
until the workbook is calculated after an edit. A formula whose references depend on values, such
as `INDIRECT` or `OFFSET`, is marked `dynamic`: its precedents are those the installed values
resolved, and a dependents page counts the `unlisted_dynamic_formulas` that could read the cell
after a value changes.

Python and Node.js `close()` calls are idempotent. Once `close()` returns, the binding-owned native
session has been released. An active calculation is cooperatively cancelled and a published preview
is discarded; subsequent operations fail with the stable `interop.session.closed` code.

## Local MCP

`cellrune-mcp` is a local stdio-only MCP `2025-11-25` server for AI hosts. It exposes a finite set
of high-level workbook workflow tools; spreadsheet functions remain formulas inside the workbook
and are not registered one by one as MCP tools. Start it with one or more explicit filesystem
roots:

```bash
cargo run --locked -p cellrune-mcp -- \
  --root /absolute/path/to/approved/workbooks
```

Its 29 tools are `workbook_create`, `workbook_open`, `workbook_open_csv`, `workbook_close`,
`workbook_summary`, `workbook_export_csv`, `workbook_read_range`, `workbook_read_formats`,
`workbook_data_validations`, `workbook_hyperlinks`, `workbook_comments`, `workbook_phonetics`,
`workbook_precedents`, `workbook_dependents`, `workbook_function_usage`,
`workbook_scan_capabilities`, `workbook_apply_changes`, `workbook_apply_changes_v2`,
`workbook_recalculate`, `workbook_calculate_targets`, `workbook_changes_since`, `workbook_save_as`,
`workbook_save_edits_as`, `workbook_preview_changes`, `workbook_preview_changes_page`,
`workbook_commit_preview`, `workbook_discard_preview`, and the sessionless `format_display_text`
and `function_catalog`, which pages through the calculated functions with their canonical name,
alias, array-result, and official-list flags.
The v2 edit tool adds stable-ID table rename, table-column rename, and table-row resize while
retaining the v1 edit shapes. The four preview tools are the retained immutable transaction
workflow: preview returns a summary and ID, page returns a byte-bounded core-cursor page, and
commit or discard consumes the interop-owned preview. `workbook_save_as` returns the shared write
report, including its lowercase `output_sha256` output identity. `workbook_save_edits_as` writes a
session's edits into the file it was opened from without calculating, under the same root and
overwrite rules, and returns the edit report. MCP clients always use the default read and
calculation limits; raising them is left to the operator.

`workbook_read_range` also takes optional forms that save response bytes: `omit_empty` leaves
out empty cells, `values_only` returns address and value pairs in `values`, `compact` returns
`rows` of values without addresses, and `max_bytes` returns the longest prefix of the page that
fits, continued from `next_offset`. Offset and limit always count positions of the range, and a
read without these options returns exactly the page it did before. Python `read_range` and
Node.js `readRange` take the same options (`omitEmpty`, `valuesOnly`, `compact`, `maxBytes` in
Node.js).

`workbook_calculate_targets` accepts `session_id`, `targets`, optional `options`, and optional
`limits`. Its limits are capped at the defaults above. The entire partial response must fit
`--max-response-bytes`; an oversized response fails without installing state, so callers can
retry with fewer targets. It uses the existing request cancellation and session lifetime controls.

`workbook_apply_changes` and `workbook_apply_changes_v2` stage their batch on a blocking worker
and stop it through the same cooperative cancellation when the request is cancelled, failing with
`mcp.operation.cancelled` without changing the session.

`workbook_recalculate` always installs a completed calculation. When its delta exceeds
`--max-response-bytes`, it reports `mcp.recalculation.response_byte_limit_exceeded` with the
installed `result_revision` and `delta_cursor`; range reads and Save As use the installed results.

The server also publishes read-only JSON resources at `cellrune://support/functions` and the
`cellrune://sessions/{session_id}/summary` resource template. Operators can set
`--max-sessions`, `--session-ttl-seconds`, `--max-response-bytes`, `--max-workbook-bytes`, and
`--log-level`; run `cellrune-mcp --help` for their defaults. Values outside the server's compiled
policy limits are rejected at startup.

`cellrune-mcp` is not published to a package registry. Prebuilt bundles for Linux, macOS, and
Windows are attached to each [GitHub release](https://github.com/emulette/cellrune/releases),
alongside their license materials and build provenance.

An MCP client can launch a release binary with configuration equivalent to:

```json
{
  "mcpServers": {
    "cellrune": {
      "command": "/absolute/path/to/cellrune-mcp",
      "args": ["--root", "/absolute/path/to/approved/workbooks"]
    }
  }
}
```

The server canonicalizes configured roots at startup. Every workbook path supplied to a tool must
be absolute, contain no `..` component, and name a location under a root as configured or as
canonicalized; components are compared exactly, except that ASCII case is ignored on Windows.
Another spelling of a root, such as a different symlink or junction to it or a Windows 8.3 short
name, is outside every root. A path outside every root, including a UNC share or device path, is
rejected before any filesystem access. An input must then resolve inside an allowed root, and a Save
As destination inside the root it names. The server bounds workbook/session/response
resources, writes protocol traffic only to stdout, writes diagnostics only to stderr, and never
provides a remote transport. Inputs are opened through an approved-root capability and read from
the same file handle under the configured archive-byte ceiling. Existing destinations are
protected unless the server starts with `--allow-overwrite` and a request also sets
`replace_existing`. Save As retains an open destination-directory capability from validation
through atomic installation, so renaming or replacing the ambient parent path cannot redirect a
write outside the approved root. Resource lists use byte-bounded cursor pagination. Preview pages
likewise return the longest complete interop prefix that fits the response limit and use opaque
cursors bound to the preview and detail section. At session capacity, create/open may evict the
least-recently-used idle session; active sessions are never evicted. TTL expiry and LRU eviction
drop the interop session, which discards its retained preview rather than maintaining a separate
MCP preview cache. Give the server the narrowest practical root; another process with write access
inside that root can still change workbook inputs and contents.

Tool results carry untrusted content. Cell text, sheet names, and defined names come from the
workbook and are returned verbatim, so a crafted workbook can place text that reads as an
instruction into a tool result. That is the same trust boundary as any other document a model
reads: the server does not rewrite workbook content, and the consuming application is responsible
for treating tool output as data rather than as instructions.

To inspect the local server before client integration:

```bash
npx --yes @modelcontextprotocol/inspector@1.0.0 \
  cargo run --locked -p cellrune-mcp -- \
  --root /absolute/path/to/approved/workbooks
```

## Scope

CellRune supports ordinary Transitional SpreadsheetML workbooks and a scoped set of Excel formula
syntax and functions. Unsupported formulas are returned as explicit per-cell calculation issues;
other formulas continue to calculate.

The following are outside the current scope:

- browser, frontend/UI, WebAssembly, hosted-service, and remote-MCP transports;
- `.xls`, `.xlsb`, and `.ods`;
- macro, add-in, external-workbook, query, or data-connection execution;
- 3-D references outside the audited direct-consumer policy above;
- data-table calculation; and
- iterative calculation and automatic host-time inputs.

[`docs/NUMERICS.md`](https://github.com/emulette/cellrune/blob/main/docs/NUMERICS.md)
records where calculated values differ from Excel and why, and documents the two calculation
options added in 0.1.3: `ArithmeticSemantics` and `FinancialSolverSemantics`, which default to
Excel's behavior and can be set to `Ieee754` and `ExtendedSearch` for what 0.1.2 did.

## Verification

Run the standard Rust test suite from the repository root:

```bash
cargo test --workspace --all-features --locked
```

Formula compatibility is regression-tested against committed Excel-saved and Apache POI
workbooks. See the
[`conformance/README.md`](https://github.com/emulette/cellrune/blob/main/conformance/README.md)
reference for the fixture layout, classifications, focused audit command, and maintenance policy.

## License

CellRune is dual-licensed under either the [MIT License](https://github.com/emulette/cellrune/blob/main/LICENSE-MIT)
or the [Apache License, Version 2.0](https://github.com/emulette/cellrune/blob/main/LICENSE-APACHE),
at your option. You need to comply with only one of them, not both. Apache-2.0 includes an
explicit patent grant; MIT does not. Both license texts are included in the source distribution.

Versions 0.1.0 through 0.1.2 were published under the MIT License alone and remain available
under those terms. The dual license applies from version 0.1.3 onward. Dependency license
information is provided in `THIRD_PARTY_LICENSES.md`.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion
in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above,
without any additional terms or conditions.
