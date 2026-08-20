#include <define.h>

#ifdef GridRiverLakeFlow
MODULE MOD_Grid_RiverLakeHistShard
!-----------------------------------------------------------------------
! DESCRIPTION:
!
!    IO-group shard output for DEF_HIST_mode = 'block' route history.
!
!    MOD_Vector_ReadWrite funnels every route-history variable through the
!    global master: a serialized mpi_recv loop plus one global mpi_barrier per
!    variable.  Measured on the step-1 baseline that stage grows faster than
!    linearly with rank count, from 3% of hist_grid_riverlake_out at 4 ranks to
!    the majority by 12.
!
!    This module replaces it with the same "one group of workers -> one IO rank
!    -> one file" MPI_Gatherv pattern the regular block history already uses
!    (share/MOD_NetCDFVector.F90).  The master takes no part: it forms a
!    singleton group of its own, so it must not enter any p_comm_group
!    collective.  Within a group the IO rank is p_root, because
!    divide_processes_into_groups splits with the global rank as key and makes
!    the lowest-ranked member the IO rank.
!
!    Shards are an internal on-disk format, reassembled offline into the same
!    single file the 'one' path writes.  They therefore carry an explicit
!    identity (schema version, case, target file, period, run segment, shard
!    index/count, grid fingerprint) so the aggregator never has to infer
!    correctness from a filename.
!
!    This lives in its own module rather than inside MOD_Vector_ReadWrite:
!    that module's job is generic vector read/write, and a river-history
!    sharding subsystem does not belong in it.
!-----------------------------------------------------------------------

   USE MOD_Precision
   IMPLICIT NONE


   ! Bump when the shard layout changes in a way an older aggregator
   ! would misread.  The aggregator refuses versions it does not know.
   ! 2: shard filenames carry the run segment, so a restart writes its own
   !    complete set instead of reusing the previous one.
   integer, PUBLIC, parameter :: ROUTE_SHARD_SCHEMA_VERSION = 2

   type, PUBLIC :: route_shard_layout_type
      logical :: built  = .false.
      integer :: nlocal = 0                 ! this rank's contribution
      integer :: ntotal = 0                 ! gathered length, IO rank only
      integer, allocatable :: cnt (:)       ! IO only, per group rank
      integer, allocatable :: dsp (:)       ! IO only, displacements
      integer, allocatable :: gid (:)       ! IO only, global ids in gathered order
   END type route_shard_layout_type

   PUBLIC :: route_shard_layout_build
   PUBLIC :: route_shard_layout_free
   PUBLIC :: route_shard_write_vector
   PUBLIC :: route_shard_write_matrix
   PUBLIC :: route_shard_write_identity
   PUBLIC :: route_shard_filename
   PUBLIC :: route_shard_grid_fingerprint
   PUBLIC :: route_shard_period_key

CONTAINS


   !> Collect the per-group counts, displacements and global ids once.
   !!
   !! Called by IO ranks and workers together; the master must not call it.
   !! Built when a history file is created, then reused for every variable,
   !! so no metadata is re-sent per variable.
   SUBROUTINE route_shard_layout_build (layout, nlocal, gid_local)

   USE MOD_Precision
   USE MOD_SPMD_Task
   IMPLICIT NONE

   type(route_shard_layout_type), intent(inout) :: layout
   integer, intent(in) :: nlocal
   integer, intent(in) :: gid_local (:)

   integer :: i
   integer, allocatable :: sbuf (:)

      CALL route_shard_layout_free (layout)

      IF (nlocal > 0 .and. size(gid_local) < nlocal) THEN
         CALL CoLM_stop ('route_shard_layout_build: gid_local shorter than nlocal')
      ENDIF

      layout%nlocal = nlocal

