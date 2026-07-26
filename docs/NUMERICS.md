# Numerical behavior

This reference describes where CellRune's calculated values match Microsoft Excel exactly, where
they match within a tolerance, and where the engines deliberately differ.

The statements below are tied to recorded workbooks and named Excel builds. Function families not
yet measured against that reference are listed separately.

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

### Selectable: iterative financial solvers

`IRR`, `XIRR`, and `RATE` are root-finding functions with no closed form, so their results depend on
the search that produced them. From 0.1.3 the search budget is a calculation option.

| | Default (`ExcelIterationBudget`) | Opt-in (`ExtendedSearch`) |
| --- | --- | --- |
| Method | Newton–Raphson | Newton–Raphson |
| Maximum iterations | 20 | 100 |
| Convergence tolerance | `1e-7` on the step | `1e-10` on the step |

The default reproduces the iteration budget and tolerance Microsoft documents for these three
functions, so an input Excel abandons is abandoned here too and yields `#NUM!`.

**What the default does not reproduce is Excel's search itself, which Microsoft does not document.**
Two searches with the same budget can still disagree about which borderline inputs converge. Expect
agreement on which inputs are hopeless, not a guarantee of an identical `#NUM!` boundary.

Releases 0.1.0 through 0.1.2 behaved as `ExtendedSearch` does, with no way to select otherwise.
That search returns a value for some inputs where Excel returns `#NUM!`. It remains available:

```rust
use cellrune::{CalculationOptions, FinancialSolverSemantics};

let options = CalculationOptions::default()
    .with_financial_solver_semantics(FinancialSolverSemantics::ExtendedSearch);
```

The default changed because a compatibility engine's job is to agree with the tool its inputs came
from. Searching longer produces the mathematically better answer, but it produces it where Excel
produces an error, and a caller comparing the two sees a defect rather than an improvement.
Whichever policy is selected, both find the same root when both converge; they differ only in when
they give up.

### Deliberate difference: `DOLLAR` currency formatting

`DOLLAR` returns a locale-independent `$` format, for example `$0.00`, `$1.00`, and `($1.00)`.

Microsoft documents the currency symbol of `DOLLAR` as dependent on system regional settings, so an
Excel installation with Korean regional settings saves `₩0`, `₩1`, and `(₩1)` for the same formula.
CellRune never reads host locale as an implicit calculation input, because doing so would make the
same workbook calculate differently on two machines and break the determinism the rest of the API
depends on. There is currently no explicit locale input to opt into the host convention.

Differences of this kind are formatting differences, not calculation failures.

### Selectable: arithmetic that cancels to near zero

Writing `0.1` in binary is inexact, so a sum or difference of decimal literals can leave a residue
where the exact answer is zero. Excel corrects such a result to zero; IEEE-754 does not. From 0.1.3
this is a calculation option, and the correction is the default.

| Formula | Default (`ExcelNearZero`) | Opt-in (`Ieee754`) |
| --- | --- | --- |
| `=0.1+0.2-0.3` | `0` | `5.551115123125783e-17` |
| `=(0.5-0.4)-0.1` | `0` | `-2.7755575615628914e-17` |
| `=SUM(0.1,0.2,-0.3)` | `0` | `5.551115123125783e-17` |

The correction is applied at each addition and subtraction, in the operator path and in the running
total that `SUM`, `AVERAGE`, `SUMIF`, `SUBTOTAL`, and `NPV` share. That matters beyond the number
itself: `=(0.1+0.2-0.3)=0` is `TRUE` under the default and `FALSE` under `Ieee754`, and the same
choice reaches every `IF` branch that compares a computed value against zero.

Releases 0.1.0 through 0.1.2 behaved as `Ieee754` does. It remains available:

```rust
use cellrune::{ArithmeticSemantics, CalculationOptions};

let options = CalculationOptions::default()
    .with_arithmetic_semantics(ArithmeticSemantics::Ieee754);
```

#### What the correction deliberately does not reach

The window is relative to the operands of the operation being corrected, so it removes residue
created *at that operation's magnitude*. `=100.1-100-0.1` is exactly zero and still returns
`-5.689893001203927e-15`: the residue was created by the first subtraction, where it is a small
fraction of `100.1`, and by the second subtraction the operands are around `0.1`, where the same
residue is far too large to look like cancellation noise.

Widening the window is not the fix. `=1.0000000000001-1` is a difference the author meant, sits at
almost the same relative magnitude, and is well within the fifteen significant digits Excel keeps —
so any threshold that catches the first corrupts the second. Separating them requires carrying an
error term through every intermediate, which is a different engine rather than a wider constant.

**Compare calculated numbers with a tolerance rather than for equality**, under either policy.

#### Not part of this: fifteen-digit display

`=1.1-1` shows `0.1` in Excel and `0.10000000000000009` here, and that is a *display* difference,
not this correction. Excel stores the same residue — `=(1.1-1)=0.1` is `FALSE` in Excel too — and
renders to fifteen significant digits. No arithmetic policy changes that value, and none should.

#### Not part of this: signed zero

No calculated value is negative zero under either policy. The engine normalizes `-0.0` at the
boundary where calculated values leave the calculation, so an empty `SUM`, `MIN(-0)`, and
`PRODUCT(-1,0)` all return positive zero as Excel reports; see the 0.1.1 entry in `CHANGELOG.md`.

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
