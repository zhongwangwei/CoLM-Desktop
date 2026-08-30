import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def test_flat_gridded_history_uses_one_collective_gather_path() -> None:
    with tempfile.TemporaryDirectory() as include_dir:
        Path(include_dir, "define.h").write_text(
            "#define GRIDBASED\n#define USEMPI\n#define FLAT_SPMD\n",
            encoding="utf-8",
        )
        source = subprocess.run(
            [
                "cpp",
                "-P",
                "-traditional-cpp",
                f"-I{include_dir}",
                f"-I{ROOT / 'include'}",
                str(ROOT / "main/MOD_HistGridded.F90"),
            ],
            check=True,
            capture_output=True,
            text=True,
        ).stdout

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
