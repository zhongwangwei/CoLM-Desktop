! Implemented TRACER lifecycle providers.
!
! Parameter-only isotope/solute tracers do not belong here. Add one entry
! only when a compiled species owns state or phase callbacks.
#ifdef BGC
TRACER_LIFECYCLE_PROVIDER(MOD_Tracer_Reactive_Methane, ch4_register_tracer_provider)
#endif
! The sediment particle species is a river-routing tracer and hard-USEs the
! GridRiverLakeFlow-only routing modules; register it only when routing is
! compiled in. Land tracers do not route and are unaffected when
! GridRiverLakeFlow is off (e.g. SinglePoint).
#ifdef GridRiverLakeFlow
TRACER_LIFECYCLE_PROVIDER(MOD_Tracer_Particle_Sediment, register_sediment_tracer_provider)
#endif
