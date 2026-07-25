# Changelog

All notable changes to CellRune are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Releases follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
  come from the committed `Cargo.lock`. Two new gates bound the ranges: a per-pull-request job
  compiles the declared floors against the minimum supported Rust version, and a scheduled job
  compiles the newest compatible graph against it.

### Fixed

- Quoted external-workbook references such as `'[1]Sheet1'!A1` were reported as supported and
  evaluated to `#REF!`. Because that is a spreadsheet error value rather than an engine issue,
  `IFERROR` could hide it, so a workbook could return a plausible number for a formula the engine
  cannot evaluate. Both the capability scan and evaluation now reject the external prefix. The
  unquoted spelling was already rejected, as the formula lexer has no bracket token.
- `SUM`, `SUMSQ`, and `NPV` returned negative zero when no numbers participated. `Iterator::sum`
  for `f64` folds from `-0.0`, which is the additive identity for floats but not the value Excel
  reports. A shared summation helper now folds from `+0.0`.
- Formula parse-error details labelled every position as `token N`, but a lexing failure has no
  token stream and was reporting a character offset under that label. Lexing failures now report
  `character N` and parsing failures continue to report `token N`.
- The lambda function group dispatched without inspecting the normalized function name, unlike
  every sibling group. A second function in that group would have silently evaluated as `MAP`.
- A release version bump had to be repeated by hand in more places than the release checklist
  listed, and the ones it missed — a pnpm lockfile, a generated loader shim, the packaged-consumer
  lockfile, and the release verification scripts — fail only during a release run. Every version
  is now derived from the workspace manifest where possible, and a new gate asserts that the
  remaining declarations agree.

### Documentation

- The 0.1.0 entry claimed capability detection for unsupported external, structured, and
  spill-postfix formula forms. Only 3-D and data-table forms have dedicated detection; the other
  three surface as `calculation.parse_error`, and structured references and spill-postfix
  references still do after this release. The entry has been corrected.
- Added `docs/NUMERICS.md`, which records where calculated values deliberately differ from Excel,
  the Excel build each statement was measured against, and which function families are unmeasured.
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

[Unreleased]: https://github.com/emulette/cellrune/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/emulette/cellrune/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/emulette/cellrune/releases/tag/v0.1.0
