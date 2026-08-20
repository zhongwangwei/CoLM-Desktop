#include <define.h>

#ifdef TRACER
MODULE MOD_Tracer_Rest

   USE MOD_Precision
   USE MOD_Tracer_Defs, only: ntracers, tracer_init_water_ratio, trc_tiny, &
      trc_water_min_for_ratio, tracers, &
      tracer_uses_delta_diagnostics, tracer_uses_land_water_transport, tracer_is_nonvolatile_solute, &
      tracer_equilibrate_dissolved, tracer_build_descriptor_identity, &
      TRACER_DESCRIPTOR_IDENTITY_WIDTH
   USE MOD_Tracer_Vars
   USE MOD_LandPatch, only: landpatch
   USE MOD_Block, only: get_filename_block
   USE MOD_Namelist, only: DEF_REST_CompressLevel
   USE MOD_NetCDFSerial, only: ncio_var_exist, ncio_inquire_varsize, ncio_read_serial, &
      ncio_write_serial, ncio_define_dimension
   USE MOD_NetCDFVector, only: ncio_read_vector, ncio_write_vector
   USE MOD_Tracer_Lifecycle, only: tracer_lifecycle_land_write_restart
   USE MOD_SPMD_Task, only: CoLM_stop

   IMPLICIT NONE

   integer, parameter :: LAND_TRACER_RESTART_SCHEMA_VERSION = 4
   integer, parameter :: LAND_TRACER_DESCRIPTOR_FIELDS = TRACER_DESCRIPTOR_IDENTITY_WIDTH
   real(r8), parameter :: LAND_TRACER_RESTART_NEGATIVE_DUST = 1.0e-12_r8

   PRIVATE :: tracer_dim_matches, land_tracer_descriptor_matches, &
      write_land_tracer_descriptor_metadata, write_land_tracer_transaction_marker, &
      land_transport_tracer_count, read_transport_patch_field, &
      read_transport_soilsnow_field, pack_transport_patch, &
      pack_transport_soilsnow, validate_land_tracer_restart_state

CONTAINS

   integer FUNCTION land_transport_tracer_count ()
      IMPLICIT NONE
      integer :: itrc

      land_transport_tracer_count = 0
      DO itrc = 1, ntracers
         IF (tracer_uses_land_water_transport(itrc)) &
            land_transport_tracer_count = land_transport_tracer_count + 1
      ENDDO
   END FUNCTION land_transport_tracer_count

   !-------------------------------------------------------------------
   ! Verify the fastest-varying extent of a restart variable matches the
   ! currently configured generic-transport row count. The land restart
   ! writer stores each pool as a compact (transport, [soilsnow,] patch)
   ! variable, and
   ! ncio_read_vector → ncio_read_serial auto-reallocates the per-block
   ! scatter buffer to the on-disk shape. If the file was written with a
   ! different transport descriptor, the downstream mpi_scatterv still uses
   ! the current compact row count as its element multiplier and will misalign or
   ! read past the buffer. Detect the mismatch up front so the caller
   ! can fall back to tracer_init_from_water instead of crashing.
   !-------------------------------------------------------------------
   logical FUNCTION tracer_dim_matches (file_restart, varname, expect_soilsnow)
#ifdef USEMPI
      USE MOD_SPMD_Task, only: p_is_io, p_comm_glb, p_err, &
         MPI_IN_PLACE, MPI_INTEGER, MPI_SUM
#else
      USE MOD_SPMD_Task, only: p_is_io
#endif
      IMPLICIT NONE
      character(len=*), intent(in) :: file_restart, varname
      ! Optional second-dimension size (soilsnow extent = nl_soil-maxsnl).
      ! When present, tracer_dim_matches additionally requires
      ! varsize(2) == expect_soilsnow; this guards against a restart
      ! written with different maxsnl / nl_soil values, which would
      ! otherwise pass the row-count-only check and then silently misalign
      ! per-layer data via ncio_read_vector's reshape.
      integer, intent(in), optional :: expect_soilsnow
      integer, allocatable :: varsize(:)
      integer :: iblkgrp, iblk, jblk, expected_rank, ntransport
      integer :: counts(3)
      character(len=256) :: fileblock
      logical :: block_shape_ok

      counts(:) = 0
      expected_rank = merge(3, 2, present(expect_soilsnow))
      ntransport = land_transport_tracer_count()

      ! Vector restart files are split by block via get_filename_block().
      ! Checking the unsuffixed base filename makes hot starts look like
      ! old/missing tracer restarts and silently reinitializes NSS state.
      IF (p_is_io) THEN
         counts(1) = landpatch%nblkgrp
         DO iblkgrp = 1, landpatch%nblkgrp
            iblk = landpatch%xblkgrp(iblkgrp)
            jblk = landpatch%yblkgrp(iblkgrp)
            CALL get_filename_block(file_restart, iblk, jblk, fileblock)

            IF (.not. ncio_var_exist(fileblock, varname, readflag = .false.)) THEN
               CYCLE
            ENDIF
            counts(2) = counts(2) + 1

            CALL ncio_inquire_varsize(fileblock, varname, varsize)
            block_shape_ok = .false.
            IF (allocated(varsize)) THEN
               IF (size(varsize) == expected_rank) THEN
                  block_shape_ok = varsize(1) == ntransport .and. &
                     varsize(expected_rank) == landpatch%vecgs%vlen(iblk,jblk)
                  IF (block_shape_ok .and. present(expect_soilsnow)) THEN
                     block_shape_ok = varsize(2) == expect_soilsnow
                  ENDIF
               ENDIF
            ENDIF
            IF (allocated(varsize)) deallocate(varsize)
            IF (.not. block_shape_ok) counts(3) = counts(3) + 1
         ENDDO
      ENDIF

#ifdef USEMPI
      CALL mpi_allreduce(MPI_IN_PLACE, counts, 3, MPI_INTEGER, MPI_SUM, p_comm_glb, p_err)
#endif

      tracer_dim_matches = ntransport > 0 .and. counts(1) > 0 .and. &
         counts(2) == counts(1) .and. counts(3) == 0
   END FUNCTION tracer_dim_matches

   logical FUNCTION land_tracer_descriptor_matches (file_restart)
#ifdef USEMPI
      USE MOD_SPMD_Task, only: p_is_io, p_iam_glb, p_comm_glb, p_err, &
         MPI_IN_PLACE, MPI_INTEGER, MPI_SUM, MPI_MIN
#else
      USE MOD_SPMD_Task, only: p_is_io
