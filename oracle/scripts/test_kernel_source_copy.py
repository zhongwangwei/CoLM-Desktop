#!/usr/bin/env python3
"""Exercise the real build script's source copy without compiling a kernel."""
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]


def main():
    source = (ROOT / "oracle/scripts/build_kernel.sh").read_text()
    # Stop after the actual archive pipeline, before compiler configuration.
    prefix = source.split("trap 'rm -rf \"$BUILD\"' EXIT", 1)[0]
    assert prefix != source, "build copy boundary changed"
    with tempfile.TemporaryDirectory(prefix="colm-copy-") as temp:
        root = Path(temp)
        script = root / "oracle/scripts/build_kernel.sh"
        script.parent.mkdir(parents=True)
        script.write_text(prefix)
        for helper in ['ensure_native_kernels.py', 'kernel-presets.config']:
            (script.parent / helper).write_bytes((ROOT / 'oracle/scripts' / helper).read_bytes())
        vendor = root / "vendor/CoLM202X"
        kept = ["share/model.F90", "share/native.c", "include/options", "Makefile"]
        stale = [".git/config", ".bld/CoLM_Mkdir.o", ".bld/Makefile", "lib/model.a",
                 "share/old.o", "share/old.mod", "share/old.smod", "run/colm.x",
                 "run/colm.exe", "run/old.so", "run/old.so.1", "run/old.dylib",
                 "run/old.dll", "share/old.a"]
        for name in kept + stale:
            path = vendor / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(name)
        (vendor / "include/Makeoptions").symlink_to("options")
        subprocess.run(["bash", str(script), "default", "kernels"], check=True)
        build = root / "kernels/build-default"
        for name in kept:
            assert (build / name).read_text() == name, name
        for name in stale:
            assert not (build / name).exists(), f"stale artifact copied: {name}"
        assert not (build / "include/Makeoptions").is_symlink()
        assert (build / "include/Makeoptions").read_text() == "include/options"
    print("PASS: source preserved, symlink dereferenced, build products excluded")


if __name__ == "__main__":
    main()
