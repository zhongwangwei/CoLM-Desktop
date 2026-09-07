#!/usr/bin/env python3
"""Regression for RiverHistConcatenate serial/MPI startup ordering."""
from pathlib import Path
import os
import re
import shutil
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "vendor/CoLM202X/postprocess/RiverHistConcatenate.F90"


def run(command, *, cwd, check=True):
    return subprocess.run(
        command,
        cwd=cwd,
        check=check,
        capture_output=True,
        text=True,
        timeout=120,
    )


def startup_prefix() -> str:
    text = SOURCE.read_text(encoding="utf-8")
    prefix = text.split("CONTAINS", 1)[0]
    assert "#ifdef USEMPI\n      CALL spmd_init\n#endif" in prefix
    assert "CALL read_namelist (nlfile)" in prefix
    assert "IF (.not. p_is_master) THEN" in prefix
    assert prefix.index("CALL read_namelist (nlfile)") < prefix.index("IF (.not. p_is_master) THEN")
    assert "#ifdef USEMPI\n      CALL spmd_exit\n#endif" in prefix
    return prefix


def support_modules(master: bool, include_spmd_api: bool) -> str:
    spmd_bits = ""
    if include_spmd_api:
        spmd_bits = """
  subroutine spmd_init()
    print '(A)', 'SPMD_INIT'
  end subroutine spmd_init
  subroutine spmd_exit()
    use MOD_Namelist, only: namelist_read
    if (.not. namelist_read) error stop 'spmd_exit before read_namelist'
    print '(A)', 'SPMD_EXIT'
  end subroutine spmd_exit
"""
    return f"""
module MOD_Precision
  implicit none
  integer, parameter :: r8 = selected_real_kind(12)
end module MOD_Precision

module MOD_Namelist
  implicit none
  logical :: namelist_read = .false.
contains
  subroutine read_namelist(path)
    character(len=*), intent(in) :: path
    namelist_read = .true.
    print '(A)', 'READ_NAMELIST:' // trim(path)
  end subroutine read_namelist
end module MOD_Namelist

module MOD_SPMD_Task
  implicit none
  logical :: p_is_master = {'.true.' if master else '.false.'}
contains
  subroutine CoLM_stop(message)
    character(len=*), intent(in), optional :: message
    if (present(message)) print '(A)', trim(message)
    error stop 1
  end subroutine CoLM_stop
{spmd_bits}end module MOD_SPMD_Task

module MOD_Filesystem
  implicit none
contains
  subroutine list_matching_paths(prefix, suffix, listfile)
    character(len=*), intent(in) :: prefix, suffix, listfile
  end subroutine list_matching_paths
  subroutine move_file(src, dst)
    character(len=*), intent(in) :: src, dst
  end subroutine move_file
end module MOD_Filesystem

module MOD_NetCDFSerial
  implicit none
end module MOD_NetCDFSerial

module netcdf
  implicit none
end module netcdf
"""


def generated_program(master: bool, include_spmd_api: bool) -> str:
    return (
        support_modules(master=master, include_spmd_api=include_spmd_api)
        + startup_prefix()
        + "CONTAINS\n"
        + "  subroutine scan_shards()\n    print '(A)', 'SCAN_SHARDS'\n  end subroutine scan_shards\n"
        + "  subroutine build_output()\n    print '(A)', 'BUILD_OUTPUT'\n  end subroutine build_output\n"
        + "  subroutine verify_and_promote()\n    print '(A)', 'VERIFY_AND_PROMOTE'\n  end subroutine verify_and_promote\n"
        + "END PROGRAM river_hist_concatenate\n"
    )


def compile_and_run(compiler: str, work: Path, name: str, *, master: bool, usempi: bool) -> subprocess.CompletedProcess:
    source = work / f"{name}.F90"
    source.write_text(generated_program(master=master, include_spmd_api=usempi), encoding="utf-8")
    (work / "define.h").write_text("", encoding="utf-8")
    exe = work / (name + (".exe" if os.name == "nt" else ""))
    command = [compiler, "-cpp", "-I", str(work)]
    if usempi:
        command.append("-DUSEMPI")
    command += [str(source), "-o", str(exe)]
    run(command, cwd=work)
    return run([str(exe), "case.nml", "target.nc"], cwd=work)


def main() -> None:
    compiler = shutil.which("gfortran")
    if not compiler:
        raise SystemExit("gfortran is required for the postprocess startup regression")

    with tempfile.TemporaryDirectory(prefix="riverhist-startup-") as directory:
        work = Path(directory)

        # Serial builds of MOD_SPMD_Task do not provide spmd_init/spmd_exit.  The
        # extracted production startup must compile/link without those symbols.
        serial = compile_and_run(compiler, work, "serial_startup", master=True, usempi=False)
        assert "SPMD_INIT" not in serial.stdout
        assert "SPMD_EXIT" not in serial.stdout
        for marker in ("READ_NAMELIST:case.nml", "SCAN_SHARDS", "BUILD_OUTPUT", "VERIFY_AND_PROMOTE"):
            assert marker in serial.stdout, serial.stdout

        # Simulate a non-master MPI rank without launching MPI: it must still read
        # the namelist before spmd_exit, because production read_namelist contains
        # collectives. It must not run master-only merge work.
        worker = compile_and_run(compiler, work, "worker_startup", master=False, usempi=True)
        lines = [line.strip() for line in worker.stdout.splitlines() if line.strip()]
        assert lines[:3] == ["SPMD_INIT", "READ_NAMELIST:case.nml", "SPMD_EXIT"], lines
        assert "SCAN_SHARDS" not in worker.stdout

    print("postprocess startup: serial link and MPI worker namelist ordering PASS")


if __name__ == "__main__":
    main()
