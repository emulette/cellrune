# Numeric contract

Where CellRune's calculated values are intended to match Microsoft Excel exactly, where they are
intended to match within a tolerance, and where they deliberately differ.

This file records only what has been verified. Function families that have not yet been measured
against a recorded Excel oracle are listed as unverified rather than given an aspirational
tolerance. A tolerance table that has not been measured would be the same kind of unverifiable
claim this file exists to remove.

## Reference oracle

Compatibility statements are made against a named build, not against "Excel" in general.

| Property | Value |
| --- | --- |
| Product | Microsoft Excel for Mac 16.111 |
| Locale | Korean system locale, `1900` date system |
| Recorded | 2026-07-24 |

Excel's own results travel inside every workbook it saves, as the cached `<v>` value of each
formula cell. That makes any Excel-authored workbook both a test input and its own ground truth.

## Verified

### Deliberate difference: iterative financial solvers

`IRR`, `XIRR`, and `RATE` are root-finding functions with no closed form, so their results depend on
the search that produced them.

| | CellRune | Excel (documented) |
| --- | --- | --- |
| Method | Newton–Raphson | undocumented |
| Maximum iterations | 100 | 20 |
| Convergence tolerance | `1e-10` on the step | `1e-7` relative |

CellRune searches longer and converges tighter. The practical consequence is not a rounding
difference: **CellRune returns a value for some inputs where Excel returns `#NUM!`**, because Excel
abandons the search after 20 attempts. Where both converge, results agree well within `1e-8`.

This is a deliberate choice in favour of the mathematically better answer. Treat a CellRune result
with an Excel `#NUM!` counterpart as a difference in search effort, not as a defect on either side.

### Deliberate difference: `DOLLAR` currency formatting

`DOLLAR` returns a locale-independent `$` format, for example `$0.00`, `$1.00`, and `($1.00)`.

Microsoft documents the currency symbol of `DOLLAR` as dependent on system regional settings, so an
Excel installation with Korean regional settings saves `₩0`, `₩1`, and `(₩1)` for the same formula.
CellRune never reads host locale as an implicit calculation input, because doing so would make the
same workbook calculate differently on two machines and break the determinism the rest of the API
depends on. There is currently no explicit locale input to opt into the host convention.

Differences of this kind are formatting differences, not calculation failures.

### Deliberate difference: IEEE-754 arithmetic near zero

CellRune evaluates arithmetic operators in IEEE-754 double precision and does not reproduce Excel's
correction that snaps a near-zero addition or subtraction result to exactly zero.

| Formula | CellRune | Excel |
| --- | --- | --- |
| `=0.1+0.2-0.3` | `5.551115123125783e-17` | `0` |
| `=(0.5-0.4)-0.1` | `-2.7755575615628914e-17` | `0` |
| `=1.1-1` | `0.10000000000000009` | `0.1` |

Compare such results with a tolerance rather than for equality. Note that this also affects
comparisons: a formula of the form `=(0.1+0.2-0.3)=0` is `FALSE` in CellRune and `TRUE` in Excel.

Signed zero is not part of this difference. No calculated value is negative zero: the engine
normalizes `-0.0` at the boundary where calculated values leave the calculation, so an empty
`SUM`, `MIN(-0)`, and `PRODUCT(-1,0)` all return positive zero as Excel reports; see the 0.1.1
entry in `CHANGELOG.md`.

### Matching: dates, including the 1900 leap-year bug

The `1900` date system reproduces Excel's historical calendar rather than the correct one. Serial
`60` is the non-existent date 1900-02-29, and the weekday sequence is preserved accordingly, so
serial `1` is a Sunday. This is required for compatibility, not an oversight: workbooks authored in
Excel carry serials produced under that calendar.

The `1904` date system is handled separately and has no such adjustment.

### Measured agreement

A workbook of 1,295 formula cells recalculated and saved by the reference oracle above compares as
follows:

| Outcome | Cells |
| --- | ---: |
| Exact match | 1,259 |
| Match within `1e-8` | 28 |
| `DOLLAR` locale difference | 8 |
| No calculated value produced | 0 |

Excluding the eight formatting differences, all 1,287 remaining results match within `1e-8`.

## Unverified

The following families have unit and golden tests derived from Microsoft's primary documentation,
but no recorded tolerance against the reference oracle. Do not read their absence from the verified
section as a claim of exactness in either direction.

- statistical distributions and inverse distributions
- closed-form financial functions
- transcendental math and trigonometric functions, which are evaluated through `libm`
- engineering and Bessel functions

Priority for measurement is statistical, then closed-form financial, then math.

## Reporting a difference

Open an issue with the formula text, the input values, the CellRune result, and the Excel result
together with the Excel build that produced it. A difference against an unnamed Excel version
cannot be acted on, because the answer may differ between builds.
