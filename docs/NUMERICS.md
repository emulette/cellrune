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

Excel stores formula results as cached `<v>` values, but an arbitrary workbook cache can be stale or
come from a save that did not fully recalculate. The controlled workbooks in this reference were
fully recalculated and saved by the named Excel profiles, retain producer metadata, and serve as
reference observations only for those profiles and builds. The saved workbooks, host metadata, and
reviewed classifications are committed under `conformance/`.

The 0.1.8 suite records `excel-online-free-en-ui-ko-kr` and
`excel-mac-2021-home-student-en-ui-ko-kr-no-euro-tools`. A missing saved value is
`host_unsupported` for that workbook. The case remains active even when every recorded workbook
lacks a value, so later releases can reuse the same setup.

## Verified

### Existing function boundaries corrected in 0.1.21

The saved Online and Mac profiles both now match the existing numeric-text cases for
`ISEVEN`/`ISODD`, the zero-life `SLN` case (`#DIV/0!`), `ERROR.TYPE` on a spill error
(`9`), the two cross-type `LOOKUP` cases (`#N/A`), and invalid text holidays in
`NETWORKDAYS` (`#VALUE!`). These are corrections against the existing frozen workbooks.

Worksheet tests also cover truncating `QUARTILE.INC` and its `QUARTILE` alias's quart
argument before range validation and interpolation: `1.9` selects quartile 1, `4.9`
selects quartile 4, and `-0.9` truncates to 0. `PERCENTILE.INC` continues to accept a
fractional probability. These additional boundaries use contract tests, not newly
recorded Excel observations.

Parity operates on the truncated magnitude of the finite binary64 input, avoiding a
saturating conversion to a signed 64-bit integer. Numeric text is accepted only in the
parity functions; other information functions keep their type-inspection semantics.

`LOOKUP` selects numeric, text, or logical candidates matching the search value's type
and skips blank or error keys. A blank search value follows the existing numeric-zero
comparison. Other lookup functions retain their own comparison rules.

Standard `WORKDAY`/`NETWORKDAYS` holidays accept numeric serials and text that reads as a
number, including date text such as `2026-01-02` in the locale profile's date order, and reject
other text.
`WORKDAY.INTL`/`NETWORKDAYS.INTL` preserve their existing numeric-only holiday collection:
text and logical entries are ignored. Both families ignore blank/logical entries,
propagate Excel errors, truncate valid fractional serials, and deduplicate holidays
under the workbook's date system. The INTL distinction is an existing CellRune contract,
not a claim of new Excel verification for every input shape.

### Selectable: iterative financial solvers

`IRR`, `XIRR`, and `RATE` are root-finding functions with no closed form, so their results depend on
the search that produced them. From 0.1.3 the search budget is a calculation option.

| Function | Default maximum | Default step tolerance | `ExtendedSearch` |
| --- | --- | --- | --- |
| `IRR`, `RATE` | 20 | `1e-7` | 100 / `1e-10` |
| `XIRR` | 100 | `1e-8` | 100 / `1e-10` |

The default reproduces the function-specific iteration budgets and tolerances Microsoft documents.
When CellRune's Newton search exhausts the applicable budget, it returns `#NUM!`. From 0.1.22 a
step that would land at or below -1 moves halfway from the current rate toward -1 instead, so a
search that overshoots the pole can still converge within the budget.

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

### Selectable: locale conventions

Microsoft documents the currency symbol of `DOLLAR` as dependent on system regional settings, so
Excel 16.114 for Mac with Korean regional settings shows `₩1.50` and `(₩1.50)` where an English
(United States) one shows `$1.50` and `($1.50)` for the same formulas. CellRune never reads
host locale as an implicit calculation input, because doing so would make the same workbook
calculate differently on two machines and break the determinism the rest of the API depends on.
Instead `CalculationOptions::with_locale_profile` selects one of the built-in locales, English
(United States) by default or Korean (Korea). The profile decides the `DOLLAR` currency symbol, the
month, weekday, and `AM`/`PM` names `TEXT`
shows, the currency symbol and date order text reads as a number with (`$5` and `1/15/2024` under
English, `₩5` and only year-first dates under Korean), and the `GROUPBY` and `PIVOTBY` total labels
(`Total` or `합계`). The labels follow a separate
display language, because an Excel with an English interface and Korean regional settings may
label totals in English. The Korean names and the label of a grand total over several levels have
not been measured, so they remain unsupported.

### Selectable: arithmetic that cancels to near zero