#ifdef USEMPI
      IF (p_is_io) THEN
         allocate (layout%cnt (0:p_np_group-1))
         allocate (layout%dsp (0:p_np_group-1))
      ELSE
         ! Unreferenced on non-root ranks, but must be allocated so the
         ! argument is always associated.
         allocate (layout%cnt (1))
         allocate (layout%dsp (1))
      ENDIF

      CALL mpi_gather (nlocal, 1, MPI_INTEGER, layout%cnt, 1, MPI_INTEGER, &
         p_root, p_comm_group, p_err)

      IF (p_is_io) THEN
         layout%dsp(0) = 0
         DO i = 1, p_np_group-1
            layout%dsp(i) = layout%dsp(i-1) + layout%cnt(i-1)
         ENDDO
         layout%ntotal = sum(layout%cnt)
         allocate (layout%gid (max(layout%ntotal,1)))
         layout%gid(:) = 0
      ELSE
         layout%ntotal = 0
         allocate (layout%gid (1))
         layout%gid(:) = 0
      ENDIF

      ! A zero-length worker still has to enter the collective; give it a
      ! valid but unread send buffer rather than an unallocated actual.
      IF (nlocal > 0) THEN
         allocate (sbuf (nlocal)); sbuf = gid_local(1:nlocal)
      ELSE
         allocate (sbuf (1));      sbuf = 0
      ENDIF

      IF (p_is_io) THEN
         CALL mpi_gatherv (MPI_IN_PLACE, 0, MPI_INTEGER, &
            layout%gid, layout%cnt, layout%dsp, MPI_INTEGER, &
            p_root, p_comm_group, p_err)
      ELSE
         CALL mpi_gatherv (sbuf, nlocal, MPI_INTEGER, &
            MPI_INULL_P, MPI_INULL_P, MPI_INULL_P, MPI_INTEGER, &
            p_root, p_comm_group, p_err)
      ENDIF

      deallocate (sbuf)
#else
      allocate (layout%cnt (0:0)); layout%cnt(0) = nlocal
      allocate (layout%dsp (0:0)); layout%dsp(0) = 0
      layout%ntotal = nlocal
      allocate (layout%gid (max(nlocal,1))); layout%gid(:) = 0
      IF (nlocal > 0) layout%gid(1:nlocal) = gid_local(1:nlocal)
#endif

      layout%built = .true.

   END SUBROUTINE route_shard_layout_build

   ! -------
   SUBROUTINE route_shard_layout_free (layout)

   IMPLICIT NONE
   type(route_shard_layout_type), intent(inout) :: layout

      IF (allocated(layout%cnt)) deallocate (layout%cnt)
      IF (allocated(layout%dsp)) deallocate (layout%dsp)
      IF (allocated(layout%gid)) deallocate (layout%gid)
      layout%built  = .false.
      layout%nlocal = 0
      layout%ntotal = 0

   END SUBROUTINE route_shard_layout_free

   !> One unit-catchment / reservoir field, gathered to the group's IO rank
   !! and written to that group's shard. No global-master buffer is ever
   !! allocated: the IO rank holds only its own group's share.
   SUBROUTINE route_shard_write_vector (layout, vector, fileshard, varname, &
         dimname, itime_in_file, longname, units)

   USE MOD_Precision
   USE MOD_SPMD_Task
   USE MOD_Namelist
   USE MOD_NetCDFSerial
   USE MOD_Vars_Global, only: spval
   IMPLICIT NONE

   type(route_shard_layout_type), intent(in) :: layout
   real(r8),         intent(in) :: vector (:)
   character(len=*), intent(in) :: fileshard, varname, dimname

   integer,          intent(in), optional :: itime_in_file
   character(len=*), intent(in), optional :: longname, units

   real(r8), allocatable :: rbuff(:), sbuff(:)
   logical :: write_attr

      IF (.not. layout%built) THEN
         CALL CoLM_stop ('route_shard_write_vector: layout not built')
      ENDIF

      IF (layout%nlocal > 0) THEN
         allocate (sbuff (layout%nlocal)); sbuff = vector(1:layout%nlocal)
      ELSE
         allocate (sbuff (1));             sbuff = spval
      ENDIF

#ifdef USEMPI
      IF (p_is_io) THEN
         allocate (rbuff (max(layout%ntotal,1))); rbuff = spval
         CALL mpi_gatherv (MPI_IN_PLACE, 0, MPI_REAL8, &
            rbuff, layout%cnt, layout%dsp, MPI_REAL8, &
            p_root, p_comm_group, p_err)
      ELSE
         CALL mpi_gatherv (sbuff, layout%nlocal, MPI_REAL8, &
            MPI_RNULL_P, MPI_INULL_P, MPI_INULL_P, MPI_REAL8, &
            p_root, p_comm_group, p_err)
      ENDIF
