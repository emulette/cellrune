# `@cellrune/node`

Node.js bindings for CellRune, a headless XLSX/XLSM reader, deterministic
calculation engine, editor, and writer.

## Requirements

- Node.js 22 or newer
- A supported macOS, Linux, or Windows platform

The package installs the matching native package through an optional
dependency. You do not need to select a platform package yourself.

## Install

```console
npm install @cellrune/node@0.1.20
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
    await workbook.save("output.xlsx");
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
calculation deltas, deterministic `todaySerial` and `nowSerial` inputs, and explicit
`arithmeticSemantics` / `financialSolverSemantics` compatibility policies. The latter accept
`"ieee_754"` and `"extended_search"` when a caller needs the calculation behavior shipped through
0.1.2; omitted fields select the Excel-compatible defaults.
The 0.1.14 calculation surface adds `CONVERT`, the four `BESSEL*` functions, and fourteen
`COMPLEX`/`IM*` functions. They return scalar numbers or Excel-compatible complex text and expose
the same catalog and calculation contract as Rust and Python.
`inspectDefinedName` returns typed static, dynamic, empty, external, invalid, and unsupported
defined-name results. `applyChangesV2` retains every v1 edit shape and adds stable-ID table rename,
table-column rename, and table-row resize operations with `changedTableIds` in its receipt.
`save()` returns a `WriteReport` whose `outputSha256` is the SHA-256 of the exact verified output
archive bytes. It is an output identity, not the input document hash.
`close()` is idempotent. Once it returns, the binding-owned native session has
been released. An active calculation is cooperatively cancelled, a published preview is discarded,
and later operations fail with `interop.session.closed`.
See the declarations bundled with the package for the complete API.

CellRune is dual-licensed under either the MIT License or the Apache License,
Version 2.0, at your option. Both texts ship with this package as `LICENSE-MIT`
and `LICENSE-APACHE`. The native package's dependency notices are in
`THIRD_PARTY_LICENSES.md`.
