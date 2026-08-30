import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def test_flat_element_neighbour_uses_collectives_and_int64_identity() -> None:
    source = subprocess.run(
        [
            "cpp",
            "-P",
            "-traditional-cpp",
            "-DFLAT_SPMD",
            f"-I{ROOT / 'include'}",
            str(ROOT / "main/HYDRO/MOD_ElementNeighbour.F90"),
        ],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    init = source.split("SUBROUTINE element_neighbour_init", 1)[1].split(
        "END SUBROUTINE element_neighbour_init", 1
    )[0]

    assert "CALL mpi_send" not in init
    assert "CALL mpi_recv" not in init
    assert "mpi_gatherv (eindex" in init
    assert "mpi_scatterv (icache2" in init
    assert "MPI_INTEGER8, idxnball" in init
    assert "mpi_gatherv (idxinq" in init
    assert "mpi_scatterv (addrinq_all" in init

    netcdf = (ROOT / "share/MOD_NetCDFSerial.F90").read_text(encoding="utf-8")
    assert "MODULE procedure ncio_read_serial_int64_2d" in netcdf
    assert "SUBROUTINE ncio_read_serial_int64_2d" in netcdf
