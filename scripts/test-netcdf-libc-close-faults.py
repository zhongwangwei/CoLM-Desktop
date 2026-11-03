#!/usr/bin/env python3
"""Linux delayed-close ENOSPC/EIO tests against actual CLI and runtime writers.

Build the current binaries, then run:
  cargo build -p colm-cli --bin colm-cli
  cargo build -p colm-runtime --example netcdf-close-probe
  python3 scripts/test-netcdf-libc-close-faults.py --cli target/debug/colm-cli \
      --runtime-probe target/debug/examples/netcdf-close-probe

No mount, data download or shared filesystem changes. The shim only faults a named
file under this test's private temporary directory, after at least 4096 bytes exist.
"""
import argparse
import errno
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--runtime-probe", type=Path, required=True)
    opts = parser.parse_args()
    assert sys.platform == "linux", "LD_PRELOAD close regression requires Linux"
    cli, probe = opts.cli.resolve(), opts.runtime_probe.resolve()
    assert cli.is_file() and probe.is_file(), "build the current CLI and runtime probe first"
    root = Path(__file__).resolve().parents[1]
    checks = []
    with tempfile.TemporaryDirectory(prefix="colm-close-fault-") as temporary:
        temp = Path(temporary).resolve()
        library = temp / "close-fault.so"
        subprocess.run(["cc", "-shared", "-fPIC", "-Wall", "-Wextra", "-Werror",
                        str(root / "scripts/fixtures/netcdf_close_fault.c"), "-o", str(library)],
                       check=True, timeout=30)

        def run(command, target, injected=None):
            trace = temp / "fault.log"
            trace.unlink(missing_ok=True)
            env = os.environ.copy()
            for name in ("LD_PRELOAD", "COLM_TEST_IO_TARGET", "COLM_TEST_IO_TRACE", "COLM_TEST_IO_ERRNO"):
                env.pop(name, None)
            if injected:
                env.update(LD_PRELOAD=str(library), COLM_TEST_IO_TARGET=str(target.resolve()),
                           COLM_TEST_IO_TRACE=str(trace), COLM_TEST_IO_ERRNO=injected)
            result = subprocess.run([str(part) for part in command], env=env,
                                    capture_output=True, text=True, timeout=30)
            evidence = trace.read_text() if trace.exists() else ""
            if injected:
                assert f"close errno={getattr(errno, injected)} " in evidence, (result.stderr, evidence)
            else:
                assert not evidence
            return result, evidence

        source = temp / "observation.csv"
        source.write_text("time,site,Qle [W/m2]\n2020-01-01 00:00:00,A,10\n2020-01-01 00:30:00,A,11\n",
                          encoding="utf-8")
        output = temp / "Observation"
        command = [cli, "observation-table-convert", source, output, "--time-column", "time",
                   "--site-column", "site", "--variable", "Qle=Qle", "--json", "1"]
        result, _ = run(command, output / "A_Flux.nc.tmp")
        assert result.returncode == 0, result.stderr
        assert json.loads(result.stdout)[0]["rows"] == 2
        destination = output / "A_Flux.nc"
        verified, _ = run([probe, "observation", destination], destination)
        assert verified.returncode == 0, verified.stderr
        previous = hashlib.sha256(destination.read_bytes()).hexdigest()
        checks.append("normal observation conversion publishes a readable result")
        for injected in ("ENOSPC", "EIO"):
            result, evidence = run(command, output / "A_Flux.nc.tmp", injected)
            assert result.returncode != 0, "close failure was reported as conversion success"
            assert "cannot close" in result.stderr, result.stderr
            assert not result.stdout.strip(), result.stdout
            assert hashlib.sha256(destination.read_bytes()).hexdigest() == previous
            checks.append(f"observation {injected}: checked close fails and previous output is unchanged ({evidence.strip()})")

            path = temp / f"river-{injected}.nc"
            result, _ = run([probe, "history", path], path)
            assert result.returncode == 0, result.stderr
            result, evidence = run([probe, "history", path], path, injected)
            assert result.returncode != 0, "river writer swallowed its close failure"
            assert "actual river history writer failed" in result.stderr, result.stderr
            assert "writer returned success" not in result.stdout
            checks.append(f"river history {injected}: production API returns failure ({evidence.strip()})")

            unchecked = temp / f"unchecked-{injected}.nc"
            result, evidence = run([probe, "unchecked", unchecked], unchecked, injected)
            assert result.returncode == 0 and "writer returned success" in result.stdout, result.stderr
            checks.append(f"negative control {injected}: Drop alone silently swallows the injected close failure ({evidence.strip()})")
    print(json.dumps({"passed": len(checks), "checks": checks}, indent=2))


if __name__ == "__main__":
    main()
