# Changelog

All notable changes to CellRune are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Releases follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

This file records concise user-visible and release-operator changes. Design rationale, test
inventories, and measurements belong in the linked documentation rather than in release entries.

## [0.1.5] - 2026-07-27

### Added

- Direct 3-D references for the audited aggregate family: `SUM`, `AVERAGE`, `AVERAGEA`, `COUNT`,
  `COUNTA`, `MAX`, `MAXA`, `MIN`, `MINA`, `PRODUCT`, `STDEV.P`, `STDEV.S`, `VAR.P`, and `VAR.S`,
  including the catalogued legacy aliases. Inclusive workbook tab order determines the span, so
  hidden sheets participate and reverse-written endpoints are equivalent.
- Integration coverage for the complete supported-function catalog, hidden sheets, reverse spans,
  whole-column 3-D inputs, and a fuzz invariant that compares 3-D `SUM` with an explicit sheet
  fold under the exact array-cell budget.

### Changed

- The capability scanner and evaluator now share one explicit direct-3-D-consumer policy.
  `INDEX` and `VLOOKUP` return Excel's `#VALUE!`, `OFFSET` returns `#REF!`, and other consumers
  remain an `UnsupportedSheetRange` engine capability issue.
- Incremental calculation stores 3-D dependencies as compact rectangle spans and counts each span
  once against the dependency-edge budget instead of expanding it into one edge per cell.
- Whole-column operands combined by unary or binary array operators use a common extent: the
  greatest populated row among the columns those operands reference, with a one-row minimum for an
  otherwise empty sheet and blanks for unpopulated cells. Directly resolved sources and operator
  outputs share one cumulative array-cell budget. Function arguments retain independent bounded
  contexts, and a returned array joins an enclosing cumulative context only when another direct
  whole-column operand established it.
- Four existing Excel-saved 3-D cases and two whole-column-array cases were activated by changing
  only their reviewed classifications; the oracle workbook and saved Excel results were not
  regenerated.

### Fixed

- Edits to the first, middle, or last sheet of a 3-D dependency now dirty the dependent formula,
  while edits outside the span do not.
- Reference-returning functions such as multi-cell `OFFSET` remain usable as aggregate inputs
  after the compact sheet-span representation is introduced.
- A whole-column array expression no longer takes its height from the sheet-wide used range. That
  made the value depend on cells outside every dependency rectangle, so writing into a column the
  expression never named changed the answer a full recalculation produced while leaving the
  incremental pass holding the previous one. `COUNT` and `AVERAGE` over whole-column products were
  affected; `SUM` was not, because the extra rows fold to zero.

## [0.1.4] - 2026-07-27

### Added

- Committed Excel-saved binary oracle workbooks and an explicit local audit command replace the
  generated JSON conformance matrix. The audit is not part of CI or publication.
- `CalculationHints` retains the optional XLSX iterative-calculation declaration across read and
  write operations, and it is carried by the interop DTO and the Python, Node.js, and MCP
  boundaries alongside the calculation mode and the host recalculation flags. `CalculationHints`
  describes the whole `calcPr` declaration, so applying one replaces the workbook's declaration
  rather than merging into it: use `with_iterative_calculation` to carry the flag through an
  edit that would otherwise clear it. CellRune still does not perform iterative calculation; the
  declaration is preserved, not obeyed.

### Changed

- `ExcelNearZero` now matches Excel's observed narrow correction boundary:
  `=0.1+0.2-0.3` becomes zero, while `=100.1-100-0.1` keeps its residue.
- CI runs the normal Rust regression suite; Excel oracles, external corpora, engine comparisons,
  and accuracy metrics remain explicit local development checks.
- Release validation and publication dependencies are separated by core, Python, Node.js, and MCP
  artifact family.

### Fixed

- Formula analysis now handles nested calls, defined names, and lambda shadowing consistently,
  preventing stale incremental results and misclassified volatile-input failures.

## [0.1.3] - 2026-07-27

### Added

