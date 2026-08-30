import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def test_flat_tracer_vector_history_reuses_collective_gather() -> None:
    for grid in ("CATCHMENT", "UNSTRUCTURED"):
        with tempfile.TemporaryDirectory() as include_dir:
            Path(include_dir, "define.h").write_text(
                f"#define {grid}\n#define USEMPI\n#define FLAT_SPMD\n",
                encoding="utf-8",
            )
            source = subprocess.run(
                [
                    "cpp",
                    "-P",
                    "-traditional-cpp",
                    f"-I{include_dir}",
                    f"-I{ROOT / 'include'}",
                    str(ROOT / "main/TRACER/MOD_Tracer_Hist.F90"),
                ],
                check=True,
                capture_output=True,
                text=True,
            ).stdout

        for name in (
            "write_history_tracer_vector_2d",
            "write_history_tracer_ratio_vector_3d",
        ):
            body = source.split(f"SUBROUTINE {name}", 1)[1].split(
                f"END SUBROUTINE {name}", 1
            )[0]
            assert "CALL gather_history_fields" in body
            assert "CALL mpi_send" not in body
            assert "CALL mpi_recv" not in body

    helper = (ROOT / "main/MOD_HistVector.F90").read_text(encoding="utf-8")
    assert "PUBLIC :: gather_history_fields" in helper
    assert "CALL mpi_gatherv" in helper
