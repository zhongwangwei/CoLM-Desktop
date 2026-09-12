import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "mksrfdata/MOD_SrfdataRestart.F90"
MESH_SOURCE = ROOT / "share/MOD_Mesh.F90"


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


def test_flat_pixelset_load_filters_int64_indices_locally() -> None:
    for grid in ("GRIDBASED", "UNSTRUCTURED", "CATCHMENT"):
        source = preprocess(grid)
        body = source.split("SUBROUTINE pixelset_load_from_file", 1)[1].split(
            "END SUBROUTINE pixelset_load_from_file", 1
        )[0]

        assert "CALL mpi_send" not in body
        assert "CALL mpi_recv" not in body
        assert "elmindx(middle) == pixelset%eindex(iset)" in body
        assert "CALL quicksort (numelm, elmindx, order)" in body
        assert "DO WHILE (lower <= upper)" in body
        assert "sbuff = pack(pixelset%eindex, msk)" in body
        assert "CALL move_alloc(sbuff, pixelset%eindex)" in body
        assert "pixelset%nset = nset" in body


def test_flat_mesh_load_replicates_saved_blocks_then_partitions_work() -> None:
    for grid in ("GRIDBASED", "UNSTRUCTURED", "CATCHMENT"):
        source = preprocess(grid)
        body = source.split("SUBROUTINE mesh_load_from_file", 1)[1].split(
            "END SUBROUTINE mesh_load_from_file", 1
        )[0]

        assert "CALL mesh_partition_spmd" in body
        assert "DO iblkme = 1, gblock%nblkme" in body
        assert "CALL mpi_send" not in body
        assert "CALL mpi_recv" not in body


def test_flat_mesh_partition_keeps_block_order_for_vector_restart_files() -> None:
    body = MESH_SOURCE.read_text(encoding="utf-8").split(
        "SUBROUTINE mesh_partition_spmd", 1
    )[1].split("END SUBROUTINE mesh_partition_spmd", 1)[0]

    assert "CALL quicksort" not in body
    assert "CALL copy_elm (mesh(ie), mesh_local(ie-ifirst+1))" in body


def test_flat_route_history_retains_global_ucat_coordinates() -> None:
    network = (ROOT / "main/HYDRO/MOD_Grid_RiverLakeNetwork.F90").read_text(
        encoding="utf-8"
    )
    route = (ROOT / "main/HYDRO/MOD_Grid_RiverLakeHistRoute.F90").read_text(
        encoding="utf-8"
    )
    distribute = network.split("! send unit catchment index to workers", 1)[1].split(
        "#else", 1
    )[0]
    assert "x_ucat_global = x_ucat" in distribute
    assert distribute.index("x_ucat_global = x_ucat") < distribute.index(
        "CALL move_alloc (x_ucat, idata1d)"
    )

    writer = route.split("SUBROUTINE route_hist_write_ucat", 1)[1].split(
        "END SUBROUTINE route_hist_write_ucat", 1
    )[0]
    assert "x_ucat_global, griducat%nlat, y_ucat_global" in writer


def test_routing_cleanup_allows_tracer_to_be_disabled() -> None:
    flow = (ROOT / "main/HYDRO/MOD_Grid_RiverLakeFlow.F90").read_text(
        encoding="utf-8"
    )
    cleanup = flow.split("SUBROUTINE grid_riverlake_flow (year, deltime)", 1)[1].rsplit(
        "END SUBROUTINE grid_riverlake_flow", 1
    )[0]

    assert "IF (allocated(acc_trc_inp)) acc_trc_inp = 0._r8" in cleanup
    assert "IF (allocated(acc_rnof_ref)) acc_rnof_ref = 0._r8" in cleanup
    assert "IF (allocated(trc_dry_drain)) trc_dry_drain = 0._r8" in cleanup
