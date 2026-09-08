import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "share/MOD_SpatialMapping.F90"


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


def test_flat_spatial_mapping_exchanges_data_with_block_owners() -> None:
    for grid in ("GRIDBASED", "UNSTRUCTURED", "CATCHMENT"):
        source = preprocess(grid)
        areal = source.split("SUBROUTINE spatial_mapping_build_arealweighted", 1)[1].split(
            "END SUBROUTINE spatial_mapping_build_arealweighted", 1
        )[0]
        bilinear = source.split("SUBROUTINE spatial_mapping_build_bilinear", 1)[1].split(
            "END SUBROUTINE spatial_mapping_build_bilinear", 1
        )[0]

        assert "CALL mpi_send" not in source
        assert "CALL mpi_recv" not in source
        assert areal.count("allocate (this%glist (0:p_np_io-1))") == 1
        assert bilinear.count("allocate (this%glist (0:p_np_io-1))") == 1
        assert source.count("CALL flat_transpose_grid_lists (this)") == 2
        assert "allocate (this%io_glist(0:p_np_glb-1))" in source
        assert "ASSOCIATE (owner_glist => this%io_glist)" in source
        assert "CALL flat_grid_to_workers_real8_2d" in source
        assert "CALL flat_grid_to_workers_real8_3d" in source
        assert "CALL flat_grid_to_workers_integer_2d" in source
        assert source.count("CALL mpi_alltoallv") >= 4
        assert "flat_sum_block" not in source
        assert "flat_max_block" not in source
