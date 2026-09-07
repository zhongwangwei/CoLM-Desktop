#!/usr/bin/env python3
"""Compiled regression checks for narrow BGC/DA physics-audit fixes."""
from pathlib import Path
import os
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
MAIN = ROOT / "vendor/CoLM202X/main"
SHARE = ROOT / "vendor/CoLM202X/share"
INCLUDE = ROOT / "vendor/CoLM202X/include"


def read(rel: str) -> str:
    return (MAIN / rel).read_text()


def compiler() -> str:
    found = shutil.which("gfortran")
    if not found:
        raise SystemExit("gfortran is required for the physics audit regression")
    return found


def compile_and_run_fortran(source: str, *, extra_flags: list[str] | None = None) -> subprocess.CompletedProcess[str]:
    flags = extra_flags or ["-fcheck=all"]
    with tempfile.TemporaryDirectory(prefix="colm-physics-audit-") as directory:
        work = Path(directory)
        probe = work / "probe.f90"
        exe = work / ("probe.exe" if os.name == "nt" else "probe")
        probe.write_text(source)
        subprocess.run([compiler(), *flags, str(probe), "-o", str(exe)], check=True, cwd=work)
        return subprocess.run([str(exe)], cwd=work, text=True, capture_output=True)


def compile_and_run_with_time_manager(probe_source: str) -> subprocess.CompletedProcess[str]:
    with tempfile.TemporaryDirectory(prefix="colm-time-audit-") as directory:
        work = Path(directory)
        probe = work / "probe.f90"
        probe.write_text(probe_source)
        fc = compiler()
        flags = ["-cpp", "-ffree-form", f"-I{INCLUDE}"]
        subprocess.run(
            [
                fc,
                *flags,
                "-c",
                str(SHARE / "MOD_Precision.F90"),
                str(SHARE / "MOD_UserDefFun.F90"),
                str(SHARE / "MOD_TimeManager.F90"),
            ],
            check=True,
            cwd=work,
        )
        exe = work / ("probe.exe" if os.name == "nt" else "probe")
        subprocess.run(
            [fc, "MOD_Precision.o", "MOD_UserDefFun.o", "MOD_TimeManager.o", str(probe), "-o", str(exe)],
            check=True,
            cwd=work,
        )
        return subprocess.run([str(exe)], cwd=work, text=True, capture_output=True)


def assert_da_nextmonth_formula() -> None:
    src = read("DA/MOD_DA_TWS.F90")
    match = re.search(r"(?im)^\s*nextmonth\s*=\s*(.*?)\s*$", src)
    assert match, "GRACE nextmonth assignment not found"
    expr = match.group(1)
    out = compile_and_run_fortran(
        "program probe\n"
        "implicit none\n"
        "integer :: month, nextmonth\n"
        "integer :: expected(12)\n"
        "expected = (/2,3,4,5,6,7,8,9,10,11,12,1/)\n"
        "do month = 1, 12\n"
        f"   nextmonth = {expr}\n"
        "   if (nextmonth /= expected(month)) error stop 'bad nextmonth'\n"
        "end do\n"
        "print *, 'nextmonth ok'\n"
        "end program\n"
    )
    assert out.returncode == 0, out.stderr
    assert "nextmonth ok" in out.stdout


def assert_bgc_sasu_uses_actual_endofyear_gate() -> None:
    src = read("BGC/MOD_BGC_CNSASU.F90")
    assert "USE MOD_TimeManager, only: isendofyear" in src
    assert re.search(r"(?i)IF\s*\(\s*isendofyear\s*\(\s*idate\s*,\s*deltim\s*\)\s*\)\s*THEN", src)
    assert not re.search(r"(?i)idate\s*\(\s*2\s*\)\s*\.eq\.\s*365", src)
    out = compile_and_run_with_time_manager(
        "program probe\n"
        "use MOD_TimeManager, only: isendofyear\n"
        "implicit none\n"
        "integer :: common_final(3), leap_final(3), leap_not_final(3)\n"
        "real(8) :: deltim\n"
        "deltim = 1800.0d0\n"
        "common_final = (/2023, 365, 86400/)\n"
        "leap_final = (/2024, 366, 86400/)\n"
        "leap_not_final = (/2024, 365, 86400/)\n"
        "if (.not. isendofyear(common_final, deltim)) error stop 'common year final step missed'\n"
        "if (.not. isendofyear(leap_final, deltim)) error stop 'leap year final step missed'\n"
        "if (isendofyear(leap_not_final, deltim)) error stop 'leap day 365 is not year end'\n"
        "print *, 'actual isendofyear ok'\n"
        "end program\n"
    )
    assert out.returncode == 0, out.stderr
    assert "actual isendofyear ok" in out.stdout


