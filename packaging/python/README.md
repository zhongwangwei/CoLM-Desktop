# Python distribution — temporarily paused

As requested on 2026-10-10, pip distribution is paused. The wheel builder
refuses both command-line and direct `build()` calls before reading binaries
or writing output. No new wheel should be generated or published.

Use the native desktop app or `colm-agent chat` instead; their features remain
available. See [terminal usage](../../docs/terminal-assistant.md).

The Python launcher, builder implementation and tests are retained for a future
explicit decision to resume distribution. Existing installations and local
wheel artifacts are not removed. Nothing has been published to PyPI.

`PIP_DISTRIBUTION_ENABLED` in `scripts/build-python-wheel.py` is currently
`False`. Restoration requires changing this source setting and verifying native
binaries, matching source/documents and installation on each target platform.
There is no command-line or environment override.

Run the retained tests (the archived builder is enabled only inside its fixture):

```sh
python3 -m unittest discover -s packaging/python/tests -v
```
