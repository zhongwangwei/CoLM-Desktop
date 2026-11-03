import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def test_flat_tracer_vector_history_reuses_collective_gather() -> None:
    for grid in ("CATCHMENT", "UNSTRUCTURED"):
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
                    str(ROOT / "main/MATERIAL/MOD_MT_Hist.F90"),
                ],
                check=True,
                capture_output=True,
                text=True,
            ).stdout

        if grid == "CATCHMENT":
            assert "SUBROUTINE write_history_mt_vector_2d" not in source
            assert "SUBROUTINE write_history_mt_ratio_vector_3d" not in source
            continue

        for name in (
            "write_history_mt_vector_2d",
            "write_history_mt_ratio_vector_3d",
        ):
            body = source.split(f"SUBROUTINE {name}", 1)[1].split(
                f"END SUBROUTINE {name}", 1
            )[0]
            assert "CALL gather_history_fields" in body
            assert "CALL mpi_send" not in body
            assert "CALL mpi_recv" not in body

    helper = (ROOT / "main/MOD_HistVector.F90").read_text(encoding="utf-8")
    assert "PUBLIC :: gather_history_fields" in helper
    assert "CALL mpi_gatherv" in helper


def test_flat_material_vector_history_gathers_empty_and_reordered_ranks() -> None:
    import os
    import shutil

    compiler = shutil.which("mpifort")
    launcher = shutil.which("mpiexec")
    assert compiler and launcher, "MPI tools are required for the material history regression"
    with tempfile.TemporaryDirectory() as directory:
        work = Path(directory)
        (work / "define.h").write_text("#define UNSTRUCTURED\n#define USEMPI\n#define FLAT_SPMD\n")
        source = subprocess.run(
            ["cpp", "-P", "-traditional-cpp", f"-I{work}", f"-I{ROOT / 'include'}",
             str(ROOT / "main/MATERIAL/MOD_MT_Hist.F90")],
            check=True, capture_output=True, text=True,
        ).stdout
        helper = (ROOT / "main/MOD_HistVector.F90").read_text()
        def procedure(text: str, name: str) -> str:
            return "SUBROUTINE " + name + text.split("SUBROUTINE " + name, 1)[1].split(
                "END SUBROUTINE " + name, 1
            )[0] + "END SUBROUTINE " + name + "\n"

        stubs = """
module MOD_Precision
 integer, parameter :: r8=kind(1.d0)
end module
module MOD_Vars_Global
 use MOD_Precision
 real(r8), parameter :: spval=-1.d36
end module
module MOD_DataType
 type pointer_int32_1d
  integer, allocatable :: val(:)
 end type
end module
module MOD_SPMD_Task
 use mpi
 integer :: p_np_glb, p_root=0, p_comm_glb=MPI_COMM_WORLD, p_err, rank
 logical :: p_is_master, p_is_worker=.true.
contains
 subroutine CoLM_stop(message)
  character(*) :: message
  print *, message
  call MPI_ABORT(p_comm_glb, 1, p_err)
 end subroutine
end module
module MOD_Namelist
 integer :: DEF_HIST_CompressLevel=0
end module
module MOD_LandElm
 integer :: numelm, totalnumelm=3
end module
module MOD_ElmVector
 use MOD_DataType
 type(pointer_int32_1d), allocatable :: elm_data_address(:)
end module
module MOD_LandPatch
 use MOD_Precision
 type partition
  integer, allocatable :: substt(:), subend(:)
  real(r8), allocatable :: subfrc(:)
 end type
 type(partition) :: elm_patch
end module
module MOD_NetCDFSerial
 use MOD_Precision
 use MOD_SPMD_Task
 integer :: writes=0
 interface ncio_write_serial
  module procedure write1, write2
 end interface
 interface ncio_write_serial_time
  module procedure write_time1, write_time2
 end interface
 interface ncio_put_attr
  module procedure attr_text, attr_real
 end interface
contains
 subroutine write1(file, name, values, dim, compress)
  character(*) :: file, name, dim
  integer :: compress
  real(r8) :: values(:)
  if (.not.p_is_master) error stop 'non-master write'
  if (size(values)/=3) error stop 'wrong vector length'
  if (any(abs(values-[2.d0,3.d0,4.d0])>1.d-12)) error stop 'wrong 2D ratio/order'
  writes=writes+1
 end subroutine
 subroutine write2(file, name, values, dim, dim2, compress)
  character(*) :: file, name, dim, dim2
  integer :: compress
  real(r8) :: values(:,:)
  if (.not.p_is_master) error stop 'non-master write'
  if (any(shape(values)/=[2,3])) error stop 'wrong layered shape'
  if (any(abs(values(1,:)-[2.d0,3.d0,4.d0])>1.d-12)) error stop 'wrong layer 1'
  if (any(abs(values(2,:)-[4.d0,6.d0,8.d0])>1.d-12)) error stop 'wrong layer 2'
  writes=writes+1
 end subroutine
 subroutine write_time1(file, name, time, values, dim, tdim, compress)
  character(*) :: file, name, dim, tdim
  integer :: time, compress
  real(r8) :: values(:)
  call write1(file,name,values,dim,compress)
 end subroutine
 subroutine write_time2(file, name, time, values, dim, dim2, tdim, compress)
  character(*) :: file, name, dim, dim2, tdim
  integer :: time, compress
  real(r8) :: values(:,:)
  call write2(file,name,values,dim,dim2,compress)
 end subroutine
 subroutine ncio_define_dimension(file, name, n)
  character(*) :: file, name
  integer :: n
 end subroutine
 subroutine attr_text(file, name, attr, value)
  character(*) :: file,name,attr,value
 end subroutine
 subroutine attr_real(file, name, attr, value)
  character(*) :: file,name,attr
  real(r8) :: value
 end subroutine
end module
module MOD_HistVector
 use MOD_Precision
 use MOD_SPMD_Task
 use MOD_Vars_Global
 use MOD_DataType
contains
""" + procedure(helper, "gather_history_fields") + """
end module
module material_writers
 use MOD_Precision
 use MOD_Vars_Global
 use MOD_NetCDFSerial
 use MOD_HistVector
 real(r8), parameter :: mt_water_min_for_delta=1.d-12, mt_tiny=1.d-30, mt_delta_sanity_max=1.d9
contains
 real(r8) function mass_to_delta(m,w,ref)
  real(r8) :: m,w,ref
  mass_to_delta=(m/w/ref-1.d0)*1000.d0
 end function
""" + procedure(source, "write_history_mt_vector_2d") + procedure(
            source, "write_history_mt_ratio_vector_3d"
        ) + """
end module
program probe
 use material_writers
 use MOD_LandElm
 use MOD_ElmVector
 use MOD_LandPatch
 real(r8), allocatable :: mass(:),water(:),mass2(:,:),water2(:,:)
 logical, allocatable :: filter(:)
 integer :: i,g
 call MPI_INIT(p_err)
 call MPI_COMM_RANK(p_comm_glb,rank,p_err)
 call MPI_COMM_SIZE(p_comm_glb,p_np_glb,p_err)
 if (p_np_glb/=3) error stop 'requires three ranks'
 p_is_master=rank==0
 allocate(elm_data_address(0:2))
 elm_data_address(0)%val=[2]
 allocate(elm_data_address(1)%val(0))
 elm_data_address(2)%val=[3,1]
 numelm=size(elm_data_address(rank)%val)
 allocate(elm_patch%substt(numelm),elm_patch%subend(numelm),elm_patch%subfrc(numelm))
 allocate(mass(numelm),water(numelm),mass2(2,numelm),water2(2,numelm),filter(numelm))
 filter=.true.
 elm_patch%subfrc=1.d0
 do i=1,numelm
  elm_patch%substt(i)=i
  elm_patch%subend(i)=i
  g=elm_data_address(rank)%val(i)
  water(i)=2.d0*g
  mass(i)=water(i)*(g+1)
  water2(:,i)=water(i)
  mass2(:,i)=[mass(i),2*mass(i)]
 enddo
 call write_history_mt_vector_2d(mass,water,1.d0,.false.,1.d-12,'probe','ratio',-1,filter,'ratio','1')
 call write_history_mt_ratio_vector_3d(mass2,water2,'probe','ratio3',-1,'soil',1,2,filter,'ratio','1')
 if (p_is_master) then
  if (writes/=2) error stop 'missing history writes'
  print *, 'PASS: material history on three ranks, including empty rank and reordered elements'
 endif
 call MPI_FINALIZE(p_err)
end program
"""
        probe = work / "probe.f90"
        probe.write_text(stubs)
        executable = work / "probe"
        subprocess.run([compiler, "-fcheck=all", str(probe), "-o", str(executable)],
                       cwd=work, check=True, capture_output=True, text=True, timeout=30)
        version = subprocess.run([compiler, "--showme:version"], capture_output=True, text=True, timeout=10)
        flags = ["--oversubscribe"] if "Open MPI" in version.stdout else []
        result = subprocess.run([launcher, *flags, "-n", "3", str(executable)], cwd=work,
                                env={**os.environ, "OMP_NUM_THREADS": "1"},
                                capture_output=True, text=True, timeout=30)
        assert result.returncode == 0, (result.stdout, result.stderr)
        assert "PASS: material history on three ranks" in result.stdout, result.stdout
