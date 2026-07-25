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
(cd bindings/python && python tests/conformance.py && python tests/interactive.py && python tests/introspection.py && python tests/responsiveness.py && python tests/typing_check.py && mypy --strict tests/typing_check.py)
(cd bindings/node && pnpm install --frozen-lockfile && pnpm build && pnpm typecheck && pnpm test)
```

The release workflows are the normative definition for target-specific wheels, npm platform
packages, MCP bundles, license notices, offline consumers, and provenance.

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