#else
      allocate (rbuff (max(layout%ntotal,1))); rbuff = spval
      IF (layout%ntotal > 0) rbuff(1:layout%ntotal) = sbuff(1:layout%ntotal)
#endif

      IF (p_is_io) THEN
         ! An IO group that owns nothing still writes its shard, so the
         ! aggregator sees a complete, self-describing set of shard_count
         ! files rather than having to distinguish "empty" from "missing".
         IF (present(itime_in_file)) THEN
            CALL ncio_write_serial_time (fileshard, varname, itime_in_file, &
               rbuff(1:max(layout%ntotal,0)), dimname, 'time', DEF_HIST_CompressLevel)
            write_attr = itime_in_file <= 1
         ELSE
            CALL ncio_write_serial (fileshard, varname, &
               rbuff(1:max(layout%ntotal,0)), dimname, DEF_HIST_CompressLevel)
            write_attr = .true.
         ENDIF

         IF (write_attr) THEN
            CALL ncio_put_attr (fileshard, varname, 'missing_value', spval)
            IF (present(longname)) CALL ncio_put_attr (fileshard, varname, 'long_name', longname)
            IF (present(units   )) CALL ncio_put_attr (fileshard, varname, 'units',     units   )
         ENDIF
      ENDIF

      IF (allocated(sbuff)) deallocate (sbuff)
      IF (allocated(rbuff)) deallocate (rbuff)

   END SUBROUTINE route_shard_write_vector

   !> One bifurcation pathway matrix (nrow x local pathways) per shard.
   !! Column order follows the gathered global-id order recorded in the
   !! layout, which the shard also stores, so the aggregator never relies
   !! on rank order.
   SUBROUTINE route_shard_write_matrix (layout, matrix, nrow, fileshard, &
         varname, rowdim, coldim, itime_in_file, longname, units)

   USE MOD_Precision
   USE MOD_SPMD_Task
   USE MOD_Namelist
   USE MOD_NetCDFSerial
   USE MOD_Vars_Global, only: spval
   IMPLICIT NONE

   type(route_shard_layout_type), intent(in) :: layout
   real(r8),         intent(in) :: matrix (:,:)
   integer,          intent(in) :: nrow
   character(len=*), intent(in) :: fileshard, varname, rowdim, coldim

   integer,          intent(in), optional :: itime_in_file
   character(len=*), intent(in), optional :: longname, units

   real(r8), allocatable :: rbuff(:,:), sbuff(:,:)
   integer,  allocatable :: cnt(:), dsp(:)
   integer  :: i
   logical  :: write_attr

      IF (.not. layout%built) THEN
         CALL CoLM_stop ('route_shard_write_matrix: layout not built')
      ENDIF
      IF (nrow <= 0) RETURN

      IF (layout%nlocal > 0) THEN
         allocate (sbuff (nrow, layout%nlocal))
         sbuff = matrix(1:nrow, 1:layout%nlocal)
      ELSE
         allocate (sbuff (nrow, 1)); sbuff = spval
      ENDIF

#ifdef USEMPI
      IF (p_is_io) THEN
         ! Element counts scale by the row dimension; the column layout is
         ! exactly the one already collected for the vector case.
         allocate (cnt (0:p_np_group-1))
         allocate (dsp (0:p_np_group-1))
         DO i = 0, p_np_group-1
            cnt(i) = nrow * layout%cnt(i)
            dsp(i) = nrow * layout%dsp(i)
         ENDDO
         allocate (rbuff (nrow, max(layout%ntotal,1))); rbuff = spval
         CALL mpi_gatherv (MPI_IN_PLACE, 0, MPI_REAL8, &
            rbuff, cnt, dsp, MPI_REAL8, p_root, p_comm_group, p_err)
         deallocate (cnt, dsp)
      ELSE
         CALL mpi_gatherv (sbuff, nrow*layout%nlocal, MPI_REAL8, &
            MPI_RNULL_P, MPI_INULL_P, MPI_INULL_P, MPI_REAL8, &
            p_root, p_comm_group, p_err)
      ENDIF
