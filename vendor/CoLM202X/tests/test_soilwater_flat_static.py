import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def test_flat_vsf_debug_stats_do_not_send_rank_zero_to_itself() -> None:
    source = subprocess.run(
        [
            "cpp",
            "-P",
            "-traditional-cpp",
            "-DFLAT_SPMD",
            f"-I{ROOT / 'include'}",
            str(ROOT / "main/HYDRO/MOD_Hydro_SoilWater.F90"),
        ],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    stats = source.split("SUBROUTINE print_VSF_iteration_stat_info", 1)[1].split(
        "END SUBROUTINE print_VSF_iteration_stat_info", 1
    )[0]

    assert "CALL mpi_allreduce" in stats
    assert "CALL mpi_send" not in stats
    assert "CALL mpi_recv" not in stats
    assert "VSF scheme this step" in stats
