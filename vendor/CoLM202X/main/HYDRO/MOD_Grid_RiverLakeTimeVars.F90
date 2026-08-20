#include <define.h>

#ifdef GridRiverLakeFlow
MODULE MOD_Grid_RiverLakeTimeVars
!-------------------------------------------------------------------------------------
! DESCRIPTION:
!
!   Time Variables in gridded hydrological processes.
!
! Created by Shupeng Zhang, Oct 2025
!-------------------------------------------------------------------------------------

   USE MOD_Precision
#ifdef TRACER
   USE MOD_Tracer_Lifecycle, only: tracer_lifecycle_route_write_restart
#endif
   USE MOD_Grid_RiverLakeBifurcation, only: write_bifurcation_restart
   IMPLICIT NONE

   real(r8), parameter :: RIVERLAKE_DRY_DEPTH = 1.e-5_r8
   integer, parameter :: GRIDRIVER_RESTART_SCHEMA_VERSION = 2
   integer, parameter :: GRIDRIVER_UCATCH_IDENTITY_VERSION = 1
   integer, parameter :: GRIDRIVER_RESERVOIR_IDENTITY_VERSION = 1

   ! -- state variables --
   real(r8), allocatable, target :: wdsrf_ucat (:) ! river or lake water depth [m]
   ! River depth paired with the carried bifurcation pathway momentum. CaMa's
   ! channel local-inertial scheme uses this previous-substep depth in a
   ! geometric mean with the current depth, so it is prognostic restart state.
   real(r8), allocatable, target :: wdsrf_ucat_prev (:) ! previous BIF depth [m]
   logical :: wdsrf_ucat_prev_valid = .false.
   logical :: wdsrf_ucat_prev_restart_found = .false.
   logical :: restart_transaction_validated = .false.
   logical :: restart_feature_manifest_present = .false.
   logical :: restart_bifurcation_enabled = .false.
   logical :: restart_levee_enabled = .false.
   real(r8), allocatable, target :: veloc_riv  (:) ! river velocity            [m/s]
   real(r8), allocatable :: momen_riv  (:) ! unit river momentum       [m^2/s]
   real(r8), allocatable :: volresv    (:) ! reservoir water volume    [m^3]
   ! Prognostic routing volume [m^3]. For ordinary cells this is the total
   ! movable river/floodplain volume; for levee cells it is the visible
   ! river-side volume and levsto carries the protected-side volume.  This is
   ! owned by TimeVars because it is a hydrologic state/restart variable, not a
   ! levee-parameter variable; the levee module may read/write it through
   ! explicit arguments only.
   real(r8), allocatable :: volwater_ucat (:)
   logical               :: volwater_ucat_valid = .false.

   ! -- routing accumulator state (must persist across restart so that a
   !    restart written mid routing-period does not drop the land-runoff
   !    increments queued for the next flush). Owned here (rather than in
   !    MOD_Grid_RiverLakeFlow) so WRITE/READ_GridRiverLakeTimeVars can
   !    serialise them without a circular USE; Flow imports them via the
   !    existing USE MOD_Grid_RiverLakeTimeVars at the top of that module.
   real(r8), save       :: acctime_rnof = 0._r8     ! accumulated land time since last routing flush [s]
   real(r8), allocatable :: acc_rnof_uc (:)         ! accumulated runoff depth per ucatch [m]

   ! -- restart file path (saved for deferred particle-tracer restart read) --
   character(len=512) :: gridriver_restart_file = ''

   ! PUBLIC MEMBER FUNCTIONS:
   PUBLIC :: allocate_GridRiverLakeTimeVars
   PUBLIC :: deallocate_GridRiverLakeTimeVars

   PUBLIC :: read_GridRiverLakeTimeVars
   PUBLIC :: write_GridRiverLakeTimeVars
   PUBLIC :: commit_GridRiverLakeRestart