Writing `0.1` in binary is inexact, so a sum or difference of decimal literals can leave a residue
where the exact answer is zero. Excel corrects some such results to zero but preserves others;
IEEE-754 preserves every residue. From 0.1.3 this is a calculation option, and Excel's narrow
correction is the default.

| Formula | Default (`ExcelNearZero`) | Opt-in (`Ieee754`) |
| --- | --- | --- |
| `=0.1+0.2-0.3` | `0` | `5.551115123125783e-17` |
| `=SUM(0.1,0.2,-0.3)` | `0` | `5.551115123125783e-17` |
| `=(0.1+0.2-0.3)` | `5.551115123125783e-17` | `5.551115123125783e-17` |
| `=(0.1+0.2-0.3)*1E+17` | `5.551115123125783` | `5.551115123125783` |
| `=SUM(0.1,0.2,-0.3,0)` | `5.551115123125783e-17` | `5.551115123125783e-17` |
| `=SUMPRODUCT({0.1,0.2,-0.3})` | `5.551115123125783e-17` | `5.551115123125783e-17` |
| `=100.1-100-0.1` | `-5.689893001203927e-15` | `-5.689893001203927e-15` |

Every value in the default column is what Excel 16.0 build 20326 returns. Excel corrects only:

- the addition or subtraction a formula ends with, unparenthesized, element by element when it
  spills (`={0.1,1}+0.2-0.3` spills `0` and `0.8999999999999999`). Text operands are read as the
  operator reads them first, so `=0.1+0.2-"0.3"` is `0` (Mac Excel 16.114). A defined name's formula is a formula of its
  own, so a name defined as `=0.1+0.2-0.3` is `0` wherever it is used. A parenthesized root, a
  further operation, an `IF` branch, and a `LET` or `LAMBDA` body keep the residue, so
  `=0.1+0.2-0.3=0` is `FALSE`;
- the final addition of the totals of `SUM`, `AVERAGE`, `AVERAGEA`, `SUBTOTAL`, and `AGGREGATE`,
  so `=SUM(0.1,0.2,-0.3)*1E+17` is `0` while `=SUM(0.1,0.2,-0.3,0)`, whose final addition adds
  zero, keeps the residue. `SUMIF(S)`, `AVERAGEIF(S)`, the database functions, `SUMPRODUCT`, and
  `NPV` do not correct their totals.

