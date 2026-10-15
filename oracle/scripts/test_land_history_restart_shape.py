#!/usr/bin/env python3
"""Exercise the actual sidecar shape guard with flat/grouped block layouts."""
from pathlib import Path
import os
import re
import shlex
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]


def main():
    source = (ROOT / 'vendor/CoLM202X/include/land_history_restart.inc').read_text()
    helper = re.search(r'   SUBROUTINE history_acc_require_shape\(.*?END SUBROUTINE history_acc_require_shape',
                       source, re.S).group(0)
    support = '''module MOD_SPMD_Task
logical :: p_is_io = .true.
contains
subroutine CoLM_stop()
stop 9
end subroutine
end module
module MOD_Block
contains
subroutine get_filename_block(file, iblk, jblk, block_file)
character(len=*), intent(in) :: file
integer, intent(in) :: iblk, jblk
character(len=*), intent(out) :: block_file
block_file = file
end subroutine
end module
module MOD_Pixelset
type vector_type
integer :: vlen(1,1), vcnt(0:3,1,1)
end type
type pixelset_type
integer :: nblkgrp=1, xblkgrp(1)=1, yblkgrp(1)=1
type(vector_type) :: vecgs
end type
end module
module shape_guard
contains
''' + helper + '''
end module
program test
use netcdf
use MOD_Pixelset
use shape_guard
implicit none
type(pixelset_type) :: pixels
integer :: ncid, varid, dims(3), rank, file_rank, extent, inner, dtype, i, status
character(len=32) :: mode, argument, vector_dim, name, dimname
call get_command_argument(1, mode)
call get_command_argument(2, argument)
read(argument,*) rank
call get_command_argument(3, vector_dim)
name = 'history_schema'
pixels%vecgs%vcnt(:,1,1) = [1,0,2,1]
#ifdef FLAT_SPMD
pixels%vecgs%vlen(1,1) = 1
#else
pixels%vecgs%vlen(1,1) = 4
#endif
extent = 4
inner = 2
dtype = NF90_DOUBLE
file_rank = rank
if (mode == 'length') extent = 1
if (mode == 'inner') inner = 5
if (mode == 'type') dtype = NF90_INT
if (mode == 'rank') file_rank = rank + 1
call check(nf90_create('sidecar.nc', NF90_CLOBBER, ncid))
do i=1,file_rank
  if (i==file_rank) then
    dimname = vector_dim
    if (mode == 'name') dimname = 'wrong_vector'
    call check(nf90_def_dim(ncid, trim(dimname), extent, dims(i)))
  else
    write(dimname,'(A,I1,A)') 'd',i,'_'//trim(name)
    if (i==1) then
      call check(nf90_def_dim(ncid, trim(dimname), inner, dims(i)))
    else
      call check(nf90_def_dim(ncid, trim(dimname), 3, dims(i)))
    endif
  endif
enddo
call check(nf90_def_var(ncid, trim(name), dtype, dims(:file_rank), varid))
call check(nf90_close(ncid))
call history_acc_require_shape('sidecar.nc', trim(name), pixels, trim(vector_dim), rank, 2, 3)
print *, 'ACCEPTED'
contains
subroutine check(code)
integer, intent(in) :: code
if (code/=NF90_NOERR) then
 print *, nf90_strerror(code)
 stop 2
endif
end subroutine
end program
'''
    fc = shlex.split(os.environ.get('FC', 'gfortran'))
    nf = os.environ.get('NF_CONFIG', 'nf-config')
    nc = os.environ.get('NC_CONFIG', 'nc-config')
    flags = shlex.split(subprocess.check_output([nf, '--fflags'], text=True))
    libs = shlex.split(subprocess.check_output([nf, '--flibs'], text=True))
    libs += shlex.split(subprocess.check_output([nc, '--libs'], text=True))
    count = 0
    with tempfile.TemporaryDirectory(prefix='colm-restart-shape-') as temp:
        root = Path(temp)
        path = root / 'test.F90'
        path.write_text(support)
        for flat in [True, False]:
            exe = root / ('flat' if flat else 'grouped')
            defines = ['-DFLAT_SPMD'] if flat else []
            subprocess.run(fc + ['-cpp', '-fcheck=all'] + defines + flags + [str(path)] + libs + ['-o', str(exe)],
                           cwd=root, check=True, timeout=60)
            cases = [('valid', rank, dim) for rank in [1, 2, 3] for dim in ['patch', 'urban']]
            cases += [(mode, 1, 'patch') for mode in ['length', 'type', 'rank', 'name']]
            cases += [('inner', 2, 'patch')]
            for mode, rank, dim in cases:
                result = subprocess.run([str(exe), mode, str(rank), dim], cwd=root,
                                        capture_output=True, text=True, timeout=10)
                expected = 0 if mode == 'valid' else 9
                assert result.returncode == expected, (flat, mode, rank, dim, result.stdout, result.stderr)
                if expected:
                    assert 'incompatible land-history sidecar shape' in result.stdout
                count += 1
    print(f'PASS: {count} flat/grouped sidecar shape cases (valid rank 1–3 patch/urban and invalid layouts)')


if __name__ == '__main__':
    main()
