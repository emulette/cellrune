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
npm install @cellrune/node@0.1.6
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

`Workbook` supports typed errors, revision-checked edit batches, incremental
calculation deltas, deterministic `todaySerial` and `nowSerial` inputs, and explicit
`arithmeticSemantics` / `financialSolverSemantics` compatibility policies. The latter accept
`"ieee_754"` and `"extended_search"` when a caller needs the calculation behavior shipped through
0.1.2; omitted fields select the Excel-compatible defaults.
`close()` is idempotent. Once it returns, the binding-owned native session has
been released. An active calculation is cooperatively cancelled, and later
operations fail with `interop.session.closed`.
See the declarations bundled with the package for the complete API.

CellRune is dual-licensed under either the MIT License or the Apache License,
Version 2.0, at your option. Both texts ship with this package as `LICENSE-MIT`
and `LICENSE-APACHE`. The native package's dependency notices are in
`THIRD_PARTY_LICENSES.md`.