A corrected result is zero when it is smaller than eight units in the last place of the larger
operand: below `2^(e-49)` for a larger operand in `[2^e, 2^(e+1))`. So `=1+7*2^-52-1` is `0`
and `=1+8*2^-52-1` is `1.7763568394002505e-15`, `=100.1-100-0.1` keeps its residue, and
`=1E+15+0.3-1E+15` is `0` although its exact decimal value is `0.3`: the rule looks at the binary
result only. Microsoft describes the correction in its
[floating-point accuracy note](https://learn.microsoft.com/en-us/troubleshoot/microsoft-365-apps/excel/floating-point-arithmetic-inaccurate-result)
without publishing the boundary; the boundary above is measured.

Releases 0.1.0 through 0.1.2 corrected no residue, as `Ieee754` does; under `Ieee754` the
comparison and rounding rules below also use the binary values. It remains available:

```rust
use cellrune::{ArithmeticSemantics, CalculationOptions};

let options = CalculationOptions::default()
    .with_arithmetic_semantics(ArithmeticSemantics::Ieee754);
```

The regression suite checks each formula above, the boundary cases, and the paths that do not
correct, bit for bit against the Excel values.

#### Also selected by this option: fifteen-digit comparison and rounding

`=1.1-1` keeps its residue, `0.10000000000000009`, under either policy; Excel shows it as `0.1`
because it renders fifteen significant digits, and so do comparisons and rounding. Under the
default:

- The comparison operators, `SWITCH`, and the criteria of `COUNTIF`, `SUMIF(S)`, `AVERAGEIF(S)`,
  `MAXIFS`, `MINIFS`, and the database functions compare two numbers by their roundings to
  fifteen significant digits, so `=(1.1-1)=0.1` and `=0.1+0.2=0.3` are `TRUE` as in Excel (the
  committed oracle cases `curated:arithmetic:c004` and `c015`), `=1=1+4E-15` and
  `=POWER(10,15)+1=POWER(10,15)` are `TRUE`, and `=1+4E-15=1+6E-15` and `=5=5+5E-15` are `FALSE`
  (Mac Excel 16.114). Numbers of different signs and a zero compare exactly. `<=` is "less or
  equal", `<>` is "not equal", so `=0.1+0.2>0.3` is `FALSE`. Comparing with zero stays exact; only
  the correction of a formula's root or an aggregate's total makes a residue zero.
- Sorting, `MATCH`, `VLOOKUP`, `HLOOKUP`, `XLOOKUP`, `LOOKUP`, `UNIQUE`, `GROUPBY`, and `MODE`
  keep exact order and equality.
- `ROUND`, `ROUNDUP`, `ROUNDDOWN`, `TRUNC`, `INT`, `MROUND`, `CEILING`, `FLOOR`, their `.MATH` and
  `.PRECISE` forms, `ISO.CEILING`, and `PERCENTRANK` read the scaled quotient to fifteen
  significant digits, a tie toward zero, before rounding it, so `ROUND(1.005,2)` is `1.01` and `ROUNDDOWN(1.15,2)`
  is `1.15`. The product is not snapped again: `FLOOR(0.3,0.1)` is three units of `0.1`,
  `0.30000000000000004`, which Excel also returns. A place past the fifteenth significant digit
  leaves the value unchanged (`ROUND(1,400)` is `1`, and `ROUND(1.23456789012345E+20,-2)` keeps
  every bit).
- `TEXT`, `FIXED`, and `DOLLAR` round the number's fifteen-significant-digit decimal, scaled
  exactly by `%` and thousands commas, and show digits past the fifteenth as zeros:
  `TEXT(1.005,"0.00")` and `FIXED(1.005,2)` are `1.01`, `TEXT(0.285,"0%")` is `29%`, and
  `TEXT(2^60,"0")` is `1152921504606850000` (Mac Excel 16.114). The display text of a cell rounds
  the same way, since it has no calculation options.

Under `Ieee754` comparison is exact and rounding uses the binary quotient, so `ROUND(1.005,2)` is
`1`, `TEXT(1.005,"0.00")` is `1.00`, and `TEXT(2^60,"0")` is `1152921504606846976`, the exact
decimal of the binary value.

### Matching: numbers in formulas and numbers joined to text

Both rules hold under either `ArithmeticSemantics`, because they decide what a number is written
as rather than how it is calculated.

- A number written in a formula keeps fifteen significant digits, and the digits after them become
  zeros, as Excel reads it: `1234567890123456` is `1234567890123450` and `0.30000000000000004`
  is `0.3`.
- A number joined to text (`&`, `CONCAT`, `LEN`, and the other text arguments) is rounded to
  fifteen significant digits, a tie toward zero (`(1234567890123450+5)&""` is `1234567890123450`,
  Mac Excel 16.114). It is written as a plain decimal when that takes at most twenty
  characters besides the sign and in scientific notation otherwise, with a signed exponent of at
  least two digits: `2/3&""` is `0.666666666666667`, `1E+19&""` is `10000000000000000000`,
  `1E+20&""` is `1E+20`, `1.5E-17&""` is `0.000000000000000015`, and `PI()*1E-5&""` is
  `3.14159265358979E-05`, as in Excel 16.0 build 20326.

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

The F, t, Z-test, and sample-covariance names added in 0.1.13 are measured
the same way. Their 60 active cases, again three per official name, are all classified `match` in
both frozen host profiles. The distribution grid additionally pins large degrees of freedom,
inverse round trips, and lower, upper, and two-tailed probabilities. Dedicated regression tests
cover representable subnormal F tails and extreme finite samples whose intermediate variances or
variance ratios would overflow without scaling.

A family's cumulative and quantile forms are evaluated through one shared tail kernel, so they
cannot drift apart from each other; the density forms are built on the same `ln_gamma` and
`ln_beta` primitives. Every kernel is first-party; no external special-function crate is linked.

### Matching: 0.1.14 engineering functions

The nineteen `CONVERT`, Bessel, and complex-text names added in 0.1.14 are measured against 33
active nominal and coercion/domain cases in both frozen host profiles. All 33 are classified
`match`. Complex worksheet results are compared as exact canonical text, including the `i`/`j`
suffix, sign, and omitted unit coefficient.

The eight finite Bessel worksheet cases use an absolute-plus-relative comparator of
`2e-7 + 5e-13 * max(abs(actual), abs(expected))`. This records compatibility with Excel's saved
values, some of which carry fewer significant digits or differ from the high-precision
mathematical function. It does not set the kernel accuracy target: independent 100-decimal-digit
reference literals cover 324 origin, transition, order-dominant, large-argument, near-root, and
parity cases under stricter magnitude-specific tolerances.

`CONVERT` preserves Excel's case-sensitive unit spelling and its documented affine temperature
semantics. The complex exponential, logarithm, power, division, product, magnitude, and square
root use scaled intermediate forms so a finite result is not lost solely because an intermediate
overflows. Non-finite worksheet results fail with the function's typed Excel error instead of
materializing NaN or infinity.

### Matching: 0.1.15 fixed-income functions

The twenty-six fixed-income functions added on top of 0.1.14 are measured against 78 active
nominal and coercion/domain cases in both frozen host profiles, plus the two curated composite
cases they unblock. All 80 are classified `match`. Day counts, coupon schedule dates, and error
codes are compared exactly; price, duration, and yield results use the default scale-relative
comparator.

Day counts are US 30/360 (basis 0), Actual/Actual (1), Actual/360 (2), Actual/365 (3), and
European 30/360 (4). Coupon schedules walk backward from maturity by `12 / frequency` months and
preserve month-end policy. `YIELD(N>1)` and `ODDFYIELD` use a safeguarded Newton/bracket root
solver with a 100-evaluation budget in Excel-compatible mode. Its `1e-7` tolerance applies both
to a candidate step and to the absolute price residual; a small step is accepted only after a
separate residual evaluation also passes. `ExtendedSearch` widens the budget to 1,000 evaluations
and tightens both checks to `1e-12`. `YIELD(N=1)` and `ODDLYIELD` are direct closed forms and never
run the solver.

Excel 16.111 worksheet observations require `settlement < maturity` for all three `TBILL*`
functions even though the published `TBILLEQ` and `TBILLPRICE` error tables only mention
`settlement > maturity`. A T-bill discount that would produce a non-positive price returns
`#NUM!`. `TBILLEQ` uses the simple quotation through 182 days and the bond-equivalent compound
branch from 183 days; worksheet literals pin 182, 183, 365, and 366 actual days.

`ACCRINTM` also distinguishes syntax that the published argument table does not make obvious:
the three-argument form with an absent `par` returns zero, while an explicit missing fourth
argument defaults `par` to 1000. Both forms are frozen in the semantic snapshot and worksheet
contract tests.

Independent literal fixtures pin the one-coupon `PRICE`/`YIELD` branches, `PRICEMAT`/`YIELDMAT`,
odd-last price/yield fractions, and both short- and long-first `ODDFPRICE` prices. The long-first
literal independently reduces the quasi-coupon cash flows with 70-digit decimal arithmetic, so no
expected value is generated by reusing the production kernel on both sides.

### Matching: 0.1.16 lookup and calendar functions

`XLOOKUP`, `DATEVALUE`, `TIMEVALUE`, `NETWORKDAYS.INTL`, and `WORKDAY.INTL` are measured against
13 active nominal, coercion, and domain cases in each frozen host profile. All 13 are classified
`match` in both profiles. The observations cover `XLOOKUP` array shape and lookup modes, fixed
date/time grammars, weekend masks, holiday ranges, and forward and reverse workday movement.

### Matching: 0.1.20 functions

The sixteen names added in 0.1.20 match all 44 function-classified active cases in both frozen
CellRune host profiles, plus one related curated `NUMBERVALUE` case. The six previously unsupported
`MDETERM` cases in the POI matrix fixture also match. These observations cover the recorded inputs;
they do not establish every function's full domain as Excel-equivalent.

`NORM.INV` and `NORM.S.INV` require `0 < probability < 1`; `NORM.INV` also requires positive
standard deviation.
Their shared quantile solves a centered erf residual near the median and a logarithmic smaller-tail
residual elsewhere. The far-tail branch uses the [Laplace continued fraction](https://dlmf.nist.gov/7.9.E1)
without exponentiating the tail. Independent 400-digit reference literals cover eighteen central,
branch-boundary, and extreme points down to the smallest positive `f64` subnormal; monotonicity is
checked across all binary tail exponents and a central probability grid. Every refinement step
charges work and observes cancellation, with at most 2,640 callback charges. A failed solve or
non-finite normal result returns `#NUM!`.

The lognormal CDF uses the existing normal lower-tail kernel at `(ln(x) - mean) / standard_dev`;
its density is assembled in log space. Lognormal functions require positive `x` (distribution)
and positive standard deviation. The inverse exponentiates the scaled normal quantile: overflow
returns `#NUM!`, and underflow returns zero. These mathematical tail checks are independent of the
saved Excel comparisons and do not impose Excel's narrower numeric input/display conventions.

`MDETERM` uses partial-pivot LU elimination with row-swap parity and a normalized diagonal product.
It returns zero on an exact zero pivot, retaining tiny nonzero determinants that `MINVERSE` rejects
under its conditioning threshold. Intermediate non-finite elimination or final overflow returns
`#NUM!`; final underflow returns zero. It does not compute an inverse to obtain a determinant.

`DDB` uses a direct declining book-value formula with a salvage floor. A period below one is treated
as the first period; for positive life below one, depreciation is cost minus salvage. Fractional
periods above one use the continuous declining-balance formula. The saved cases and separate
boundary literals cover ordinary periods and fractional life, rather than every Excel fractional
period convention. `XNPV` sums each cash flow discounted by actual elapsed days divided by 365.
It shares XIRR's strict numeric/date validation but does not require both positive and negative
flows. Rate `-1` is invalid; a rate below `-1` is accepted only when the resulting powers and sum
remain finite. Neither function uses an iterative financial solver.

### Matching: 0.1.22 functions

The nineteen names added in 0.1.22 and the legacy names `CHIDIST`, `WEIBULL`, and `CONFIDENCE`
match all 62 function-classified active cases in both frozen CellRune host profiles. Six existing
cases previously recorded as divergent, including the `CHAR`, `IRR`, and `MATCH` coercion cases,
also match after the 0.1.22 fixes. These observations cover the recorded inputs; they do not
establish every function's full domain as Excel-equivalent.

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

`CHISQ.INV`, `CHISQ.INV.RT`, and `CHIINV` take the incomplete gamma of shapes from `1e4`
(20,000 degrees of freedom) from a Gauss–Legendre quadrature of the integrand around its mean,
on panels narrowed to its steepness, so their quantiles keep their digits up to Excel's `1e10`
degrees of freedom; the series and continued fraction above would need about `√a` steps there.
Excel 16.114 itself returns `#NUM!` for some quantiles near that limit, such as
`CHISQ.INV.RT(0.05,1E10)`, `CHISQ.INV.RT(0.05,9.9E9)`, and `CHISQ.INV(0.95,1E10)`, while it
calculates `CHISQ.INV.RT(0.05,5E9)` and `CHISQ.INV.RT(0.1,1E10)`; CellRune returns the quantile
for all of them. `CHISQ.DIST`, `CHISQ.DIST.RT`, `GAMMA.DIST`, and `GAMMA.INV` keep the series and
continued fraction, whose results these releases do not change.

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

The current engine audit of the frozen 0.1.8 workbooks records:

| Workbook | Selected results | Match | Divergent | Not implemented | Host unsupported | Excluded |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Apache POI formula fixture | 1,295 | 1,290 | 5 | 0 | 0 | 0 |
| Apache POI matrix fixture | 266 | 250 | 16 | 0 | 0 | 0 |
| CellRune formula oracle — Excel Online | 1,496 | 1,487 | 2 | 7 | 0 | 0 |
| CellRune formula oracle — Mac Excel 2021 | 1,496 | 1,462 | 4 | 7 | 23 | 0 |

`match` uses each case's reviewed comparator: finite numbers default to a scale-relative `1e-8`,
while cancellation and signed-zero probes use exact bits. `divergent` records and enforces both
the Excel value and the current CellRune value with an explanatory note. The other states make
unsupported or non-comparable cases explicit rather than dropping them from the denominator.
These counts are an audit inventory, not a composite score or release threshold.

The 0.1.8 workbook has 1,527 primary cases, 1,496 active cases, and 1,892 formula cells. Excel Online
and Mac Excel 2021 both saved the workbook without losing formulas. A host that stores no usable
value for a case is recorded as `host_unsupported`. The three PIVOTBY cases retain semantic empty
anchors and complete array results, and all three match on both hosts. The audit calculates each
host with its locale conventions: Korean regional settings, with Korean total labels for Excel Online
(`합계`) and English ones for Mac Excel 2021 (`Total`). Missing host values do not require regenerating
the workbook or block publication.

## Unverified

The following families have unit and literal regression tests derived from Microsoft's primary
documentation, but no recorded tolerance against the reference oracle. Do not read their absence
from the verified section as a claim of exactness in either direction.

- statistical distributions and inverse distributions outside the gamma, beta, binomial,
  hypergeometric, F, t, normal, and lognormal families measured in 0.1.12, 0.1.13, and 0.1.20,
  and the chi-squared and Weibull distributions measured in 0.1.22
- closed-form financial functions outside the recorded DDB and XNPV cases
- transcendental math and trigonometric functions, which are evaluated through `libm`
- engineering functions outside the measured 0.1.14 `CONVERT`, Bessel, and complex-text families
  and the 0.1.22 `IMLOG10` and `IMLOG2`

Priority for measurement is statistical, then closed-form financial, then math.

## Reporting a difference

Open an issue with the formula text, the input values, the CellRune result, and the Excel result
together with the Excel build that produced it. Excel's answer can differ between builds, so the
build matters.