CONTAINS

   SUBROUTINE build_gridriver_ucatch_identity (identity)

   USE MOD_SPMD_Task
   USE MOD_Grid_RiverLakeNetwork, only: numucat, x_ucat, y_ucat, ucat_next
   IMPLICIT NONE

   real(r8), allocatable, intent(out) :: identity(:,:)

      allocate (identity(4, numucat))
      identity = 0._r8

      IF (p_is_worker .and. numucat > 0) THEN
         identity(1, :) = real(GRIDRIVER_UCATCH_IDENTITY_VERSION, r8)
         identity(2, :) = real(x_ucat, r8)
         identity(3, :) = real(y_ucat, r8)
         identity(4, :) = real(ucat_next, r8)
      ENDIF

   END SUBROUTINE build_gridriver_ucatch_identity


   SUBROUTINE write_gridriver_ucatch_identity (file_restart)

   USE MOD_SPMD_Task
   USE MOD_Namelist, only: DEF_REST_CompressLevel
   USE MOD_NetCDFSerial, only: ncio_define_dimension, ncio_write_serial
   USE MOD_Vector_ReadWrite, only: vector_gather_matrix_to_master
   USE MOD_Grid_RiverLakeNetwork, only: numucat, totalnumucat, ucat_ucid
   IMPLICIT NONE

   character(len=*), intent(in) :: file_restart
   integer, allocatable :: global_id_local(:)
   real(r8), allocatable :: identity(:,:), identity_global(:,:)

      ! The empty identity is valid and needs no zero-column NetCDF variable.
      IF (totalnumucat <= 0) RETURN

      CALL build_gridriver_ucatch_identity (identity)
      allocate (global_id_local(numucat))
      IF (p_is_worker .and. numucat > 0) global_id_local = ucat_ucid

      CALL vector_gather_matrix_to_master (identity, 4, numucat, totalnumucat, &
         global_id_local, identity_global)

      IF (p_is_master) THEN
         CALL ncio_define_dimension (file_restart, 'gridriver_ucatch_identity_field', 4)
         CALL ncio_write_serial (file_restart, 'gridriver_ucatch_identity', identity_global, &
            'gridriver_ucatch_identity_field', 'ucatch', DEF_REST_CompressLevel)
         deallocate (identity_global)
      ENDIF

      deallocate (identity)
      deallocate (global_id_local)

   END SUBROUTINE write_gridriver_ucatch_identity


   SUBROUTINE build_gridriver_reservoir_identity (identity)

   USE MOD_SPMD_Task
   USE MOD_Grid_Reservoir, only: numresv, resv_ucid
   IMPLICIT NONE

   real(r8), allocatable, intent(out) :: identity(:,:)

      allocate (identity(2, numresv))
      identity = 0._r8

      IF (p_is_worker .and. numresv > 0) THEN
         identity(1, :) = real(GRIDRIVER_RESERVOIR_IDENTITY_VERSION, r8)
         identity(2, :) = real(resv_ucid, r8)
      ENDIF

   END SUBROUTINE build_gridriver_reservoir_identity


   SUBROUTINE write_gridriver_reservoir_identity (file_restart)

   USE MOD_SPMD_Task
   USE MOD_Namelist, only: DEF_REST_CompressLevel
   USE MOD_NetCDFSerial, only: ncio_define_dimension, ncio_write_serial
   USE MOD_Vector_ReadWrite, only: vector_gather_matrix_to_master
   USE MOD_Grid_Reservoir, only: numresv, totalnumresv, resv_global_id
   IMPLICIT NONE

   character(len=*), intent(in) :: file_restart
   integer, allocatable :: global_id_local(:)
   real(r8), allocatable :: identity(:,:), identity_global(:,:)

      IF (totalnumresv <= 0) RETURN

      CALL build_gridriver_reservoir_identity (identity)
      allocate (global_id_local(numresv))
      IF (p_is_worker .and. numresv > 0) global_id_local = resv_global_id

      CALL vector_gather_matrix_to_master (identity, 2, numresv, totalnumresv, &
         global_id_local, identity_global)

      IF (p_is_master) THEN
         CALL ncio_define_dimension (file_restart, 'gridriver_reservoir_identity_field', 2)
         CALL ncio_write_serial (file_restart, 'gridriver_reservoir_identity', identity_global, &
            'gridriver_reservoir_identity_field', 'reservoir', DEF_REST_CompressLevel)
         deallocate (identity_global)
      ENDIF

      deallocate (identity)
      deallocate (global_id_local)

   END SUBROUTINE write_gridriver_reservoir_identity


   SUBROUTINE validate_gridriver_restart_transaction (file_restart, legacy_restart)

   USE MOD_SPMD_Task
   USE MOD_NetCDFSerial, only: ncio_var_exist, ncio_read_bcast_serial
   USE MOD_Grid_RiverLakeNetwork, only: totalnumucat
   IMPLICIT NONE

   character(len=*), intent(in) :: file_restart
   logical, intent(out) :: legacy_restart
   integer :: has_flags(5), restart_schema, restart_complete, identity_expected
   integer :: feature_bifurcation, feature_levee

      has_flags = 0
      restart_transaction_validated = .false.
      restart_feature_manifest_present = .false.
      restart_bifurcation_enabled = .false.
      restart_levee_enabled = .false.
      IF (p_is_master) THEN
         IF (ncio_var_exist(file_restart, 'gridriver_restart_schema', readflag = .false.)) has_flags(1) = 1
         IF (ncio_var_exist(file_restart, 'gridriver_restart_complete', readflag = .false.)) has_flags(2) = 1
         IF (ncio_var_exist(file_restart, 'gridriver_ucatch_identity', readflag = .false.)) has_flags(3) = 1
         IF (ncio_var_exist(file_restart, 'gridriver_restart_feature_bifurcation', &
            readflag = .false.)) has_flags(4) = 1
         IF (ncio_var_exist(file_restart, 'gridriver_restart_feature_levee', &
            readflag = .false.)) has_flags(5) = 1
      ENDIF
