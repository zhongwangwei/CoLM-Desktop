# colm-model

NetCDF-free model storage used by CLI and desktop integration. All public request
structures reject unknown fields. `Overlay.parameters` preserves the GUI's
catalog IDs, raw keys and explicit case/LCT/PFT/PC/process scopes. Values are NML
literals; fields are checked against existing schema and scientific validators.
`Overlay.physics` contains model controls, not case dates, sites or output paths.

`Registry::install_official` registers an immutable baseline from a verified
`CodeRef`; `commit: None` means unknown, never a fabricated Git identity. A changed
official source gets a new official revision. Draft creation inherits a fixed
parent revision and merges requested overlays. Saving replaces the complete
draft overlay with optimistic `generation` checking; this permits removing an
inherited override. `freeze(id, asset_root)` copies and checks assets and returns
an immutable revision. Unchanged content reuses the same revision; inherited and
previously frozen assets need not remain in the user's editing directory.

`migrate_case(case, code)` preserves all existing files, creating only
`model-binding.json` and `case-data.json`. It imports explicit model settings,
process files and network assets without assuming that an omitted setting once
had today's default. Repeated migration returns the original binding.
`rollback_migration` removes the sidecars and leaves historical revisions intact.
Model IDs derived from content make a rolled-back migration reusable.

`materialize_run(version, source_case, run_case, operation_id)` requires a new run
directory outside the source case and an existing parent directory. It copies
small configuration and climate files, removes recognized legacy model settings
from the run copy, and applies the frozen revision. Large scientific data stay
shared through absolute read-only input references. Outputs are directed into
the run directory. Original configuration, results and networks are never edited.
The manifest records the revision, code, source/effective NML hashes and copied
configuration identities. Engine/kernel resolution, macro compatibility, external
dataset identities and remote scheduling remain the CLI's responsibilities.

Registry writes and migrations use exclusive lock files, atomic JSON replacement
and staging-directory publication. A process crash may leave a lock or staging
directory; recovery must confirm that its recorded writer is no longer active.
Never remove another running writer's lock. Malformed/tampered revisions fail
closed rather than silently disappearing from the version list.

Focused checks (deferred to the requested final verification phase):

```sh
cargo test -p colm-model
cargo clippy -p colm-model --all-targets -- -D warnings
```
