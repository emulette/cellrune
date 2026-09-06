# Targeted calculation measurements for 0.1.18

Run `cargo bench -p cellrune-integration-tests --bench targeted_calculation --locked`.

The workload has 10,000 rows: literal `A[row]`, formula `B[row] = A[row] + 1`, and
formula `C[row] = B[row] * 2`, for 20,000 formulas. Each request selects a rectangle
in column C. First partial requests and full calculations use independently constructed
fresh sessions. Timings include preparation, initial source identity, evaluation, and result
construction, and exclude workbook construction and file I/O. The cached request uses a current
complete calculation. Every returned value is compared with full calculation outside the timer.

Measured on 2026-09-06, Windows x64, Intel Core Ultra 7 155H, Rust 1.92.0,
optimized Cargo bench profile. Values are medians of five samples on a local development machine.

| Requested output cells | Full calculation (ms) | First partial request (ms) | Current-cache partial request (ms) | Partial evaluator invocations |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 231.337 | 8.502 | 0.011 | 2 |
| 100 | 237.435 | 11.719 | 0.053 | 200 |
| 10,000 | 218.132 | 221.554 | 4.055 | 20,000 |

Small output scopes avoid most formula parsing and evaluation. When the requested precedents
cover all formulas, this workload has comparable cost to full calculation; targeted calculation
does not promise a speedup for complete scopes. Workbook layout inspection still scales with
source metadata. These measurements cover this synthetic core/session workload, not XLSX input,
binding transport, MCP serialization, peak memory, or a general performance threshold.

The implementation was checked with the workspace test suite (962 passed, two existing external
corpus tests ignored), workspace Clippy with warnings denied, Node runtime/type/package-consumer
checks, Python runtime/type checks and a Windows CPython 3.14 release wheel, and a freshly packaged
Rust consumer. Cross-platform release builds remain the existing CI responsibility. The frozen
Excel Oracle fixtures and observations were unchanged.
