#include <define.h>

! Methane is one of TRACER's four families (isotope/solute/particle/gas).
! TRACER itself is a runtime switch now (DEF_USE_TRACER, MOD_Namelist.F90).
! This module hard-USEs BGC carbon/nitrogen pool modules (MOD_BGC_Vars_*),
! which used to make BGC a real compile-time gate for it; BGC is a runtime
! switch now too (DEF_USE_BGC, MOD_Namelist.F90), so those pool modules
! always compile in and this module always compiles in alongside them.
! Runtime activation still only happens via DEF_USE_TRACER at the
! provider-registration call site (register_all_tracer_providers, via
! tracer_lifecycle_init <- land_tracer_init, called from CoLM.F90); methane
! specifically also requires DEF_USE_BGC = .true. (validated in
! MOD_Tracer_Defs.F90:validate_tracer_descriptor and in MOD_Namelist.F90's
! BGC/CROP conflict-check block).
MODULE MOD_Tracer_Reactive_Methane_Preprocessing

   USE MOD_SPMD_Task, only: CoLM_stop
   USE MOD_Tracer_Defs, only: tracer_defs_init, tracer_index_for_name, tracer_param_file_for_index, &
      tracer_is_gas
   USE MOD_Tracer_Reactive_Methane_Const, only: DEF_METHANE, read_methane_namelist
   USE MOD_Tracer_Reactive_Methane_Registry, only: igas_ch4, methane_is_active

   IMPLICIT NONE
   PRIVATE

   PUBLIC :: methane_preprocessing_requirements

CONTAINS

   SUBROUTINE methane_preprocessing_requirements (requires_lake_soilc, requires_spatial_ph)
      IMPLICIT NONE
      logical, intent(out) :: requires_lake_soilc, requires_spatial_ph

      character(len=512) :: file_param
      logical :: found

      requires_lake_soilc = .false.
      requires_spatial_ph = .false.

      CALL tracer_defs_init ()
      igas_ch4 = tracer_index_for_name('CH4', 'METHANE')
      IF (.not. methane_is_active()) RETURN
      IF (.not. tracer_is_gas(igas_ch4)) THEN
         CALL CoLM_stop (' ***** ERROR: CH4/METHANE preprocessing descriptor must use family=gas')
      ENDIF

      ! Use the runtime parser so keyed aliases, positional files, separators,
      ! and null handling cannot drift between mksrfdata and colm.x.
      CALL tracer_param_file_for_index (igas_ch4, 'CH4,METHANE', file_param, found)
      IF (.not. found) THEN
         CALL CoLM_stop (' ***** ERROR: CH4 requires DEF_TRACER_PARAM_FILES to include a CH4 parameter file')
      ENDIF
      CALL read_methane_namelist (file_param)

      requires_lake_soilc = DEF_METHANE%allowlakeprod
      requires_spatial_ph = DEF_METHANE%use_spatial_ph

   END SUBROUTINE methane_preprocessing_requirements

END MODULE MOD_Tracer_Reactive_Methane_Preprocessing