#ifdef USEMPI
      CALL mpi_bcast (has_flags, 5, MPI_INTEGER, p_address_master, p_comm_glb, p_err)
#endif

      legacy_restart = all(has_flags == 0)
      IF (legacy_restart) THEN
         IF (p_is_master) THEN
            write(*,'(A)') 'WARNING: legacy GridRiverLake restart has no transaction or ucatch identity metadata.'
            call flush(6)
         ENDIF
         restart_transaction_validated = .true.
         RETURN
      ENDIF

      identity_expected = merge(1, 0, totalnumucat > 0)
      IF (has_flags(1) /= 1 .or. has_flags(2) /= 1 .or. has_flags(3) /= identity_expected) THEN
         CALL CoLM_stop ('incomplete GridRiverLake restart transaction metadata')
      ENDIF

      CALL ncio_read_bcast_serial (file_restart, 'gridriver_restart_schema', restart_schema)
      CALL ncio_read_bcast_serial (file_restart, 'gridriver_restart_complete', restart_complete)
      IF (restart_schema /= 1 .and. restart_schema /= GRIDRIVER_RESTART_SCHEMA_VERSION) THEN
         CALL CoLM_stop ('unsupported GridRiverLake restart schema')
      ENDIF
      IF (restart_complete /= 1) THEN
         CALL CoLM_stop ('GridRiverLake restart transaction is not complete')
      ENDIF

      ! Schema v1 predates explicit feature ownership. Preserve its original
      ! permissive deferred-reader behaviour; schema v2 makes both declarations
      ! mandatory so an absent payload can no longer be confused with disabled
      ! physics.
      IF (restart_schema == 1) THEN
         restart_transaction_validated = .true.
         RETURN
      ENDIF
      IF (restart_schema == GRIDRIVER_RESTART_SCHEMA_VERSION) THEN
         IF (has_flags(4) /= 1 .or. has_flags(5) /= 1) THEN
            CALL CoLM_stop ('GridRiverLake schema-v2 restart is missing feature manifest')
         ENDIF
         CALL ncio_read_bcast_serial (file_restart, &
            'gridriver_restart_feature_bifurcation', feature_bifurcation)
         CALL ncio_read_bcast_serial (file_restart, &
            'gridriver_restart_feature_levee', feature_levee)
         IF ((feature_bifurcation /= 0 .and. feature_bifurcation /= 1) .or. &
             (feature_levee /= 0 .and. feature_levee /= 1)) THEN
            CALL CoLM_stop ('invalid GridRiverLake restart feature manifest')
         ENDIF
         restart_feature_manifest_present = .true.
         restart_bifurcation_enabled = feature_bifurcation == 1
         restart_levee_enabled = feature_levee == 1
      ENDIF
      restart_transaction_validated = .true.

   END SUBROUTINE validate_gridriver_restart_transaction


   SUBROUTINE validate_gridriver_ucatch_identity (file_restart, legacy_restart)

   USE MOD_SPMD_Task
   USE MOD_Vector_ReadWrite, only: vector_read_matrix_and_scatter
   USE MOD_Grid_RiverLakeNetwork, only: numucat, totalnumucat, ucat_ucid
   USE, INTRINSIC :: ieee_arithmetic, only: ieee_is_finite
   IMPLICIT NONE

   character(len=*), intent(in) :: file_restart
   logical, intent(in) :: legacy_restart
   integer :: mismatch_count, first_mismatch_gid, i
   integer, allocatable :: global_id_local(:)
   real(r8), allocatable :: identity(:,:), identity_restart(:,:)

      IF (legacy_restart) RETURN
      IF (totalnumucat <= 0) RETURN

      CALL build_gridriver_ucatch_identity (identity)
      allocate (global_id_local(numucat))
      IF (p_is_worker .and. numucat > 0) global_id_local = ucat_ucid

      CALL vector_read_matrix_and_scatter (file_restart, identity_restart, 4, numucat, &
         'gridriver_ucatch_identity', global_id_local, totalnumucat)

      mismatch_count = 0
      first_mismatch_gid = huge(first_mismatch_gid)
      IF (p_is_worker) THEN
         DO i = 1, numucat
            ! Validate finiteness first: `/=` against NaN may raise invalid
            ! before the identity mismatch can be rejected under FP traps.
            IF (any(.not. ieee_is_finite(identity_restart(:, i)))) THEN
               mismatch_count = mismatch_count + 1
               first_mismatch_gid = min(first_mismatch_gid, global_id_local(i))
            ELSEIF (any(identity_restart(:, i) /= identity(:, i))) THEN
               mismatch_count = mismatch_count + 1
               first_mismatch_gid = min(first_mismatch_gid, global_id_local(i))
            ENDIF
         ENDDO
      ENDIF
