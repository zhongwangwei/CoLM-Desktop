#include <define.h>

#ifdef GridRiverLakeFlow
MODULE MOD_Grid_RiverLakeHistRoute
!-----------------------------------------------------------------------
! DESCRIPTION:
!
!    Single dispatch point for every route-history variable.
!
!    Step 3 of the river-history sharding plan requires that no variable is
!    written twice, once for DEF_HIST_mode='one' and once for 'block'.  Two
!    hand-maintained lists would guarantee that the next variable added is
!    wired into only one of them, and the omission would be invisible: the
!    'one' file would still look complete.
!
!    So each variable keeps exactly ONE write call, to the routines here,
!    and the mode is decided in one place.  Coverage is then a property of
!    the call sites rather than of a list someone has to remember to update.
!
!    This lives in its own module rather than in MOD_Grid_RiverLakeHist
!    because the tracer route-history writers must reach the same dispatch,
!    and MOD_Grid_RiverLakeHist already depends on the tracer lifecycle.
!    It also keeps the river-network dependency out of the generic
!    MOD_Vector_ReadWrite utility.
!
!    Usage per history write:
!
!       CALL route_hist_begin (file_hist_ucat, itime, is_first_in_file)
!       ... CALL route_hist_write_ucat (...) for every variable ...
!       CALL route_hist_end ()
!
!-----------------------------------------------------------------------

   USE MOD_Precision
   USE MOD_SPMD_Task
   USE MOD_Namelist
   USE MOD_NetCDFSerial
   ! Sharded (block-mode) writers live in their own module; the generic
   ! master-gather writers stay in MOD_Vector_ReadWrite.
   USE MOD_Vector_ReadWrite
   USE MOD_Grid_RiverLakeHistShard
   USE MOD_Grid_RiverLakeNetwork, only: numucat, totalnumucat, ucat_data_address, &
      ucat_ucid, x_ucat, y_ucat, griducat
   USE MOD_Grid_Reservoir, only: numresv, totalnumresv, resv_data_address, dam_GRAND_ID

   IMPLICIT NONE

   PUBLIC :: route_hist_begin
   PUBLIC :: route_hist_end
   PUBLIC :: route_hist_write_ucat
   PUBLIC :: route_hist_write_resv
   PUBLIC :: route_hist_write_bif_matrix
   PUBLIC :: route_hist_is_block
   PUBLIC :: route_hist_shard_file
   PUBLIC :: route_hist_require_migrated
   PUBLIC :: route_hist_write_pending_manifest

   ! ---- state for the current history write ----
   logical            :: rh_active     = .false.
   logical            :: rh_block      = .false.
   integer            :: rh_itime      = 0
   logical            :: rh_first      = .false.
   character(len=256) :: rh_file_one   = ''
   character(len=256) :: rh_file_shard = ''

   type(route_shard_layout_type) :: rh_ucat_layout
   type(route_shard_layout_type) :: rh_resv_layout
   type(route_shard_layout_type) :: rh_bif_layout
   logical :: rh_bif_layout_built = .false.

   ! Identity of the current run segment: the date at which this process first
   ! wrote a shard. A restart re-enters with a different one, which is what
   ! lets the aggregator tell two segments of the same period apart.
   integer :: rh_seg_date(3) = 0
   character(len=64) :: rh_segment_id = ''
   logical :: rh_seg_set = .false.
   ! Grid coordinates are owned by MOD_Grid_RiverLakeHist and passed in; cache
   ! them so the fingerprint can be recomputed without re-plumbing them.
   real(r8), allocatable :: rh_lon_cache(:), rh_lat_cache(:)