- `CellPhonetics::resolved_runs` returns each phonetic run's range as byte offsets into the cell's
  base text, translated from the UTF-16 code units XLSX stores, as `ResolvedPhoneticRun`.
- `DocumentPresentation::phonetic_cell_entries` iterates a sheet's cells that carry at least one
  run, in address order. Cells holding only `phoneticPr` display properties without `rPh` runs are
  skipped and remain reachable through `cell_phonetics`.
- `ArithmeticSemantics` on `CalculationOptions` selects how arithmetic that cancels to near zero is
  handled. The default `ExcelNearZero` returns `0` for `=0.1+0.2-0.3`, in the operator path, the
  array path, and `SUM`, `AVERAGE`, `SUMIF(S)`, `AVERAGEIF(S)`, `SUBTOTAL`, `SUMPRODUCT` and `NPV`.
  `Ieee754` keeps the residue, as 0.1.2 did.
- `FinancialSolverSemantics` on `CalculationOptions` selects the solver budget. The default
  `ExcelIterationBudget` applies Microsoft's documented policy — 20 iterations/`1e-7` for `IRR` and
  `RATE`, 100/`1e-8` for `XIRR`. `ExtendedSearch` searches longer, as 0.1.2 did.
- Both options are carried by the interop DTO and exposed by the Python, Node.js, and MCP
  boundaries. `docs/NUMERICS.md` records the policies.

### Changed

- **Calculated numbers change by default.** Arithmetic that cancels to near zero now returns zero,
  and the financial solvers now give up where Excel gives up instead of searching four times
  longer. Callers who depended on 0.1.2's numbers should select `ArithmeticSemantics::Ieee754` and
  `FinancialSolverSemantics::ExtendedSearch`; callers comparing against Excel need no change.
- CellRune is dual-licensed under `MIT OR Apache-2.0` instead of MIT alone. Releases 0.1.0 through
  0.1.2 remain under MIT; the dual license applies from 0.1.3 onward. Every distributed artifact
  now carries `LICENSE-MIT` and `LICENSE-APACHE` in place of the single `LICENSE` file.
- The packaged-license CI gate enumerates its copies from the distribution directories rather than
  naming each file. The previous list covered three of the eleven copies, so the eight npm platform
  packages carried license text no gate compared against the original.

## [0.1.2] - 2026-07-26

### Added

- A single data-driven conformance expectation suite. `conformance/` holds redistributable
  matrices — every literal, every formula, and the value a recorded oracle saved for it — and
  the normal workspace test run reconstructs every matrix and compares CellRune's calculation
  against the oracle. New functions and syntax extend the same data set.
  The first matrix is the Apache POI `FormulaEvalTestData` corpus (Apache-2.0): 1,295 cases
  against a saved Microsoft Excel 2013 calculation cache, 1,290 matching within a
  scale-relative `1e-8` and 5 divergences documented case by case. A documented divergence is
  enforced in both directions, so neither the oracle side nor the CellRune side of it can drift
  silently, and a divergence without an explanatory note fails the test. An extraction tool
  (`extract_conformance_matrix`) accepts ordinary and resolved shared formulas and rejects
  workbook metadata the v1 matrix cannot reproduce.
- A semver invariant inside the existing test suite (`tests/public_api.rs`). The sixteen exported
  enums that are not `non_exhaustive` are matched without wildcard arms with every variant
  payload bound at its exact type, and function-pointer constants pin the positional signatures
  of `WorkbookSnapshot::new_with_metadata` and the monomorphic read → calculate → write entry
  points. This remains one test as the API grows.

### Changed

- Rust dependency requirements now declare the oldest API-compatible, advisory-clean versions
  instead of using the release that happened to be newest when each manifest was edited. All
  workspace requirements live in the root manifest, the lockfile still selects the reproducible
  current graph, and every release family a broad range admits is exercised by the floor or the
  newest-compatible suite. Fifteen of the twenty external workspace floors moved down; five
  remain at their current releases because they are the first secure version or belong to the
  same release family (`quick-xml`, PyO3 and its build helper, and `calamine`), or because their
  types appear in CellRune's public signatures and a public dependency must stay within one
  semver family (`cap-std`). The fuzz crate's `libfuzzer-sys` stays exact-pinned as a tool
  input: its graph is excluded from the workspace and always built `--locked`, so no suite
  could exercise a range there.
