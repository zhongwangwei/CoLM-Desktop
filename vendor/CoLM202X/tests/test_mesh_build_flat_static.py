import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "share/MOD_Mesh.F90"


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
