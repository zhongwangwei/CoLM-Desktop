import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "mksrfdata/MOD_SrfdataRestart.F90"


def preprocess(grid: str) -> str:
    with tempfile.TemporaryDirectory() as include_dir:
        Path(include_dir, "define.h").write_text(
            f"#define {grid}\n#define USEMPI\n#define FLAT_SPMD\n",
            encoding="utf-8",
        )
        return subprocess.run(
            [
                "cpp",
                "-P",
                "-traditional-cpp",
                f"-I{include_dir}",
                f"-I{ROOT / 'include'}",
                str(SOURCE),
            ],
            check=True,
            capture_output=True,
            text=True,
        ).stdout


def test_flat_mesh_save_gathers_to_rank_zero_without_role_messages() -> None:
    for grid in ("GRIDBASED", "UNSTRUCTURED", "CATCHMENT"):
        source = preprocess(grid)
        save = source.split("SUBROUTINE mesh_save_to_file", 1)[1].split(
            "END SUBROUTINE mesh_save_to_file", 1
        )[0]
        blocks = source.split("SUBROUTINE mesh_save_blocks_flat", 1)[1].split(
            "END SUBROUTINE mesh_save_blocks_flat", 1
        )[0]

        assert "CALL mesh_save_blocks_flat(filename)" in save
        assert "CALL mpi_send" not in save + blocks
        assert "CALL mpi_recv" not in save + blocks
        assert blocks.count("CALL mpi_allgather") == 2
        assert blocks.count("CALL mpi_gatherv") == 3
        assert "MPI_INTEGER8" in blocks
        assert "IF (p_is_master .and. nelm_glb > 0) THEN" in blocks