#ifdef USEMPI
      CALL mpi_allreduce (MPI_IN_PLACE, mismatch_count, 1, MPI_INTEGER, MPI_SUM, p_comm_glb, p_err)
      CALL mpi_allreduce (MPI_IN_PLACE, first_mismatch_gid, 1, MPI_INTEGER, MPI_MIN, p_comm_glb, p_err)
#endif

      IF (allocated(identity_restart)) deallocate (identity_restart)
      deallocate (identity)
      deallocate (global_id_local)

      IF (mismatch_count > 0) THEN
         IF (p_is_master) write(*,'(A,I0,A,I0)') &
            'ERROR: GridRiverLake restart ucatch identity mismatch; count=', &
            mismatch_count, ', first global ucatch=', first_mismatch_gid
         CALL CoLM_stop ('Refusing to load GridRiverLake state for a different ucatch network')
      ENDIF

   END SUBROUTINE validate_gridriver_ucatch_identity


   SUBROUTINE validate_gridriver_reservoir_identity (file_restart, legacy_restart)

   USE MOD_SPMD_Task
   USE MOD_Namelist, only: DEF_Reservoir_Method
   USE MOD_NetCDFSerial, only: ncio_var_exist
   USE MOD_Vector_ReadWrite, only: vector_read_matrix_and_scatter
   USE MOD_Grid_Reservoir, only: numresv, totalnumresv, resv_global_id
   USE, INTRINSIC :: ieee_arithmetic, only: ieee_is_finite
   IMPLICIT NONE

   character(len=*), intent(in) :: file_restart
   logical, intent(in) :: legacy_restart
   logical :: has_identity
   integer :: mismatch_count, first_mismatch_gid, i
   integer, allocatable :: global_id_local(:)
   real(r8), allocatable :: identity(:,:), identity_restart(:,:)

      IF (legacy_restart) RETURN
      IF (DEF_Reservoir_Method <= 0 .or. totalnumresv <= 0) RETURN

      has_identity = .false.
      IF (p_is_master) has_identity = ncio_var_exist(&
         file_restart, 'gridriver_reservoir_identity', readflag = .false.)
#ifdef USEMPI
      CALL mpi_bcast (has_identity, 1, MPI_LOGICAL, p_address_master, p_comm_glb, p_err)
#endif
      IF (.not. has_identity) THEN
         ! Schema v1 predates this identity.  Schema v2 writes volresv as a
         ! required transaction field and must also prove what each ordinal
         ! represents before that state may be scattered.
         IF (restart_feature_manifest_present) THEN
            CALL CoLM_stop ('GridRiverLake schema-v2 restart is missing required reservoir identity')
         ENDIF
         RETURN
      ENDIF

      CALL build_gridriver_reservoir_identity (identity)
      allocate (global_id_local(numresv))
      IF (p_is_worker .and. numresv > 0) global_id_local = resv_global_id

      CALL vector_read_matrix_and_scatter (file_restart, identity_restart, 2, numresv, &
         'gridriver_reservoir_identity', global_id_local, totalnumresv)

      mismatch_count = 0
      first_mismatch_gid = huge(first_mismatch_gid)
      IF (p_is_worker) THEN
         DO i = 1, numresv
            IF (any(.not. ieee_is_finite(identity_restart(:, i)))) THEN
               mismatch_count = mismatch_count + 1
               first_mismatch_gid = min(first_mismatch_gid, global_id_local(i))
            ELSEIF (any(identity_restart(:, i) /= identity(:, i))) THEN
               mismatch_count = mismatch_count + 1
               first_mismatch_gid = min(first_mismatch_gid, global_id_local(i))
            ENDIF
         ENDDO
      ENDIF
#ifdef USEMPI
      CALL mpi_allreduce (MPI_IN_PLACE, mismatch_count, 1, MPI_INTEGER, MPI_SUM, p_comm_glb, p_err)
      CALL mpi_allreduce (MPI_IN_PLACE, first_mismatch_gid, 1, MPI_INTEGER, MPI_MIN, p_comm_glb, p_err)
#endif

      IF (allocated(identity_restart)) deallocate (identity_restart)
      deallocate (identity)
      deallocate (global_id_local)

      IF (mismatch_count > 0) THEN
         IF (p_is_master) write(*,'(A,I0,A,I0)') &
            'ERROR: GridRiverLake restart reservoir identity mismatch; count=', &
            mismatch_count, ', first reservoir ordinal=', first_mismatch_gid
         CALL CoLM_stop ('Refusing to load GridRiverLake state for a different reservoir configuration')
      ENDIF

   END SUBROUTINE validate_gridriver_reservoir_identity


   SUBROUTINE commit_GridRiverLakeRestart (file_restart)

   USE MOD_SPMD_Task
   USE MOD_NetCDFSerial, only: ncio_write_serial
   IMPLICIT NONE

   character(len=*), intent(in) :: file_restart