CONTAINS

   logical FUNCTION route_hist_is_block ()
      route_hist_is_block = rh_block
   END FUNCTION route_hist_is_block

   FUNCTION route_hist_shard_file () RESULT (f)
   character(len=256) :: f
      f = rh_file_shard
   END FUNCTION route_hist_shard_file

   !> Guard for write paths not yet migrated to this dispatcher.
   !!
   !! Block mode must never produce a shard set that is quietly missing a
   !! variable, so an unmigrated path stops the run instead of writing an
   !! incomplete but plausible result.
   SUBROUTINE route_hist_require_migrated (what)
   character(len=*), intent(in) :: what
      IF (rh_block) THEN
         IF (p_is_master) write(6,*) &
            '***** ERROR: DEF_HIST_mode=block but this route-history writer is ', &
            'not yet migrated to the shard dispatcher: ', trim(what)
         CALL CoLM_stop ('route history: unmigrated writer under block mode')
      ENDIF
   END SUBROUTINE route_hist_require_migrated

   ! ------------------------------------------------------------------
   !! lon_ucat/lat_ucat are owned by MOD_Grid_RiverLakeHist, which USEs this
   !! module; they are passed in rather than re-derived here so there is only
   !! one definition of the output grid coordinates.
   !! Owns file creation and the time axis for BOTH modes, and returns the
   !! record index the writers below use.  Keeping it here is what stops the
   !! two modes from growing separate copies of the skeleton logic.
   SUBROUTINE route_hist_begin (file_hist_ucat, idate, is_first_in_file, &
         lon_ucat, lat_ucat, itime_in_file_ucat)

   character(len=*), intent(in)  :: file_hist_ucat
   integer,          intent(in)  :: idate(3)
   logical,          intent(in)  :: is_first_in_file
   real(r8),         intent(in)  :: lon_ucat (:), lat_ucat (:)
   integer,          intent(out) :: itime_in_file_ucat

   character(len=256) :: fshard
   logical :: fexists

      rh_file_one = file_hist_ucat
      rh_first    = is_first_in_file
      IF (.not. rh_seg_set) THEN
         rh_seg_date = idate
         rh_seg_set  = .true.
         write(rh_segment_id,'(A,I4.4,I3.3,I6.6)') 'seg', idate(1), idate(2), idate(3)
      ENDIF
      IF (.not. allocated(rh_lon_cache)) THEN
         allocate (rh_lon_cache(size(lon_ucat))); rh_lon_cache = lon_ucat
         allocate (rh_lat_cache(size(lat_ucat))); rh_lat_cache = lat_ucat
      ENDIF
      rh_block    = (trim(DEF_HIST_mode) == 'block')
      rh_active   = .true.
      itime_in_file_ucat = 0

      IF (.not. rh_block) THEN
         ! ---- 'one' mode: the single global file, master only ----------
         IF (p_is_master) THEN
            inquire (file=trim(file_hist_ucat), exist=fexists)
            IF (.not. fexists) THEN
               CALL ncio_create_file (trim(file_hist_ucat))
               CALL ncio_define_dimension (trim(file_hist_ucat), 'time', 0)
               CALL ncio_define_dimension (trim(file_hist_ucat), 'lat_ucat', griducat%nlat)
               CALL ncio_define_dimension (trim(file_hist_ucat), 'lon_ucat', griducat%nlon)
               CALL ncio_write_serial (trim(file_hist_ucat), 'lat_ucat', lat_ucat, 'lat_ucat')
               CALL ncio_write_serial (trim(file_hist_ucat), 'lon_ucat', lon_ucat, 'lon_ucat')
               ! Reservoir axis belongs to the skeleton, not to the first
               ! reservoir variable: it used to be guarded by an 'is the file
               ! new' flag computed in the caller, which is exactly the kind of
               ! state that goes stale when the skeleton moves.
               IF (DEF_Reservoir_Method > 0 .and. totalnumresv > 0) THEN
                  CALL ncio_define_dimension (trim(file_hist_ucat), 'reservoir', totalnumresv)
                  CALL ncio_write_serial (trim(file_hist_ucat), 'resv_GRAND_ID', &
                     dam_GRAND_ID, 'reservoir')
                  CALL ncio_put_attr (trim(file_hist_ucat), 'resv_GRAND_ID', &
                     'long_name', 'reservoir GRAND ID')
               ENDIF
            ENDIF
            CALL ncio_write_time (trim(file_hist_ucat), 'time', idate, &
               itime_in_file_ucat, DEF_HIST_FREQ)
         ENDIF
         rh_itime = itime_in_file_ucat
         RETURN
      ENDIF

      ! ---- block mode: one shard per IO group -------------------------
      CALL route_shard_filename (trim(file_hist_ucat), max(p_iam_io,0), fshard, &
         trim(rh_segment_id))
      rh_file_shard = fshard

      ! Layouts are collective over p_comm_group and must therefore be built
      ! by IO ranks and workers together; the master is a singleton group and
      ! takes no part.
      IF (p_is_io .or. p_is_worker) THEN
         IF (is_first_in_file .or. .not. rh_ucat_layout%built) THEN
            CALL route_shard_layout_build (rh_ucat_layout, local_ucat_count(), local_ucat_ids())
            CALL route_shard_layout_build (rh_resv_layout, local_resv_count(), local_resv_ids())
         ENDIF
      ENDIF

      IF (p_is_io) THEN
         inquire (file=trim(rh_file_shard), exist=fexists)
         ! A new run segment writes its own complete shard set, so a restart
         ! that changes the IO-group count is safe: the previous segment's
         ! shards stay valid and self-describing, and the aggregator merges
         ! the segments by time record.
         IF (.not. fexists) THEN
            CALL create_shard_skeleton (lon_ucat, lat_ucat)
            ! The aggregator decides which shards belong together purely from
            ! these attributes and refuses a shard without them, so writing
            ! them is not optional. They were previously written only by the
            ! test harness, which is why every test passed while production
            ! block output could not be aggregated at all.
            CALL write_shard_identity (idate)
            ! Leave a discoverable note next to the target the moment the first
            ! shard appears. Block-mode output is not the file analysis scripts
            ! expect, and a run that ends without aggregation would otherwise
            ! look finished.
            IF (p_iam_io == 0) CALL route_hist_write_pending_manifest ()
         ENDIF
         CALL ncio_write_time (trim(rh_file_shard), 'time', idate, &
            itime_in_file_ucat, DEF_HIST_FREQ)
      ENDIF
      rh_itime = itime_in_file_ucat

   END SUBROUTINE route_hist_begin

   ! ------------------------------------------------------------------
   !> Records that this history file still needs offline aggregation, and the
   !! exact command that does it.  Removed by the aggregator on success.
   SUBROUTINE route_hist_write_pending_manifest ()

   integer :: u

      OPEN (newunit=u, file=trim(rh_file_one)//'.pending', status='replace', action='write')
      write(u,'(A)')   'This history output is SHARDED and not yet aggregated.'
      write(u,'(A)')   'DEF_HIST_mode = block writes one shard per IO group; the single'
      write(u,'(A)')   'file that analysis scripts expect does not exist yet.'
      write(u,'(A)')   ''
      write(u,'(A,A)') 'target : ', trim(rh_file_one)
      write(u,'(A,A)') 'shards : ', trim(rh_file_shard(1:index(rh_file_shard,'_shard')+5))//'NNNNN.nc'
      write(u,'(A)')   ''
      write(u,'(A)')   'Run before analysing:'
      write(u,'(A)')   '   run/scripts/concatenate_history <namelist> <this directory>'
      CLOSE (u)

   END SUBROUTINE route_hist_write_pending_manifest

   !> Stamp this shard's identity. segment_id distinguishes continuous run
   !! segments: a restart re-enters with a new one, and the aggregator merges
   !! across segments but refuses conflicting time records.
   SUBROUTINE write_shard_identity (idate)

   USE MOD_Namelist, only: DEF_CASE_NAME
   integer, intent(in) :: idate(3)

   character(len=256) :: period_key, segment_id
   character(len=256) :: fingerprint

      ! Derived from the target file, never from idate -- see the comment on
      ! route_shard_period_key. The harness calls the same function, so a
      ! divergence cannot hide behind a hardcoded key that happens to agree.
      period_key = route_shard_period_key (trim(rh_file_one))
      segment_id = rh_segment_id
      fingerprint = route_shard_grid_fingerprint (griducat%nlon, griducat%nlat, &
         rh_lon_cache, rh_lat_cache)

      CALL route_shard_write_identity (trim(rh_file_shard), trim(rh_file_one), &
         trim(DEF_CASE_NAME), trim(period_key), trim(segment_id), &
         max(p_iam_io,0), max(p_np_io,1), trim(fingerprint), &
         0._r8, 0._r8, 0)

   END SUBROUTINE write_shard_identity

   SUBROUTINE route_hist_end ()

      rh_active = .false.

   END SUBROUTINE route_hist_end

   ! ------------------------------------------------------------------
   !> One unit-catchment field. In 'one' mode this is the existing
   !! gather-to-master + regrid + serial write; in 'block' mode it is a
   !! group-local gatherv into this group's shard.
   SUBROUTINE route_hist_write_ucat (vector, varname, longname, units, no_time)

   real(r8),         intent(in) :: vector (:)
   character(len=*), intent(in) :: varname
   character(len=*), intent(in), optional :: longname, units
   logical,          intent(in), optional :: no_time

   logical :: with_time

      CALL assert_active ('route_hist_write_ucat')

      with_time = .true.
      IF (present(no_time)) with_time = .not. no_time

      IF (rh_block) THEN
         IF (with_time) THEN
            CALL route_shard_write_vector (rh_ucat_layout, vector, trim(rh_file_shard), &
               varname, 'unitcat_local', rh_itime, longname, units)
         ELSE
            CALL route_shard_write_vector (rh_ucat_layout, vector, trim(rh_file_shard), &
               varname, 'unitcat_local', longname=longname, units=units)
         ENDIF
      ELSE
         IF (with_time) THEN
            CALL vector_gather_map2grid_and_write (vector, numucat, totalnumucat, &
               ucat_data_address, griducat%nlon, x_ucat, griducat%nlat, y_ucat,   &
               trim(rh_file_one), varname, 'lon_ucat', 'lat_ucat', rh_itime,      &
               longname, units)
         ELSE
            CALL vector_gather_map2grid_and_write (vector, numucat, totalnumucat, &
               ucat_data_address, griducat%nlon, x_ucat, griducat%nlat, y_ucat,   &
               trim(rh_file_one), varname, 'lon_ucat', 'lat_ucat',                &
               longname=longname, units=units)
         ENDIF
      ENDIF

   END SUBROUTINE route_hist_write_ucat

   ! ------------------------------------------------------------------
   SUBROUTINE route_hist_write_resv (vector, varname, longname, units)

   real(r8),         intent(in) :: vector (:)
   character(len=*), intent(in) :: varname
   character(len=*), intent(in), optional :: longname, units

      CALL assert_active ('route_hist_write_resv')

      IF (rh_block) THEN
         CALL route_shard_write_vector (rh_resv_layout, vector, trim(rh_file_shard), &
            varname, 'reservoir_local', rh_itime, longname, units)
      ELSE
         CALL vector_gather_and_write (vector, numresv, totalnumresv, resv_data_address, &
            trim(rh_file_one), varname, 'reservoir', rh_itime, longname, units)
      ENDIF

   END SUBROUTINE route_hist_write_resv

   ! ------------------------------------------------------------------
   !> Bifurcation pathway matrix. The pathway layout is rebuilt on demand
   !! because pathway ownership is independent of the unit-catchment split.
   SUBROUTINE route_hist_write_bif_matrix (matrix, nrow, ncol_local, global_id, &
         ncol_global, varname, longname, units)

   real(r8),         intent(in) :: matrix (:,:)
   integer,          intent(in) :: nrow, ncol_local, ncol_global
   integer,          intent(in) :: global_id (:)
   character(len=*), intent(in) :: varname
   character(len=*), intent(in), optional :: longname, units

   real(r8), allocatable :: wdata(:,:)

      CALL assert_active ('route_hist_write_bif_matrix')

      IF (rh_block) THEN
         IF (p_is_io .or. p_is_worker) THEN
            IF (.not. rh_bif_layout_built) THEN
               CALL route_shard_layout_build (rh_bif_layout, ncol_local, global_id)
               rh_bif_layout_built = .true.
               IF (p_is_io) CALL define_bif_shard_dims (nrow)
            ENDIF
            CALL route_shard_write_matrix (rh_bif_layout, matrix, nrow, &
               trim(rh_file_shard), varname, 'bifurcation_level', &
               'bifurcation_pathway_local', rh_itime, longname, units)
         ENDIF
      ELSE
         CALL vector_gather_matrix_to_master (matrix, nrow, ncol_local, ncol_global, &
            global_id, wdata)
         IF (p_is_master) THEN
            CALL ncio_write_serial_time (trim(rh_file_one), varname, rh_itime, wdata, &
               'bifurcation_level', 'bifurcation_pathway', 'time', DEF_HIST_CompressLevel)
            IF (rh_itime <= 1) THEN
               IF (present(longname)) CALL ncio_put_attr (trim(rh_file_one), varname, 'long_name', longname)
               IF (present(units   )) CALL ncio_put_attr (trim(rh_file_one), varname, 'units',     units)
            ENDIF
            deallocate (wdata)
         ENDIF
      ENDIF

   END SUBROUTINE route_hist_write_bif_matrix

   ! ================= internal helpers =================

   SUBROUTINE assert_active (who)
   character(len=*), intent(in) :: who
      IF (.not. rh_active) CALL CoLM_stop (trim(who)//': route_hist_begin was not called')
   END SUBROUTINE assert_active

   integer FUNCTION local_ucat_count ()
      local_ucat_count = 0
      IF (p_is_worker) local_ucat_count = numucat
   END FUNCTION local_ucat_count

   FUNCTION local_ucat_ids () RESULT (ids)
   integer, allocatable :: ids(:)
      IF (p_is_worker .and. numucat > 0 .and. allocated(ucat_ucid)) THEN
         allocate (ids(numucat)); ids = ucat_ucid(1:numucat)
      ELSE
         allocate (ids(1)); ids = 0
      ENDIF
   END FUNCTION local_ucat_ids

   integer FUNCTION local_resv_count ()
      local_resv_count = 0
      IF (p_is_worker) local_resv_count = numresv
   END FUNCTION local_resv_count

   !> Reservoirs carry no separate stable id array, so the global index the
   !! 'one' path already uses is reconstructed from the scatter address book.
   FUNCTION local_resv_ids () RESULT (ids)
   integer, allocatable :: ids(:)
      IF (p_is_worker .and. numresv > 0 .and. allocated(resv_data_address)) THEN
         allocate (ids(numresv))
         ids = resv_data_address(p_iam_worker)%val(1:numresv)
      ELSE
         allocate (ids(1)); ids = 0
      ENDIF
   END FUNCTION local_resv_ids

   SUBROUTINE create_shard_skeleton (lon_ucat, lat_ucat)

   real(r8), intent(in) :: lon_ucat (:), lat_ucat (:)

      CALL ncio_create_file (trim(rh_file_shard))
      CALL ncio_define_dimension (trim(rh_file_shard), 'time', 0)
      CALL ncio_define_dimension (trim(rh_file_shard), 'unitcat_local', &
         max(rh_ucat_layout%ntotal, 0))
      CALL ncio_define_dimension (trim(rh_file_shard), 'reservoir_local', &
         max(rh_resv_layout%ntotal, 0))

      ! The ids this shard owns, plus the grid metadata the aggregator needs
      ! to rebuild lon_ucat x lat_ucat without consulting any other file.
      CALL ncio_write_serial (trim(rh_file_shard), 'ucat_ucid', &
         rh_ucat_layout%gid(1:max(rh_ucat_layout%ntotal,0)), 'unitcat_local')
      CALL ncio_write_serial (trim(rh_file_shard), 'x_ucat', &
         shard_x_ucat(), 'unitcat_local')
      CALL ncio_write_serial (trim(rh_file_shard), 'y_ucat', &
         shard_y_ucat(), 'unitcat_local')
      IF (rh_resv_layout%ntotal > 0) THEN
         CALL ncio_write_serial (trim(rh_file_shard), 'resv_global_index', &
            rh_resv_layout%gid(1:rh_resv_layout%ntotal), 'reservoir_local')
         IF (allocated(dam_GRAND_ID)) &
            CALL ncio_write_serial (trim(rh_file_shard), 'resv_GRAND_ID', &
               shard_resv_grand_id(), 'reservoir_local')
      ENDIF

      CALL ncio_define_dimension (trim(rh_file_shard), 'lon_ucat', griducat%nlon)
      CALL ncio_define_dimension (trim(rh_file_shard), 'lat_ucat', griducat%nlat)
      CALL ncio_write_serial (trim(rh_file_shard), 'lon_ucat', lon_ucat, 'lon_ucat')
      CALL ncio_write_serial (trim(rh_file_shard), 'lat_ucat', lat_ucat, 'lat_ucat')

   END SUBROUTINE create_shard_skeleton

   FUNCTION shard_x_ucat () RESULT (xs)
   integer, allocatable :: xs(:)
   integer :: k, n
      n = max(rh_ucat_layout%ntotal, 0)
      allocate (xs(max(n,1))); xs = 0
      DO k = 1, n
         xs(k) = x_ucat(rh_ucat_layout%gid(k))
      ENDDO
   END FUNCTION shard_x_ucat

   FUNCTION shard_y_ucat () RESULT (ys)
   integer, allocatable :: ys(:)
   integer :: k, n
      n = max(rh_ucat_layout%ntotal, 0)
      allocate (ys(max(n,1))); ys = 0
      DO k = 1, n
         ys(k) = y_ucat(rh_ucat_layout%gid(k))
      ENDDO
   END FUNCTION shard_y_ucat

   FUNCTION shard_resv_grand_id () RESULT (ids)
   integer, allocatable :: ids(:)
   integer :: k, n
      n = max(rh_resv_layout%ntotal, 0)
      allocate (ids(max(n,1))); ids = 0
      DO k = 1, n
         ids(k) = dam_GRAND_ID(rh_resv_layout%gid(k))
      ENDDO
   END FUNCTION shard_resv_grand_id

   SUBROUTINE define_bif_shard_dims (nrow)
   integer, intent(in) :: nrow
      CALL ncio_define_dimension (trim(rh_file_shard), 'bifurcation_level', nrow)
      CALL ncio_define_dimension (trim(rh_file_shard), 'bifurcation_pathway_local', &
         max(rh_bif_layout%ntotal, 0))
      CALL ncio_write_serial (trim(rh_file_shard), 'pth_global_id', &
         rh_bif_layout%gid(1:max(rh_bif_layout%ntotal,0)), 'bifurcation_pathway_local')
   END SUBROUTINE define_bif_shard_dims

END MODULE MOD_Grid_RiverLakeHistRoute
#endif