#endif
      IMPLICIT NONE
      character(len=*), intent(in) :: file_restart
      integer :: iblkgrp, iblk, jblk, schema, complete, transport_count
      integer :: counts(5)
      integer, allocatable :: expected(:,:), ondisk(:,:)
      integer, allocatable :: local_reference_identity(:,:), reference_identity(:,:)
      integer, allocatable :: varsize(:)
      integer :: local_reference_schema, local_reference_count
      integer :: local_reference_rank, reference_rank(1), reference_metadata(2)
      integer :: generation_mismatch_count(1)
      character(len=256) :: fileblock
      logical :: block_ok, has_schema, has_count, has_identity, has_commit
      logical :: has_local_generation, generation_mismatch

      counts = 0
      has_local_generation = .false.
      generation_mismatch = .false.
      local_reference_schema = -1
      local_reference_count = -1
      CALL tracer_build_descriptor_identity(expected, transport_only=.true.)

      IF (p_is_io) THEN
         DO iblkgrp = 1, landpatch%nblkgrp
            iblk = landpatch%xblkgrp(iblkgrp)
            jblk = landpatch%yblkgrp(iblkgrp)
            CALL get_filename_block(file_restart, iblk, jblk, fileblock)
            counts(1) = counts(1) + 1
            has_schema = ncio_var_exist(fileblock, 'trc_land_restart_schema', readflag=.false.)
            has_count = ncio_var_exist(fileblock, 'trc_land_transport_count', readflag=.false.)
            has_identity = ncio_var_exist(fileblock, 'trc_land_descriptor_identity', readflag=.false.)
            has_commit = ncio_var_exist(fileblock, 'trc_land_restart_complete', readflag=.false.)

            ! No marker is an old, non-transactional restart.  It is safe only
            ! when every block is old: the caller cold-starts the entire generic
            ! land domain before any vector scatter.
            IF (.not. has_commit) THEN
               counts(2) = counts(2) + 1
               CYCLE
            ENDIF
            IF (.not. has_schema) THEN
               counts(5) = counts(5) + 1
               CYCLE
            ENDIF

            ! Validate ranks before dispatching to rank-specific readers.  Once
            ! a commit marker exists, partial/malformed metadata is corruption,
            ! not a legacy format that may be silently defaulted.
            block_ok = .true.
            schema = -1
            CALL ncio_inquire_varsize(fileblock, 'trc_land_restart_complete', varsize)
            IF (.not. allocated(varsize)) THEN
               block_ok = .false.
            ELSE
               block_ok = size(varsize) == 0
               deallocate(varsize)
            ENDIF
            CALL ncio_inquire_varsize(fileblock, 'trc_land_restart_schema', varsize)
            IF (.not. allocated(varsize)) THEN
               block_ok = .false.
            ELSE
               block_ok = size(varsize) == 0
               deallocate(varsize)
            ENDIF
            IF (block_ok) THEN
               CALL ncio_read_serial(fileblock, 'trc_land_restart_complete', complete)
               IF (complete /= 1) block_ok = .false.
            ENDIF
            IF (block_ok) THEN
               CALL ncio_read_serial(fileblock, 'trc_land_restart_schema', schema)
            ENDIF
            transport_count = -1
            IF (block_ok .and. schema == LAND_TRACER_RESTART_SCHEMA_VERSION) THEN
               IF (.not. has_count) THEN
                  block_ok = .false.
               ELSE
                  CALL ncio_inquire_varsize(fileblock, 'trc_land_transport_count', varsize)
                  IF (.not. allocated(varsize)) THEN
                     block_ok = .false.
                  ELSE
                     block_ok = size(varsize) == 0
                     deallocate(varsize)
                  ENDIF
                  IF (block_ok) THEN
                     CALL ncio_read_serial(fileblock, 'trc_land_transport_count', transport_count)
                     block_ok = transport_count >= 0 .and. transport_count <= 1000
                  ENDIF
               ENDIF
            ENDIF

            ! Schema 4 represents a provider-only checkpoint explicitly with
            ! transport_count=0.  It intentionally carries no zero-length
            ! descriptor dimension; a stale descriptor in a reused file is
            ! ignored because the committed count is the source of truth.
            IF (block_ok .and. schema == LAND_TRACER_RESTART_SCHEMA_VERSION .and. &
                transport_count > 0) THEN
               IF (.not. has_identity) THEN
                  block_ok = .false.
               ELSE
                  CALL ncio_inquire_varsize(fileblock, 'trc_land_descriptor_identity', varsize)
                  IF (.not. allocated(varsize)) THEN
                     block_ok = .false.
                  ELSE
                     IF (size(varsize) /= 2) THEN
                        block_ok = .false.
                     ELSE
                        block_ok = varsize(1) == LAND_TRACER_DESCRIPTOR_FIELDS .and. &
                           varsize(2) == transport_count
                     ENDIF
                     deallocate(varsize)
                  ENDIF
                  IF (block_ok) &
                     CALL ncio_read_serial(fileblock, 'trc_land_descriptor_identity', ondisk)
               ENDIF
            ELSEIF (block_ok .and. schema /= LAND_TRACER_RESTART_SCHEMA_VERSION) THEN
               ! The previous transactional schema had no explicit count.  A
               ! committed descriptor is coherent but incompatible and must
               ! cold-start, while marker/schema-only fragments remain fatal.
               IF (.not. has_identity) THEN
                  block_ok = .false.
               ELSE
                  CALL ncio_inquire_varsize(fileblock, 'trc_land_descriptor_identity', varsize)
                  IF (.not. allocated(varsize)) THEN
                     block_ok = .false.
                  ELSE
                     block_ok = size(varsize) == 2
                     IF (block_ok) THEN
                        block_ok = varsize(1) == LAND_TRACER_DESCRIPTOR_FIELDS .and. &
                           varsize(2) > 0 .and. varsize(2) <= 1000
                        IF (block_ok) transport_count = varsize(2)
                     ENDIF
                     deallocate(varsize)
                  ENDIF
                  IF (block_ok) &
                     CALL ncio_read_serial(fileblock, 'trc_land_descriptor_identity', ondisk)
               ENDIF
            ENDIF

            ! Compare every individually coherent block with the first one
            ! owned by this IO rank.  The cross-rank reference below then
            ! compares those exact local representatives.  Count zero carries
            ! no identity by design, so stale identity bytes are never read or
            ! admitted into the generation key.
            IF (block_ok) THEN
               IF (transport_count > 0 .and. .not. allocated(ondisk)) block_ok = .false.
            ENDIF
            IF (block_ok) THEN
               IF (.not. has_local_generation) THEN
                  has_local_generation = .true.
                  local_reference_schema = schema
                  local_reference_count = transport_count
                  IF (transport_count > 0) THEN
                     allocate(local_reference_identity(size(ondisk, 1), size(ondisk, 2)))
                     local_reference_identity = ondisk
                  ENDIF
               ELSEIF (schema /= local_reference_schema .or. &
                       transport_count /= local_reference_count) THEN
                  generation_mismatch = .true.
               ELSEIF (transport_count > 0) THEN
                  IF (.not. allocated(local_reference_identity)) THEN
                     generation_mismatch = .true.
                  ELSEIF (any(ondisk /= local_reference_identity)) THEN
                     generation_mismatch = .true.
                  ENDIF
               ENDIF
            ENDIF
            IF (.not. block_ok) THEN
               counts(5) = counts(5) + 1
            ELSEIF (schema /= LAND_TRACER_RESTART_SCHEMA_VERSION .or. &
                    transport_count /= size(expected, 2)) THEN
               counts(4) = counts(4) + 1
            ELSEIF (transport_count == 0) THEN
               counts(3) = counts(3) + 1
            ELSEIF (&
                    size(ondisk, 1) /= size(expected, 1) .or. &
                    size(ondisk, 2) /= size(expected, 2)) THEN
               counts(4) = counts(4) + 1
            ELSEIF (.not. all(ondisk == expected)) THEN
               counts(4) = counts(4) + 1
            ELSE
               counts(3) = counts(3) + 1
            ENDIF
            IF (allocated(ondisk)) deallocate(ondisk)
         ENDDO
      ENDIF

#ifdef USEMPI
      CALL mpi_allreduce(MPI_IN_PLACE, counts, 5, MPI_INTEGER, MPI_SUM, p_comm_glb, p_err)

      ! Select one rank's exact generation as the global reference.  A single
      ! broadcast plus a logical reduction is exact (unlike a checksum) while
      ! avoiding a maximum-sized identity reduction on every rank.
      local_reference_rank = huge(0)
      IF (has_local_generation) local_reference_rank = p_iam_glb
      reference_rank(1) = local_reference_rank
      CALL mpi_allreduce(MPI_IN_PLACE, reference_rank, 1, MPI_INTEGER, MPI_MIN, p_comm_glb, p_err)
      IF (reference_rank(1) /= huge(0)) THEN
         reference_metadata = 0
         IF (p_iam_glb == reference_rank(1)) THEN
            reference_metadata(1) = local_reference_schema
            reference_metadata(2) = local_reference_count
         ENDIF
         CALL mpi_bcast(reference_metadata, 2, MPI_INTEGER, reference_rank(1), p_comm_glb, p_err)
         IF (has_local_generation) THEN
            IF (local_reference_schema /= reference_metadata(1) .or. &
                local_reference_count /= reference_metadata(2)) generation_mismatch = .true.
         ENDIF
         IF (reference_metadata(2) > 0) THEN
            allocate(reference_identity(LAND_TRACER_DESCRIPTOR_FIELDS, reference_metadata(2)))
            reference_identity = 0
            IF (p_iam_glb == reference_rank(1)) THEN
               IF (allocated(local_reference_identity)) THEN
                  reference_identity = local_reference_identity
               ELSE
                  generation_mismatch = .true.
               ENDIF
            ENDIF
            CALL mpi_bcast(reference_identity, size(reference_identity), MPI_INTEGER, &
               reference_rank(1), p_comm_glb, p_err)
            IF (has_local_generation .and. &
                local_reference_schema == reference_metadata(1) .and. &
                local_reference_count == reference_metadata(2)) THEN
               IF (.not. allocated(local_reference_identity)) THEN
                  generation_mismatch = .true.
               ELSEIF (any(local_reference_identity /= reference_identity)) THEN
                  generation_mismatch = .true.
               ENDIF
            ENDIF
            deallocate(reference_identity)
         ENDIF
      ENDIF
      generation_mismatch_count(1) = merge(1, 0, generation_mismatch)
      CALL mpi_allreduce(MPI_IN_PLACE, generation_mismatch_count, 1, MPI_INTEGER, &
         MPI_SUM, p_comm_glb, p_err)
      generation_mismatch = generation_mismatch_count(1) > 0
