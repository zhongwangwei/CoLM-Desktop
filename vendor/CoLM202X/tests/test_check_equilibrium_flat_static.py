import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def test_flat_equilibrium_output_uses_collectives_without_role_messages() -> None:
    source = subprocess.run(
        [
            "cpp",
            "-P",
            "-traditional-cpp",
            "-DFLAT_SPMD",
            f"-I{ROOT / 'include'}",
            str(ROOT / "main/MOD_CheckEquilibrium.F90"),
        ],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    check = source.split("SUBROUTINE CheckEquilibrium", 1)[1].split(
        "END SUBROUTINE CheckEquilibrium", 1
    )[0]
    output = source.split("SUBROUTINE map_and_write_check_var", 1)[1].split(
        "END SUBROUTINE map_and_write_check_var", 1
    )[0]

    assert "CALL mpi_allreduce" in check
    assert "CALL mpi_send" not in check
    assert "CALL mpi_recv" not in check
    assert "CALL mpi_gather" in output
    assert "CALL mpi_gatherv" in output
    assert "CALL mpi_send" not in output
    assert "CALL mpi_recv" not in output