def subroutine_body(src: str, name: str) -> str:
    match = re.search(
        rf"(?is)\bSUBROUTINE\s+{name}\b(?P<body>.*?)\bEND\s+SUBROUTINE\s+{name}\b",
        src,
    )
    assert match, f"subroutine {name} not found"
    return match.group("body")


def assert_synop_qref_preserves_fractional_humidity() -> None:
    src = read("DA/MOD_DA_SM.F90")
    match = re.search(r"(?im)^\s*(real\(r8\)\s*,\s*allocatable\s*::\s*synop_qref\s*\(:\).*)$", src)
    assert match, "real synop_qref declaration not found"
    assert not re.search(r"(?im)^\s*integer\s*,\s*allocatable\s*::\s*synop_qref\s*\(:\)", src)
    decl = match.group(1)
    out = compile_and_run_fortran(
        "program probe\n"
        "implicit none\n"
        "integer, parameter :: r8 = selected_real_kind(12)\n"
        f"{decl}\n"
        "allocate(synop_qref(1))\n"
        "synop_qref(1) = 0.0123456789_r8\n"
        "if (abs(synop_qref(1) - 0.0123456789_r8) > 1.0e-12_r8) error stop 'qref fraction lost'\n"
        "print *, 'qref fraction ok'\n"
        "end program\n"
    )
    assert out.returncode == 0, out.stderr
    assert "qref fraction ok" in out.stdout
    end_body = subroutine_body(src, "end_DA_SM")
    assert "IF (allocated (synop_lut)) deallocate (synop_lut)" in end_body


def assert_da_lifecycle_deallocates_owned_arrays() -> None:
    tws = read("DA/MOD_DA_TWS.F90")
    for name in ["obsyear", "obsmonth", "zwt_acc_prev_m", "zwt_acc_this_m"]:
        assert f"IF (allocated({name}" in tws or f"IF (allocated({name} " in tws, f"missing deallocate for {name}"


def extract_ensemble_guard() -> str:
    src = read("DA/MOD_DA_Ensemble.F90")
    guard = re.search(
        r"(?is)(IF\s*\(\s*mod\s*\(\s*DEF_DA_ENS_NUM\s*,\s*2\s*\)\s*/=\s*0\s*\)\s*THEN.*?END\s*IF)",
        src,
    )
    assert guard, "odd DEF_DA_ENS_NUM must fail fast before Box-Muller pair loops"
    assert src.find(guard.group(1)) < src.find("CALL random_number(u1)")
    return guard.group(1)


def run_ensemble_guard_probe(ensemble_count: int, expect_stopped: bool) -> None:
    guard = extract_ensemble_guard()
    out = compile_and_run_fortran(
        "program probe\n"
        "implicit none\n"
        f"integer, parameter :: DEF_DA_ENS_NUM = {ensemble_count}\n"
        "logical :: stopped\n"
        "stopped = .false.\n"
        f"{guard}\n"
        f"if (stopped .neqv. {'.true.' if expect_stopped else '.false.'}) error stop 'bad odd/even ensemble gate'\n"
        "print *, 'ensemble gate ok'\n"
        "contains\n"
        "subroutine CoLM_stop(message)\n"
        "character(len=*), intent(in), optional :: message\n"
        "stopped = .true.\n"
        "end subroutine CoLM_stop\n"
        "end program\n"
    )
    assert out.returncode == 0, out.stderr
    assert "ensemble gate ok" in out.stdout


def assert_odd_ensemble_rejected_even_accepted() -> None:
    run_ensemble_guard_probe(5, True)
    run_ensemble_guard_probe(4, False)


