#include <define.h>

#ifdef TRACER
MODULE MOD_Tracer_Isotope_Registrations

#define TRACER_ISOTOPE_SPECIES(module_name, register_fn) USE module_name, only: register_fn
#include "tracer_isotope_species.inc"
#undef TRACER_ISOTOPE_SPECIES

   IMPLICIT NONE
   SAVE
   PRIVATE

   logical :: isotope_physics_registered = .false.

   PUBLIC :: ensure_isotope_physics_registered
   PUBLIC :: register_all_isotope_physics

CONTAINS

   SUBROUTINE ensure_isotope_physics_registered ()
      IF (isotope_physics_registered) RETURN
      CALL register_all_isotope_physics ()
      isotope_physics_registered = .true.
   END SUBROUTINE ensure_isotope_physics_registered

   SUBROUTINE register_all_isotope_physics ()
#define TRACER_ISOTOPE_SPECIES(module_name, register_fn) CALL register_fn ()
#include "tracer_isotope_species.inc"
#undef TRACER_ISOTOPE_SPECIES
   END SUBROUTINE register_all_isotope_physics

END MODULE MOD_Tracer_Isotope_Registrations
#endif
