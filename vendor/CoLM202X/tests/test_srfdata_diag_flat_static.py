import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "mksrfdata/MOD_SrfdataDiag.F90"


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


def test_flat_surface_diagnostics_gather_owned_blocks_for_single_file() -> None:
    for grid in ("GRIDBASED", "UNSTRUCTURED", "CATCHMENT"):
        source = preprocess(grid)
        body = source.split("SUBROUTINE srfdata_map_and_write", 1)[1].split(
            "END SUBROUTINE srfdata_map_and_write", 1
        )[0]

        assert "CALL mpi_send" not in body
        assert "CALL mpi_recv" not in body
        assert "CALL gather_srfdata_diag" in body
        assert "IF (p_is_master) THEN" in body
        assert "global_fields(ityp,:,:)" in body
        block = body.split("ELSEIF (trim(wmode) == 'block') THEN", 1)[1]
        assert "IF (p_is_io) THEN" in block
        assert "IF (gblock%pio(iblk,jblk) /= p_iam_glb) CYCLE" in block