#endif

      IF (generation_mismatch .or. counts(5) > 0 .or. &
          (counts(2) > 0 .and. counts(2) < counts(1)) .or. &
          (counts(3) > 0 .and. counts(4) > 0)) THEN
         CALL CoLM_stop('malformed/incomplete generic land tracer restart transaction')
      ENDIF

      ! A complete but different schema/descriptor is a coherent checkpoint
      ! for another configuration.  Cold-start the generic domain as one unit;
      ! never mix its rows with the current descriptor.
      land_tracer_descriptor_matches = counts(1) > 0 .and. counts(3) == counts(1)
      IF (allocated(local_reference_identity)) deallocate(local_reference_identity)
      deallocate(expected)
   END FUNCTION land_tracer_descriptor_matches

   SUBROUTINE write_land_tracer_transaction_marker (file_restart, complete)
      USE MOD_SPMD_Task, only: p_is_io
      IMPLICIT NONE
      character(len=*), intent(in) :: file_restart
      integer, intent(in) :: complete
      integer :: iblkgrp, iblk, jblk
      character(len=256) :: fileblock

      IF (p_is_io) THEN
         DO iblkgrp = 1, landpatch%nblkgrp
            iblk = landpatch%xblkgrp(iblkgrp)
            jblk = landpatch%yblkgrp(iblkgrp)
            CALL get_filename_block(file_restart, iblk, jblk, fileblock)
            CALL ncio_write_serial(fileblock, 'trc_land_restart_complete', complete)
         ENDDO
      ENDIF
   END SUBROUTINE write_land_tracer_transaction_marker

   SUBROUTINE write_land_tracer_descriptor_metadata (file_restart)
      USE MOD_SPMD_Task, only: p_is_io
      IMPLICIT NONE
      character(len=*), intent(in) :: file_restart
      integer :: iblkgrp, iblk, jblk
      integer, allocatable :: identity(:,:)
      character(len=256) :: fileblock

      CALL tracer_build_descriptor_identity(identity, transport_only=.true.)
      IF (p_is_io) THEN
         DO iblkgrp = 1, landpatch%nblkgrp
            iblk = landpatch%xblkgrp(iblkgrp)
            jblk = landpatch%yblkgrp(iblkgrp)
            CALL get_filename_block(file_restart, iblk, jblk, fileblock)
            CALL ncio_write_serial(fileblock, 'trc_land_restart_schema', LAND_TRACER_RESTART_SCHEMA_VERSION)
            CALL ncio_write_serial(fileblock, 'trc_land_transport_count', size(identity, 2))
            IF (size(identity, 2) > 0) THEN
               CALL ncio_define_dimension(fileblock, 'trc_land_descriptor_field', LAND_TRACER_DESCRIPTOR_FIELDS)
               CALL ncio_define_dimension(fileblock, 'trc_land_transport', size(identity, 2))
               CALL ncio_write_serial(fileblock, 'trc_land_descriptor_identity', identity, &
                  'trc_land_descriptor_field', 'trc_land_transport', DEF_REST_CompressLevel)
            ENDIF
         ENDDO
      ENDIF
      deallocate(identity)
   END SUBROUTINE write_land_tracer_descriptor_metadata

   SUBROUTINE pack_transport_patch (source, packed)
      IMPLICIT NONE
      real(r8), intent(in) :: source(:,:)
      real(r8), intent(out) :: packed(:,:)
      integer :: itrc, k

      packed = 0._r8
      k = 0
      DO itrc = 1, ntracers
         IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
         k = k + 1
         packed(k, :) = source(itrc, :)
      ENDDO
   END SUBROUTINE pack_transport_patch

   SUBROUTINE pack_transport_soilsnow (source, packed)
      IMPLICIT NONE
      real(r8), intent(in) :: source(:,:,:)
      real(r8), intent(out) :: packed(:,:,:)
      integer :: itrc, k

      packed = 0._r8
      k = 0
      DO itrc = 1, ntracers
         IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
         k = k + 1
         packed(k, :, :) = source(itrc, :, :)
      ENDDO
   END SUBROUTINE pack_transport_soilsnow

   SUBROUTINE read_transport_patch_field (file_restart, varname, target)
      USE MOD_SPMD_Task, only: p_is_worker
      IMPLICIT NONE
      character(len=*), intent(in) :: file_restart, varname
      real(r8), allocatable, intent(inout) :: target(:,:)
      real(r8), allocatable :: packed(:,:)
      integer :: itrc, k, ntransport

      ntransport = land_transport_tracer_count()
      CALL ncio_read_vector(file_restart, varname, ntransport, landpatch, packed, &
         known_present=.true.)
      IF (p_is_worker) THEN
         IF (landpatch%nset > 0 .and. .not. allocated(target)) &
            CALL CoLM_stop('generic land tracer restart target is not allocated')
         IF (allocated(target)) THEN
            target = 0._r8
            k = 0
            DO itrc = 1, ntracers
               IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
               k = k + 1
               target(itrc, :) = packed(k, :)
            ENDDO
         ENDIF
      ENDIF
      IF (allocated(packed)) deallocate(packed)
   END SUBROUTINE read_transport_patch_field

   SUBROUTINE read_transport_soilsnow_field (file_restart, varname, nlayers, target)
      USE MOD_SPMD_Task, only: p_is_worker
      IMPLICIT NONE
      character(len=*), intent(in) :: file_restart, varname
      integer, intent(in) :: nlayers
      real(r8), allocatable, intent(inout) :: target(:,:,:)
      real(r8), allocatable :: packed(:,:,:)
      integer :: itrc, k, ntransport

      ntransport = land_transport_tracer_count()
      CALL ncio_read_vector(file_restart, varname, ntransport, nlayers, landpatch, packed)
      IF (p_is_worker) THEN
         IF (landpatch%nset > 0 .and. .not. allocated(target)) &
            CALL CoLM_stop('generic layered land tracer restart target is not allocated')
         IF (allocated(target)) THEN
            target = 0._r8
            k = 0
            DO itrc = 1, ntracers
               IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
               k = k + 1
               target(itrc, :, :) = packed(k, :, :)
            ENDDO
         ENDIF
      ENDIF
      IF (allocated(packed)) deallocate(packed)
   END SUBROUTINE read_transport_soilsnow_field

   SUBROUTINE count_nonnegative_patch (field, invalid_counts)
      USE, INTRINSIC :: ieee_arithmetic, only: ieee_is_finite
      IMPLICIT NONE
      real(r8), intent(in) :: field(:,:)
      integer, intent(inout) :: invalid_counts(:)
      integer :: itrc, ip

      DO itrc = 1, ntracers
         IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
         DO ip = 1, size(field, 2)
            IF (.not. ieee_is_finite(field(itrc, ip))) THEN
               invalid_counts(1) = invalid_counts(1) + 1
            ELSEIF (field(itrc, ip) < -LAND_TRACER_RESTART_NEGATIVE_DUST) THEN
               invalid_counts(2) = invalid_counts(2) + 1
            ENDIF
         ENDDO
      ENDDO
   END SUBROUTINE count_nonnegative_patch

   SUBROUTINE count_nonnegative_soilsnow (field, invalid_counts)
      USE, INTRINSIC :: ieee_arithmetic, only: ieee_is_finite
      IMPLICIT NONE
      real(r8), intent(in) :: field(:,:,:)
      integer, intent(inout) :: invalid_counts(:)
      integer :: itrc, j, ip

      DO itrc = 1, ntracers
         IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
         DO ip = 1, size(field, 3)
            DO j = 1, size(field, 2)
               IF (.not. ieee_is_finite(field(itrc, j, ip))) THEN
                  invalid_counts(1) = invalid_counts(1) + 1
               ELSEIF (field(itrc, j, ip) < -LAND_TRACER_RESTART_NEGATIVE_DUST) THEN
                  invalid_counts(2) = invalid_counts(2) + 1
               ENDIF
            ENDDO
         ENDDO
      ENDDO
   END SUBROUTINE count_nonnegative_soilsnow

   SUBROUTINE count_signed_patch (field, invalid_counts)
      USE, INTRINSIC :: ieee_arithmetic, only: ieee_is_finite
      IMPLICIT NONE
      real(r8), intent(in) :: field(:,:)
      integer, intent(inout) :: invalid_counts(:)
      integer :: itrc, ip

      DO itrc = 1, ntracers
         IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
         DO ip = 1, size(field, 2)
            IF (.not. ieee_is_finite(field(itrc, ip))) &
               invalid_counts(3) = invalid_counts(3) + 1
         ENDDO
      ENDDO
   END SUBROUTINE count_signed_patch

   SUBROUTINE count_peclet_patch (field, invalid_counts)
      USE, INTRINSIC :: ieee_arithmetic, only: ieee_is_finite
      IMPLICIT NONE
      real(r8), intent(in) :: field(:,:)
      integer, intent(inout) :: invalid_counts(:)
      integer :: itrc, ip

      DO itrc = 1, ntracers
         IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
         DO ip = 1, size(field, 2)
            IF (.not. ieee_is_finite(field(itrc, ip))) THEN
               invalid_counts(3) = invalid_counts(3) + 1
            ELSEIF (field(itrc, ip) < 0._r8 .or. field(itrc, ip) > 1._r8) THEN
               invalid_counts(4) = invalid_counts(4) + 1
            ENDIF
         ENDDO
      ENDDO
   END SUBROUTINE count_peclet_patch

   SUBROUTINE clamp_nonnegative_patch (field)
      IMPLICIT NONE
      real(r8), intent(inout) :: field(:,:)
      integer :: itrc

      DO itrc = 1, ntracers
         IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
         WHERE (field(itrc, :) < 0._r8) field(itrc, :) = 0._r8
      ENDDO
   END SUBROUTINE clamp_nonnegative_patch

   SUBROUTINE clamp_nonnegative_soilsnow (field)
      IMPLICIT NONE
      real(r8), intent(inout) :: field(:,:,:)
      integer :: itrc

      DO itrc = 1, ntracers
         IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
         WHERE (field(itrc, :, :) < 0._r8) field(itrc, :, :) = 0._r8
      ENDDO
   END SUBROUTINE clamp_nonnegative_soilsnow

   SUBROUTINE validate_land_tracer_restart_state ()
