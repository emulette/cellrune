# Numerical behavior

This reference describes where CellRune's calculated values match Microsoft Excel exactly, where
they match within a tolerance, and where the engines deliberately differ.

The statements below are tied to recorded workbooks and named Excel builds. Function families not
yet measured against that reference are listed separately.

## Reference oracle

Compatibility statements are made against committed, named saved-cache baselines, not against
"Excel" in general. CellRune records Excel Online and Mac Excel 2021 workbooks independently.
Excel Online is the primary compatibility reference; Mac Excel 2021 is an additional reference.

| Workbook source | Excel cache producer | Locale | Recorded |
| --- | --- | --- | --- |
| CellRune 0.1.7 formula oracle | Microsoft Excel Online, AppVersion `16.0300` | en-US UI; ko-KR regional format | 2026-07-29 |
| CellRune 0.1.7 formula oracle | Microsoft Macintosh Excel, AppVersion `16.0300` | en-US UI; ko-KR regional format | 2026-07-29 |
| Apache POI formula fixture | Microsoft Excel 2013, AppVersion `15.0300` | en-US currency formatting observed; `1900` date system | 2016-02-15 |
| Apache POI matrix fixture | Microsoft Excel 2016, AppVersion `16.0300` | not recorded; `1900` date system | 2017-07-27 |

Excel's own results travel inside every workbook it saves, as the cached `<v>` value of each
formula cell. That makes any Excel-authored workbook both a test input and its own ground truth.
The saved workbooks, host metadata, and reviewed classifications are committed under
`conformance/`.

The 0.1.7 suite records `excel-online-free-en-ui-ko-kr` and
`excel-mac-2021-home-student-en-ui-ko-kr-no-euro-tools`. A missing saved value is
`host_unsupported` for that workbook. The case remains active even when every recorded workbook
lacks a value, so later 0.1.7 patch releases can reuse the same setup.

## Verified

### Selectable: iterative financial solvers

`IRR`, `XIRR`, and `RATE` are root-finding functions with no closed form, so their results depend on
the search that produced them. From 0.1.3 the search budget is a calculation option.

| Function | Default maximum | Default step tolerance | `ExtendedSearch` |
| --- | --- | --- | --- |
| `IRR`, `RATE` | 20 | `1e-7` | 100 / `1e-10` |
| `XIRR` | 100 | `1e-8` | 100 / `1e-10` |

The default reproduces the function-specific iteration budgets and tolerances Microsoft documents.
When CellRune's Newton search exhausts the applicable budget, it returns `#NUM!`.

The default reproduces the budget, not Excel's search itself, which Microsoft does not document.
Two searches with the same budget can still disagree on which borderline inputs converge.

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
where the exact answer is zero. Excel corrects some such results to zero but preserves others;
IEEE-754 preserves every residue. From 0.1.3 this is a calculation option, and Excel's narrow
correction is the default.

| Formula | Default (`ExcelNearZero`) | Opt-in (`Ieee754`) |
| --- | --- | --- |
| `=0.1+0.2-0.3` | `0` | `5.551115123125783e-17` |
| `=(0.5-0.4)-0.1` | `0` | `-2.7755575615628914e-17` |
| `=SUM(0.1,0.2,-0.3)` | `0` | `5.551115123125783e-17` |
| `=SUMPRODUCT({0.1,0.2,-0.3})` | `0` | `5.551115123125783e-17` |
| `=100.1-100-0.1` | `-5.689893001203927e-15` | `-5.689893001203927e-15` |

The correction is applied at each addition and subtraction, in the operator path and in the
policy-aware running totals used by `SUM`, `AVERAGE`, `SUMIF(S)`, `AVERAGEIF(S)`, `SUBTOTAL`,
`SUMPRODUCT`, and `NPV`. Alongside the `f64` result, scalar addition/subtraction trees and
aggregate accumulators carry an exact trace of the parsed decimal inputs. `SUMPRODUCT` multiplies
before it adds, and the product of two exact decimals is itself an exact decimal, so its terms stay
traceable. `NPV` divides before it adds, and carries the same proof as an exact rational trace
through discounting.

LET and MAP scope transport does not choose between `ExcelNearZero` and `Ieee754`. A scope entry
preserves the value together with its optional decimal trace across scalar, array, calculated-cell,
and cell/range-reference bindings; the arithmetic operator still consults the workbook's selected
mode when it consumes that value. Consequently `LET(x,0.1+0.2,x-0.3)` is identical to its direct
equivalent in each mode: zero under `ExcelNearZero`, and the raw IEEE-754 residue under `Ieee754`.
The scope representation prevents trace loss; it does not impose Excel arithmetic semantics.

