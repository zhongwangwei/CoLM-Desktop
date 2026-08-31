import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "share/MOD_Mesh.F90"
BLOCK_SOURCE = ROOT / "share/MOD_Block.F90"
AGGREGATION_SOURCE = ROOT / "mksrfdata/MOD_AggregationRequestData.F90"


def test_flat_mesh_build_has_no_role_messages() -> None:
    for grid in ("GRIDBASED", "UNSTRUCTURED", "CATCHMENT"):
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
                    str(SOURCE),
                ],
                check=True,
                capture_output=True,
                text=True,
            ).stdout

        body = source.split("SUBROUTINE mesh_build", 1)[1].split(
            "END SUBROUTINE mesh_build", 1
        )[0]
        assert "CALL mpi_send" not in body
        assert "CALL mpi_recv" not in body
        assert "CALL mesh_partition_spmd ()" in body
        assert "gblock%pio(iblk_p,jblk_p) >= 0" in body


def test_flat_blocks_replicate_active_blocks_on_each_rank() -> None:
    with tempfile.TemporaryDirectory() as include_dir:
        Path(include_dir, "define.h").write_text(
            "#define UNSTRUCTURED\n#define USEMPI\n#define FLAT_SPMD\n",
            encoding="utf-8",
        )
        source = subprocess.run(
            [
                "cpp",
                "-P",
                "-traditional-cpp",
                f"-I{include_dir}",
                f"-I{ROOT / 'include'}",
                str(BLOCK_SOURCE),
            ],
            check=True,
            capture_output=True,
            text=True,
        ).stdout

    assert "this%nblkme = count(this%pio >= 0)" in source
    assert "is_local = this%pio(iblk,jblk) >= 0" in source


def test_flat_aggregation_request_data_has_no_per_call_global_mpi_exchange() -> None:
    with tempfile.TemporaryDirectory() as include_dir:
        Path(include_dir, "define.h").write_text(
            "#define UNSTRUCTURED\n#define USEMPI\n#define FLAT_SPMD\n",
            encoding="utf-8",
        )
        source = subprocess.run(
            [
                "cpp",
                "-P",
                "-traditional-cpp",
                f"-I{include_dir}",
                f"-I{ROOT / 'include'}",
                str(AGGREGATION_SOURCE),
            ],
            check=True,
            capture_output=True,
            text=True,
        ).stdout

    body = source.split("SUBROUTINE aggregation_request_data", 1)[1].split(
        "END SUBROUTINE aggregation_request_data", 1
    )[0]
    lowered = body.lower()

    assert "call mpi_alltoall" not in lowered
    assert "call mpi_alltoallv" not in lowered
    assert "call mpi_send" not in lowered
    assert "call mpi_recv" not in lowered
    assert "CALL aggregation_exchange_" not in body
    assert "FLAT_SPMD requested a non-local block" not in body
    assert "data_r8_2d_out1(ireq) = data_r8_2d_in1%blk" in body
    assert "deallocate (xlist)" in body
    assert "deallocate (ylist)" in body