#ifdef USEMPI
      CALL mpi_barrier (p_comm_glb, p_err)
#endif
      IF (p_is_master) CALL ncio_write_serial (file_restart, 'gridriver_restart_complete', 1)

   END SUBROUTINE commit_GridRiverLakeRestart

   SUBROUTINE allocate_GridRiverLakeTimeVars

   USE MOD_SPMD_Task
   USE MOD_Grid_RiverLakeNetwork, only: numucat
   USE MOD_Grid_Reservoir,        only: numresv
   IMPLICIT NONE

   integer :: ncell_state, nresv_state

      ncell_state = 0
      nresv_state = 0
      IF (p_is_worker) THEN
         ncell_state = numucat
         nresv_state = numresv
      ENDIF

      ! Restart gather wrappers are entered by master, worker, and IO ranks.
      ! Keep every actual argument allocated, using zero-length arrays where a
      ! rank owns no state, so the assumed-shape dummy contract is valid.
      allocate (wdsrf_ucat (ncell_state))
      allocate (wdsrf_ucat_prev (ncell_state))
      allocate (veloc_riv  (ncell_state))
      allocate (momen_riv  (ncell_state))
      allocate (volresv (nresv_state))
      allocate (acc_rnof_uc (ncell_state))
      acc_rnof_uc = 0._r8
      wdsrf_ucat_prev = 0._r8

      ! Allocated on every rank because read_levee_restart is collective and
      ! receives this TimeVars-owned state via an assumed-shape argument.
      allocate (volwater_ucat(ncell_state))
      volwater_ucat = 0._r8

      acctime_rnof = 0._r8
      wdsrf_ucat_prev_valid = .false.
      wdsrf_ucat_prev_restart_found = .false.
      restart_transaction_validated = .false.
      restart_feature_manifest_present = .false.
      restart_bifurcation_enabled = .false.
      restart_levee_enabled = .false.
      volwater_ucat_valid = .false.
      gridriver_restart_file = ''

   END SUBROUTINE allocate_GridRiverLakeTimeVars


   SUBROUTINE READ_GridRiverLakeTimeVars (file_restart)

   USE MOD_SPMD_Task
   USE MOD_Namelist,              only: DEF_Reservoir_Method
   USE MOD_Vector_ReadWrite
   USE MOD_NetCDFSerial,          only: ncio_var_exist, ncio_read_bcast_serial
   USE MOD_Grid_RiverLakeNetwork, only: numucat, totalnumucat, ucat_data_address, &
      totalnpthout, npthlev_bif
   USE MOD_Grid_Reservoir,        only: numresv, resv_data_address, totalnumresv
   USE, INTRINSIC :: ieee_arithmetic, only: ieee_is_finite
   IMPLICIT NONE

   character(len=*), intent(in) :: file_restart
   logical :: has_var, legacy_restart, restore_previous_depth
   integer :: base_var_flags(6), invalid_prev_count, invalid_base_count, i

      gridriver_restart_file = trim(file_restart)
      CALL validate_gridriver_restart_transaction (file_restart, legacy_restart)
      CALL validate_gridriver_ucatch_identity (file_restart, legacy_restart)
      CALL validate_gridriver_reservoir_identity (file_restart, legacy_restart)

      ! A restart read is a new state transaction. Missing optional variables
      ! must retain cold-start defaults, not state from an earlier in-process
      ! read of another file.
      acctime_rnof = 0._r8
      IF (allocated(acc_rnof_uc)) acc_rnof_uc = 0._r8
      IF (allocated(wdsrf_ucat_prev)) wdsrf_ucat_prev = 0._r8
      wdsrf_ucat_prev_valid = .false.
      wdsrf_ucat_prev_restart_found = .false.
      IF (allocated(volwater_ucat)) volwater_ucat = 0._r8
      volwater_ucat_valid = .false.

      ! Probe the base-state payload once on master. Schema v2 turns the
      ! writer's always-emitted physical state into a complete transaction;
      ! schema v1 and fully legacy files retain their historical optional
      ! accumulator/visible-volume fields.
      base_var_flags = 0
      IF (p_is_master) THEN
         IF (ncio_var_exist(file_restart, 'wdsrf_ucat', readflag = .false.)) base_var_flags(1) = 1
         IF (ncio_var_exist(file_restart, 'veloc_riv', readflag = .false.)) base_var_flags(2) = 1
         IF (ncio_var_exist(file_restart, 'acctime_rnof', readflag = .false.)) base_var_flags(3) = 1
         IF (ncio_var_exist(file_restart, 'acc_rnof_uc', readflag = .false.)) base_var_flags(4) = 1
         IF (ncio_var_exist(file_restart, 'volwater_ucat', readflag = .false.)) base_var_flags(5) = 1
         IF (ncio_var_exist(file_restart, 'volresv', readflag = .false.)) base_var_flags(6) = 1
      ENDIF
