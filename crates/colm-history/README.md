# colm-history

A NetCDF-free, append-only SQLite operation ledger. `History::open_default()` uses
`CoLM-Desktop/history.sqlite3` under the OS application-data directory. Use
`History::open(path)` for explicit databases and `:memory:` for unit tests.
CLI and GUI should both use `default_db_path()` (or the same explicitly selected
path); do not substitute a different Tauri application-data root.

Persist `Event::start(kind)` before performing the action. Persist
`start.next(EventPhase::Finished, "succeeded" | "failed" | "cancelled")` after
observing the outcome. Preserve the operation ID for retries of ledger writes;
never retry execution merely because a terminal event is missing. `Event::next`
copies the metadata snapshot: case ID/path, version/revision, host/location,
initiator, parent operation, command and JSON detail. Callers must propagate
append failures; the ledger cannot prove execution succeeded when writing fails.

`query(&Query)` returns aggregate operations with filters, deterministic ascending
or descending order, offset and limit (maximum 10,000). Unknown times remain null.
Unfinished operations have `needs_reconciliation=true`: inspect real local
processes or remote scheduler jobs before offering restart. `detail(id)` returns
all immutable events and up to 100 related operations (parent, children, siblings,
same case). `export_csv` / `export_json` export **all matching records**, ignoring
UI `offset` and `limit` while preserving filters and sort direction. Each export
reads one consistent SQLite snapshot in batches of 1,000. Only the final returned
string grows with output size. CSV protects text cells against spreadsheet
formulas while preserving typed numeric fields.

`import_case` reads `.colm/runs.jsonl`, `.colm-remote.json`, and immediate
`out/*/produced-by.json` files. `import_study` reads the manifest, state checkpoint
generations, and `.colm-remote-study.json`. `import_file` also supports workspace
`gates` objects or standalone gate collections. Import source files are never
modified. Each file is parsed before its transaction starts; malformed input is
reported, not silently skipped. Content hashes deduplicate legacy records,
including the same provenance in runs.jsonl and produced-by.json. A Study's ID
links its checkpoint events and generation ordering prevents old snapshots from
rewinding its state. Checkpoint payload hashes are verified; legacy checkpoints
whose hash used typed-field order use the existing Study reader's verification
and then `import_verified_record(&json!({"sequence": loaded.sequence,
"payload": loaded.payload}), ImportKind::Study, stable_source_key, None)`.
That API deliberately trusts its caller's prior validation, preserves checkpoint
generation, and applies the same sanitization and stable-content deduplication. Missing historical fields are never fabricated. Remote submission
provenance is not completion evidence. Imported paths are explicitly marked as
requiring input identity verification before reproduction; the ledger never
assumes that a file at the same path still has identical content.

Commands are stored as `Command { program, argv, cwd }`, never shell strings.
Credential options, secret environment assignments, authenticated URLs, and shell
command strings are redacted. Detail sanitization uses structured key boundaries,
not text regexes: credential keys, environment maps, raw output and unstructured
commands are omitted, while scientific text and numbers are preserved. Callers
must supply non-sensitive free-form prose and executable/argument semantics;
this API cannot identify an arbitrary secret disguised as a scientific value.
Large logs, manifests and fingerprint tables belong in files referenced by path;
events over 1 MiB fail rather than truncating evidence. Study member/task arrays
are not copied into the ledger. Authentication records retain only login method.

SQLite uses WAL, FULL synchronization, a five-second busy timeout, atomic event
batches, uniqueness constraints and UPDATE/DELETE rejection triggers. Event ID
reuse for changed content fails. Separate processes open their own connections.

Dependency decision: rusqlite 0.32 with bundled SQLite maintains compatibility
with the workspace's Rust 1.85.1 contract. Current upstream explicitly requires
Rust 1.88, so upgrading is a separate workspace MSRV decision. References:
https://github.com/rusqlite/rusqlite/blob/v0.32.1/README.md and
https://github.com/rusqlite/rusqlite/blob/master/Cargo.toml.

Validation is intentionally deferred to the user's centralized verification
stage: `cargo test -p colm-history`, `cargo clippy -p colm-history --all-targets`,
and the actual MSRV/dependency build. Only rustfmt parsing has been run during
implementation.
