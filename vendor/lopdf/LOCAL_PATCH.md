# lopdf 0.42.0: Rust 1.85 syntax backport

Published source: https://static.crates.io/crates/lopdf/lopdf-0.42.0.crate

Archive SHA256: `25aab26d99567469098e64a02f42679f8965c6401263eefa31d8f2dcc37a221c`.
The original MIT license is preserved in `LICENSE`. `UPSTREAM_FILES.sha256` records every
file from that verified published archive.

Only `src/object.rs` changes: the BaseEncoding lookup uses nested `if let` rather than
let chains, preserving its result and error propagation on Rust 1.85.1. The upstream
manifest declares Rust 1.85 but its let-chain syntax requires a later compiler.

Do not downgrade below 0.42 to solve this: 0.42 retains the nesting-depth security fix
for [RUSTSEC-2026-0187](https://rustsec.org/advisories/RUSTSEC-2026-0187.html).
The workspace MSRV test verifies every upstream file, reversing only this exact syntax
change before hashing. Remove the local patch when an upstream release both preserves
the security fix and passes the declared minimum compiler.
