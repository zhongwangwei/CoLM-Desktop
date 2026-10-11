"""Launch the bundled CoLM native programs without changing their arguments."""
import os
from pathlib import Path
import subprocess
import sys


def resources():
    """Directory containing the native binaries, source archive and documents."""
    return Path(__file__).resolve().parent / "_native" / "bin"


def _launch(name):
    suffix = ".exe" if os.name == "nt" else ""
    executable = resources() / (name + suffix)
    argv = [str(executable), *sys.argv[1:]]
    if os.name != "nt":
        return os.execv(str(executable), argv)
    # Windows has no POSIX exec replacement. The child shares the console and
    # receives Ctrl+C itself; wait for its exit rather than killing it early.
    child = subprocess.Popen(argv)
    while True:
        try:
            raise SystemExit(child.wait())
        except KeyboardInterrupt:
            continue


def agent():
    _launch("colm-agent")


def cli():
    _launch("colm-cli")


def mcp():
    _launch("colm-mcp")


def surface():
    _launch("mksrfdata-rs")


def initialize():
    _launch("mkinidata-rs")


def runtime():
    _launch("colm-rs")
