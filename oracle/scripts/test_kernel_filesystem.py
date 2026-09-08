#!/usr/bin/env python3
"""Compile the production filesystem helper; never run paths through a test shell."""
from pathlib import Path
import ntpath
import os
import re
import shutil
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "vendor/CoLM202X"


def main():
    compiler = shutil.which("gfortran")
    if not compiler:
        raise SystemExit("gfortran is required for the kernel filesystem regression")
    namelist = (SOURCE / "share/MOD_Namelist.F90").read_text()
    call = re.search(r"(?im)^\s*CALL (?:system|make_directory)\([^\n]*DEF_dir_output[^\n]*", namelist)
    assert call, "production directory creation call not found"
    with tempfile.TemporaryDirectory(prefix="colm-filesystem-") as directory:
        work = Path(directory)
        module = SOURCE / "share/MOD_Filesystem.F90"
        assert module.exists(), "MOD_Filesystem.F90 is required"

        (work / "spmd.f90").write_text(
            "module MOD_SPMD_Task\ncontains\nsubroutine CoLM_stop(message)\n"
            "character(*), intent(in), optional :: message\nif (present(message)) print *, message\n"
            "error stop 1\nend subroutine\nend module\n"
        )
        subprocess.run(
            [os.environ.get("CC", "cc"), "-c", str(SOURCE / "share/CoLM_Mkdir.c"), "-o", "filesystem.o"],
            cwd=work, check=True,
        )
        (work / "probe.f90").write_text(
            "program probe\n"
            "use MOD_Filesystem, only: make_directory, copy_file, list_matching_paths, move_file\n"
            "implicit none\ncharacter(1024) :: mode, a, b, c, DEF_dir_output\n"
            "call get_command_argument(1, mode)\n"
            "select case (trim(mode))\n"
            "case ('mkdir')\ncall get_command_argument(2, DEF_dir_output)\n"
            + call.group(0) + "\n"
            "case ('copy')\ncall get_command_argument(2, a)\ncall get_command_argument(3, b)\n"
            "call copy_file(trim(a), trim(b))\n"
            "case ('list')\ncall get_command_argument(2, a)\ncall get_command_argument(3, b)\n"
            "call get_command_argument(4, c)\ncall list_matching_paths(trim(a), trim(b), trim(c))\n"
            "case ('move')\ncall get_command_argument(2, a)\ncall get_command_argument(3, b)\n"
            "call move_file(trim(a), trim(b))\n"
            "case default\nerror stop 2\nend select\nend program\n"
        )
        executable = work / ("probe.exe" if os.name == "nt" else "probe")
        subprocess.run(
            [compiler, "-cpp", "-fcheck=all", "spmd.f90", str(module), "filesystem.o", "probe.f90", "-o", str(executable)],
            cwd=work, check=True,
        )
        env = {**os.environ, "COLM_MKDIR_TEST": "expanded"}
        paths = ["space dir/child", "O'Brien & data/child", "percent%COLM_MKDIR_TEST%/child"]
        paths += [str(work / "absolute dir/child"), "trailing/child/", "./parent/../sibling/child"]
        if os.name != "nt":
            paths += ["literal$(echo expanded)/child", "literal; touch INJECTED; #/child", "-leading/child", "literal\\backslash/child"]
        else:
            paths += ["backslash\\child/leaf"]
        for path in paths:
            for _ in range(2):
                result = subprocess.run([str(executable), "mkdir", path], cwd=work, env=env, capture_output=True, timeout=300)
                assert result.returncode == 0, (path, result.stdout, result.stderr)
                assert (work / path).is_dir(), f"directory was not created literally: {path!r}"
                assert not (work / "INJECTED").exists(), "path executed as a shell command"
        (work / "file").write_text("do not replace")
        for path in ["file", "file/child"]:
            result = subprocess.run([str(executable), "mkdir", path], cwd=work, capture_output=True, timeout=300)
            assert result.returncode != 0, f"file conflict was silently accepted: {path}"
            assert (work / "file").read_text() == "do not replace"

        src_dir = work / "copy O'Brien; touch INJECTED; #"
        dst_dir = work / "dest space"
        src_dir.mkdir()
        dst_dir.mkdir()
        src = src_dir / "mesh.nc"
        dst = dst_dir / "mesh copy.nc"
        payload = bytes(range(256)) * 8193  # Exercise more than two copy chunks.
        src.write_bytes(payload)
        result = subprocess.run([str(executable), "copy", str(src), str(dst)], cwd=work, capture_output=True, timeout=300)
        assert result.returncode == 0, result.stderr
        assert dst.read_bytes() == payload
        assert not (work / "INJECTED").exists(), "copy path executed as a shell command"

        result = subprocess.run([str(executable), "copy", str(src), str(src)], cwd=work, capture_output=True, timeout=300)
        assert result.returncode != 0, "copy onto the same path was allowed to truncate the source"
        assert src.read_bytes() == payload
        try:
            hardlink = work / "hardlink.nc"
            os.link(src, hardlink)
        except (AttributeError, NotImplementedError, OSError):
            hardlink = None
        if hardlink is not None:
            result = subprocess.run([str(executable), "copy", str(src), str(hardlink)], cwd=work, capture_output=True, timeout=300)
            assert result.returncode != 0, "copy onto a hardlink of the source was allowed"
            assert src.read_bytes() == payload

        result = subprocess.run([str(executable), "copy", str(src), str(work / "missing parent/out.nc")], cwd=work, capture_output=True, timeout=300)
        assert result.returncode != 0, "copy into a missing parent was silently accepted"

        list_dir = work / "list O'Brien & data"
        list_dir.mkdir()
        for name in ["LAI_0002.nc", "skip.txt", "LAI_0001.nc", "LAI_0003.tmp"]:
            (list_dir / name).write_text(name)
        list_file = work / "matches.txt"
        result = subprocess.run(
            [str(executable), "list", str(list_dir / "LAI_"), ".nc", str(list_file)],
            cwd=work, capture_output=True, timeout=300,
        )
        assert result.returncode == 0, result.stderr
        listed = list_file.read_text().splitlines()
        expected = [str(list_dir / "LAI_0001.nc"), str(list_dir / "LAI_0002.nc")]
        if os.name == "nt":
            # MSYS2 Python and native C may spell the same Windows path differently.
            listed = [ntpath.normpath(path) for path in listed]
            expected = [ntpath.normpath(path) for path in expected]
        assert listed == expected, (listed, expected)
        result = subprocess.run(
            [str(executable), "list", str(Path(work.anchor) / "colm_missing_532d0cae_"), ".nc", str(list_file)],
            cwd=work, capture_output=True, timeout=300,
        )
        assert result.returncode == 0, result.stderr
        assert list_file.read_bytes() == b"", "root/drive-root prefix was not handled literally"
        if os.name != "nt":
            backslash_dir = work / "list\\backslash"
            backslash_dir.mkdir()
            (backslash_dir / "LAI_0004.nc").write_text("backslash")
            result = subprocess.run(
                [str(executable), "list", str(backslash_dir / "LAI_"), ".nc", str(list_file)],
                cwd=work, capture_output=True, timeout=300,
            )
            assert result.returncode == 0, result.stderr
            assert list_file.read_text().splitlines() == [str(backslash_dir / "LAI_0004.nc")]

        moved = work / "moved O'Brien.nc"
        result = subprocess.run([str(executable), "move", str(dst), str(moved)], cwd=work, capture_output=True, timeout=300)
        assert result.returncode == 0, result.stderr
        assert moved.read_bytes() == payload
        assert not dst.exists(), "rename left the source behind"

        # A failed replacement must not destroy the existing destination.
        result = subprocess.run([str(executable), "move", str(work / "missing.nc"), str(moved)], cwd=work, capture_output=True, timeout=300)
        assert result.returncode != 0, "missing rename source was accepted"
        assert moved.read_bytes() == payload, "failed rename destroyed the destination"
        dst.write_bytes(b"replacement")
        result = subprocess.run([str(executable), "move", str(dst), str(moved)], cwd=work, capture_output=True, timeout=300)
        assert result.returncode == 0, result.stderr
        assert moved.read_bytes() == b"replacement"

        result = subprocess.run([str(executable), "copy", str(src_dir), str(moved)], cwd=work, capture_output=True, timeout=300)
        assert result.returncode != 0, "copy accepted a directory source"
        assert moved.read_bytes() == b"replacement", "invalid copy source destroyed the destination"

    source_text = "\n".join(
        source.read_text(errors="ignore")
        for source in SOURCE.rglob("*.F90")
        if ".bld" not in source.parts
    )
    assert not re.search(r"(?im)^\s*CALL\s+(?:system|execute_command_line)\s*\([^\n]*(mkdir|cp|mv|ls|rm)\b", source_text)
    print("kernel filesystem: mkdir, copy, list, rename literal paths PASS")


if __name__ == "__main__":
    main()