#ifdef USEMPI
      CALL mpi_bcast (base_var_flags, 6, MPI_INTEGER, p_address_master, p_comm_glb, p_err)
#endif
      IF (restart_feature_manifest_present) THEN
         IF (base_var_flags(3) /= 1) THEN
            CALL CoLM_stop ('GridRiverLake schema-v2 restart is missing required base state')
         ENDIF
         IF (totalnumucat > 0) THEN
            IF (base_var_flags(1) /= 1 .or. base_var_flags(2) /= 1 .or. &
                base_var_flags(4) /= 1 .or. base_var_flags(5) /= 1) THEN
               CALL CoLM_stop ('GridRiverLake schema-v2 restart is missing required base state')
            ENDIF
         ENDIF
         IF (DEF_Reservoir_Method > 0 .and. totalnumresv > 0 .and. base_var_flags(6) /= 1) THEN
            CALL CoLM_stop ('GridRiverLake schema-v2 restart is missing required base state')
         ENDIF
      ENDIF

      IF (totalnumucat > 0) THEN
         CALL vector_read_and_scatter (file_restart, wdsrf_ucat, numucat, 'wdsrf_ucat', ucat_data_address)
         CALL vector_read_and_scatter (file_restart, veloc_riv,  numucat, 'veloc_riv',  ucat_data_address)
      ENDIF

      ! Additive restart field: old/legacy files lack it, so initialize the
      ! first semi-implicit BIF step with current depth. New files preserve the
      ! exact depth paired with pth_momen, including MPI repartitioning.
      has_var = .false.
      IF (p_is_master .and. totalnumucat > 0) &
         has_var = ncio_var_exist(file_restart, 'wdsrf_ucat_prev', readflag = .false.)
#ifdef USEMPI
      CALL mpi_bcast (has_var, 1, MPI_LOGICAL, p_address_master, p_comm_glb, p_err)
#endif
      restore_previous_depth = restart_bifurcation_enabled .and. &
         totalnpthout > 0 .and. npthlev_bif > 0
      IF (restart_feature_manifest_present .and. .not. restore_previous_depth) has_var = .false.
      IF (.not. has_var .and. .not. legacy_restart .and. totalnumucat > 0 .and. &
          (.not. restart_feature_manifest_present .or. restore_previous_depth)) THEN
         CALL CoLM_stop ('GridRiverLake restart transaction is missing wdsrf_ucat_prev')
      ENDIF
      IF (has_var) THEN
         CALL vector_read_and_scatter (file_restart, wdsrf_ucat_prev, numucat, &
            'wdsrf_ucat_prev', ucat_data_address)

         invalid_prev_count = 0
         DO i = 1, size(wdsrf_ucat_prev)
            ! Fortran logical evaluation is not short-circuiting: never compare
            ! a NaN against zero when invalid floating-point traps are enabled.
            IF (.not. ieee_is_finite(wdsrf_ucat_prev(i))) THEN
               invalid_prev_count = invalid_prev_count + 1
            ELSEIF (wdsrf_ucat_prev(i) < 0._r8) THEN
               invalid_prev_count = invalid_prev_count + 1
            ENDIF
         ENDDO
#ifdef USEMPI
         CALL mpi_allreduce (MPI_IN_PLACE, invalid_prev_count, 1, MPI_INTEGER, &
            MPI_SUM, p_comm_glb, p_err)
