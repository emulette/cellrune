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
| CellRune 0.1.8 formula oracle | Microsoft Excel Online, AppVersion `16.0300` | en-US UI; ko-KR regional format | 2026-07-30 |
| CellRune 0.1.8 formula oracle | Microsoft Macintosh Excel, AppVersion `16.0300` | en-US UI; ko-KR regional format | 2026-07-30 |
| Apache POI formula fixture | Microsoft Excel 2013, AppVersion `15.0300` | en-US currency formatting observed; `1900` date system | 2016-02-15 |
| Apache POI matrix fixture | Microsoft Excel 2016, AppVersion `16.0300` | not recorded; `1900` date system | 2017-07-27 |

Excel's own results travel inside every workbook it saves, as the cached `<v>` value of each
formula cell. That makes any Excel-authored workbook both a test input and its own ground truth.
The saved workbooks, host metadata, and reviewed classifications are committed under
`conformance/`.

The 0.1.8 suite records `excel-online-free-en-ui-ko-kr` and
`excel-mac-2021-home-student-en-ui-ko-kr-no-euro-tools`. A missing saved value is
`host_unsupported` for that workbook. The case remains active even when every recorded workbook
lacks a value, so later releases can reuse the same setup.

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

### Matching: probability distributions

The gamma, beta, binomial, and hypergeometric families added in 0.1.12 are measured against the
frozen two-profile oracle rather than against documentation alone. The suite holds 60 active cases
for the twenty names, three per name, and all 60 are classified `match` in both
`excel-online-free-en-ui-ko-kr` and `excel-mac-2021-home-student-en-ui-ko-kr-no-euro-tools`.

The F, t, Z-test, and sample-covariance names added in the current unreleased source are measured
the same way. Their 60 active cases, again three per official name, are all classified `match` in
both frozen host profiles. The distribution grid additionally pins large degrees of freedom,
inverse round trips, and lower, upper, and two-tailed probabilities. Dedicated regression tests
cover representable subnormal F tails and extreme finite samples whose intermediate variances or
variance ratios would overflow without scaling.

A family's cumulative and quantile forms are evaluated through one shared tail kernel, so they
cannot drift apart from each other; the density forms are built on the same `ln_gamma` and
`ln_beta` primitives. Every kernel is first-party; no external special-function crate is linked.

### Deliberate difference: probability-distribution numeric policies

The policies below are chosen, documented, and pinned by tests. Each one prefers a typed Excel
error over a number the engine cannot stand behind.

`ln_gamma` is a Lanczos approximation with `g = 607/128` and the 14-term Godfrey coefficient set.
Its relative error stays below `1e-13` across the representable domain, degrading to a few ULP of
absolute error near the zeros at `x = 1` and `x = 2`. Its normalization is assembled in log space,
so small positive arguments do not overflow an otherwise representable result. Every other kernel
here inherits that accuracy floor.

`GAMMA.DIST` forms `ln(x) - ln(beta)` before materializing the scaled coordinate. This preserves
finite lower tails when direct division would underflow and resolves ratios above the largest
finite double at their limiting CDF or density. `BETA.DIST` and `BETA.INV` normalize finite support
coordinates before subtraction when `B - A` overflows; density Jacobians use the resulting log
width, and inverse interpolation uses a finite convex combination.

F and t probabilities evaluate the needed incomplete-beta tail directly instead of subtracting
it from one. The kernel carries both coordinate logarithms, so an F ratio that overflows or a
coordinate that underflows can still produce a representable subnormal answer. For shapes of at
least `1e6` within twelve standard deviations of the mean, it uses a uniform central expansion;
outside that region a compensated continued fraction avoids cancellation between a large
normalization term and the fraction. Every central-series and continued-fraction step charges the
function-iteration budget and observes cancellation. F and t inverse functions refine against
those same tails, and the covariance and test functions compute sample moments and standard
errors with explicit scaling before squaring or summing.

At extreme equal shapes, `a = b` on the order of `5e6` and above — a bound that follows from the
kernel's `lnΓ` ULP error model rather than from an in-repo fixture — the incomplete-beta symmetry
seam is floored by the ULP of those `lnΓ` terms, so the two branches can disagree — and order — by
that margin. This is accepted fail-closed policy: a quantile refinement that cannot meet both the
bracket-width and probability-residual tolerances returns `#N/A` from `GAMMA.INV` and `BETA.INV`,
the error Microsoft documents for a failed inverse-distribution search. It never returns an
unconverged number.