#ifdef USEMPI
      USE MOD_SPMD_Task, only: p_is_worker, p_is_master, p_comm_glb, p_err, &
         MPI_IN_PLACE, MPI_INTEGER, MPI_SUM
#else
      USE MOD_SPMD_Task, only: p_is_worker, p_is_master
#endif
      IMPLICIT NONE
      integer :: invalid_counts(5)

      ! [nonfinite physical amount, significantly negative physical amount,
      !  nonfinite signed/diagnostic value, Peclet outside [0,1],
      !  missing state allocation].  trc_wa is
      ! deliberately signed because the wetland correction debt is prognostic;
      ! leaf isotope storage is likewise a signed NSS anomaly.
      invalid_counts = 0
      IF (p_is_worker) THEN
         IF (landpatch%nset > 0) THEN
            IF (.not. (allocated(trc_ldew_rain) .and. allocated(trc_ldew_snow) .and. &
                       allocated(trc_wliq_soisno) .and. allocated(trc_wice_soisno) .and. &
                       allocated(trc_wa) .and. allocated(trc_wdsrf) .and. allocated(trc_wetwat) .and. &
                       allocated(trc_surface_residue) .and. allocated(trc_subsurface_residue) .and. &
                       allocated(trc_solid_soisno) .and. allocated(trc_canopy_solid) .and. &
                       allocated(trc_surface_solid) .and. allocated(trc_subsurface_solid) .and. &
                       allocated(trc_waterstorage_solid) .and. allocated(trc_scv) .and. &
                       allocated(trc_waterstorage) .and. allocated(trc_leaf_delta_e) .and. &
                       allocated(trc_leaf_delta_b) .and. allocated(trc_leaf_peclet) .and. &
                       allocated(trc_leaf_water_moles) .and. allocated(trc_leaf_iso_storage))) &
               invalid_counts(5) = invalid_counts(5) + 1
         ENDIF
         IF (allocated(trc_ldew_rain)) CALL count_nonnegative_patch(trc_ldew_rain, invalid_counts)
         IF (allocated(trc_ldew_snow)) CALL count_nonnegative_patch(trc_ldew_snow, invalid_counts)
         IF (allocated(trc_wliq_soisno)) CALL count_nonnegative_soilsnow(trc_wliq_soisno, invalid_counts)
         IF (allocated(trc_wice_soisno)) CALL count_nonnegative_soilsnow(trc_wice_soisno, invalid_counts)
         IF (allocated(trc_wa)) CALL count_signed_patch(trc_wa, invalid_counts)
         IF (allocated(trc_wdsrf)) CALL count_nonnegative_patch(trc_wdsrf, invalid_counts)
         IF (allocated(trc_wetwat)) CALL count_nonnegative_patch(trc_wetwat, invalid_counts)
         IF (allocated(trc_surface_residue)) CALL count_nonnegative_patch(trc_surface_residue, invalid_counts)
         IF (allocated(trc_subsurface_residue)) CALL count_nonnegative_patch(trc_subsurface_residue, invalid_counts)
         IF (allocated(trc_solid_soisno)) CALL count_nonnegative_soilsnow(trc_solid_soisno, invalid_counts)
         IF (allocated(trc_canopy_solid)) CALL count_nonnegative_patch(trc_canopy_solid, invalid_counts)
         IF (allocated(trc_surface_solid)) CALL count_nonnegative_patch(trc_surface_solid, invalid_counts)
         IF (allocated(trc_subsurface_solid)) CALL count_nonnegative_patch(trc_subsurface_solid, invalid_counts)
         IF (allocated(trc_waterstorage_solid)) CALL count_nonnegative_patch(trc_waterstorage_solid, invalid_counts)
         IF (allocated(trc_scv)) CALL count_nonnegative_patch(trc_scv, invalid_counts)
         IF (allocated(trc_waterstorage)) CALL count_nonnegative_patch(trc_waterstorage, invalid_counts)
         IF (allocated(trc_leaf_delta_e)) CALL count_signed_patch(trc_leaf_delta_e, invalid_counts)
         IF (allocated(trc_leaf_delta_b)) CALL count_signed_patch(trc_leaf_delta_b, invalid_counts)
         IF (allocated(trc_leaf_peclet)) CALL count_peclet_patch(trc_leaf_peclet, invalid_counts)
         IF (allocated(trc_leaf_water_moles)) CALL count_nonnegative_patch(trc_leaf_water_moles, invalid_counts)
         IF (allocated(trc_leaf_iso_storage)) CALL count_signed_patch(trc_leaf_iso_storage, invalid_counts)
      ENDIF
#ifdef USEMPI
      CALL mpi_allreduce(MPI_IN_PLACE, invalid_counts, size(invalid_counts), &
         MPI_INTEGER, MPI_SUM, p_comm_glb, p_err)
