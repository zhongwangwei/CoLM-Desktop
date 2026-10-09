#!/usr/bin/env python3
"""Explicitly bind the curated knowledge cards to reviewed source bytes."""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import tempfile
import time

DOCUMENT = "colm-process-knowledge.md"
MANIFEST = "docs/knowledge-sources.json"


def regular_file(root, relative):
    parts = PurePosixPath(relative).parts
    if (
        not parts
        or PurePosixPath(relative).is_absolute()
        or "\\" in relative
        or "\x00" in relative
        or any(part in (".", "..") or part.startswith(".") for part in parts)
        or ":" in relative
    ):
        raise ValueError(f"unsafe path: {relative}")
    path = root
    for part in parts:
        path = path / part
        if path.is_symlink():
            raise ValueError(f"symlink is not allowed: {relative}")
    if not path.is_file():
        raise ValueError(f"missing regular file: {relative}")
    return path


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def document_binding(root):
    document = regular_file(root, f"docs/{DOCUMENT}")
    # Only explicit source citations are dependencies; symbols are not filenames.
    citations = re.findall(r"`((?:crates/|vendor/)[^`]+)`", document.read_text(encoding="utf-8"))
    sources = {}
    for citation in citations:
        relative = citation.split("::", 1)[0]
        if not (relative.startswith("crates/") or relative.startswith("vendor/CoLM202X/")):
            raise ValueError(f"source outside supported roots: {relative}")
        if PurePosixPath(relative).suffix.lower() not in (".rs", ".f90", ".f", ".h"):
            raise ValueError(f"not a supported source file: {relative}")
        sources[relative] = digest(regular_file(root, relative))
    if not sources:
        raise ValueError("document has no explicit source citations")
    return {"document_sha256": digest(document), "sources": dict(sorted(sources.items()))}


def bind(root):
    binding = document_binding(root)
    manifest = root / MANIFEST
    if manifest.is_symlink():
        raise ValueError("manifest must not be a symlink")
    result = {
        "schema": 1,
        "bound_at_unix_ms": time.time_ns() // 1_000_000,
        "documents": {DOCUMENT: binding},
    }
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", dir=manifest.parent, delete=False) as handle:
            temporary = Path(handle.name)
            json.dump(result, handle, indent=2, ensure_ascii=False)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, manifest)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)
    return result


def check(root):
    current = document_binding(root)
    baseline = json.loads(regular_file(root, MANIFEST).read_text(encoding="utf-8"))
    if not isinstance(baseline, dict) or baseline.get("schema") != 1:
        raise ValueError("unsupported manifest schema")
    previous = baseline["documents"][DOCUMENT]
    if (
        not isinstance(previous, dict)
        or not isinstance(previous.get("sources"), dict)
        or not re.fullmatch(r"[0-9a-f]{64}", str(previous.get("document_sha256", "")))
        or not previous["sources"]
        or any(not re.fullmatch(r"[0-9a-f]{64}", str(value)) for value in previous["sources"].values())
    ):
        raise ValueError("invalid document binding")
    changed = sorted(
        name for name in set(previous["sources"]) | set(current["sources"])
        if previous["sources"].get(name) != current["sources"].get(name)
    )
    document_changed = previous["document_sha256"] != current["document_sha256"]
    return {"status": "needs_review" if changed or document_changed else "matched",
            "document_changed": document_changed, "changed_sources": changed}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    action = parser.add_mutually_exclusive_group(required=True)
    action.add_argument("--reviewed", action="store_true", help="attest that the cards and their cited sources have been reviewed; replace the binding")
    action.add_argument("--check", action="store_true", help="check byte identities without modifying the binding")
    args = parser.parse_args()
    try:
        result = bind(args.root) if args.reviewed else check(args.root)
    except (OSError, ValueError, KeyError, TypeError) as error:
        parser.exit(2, f"knowledge binding failed: {error}\n")
    print(json.dumps(result, indent=2, ensure_ascii=False))
    return 1 if result.get("status") == "needs_review" else 0


if __name__ == "__main__":
    raise SystemExit(main())
