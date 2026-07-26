# Roadmap

The README's [Scope](https://github.com/emulette/cellrune/blob/main/README.md#scope) section says
what CellRune does not support today. This document says which of those gaps are planned, in what
order, and which ones are deliberately out of scope.

**Version numbers here are a plan, not a commitment.** They record the order work is expected to
land in and the prerequisites between pieces. Ordering can change, and an unreleased version's
contents can move. Nothing below is a dated promise. The
[CHANGELOG](https://github.com/emulette/cellrune/blob/main/CHANGELOG.md) is the record of what
actually shipped.

Current release: **0.1.2**.

## Planned releases

| Version | Theme | What callers see |
|---|---|---|
| 0.1.3 | Phonetic consumption, calculation compatibility modes | Bulk iteration and UTF-16 range resolution for phonetic annotations; an explicit choice between Excel-compatible and pure IEEE-754 numeric semantics. **Changes default calculated values** — see below. |
| 0.1.4 | Preparatory refactor | No behavior change, except three latent dependency-tracking defects fixed |
| 0.1.5 | 3-D references, whole-column arrays | `SUM(Sheet1:Sheet3!A1)` and `SUM(A:A*B:B)` calculate. Two unsupported items removed. |
| 0.1.6 | Table reading | Table metadata is exposed on the workbook model; structured references reclassify from a parse error to an explicit unsupported-capability issue |
| 0.1.7 | `LET` | `LET` calculates |
| 0.1.8 | `LAMBDA` core | Named `LAMBDA` definitions and closures calculate |
| 0.1.9 | Lambda helpers, immediate invocation | `BYROW`, `BYCOL`, `SCAN`, `MAKEARRAY`, `REDUCE`, `ISOMITTED`, and `LAMBDA(...)(args)`. One unsupported item removed. |
| 0.1.10 | Structured-reference syntax | No new evaluation — the parser accepts the full bracket grammar so classification is accurate |
| 0.1.11 | Structured-reference resolution | `Table1[Amount]` and its variants calculate. One unsupported item removed. |
| 0.1.12 | Spill-postfix references, function catalog target | `A1#` calculates; the function catalog reaches its expansion target |
| 0.1.13 | Table editing | Renaming a table column rewrites the structured references that use it |
| 0.2.0 | Pre-1.0 preparation | No new features. Public enums become extensible before 1.0 freezes them. |

### 0.1.3 changes default calculated values

0.1.3 is planned to make Excel-compatible arithmetic and Excel's per-function financial solver
contracts the default, rather than the pure IEEE-754 arithmetic and extended solver search that
0.1.2 uses. Under the new default `=(0.1+0.2-0.3)=0` is `TRUE`, matching Excel, where 0.1.2 answers
`FALSE`.

This changes results and branch outcomes for existing callers, so it is a deliberate behavior
change rather than a bug fix that happens to be invisible. The 0.1.2 semantics stay available as an
explicit opt-in, and the release notes will carry a migration example. Both modes are fully
deterministic; the difference is which compatibility contract they honor.

## Function catalog

The catalog currently pins 278 official Excel-facing names. Expansion toward 420 is a workstream
that runs alongside the releases above in small batches from 0.1.5, not a single release.

A name counts only once argument arity and omission, coercion rules, blank/text/logical/error
propagation, error boundaries, array shape and spill policy, date system, and iteration budget are
implemented and tested against Microsoft's primary documentation, and only once the capability scan
and calculation agree on the same contract. Registering a name or calculating some of its arguments
does not count. Published counts for other engines are not necessarily measured the same way.

Missing a batch target does not hold a release.

## Prerequisites

Some of the ordering above is a hard dependency rather than a preference:

- **0.1.6 → 0.1.10 → 0.1.11.** Structured-reference syntax cannot be classified correctly without
  the table model, and cannot be resolved without the syntax.
- **0.1.7 → 0.1.8 → 0.1.9.** Closures need the scope model `LET` introduces; the helper kernels
  need closures.
- **0.1.10 → 0.1.13.** The table-edit formula rewriter consumes the structured-reference sublexer.

After 0.1.4, the 0.1.5, 0.1.6, and 0.1.7 branches are independent of one another and can land in
whatever order they become ready.

## Under consideration

Not scheduled, not ruled out:

- **Iterative calculation.** Few workbook engines implement it, and CellRune's bounded-work
  contract needs an explicit iteration budget design first.
- **Data-table calculation.** Data-table forms are already detected and reported as an explicit
  unsupported capability rather than a parse failure; evaluation is what is missing.
- **Intersection and union operators.** The intersection operator needs the formula lexer to stop
  discarding whitespace; the union operator does not exist in the grammar at all.
- **Phonetic authoring beyond the current surface**, including `PHONETIC()` calculation, rich-text
  plus phonetic authoring, and RTL pane authoring.

## Out of scope

These are deliberate boundaries, not backlog items:

- `.xls`, `.xlsb`, `.ods`, and CSV. CellRune reads Transitional SpreadsheetML `.xlsx`/`.xlsm` only.
- Macro, add-in, query, and data-connection execution. Macros are never executed.
- External-workbook link following. External links are reported as a diagnostic and never resolved.
  Reading cached external values would add an unbudgeted parsing surface and a new trust boundary.
- Reading the host clock. `TODAY()` and `NOW()` require explicitly injected serials, and no
  calculation reads ambient time or host locale.
- Complete Excel compatibility. Known deliberate numeric differences are recorded in
  [`NUMERICS.md`](https://github.com/emulette/cellrune/blob/main/docs/NUMERICS.md).

## Before 1.0

1.0 is not scheduled. It follows 0.2.0 stabilizing, and the language bindings follow the core.

0.2.0 exists for one reason: sixteen enums re-exported from the crate root are not
`#[non_exhaustive]`. Adding that attribute is itself a breaking change, so it has to happen before
1.0 freezes the public API — otherwise no variant could be added to any of them for the whole 1.x
line. The invariant test `crates/cellrune/tests/public_api.rs` pins the current set.
