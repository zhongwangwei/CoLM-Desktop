import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "share/MOD_Namelist.F90"


def test_flat_namelist_disables_dedicated_history_writeback() -> None:
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

    body = source.split("SUBROUTINE read_namelist", 1)[1].split(
        "END SUBROUTINE read_namelist", 1
    )[0]
    broadcast = body.index("CALL mpi_bcast (DEF_HIST_WriteBack")
    disabled = body.index("DEF_HIST_WriteBack = .false.")
    assert disabled > broadcast
    assert "FLAT_SPMD disables DEF_HIST_WriteBack" in body