def assert_grace_first_observation_avoids_zero_divide() -> None:
    src = read("DA/MOD_DA_TWS.F90")
    block = re.search(
        r"(?is)(IF\s*\(\s*has_prev_grace_obs\s*\)\s*THEN\s*\n\s*zwt_acc_prev_m\s*=\s*zwt_acc_prev_m\s*/\s*nac_grace_prev\s*\n\s*rnof_prev_m1\s*=\s*rnof_prev_m1\s*/\s*nac_grace_this\s*\n\s*ENDIF)",
        src,
    )
    assert block, "GRACE previous-month divisions must be under has_prev_grace_obs"
    out = compile_and_run_fortran(
        "program probe\n"
        "implicit none\n"
        "logical :: has_prev_grace_obs\n"
        "real(8) :: zwt_acc_prev_m(1), rnof_prev_m1(1), nac_grace_this\n"
        "integer :: nac_grace_prev\n"
        "zwt_acc_prev_m = 1.0d0\n"
        "rnof_prev_m1 = 1.0d0\n"
        "nac_grace_prev = 0\n"
        "nac_grace_this = 1.0d0\n"
        "has_prev_grace_obs = .false.\n"
        f"{block.group(1)}\n"
        "if (zwt_acc_prev_m(1) /= 1.0d0) error stop 'unexpected first-observation mutation'\n"
        "has_prev_grace_obs = .true.\n"
        "nac_grace_prev = 1\n"
        f"{block.group(1)}\n"
        "print *, 'grace first observation ok'\n"
        "end program\n",
        extra_flags=["-fcheck=all", "-ffpe-trap=zero,invalid,overflow"],
    )
    assert out.returncode == 0, out.stderr
    assert "grace first observation ok" in out.stdout




def extract_balanced_if_block(src: str, start_regex: str) -> str:
    lines = src.splitlines()
    start_index = None
    pattern = re.compile(start_regex, re.IGNORECASE)
    for idx, line in enumerate(lines):
        if pattern.search(line):
            start_index = idx
            break
    assert start_index is not None, f"IF block start not found: {start_regex}"
    depth = 0
    block: list[str] = []
    for line in lines[start_index:]:
        block.append(line)
        stripped = line.strip()
        starts_if = re.search(r"\bIF\b.*\bTHEN\b", stripped, re.IGNORECASE) and not re.match(r"\s*ELSE\s*IF\b", stripped, re.IGNORECASE)
        if starts_if:
            depth += 1
        if re.search(r"\bEND\s*IF\b|\bENDIF\b", stripped, re.IGNORECASE):
            depth -= 1
            if depth == 0:
                return "\n".join(block)
    raise AssertionError(f"unterminated IF block: {start_regex}")


def assert_lulcc_missing_source_stops_before_frnp_use() -> None:
    src = read("LULCC/MOD_Lulcc_MassEnergyConserve.F90")
    block = extract_balanced_if_block(src, r"IF\s*\(\s*lccpct_np\s*\(\s*ilc\s*\)\s*\.gt\.\s*0\s*\)\s*THEN")
    assert "source patch not found" in block
    assert re.search(r"(?i)CALL\s+CoLM_stop\s*\(\s*['\"]LULCC source patch not found", block)
    out = compile_and_run_fortran(
        "program probe\n"
        "implicit none\n"
        "integer, parameter :: nlc = 3\n"
        "integer :: ilc, k, inp_, np, np_, j, selfnp_\n"
        "integer :: grid_patch_s_(1), grid_patch_e_(1), patchclass(1), patchclass_(2), frnp_(2)\n"
        "real(8) :: lccpct_np(nlc)\n"
        "logical :: FOUND, stopped\n"
        "stopped = .false.\n"
        "np = 1; np_ = 1; j = 1\n"
        "patchclass(1) = 1\n"
        "grid_patch_s_(1) = 1; grid_patch_e_(1) = 2\n"
        "lccpct_np = 0.0d0; lccpct_np(1) = 0.5d0; lccpct_np(2) = 0.5d0\n"
        "patchclass_ = (/1, 2/)\n"
        "k = 0; selfnp_ = -1; frnp_ = -999\n"
        "do ilc = 1, nlc\n"
        f"{block}\n"
        "end do\n"
        "if (stopped) error stop 'complete source mapping should not stop'\n"
        "if (k /= 2) error stop 'complete source mapping count changed'\n"
        "if (frnp_(1) /= 1 .or. frnp_(2) /= 2) error stop 'complete source mapping not preserved'\n"
        "patchclass_ = (/1, 3/)\n"
        "stopped = .false.; k = 0; selfnp_ = -1; frnp_ = -999\n"
        "do ilc = 1, nlc\n"
        f"{block}\n"
        "end do\n"
        "if (.not. stopped) error stop 'missing source mapping did not stop'\n"
        "print *, 'lulcc source mapping ok'\n"
        "contains\n"
        "subroutine CoLM_stop(message)\n"
        "character(len=*), intent(in), optional :: message\n"
        "stopped = .true.\n"
        "end subroutine CoLM_stop\n"
        "end program\n",
        extra_flags=["-fcheck=all", "-ffree-line-length-0"],
    )
    assert out.returncode == 0, out.stderr + out.stdout
    assert "lulcc source mapping ok" in out.stdout



