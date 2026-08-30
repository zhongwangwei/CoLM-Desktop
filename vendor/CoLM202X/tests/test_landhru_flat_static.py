import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "mksrfdata/MOD_LandHRU.F90"


def test_flat_landhru_uses_replicated_metadata_and_global_collectives() -> None:
    with tempfile.TemporaryDirectory() as include_dir:
        Path(include_dir, "define.h").write_text(
            "#define CATCHMENT\n#define USEMPI\n#define FLAT_SPMD\n",
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

    body = source.split("SUBROUTINE landhru_build", 1)[1].split(
        "END SUBROUTINE landhru_build", 1
    )[0]
    assert "CALL mpi_send" not in body
    assert "CALL mpi_recv" not in body
    assert "numhru_all_g(landelm%eindex)" in body
    assert "lakeid(landelm%eindex)" in body
    assert "CALL move_alloc (ibuff, lakeid)" in body
    assert "CALL mpi_reduce (numhru, nhru_glb" in body
    assert "p_comm_worker" not in body