- The `conformance/` fixtures now live in `binding-contract/`, which is what they are: a
  cross-binding API behavior contract exercised by the Rust interop, Python, and Node test
  suites. Two 1 KB fixtures cannot carry an Excel conformance claim, so the `conformance/` name
  is reserved for the Excel expectation suite that can.
- The nightly sanitizer campaign remains one job over four stable trust boundaries. It records
  every target's result before failing, so one crash cannot starve later campaigns and new
  functions do not create new fuzz jobs.
- The MSRV check covers `--all-targets` rather than `--lib`, so tests, benches, and the
  extraction tool must also build on the declared floor.
- The 50k benchmark is demoted from a scheduled nightly job and a required release gate to an
  explicit observational command, and its output-verifier script is removed with the jobs. The
  benchmark keeps its internal correctness assertions, and functional correctness remains
  covered by the generated-workbook tests.
- Release publish jobs are re-run safe. The crates.io, npm, and GitHub-release jobs probe for
  what is already live and skip it — the GitHub release also counts its attached bundles, so an
  interrupted asset upload is completed rather than skipped — and the PyPI upload passes
  `skip-existing`. A rerun of a partially published tag therefore converges on fully published
  instead of ending red: the final runs of both prior releases stayed red because their
  already-published registries could only fail again or be rejected by hand.

### Fixed

- A workbook containing an empty sheet was rejected with `xlsx.invalid_worksheet`. Excel and
  every mainstream producer serialize an empty sheet as a self-closing `<sheetData/>`, which
  the reader's required-sheetData rule did not recognize as the element's empty form, so even a
  brand-new workbook saved by Excel failed to open. Empty sheets now read as sheets with no
  cells; a duplicated `<sheetData/>` is still rejected.

### Documentation

- The README carries a Verification section: the public conformance-suite numbers with their
  exact oracle, and the private-audit results (Excel for Mac 16.111 corpus recalculation,
  14-workbook full-span audits, the 250k-formula timing, and the 4-of-4 producer matrix) with
  their provenance and non-distribution stated.

## [0.1.1] - 2026-07-25

### Changed

- Linux wheels target a `manylinux_2_28` (glibc 2.28) baseline instead of `manylinux_2_35`. The
  previous tag came from whichever glibc the build runner happened to ship, and it excluded RHEL 9
  and Amazon Linux 2023 by 0.01 of a glibc version; both fell back to the source distribution,
  which requires a Rust toolchain the installing user rarely has. The baseline is now an explicit
  build input, produced with the same pinned Zig the npm platform packages already use, and the
  resulting platform tag is asserted during release verification rather than assumed.
- Dependency requirements of the published `cellrune` crate are caret ranges instead of `=` pins.
  An exact pin made the crate unresolvable alongside any dependent needing a newer patch of the
  same dependency, with no remedy available to that dependent. Build reproducibility continues to
  come from the committed `Cargo.lock`. Two new scheduled gates bound the ranges: one runs the test
  suite against the declared floors on the minimum supported Rust version, and one runs it against
  the newest compatible graph. Both resolve live rather than from the lockfile, so they are
  scheduled alongside the advisory tier rather than gating pull requests: a failure there means
  upstream moved, not that the commit under test did.

### Fixed

