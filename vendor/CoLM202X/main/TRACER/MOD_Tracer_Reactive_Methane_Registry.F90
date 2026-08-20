#include <define.h>

! Methane is one of TRACER's four families (isotope/solute/particle/gas).
! TRACER itself is a runtime switch now (DEF_USE_TRACER, MOD_Namelist.F90);
! this module still needs BGC at compile time (it hard-USEs BGC carbon/
! nitrogen pools), so it stays behind an #ifdef and is only gated on
! DEF_USE_TRACER at the provider-registration call site
! (register_all_tracer_providers, via tracer_lifecycle_init <-
! land_tracer_init, called from CoLM.F90).
#ifdef BGC
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
#endif