#endif
         IF (invalid_prev_count > 0) THEN
            IF (.not. legacy_restart) THEN
               CALL CoLM_stop ('GridRiverLake restart has invalid wdsrf_ucat_prev')
            ENDIF
            ! A legacy file has no transaction-level integrity guarantee. Keep
            ! it readable, but cold-start the paired BIF momentum/depth unit.
            wdsrf_ucat_prev = wdsrf_ucat
         ELSE
            wdsrf_ucat_prev_restart_found = .true.
         ENDIF
      ELSE
         wdsrf_ucat_prev = wdsrf_ucat
      ENDIF
      wdsrf_ucat_prev_valid = .true.

      IF (DEF_Reservoir_Method > 0) THEN
         IF (totalnumresv > 0) THEN
            CALL vector_read_and_scatter (file_restart, volresv, numresv, 'volresv', resv_data_address)
         ENDIF
      ENDIF

      ! Routing accumulator recovery. Absent variables (old-format restart
      ! or first cold start) keep the zero defaults from allocate_*, so the
      ! old behaviour is preserved when the file predates this persistence.
      IF (base_var_flags(3) == 1) THEN
         CALL ncio_read_bcast_serial (file_restart, 'acctime_rnof', acctime_rnof)
      ENDIF

      IF (base_var_flags(4) == 1) THEN
         CALL vector_read_and_scatter (file_restart, acc_rnof_uc, numucat, 'acc_rnof_uc', ucat_data_address)
      ENDIF

      IF (base_var_flags(5) == 1) THEN
         CALL vector_read_and_scatter (file_restart, volwater_ucat, numucat, 'volwater_ucat', ucat_data_address)
      ENDIF

      ! Validate finiteness before every ordered comparison so corrupted NaNs
      ! are rejected cleanly under -ffpe-trap=invalid. acc_rnof_uc may be
      ! signed by upstream forcing/numerics, so only its finiteness is an
      ! invariant here.
      invalid_base_count = 0
      DO i = 1, size(wdsrf_ucat)
         IF (.not. ieee_is_finite(wdsrf_ucat(i))) THEN
            invalid_base_count = invalid_base_count + 1
         ELSEIF (wdsrf_ucat(i) < 0._r8) THEN
            invalid_base_count = invalid_base_count + 1
         ENDIF
         IF (.not. ieee_is_finite(veloc_riv(i))) THEN
            invalid_base_count = invalid_base_count + 1
         ELSEIF (abs(veloc_riv(i)) > 50._r8) THEN
            invalid_base_count = invalid_base_count + 1
         ENDIF
         IF (base_var_flags(4) == 1) THEN
            IF (.not. ieee_is_finite(acc_rnof_uc(i))) &
               invalid_base_count = invalid_base_count + 1
         ENDIF
         IF (base_var_flags(5) == 1) THEN
            IF (.not. ieee_is_finite(volwater_ucat(i))) THEN
               invalid_base_count = invalid_base_count + 1
            ELSEIF (volwater_ucat(i) < 0._r8) THEN
               invalid_base_count = invalid_base_count + 1
            ENDIF
         ENDIF
      ENDDO
      IF (DEF_Reservoir_Method > 0 .and. totalnumresv > 0 .and. base_var_flags(6) == 1) THEN
         DO i = 1, size(volresv)
            IF (.not. ieee_is_finite(volresv(i))) THEN
               invalid_base_count = invalid_base_count + 1
            ELSEIF (volresv(i) < 0._r8) THEN
               invalid_base_count = invalid_base_count + 1
            ENDIF
         ENDDO
      ENDIF
      IF (p_is_master .and. base_var_flags(3) == 1) THEN
         IF (.not. ieee_is_finite(acctime_rnof)) THEN
            invalid_base_count = invalid_base_count + 1
         ELSEIF (acctime_rnof < 0._r8) THEN
            invalid_base_count = invalid_base_count + 1
         ENDIF
      ENDIF
#ifdef USEMPI
      CALL mpi_allreduce (MPI_IN_PLACE, invalid_base_count, 1, MPI_INTEGER, &
         MPI_SUM, p_comm_glb, p_err)
