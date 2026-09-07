#!/usr/bin/env python3
"""Regression for CaMa CMF_CheckNanB nonfinite handling under FPE traps."""
from pathlib import Path
import os
import re
import shutil
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "vendor/CoLM202X/extends/CaMa/src"
CMF_UTILS = SOURCE / "cmf_utils_mod.F90"


CALL_RE = re.compile(r"CMF_CheckNanB\s*\(", re.IGNORECASE)
ZERO_ARG_RE = re.compile(r"CMF_CheckNanB\s*\(.*?,\s*0\._JPRB\s*\)", re.IGNORECASE)
FUNC_RE = re.compile(
    r"FUNCTION\s+CMF_CheckNanB\s*\(.*?END\s+FUNCTION\s+CMF_CheckNanB",
    re.IGNORECASE | re.DOTALL,
)


def run(command, *, cwd, check=True):
    return subprocess.run(
        command,
        cwd=cwd,
        check=check,
        capture_output=True,
        text=True,
        timeout=120,
    )


def assert_all_callers_keep_zero_argument() -> None:
    calls = []
    for path in sorted(SOURCE.glob("*.F90")):
        text = path.read_text(encoding="utf-8", errors="ignore")
        for lineno, line in enumerate(text.splitlines(), 1):
            if "CMF_CheckNanB" not in line or line.lstrip().startswith("!") or "FUNCTION" in line.upper():
                continue
            if CALL_RE.search(line):
                calls.append((path.relative_to(ROOT), lineno, line.strip()))
    assert calls, "no active CMF_CheckNanB callers found"
    for path, lineno, line in calls:
        normalized = line.replace(" ", "")
        assert ZERO_ARG_RE.search(normalized), (
            f"{path}:{lineno} does not pass zero as CMF_CheckNanB second argument: {line!r}"
        )


def extracted_production_function() -> str:
    text = CMF_UTILS.read_text(encoding="utf-8")
    match = FUNC_RE.search(text)
    assert match, "production CMF_CheckNanB function not found"
    function = match.group(0)
    assert "IEEE_IS_FINITE" in function.upper(), "production helper must use IEEE_IS_FINITE"
    executable_lines = "\n".join(line.split("!", 1)[0] for line in function.splitlines())
    assert "VAR*zero" not in executable_lines.replace(" ", ""), "trap-prone VAR*zero check remains"
    return function


def main() -> None:
    compiler = shutil.which("gfortran")
    if not compiler:
        raise SystemExit("gfortran is required for the CaMa finite regression")

    assert_all_callers_keep_zero_argument()
    production_function = extracted_production_function()

    with tempfile.TemporaryDirectory(prefix="cama-finite-") as directory:
        work = Path(directory)
        (work / "old_semantics.f90").write_text(
            "program old_semantics\n"
            "  use, intrinsic :: ieee_arithmetic\n"
            "  implicit none\n"
            "  real(8) :: finite, zero, nanv, pinf, ninf\n"
            "  finite = 1.25_8\n"
            "  zero = 0._8\n"
            "  nanv = ieee_value(nanv, ieee_quiet_nan)\n"
            "  pinf = ieee_value(pinf, ieee_positive_inf)\n"
            "  ninf = ieee_value(ninf, ieee_negative_inf)\n"
            "  if (old_bad(finite, zero)) error stop 10\n"
            "  if (.not. old_bad(nanv, zero)) error stop 11\n"
            "  if (.not. old_bad(pinf, zero)) error stop 12\n"
            "  if (.not. old_bad(ninf, zero)) error stop 13\n"
            "  print '(A)', 'old zero semantics: finite=F NaN=T +Inf=T -Inf=T'\n"
            "contains\n"
            "  logical function old_bad(var, zero)\n"
            "    real(8), intent(in) :: var, zero\n"
            "    old_bad = .false.\n"
            "    if (var*zero /= zero) old_bad = .true.\n"
            "  end function old_bad\n"
            "end program old_semantics\n",
            encoding="utf-8",
        )
        old_exe = work / ("old_semantics.exe" if os.name == "nt" else "old_semantics")
        run([compiler, "old_semantics.f90", "-o", str(old_exe)], cwd=work)
        old_result = run([str(old_exe)], cwd=work)
        assert "+Inf=T" in old_result.stdout and "-Inf=T" in old_result.stdout

        old_trap_exe = work / ("old_semantics_trap.exe" if os.name == "nt" else "old_semantics_trap")
        run([compiler, "-ffpe-trap=invalid", "old_semantics.f90", "-o", str(old_trap_exe)], cwd=work)
        old_trap = run([str(old_trap_exe)], cwd=work, check=False)
        assert old_trap.returncode != 0, "old VAR*zero semantics did not trip invalid FPE under traps"

        (work / "production_probe.f90").write_text(
            "module PARKIND1\n"
            "  implicit none\n"
            "  integer, parameter :: JPRB = selected_real_kind(12)\n"
            "end module PARKIND1\n"
            "module extracted_cmf_utils\n"
            "  use PARKIND1, only: JPRB\n"
            "  implicit none\n"
            "contains\n"
            f"{production_function}\n"
            "end module extracted_cmf_utils\n"
            "program production_probe\n"
            "  use, intrinsic :: ieee_arithmetic\n"
            "  use PARKIND1, only: JPRB\n"
            "  use extracted_cmf_utils, only: CMF_CheckNanB\n"
            "  implicit none\n"
            "  real(JPRB) :: finite, zero, nanv, pinf, ninf\n"
            "  zero = 0._JPRB\n"
            "  finite = 1.25_JPRB\n"
            "  nanv = ieee_value(nanv, ieee_quiet_nan)\n"
            "  pinf = ieee_value(pinf, ieee_positive_inf)\n"
            "  ninf = ieee_value(ninf, ieee_negative_inf)\n"
            "  if (CMF_CheckNanB(finite, zero)) error stop 20\n"
            "  if (.not. CMF_CheckNanB(nanv, zero)) error stop 21\n"
            "  if (.not. CMF_CheckNanB(pinf, zero)) error stop 22\n"
            "  if (.not. CMF_CheckNanB(ninf, zero)) error stop 23\n"
            "  print '(A)', 'production CMF_CheckNanB: finite=F NaN=T +Inf=T -Inf=T'\n"
            "end program production_probe\n",
            encoding="utf-8",
        )
        production_exe = work / ("production_probe.exe" if os.name == "nt" else "production_probe")
        run(
            [
                compiler,
                "-ffpe-trap=invalid",
                "-fcheck=all",
                "production_probe.f90",
                "-o",
                str(production_exe),
            ],
            cwd=work,
        )
        production_result = run([str(production_exe)], cwd=work)
        assert "finite=F NaN=T +Inf=T -Inf=T" in production_result.stdout

    print("cama finite: CMF_CheckNanB preserves zero-arg nonfinite classification without VAR*0 FPE PASS")


if __name__ == "__main__":
    main()
