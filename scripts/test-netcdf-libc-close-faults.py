#!/usr/bin/env python3
"""Linux delayed-close ENOSPC/EIO tests against actual CLI and runtime writers.

Build the normal binaries plus phase-marked probes, then run:
  cargo build -p colm-cli --bin colm-cli
  cargo build -p colm-runtime --example netcdf-close-probe
  bash scripts/build-netcdf-fault-probes.sh /private/output
  python3 scripts/test-netcdf-libc-close-faults.py --cli /private/output/colm-cli \
      --runtime-probe /private/output/netcdf-close-probe --plain-cli target/debug/colm-cli

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
    if not __debug__:
        raise SystemExit("NetCDF fault checks require assertions; remove -O or PYTHONOPTIMIZE")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--runtime-probe", type=Path, required=True)
    parser.add_argument("--plain-cli", type=Path, required=True)
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

        def run(command, target, injected=None, final_only=True):
            trace = temp / "fault.log"
            trace.unlink(missing_ok=True)
            phase = temp / "inside-nc-close"
            phase.unlink(missing_ok=True)
            env = os.environ.copy()
            for name in ("LD_PRELOAD", "COLM_TEST_IO_TARGET", "COLM_TEST_IO_TRACE", "COLM_TEST_IO_ERRNO",
                         "COLM_TEST_IO_CLOSE_PHASE", "COLM_TEST_IO_ONLY_FINAL_CLOSE"):
                env.pop(name, None)
            if injected:
                env.update(LD_PRELOAD=str(library), COLM_TEST_IO_TARGET=str(target.resolve()),
                           COLM_TEST_IO_TRACE=str(trace), COLM_TEST_IO_ERRNO=injected,
                           COLM_TEST_IO_CLOSE_PHASE=str(phase))
                if final_only:
                    env["COLM_TEST_IO_ONLY_FINAL_CLOSE"] = "1"
            result = subprocess.run([str(part) for part in command], env=env,
                                    capture_output=True, text=True, timeout=30)
            evidence = trace.read_text() if trace.exists() else ""
            if injected:
                fault = f"close errno={getattr(errno, injected)} "
                assert fault in evidence, (result.stderr, evidence)
                if final_only:
                    assert evidence.index("nc_close enter") < evidence.index(fault), evidence
                    assert "phase=final" in evidence, evidence
                    assert "nc_close exit result=-101" in evidence, (result.stderr, evidence)
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
        source.write_text("time,site,Qle [W/m2]\n2020-01-01 00:00:00,A,20\n2020-01-01 00:30:00,A,21\n",
                          encoding="utf-8")
        for injected in ("ENOSPC", "EIO"):
            # H5Fcreate probes an existing pathname before truncation. A leftover
            # large temporary file makes its earlier libc close fault eligible.
            # Prove that distinct phase and preserve its graceful-failure control.
            for early_cli, label in ((cli, "phase-marked"), (opts.plain_cli.resolve(), "normal")):
                early_output = temp / f"early-{injected}-{label}"
                early_output.mkdir()
                early_tmp = early_output / "A_Flux.nc.tmp"
                early_tmp.write_bytes(destination.read_bytes())
                early_destination = early_output / "A_Flux.nc"
                early_destination.write_bytes(destination.read_bytes())
                early_command = [early_cli, *command[1:]]
                early_command[3] = early_output
                result, evidence = run(early_command, early_tmp, injected, final_only=False)
                assert result.returncode > 0, (result.returncode, result.stderr, evidence)
                assert "netcdf error(13): Permission denied" in result.stderr, result.stderr
                assert not result.stdout.strip(), result.stdout
                fault_index = evidence.index(f"close errno={getattr(errno, injected)} ")
                assert "nc_close enter" not in evidence[:fault_index], evidence
                assert "phase=outside" in evidence, evidence
                assert hashlib.sha256(early_destination.read_bytes()).hexdigest() == previous
                checks.append(f"{label} early {injected}: H5Fcreate fails before final nc_close and keeps old output ({evidence.strip()})")
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
        result, _ = run(command, output / "A_Flux.nc.tmp")
        assert result.returncode == 0, result.stderr
        assert hashlib.sha256(destination.read_bytes()).hexdigest() != previous
        verified, _ = run([probe, "observation-updated", destination], destination)
        assert verified.returncode == 0, verified.stderr
        checks.append("retry without the fault publishes readable changed observation data")
    print(json.dumps({"passed": len(checks), "checks": checks,
                      "scope": "actual libc close ENOSPC/EIO on private files, with real NetCDF I/O"}, indent=2))


if __name__ == "__main__":
    main()
