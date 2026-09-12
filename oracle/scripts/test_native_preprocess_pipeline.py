#!/usr/bin/env python3
"""Run Rust mksrfdata -> mkinidata on the generated CN-Cng fixture."""

from pathlib import Path
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[2]
CASE = ROOT / "oracle/work/generated/case.nml"


def main() -> None:
    with tempfile.TemporaryDirectory(prefix="colm-native-preprocess-") as temp:
        temp = Path(temp)
        output = temp / "out"
        case = temp / "case.nml"
        case.write_text(CASE.read_text().replace(
            f"DEF_dir_output = '{ROOT}/oracle/work/generated/out/'",
            f"DEF_dir_output = '{output}/'",
        ))
        for package, binary in (("colm-srfdata", "mksrfdata-rs"), ("colm-init", "mkinidata-rs")):
            subprocess.run(
                ["cargo", "run", "-q", "-p", package, "--bin", binary, "--", str(case), "--land-cover", "igbp"],
                cwd=ROOT,
                check=True,
            )
        restart = output / "CN-Cng/restart"
        expected = [
            output / "CN-Cng/landdata/srfdata.nc",
            restart / "const/CN-Cng_restart_const_lc2005.nc",
            restart / "const/CN-Cng_restart_const_lc2005_w180_s90.nc",
            restart / "2008-001-00000/CN-Cng_restart_2008-001-00000_lc2005_w180_s90.nc",
        ]
        missing = [str(path) for path in expected if not path.is_file()]
        if missing:
            raise SystemExit("missing native output:\n" + "\n".join(missing))


if __name__ == "__main__":
    main()