#endif

      IF (any(invalid_counts > 0)) THEN
         IF (p_is_master) WRITE(*,'(A,5(I0,1X))') &
            'ERROR generic land tracer restart state counts '// &
            '[amount_nan amount_neg signed_nan peclet_range missing_state]: ', invalid_counts
         CALL CoLM_stop('invalid generic land tracer restart state')
      ENDIF

      ! Only canonicalize the tiny negative dust admitted above.  Signed debt
      ! and anomaly fields are never clipped.
      IF (p_is_worker) THEN
         IF (allocated(trc_ldew_rain)) CALL clamp_nonnegative_patch(trc_ldew_rain)
         IF (allocated(trc_ldew_snow)) CALL clamp_nonnegative_patch(trc_ldew_snow)
         IF (allocated(trc_wliq_soisno)) CALL clamp_nonnegative_soilsnow(trc_wliq_soisno)
         IF (allocated(trc_wice_soisno)) CALL clamp_nonnegative_soilsnow(trc_wice_soisno)
         IF (allocated(trc_wdsrf)) CALL clamp_nonnegative_patch(trc_wdsrf)
         IF (allocated(trc_wetwat)) CALL clamp_nonnegative_patch(trc_wetwat)
         IF (allocated(trc_surface_residue)) CALL clamp_nonnegative_patch(trc_surface_residue)
         IF (allocated(trc_subsurface_residue)) CALL clamp_nonnegative_patch(trc_subsurface_residue)
         IF (allocated(trc_solid_soisno)) CALL clamp_nonnegative_soilsnow(trc_solid_soisno)
         IF (allocated(trc_canopy_solid)) CALL clamp_nonnegative_patch(trc_canopy_solid)
         IF (allocated(trc_surface_solid)) CALL clamp_nonnegative_patch(trc_surface_solid)
         IF (allocated(trc_subsurface_solid)) CALL clamp_nonnegative_patch(trc_subsurface_solid)
         IF (allocated(trc_waterstorage_solid)) CALL clamp_nonnegative_patch(trc_waterstorage_solid)
         IF (allocated(trc_scv)) CALL clamp_nonnegative_patch(trc_scv)
         IF (allocated(trc_waterstorage)) CALL clamp_nonnegative_patch(trc_waterstorage)
         IF (allocated(trc_leaf_water_moles)) CALL clamp_nonnegative_patch(trc_leaf_water_moles)
      ENDIF
   END SUBROUTINE validate_land_tracer_restart_state

   SUBROUTINE tracer_init_from_water (numpatch, maxsnl, nl_soil, &
      ldew_rain, ldew_snow, wliq_soisno, wice_soisno, &
      wa, wdsrf, wetwat, scv, waterstorage)

      IMPLICIT NONE
      integer,  intent(in) :: numpatch, maxsnl, nl_soil
      real(r8), intent(in) :: ldew_rain(numpatch)
      real(r8), intent(in) :: ldew_snow(numpatch)
      real(r8), intent(in) :: wliq_soisno(maxsnl+1:nl_soil, numpatch)
      real(r8), intent(in) :: wice_soisno(maxsnl+1:nl_soil, numpatch)
      real(r8), intent(in) :: wa(numpatch)
      real(r8), intent(in) :: wdsrf(numpatch)
      real(r8), intent(in) :: wetwat(numpatch)
      real(r8), intent(in) :: scv(numpatch)
      ! Optional: per-patch irrigation reservoir from
      ! MOD_Vars_TimeVariables. Present when DEF_USE_IRRIGATION is on
      ! under CROP. When passed, trc_waterstorage is cold-started to
      ! waterstorage*R_init so the first tracer_save_storage sees a
      ! correct starting inventory; without this the pool would be 0
      ! and the first step's balance check would under-count the
      ! reservoir mass that the water side already tracks.
      real(r8), intent(in), optional :: waterstorage(numpatch)

      integer  :: itrc, ip, j, snl_local
      real(r8) :: R_init

      IF (allocated(trc_surface_residue)) trc_surface_residue = 0._r8
      IF (allocated(trc_subsurface_residue)) trc_subsurface_residue = 0._r8
      IF (allocated(trc_solid_soisno)) trc_solid_soisno = 0._r8
      IF (allocated(trc_canopy_solid)) trc_canopy_solid = 0._r8
      IF (allocated(trc_surface_solid)) trc_surface_solid = 0._r8
      IF (allocated(trc_subsurface_solid)) trc_subsurface_solid = 0._r8
      IF (allocated(trc_waterstorage_solid)) trc_waterstorage_solid = 0._r8
      DO itrc = 1, ntracers
         IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
         R_init = tracer_init_water_ratio(itrc)
         DO ip = 1, numpatch
            trc_ldew_rain(itrc, ip) = ldew_rain(ip) * R_init
            trc_ldew_snow(itrc, ip) = ldew_snow(ip) * R_init
            CALL tracer_equilibrate_dissolved(itrc, max(ldew_rain(ip), 0._r8), &
               trc_ldew_rain(itrc, ip), trc_canopy_solid(itrc, ip))
            DO j = maxsnl + 1, nl_soil
               trc_wliq_soisno(itrc, j, ip) = max(wliq_soisno(j, ip), 0._r8) * R_init
               trc_wice_soisno(itrc, j, ip) = max(wice_soisno(j, ip), 0._r8) * R_init
               CALL tracer_equilibrate_dissolved(itrc, max(wliq_soisno(j, ip), 0._r8), &
                  trc_wliq_soisno(itrc, j, ip), trc_solid_soisno(itrc, j, ip))
            ENDDO
            ! wa uses the SIGNED water value so a hydrology restart with an
            ! aquifer-debt state (wa<0, recorded by the wetland branch at
            ! MOD_SoilSnowHydrology.F90:1200-1203) starts with a matching
            ! trc_wa<0. Without this the first wetland recovery step after a
            ! cold tracer-start would see pool_water = wa_bef + inputs but
            ! pool_tracer = 0 + inputs*R_atm (trc_wa_bef was clamped to 0),
            ! producing a 2× over-concentration in the recovered wetwat.
            ! wdsrf/wetwat stay with max(.,0) since WATER_VSF never leaves
            ! them negative by construction (overflow case L1196-1207).
            trc_wa    (itrc, ip) = wa(ip) * R_init
            trc_wdsrf (itrc, ip) = max(wdsrf(ip),  0._r8) * R_init
            trc_wetwat(itrc, ip) = max(wetwat(ip), 0._r8) * R_init
            CALL tracer_equilibrate_dissolved(itrc, max(wdsrf(ip), 0._r8), &
               trc_wdsrf(itrc, ip), trc_surface_solid(itrc, ip))
            CALL tracer_equilibrate_dissolved(itrc, max(wetwat(ip), 0._r8), &
               trc_wetwat(itrc, ip), trc_surface_solid(itrc, ip))
            CALL tracer_equilibrate_dissolved(itrc, wa(ip), &
               trc_wa(itrc, ip), trc_subsurface_solid(itrc, ip))
            ! Reconstruct snl from the snow layer water content (same
            ! recipe as CoLMMAIN L770-774): snl counts non-empty snow
            ! layers from the top. snl is a runtime scalar, not a
            ! persistent per-patch array, so we infer it here.
            ! Relies on CoLM's snow-column contiguity convention: active
            ! snow layers occupy indices [snl+1 : 0] with no gaps, so
            ! iterating from j=0 downward and EXIT-ing at the first empty
            ! slot correctly recovers snl. If a future snow path violates
            ! this (gaps in the column), replace EXIT with a count of
            ! non-empty slots instead.
            snl_local = 0
            DO j = 0, maxsnl + 1, -1
               IF (wliq_soisno(j, ip) + wice_soisno(j, ip) > 0._r8) THEN
                  snl_local = snl_local - 1
               ELSE
                  EXIT
               ENDIF
            ENDDO
            ! trc_scv holds the pre-layer snow tracer pool. It is only
            ! populated when snl==0 (thin snow, no layer yet); once a
            ! snow layer is created, tracer lives in trc_wice/wliq and
            ! trc_scv stays zero.
            IF (snl_local == 0) THEN
               trc_scv(itrc, ip) = max(scv(ip), 0._r8) * R_init
            ELSE
               trc_scv(itrc, ip) = 0._r8
            ENDIF
            IF (present(waterstorage) .and. allocated(trc_waterstorage)) THEN
               trc_waterstorage(itrc, ip) = max(waterstorage(ip), 0._r8) * R_init
               CALL tracer_equilibrate_dissolved(itrc, max(waterstorage(ip), 0._r8), &
                  trc_waterstorage(itrc, ip), trc_waterstorage_solid(itrc, ip))
            ENDIF
         ENDDO
      ENDDO
      CALL zero_provider_owned_land_tracer_state()
   END SUBROUTINE tracer_init_from_water

   SUBROUTINE read_land_tracer_restart (file_restart, maxsnl, nl_soil, found_restart, &
      scv_missing, waterstorage_missing)
      USE MOD_SPMD_Task, only: p_is_master
      IMPLICIT NONE
      character(len=*), intent(in) :: file_restart
      integer, intent(in) :: maxsnl, nl_soil
      logical, intent(out) :: found_restart
      ! These outputs remain for caller ABI compatibility.  Transactional
      ! schema v4 never mixes an optional legacy pool into a hot generic load:
      ! an old checkpoint cold-starts the whole generic land domain instead.
      logical, optional, intent(out) :: scv_missing
      logical, optional, intent(out) :: waterstorage_missing
      integer :: ntransport
      logical :: descriptor_matches, restart_complete, field_matches

      found_restart = .false.
      IF (present(scv_missing)) scv_missing = .false.
      IF (present(waterstorage_missing)) waterstorage_missing = .false.
      CALL zero_provider_owned_land_tracer_state()
      ntransport = land_transport_tracer_count()
      descriptor_matches = land_tracer_descriptor_matches(file_restart)
      IF (ntransport <= 0) THEN
         ! Provider-owned species have their own lifecycle restart callbacks.
         ! Validate any present transaction (marker=0/partial remains fatal),
         ! but do not create or consume zero-length generic tracer dimensions.
         found_restart = .true.
         RETURN
      ENDIF

      ! A missing commit marker is a legacy checkpoint and causes a coherent
      ! cold start.  Once a marker exists, the descriptor checker rejects
      ! incomplete metadata/transactions; a complete descriptor for another
      ! tracer configuration is safe but incompatible and also cold-starts.
      IF (.not. descriptor_matches) THEN
         IF (p_is_master) WRITE(*,'(3A)') &
            'Generic land tracer restart is legacy/incompatible in ', &
            TRIM(file_restart), '; using whole-domain cold start.'
         RETURN
      ENDIF

      ! A committed current transaction contains every generic prognostic and
      ! NSS field on every block with the exact compact transport-row extent.
      ! Finish all collective probes before the first vector read so no worker
      ! can observe a partially scattered checkpoint.
      restart_complete = .true.
      field_matches = tracer_dim_matches(file_restart, 'trc_ldew_rain')
      restart_complete = restart_complete .and. field_matches
      field_matches = tracer_dim_matches(file_restart, 'trc_ldew_snow')
      restart_complete = restart_complete .and. field_matches
      field_matches = tracer_dim_matches(file_restart, 'trc_wliq_soisno', &
                                         expect_soilsnow=nl_soil-maxsnl)
      restart_complete = restart_complete .and. field_matches
      field_matches = tracer_dim_matches(file_restart, 'trc_wice_soisno', &
                                         expect_soilsnow=nl_soil-maxsnl)
      restart_complete = restart_complete .and. field_matches
      field_matches = tracer_dim_matches(file_restart, 'trc_solid_soisno', &
                                         expect_soilsnow=nl_soil-maxsnl)
      restart_complete = restart_complete .and. field_matches
      field_matches = tracer_dim_matches(file_restart, 'trc_wa')
      restart_complete = restart_complete .and. field_matches
      field_matches = tracer_dim_matches(file_restart, 'trc_wdsrf')
      restart_complete = restart_complete .and. field_matches
      field_matches = tracer_dim_matches(file_restart, 'trc_wetwat')
      restart_complete = restart_complete .and. field_matches
      field_matches = tracer_dim_matches(file_restart, 'trc_surface_residue')
      restart_complete = restart_complete .and. field_matches
      field_matches = tracer_dim_matches(file_restart, 'trc_subsurface_residue')
      restart_complete = restart_complete .and. field_matches
      field_matches = tracer_dim_matches(file_restart, 'trc_canopy_solid')
      restart_complete = restart_complete .and. field_matches
      field_matches = tracer_dim_matches(file_restart, 'trc_surface_solid')
      restart_complete = restart_complete .and. field_matches
      field_matches = tracer_dim_matches(file_restart, 'trc_subsurface_solid')
      restart_complete = restart_complete .and. field_matches
      field_matches = tracer_dim_matches(file_restart, 'trc_waterstorage_solid')
      restart_complete = restart_complete .and. field_matches
      field_matches = tracer_dim_matches(file_restart, 'trc_scv')
      restart_complete = restart_complete .and. field_matches
      field_matches = tracer_dim_matches(file_restart, 'trc_waterstorage')
      restart_complete = restart_complete .and. field_matches
      field_matches = tracer_dim_matches(file_restart, 'trc_leaf_delta_e')
      restart_complete = restart_complete .and. field_matches
      field_matches = tracer_dim_matches(file_restart, 'trc_leaf_delta_b')
      restart_complete = restart_complete .and. field_matches
      field_matches = tracer_dim_matches(file_restart, 'trc_leaf_peclet')
      restart_complete = restart_complete .and. field_matches
      field_matches = tracer_dim_matches(file_restart, 'trc_leaf_water_moles')
      restart_complete = restart_complete .and. field_matches
      field_matches = tracer_dim_matches(file_restart, 'trc_leaf_iso_storage')
      restart_complete = restart_complete .and. field_matches
      IF (.not. restart_complete) &
         CALL CoLM_stop('incomplete or malformed committed generic land tracer restart')

      CALL read_transport_patch_field(file_restart, 'trc_ldew_rain', trc_ldew_rain)
      CALL read_transport_patch_field(file_restart, 'trc_ldew_snow', trc_ldew_snow)
      CALL read_transport_soilsnow_field(file_restart, 'trc_wliq_soisno', &
         nl_soil-maxsnl, trc_wliq_soisno)
      CALL read_transport_soilsnow_field(file_restart, 'trc_wice_soisno', &
         nl_soil-maxsnl, trc_wice_soisno)
      CALL read_transport_soilsnow_field(file_restart, 'trc_solid_soisno', &
         nl_soil-maxsnl, trc_solid_soisno)
      CALL read_transport_patch_field(file_restart, 'trc_wa', trc_wa)
      CALL read_transport_patch_field(file_restart, 'trc_wdsrf', trc_wdsrf)
      CALL read_transport_patch_field(file_restart, 'trc_wetwat', trc_wetwat)
      CALL read_transport_patch_field(file_restart, 'trc_surface_residue', trc_surface_residue)
      CALL read_transport_patch_field(file_restart, 'trc_subsurface_residue', trc_subsurface_residue)
      CALL read_transport_patch_field(file_restart, 'trc_canopy_solid', trc_canopy_solid)
      CALL read_transport_patch_field(file_restart, 'trc_surface_solid', trc_surface_solid)
      CALL read_transport_patch_field(file_restart, 'trc_subsurface_solid', trc_subsurface_solid)
      CALL read_transport_patch_field(file_restart, 'trc_waterstorage_solid', trc_waterstorage_solid)
      CALL read_transport_patch_field(file_restart, 'trc_scv', trc_scv)
      CALL read_transport_patch_field(file_restart, 'trc_waterstorage', trc_waterstorage)
      CALL read_transport_patch_field(file_restart, 'trc_leaf_delta_e', trc_leaf_delta_e)
      CALL read_transport_patch_field(file_restart, 'trc_leaf_delta_b', trc_leaf_delta_b)
      CALL read_transport_patch_field(file_restart, 'trc_leaf_peclet', trc_leaf_peclet)
      CALL read_transport_patch_field(file_restart, 'trc_leaf_water_moles', trc_leaf_water_moles)
      CALL read_transport_patch_field(file_restart, 'trc_leaf_iso_storage', trc_leaf_iso_storage)

      CALL validate_land_tracer_restart_state()
      CALL zero_provider_owned_land_tracer_state()
      found_restart = .true.
   END SUBROUTINE read_land_tracer_restart

   !-------------------------------------------------------------------
   ! Rebuild only trc_scv from the current scv state, leaving every
   ! other tracer pool as-is. Used after read_land_tracer_restart when
   ! the restart file was otherwise complete but lacked trc_scv (old
   ! format). Uses the same snl-from-water heuristic as
   ! tracer_init_from_water to decide when trc_scv is meaningful.
   !-------------------------------------------------------------------
   SUBROUTINE tracer_init_scv_from_water (numpatch, maxsnl, nl_soil, &
      wliq_soisno, wice_soisno, scv)
      IMPLICIT NONE
      integer,  intent(in) :: numpatch, maxsnl, nl_soil
      real(r8), intent(in) :: wliq_soisno(maxsnl+1:nl_soil, numpatch)
      real(r8), intent(in) :: wice_soisno(maxsnl+1:nl_soil, numpatch)
      real(r8), intent(in) :: scv(numpatch)

      integer  :: itrc, ip, j, snl_local
      real(r8) :: R_init

      IF (ntracers <= 0) RETURN

      DO itrc = 1, ntracers
         IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
         R_init = tracer_init_water_ratio(itrc)
         DO ip = 1, numpatch
            snl_local = 0
            DO j = 0, maxsnl + 1, -1
               IF (wliq_soisno(j, ip) + wice_soisno(j, ip) > 0._r8) THEN
                  snl_local = snl_local - 1
               ELSE
                  EXIT
               ENDIF
            ENDDO
            IF (snl_local == 0) THEN
               trc_scv(itrc, ip) = max(scv(ip), 0._r8) * R_init
            ELSE
               trc_scv(itrc, ip) = 0._r8
            ENDIF
         ENDDO
      ENDDO
   END SUBROUTINE tracer_init_scv_from_water

   SUBROUTINE tracer_init_waterstorage_from_ratio (numpatch, maxsnl, nl_soil, &
      ldew_rain, ldew_snow, wliq_soisno, wice_soisno, &
      wa, wdsrf, wetwat, scv, waterstorage)
      IMPLICIT NONE
      integer,  intent(in) :: numpatch, maxsnl, nl_soil
      real(r8), intent(in) :: ldew_rain(numpatch)
      real(r8), intent(in) :: ldew_snow(numpatch)
      real(r8), intent(in) :: wliq_soisno(maxsnl+1:nl_soil, numpatch)
      real(r8), intent(in) :: wice_soisno(maxsnl+1:nl_soil, numpatch)
      real(r8), intent(in) :: wa(numpatch)
      real(r8), intent(in) :: wdsrf(numpatch)
      real(r8), intent(in) :: wetwat(numpatch)
      real(r8), intent(in) :: scv(numpatch)
      real(r8), intent(in) :: waterstorage(numpatch)

      integer  :: itrc, ip, j, snl_local
      real(r8) :: R_init, ratio, water_ref, tracer_ref

      IF (ntracers <= 0) RETURN
      IF (.not. allocated(trc_waterstorage)) RETURN

      DO itrc = 1, ntracers
         IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
         R_init = tracer_init_water_ratio(itrc)
         DO ip = 1, numpatch
            snl_local = 0
            DO j = 0, maxsnl + 1, -1
               IF (wliq_soisno(j, ip) + wice_soisno(j, ip) > 0._r8) THEN
                  snl_local = snl_local - 1
               ELSE
                  EXIT
               ENDIF
            ENDDO
            water_ref = max(ldew_rain(ip), 0._r8) + max(ldew_snow(ip), 0._r8) &
                      + max(wa(ip), 0._r8) + max(wdsrf(ip), 0._r8) &
                      + max(wetwat(ip), 0._r8) + max(scv(ip), 0._r8)
            tracer_ref = max(trc_ldew_rain(itrc, ip), 0._r8) &
                       + max(trc_ldew_snow(itrc, ip), 0._r8) &
                       + max(trc_wa(itrc, ip), 0._r8) &
                       + max(trc_wdsrf(itrc, ip), 0._r8) &
                       + max(trc_wetwat(itrc, ip), 0._r8) &
                       + max(trc_scv(itrc, ip), 0._r8)
            DO j = snl_local + 1, nl_soil
               water_ref = water_ref + max(wliq_soisno(j, ip), 0._r8)
               tracer_ref = tracer_ref + max(trc_wliq_soisno(itrc, j, ip), 0._r8)
               water_ref = water_ref + max(wice_soisno(j, ip), 0._r8)
               tracer_ref = tracer_ref + max(trc_wice_soisno(itrc, j, ip), 0._r8)
            ENDDO
            IF (water_ref > trc_water_min_for_ratio) THEN
               ratio = tracer_ref / water_ref
            ELSE
               ratio = R_init
            ENDIF
            trc_waterstorage(itrc, ip) = max(waterstorage(ip), 0._r8) * ratio
            CALL tracer_equilibrate_dissolved(itrc, max(waterstorage(ip), 0._r8), &
               trc_waterstorage(itrc, ip), trc_waterstorage_solid(itrc, ip))
         ENDDO
      ENDDO
   END SUBROUTINE tracer_init_waterstorage_from_ratio

   SUBROUTINE tracer_enforce_solubility_from_water (numpatch, maxsnl, nl_soil, &
      ldew_rain, wliq_soisno, wa, wdsrf, wetwat, waterstorage)
      integer, intent(in) :: numpatch, maxsnl, nl_soil
      real(r8), intent(in) :: ldew_rain(numpatch)
      real(r8), intent(in) :: wliq_soisno(maxsnl+1:nl_soil, numpatch)
      real(r8), intent(in) :: wa(numpatch), wdsrf(numpatch), wetwat(numpatch)
      real(r8), intent(in), optional :: waterstorage(numpatch)

      integer :: itrc, ip, j
      real(r8) :: surface_water, surface_tracer

      DO itrc = 1, ntracers
         IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
         DO ip = 1, numpatch
            CALL tracer_equilibrate_dissolved(itrc, max(ldew_rain(ip), 0._r8), &
               trc_ldew_rain(itrc, ip), trc_canopy_solid(itrc, ip))
            DO j = maxsnl + 1, nl_soil
               CALL tracer_equilibrate_dissolved(itrc, max(wliq_soisno(j, ip), 0._r8), &
                  trc_wliq_soisno(itrc, j, ip), trc_solid_soisno(itrc, j, ip))
            ENDDO
            surface_water = max(wdsrf(ip), 0._r8) + max(wetwat(ip), 0._r8)
            surface_tracer = trc_wdsrf(itrc, ip) + trc_wetwat(itrc, ip)
            CALL tracer_equilibrate_dissolved(itrc, surface_water, surface_tracer, &
               trc_surface_solid(itrc, ip))
            IF (surface_water > trc_water_min_for_ratio) THEN
               trc_wdsrf(itrc, ip) = surface_tracer * max(wdsrf(ip), 0._r8) / surface_water
               trc_wetwat(itrc, ip) = surface_tracer * max(wetwat(ip), 0._r8) / surface_water
            ELSE
               trc_wdsrf(itrc, ip) = 0._r8
               trc_wetwat(itrc, ip) = 0._r8
            ENDIF
            CALL tracer_equilibrate_dissolved(itrc, wa(ip), trc_wa(itrc, ip), &
               trc_subsurface_solid(itrc, ip))
            IF (present(waterstorage) .and. allocated(trc_waterstorage)) THEN
               CALL tracer_equilibrate_dissolved(itrc, max(waterstorage(ip), 0._r8), &
                  trc_waterstorage(itrc, ip), trc_waterstorage_solid(itrc, ip))
            ENDIF
         ENDDO
      ENDDO
   END SUBROUTINE tracer_enforce_solubility_from_water

   SUBROUTINE write_land_tracer_restart (file_restart, maxsnl, nl_soil, numpatch, &
      ldew_rain, ldew_snow, wliq_soisno, wice_soisno, wa, wdsrf, wetwat, scv, waterstorage)
      USE MOD_SPMD_Task, only: p_is_worker
      IMPLICIT NONE
      character(len=*), intent(in) :: file_restart
      integer, intent(in) :: maxsnl, nl_soil, numpatch
      real(r8), intent(in) :: ldew_rain(numpatch)
      real(r8), intent(in) :: ldew_snow(numpatch)
      real(r8), intent(in) :: wliq_soisno(maxsnl+1:nl_soil, numpatch)
      real(r8), intent(in) :: wice_soisno(maxsnl+1:nl_soil, numpatch)
      real(r8), intent(in) :: wa(numpatch)
      real(r8), intent(in) :: wdsrf(numpatch)
      real(r8), intent(in) :: wetwat(numpatch)
      real(r8), intent(in) :: scv(numpatch)
      real(r8), intent(in), optional :: waterstorage(numpatch)

      integer :: ntransport
      real(r8), allocatable :: restart_patch(:,:)
      real(r8), allocatable :: restart_soilsnow(:,:,:)
      logical :: have_patch_data

      CALL zero_provider_owned_land_tracer_state()
      ntransport = land_transport_tracer_count()
      IF (ntransport <= 0) THEN
         ! Commit an explicit empty generic transaction.  The count, not any
         ! stale descriptor/vector left in a reused file, proves that this
         ! generation owns no generic state.  Never create a zero-length NetCDF
         ! dimension; provider-owned state is persisted by lifecycle callbacks.
         CALL write_land_tracer_transaction_marker(file_restart, 0)
         CALL write_land_tracer_descriptor_metadata(file_restart)
         CALL write_land_tracer_transaction_marker(file_restart, 1)
         RETURN
      ENDIF

      ! Reject corrupt in-memory state before publishing the in-progress
      ! marker. A crash after marker=0 can never masquerade as a committed load.
      CALL validate_land_tracer_restart_state()
      CALL write_land_tracer_transaction_marker(file_restart, 0)

      have_patch_data = p_is_worker .and. numpatch > 0
      allocate(restart_patch(ntransport, numpatch))

      restart_patch = 0._r8
      IF (have_patch_data .and. allocated(trc_ldew_rain)) &
         CALL pack_transport_patch(trc_ldew_rain, restart_patch)
      CALL ncio_write_vector(file_restart, 'trc_ldew_rain', 'trc_land_transport', ntransport, &
         'patch', landpatch, restart_patch, DEF_REST_CompressLevel)

      restart_patch = 0._r8
      IF (have_patch_data .and. allocated(trc_ldew_snow)) &
         CALL pack_transport_patch(trc_ldew_snow, restart_patch)
      CALL ncio_write_vector(file_restart, 'trc_ldew_snow', 'trc_land_transport', ntransport, &
         'patch', landpatch, restart_patch, DEF_REST_CompressLevel)

      allocate(restart_soilsnow(ntransport, nl_soil-maxsnl, numpatch))
      restart_soilsnow = 0._r8
      IF (have_patch_data .and. allocated(trc_wliq_soisno)) &
         CALL pack_transport_soilsnow(trc_wliq_soisno, restart_soilsnow)
      CALL ncio_write_vector(file_restart, 'trc_wliq_soisno', 'trc_land_transport', ntransport, &
         'soilsnow', nl_soil-maxsnl, 'patch', landpatch, restart_soilsnow, DEF_REST_CompressLevel)

      restart_soilsnow = 0._r8
      IF (have_patch_data .and. allocated(trc_wice_soisno)) &
         CALL pack_transport_soilsnow(trc_wice_soisno, restart_soilsnow)
      CALL ncio_write_vector(file_restart, 'trc_wice_soisno', 'trc_land_transport', ntransport, &
         'soilsnow', nl_soil-maxsnl, 'patch', landpatch, restart_soilsnow, DEF_REST_CompressLevel)

      restart_soilsnow = 0._r8
      IF (have_patch_data .and. allocated(trc_solid_soisno)) &
         CALL pack_transport_soilsnow(trc_solid_soisno, restart_soilsnow)
      CALL ncio_write_vector(file_restart, 'trc_solid_soisno', 'trc_land_transport', ntransport, &
         'soilsnow', nl_soil-maxsnl, 'patch', landpatch, restart_soilsnow, DEF_REST_CompressLevel)
      deallocate(restart_soilsnow)

      restart_patch = 0._r8
      IF (have_patch_data .and. allocated(trc_wa)) CALL pack_transport_patch(trc_wa, restart_patch)
      CALL ncio_write_vector(file_restart, 'trc_wa', 'trc_land_transport', ntransport, &
         'patch', landpatch, restart_patch, DEF_REST_CompressLevel)

      restart_patch = 0._r8
      IF (have_patch_data .and. allocated(trc_wdsrf)) CALL pack_transport_patch(trc_wdsrf, restart_patch)
      CALL ncio_write_vector(file_restart, 'trc_wdsrf', 'trc_land_transport', ntransport, &
         'patch', landpatch, restart_patch, DEF_REST_CompressLevel)

      restart_patch = 0._r8
      IF (have_patch_data .and. allocated(trc_wetwat)) CALL pack_transport_patch(trc_wetwat, restart_patch)
      CALL ncio_write_vector(file_restart, 'trc_wetwat', 'trc_land_transport', ntransport, &
         'patch', landpatch, restart_patch, DEF_REST_CompressLevel)

      restart_patch = 0._r8
      IF (have_patch_data .and. allocated(trc_surface_residue)) &
         CALL pack_transport_patch(trc_surface_residue, restart_patch)
      CALL ncio_write_vector(file_restart, 'trc_surface_residue', 'trc_land_transport', ntransport, &
         'patch', landpatch, restart_patch, DEF_REST_CompressLevel)

      restart_patch = 0._r8
      IF (have_patch_data .and. allocated(trc_subsurface_residue)) &
         CALL pack_transport_patch(trc_subsurface_residue, restart_patch)
      CALL ncio_write_vector(file_restart, 'trc_subsurface_residue', 'trc_land_transport', ntransport, &
         'patch', landpatch, restart_patch, DEF_REST_CompressLevel)

      restart_patch = 0._r8
      IF (have_patch_data .and. allocated(trc_canopy_solid)) &
         CALL pack_transport_patch(trc_canopy_solid, restart_patch)
      CALL ncio_write_vector(file_restart, 'trc_canopy_solid', 'trc_land_transport', ntransport, &
         'patch', landpatch, restart_patch, DEF_REST_CompressLevel)

      restart_patch = 0._r8
      IF (have_patch_data .and. allocated(trc_surface_solid)) &
         CALL pack_transport_patch(trc_surface_solid, restart_patch)
      CALL ncio_write_vector(file_restart, 'trc_surface_solid', 'trc_land_transport', ntransport, &
         'patch', landpatch, restart_patch, DEF_REST_CompressLevel)

      restart_patch = 0._r8
      IF (have_patch_data .and. allocated(trc_subsurface_solid)) &
         CALL pack_transport_patch(trc_subsurface_solid, restart_patch)
      CALL ncio_write_vector(file_restart, 'trc_subsurface_solid', 'trc_land_transport', ntransport, &
         'patch', landpatch, restart_patch, DEF_REST_CompressLevel)

      restart_patch = 0._r8
      IF (have_patch_data .and. allocated(trc_waterstorage_solid)) &
         CALL pack_transport_patch(trc_waterstorage_solid, restart_patch)
      CALL ncio_write_vector(file_restart, 'trc_waterstorage_solid', 'trc_land_transport', ntransport, &
         'patch', landpatch, restart_patch, DEF_REST_CompressLevel)

      restart_patch = 0._r8
      IF (have_patch_data .and. allocated(trc_scv)) CALL pack_transport_patch(trc_scv, restart_patch)
      CALL ncio_write_vector(file_restart, 'trc_scv', 'trc_land_transport', ntransport, &
         'patch', landpatch, restart_patch, DEF_REST_CompressLevel)

      restart_patch = 0._r8
      IF (have_patch_data .and. allocated(trc_waterstorage)) &
         CALL pack_transport_patch(trc_waterstorage, restart_patch)
      CALL ncio_write_vector(file_restart, 'trc_waterstorage', 'trc_land_transport', ntransport, &
         'patch', landpatch, restart_patch, DEF_REST_CompressLevel)

      restart_patch = 0._r8
      IF (have_patch_data .and. allocated(trc_leaf_delta_e)) &
         CALL pack_transport_patch(trc_leaf_delta_e, restart_patch)
      CALL ncio_write_vector(file_restart, 'trc_leaf_delta_e', 'trc_land_transport', ntransport, &
         'patch', landpatch, restart_patch, DEF_REST_CompressLevel)

      restart_patch = 0._r8
      IF (have_patch_data .and. allocated(trc_leaf_delta_b)) &
         CALL pack_transport_patch(trc_leaf_delta_b, restart_patch)
      CALL ncio_write_vector(file_restart, 'trc_leaf_delta_b', 'trc_land_transport', ntransport, &
         'patch', landpatch, restart_patch, DEF_REST_CompressLevel)

      restart_patch = 0._r8
      IF (have_patch_data .and. allocated(trc_leaf_peclet)) &
         CALL pack_transport_patch(trc_leaf_peclet, restart_patch)
      CALL ncio_write_vector(file_restart, 'trc_leaf_peclet', 'trc_land_transport', ntransport, &
         'patch', landpatch, restart_patch, DEF_REST_CompressLevel)

      restart_patch = 0._r8
      IF (have_patch_data .and. allocated(trc_leaf_water_moles)) &
         CALL pack_transport_patch(trc_leaf_water_moles, restart_patch)
      CALL ncio_write_vector(file_restart, 'trc_leaf_water_moles', 'trc_land_transport', ntransport, &
         'patch', landpatch, restart_patch, DEF_REST_CompressLevel)

      restart_patch = 0._r8
      IF (have_patch_data .and. allocated(trc_leaf_iso_storage)) &
         CALL pack_transport_patch(trc_leaf_iso_storage, restart_patch)
      CALL ncio_write_vector(file_restart, 'trc_leaf_iso_storage', 'trc_land_transport', ntransport, &
         'patch', landpatch, restart_patch, DEF_REST_CompressLevel)

      deallocate(restart_patch)
      CALL write_land_tracer_descriptor_metadata(file_restart)
      CALL write_land_tracer_transaction_marker(file_restart, 1)
   END SUBROUTINE write_land_tracer_restart

   SUBROUTINE write_tracer_restart_all (file_restart, maxsnl, nl_soil, numpatch, &
      ldew_rain, ldew_snow, wliq_soisno, wice_soisno, wa, wdsrf, wetwat, scv, &
      compress, waterstorage)
      IMPLICIT NONE
      character(len=*), intent(in) :: file_restart
      integer, intent(in) :: maxsnl, nl_soil, numpatch, compress
      real(r8), intent(in) :: ldew_rain(numpatch)
      real(r8), intent(in) :: ldew_snow(numpatch)
      real(r8), intent(in) :: wliq_soisno(maxsnl+1:nl_soil, numpatch)
      real(r8), intent(in) :: wice_soisno(maxsnl+1:nl_soil, numpatch)
      real(r8), intent(in) :: wa(numpatch)
      real(r8), intent(in) :: wdsrf(numpatch)
      real(r8), intent(in) :: wetwat(numpatch)
      real(r8), intent(in) :: scv(numpatch)
      real(r8), intent(in), optional :: waterstorage(numpatch)

      IF (present(waterstorage)) THEN
         CALL write_land_tracer_restart(file_restart, maxsnl, nl_soil, numpatch, &
            ldew_rain, ldew_snow, wliq_soisno, wice_soisno, wa, wdsrf, wetwat, scv, waterstorage)
      ELSE
         CALL write_land_tracer_restart(file_restart, maxsnl, nl_soil, numpatch, &
            ldew_rain, ldew_snow, wliq_soisno, wice_soisno, wa, wdsrf, wetwat, scv)
      ENDIF

      CALL tracer_lifecycle_land_write_restart(file_restart, compress)

   END SUBROUTINE write_tracer_restart_all

END MODULE MOD_Tracer_Rest
#endif
