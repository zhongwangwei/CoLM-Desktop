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
MODULE MOD_Tracer_Reactive_Methane_Registry
!=======================================================================
! Methane's tracer index is populated by lifecycle provider registration.
!
!=======================================================================

   IMPLICIT NONE
   SAVE
   PRIVATE

   ! Sentinel value: tracer absent (or non-reactive) in the registry.
   integer, parameter :: METHANE_GAS_ABSENT = -1

   integer :: igas_ch4 = METHANE_GAS_ABSENT

   PUBLIC :: methane_is_active
   PUBLIC :: igas_ch4, METHANE_GAS_ABSENT

CONTAINS

   logical FUNCTION methane_is_active ()
      methane_is_active = (igas_ch4 > 0)
   END FUNCTION methane_is_active


END MODULE MOD_Tracer_Reactive_Methane_Registry
