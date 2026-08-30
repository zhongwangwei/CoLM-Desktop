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


def test_flat_spatial_mapping_uses_local_replicas_and_collectives() -> None:
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
        assert "pbuff(iproc)%val(ig) = gdata%blk" in source
        assert "CALL flat_sum_block_2d" in source
        assert "CALL flat_sum_block_3d" in source
        assert "CALL flat_sum_block_4d" in source
        assert "CALL flat_max_block_2d" in source
        assert source.count("CALL mpi_allreduce") >= 8