`HYPGEOM.DIST` and `HYPGEOMDIST` evaluate sample sizes through 10,000 as a falling-factorial
product whose factors are individually moderate, so no two large `lnΓ` values are ever differenced.
Above that sample size the kernel uses the `lnΓ` form, whose relative error grows with the
population as `N·ln(N)·f64::EPSILON`. The threshold caps the per-evaluation cost while covering
every sample size a spreadsheet realistically draws.

`BINOM.DIST` lower CDFs, cumulative `NEGBINOM.DIST`, and the CDF probes used by `BINOM.INV` and
`CRITBINOM` use regularized incomplete-beta identities instead of enumerating the support.
Binomial and negative-binomial masses form `ln(1-p)` with `ln1p`, and the two one-term binomial
CDF edges use stable closed forms, so a tiny probability is not rounded away before exponentiation.
`BINOM.INV` and `CRITBINOM` still bisect over integer results and verify
`CDF(k-1) < alpha ≤ CDF(k)` before returning; a failed verification is `#NUM!`. Minimality is exact
against this module's own `f64` incomplete-beta CDF. Full-support and degenerate-probability cases
return their exact endpoint without entering an iterative kernel. Interior `BINOM.DIST.RANGE`
summations and non-degenerate binomial quantiles reject support indices above `2^53` with `#NUM!`,
rather than silently saturating or aliasing a lossy `f64`-to-integer conversion.

The incomplete-gamma kernel deliberately resolves a tail below `ln(f64::MIN_POSITIVE)`, about
`-708.396`, to zero. The incomplete-beta kernel instead keeps the direct tail in log space through
its final exponentiation, preserving subnormal F, t, beta, binomial, and negative-binomial
probabilities when `f64` can represent them. Values below the smallest `f64` subnormal still
resolve to zero and their complements to one.

Density endpoints follow Excel's documented pole, limit, and zero cases rather than the IEEE
result of the formula:

| Function | Endpoint | Shape < 1 | Shape = 1 | Shape > 1 |
| --- | --- | --- | --- | --- |
| `GAMMA.DIST` | `x = 0` | `#NUM!` | `1 / beta` | `0` |
| `BETA.DIST` | `x = A` | `#NUM!` | `beta / (B - A)` | `0` |
| `BETA.DIST` | `x = B` | `#NUM!` | `alpha / (B - A)` | `0` |

Interior `BETA.DIST` densities carry the `1 / (B - A)` Jacobian, matching Microsoft's own
documented example.

### Measured agreement

The 0.1.8 audit records:

| Workbook | Selected results | Match | Divergent | Not implemented | Host unsupported | Excluded |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Apache POI formula fixture | 1,295 | 1,290 | 5 | 0 | 0 | 0 |
| Apache POI matrix fixture | 266 | 158 | 48 | 60 | 0 | 0 |
| CellRune formula oracle — Excel Online | 1,496 | 890 | 21 | 585 | 0 | 0 |
| CellRune formula oracle — Mac Excel 2021 | 1,496 | 874 | 21 | 578 | 23 | 0 |

`match` uses each case's reviewed comparator: finite numbers default to a scale-relative `1e-8`,
while cancellation and signed-zero probes use exact bits. `divergent` records and enforces both
the Excel value and the current CellRune value with an explanatory note. The other states make
unsupported or non-comparable cases explicit rather than dropping them from the denominator.
These counts are an audit inventory, not a composite score or release threshold.

The 0.1.8 workbook has 1,527 primary cases, 1,496 active cases, and 1,892 formula cells. Excel Online
and Mac Excel 2021 both saved the workbook without losing formulas. A host that stores no usable
value for a case is recorded as `host_unsupported`. Both PIVOTBY probes instead retain semantic
empty anchors and complete 5×4 results in both workbooks; they are currently `not_implemented` on
the CellRune side. Missing host values do not require regenerating the workbook or block
publication.

## Unverified

The following families have unit and golden tests derived from Microsoft's primary documentation,
but no recorded tolerance against the reference oracle. Do not read their absence from the verified
section as a claim of exactness in either direction.

- statistical distributions and inverse distributions outside the gamma, beta, binomial,
  hypergeometric, F, and t families measured in 0.1.12 and the current unreleased source
- closed-form financial functions
- transcendental math and trigonometric functions, which are evaluated through `libm`
- engineering and Bessel functions

Priority for measurement is statistical, then closed-form financial, then math.

## Reporting a difference

Open an issue with the formula text, the input values, the CellRune result, and the Excel result
together with the Excel build that produced it. Excel's answer can differ between builds, so the
build matters.
