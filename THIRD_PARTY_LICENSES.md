# Third-party licenses

CellRune is distributed under the MIT OR Apache-2.0 licenses. Its Cargo
dependencies remain under their own licenses; this notice does not relicense
them.

## Runtime dependency graph

The following table records the normal dependency graph selected by
`cargo tree --locked -p cellrune --edges normal` for CellRune 0.1.5.
Transitive dependencies are included so the release boundary can be audited
without confusing development-only tools with shipped library requirements.

| Crate | Version | SPDX expression |
| --- | --- | --- |
| `block-buffer` | 0.12.1 | `MIT OR Apache-2.0` |
| `cfg-if` | 1.0.4 | `MIT OR Apache-2.0` |
| `cpufeatures` | 0.3.0 | `MIT OR Apache-2.0` |
| `crc32fast` | 1.5.0 | `MIT OR Apache-2.0` |
| `crypto-common` | 0.2.2 | `MIT OR Apache-2.0` |
| `digest` | 0.11.3 | `MIT OR Apache-2.0` |
| `equivalent` | 1.0.2 | `Apache-2.0 OR MIT` |
| `flate2` | 1.1.9 | `MIT OR Apache-2.0` |
| `hashbrown` | 0.17.1 | `MIT OR Apache-2.0` |
| `hybrid-array` | 0.4.13 | `MIT OR Apache-2.0` |
| `indexmap` | 2.14.0 | `Apache-2.0 OR MIT` |
| `libc` | 0.2.189 | `MIT OR Apache-2.0` |
| `libm` | 0.2.16 | `MIT` |
| `memchr` | 2.8.3 | `Unlicense OR MIT` |
| `quick-xml` | 0.41.0 | `MIT` |
| `sha2` | 0.11.0 | `MIT OR Apache-2.0` |
| `typenum` | 1.20.1 | `MIT OR Apache-2.0` |
| `typed-path` | 0.12.3 | `MIT OR Apache-2.0` |
| `zip` | 8.6.0 | `MIT` |
| `zlib-rs` | 0.6.6 | `Zlib` |

`libm`, `quick-xml`, `sha2`, and `zip` are CellRune's direct runtime dependencies.
The remaining crates are selected transitively by those dependencies.

The table covers the default feature set. The optional `capability-fs` feature adds
`cap-std` and its transitive graph, which this table does not enumerate. Those crates
are permissive and are license-gated in CI, because `deny.toml` sets `all-features = true`.
List them with `cargo tree --locked -p cellrune --features capability-fs --edges normal`.

## Development dependencies

`calamine` is used only for differential development audits. `serde` and
`serde_json` are used only by tests and evidence tooling. They are not linked
into CellRune's normal library dependency graph. The complete locked graph,
including all transitive development dependencies, is checked by
`cargo-deny` against the repository's license, advisory, source, and
duplicate-version policy.

```bash
cargo tree --locked -p cellrune --edges normal
cargo deny list --layout crate
cargo deny check
```

The `.crate` source archive contains CellRune's source, both of its license
texts, and this notice, not copies of dependency source trees. Cargo obtains
each dependency separately with that crate's own license files and registry
metadata.

Before distributing an executable or another artifact that bundles or
statically links third-party software:

1. derive the exact component list and SPDX expressions from the final locked
   release graph;
2. include every license text and attribution required by those components;
3. update this notice in the same change; and
4. rerun `cargo deny check` against the final lockfile and feature set.
