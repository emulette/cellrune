# Contributing

Contributions are welcome. Keep changes focused, preserve the public error and calculation
contracts, and add deterministic tests for behavior changes.

## Checks

The repository pins its Rust toolchain in `rust-toolchain.toml`. Install `cargo-deny` separately,
then run:

```bash
cargo fmt --all --check
cargo fmt --manifest-path fuzz/Cargo.toml -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo clippy --package cellrune --lib --all-features --locked -- -D warnings -D clippy::missing_errors_doc -D clippy::missing_panics_doc -D clippy::doc_markdown
cargo test --workspace --lib --bins --tests --examples --all-features --locked
cargo deny --all-features check
cargo deny --manifest-path fuzz/Cargo.toml --config fuzz/deny.toml check
RUSTFLAGS="-D warnings" cargo +1.88.0 check --workspace --lib --all-features --locked
cargo package --locked -p cellrune
cargo run --locked --manifest-path release-tests/package-consumer/Cargo.toml
python3 bindings/scripts/test_verify_release_artifacts.py
```

Run `cargo package` before the standalone consumer because the consumer compiles against Cargo's
extracted package rather than the workspace source tree.

Python build and audit inputs use checked-in universal hash locks. Node development dependencies
use `pnpm-lock.yaml`. In an activated Python 3.10–3.14 virtual environment, build both native
bindings and test them without updating either dependency graph:

```bash
python -m pip install --require-hashes -r bindings/python/requirements-dev.txt
(cd bindings/python && maturin develop --release --locked)
(cd bindings/python && python tests/binding_contract.py && python tests/interactive.py && python tests/introspection.py && python tests/responsiveness.py && python tests/typing_check.py && mypy --strict tests/typing_check.py)
(cd bindings/node && pnpm install --frozen-lockfile && pnpm build && pnpm typecheck && pnpm test)
```

The release workflows are the normative definition for target-specific wheels, npm platform
packages, MCP bundles, license notices, offline consumers, and provenance.

## Dependency requirements

External Rust requirements for workspace members are defined once in `[workspace.dependencies]`
in the root `Cargo.toml`. A version there is the lowest supported dependency, not the version used
for reproducible builds; the checked-in `Cargo.lock` records the latter.

A dependency floor must satisfy all of these conditions:

- every API and feature CellRune uses is present;
- the full workspace suite passes on Rust 1.88 with the direct dependency minimized;
- the resolved floor graph passes the repository's advisory policy; and
- every version admitted by a cross-major range uses the same tested API and behavior boundary.

Security can therefore keep a floor above the first version that merely compiles. Keep an explicit
upper bound when CellRune intentionally spans multiple semver-incompatible release lines, and
never span more families than the floor and newest-compatible suites actually resolve — a family
in the middle of a range would be admitted untested. A dependency whose types appear in
CellRune's public signatures must stay within one semver family: a multi-major range would let a
dependent's resolver hand CellRune a different copy of that type than the dependent's own. Do not
replace a floor with the current release during routine upgrades; update `Cargo.lock` instead.
Test a proposed floor in a temporary worktree with:

```bash
cargo +nightly-2026-07-22 -Z direct-minimal-versions generate-lockfile
RUSTUP_TOOLCHAIN=1.88.0 RUSTFLAGS="-D warnings" cargo test --workspace --lib --bins --tests --examples --all-features
cargo deny --all-features check
```

Exact versions remain appropriate for CellRune crates and npm platform packages that are released
in lockstep. Python hash locks, the Python build backend, npm development tools, and their lockfiles
are reproducible tool inputs rather than downstream compatibility requirements; update and verify
those intentionally instead of treating them as dependency floors.

## Requirements

- Keep source, comments, errors, user-facing documents, and commit messages in English.
- Use non-English test values only when they validate Unicode or localization behavior.
- Keep XLSX and formula support claims precise; do not imply complete Excel compatibility.
- Do not commit workbook binaries, customer data, personal metadata, secrets, or third-party
  workbook content.
- Use deterministic generated packages for public reader tests.
- Validate every dependency against the repository policy.
- Keep the packaged license and third-party notice identical to their repository copies.

Unless stated otherwise, contributions are licensed under the MIT License. By submitting a
contribution, you confirm that you have the right to provide it under that license. CellRune does
not require a separate contributor license agreement or commit sign-off.
