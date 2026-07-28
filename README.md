# CellRune

CellRune is a Rust library for bounded XLSX/XLSM reading and deterministic formula calculation. It
keeps the workbook read from disk immutable, returns recalculated values in a separate snapshot,
and can retain an exact package backing for explicit round-trip writing.

## Rust installation

The CellRune Rust crate 0.1.5 requires Rust 1.88 or newer.

```bash
cargo add cellrune@0.1.5
```

Or add the dependency directly:

```toml
[dependencies]
cellrune = "0.1.5"
```

## Features

- reads `.xlsx` files from paths, byte slices, or `Read + Seek` streams;
- opens package-backed `.xlsx` and `.xlsm` documents with exact SHA-256 identity and bounded
  round-trip preservation;
- preserves sheet order, sparse cells, formulas, saved results, defined names, and relevant
  number-format metadata;
- expands shared formulas while preserving absolute and relative references;
- returns typed formula values and stable per-cell calculation issues in one result snapshot;
- reports normalized per-workbook function demand and exposes the implemented function catalog;
- evaluates audited 3-D aggregate references across workbook tab order, including hidden sheets and
  reverse-written endpoints, without expanding the sheet span into dependency edges;
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
`with_today_serial` for `TODAY()` and `with_now_serial` for `NOW()`. Use
`with_arithmetic_semantics` and `with_financial_solver_semantics` to opt into the raw IEEE-754 and
extended-search behavior shipped through 0.1.2; the defaults select Excel-compatible cancellation
and Microsoft's function-specific solver budgets. Use
`supported_function_catalog` for the build's exact function surface and `scan_function_usage` to
summarize the functions used by a workbook. `scan_formula_capabilities` remains available as an
optional static inventory for migration planning and user-interface reporting; calculation does
not require it.
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

The 0.1.5 release line targets Python 3.10 through 3.14 and Node.js 22 or newer. Install the
bindings with:

```bash
python -m pip install "cellrune==0.1.5"
npm install "@cellrune/node@0.1.5"
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
    # 0.1.2-compatible calculation remains available when required:
    workbook.calculate(
        arithmetic_semantics="ieee_754",
        financial_solver_semantics="extended_search",
    )
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
  await workbook.calculate({
    arithmeticSemantics: "ieee_754",
    financialSolverSemantics: "extended_search",
  });
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
syntax and functions. Unsupported formulas are returned as explicit per-cell calculation issues;
other formulas continue to calculate.

The following are outside the current scope:

- `.xls`, `.xlsb`, `.ods`, and CSV;
- macro, add-in, external-workbook, query, or data-connection execution;
- table structured references and 3-D references outside the audited direct-consumer policy above;
- spill postfix references such as `A1#`, general `LAMBDA`, and data-table calculation; and
- iterative calculation and automatic host-time inputs.

[`docs/NUMERICS.md`](https://github.com/emulette/cellrune/blob/main/docs/NUMERICS.md)
records where calculated values differ from Excel and why, and documents the two calculation
options added in 0.1.3: `ArithmeticSemantics` and `FinancialSolverSemantics`, which default to
Excel's behavior and can be set to `Ieee754` and `ExtendedSearch` for what 0.1.2 did.

## Verification

The `conformance/` tree carries the binary workbooks Excel actually calculated, together with
their hashes, host metadata, and reviewed expectations. They are audited explicitly during local
development:

```bash
cargo run \
  --package cellrune-integration-tests \
  --bin check_excel_oracle \
  --locked
```

This audit is deliberately separate from `cargo test`, CI, and publication. It covers 1,295
formula cells from Apache POI's `FormulaEvalTestData`, 266 materialized array results from its
matrix fixture, and 672 selected results from the CellRune-authored formula oracle. Every selected
case is classified; missing or extra classifications fail locally. The older POI formula cache
currently has 1,290 matches and 5 documented divergences, enforced in both directions.

Two additional corpus tests are compiled but marked `#[ignore]` because their third-party inputs
are not distributed in this repository. A developer who has supplied those inputs can run them
explicitly:

```bash
WORKBOOK_FORMULA_CORPUS=/path/to/formulas.xlsx \
  cargo test -p cellrune-integration-tests --test external_formula_corpus -- --ignored

CELLRUNE_WORKBOOK_CORPUS=/path/to/workbook-or-directory \
  cargo test -p cellrune-integration-tests --test external_workbook_corpus -- --ignored
```

`#[ignore]` keeps a test registered and compilable while excluding it from ordinary `cargo test`;
cloning the repository does not provide either external corpus.

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