def assert_lake_adjust_uses_actual_routine_for_zero_and_tiny_depths() -> None:
    src = read("MOD_Lake.F90")
    body_match = re.search(
        r"(?is)(SUBROUTINE\s+adjust_lake_layer\b.*?END\s+SUBROUTINE\s+adjust_lake_layer\b)",
        src,
    )
    assert body_match, "adjust_lake_layer routine not found"
    routine = body_match.group(1)
    assert re.search(r"(?i)USE\s*,\s*INTRINSIC\s*::\s*ieee_arithmetic\s*,\s*only\s*:\s*ieee_is_finite", routine)
    assert re.search(r"(?i)USE\s+MOD_SPMD_Task\s*,\s*only\s*:\s*CoLM_stop", routine)
    assert re.search(r"(?i)Lake layer thickness must be finite", routine)
    assert re.search(r"(?i)ieee_is_finite\s*\(\s*dz_lake\s*\)", routine)
    assert re.search(r"(?i)any\s*\(\s*dz_lake\s*<\s*0\._r8\s*\)", routine)
    assert re.search(r"(?i)IF\s*\(\s*wdsrfm\s*<=\s*0\._r8\s*\)\s*RETURN", routine)
    assert re.search(r"(?i)DO\s+WHILE\s*\(\s*resi\s*>\s*0\._r8\s*\)", routine)
    assert "epsilon(1._r8)*max(1._r8, wdsrfm)" not in routine

    precision = (SHARE / "MOD_Precision.F90").read_text()
    constants = (MAIN / "MOD_Const_Physical.F90").read_text()
    positive_source = (
        f"{precision}\n"
        f"{constants}\n"
        "module MOD_SPMD_Task\n"
        "implicit none\n"
        "contains\n"
        "subroutine CoLM_stop(message)\n"
        "character(len=*), intent(in), optional :: message\n"
        "error stop 'CoLM_stop called'\n"
        "end subroutine CoLM_stop\n"
        "end module MOD_SPMD_Task\n"
        "module lake_probe_mod\n"
        "use MOD_Precision\n"
        "implicit none\n"
        "contains\n"
        f"{routine}\n"
        "end module lake_probe_mod\n"
        "program probe\n"
        "use, intrinsic :: ieee_arithmetic, only: ieee_value, ieee_quiet_nan\n"
        "use MOD_Precision\n"
        "use lake_probe_mod, only: adjust_lake_layer\n"
        "implicit none\n"
        "integer, parameter :: nl_lake = 10\n"
        "integer :: i\n"
        "real(r8) :: dz(nl_lake), t(nl_lake), ice(nl_lake), t_before(nl_lake), ice_before(nl_lake)\n"
        "dz = 0.1_r8; t = 274._r8; ice = 0._r8\n"
        "call adjust_lake_layer(nl_lake, dz, t, ice)\n"
        "if (abs(sum(dz) - 1._r8) > 1.e-12_r8) error stop 'valid positive remap changed total depth'\n"
        "if (any(t /= t)) error stop 'valid positive remap produced NaN temperature'\n"
        "if (any(ice < 0._r8) .or. any(ice > 1._r8)) error stop 'valid positive remap produced bad ice fraction'\n"
        "do i = 1, nl_lake\n"
        "   dz(i) = real(i, r8) * 1.e-3_r8\n"
        "   t(i) = 272._r8 + 0.25_r8*real(i, r8)\n"
        "   ice(i) = merge(0.35_r8, 0.0_r8, mod(i, 2) == 0)\n"
        "end do\n"
        "call adjust_lake_layer(nl_lake, dz, t, ice)\n"
        "if (abs(sum(dz) - 0.055_r8) > 1.e-12_r8) error stop 'mixed remap changed total depth'\n"
        "if (any(t /= t)) error stop 'mixed remap produced NaN temperature'\n"
        "if (any(ice < 0._r8) .or. any(ice > 1._r8)) error stop 'mixed remap produced bad ice fraction'\n"
        "dz = 1.e-21_r8; t = 274._r8; ice = 0._r8\n"
        "call adjust_lake_layer(nl_lake, dz, t, ice)\n"
        "if (abs(sum(dz) - 1.e-20_r8) > 1.e-30_r8) error stop 'tiny positive remap changed total depth'\n"
        "if (any(t /= t)) error stop 'tiny positive remap left temperature undefined'\n"
        "if (any(abs(t - 274._r8) > 1.e-12_r8)) error stop 'tiny positive remap did not preserve uniform temperature'\n"
        "if (any(abs(ice) > 1.e-12_r8)) error stop 'tiny positive remap did not preserve liquid state'\n"
        "dz = 0._r8\n"
        "do i = 1, nl_lake\n"
        "   t(i) = 270._r8 + real(i, r8)\n"
        "end do\n"
        "ice = 0.25_r8\n"
        "t_before = t; ice_before = ice\n"
        "call adjust_lake_layer(nl_lake, dz, t, ice)\n"
        "if (any(dz /= 0._r8)) error stop 'zero-depth dry no-op changed thickness'\n"
        "if (any(abs(t - t_before) > 0._r8)) error stop 'zero-depth dry no-op changed temperature'\n"
        "if (any(abs(ice - ice_before) > 0._r8)) error stop 'zero-depth dry no-op changed ice fraction'\n"
        "print *, 'lake adjust extracted routine ok'\n"
        "end program\n"
    )
    out = compile_and_run_fortran(
        positive_source,
        extra_flags=["-fcheck=all", "-finit-real=snan", "-ffpe-trap=zero,invalid,overflow", "-ffree-line-length-0"],
    )
    assert out.returncode == 0, out.stderr + out.stdout
    assert "lake adjust extracted routine ok" in out.stdout

    negative_source = positive_source.replace(
        "print *, 'lake adjust extracted routine ok'\nend program\n",
        "dz = 0.1_r8; dz(3) = -0.01_r8\n"
        "call adjust_lake_layer(nl_lake, dz, t, ice)\n"
        "error stop 'negative thickness did not reject'\n"
        "end program\n",
    )
    negative = compile_and_run_fortran(
        negative_source,
        extra_flags=["-fcheck=all", "-ffpe-trap=zero,invalid,overflow", "-ffree-line-length-0"],
    )
    assert negative.returncode != 0, "negative lake thickness should call CoLM_stop"
    assert "CoLM_stop called" in negative.stderr


    nan_source = positive_source.replace(
        "print *, 'lake adjust extracted routine ok'\nend program\n",
        "dz = 0.1_r8; dz(2) = ieee_value(0._r8, ieee_quiet_nan)\n"
        "call adjust_lake_layer(nl_lake, dz, t, ice)\n"
        "error stop 'NaN thickness did not reject'\n"
        "end program\n",
    )
    finite = compile_and_run_fortran(
        nan_source,
        extra_flags=["-fcheck=all", "-ffree-line-length-0"],
    )
    assert finite.returncode != 0, "NaN lake thickness should call CoLM_stop"
    assert "CoLM_stop called" in finite.stderr

def main() -> None:
    assert_da_nextmonth_formula()
    assert_bgc_sasu_uses_actual_endofyear_gate()
    assert_synop_qref_preserves_fractional_humidity()
    assert_da_lifecycle_deallocates_owned_arrays()
    assert_odd_ensemble_rejected_even_accepted()
    assert_grace_first_observation_avoids_zero_divide()
    assert_lulcc_missing_source_stops_before_frnp_use()
    assert_lake_adjust_uses_actual_routine_for_zero_and_tiny_depths()
    print("physics audit regressions PASS")


if __name__ == "__main__":
    main()
