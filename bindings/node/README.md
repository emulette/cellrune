# `@cellrune/node`

Node.js bindings for CellRune, a headless XLSX/XLSM reader, deterministic
calculation engine, editor, and writer.

## Requirements

- Node.js 22 or newer
- A supported macOS, Linux, or Windows platform

The package installs the matching native package through an optional
dependency. You do not need to select a platform package yourself.

## Function usage

`workbook.functionUsage()` returns static function demand with `schemaVersion: 2`.
Each entry's `callCount` is a `bigint`. Values below `18446744073709551615n` are
exact; that maximum means at least that many calls. Use bigint arithmetic and
`entry.callCount.toString()` when writing a count to JSON. This replaces the
previous `number` field so large defined-name graphs do not lose precision.

## Install

```console
npm install @cellrune/node@0.1.26
```

## CommonJS

```js
const { Workbook } = require("@cellrune/node");

async function main() {
  const workbook = Workbook.create();
  try {
    workbook.setNumber("Sheet1", "A1", 41);
    workbook.setFormula("Sheet1", "B1", "=A1+1");
    await workbook.calculate();
    await workbook.save("output.xlsx", { replaceExisting: true });
  } finally {
    workbook.close();
  }
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
```

## ES modules

```js
import { Workbook } from "@cellrune/node";

const workbook = await Workbook.openPath("input.xlsx");
try {
  const page = workbook.readRange("Sheet1", "A1", "D20", { limit: 80 });
  console.log(page.cells);
} finally {
  workbook.close();
}
```

## Verified change preview

`previewChanges` captures a revision-checked v2 edit candidate and calculates
it without changing the live workbook. It is the only preview call that returns
a `Promise`; page, commit, and discard are synchronous.

```ts
const revision = workbook.summary().semanticRevision;
const preview = await workbook.previewChanges(revision, [
  { kind: "setValue", sheet: "Sheet1", address: "A1", value: { kind: "number", value: 42 } },
]);

const page = workbook.previewChangesPage(preview.previewId, {
  section: "preview_results",
  limit: 100,
});

const receipt = workbook.commitPreview(preview.previewId);
// Or, without changing the workbook: workbook.discardPreview(preview.previewId);
```

