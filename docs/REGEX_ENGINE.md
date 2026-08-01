# Regular expression engine

CellRune's `REGEXEXTRACT`, `REGEXREPLACE`, and `REGEXTEST` functions use PCRE2 through
`pcre2 0.2.11` and `pcre2-sys 0.2.10`. The locked `pcre2-sys` release carries the bundled
PCRE2 10.46 source. The adapter enables UTF and Unicode-properties modes and leaves JIT disabled,
so look-around, backreferences, and capture numbering have one native implementation across the
supported release targets. CellRune expands PCRE2 10.46's default replacement forms `$$`, `$&`,
the prefix token (a dollar sign followed by a grave accent), `$'`, `$_`, `$n`, `${n}`, `${name}`,
and `$<name>` from the captured native spans. The
`${*MARK}` form is excluded because the safe Rust wrapper does not expose PCRE2 match marks;
extended-substitution mode is not enabled. Global matching also rejects a root-recursive pattern
(`(?R)` or a numeric-zero `(?0)`, `\g<0>`, or `\g'0'` form, including leading zeroes) if an
empty match requires PCRE2's anchored non-empty retry, because the safe wrapper does not expose
the native match-option combination needed to preserve root identity.

## Toolchain and targets

The dependency graph builds with CellRune's Rust 1.88 MSRV. Native release validation covers
both Apple architectures, both Windows MSVC architectures, and the GNU and musl variants of both
Linux architectures listed in `deny.toml`.

Repository builds set `PCRE2_SYS_STATIC=1` through `.cargo/config.toml`. This makes wheels, Node
packages, the MCP executable, and other repository-built native artifacts use the bundled static
PCRE2 implementation instead of acquiring an undeclared runtime dependency on a host library.
Rust consumers compiling the published `cellrune` crate own their final linkage and may explicitly
override that environment setting to select a compatible system PCRE2.

PCRE2 is a native C dependency. CellRune does not currently declare a WebAssembly release target;
adding one requires a separately verified PCRE2 toolchain and does not permit a reduced function
catalog or a different regular-expression engine.

`pcre2` and `pcre2-sys` are available under `Unlicense OR MIT`. The bundled PCRE2 source carries
its BSD-3-Clause notice, and its linked SLJIT implementation carries a separate BSD-style notice
from Zoltan Herczeg. Generated native-package notices include both version-gated upstream notices,
the exact locked component list, and all discovered license files; `cargo deny` admits the
corresponding SPDX expressions.

## Execution bounds

The shared adapter charges pattern and input bytes to the calculation budgets. Matching starts with
a small native-work tier and retries match-limit failures at geometrically increasing, capped tiers;
the full allowance of every attempt is charged before the call, so ordinary linear expressions are
not constrained by a worst-case global-call estimate while repeated pathological work stays cumulative.
PCRE2 depth and heap limits are also bounded. A conservative pattern-by-input scan allowance covers
deterministic native work that PCRE2's backtracking counter does not observe. Plain case-sensitive
literal patterns with default start options use linear pattern and input charges instead. Capture
bookkeeping and the aggregate bytes copied into extracted arrays are charged before allocation.
Cancellation is polled before and after native
calls and between matches. Global matching follows PCRE2's empty-match rule by retrying a
non-empty anchored match at the same UTF-8 boundary before advancing one character. Extracted
arrays use the array-cell limit, and every constructed text result is checked against the text-byte
limit while it is built.
