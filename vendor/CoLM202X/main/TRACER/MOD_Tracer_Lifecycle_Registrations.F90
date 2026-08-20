#include <define.h>

#ifdef TRACER
SUBROUTINE register_all_tracer_providers ()
!=======================================================================
! Single manifest for compiled species that attach lifecycle hooks.
! Generic isotope/solute tracers remain descriptor-only.
!=======================================================================

#define TRACER_LIFECYCLE_PROVIDER(module_name, register_fn) USE module_name, only: register_fn
#include "tracer_lifecycle_providers.inc"
#undef TRACER_LIFECYCLE_PROVIDER

   IMPLICIT NONE

#define TRACER_LIFECYCLE_PROVIDER(module_name, register_fn) CALL register_fn ()
#include "tracer_lifecycle_providers.inc"
#undef TRACER_LIFECYCLE_PROVIDER

END SUBROUTINE register_all_tracer_providers
#endif