Preview IDs and revision/cursor fields are `bigint`; preview DTO fields use
camel case. A session retains no more than one active preview calculation and
one published preview. A failed, cancelled, stale, or oversized replacement
leaves a prior published preview available. Pre-commit cancellation or a
resource error is retryable; a stale or successful commit consumes the preview.
Pass an opaque `PreviewCursor` from `previewChangesPage` back unchanged for the
same preview and section. The complete shared lifecycle and page contract is in
[`llms.txt`](https://github.com/emulette/cellrune/blob/main/llms.txt).

Calculate a few outputs before any full calculation with:

```javascript
const result = await workbook.calculateTargets([
  { sheet: "Sheet1", start: "B1" },
  { sheet: "Sheet1", start: "D1", end: "D10" },
], { limits: { maxResultCells: 100, maxEvaluatedCells: 10000 } });
```

`TargetCalculationResult` has `scope: "targets"`, a `bigint` `semanticRevision`, source identity,
typed `cells`, deterministic options, limits, and `evaluatedCount`, `parsedFormulaCount`, and
`reusedCount`. It includes only requested cells in sheet-ID and row-major order. Calculation
parses/evaluates required precedents, deduplicates overlap, and can reuse a compatible current
complete cache. It does not install a full cache, clear dirty formulas, advance deltas, invalidate
a preview, or satisfy the full-calculation requirement for saving. Defaults allow 1024 targets,
10000 returned cells, and 100000 evaluator invocations, including dynamic retries. Known array
followers evaluate the whole anchor; include the anchor when requesting an undeclared spill
whose owner has not yet been calculated. Saved formula caches are never treated as current.
Use declared spill ranges or full calculation if independent undeclared spills may overlap;
anchors outside the dependency scope are not discovered by a partial request.

`Workbook` supports typed errors, revision-checked edit batches, incremental
calculation deltas, deterministic `todaySerial` and `nowSerial` inputs, a `randomSeed` for
`RAND`, `RANDBETWEEN`, and `RANDARRAY`, a `localeProfile` (`{ locale: "ko-KR" }` writes `DOLLAR`
in won and counts Hangul as two bytes in `LENB`), and explicit
`arithmeticSemantics` / `financialSolverSemantics` compatibility policies. The latter accept
`"ieee_754"` and `"extended_search"` when a caller needs the calculation behavior shipped through
0.1.2; omitted fields select the Excel-compatible defaults.
`formatDisplayText(value, formatCode, { dateSystem, localeProfile })` returns the text a cell
shows for a `{ kind, value }` cell value and an Excel number-format code.
The 0.1.14 calculation surface adds `CONVERT`, the four `BESSEL*` functions, and fourteen
`COMPLEX`/`IM*` functions. They return scalar numbers or Excel-compatible complex text and expose
the same catalog and calculation contract as Rust and Python.
`inspectDefinedName` returns typed static, dynamic, empty, external, invalid, and unsupported
defined-name results. `applyChangesV2` retains every v1 edit shape and adds stable-ID table rename,
table-column rename, and table-row resize operations with `changedTableIds` in its receipt.
`save()` returns a `WriteReport` whose `outputSha256` is the SHA-256 of the exact verified output
archive bytes. It is an output identity, not the input document hash.
`saveEdits()` and `editsToBytes()` write only the edits of a workbook opened from a file, without
calculating: formulas the edits can affect lose their saved results and the file asks the
spreadsheet app to recalculate them, and formulas CellRune cannot calculate do not block the save.
`save()`, `toBytes()`, `saveEdits()`, and `editsToBytes()` read the workbook when their worker
runs, so do not edit it until the returned Promise settles.
`openPath`/`fromBytes` accept `readLimits` and `calculate`/`recalculate`/`previewChanges` accept
`calculationLimits` to change individual default limits, and `scanCapabilities()` pages through the
static per-formula support report. `dataValidations(sheet, { offset, limit })` pages through a
sheet's read-only data validation rules, and `hyperlinks` and `comments` through its hyperlinks and
cell comments. `readFormats(sheet, start, end, { offset, limit })` reads
cell fonts, fills, borders, and alignments with column widths and row heights, and
`applyChangesV2` takes `setCellFormat`, `setColumnWidth`, and `setRowHeight` changes; a
`setCellFormat` font, border, or alignment sets only the properties it gives, and its fill replaces
the whole fill.
It also takes the
`widenTableColumns` and `widenSheetAutoFilter` widening edits, and the presentation-only
`setFrozenPane`, `setPhonetics`, and `clearPhonetics` changes. Summaries report a sheet's
`frozenPane` when it has one, and `phonetics(sheet, { offset, limit })` pages through its phonetic
runs. A limit failure carries its figures in `error.details.limit`, and an unavailable result that
exceeded a calculation limit names it in `limit`.
`readRange(sheet, start, end, { omitEmpty, valuesOnly, compact, maxBytes })` returns a page without
empty cells, as address and value pairs, as compact rows, or as the longest prefix within a byte
limit. `readValues(sheet, start, end)` reads a rectangle as rows of typed values, and
`writeValues(sheet, start, rows)` writes equal-length rows of values as one atomic edit.
`Workbook.fromCsv(bytes, { sheetName, delimiter })` opens RFC 4180 CSV as a one-sheet workbook, and
`toCsv(sheet, { delimiter })` exports one calculated sheet; the value rule is in the repository
README.
`precedents(sheet, address, { offset, limit })` and `dependents(...)` page through what a cell's
formula reads and the formulas that read the cell, from the current calculation's graph.
`calculate`, `recalculate`, `calculateTargets`, and `previewChanges` accept an `AbortSignal` as
`signal`; aborting it cancels the calculation cooperatively and rejects with `session.cancelled`
(`calculation.target.cancelled` for `calculateTargets`).
`close()` is idempotent. Once it returns, the binding-owned native session has
been released. An active calculation is cooperatively cancelled, a published preview is discarded,
and later operations fail with `interop.session.closed`.
See the declarations bundled with the package for the complete API.

CellRune is dual-licensed under either the MIT License or the Apache License,
Version 2.0, at your option. Both texts ship with this package as `LICENSE-MIT`
and `LICENSE-APACHE`. The native package's dependency notices are in
`THIRD_PARTY_LICENSES.md`.