- Quoted external-workbook references such as `'[1]Sheet1'!A1` were reported as supported and
  evaluated to `#REF!`. Because that is a spreadsheet error value rather than an engine issue,
  `IFERROR` could hide it, so a workbook could return a plausible number for a formula the engine
  cannot evaluate. Both the capability scan and evaluation now reject the external prefix. The
  unquoted spelling was already rejected, as the formula lexer has no bracket token. A reference
  built at run time by `INDIRECT` is the deliberate exception: the scan cannot read inside the
  text argument, so it reports the cell as supported, and evaluation answers `#REF!` there as
  Excel does rather than an engine issue the scan never predicted and no caller could recover
  from. An external prefix written as a saved path keeps its drive letter and is reported once,
  not also as a 3-D sheet range the formula does not contain.
- Calculated zeros could be negative. `Iterator::sum` for `f64` folds from `-0.0`, `f64::min` and
  `f64::max` may return either operand when both compare equal, and `Iterator::product` inherits
  the sign of an odd number of negative factors, so `SUM`, `MIN`, `MAX`, `PRODUCT`, and the `*A`
  variants could each report `-0` where Excel reports `0`. The boundary every calculated value
  crosses now normalizes negative zero, rather than each kernel being expected to.
- Formula parse-error details labelled every position as `token N`, but a lexing failure has no
  token stream and was reporting a character offset under that label. Lexing failures now report
  `character N` and parsing failures continue to report `token N`.
- The lambda function group dispatched without inspecting the normalized function name, unlike
  every sibling group. A second function in that group would have silently evaluated as `MAP`.
- A release version bump had to be repeated by hand in more places than the release checklist
  listed, and the ones it missed — a pnpm lockfile, a generated loader shim, the packaged-consumer
  lockfile, and the release verification scripts — fail only during a release run. Every version
  is now derived from the workspace manifest where possible, and a new gate asserts that the
  remaining declarations agree, including the install commands and the supported-version statement
  that ship on the registry project pages. Every check in that gate fails when its pattern matches
  nothing, so a moved file or a regenerated shim cannot report the same green result as a correct
  bump, and the gate has its own tests.
- A Linux wheel could declare a compressed platform-tag set whose first component was a legacy
  alias, which hid a newer glibc requirement behind it from the release verification. Every
  component of the set is now checked against the supported baseline.
- The nightly fuzz tier could not install its fuzzer. `taiki-e/install-action` has no cargo-fuzz
  manifest, the job deliberately disables the binstall fallback, and the schedule had never fired
  since the repository went public, so the defect surfaced only on the tier's first manual
  dispatch. cargo-fuzz is now built from crates.io with `cargo install --locked`, which keeps the
  exact-version pin and the registry checksum verification.
- The npm publish job could not authenticate. Trusted publishing requires npm 11.5.1 to exchange
  the job's OIDC identity for a registry credential, and Node 22 bundles npm 10, which publishes
  without credentials and receives the rejection masked as E404. The nine-package dry-run gate
  cannot catch this because a dry run never authenticates, so the failure surfaced on the first
  real upload — before any package was published, leaving the retry clean. The job now upgrades
  to a pinned npm 11 before publishing.

### Documentation

- The 0.1.0 entry claimed capability detection for unsupported external, structured, and
  spill-postfix formula forms. Only 3-D and data-table forms have dedicated detection; the other
  three surface as `calculation.parse_error`, and structured references and spill-postfix
  references still do after this release. The entry has been corrected.
- Added `docs/NUMERICS.md`, which records where calculated values deliberately differ from Excel,
  the Excel build each statement was measured against, and which function families are unmeasured.
  It is linked absolutely: both published READMEs are also the long description crates.io, PyPI,
  and npm render, none of which resolve a repository-relative path, and a gate now enforces that.
- Installation instructions no longer describe the registry artifacts as pending.

## [0.1.0] - 2026-07-25

### Added

- Bounded `.xlsx` package inspection and reading from paths, byte slices, and `Read + Seek`
  streams, with ZIP, XML, workbook, formula, text, relationship, and allocation limits.
- Immutable sparse workbook snapshots with typed values, formulas, saved results, shared and array
  metadata, defined names, sheet visibility, date systems, number formats, diagnostics,
  and provenance.
