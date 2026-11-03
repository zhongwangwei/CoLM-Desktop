#!/usr/bin/env python3
"""Run the production debug/range broadcasts and guarded collectives on three ranks."""
from pathlib import Path
import os
import re
import shutil
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[2]


def main():
    compiler = shutil.which("mpifort")
    launcher = shutil.which("mpiexec")
    assert compiler and launcher, "mpifort and mpiexec are required for the namelist MPI regression"
    source = (ROOT / "vendor/CoLM202X/share/MOD_Namelist.F90").read_text(encoding="utf-8")
    # These must be outside the master-only read block, before any other broadcasts.
    broadcasts = re.search(
        r"(?im)^\s*ENDIF\s*\n\s*#ifdef USEMPI\s*\n"
        r"((?:\s*CALL mpi_bcast[^\n]*\n){2})", source
    )
    assert broadcasts, "global namelist broadcasts must follow the master-only read block"
    calls = broadcasts.group(1)
    names = re.findall(r"mpi_bcast\s*\(\s*(\w+)", calls, flags=re.I)
    assert names == ["DEF_USE_CoLMDEBUG", "DEF_USE_RangeCheck"], names
    declarations = []
    for name in names:
        declaration = re.search(rf"(?im)^\s*logical\s*::\s*{name}\s*=\s*\.false\.\s*$", source)
        assert declaration, name
        declarations.append(declaration.group(0))
    program = """
program test
  use mpi
  implicit none
  integer :: p_err, p_address_master=0, p_comm_glb=MPI_COMM_WORLD
  integer :: rank, ranks, mode, active, total, rank_total
""" + "\n".join(declarations) + """
  call MPI_INIT(p_err)
  call MPI_COMM_RANK(p_comm_glb,rank,p_err)
  call MPI_COMM_SIZE(p_comm_glb,ranks,p_err)
  do mode=0,3
    DEF_USE_CoLMDEBUG=btest(mode,0)
    DEF_USE_RangeCheck=btest(mode,1)
    if (rank /= p_address_master) then
      DEF_USE_CoLMDEBUG=.not.DEF_USE_CoLMDEBUG
      DEF_USE_RangeCheck=.not.DEF_USE_RangeCheck
    endif
""" + calls + """
    if (DEF_USE_CoLMDEBUG) then
      call MPI_ALLREDUCE(rank,rank_total,1,MPI_INTEGER,MPI_SUM,p_comm_glb,p_err)
      if (rank_total /= ranks*(ranks-1)/2) call MPI_ABORT(p_comm_glb,1,p_err)
    endif
    if (DEF_USE_RangeCheck) then
      call MPI_ALLREDUCE(rank,rank_total,1,MPI_INTEGER,MPI_SUM,p_comm_glb,p_err)
      if (rank_total /= ranks*(ranks-1)/2) call MPI_ABORT(p_comm_glb,2,p_err)
    endif
    active=merge(1,0,DEF_USE_CoLMDEBUG)
    call MPI_ALLREDUCE(active,total,1,MPI_INTEGER,MPI_SUM,p_comm_glb,p_err)
    if (total /= merge(ranks,0,btest(mode,0))) call MPI_ABORT(p_comm_glb,3,p_err)
    active=merge(1,0,DEF_USE_RangeCheck)
    call MPI_ALLREDUCE(active,total,1,MPI_INTEGER,MPI_SUM,p_comm_glb,p_err)
    if (total /= merge(ranks,0,btest(mode,1))) call MPI_ABORT(p_comm_glb,4,p_err)
  enddo
  if (rank == p_address_master) print *, 'PASS: debug/range flags and collectives agree on',ranks,'ranks'
  call MPI_FINALIZE(p_err)
end program
"""
    with tempfile.TemporaryDirectory(prefix="colm-mpi-flags-") as directory:
        work = Path(directory)
        probe = work / "probe.f90"
        probe.write_text(program, encoding="utf-8")
        executable = work / ("probe.exe" if os.name == "nt" else "probe")
        subprocess.run([compiler, "-fcheck=all", str(probe), "-o", str(executable)], cwd=work, check=True, timeout=30)
        # Open MPI 5 ignores the old OMPI mapping variable; MPICH rejects this flag.
        version = subprocess.check_output([launcher, "--version"], text=True, timeout=10)
        oversubscribe = ["--oversubscribe"] if "Open MPI" in version else []
        result = subprocess.run([launcher, *oversubscribe, "-n", "3", str(executable)], cwd=work,
                                capture_output=True, text=True, timeout=30)
        assert result.returncode == 0, (result.stdout, result.stderr)
        assert "PASS: debug/range flags" in result.stdout, result.stdout
        print(result.stdout, end="")


if __name__ == "__main__":
    main()
