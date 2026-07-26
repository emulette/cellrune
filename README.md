# CellRune

CellRune is a Rust library for bounded XLSX/XLSM reading and deterministic formula calculation. It
keeps the workbook read from disk immutable, returns recalculated values in a separate snapshot,
and can retain an exact package backing for explicit round-trip writing.

## Rust installation

The CellRune Rust crate 0.1.1 requires Rust 1.88 or newer.

```bash
cargo add cellrune@0.1.1
```

Or add the dependency directly:

```toml
[dependencies]
cellrune = "0.1.1"
```

## Features

- reads `.xlsx` files from paths, byte slices, or `Read + Seek` streams;
- opens package-backed `.xlsx` and `.xlsm` documents with exact SHA-256 identity and bounded
  round-trip preservation;
- preserves sheet order, sparse cells, formulas, saved results, defined names, and relevant
  number-format metadata;
- expands shared formulas while preserving absolute and relative references;
- reports unsupported formula capabilities before calculation;
- reports normalized per-workbook function demand and exposes the implemented function catalog;
- applies configurable limits to ZIP, XML, workbook, formula, dependency, text, and array work;
- never executes macros, never follows external links, and never reads the host clock for
  `TODAY()` or `NOW()`;
- returns stable error and issue codes for programmatic handling;
- materializes recalculated typed results into existing `.xlsx`/`.xlsm` packages with strict or
  explicit cache-invalidation policies;
- creates canonical `.xlsx` workbooks and applies typed cell, formula, sheet, name,
  number-format, date-system, and calculation-property edits through `WorkbookDraft`;
- reads, queries, preserves, and explicitly authors SpreadsheetML phonetic annotations and default
  frozen panes without mixing presentation state into formula calculation;
- exposes the same versioned read/edit/calculate/write contract through typed Python and
  Node.js/TypeScript native packages;
- supports atomic typed edit batches, persistent parsed/dependency state, safe incremental
  recalculation, bounded result deltas, cooperative cancellation, and stale-result rejection;
- provides a local stdio MCP server with high-level open, inspect, edit, recalculate, range-read,
  delta, and verified Save As tools over the same interop session; and
- raw-copies unchanged package entries without exposing ZIP or XML implementation types.

## Usage

```rust
use cellrune::{
    CalculationOptions, ReadOptions, calculate_workbook, read_xlsx_path, scan_formula_capabilities,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let workbook = read_xlsx_path("input.xlsx", ReadOptions::default())?;
    let capabilities = scan_formula_capabilities(&workbook);

    if capabilities.is_supported() {
        let calculation = calculate_workbook(&workbook, CalculationOptions::default());
        println!("calculated {} formulas", calculation.len());
    }

    Ok(())
}
```

Reading and calculation are separate operations. `calculate_workbook` does not modify the source
`WorkbookSnapshot` or its saved XLSX results. Volatile functions require deterministic inputs
through `CalculationOptions`: `with_today_serial` for `TODAY()` and `with_now_serial` for `NOW()`.
Use `supported_function_catalog` for the build's exact function surface and
`scan_function_usage` to rank the supported and unsupported functions actually used by a workbook.
`INDEX` follows Excel's zero-index reference behavior: a zero row or column selects the complete
corresponding column or row, and zero for both selects the complete input range. Scalar formulas
apply legacy implicit intersection, while array formulas can materialize the selected rectangle.

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
semantics when formula, name, sheet, option, dynamic-reference, or spill topology is uncertain.
Forced incremental mode fails closed instead of guessing. Sessions use optimistic semantic
revisions for atomic edits, retain bounded result-delta history, and reject stale calculations.
A batch whose accepted operations make no semantic change keeps the current revision, topology,
and installed calculation instead of forcing a redundant recalculation.
Long-running work can be prepared outside the session lock with `prepare_recalculation`, cancelled
through a request-owned `CancellationToken`, and installed only if its source revision is still
current.

`open_xlsx_document_*` retains the exact input package for writing.
`write_recalculated_xlsx_bytes`, `write_recalculated_xlsx`, and
`write_recalculated_xlsx_path` bind a calculation to that exact input, update typed formula
caches, remove stale calculation chains, preserve unrelated package content, and reopen the output
before reporting success. Strict mode rejects incomplete calculations without producing an
artifact; cache invalidation is an explicit opt-in policy. `write_preserved_xlsx_bytes` remains
available for an unchanged preservation copy.

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

The 0.1.1 release line targets Python 3.10 through 3.14 and Node.js 22 or newer. Install the
bindings with:

```bash
python -m pip install "cellrune==0.1.1"
npm install "@cellrune/node@0.1.1"
```

The bindings expose the same versioned read, edit, calculate, and write contract. Native package
availability remains platform-specific; package managers must select a wheel or exact-version npm
platform package compatible with the current runtime.

Python workbooks are context managers:

```python
from cellrune import Workbook

with Workbook.create() as workbook:
    workbook.set_number("Sheet1", "A1", 41.0)
    workbook.set_formula("Sheet1", "B1", "=A1+1")
    workbook.calculate()
    workbook.save("output.xlsx")
```

In a Node.js ES module, close the workbook in `finally`:

```js
import { Workbook } from "@cellrune/node";

const workbook = Workbook.create();
try {
  workbook.setNumber("Sheet1", "A1", 41);
  workbook.setFormula("Sheet1", "B1", "=A1+1");
  await workbook.calculate();
  await workbook.save("output.xlsx");
} finally {
  workbook.close();
}
```

Python and Node.js `close()` calls are idempotent. Once `close()` returns, the binding-owned native
session has been released. An active calculation is cooperatively cancelled, and subsequent
operations fail with the stable `interop.session.closed` code.

## Local MCP

`cellrune-mcp` is a local stdio-only MCP `2025-11-25` server for AI hosts. It exposes a finite set
of high-level workbook workflow tools; spreadsheet functions remain formulas inside the workbook
and are not registered one by one as MCP tools. Start it with one or more explicit filesystem
roots:

```bash
cargo run --locked -p cellrune-mcp -- \
  --root /absolute/path/to/approved/workbooks
```

Its 11 tools are `workbook_create`, `workbook_open`, `workbook_close`, `workbook_summary`,
`workbook_read_range`, `workbook_function_usage`, `workbook_scan_capabilities`,
`workbook_apply_changes`, `workbook_recalculate`, `workbook_changes_since`, and
`workbook_save_as`.

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
be absolute and resolve inside one of those roots. The server bounds workbook/session/response
resources, writes protocol traffic only to stdout, writes diagnostics only to stderr, and never
provides a remote transport. Inputs are opened through an approved-root capability and read from
the same file handle under the configured archive-byte ceiling. Existing destinations are
protected unless the server starts with `--allow-overwrite` and a request also sets
`replace_existing`. Save As retains an open destination-directory capability from validation
through atomic installation, so renaming or replacing the ambient parent path cannot redirect a
write outside the approved root. Resource lists use byte-bounded cursor pagination. At session
capacity, create/open may evict the least-recently-used idle session; active sessions are never
evicted. Give the server the narrowest practical root; another process with write access inside
that root can still change workbook inputs and contents.

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
syntax and functions. Use `scan_formula_capabilities` to check a workbook before relying on
calculated values.

The following are outside the current scope:

- `.xls`, `.xlsb`, `.ods`, and CSV;
- macro, add-in, external-workbook, query, or data-connection execution;
- table structured references and 3-D references;
- spill postfix references such as `A1#`, general `LAMBDA`, and data-table calculation; and
- iterative calculation and automatic host-time inputs.

CellRune does not claim complete Excel compatibility.
[`docs/NUMERICS.md`](https://github.com/emulette/cellrune/blob/main/docs/NUMERICS.md)
records where calculated values are known to differ from Excel and why, including the iterative
financial solvers, `DOLLAR` currency formatting, and IEEE-754 arithmetic near zero. Read it before
comparing results for equality.

## Verification

The `conformance/` tree carries redistributable expectation matrices — every literal, every
formula, and the value a recorded oracle saved for that formula — and the normal workspace test
run reconstructs each matrix, calculates it, and compares CellRune's values with the oracle's.
The first matrix is the Apache POI `FormulaEvalTestData` corpus (Apache-2.0): 1,295
formula cases against the workbook's saved Microsoft Excel 2013 calculation cache, of which
1,290 match within a scale-relative `1e-8` and 5 are divergences documented case by case where
that 2013-era cache predates current Excel behavior. Documented divergences are enforced in
both directions: the test fails if CellRune stops matching where it must, and also if a
recorded divergence quietly disappears or changes shape.

Development audits recorded against the private corpus (2026-07-24, not distributed) add: a
Microsoft Excel for Mac 16.111 recalculation of the same corpus matching CellRune on all 1,287
non-locale results within `1e-8`, with the eight `DOLLAR` locale strings documented in
`docs/NUMERICS.md`; full-span audits of 14 real workbooks — 331,322 calculation entries with
748 unavailable results: 695 blocked by unavailable upstream cells, 44 circular references,
and 9 formula containers without stored text, none caused by an unsupported function or
expression; the largest of them, roughly 250,000 calculated formulas, completing its
full preserve, recalculate, and reopen audit in 5 minutes 22 seconds; and producer fixtures
from Excel, LibreOffice, Google Sheets, and openpyxl passing 4 of 4.

Public CI uses generated, redistributable workbook fixtures and the expectation matrices above.
Private development corpora and native-producer evidence are not distributed or represented as
release blockers; validate workbooks from your own producers against the supported-capability
report before relying on them.

## License

CellRune is available under the MIT License. Dependency license information is provided in
`THIRD_PARTY_LICENSES.md`.