- Package-backed `.xlsx` and `.xlsm` documents with exact SHA-256 source identity that preserve
  unknown and unchanged parts without executing macros, following external links, or reading
  host-time inputs.
- Static formula capability scans and deterministic recalculation into a separate owned result
  snapshot, including direct formula cells, legacy-array regions, and dynamic-spill regions.
- A catalog of 278 official Excel-facing function names, comprising 265 official calculation
  kernels and 13 compatibility aliases, plus one non-official OOXML dummy-function marker,
  workbook function-demand reports, and stable sample locations.
- Explicit deterministic inputs for `TODAY()` and `NOW()`, bounded calculation work, stable
  per-formula issue codes, and spreadsheet error values kept distinct from unsupported engine
  capabilities.
- Formula parsing and evaluation for scalar, range, lookup, logical, aggregate, math,
  trigonometric, combinatoric, engineering, information, text, date/time, dynamic-array,
  statistical, and financial function groups.
- Excel-compatible `INDEX` zero row/column references, including scalar implicit intersection,
  reference composition, incremental dependencies, and row, column, or full-rectangle array
  materialization.
- Capability detection for unsupported 3-D and data-table formula forms.
- Verified recalculation writing that binds results to the exact source, updates typed caches,
  removes stale calculation chains, preserves unrelated package content, and supports strict or
  explicit cache-invalidation policy.
- Canonical workbook creation with dynamic-formula authoring, plus preservation-aware
  `WorkbookDraft` editing for typed cells, normal formulas, sheets, defined names, number formats,
  date systems, and calculation properties. Existing document-backed dynamic formulas can be
  recalculated and cache-rewritten, while adding or replacing them fails closed.
- Atomic typed edit batches with semantic revisions, complete rollback on validation failure,
  no-op revision and calculation preservation, and Save As path writes that refuse implicit
  destination replacement.
- SpreadsheetML phonetic annotation inspection and authoring, row/column phonetic-visibility
  handling, and default frozen-pane inspection and authoring under separate presentation revisions.
- Persistent `WorkbookCalculationSession` state with incremental recalculation, conservative full
  fallback, optimistic revision checks, request-owned cooperative cancellation, stale-result
  rejection, and bounded paged result deltas.
- Typed Python 3.10–3.14 bindings built with PyO3 and maturin.
- Typed Node.js 22+ CommonJS and ESM bindings built with napi-rs and exact-version native platform
  packages.
- Explicit, idempotent native-session cleanup through Python context managers and Python/Node.js
  `close()`, including cooperative cancellation of active calculation and stable closed-session
  errors.
- A local stdio-only MCP server with 11 high-level workbook tools, explicit approved roots,
  bounded TTL/LRU sessions, capability-bound bounded input reads, byte-bounded responses and
  resource pagination, cooperative cancellation, and capability-bound atomic Save As behavior.
- Stable validation, read, write, calculation, session, diagnostic, and spreadsheet-value error
  boundaries with machine-readable codes, including a core-owned `ValidationErrorCode` covering
  every current `ValidationError` variant.
- Runnable examples for inspection, capability scanning, error handling, saved-versus-calculated
  values, calculation, canonical authoring, recalculation Save As, dynamic arrays, phonetic
  inspection, and phonetic authoring.
- Generated workbook integration tests, binding conformance tests, MCP protocol tests, fuzz
  targets, MSRV checks, dependency-policy checks, package-consumer verification, and
  cross-platform CI.

### Known limitations

- Public CI uses generated, redistributable workbook fixtures. The private external formula corpus,
  user workbook corpus, and native-producer evidence used during development are not distributed
  with 0.1.0 and are not represented as release gates.

[Unreleased]: https://github.com/emulette/cellrune/compare/v0.1.5...HEAD
[0.1.5]: https://github.com/emulette/cellrune/compare/v0.1.4...v0.1.5
[0.1.4]: https://github.com/emulette/cellrune/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/emulette/cellrune/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/emulette/cellrune/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/emulette/cellrune/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/emulette/cellrune/releases/tag/v0.1.0
