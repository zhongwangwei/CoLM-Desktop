#!/usr/bin/env python3
"""Compile the production initialization routine; check traps and real NetCDF IO."""
from pathlib import Path
import os
import re
import shlex
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]


def main():
    text = (ROOT / 'vendor/CoLM202X/share/MOD_NetCDFSerial.F90').read_text()
    helper = re.search(r'   SUBROUTINE ncio_initialize \(\).*?END SUBROUTINE ncio_initialize',
                       text, re.S).group(0)
    for path in ['mksrfdata/MKSRFDATA.F90', 'mkinidata/CoLMINI.F90', 'main/CoLM.F90']:
        source = (ROOT / 'vendor/CoLM202X' / path).read_text()
        assert source.index('CALL ncio_initialize ()') < source.index('CALL read_namelist (nlfile)'), path
    module = '''module initialization_test
use, intrinsic :: ieee_exceptions
implicit none
integer :: expected_status = 0
contains
''' + helper + '''
subroutine nccheck(status)
integer, intent(in) :: status
logical :: halting, flag
call ieee_get_halting_mode(ieee_invalid, halting)
if (.not. halting) error stop 'invalid trap lost before error handling'
call ieee_get_halting_mode(ieee_divide_by_zero, halting)
if (.not. halting) error stop 'divide-by-zero trap lost before error handling'
call ieee_get_halting_mode(ieee_overflow, halting)
if (.not. halting) error stop 'overflow trap lost before error handling'
call ieee_get_flag(ieee_invalid, flag)
if (flag) error stop 'initialization leaked invalid flag'
if (status /= expected_status) error stop 'initialization status lost'
end subroutine
end module
'''
    program = '''program test
use initialization_test
use netcdf
implicit none
integer :: status, ncid
logical :: halting
call ieee_set_halting_mode(ieee_all, .false.)
call ieee_set_flag(ieee_all, .false.)
call ieee_set_halting_mode(ieee_invalid, .true.)
call ieee_set_halting_mode(ieee_divide_by_zero, .true.)
call ieee_set_halting_mode(ieee_overflow, .true.)
call ieee_set_flag(ieee_inexact, .true.)
#ifdef FAKE_INIT
expected_status = -42
#endif
call ncio_initialize()
call ieee_get_flag(ieee_invalid, halting)
if (halting) error stop 'initialization leaked invalid to caller'
call ieee_get_flag(ieee_inexact, halting)
if (.not. halting) error stop 'caller inexact flag lost'
#ifndef FAKE_INIT
status = nf90_create('probe.nc', ior(NF90_CLOBBER,NF90_NETCDF4), ncid)
if (status /= NF90_NOERR) error stop 'create failed'
status = nf90_close(ncid)
if (status /= NF90_NOERR) error stop 'close failed'
status = nf90_open('probe.nc', NF90_NOWRITE, ncid)
if (status /= NF90_NOERR) error stop 'open failed'
status = nf90_close(ncid)
if (status /= NF90_NOERR) error stop 'close failed'
#endif
call ieee_get_halting_mode(ieee_invalid, halting)
if (.not. halting) error stop 'invalid trap lost after IO'
print *, 'PASS: initialization restores IEEE status and traps'
end program
#ifdef FAKE_INIT
function nc_initialize() bind(C, name='nc_initialize') result(code)
use, intrinsic :: iso_c_binding
use, intrinsic :: ieee_exceptions
implicit none
integer(c_int) :: code
logical :: halting
call ieee_get_halting_mode(ieee_invalid, halting)
if (halting) error stop 'invalid trap active inside initialization'
call ieee_set_flag(ieee_invalid, .true.)
call ieee_set_flag(ieee_inexact, .false.)
code = -42
end function
#endif
'''
    fc = shlex.split(os.environ.get('FC', 'gfortran'))
    nf = os.environ.get('NF_CONFIG', 'nf-config')
    flags = shlex.split(subprocess.check_output([nf, '--fflags'], text=True))
    libs = shlex.split(subprocess.check_output([nf, '--flibs'], text=True))
    nc = os.environ.get('NC_CONFIG', 'nc-config')
    libs += shlex.split(subprocess.check_output([nc, '--libs'], text=True))
    with tempfile.TemporaryDirectory(prefix='colm-nc-init-') as temp:
        root = Path(temp)
        source = root / 'test.F90'
        source.write_text(module + program)
        for fake in [True, False]:
            exe = root / ('fake-init' if fake else 'real-init')
            command = fc + ['-cpp', '-ffpe-trap=invalid,zero,overflow']
            if fake:
                command.append('-DFAKE_INIT')
            subprocess.run(command + flags + [str(source)] + libs + ['-o', str(exe)], cwd=root, check=True)
            subprocess.run([str(exe)], cwd=root, check=True, timeout=30)
    print('PASS: all three entrypoints initialize NetCDF before input reads')


if __name__ == '__main__':
    main()