#else
      allocate (rbuff (nrow, max(layout%ntotal,1))); rbuff = spval
      IF (layout%ntotal > 0) rbuff(:,1:layout%ntotal) = sbuff(:,1:layout%ntotal)
#endif

      IF (p_is_io) THEN
         IF (present(itime_in_file)) THEN
            CALL ncio_write_serial_time (fileshard, varname, itime_in_file, &
               rbuff(1:nrow,1:max(layout%ntotal,0)), rowdim, coldim, 'time', &
               DEF_HIST_CompressLevel)
            write_attr = itime_in_file <= 1
         ELSE
            CALL ncio_write_serial (fileshard, varname, &
               rbuff(1:nrow,1:max(layout%ntotal,0)), rowdim, coldim, &
               DEF_HIST_CompressLevel)
            write_attr = .true.
         ENDIF

         IF (write_attr) THEN
            CALL ncio_put_attr (fileshard, varname, 'missing_value', spval)
            IF (present(longname)) CALL ncio_put_attr (fileshard, varname, 'long_name', longname)
            IF (present(units   )) CALL ncio_put_attr (fileshard, varname, 'units',     units   )
         ENDIF
      ENDIF

      IF (allocated(sbuff)) deallocate (sbuff)
      IF (allocated(rbuff)) deallocate (rbuff)

   END SUBROUTINE route_shard_write_matrix

   !> Shard file name. Purely for locating the file: correctness of the
   !! aggregation is decided by the identity attributes, never by this.
   !!
   !! The segment is part of the name because a restart inside one history
   !! period must not reuse the previous run's shards: if the IO-group count
   !! changed, those shards partition the domain differently and stale ones
   !! would linger. Each continuous run segment writes its own complete set,
   !! and the aggregator merges the segments by time record.
   SUBROUTINE route_shard_filename (file_target, shard_index, fileshard, segment)

   IMPLICIT NONE
   character(len=*),   intent(in)  :: file_target
   integer,            intent(in)  :: shard_index
   character(len=256), intent(out) :: fileshard
   character(len=*),   intent(in), optional :: segment

   integer :: i
   character(len=8)  :: cidx
   character(len=80) :: seg

      write(cidx,'(I5.5)') shard_index
      seg = ''
      IF (present(segment)) THEN
         IF (len_trim(segment) > 0) seg = '_' // trim(segment)
      ENDIF

      i = len_trim(file_target)
      IF (i > 3) THEN
         IF (file_target(i-2:i) == '.nc') THEN
            fileshard = file_target(1:i-3) // trim(seg) // '_shard' // trim(cidx) // '.nc'
            RETURN
         ENDIF
      ENDIF
      fileshard = trim(file_target) // trim(seg) // '_shard' // trim(cidx) // '.nc'

   END SUBROUTINE route_shard_filename

   !> The period key that identifies the target history file, derived from that
   !! file's own path. It must NOT be built from the current date: a restart
   !! partway through a period writes a segment whose start date differs from
   !! the period, and the aggregator requires every segment of one file to carry
   !! the same key. The basename is exactly the period by construction of
   !! DEF_HIST_groupby. Lives here, public, so the test harness derives the key
   !! through the same code production does instead of hardcoding one that
   !! happens to agree.
   FUNCTION route_shard_period_key (file_target) RESULT (key)

   IMPLICIT NONE
   character(len=*), intent(in) :: file_target
   character(len=256) :: key

   integer :: i

      i   = index(file_target, '/', back=.true.)
      key = file_target(i+1:)

   END FUNCTION route_shard_period_key

   !> Deterministic description of the output grid, compared by the
   !! aggregator so shards from a different grid can never be merged.
   FUNCTION route_shard_grid_fingerprint (nlon, nlat, lon, lat) RESULT (fp)

   USE MOD_Precision
   IMPLICIT NONE
   integer,  intent(in) :: nlon, nlat
   real(r8), intent(in) :: lon (:), lat (:)
   character(len=256)   :: fp

   character(len=32) :: c1, c2, c3, c4, c5, c6

      write(c1,'(I0)')      nlon
      write(c2,'(I0)')      nlat
      write(c3,'(ES16.9)')  lon(1)
      write(c4,'(ES16.9)')  lon(max(nlon,1))
      write(c5,'(ES16.9)')  lat(1)
      write(c6,'(ES16.9)')  lat(max(nlat,1))

      fp = trim(adjustl(c1)) // 'x' // trim(adjustl(c2)) // ':' // &
           trim(adjustl(c3)) // ':' // trim(adjustl(c4)) // ':' // &
           trim(adjustl(c5)) // ':' // trim(adjustl(c6))

   END FUNCTION route_shard_grid_fingerprint

   !> Write the shard's identity in one open/close cycle.
   !!
   !! ncio_put_attr reopens the file per attribute; nine of those per shard
   !! per history file is pure overhead, and these attributes are what the
   !! aggregator keys on, so they are written together and atomically.
   SUBROUTINE route_shard_write_identity (fileshard, target_basename, case_name, &
         period_key, segment_id, shard_index, shard_count, grid_fingerprint, &
         time_first, time_last, time_count)

   USE MOD_Precision
   USE MOD_SPMD_Task, only: CoLM_stop
   USE netcdf
   IMPLICIT NONE

   character(len=*), intent(in) :: fileshard, target_basename, case_name
   character(len=*), intent(in) :: period_key, segment_id, grid_fingerprint
   integer,          intent(in) :: shard_index, shard_count, time_count
   real(r8),         intent(in) :: time_first, time_last

   integer :: ncid, ierr

      ierr = nf90_open (trim(fileshard), NF90_WRITE, ncid)
      IF (ierr /= NF90_NOERR) CALL CoLM_stop &
         ('route_shard_write_identity: cannot open '//trim(fileshard))

      ierr = nf90_redef (ncid)
      IF (ierr /= NF90_NOERR .and. ierr /= NF90_EINDEFINE) CALL CoLM_stop &
         ('route_shard_write_identity: cannot enter define mode')

      CALL put_int  ('river_hist_shard_schema_version', ROUTE_SHARD_SCHEMA_VERSION)
      CALL put_str  ('target_history_basename', target_basename)
      CALL put_str  ('case_name',               case_name)
      CALL put_str  ('history_period_key',      period_key)
      CALL put_str  ('segment_id',              segment_id)
      CALL put_int  ('shard_index',             shard_index)
      CALL put_int  ('shard_count',             shard_count)
      CALL put_str  ('grid_fingerprint',        grid_fingerprint)
      CALL put_real ('time_first',              time_first)
      CALL put_real ('time_last',               time_last)
      CALL put_int  ('time_count',              time_count)

      ierr = nf90_enddef (ncid)
      IF (ierr /= NF90_NOERR .and. ierr /= NF90_ENOTINDEFINE) CALL CoLM_stop &
         ('route_shard_write_identity: cannot leave define mode')

      ierr = nf90_close (ncid)
      IF (ierr /= NF90_NOERR) CALL CoLM_stop &
         ('route_shard_write_identity: cannot close '//trim(fileshard))

   CONTAINS

      SUBROUTINE put_str (name, value)
      character(len=*), intent(in) :: name, value
      integer :: e
         e = nf90_put_att (ncid, NF90_GLOBAL, trim(name), trim(value))
         IF (e /= NF90_NOERR) CALL CoLM_stop &
            ('route_shard_write_identity: cannot write attribute '//trim(name))
      END SUBROUTINE put_str

      SUBROUTINE put_int (name, value)
      character(len=*), intent(in) :: name
      integer,          intent(in) :: value
      integer :: e
         e = nf90_put_att (ncid, NF90_GLOBAL, trim(name), value)
         IF (e /= NF90_NOERR) CALL CoLM_stop &
            ('route_shard_write_identity: cannot write attribute '//trim(name))
      END SUBROUTINE put_int

      SUBROUTINE put_real (name, value)
      character(len=*), intent(in) :: name
      real(r8),         intent(in) :: value
      integer :: e
         e = nf90_put_att (ncid, NF90_GLOBAL, trim(name), value)
         IF (e /= NF90_NOERR) CALL CoLM_stop &
            ('route_shard_write_identity: cannot write attribute '//trim(name))
      END SUBROUTINE put_real

   END SUBROUTINE route_shard_write_identity

END MODULE MOD_Grid_RiverLakeHistShard
#endif
