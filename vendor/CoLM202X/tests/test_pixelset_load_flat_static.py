import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "mksrfdata/MOD_SrfdataRestart.F90"


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
        assert "mesh(ie)%indx == pixelset%eindex(iset)" in body
        assert "sbuff = pack(pixelset%eindex, msk)" in body
        assert "CALL move_alloc(sbuff, pixelset%eindex)" in body
        assert "pixelset%nset = nset" in body
