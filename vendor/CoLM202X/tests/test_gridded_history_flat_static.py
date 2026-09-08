import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def preprocess(path: Path) -> str:
    with tempfile.TemporaryDirectory() as include_dir:
        Path(include_dir, "define.h").write_text(
            "#define GRIDBASED\n#define GridRiverLakeFlow\n#define USEMPI\n#define FLAT_SPMD\n",
            encoding="utf-8",
        )
        return subprocess.run(
            [
                "cpp",
                "-P",
                "-traditional-cpp",
                f"-I{include_dir}",
                f"-I{ROOT / 'include'}",
                str(path),
            ],
            check=True,
            capture_output=True,
            text=True,
        ).stdout


def test_flat_gridded_history_uses_one_collective_gather_path() -> None:
    source = preprocess(ROOT / "main/MOD_HistGridded.F90")

    helper = source.split("SUBROUTINE gather_gridded_history", 1)[1].split(
        "END SUBROUTINE gather_gridded_history", 1
    )[0]
    assert "CALL mpi_allgather" in helper
    assert "CALL mpi_gatherv" in helper

    for name in (
        "hist_write_var_real8_2d",
        "hist_write_var_real8_3d",
        "hist_write_var_real8_4d",
    ):
        body = source.split(f"SUBROUTINE {name}", 1)[1].split(
            f"END SUBROUTINE {name}", 1
        )[0]
        assert "CALL gather_gridded_history" in body
        assert "CALL mpi_send" not in body
        assert "CALL mpi_recv" not in body
        block = body.split("ELSEIF (trim(DEF_HIST_mode) == 'block') THEN", 1)[1]
        assert "IF (gblock%pio(iblk,jblk) /= p_iam_glb) CYCLE" in block

    write_time = source.split("SUBROUTINE hist_gridded_write_time", 1)[1].split(
        "END SUBROUTINE hist_gridded_write_time", 1
    )[0]
    block = write_time.split("ELSEIF (trim(DEF_HIST_mode) == 'block') THEN", 1)[1]
    assert "IF (gblock%pio(iblk,jblk) /= p_iam_glb) CYCLE" in block
    assert "CALL mpi_allreduce (MPI_IN_PLACE, itime, 1, MPI_INTEGER, MPI_MAX" in block
    assert "IF (.not. p_is_master) CALL mpi_bcast" not in block


def test_flat_route_history_writes_one_local_shard_per_rank() -> None:
    source = preprocess(ROOT / "main/HYDRO/MOD_Grid_RiverLakeHistShard.F90")
    layout = source.split("SUBROUTINE route_shard_layout_build ", 1)[1].split(
        "END SUBROUTINE route_shard_layout_build", 1
    )[0]
    assert "mpi_gather" not in layout.split("RETURN", 1)[0].lower()

    for name in ("route_shard_write_vector", "route_shard_write_matrix"):
        body = source.split(f"SUBROUTINE {name} ", 1)[1].split(
            f"END SUBROUTINE {name}", 1
        )[0]
        assert "mpi_gather" not in body.lower(), name
    assert "layout%ntotal = nlocal" in source
    assert "rbuff(1:layout%ntotal) = sbuff(1:layout%ntotal)" in source