A result is replaced with zero only when both of these conditions hold:

1. the exact decimal or rational trace is zero; and
2. the binary residue is at most `1e-15 * max(abs(left), abs(right))` for the cancelling
   operation.

The exact trace prevents a nearby real difference from being swallowed. The relative binary
boundary distinguishes Excel's saved zero for `=0.1+0.2-0.3` from its saved residue for
`=100.1-100-0.1`. The `1e-15` coefficient is an empirical compatibility contract pinned by the
committed Excel workbook; Microsoft describes its near-zero correction and 15-digit precision but
does not publish this boundary as an algorithm. See Microsoft's
[floating-point accuracy note](https://learn.microsoft.com/en-us/troubleshoot/microsoft-365-apps/excel/floating-point-arithmetic-inaccurate-result).

The choice matters beyond the number itself:
`=(0.1+0.2-0.3)=0` is `TRUE` under the default and `FALSE` under `Ieee754`, and the same choice
reaches every `IF` branch that compares a computed value against zero.

Releases 0.1.0 through 0.1.2 behaved as `Ieee754` does. It remains available:

```rust
use cellrune::{ArithmeticSemantics, CalculationOptions};

let options = CalculationOptions::default()
    .with_arithmetic_semantics(ArithmeticSemantics::Ieee754);
```

The ordinary regression suite checks exact-trace propagation and the relative boundary in
table-driven cases. The separate local oracle audit checks both
`=0.1+0.2-0.3`, which Excel saved as zero, and `=100.1-100-0.1`, which Excel saved with its residue,
using bit-exact comparisons. It also keeps
`=100.1-100-0.099999999999999`, whose exact value is nonzero, nonzero. Operator, array, ordinary
aggregate, conditional aggregate, and `SUMPRODUCT` paths are compared under both modes, including
a `SUMPRODUCT` whose terms cancel only after multiplication.

Under `Ieee754` no path consults the exact trace, so none is computed.

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

### Deliberate difference: the height of a whole-column array

Excel treats `A:A` in an array expression as the full 1,048,576 rows. CellRune materializes a
bounded height instead, so `=COUNT(A:A*B:B)` counts the populated extent rather than a million
rows. Some clamp is unavoidable; which one is a real choice, and this is the one CellRune makes:

**The extent is the greatest populated row among the columns the expression references** — not the
sheet's overall used range. Two whole-column operands of different heights therefore share one
height, the taller of the two, and the shorter one contributes blanks that arithmetic coerces to
zero. That reproduces Excel for the cases the oracle fixes: `SUM(T:T*U:U)` at equal heights and
`SUM(T:T*V:V)` at unequal heights both match the saved Excel results.

Scoping the clamp to the referenced columns rather than to the whole sheet is what makes the result
reproducible. A sheet-wide clamp would let a value written into a column the expression never names
change its answer, while the formula's dependency rectangles — which cover only the named columns —
stayed untouched. A full recalculation would then produce one answer and an incremental pass would
keep another. Under the column-scoped rule the value is a function of the recorded dependencies
alone, so the two passes agree by construction.

Aggregates that walk a whole-column reference directly, such as `SUM(A:A)`, are unaffected either
way: they skip blank cells, so a wider clamp cannot change their result.

### Measured agreement

The 0.1.7 audit records:

| Workbook | Selected results | Match | Divergent | Not implemented | Host unsupported | Excluded |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Apache POI formula fixture | 1,295 | 1,290 | 5 | 0 | 0 | 0 |
| Apache POI matrix fixture | 266 | 158 | 48 | 60 | 0 | 0 |
| CellRune formula oracle — Excel Online | 672 | 404 | 11 | 255 | 2 | 0 |
| CellRune formula oracle — Mac Excel 2021 | 672 | 401 | 10 | 239 | 22 | 0 |

`match` uses each case's reviewed comparator: finite numbers default to a scale-relative `1e-8`,
while cancellation and signed-zero probes use exact bits. `divergent` records and enforces both
the Excel value and the current CellRune value with an explanatory note. The other states make
unsupported or non-comparable cases explicit rather than dropping them from the denominator.
These counts are an audit inventory, not a composite score or release threshold.

The 0.1.7 workbook has 703 primary cases, 672 active cases, and 897 formula cells. Excel Online
and Mac Excel 2021 both saved the workbook without losing formulas. A host that stores no usable
value for a case is recorded as `host_unsupported`; this includes both PIVOTBY probes in both
workbooks. Missing host values do not require regenerating the workbook or block publication.

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
together with the Excel build that produced it. Excel's answer can differ between builds, so the
build matters.
