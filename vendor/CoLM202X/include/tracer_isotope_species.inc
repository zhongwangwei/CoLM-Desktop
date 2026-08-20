! Isotope fractionation physics manifest.
!
! Add one line per implemented isotope physics package:
!   TRACER_ISOTOPE_SPECIES(Module_Name, register_physics_subroutine)
!
! MOD_Tracer_Isotope_Registrations.F90 expands this single manifest into
! both USE statements and registration CALLs. Do not add placeholder isotope
! species; add a line only when the physics implementation exists.
TRACER_ISOTOPE_SPECIES(MOD_Tracer_Isotope_O18, register_o18_isotope_physics)
TRACER_ISOTOPE_SPECIES(MOD_Tracer_Isotope_HDO, register_hdo_isotope_physics)
