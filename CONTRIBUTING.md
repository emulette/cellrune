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

CI and release gates are deliberately few. A gate exists only to stop an irreversible failure —
a registry upload, a wrong-version tag — or to keep a stable validation boundary green. New
verification lands as ordinary tests, suite cases, or observational commands by default;
promoting anything to a required job is an explicit maintainer decision, never a side effect of
adding the verification.

## When CI runs what

Pull requests and pushes to `main` run source checks on Linux. The platform matrix and every
distributable artifact are built where they are published — on the release tag.

| Event | What runs |
| --- | --- |
| Pull request | `quality` and `msrv`, plus one Python and one Node smoke job when bindings are touched |
| Push to `main` | The above, plus macOS and Windows core tests and the packaged-crate consumer |
| `v*` tag | Everything, then publication |
| `Maintenance` dispatch | Whichever tiers you select — see below |

The Python and Node smoke jobs run the **declared floor** (Python 3.10, Node 22), not the newest
runtime. Breakage on a new runtime is caught by Dependabot and by the tag build; breakage on the
floor is caught by nothing else and would ship.

## Before tagging a release

Nothing here runs on a schedule. Four checks read inputs this repository does not control — the
advisory database, live crates.io resolution at both ends of the published version ranges, and the
hosted runner images — so they cannot be answered by a push tier and are not useful on a clock
either. Run them deliberately, from the **Maintenance** workflow's `Run workflow` button:

| Input | Run it when |
| --- | --- |
| `advisories` | Always. Cheap. |
| `latest_dependencies` | The release touched dependency requirements or the MSRV |
| `dependency_floor` | The release touched dependency requirements. Resolves every direct requirement to its declared floor, which no other job ever compiles |
| `fuzz` | The release touched the parser, the reader, the writer, or the session |
| `binding_artifacts` | The release touched anything cross-compiled. Builds the eight targets, the macOS deployment targets, and the musl containers — the things hosted-runner image drift breaks, and which nothing else builds until the tag itself |

A gate that has never been run has never passed. The fuzz tier was uninstallable from the first
commit and nobody knew, because its schedule had not fired since the repository went public; it
was found only by dispatching it manually before a tag. If you change one of these jobs, dispatch
it in the same commit.

Then check that `[workspace.package] version` in `Cargo.toml` matches the tag you intend to push —
`release.yml` rejects a tag that disagrees, but fixing it after the tag is more work than before.

## Benchmarks

The 50k-workbook benchmark is an observational command, not a CI job or a release blocker:

```bash
cargo bench --package cellrune-integration-tests --bench hardening --locked -- 50000 1
```

It reads, calculates, writes, and reopens a generated 50,000-row workbook and asserts formula
correctness internally, so a run that completes is a run that passed. Timing varies with runner
noise and input shape; record the number and environment in the pull request when a change is
expected to move it, and do not turn it into a gate.

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

## License

CellRune is dual-licensed under either the MIT License (`LICENSE-MIT`) or the Apache License,
Version 2.0 (`LICENSE-APACHE`), at the recipient's option.

Unless stated otherwise, a contribution you intentionally submit for inclusion in CellRune is
licensed under those same dual terms, with no additional terms or conditions. By submitting a
contribution, you confirm that you have the right to provide it under both licenses. CellRune does
not require a separate contributor license agreement or commit sign-off.

Every distributed artifact carries both license texts, so a new distribution boundary has to copy
both. The `Verify packaged license text` step in `ci.yml` enumerates the copies and compares each
one against the repository original.
