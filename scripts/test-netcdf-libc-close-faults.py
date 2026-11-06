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
        # Guard the injection boundary independently of the actual NetCDF writer.
        target_check = temp / "target-check.c"
        target_check.write_text(r'''
#include <assert.h>
#include "netcdf_fault_target.h"
int main(void) {
    const char *target = "/private/fixture/Observation/A_Flux.nc";
    unsetenv("COLM_TEST_IO_TARGET_MODE");
    assert(colm_fault_target_matches(target, target));
    assert(!colm_fault_target_matches("/private/fixture/Observation/.A_Flux.nc.1-2-3.tmp", target));
    setenv("COLM_TEST_IO_TARGET_MODE", "observation-temp", 1);
    setenv("COLM_TEST_IO_ROOT", "/private/fixture", 1);
    assert(colm_fault_target_matches("/private/fixture/Observation/.A_Flux.nc.1-2-3.tmp", target));
    const char *bad[] = {target, "/private/fixture-other/Observation/.A_Flux.nc.1-2-3.tmp", "/private/fixture/Elsewhere/.A_Flux.nc.1-2-3.tmp", "/private/fixture/Observation/.B_Flux.nc.1-2-3.tmp", "/private/fixture/Observation/.A_Flux.nc.-2-3.tmp", "/private/fixture/Observation/.A_Flux.nc.1-2-3.tmp.extra", "/private/fixture/Observation/.A_Flux.nc.1-2.tmp", "/private/fixture/Observation/.A_Flux.nc.1-2-3-4.tmp", ""};
    for (unsigned i=0;i<sizeof(bad)/sizeof(*bad);i++) assert(!colm_fault_target_matches(bad[i],target));
    setenv("COLM_TEST_IO_ROOT", "/private/other", 1);
    assert(!colm_fault_target_matches("/private/fixture/Observation/.A_Flux.nc.1-2-3.tmp", target));
    unsetenv("COLM_TEST_IO_ROOT");
    assert(!colm_fault_target_matches("/private/fixture/Observation/.A_Flux.nc.1-2-3.tmp", target));
}
''', encoding="utf-8")
        target_binary = temp / "target-check"
        subprocess.run(["cc", "-Wall", "-Wextra", "-Werror", "-I",
                        str(root / "scripts/fixtures"), str(target_check), "-o", str(target_binary)],
                       check=True, timeout=30)
        subprocess.run([str(target_binary)], check=True, timeout=10)
        checks.append("fault target matching accepts only the private exact or unique staging path")
        library = temp / "close-fault.so"
        subprocess.run(["cc", "-shared", "-fPIC", "-Wall", "-Wextra", "-Werror",
                        str(root / "scripts/fixtures/netcdf_close_fault.c"), "-o", str(library)],
                       check=True, timeout=30)

        def run(command, target, injected=None, final_only=True, observation_temp=False):
            trace = temp / "fault.log"
            trace.unlink(missing_ok=True)
            phase = temp / "inside-nc-close"
            phase.unlink(missing_ok=True)
            env = os.environ.copy()
            for name in ("LD_PRELOAD", "COLM_TEST_IO_TARGET", "COLM_TEST_IO_TRACE", "COLM_TEST_IO_ERRNO",
                         "COLM_TEST_IO_CLOSE_PHASE", "COLM_TEST_IO_ONLY_FINAL_CLOSE",
                         "COLM_TEST_IO_TARGET_MODE", "COLM_TEST_IO_ROOT"):
                env.pop(name, None)
            if injected:
                env.update(LD_PRELOAD=str(library), COLM_TEST_IO_TARGET=str(target.resolve()),
                           COLM_TEST_IO_TRACE=str(trace), COLM_TEST_IO_ERRNO=injected,
                           COLM_TEST_IO_CLOSE_PHASE=str(phase))
                if final_only:
                    env["COLM_TEST_IO_ONLY_FINAL_CLOSE"] = "1"
            if injected and observation_temp:
                assert target.resolve().is_relative_to(temp)
                env.update(COLM_TEST_IO_TARGET_MODE="observation-temp", COLM_TEST_IO_ROOT=str(temp))
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
                   "--site-column", "site", "--variable", "Qle=Qle", "--json", "1", "--overwrite", "1"]
        result, _ = run([opts.plain_cli.resolve(), *command[1:]], output / "A_Flux.nc")
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
            # A dedicated direct-create probe exercises H5Fcreate's earlier close.
            # Production conversion uses unique staging names and never reuses this file.
            early_tmp = temp / f"early-{injected}.nc"
            early_tmp.write_bytes(destination.read_bytes())
            result, evidence = run([probe, "unchecked", early_tmp], early_tmp,
                                   injected, final_only=False)
            assert result.returncode > 0, (result.returncode, result.stderr, evidence)
            assert "netcdf error(13): Permission denied" in result.stderr, result.stderr
            assert not result.stdout.strip(), result.stdout
            fault_index = evidence.index(f"close errno={getattr(errno, injected)} ")
            assert "nc_close enter" not in evidence[:fault_index], evidence
            assert "phase=outside" in evidence, evidence
            assert hashlib.sha256(destination.read_bytes()).hexdigest() == previous
            checks.append(f"direct-create probe early {injected}: H5Fcreate fails before final nc_close ({evidence.strip()})")
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
                      "scope": "actual libc close ENOSPC/EIO on private files, with real NetCDF I/O"}, indent=2))


if __name__ == "__main__":
    main()
