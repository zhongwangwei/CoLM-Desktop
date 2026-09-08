import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def preprocess(path: Path, grid: str) -> str:
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
                str(path),
            ],
            check=True,
            capture_output=True,
            text=True,
        ).stdout


def test_flat_methane_ph_uses_all_to_all_sparse_exchange() -> None:
    for grid in ("CATCHMENT", "UNSTRUCTURED"):
        mapping = preprocess(
            ROOT / "main/TRACER/MOD_Tracer_Reactive_Methane_PHMapping.F90", grid
        )
        aggregate = preprocess(ROOT / "mksrfdata/Aggregation_MethanePH.F90", grid)

        mapping_body = mapping.split("SUBROUTINE build_methane_ph_areal_mapping", 1)[
            1
        ].split("END SUBROUTINE build_methane_ph_areal_mapping", 1)[0]
        aggregate_body = aggregate.split("SUBROUTINE aggregate_sparse_ph", 1)[1].split(
            "END SUBROUTINE aggregate_sparse_ph", 1
        )[0]

        assert "type(grid_list_type), allocatable :: io_glist(:)" in mapping
        assert "CALL mpi_alltoall" in mapping_body
        assert mapping_body.count("CALL mpi_alltoallv") == 2
        assert "CALL mpi_send" not in mapping_body
        assert "CALL mpi_recv" not in mapping_body
        assert "mapping%io_glist" in aggregate_body
        assert aggregate_body.count("CALL mpi_alltoallv") == 2
        assert "CALL mpi_send" not in aggregate_body
        assert "CALL mpi_recv" not in aggregate_body
