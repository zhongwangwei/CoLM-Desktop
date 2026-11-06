#!/usr/bin/env python3
"""Verify propagation of ENOSPC/EIO returned by the real NetCDF C close boundary.

  bash scripts/build-netcdf-fault-probes.sh /private/output
  python3 scripts/test-netcdf-close-faults.py --cli /private/output/colm-cli \
      --runtime-probe /private/output/netcdf-close-probe

The target-only linker wrapper performs the actual nc_close (including real file
I/O), then reports the injected close error for a private fixture path or its
strictly matched unique observation staging file.
This tests checked-close propagation, not filesystem capacity exhaustion. The
separate test-netcdf-libc-close-faults.py verifies raw POSIX close errors and the
vendored netCDF fix that avoids inspecting HDF5 state after failed close.
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
    if not __debug__:
        raise SystemExit("NetCDF fault checks require assertions; remove -O or PYTHONOPTIMIZE")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--runtime-probe", type=Path, required=True)
    opts = parser.parse_args()
    assert sys.platform == "linux", "link-wrapped NetCDF close regression requires Linux"
    cli, probe = opts.cli.resolve(), opts.runtime_probe.resolve()
    assert cli.is_file() and probe.is_file(), "build the current CLI and runtime probe first"
    checks = []
    with tempfile.TemporaryDirectory(prefix="colm-close-fault-") as temporary:
        temp = Path(temporary).resolve()

        def run(command, target, injected=None, observation_temp=False):
            trace = temp / "fault.log"
            trace.unlink(missing_ok=True)
            env = os.environ.copy()
            for name in ("LD_PRELOAD", "COLM_TEST_IO_TARGET", "COLM_TEST_IO_TRACE", "COLM_TEST_IO_ERRNO",
                         "COLM_TEST_IO_CLOSE_PHASE", "COLM_TEST_IO_ONLY_FINAL_CLOSE",
                         "COLM_TEST_IO_TARGET_MODE", "COLM_TEST_IO_ROOT"):
                env.pop(name, None)
            if injected:
                env.update(COLM_TEST_IO_TARGET=str(target.resolve()),
                           COLM_TEST_IO_TRACE=str(trace), COLM_TEST_IO_ERRNO=injected)
            if injected and observation_temp:
                assert target.resolve().is_relative_to(temp)
                env.update(COLM_TEST_IO_TARGET_MODE="observation-temp", COLM_TEST_IO_ROOT=str(temp))
            result = subprocess.run([str(part) for part in command], env=env,
                                    capture_output=True, text=True, timeout=30)
            evidence = trace.read_text() if trace.exists() else ""
            if injected:
                assert f"nc_close errno={getattr(errno, injected)} " in evidence, (result.stderr, evidence)
            else:
                assert not evidence
            return result, evidence

        source = temp / "observation.csv"
        source.write_text("time,site,Qle [W/m2]\n2020-01-01 00:00:00,A,10\n2020-01-01 00:30:00,A,11\n",
                          encoding="utf-8")
        output = temp / "Observation"
        command = [cli, "observation-table-convert", source, output, "--time-column", "time",
                   "--site-column", "site", "--variable", "Qle=Qle", "--json", "1", "--overwrite", "1"]
        result, _ = run(command, output / "A_Flux.nc")
        assert result.returncode == 0, result.stderr
        assert json.loads(result.stdout)[0]["rows"] == 2
        destination = output / "A_Flux.nc"
        verified, _ = run([probe, "observation", destination], destination)
        assert verified.returncode == 0, verified.stderr
        previous = hashlib.sha256(destination.read_bytes()).hexdigest()
        checks.append("normal observation conversion publishes a readable result")
        source.write_text("time,site,Qle [W/m2]\n2020-01-01 00:00:00,A,20\n2020-01-01 00:30:00,A,21\n",
                          encoding="utf-8")
        for injected in ("ENOSPC", "EIO"):
            result, evidence = run(command, output / "A_Flux.nc", injected, observation_temp=True)
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
        result, _ = run(command, output / "A_Flux.nc")
        assert result.returncode == 0, result.stderr
        assert hashlib.sha256(destination.read_bytes()).hexdigest() != previous
        verified, _ = run([probe, "observation-updated", destination], destination)
        assert verified.returncode == 0, verified.stderr
        checks.append("retry without the fault publishes readable changed observation data")
    print(json.dumps({"passed": len(checks), "checks": checks,
                      "scope": "NetCDF C API close errors after actual close; raw libc close propagation is verified separately"}, indent=2))


if __name__ == "__main__":
    main()