#endif
      IF (invalid_base_count > 0) THEN
         CALL CoLM_stop ('invalid GridRiverLake restart base state')
      ENDIF
      IF (base_var_flags(5) == 1) volwater_ucat_valid = .true.

      ! Note: levee restart (levsto) is read separately in grid_riverlake_flow_init
      ! after levee_init() allocates the levsto array. Same deferred-restart pattern.

      ! Note: bifurcation restart (pth_veloc, pth_momen) is read separately
      ! in grid_riverlake_flow_init after bifurcation_init() allocates the
      ! arrays. Same deferred-restart pattern as levee.

      ! Note: tracer restart is read separately in grid_riverlake_flow_init
      ! after river_lake_tracer_init() allocates the arrays. Same pattern as others.

      ! Note: particle-tracer restart is read through MOD_Tracer_Lifecycle,
      ! called from grid_riverlake_flow_init after particle species are initialized.

   END SUBROUTINE READ_GridRiverLakeTimeVars


   SUBROUTINE WRITE_GridRiverLakeTimeVars (file_restart)

   USE MOD_SPMD_Task
   USE MOD_Namelist,              only: DEF_Reservoir_Method, DEF_USE_LEVEE, DEF_USE_BIFURCATION
   USE MOD_NetCDFSerial
   USE MOD_Vector_ReadWrite
   USE MOD_Grid_RiverLakeNetwork, only: numucat, totalnumucat, ucat_data_address, &
      totalnpthout, npthlev_bif
   USE MOD_Grid_Reservoir,        only: numresv, totalnumresv, resv_data_address
   USE MOD_Grid_RiverLakeLevee,   only: levsto
   IMPLICIT NONE

   character(len=*), intent(in) :: file_restart

      IF (p_is_master) THEN
         CALL ncio_create_file (trim(file_restart))
         CALL ncio_define_dimension(file_restart, 'ucatch', totalnumucat)
         CALL ncio_write_serial (file_restart, 'gridriver_restart_schema', GRIDRIVER_RESTART_SCHEMA_VERSION)
         CALL ncio_write_serial (file_restart, 'gridriver_restart_complete', 0)
         CALL ncio_write_serial (file_restart, 'gridriver_restart_feature_bifurcation', &
            merge(1, 0, DEF_USE_BIFURCATION))
         CALL ncio_write_serial (file_restart, 'gridriver_restart_feature_levee', &
            merge(1, 0, DEF_USE_LEVEE))
      ENDIF

      CALL write_gridriver_ucatch_identity (file_restart)

      CALL vector_gather_and_write (&
         wdsrf_ucat, numucat, totalnumucat, ucat_data_address, file_restart, 'wdsrf_ucat', 'ucatch')

      IF (DEF_USE_BIFURCATION .and. totalnpthout > 0 .and. npthlev_bif > 0) THEN
         ! A restart may be requested before the first BIF routing flush. In
         ! that case current depth is the correct cold-start previous depth.
         IF (.not. wdsrf_ucat_prev_valid) THEN
            wdsrf_ucat_prev = wdsrf_ucat
            wdsrf_ucat_prev_valid = .true.
         ENDIF
         CALL vector_gather_and_write (&
            wdsrf_ucat_prev, numucat, totalnumucat, ucat_data_address, &
            file_restart, 'wdsrf_ucat_prev', 'ucatch')
      ENDIF

      CALL vector_gather_and_write (&
         veloc_riv, numucat, totalnumucat, ucat_data_address, file_restart, 'veloc_riv', 'ucatch')

      ! Persist the routing accumulator so a restart written mid routing
      ! period does not lose the queued land-runoff increments. Paired
      ! with the acctime_rnof / acc_rnof_uc reads in READ_*.
      IF (p_is_master) CALL ncio_write_serial (file_restart, 'acctime_rnof', acctime_rnof)
      CALL vector_gather_and_write (&
         acc_rnof_uc, numucat, totalnumucat, ucat_data_address, file_restart, 'acc_rnof_uc', 'ucatch')

      IF (DEF_Reservoir_Method > 0) THEN
         IF (totalnumresv > 0) THEN

            IF (p_is_master) CALL ncio_define_dimension(file_restart, 'reservoir', totalnumresv)

            CALL write_gridriver_reservoir_identity (file_restart)

            CALL vector_gather_and_write (&
               volresv, numresv, totalnumresv, resv_data_address, file_restart, 'volresv', 'reservoir')
         ENDIF
      ENDIF

      CALL vector_gather_and_write (&
         volwater_ucat, numucat, totalnumucat, ucat_data_address, file_restart, 'volwater_ucat', 'ucatch')

      IF (DEF_USE_LEVEE) THEN
         CALL vector_gather_and_write (&
            levsto, numucat, totalnumucat, ucat_data_address, file_restart, 'levsto', 'ucatch')
      ENDIF

      IF (DEF_USE_BIFURCATION) THEN
         CALL write_bifurcation_restart(file_restart)
      ENDIF

#ifdef TRACER
      CALL tracer_lifecycle_route_write_restart(file_restart)
#endif

   END SUBROUTINE WRITE_GridRiverLakeTimeVars

   SUBROUTINE deallocate_GridRiverLakeTimeVars

   IMPLICIT NONE

      IF (allocated (wdsrf_ucat )) deallocate (wdsrf_ucat )
      IF (allocated (wdsrf_ucat_prev)) deallocate (wdsrf_ucat_prev)
      IF (allocated (veloc_riv  )) deallocate (veloc_riv  )
      IF (allocated (momen_riv  )) deallocate (momen_riv  )
      IF (allocated (volresv    )) deallocate (volresv    )
      IF (allocated (volwater_ucat)) deallocate (volwater_ucat)
      IF (allocated (acc_rnof_uc)) deallocate (acc_rnof_uc)
      acctime_rnof = 0._r8
      wdsrf_ucat_prev_valid = .false.
      wdsrf_ucat_prev_restart_found = .false.
      restart_transaction_validated = .false.
      restart_feature_manifest_present = .false.
      restart_bifurcation_enabled = .false.
      restart_levee_enabled = .false.
      volwater_ucat_valid = .false.
      gridriver_restart_file = ''

   END SUBROUTINE deallocate_GridRiverLakeTimeVars

END MODULE MOD_Grid_RiverLakeTimeVars
#endif
