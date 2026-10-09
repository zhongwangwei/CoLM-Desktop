# Knowledge bindings and source updates

The curated `colm-process-knowledge.md` cards are bound to the raw SHA-256 contents of the document and every explicitly cited source file. The baseline is `docs/knowledge-sources.json` (schema 1). A matching hash establishes that the reviewed bytes are unchanged; it does not validate scientific causality, an installed kernel's behavior, or a particular run.

`bound_at_unix_ms` is the UTC time the baseline was explicitly recorded. It is not the time a source was changed. Source history remains in Git. Documents without an explicit binding remain unbound; they must not be described as source-verified simply because their text was retrieved.

## Runtime retrieval

`search_docs` checks matching documents against the current source bytes on each search. Its `knowledge_checks` report one of four states:

- `current`: the document and every bound source hash match the baseline.
- `needs_review`: the document or a cited source changed, or a cited file is unavailable. Stored assertions are withheld from the search results.
- `unbound`: the document has no explicit baseline and cannot establish current-version behavior.
- `unknown`: verification could not be completed, for example because the manifest is malformed or source identity is unavailable.

For changed, readable sources, the same response includes freshly read, bounded source excerpts, current hashes, and `read_file` follow-ups. These excerpts locate current evidence; inspect the complete relevant formula and switches before answering. A search does not update the baseline or automatically certify the explanation. Source excerpts and result counts are bounded, so follow-up retrieval may be required.

Application source identity and knowledge bindings describe the application's source snapshot. They do not identify the selected simulation kernel or prove it implements the same branches. Establish the run's kernel manifest, macros, and case configuration separately.

## Maintaining the cards

1. Check the current baseline without changing it:

   ```sh
   python3 scripts/bind-knowledge.py --check
   ```

   `matched` means all bound bytes match. Exit status 1 / `needs_review` identifies changed source files or document content. Missing sources, unsafe paths, invalid manifests and other check failures return status 2 and require investigation.

2. Review changes in the cited sources, including the applicable branch, formula, units and switches. Correct the cards where necessary and retain concrete source citations. Re-read current source before presenting a changed card's explanation as applicable to the current version. A discrepancy is a request for review, not evidence that the old statement is wrong or that the new code is scientifically correct.
3. After that review, explicitly record the new baseline:

   ```sh
   python3 scripts/bind-knowledge.py --reviewed
   ```

   This command replaces the baseline atomically and records the binding time. It extracts the cards' backtick-quoted `crates/...` and `vendor/CoLM202X/...` source citations, strips any `::symbol` suffix, and hashes the full files and document. It rejects missing files, symlinks and paths outside the supported source roots. Review the manifest diff alongside the source and documentation changes and commit them together.

Builds and searches must never run `--reviewed` automatically. Doing so would relabel unchecked content as current. Installation packages must ship the existing baseline and matching documents with the corresponding source snapshot; any mismatch remains visible as requiring review.

This is a per-document binding: one changed cited file marks the entire card document for review. Dependency lists include explicit citations only; adding an indirectly relevant source requires citing it in the cards and reviewing the resulting binding. The real-task evaluation collection remains deferred.
