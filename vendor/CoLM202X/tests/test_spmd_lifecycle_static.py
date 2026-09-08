from pathlib import Path
import shutil
import subprocess

import pytest


ROOT = Path(__file__).resolve().parents[1]
SOURCE = (ROOT / "share/MOD_SPMD_Task.F90").read_text(encoding="utf-8")
COLM = (ROOT / "main/CoLM.F90").read_text(encoding="utf-8")


def routine(source: str, name: str) -> str:
    return source.split(f"SUBROUTINE {name}", 1)[1].split(
        f"END SUBROUTINE {name}", 1
    )[0]


def test_spmd_only_finalizes_mpi_it_initialized() -> None:
    assert "USE mpi" in SOURCE
    assert "mpif.h" not in SOURCE
    assert "p_mpi_owned = .not. mpi_inited" in SOURCE
    assert "IF (p_mpi_owned) CALL mpi_finalize(p_err)" in SOURCE


def test_spmd_communicators_start_null_for_early_exit() -> None:
    for name in (
        "p_comm_glb_plus",
        "p_comm_glb",
        "p_comm_group",
        "p_comm_io",
        "p_comm_worker",
    ):
        assert f"integer :: {name} = MPI_COMM_NULL" in SOURCE
    for flag in ("p_is_io", "p_is_worker", "p_is_writeback"):
        assert f"logical :: {flag} = .false." in SOURCE


def test_spmd_exit_frees_only_live_owned_communicators() -> None:
    exit_body = routine(SOURCE, "spmd_exit")
    assert "p_comm_glb /= MPI_COMM_NULL" in exit_body
    assert "p_comm_group /= MPI_COMM_NULL" in exit_body
    assert "p_is_io .and. p_comm_io /= MPI_COMM_NULL" in exit_body
    assert "p_is_worker .and. p_comm_worker /= MPI_COMM_NULL" in exit_body
    assert "p_comm_glb_plus /= MPI_COMM_NULL" in exit_body
    assert "CALL mpi_barrier (p_comm_glb, p_err)" in exit_body


def test_flat_spmd_frees_only_the_duplicated_global_communicator() -> None:
    exit_body = routine(SOURCE, "spmd_exit")
    flat = exit_body.split("#ifdef FLAT_SPMD", 1)[1].split("#else", 1)[0]
    assert flat.count("mpi_comm_free") == 1
    assert "p_comm_glb" in flat


def test_writeback_rank_has_no_global_comm_after_split() -> None:
    assign = routine(SOURCE, "spmd_assign_writeback")
    writeback_branch = assign.split("ELSE", 1)[1]
    assert "p_comm_glb = MPI_COMM_NULL" in writeback_branch
    assert "p_iam_glb = -1" in writeback_branch
    assert "p_np_glb = 0" in writeback_branch


def test_usesplitai_owns_direct_init_and_scheme_iii_always_fails_fast() -> None:
    assert "CALL MPI_Initialized(split_mpi_inited, ierr)" in COLM
    assert "split_mpi_owned = .not. split_mpi_inited" in COLM
    assert "IF (split_mpi_owned) CALL MPI_Init(ierr)" in COLM
    assert "IF (new_comm /= MPI_COMM_NULL) CALL MPI_Comm_free(new_comm, ierr)" in COLM
    assert "IF (split_mpi_owned) CALL MPI_Finalize(ierr)" in COLM
    assert "Precipitation scheme III is unavailable" in COLM
    assert "no Python MPI server ranks" in COLM


@pytest.mark.parametrize(
    ("extra_flags", "body"),
    [
        ([], "call spmd_init()\n  call spmd_exit()"),
        ([], "call spmd_init()\n  call spmd_assign_writeback()\n  call spmd_exit()"),
        (["-DFLAT_SPMD"], "call spmd_init()\n  call divide_processes_into_groups(1)\n  call spmd_exit()"),
    ],
)
def test_spmd_lifecycle_mpi_smoke(tmp_path: Path, extra_flags: list[str], body: str) -> None:
    mpifort = shutil.which("mpifort") or shutil.which("mpif90")
    mpiexec = shutil.which("mpiexec")
    if mpifort is None or mpiexec is None:
        pytest.skip("MPI compiler/runtime is not available")

    harness = tmp_path / "spmd_lifecycle_probe.F90"
    harness.write_text(
        "program spmd_lifecycle_probe\n"
        "  use MOD_SPMD_Task\n"
        "  implicit none\n"
        f"  {body}\n"
        "end program\n",
        encoding="utf-8",
    )
    exe = tmp_path / "spmd_lifecycle_probe"
    cmd = [
        mpifort,
        "-cpp",
        *extra_flags,
        f"-I{ROOT / 'include'}",
        f"-J{tmp_path}",
        str(ROOT / "share/MOD_Precision.F90"),
        str(ROOT / "share/MOD_SPMD_Task.F90"),
        str(harness),
        "-o",
        str(exe),
    ]
    built = subprocess.run(cmd, capture_output=True, text=True, timeout=30)
    assert built.returncode == 0, built.stdout + built.stderr

    ran = subprocess.run(
        [mpiexec, "-n", "2", str(exe)],
        capture_output=True,
        text=True,
        timeout=30,
    )
    assert ran.returncode == 0, ran.stdout + ran.stderr
