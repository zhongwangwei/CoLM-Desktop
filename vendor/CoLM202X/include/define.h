! 1. Spatial structure:
!    Select one of the following options.
#define GRIDBASED
#undef CATCHMENT
#undef UNSTRUCTURED
#undef SinglePoint

! 2. Land TYPE classification: still a compile-time choice (see the
!    header comment above -- N_land_classification and its lookup tables
!    in MOD_Const_LC.F90 are parameter-sized differently per choice).
!    The subgrid *structure* that used to live here as LULC_IGBP_PFT/
!    LULC_IGBP_PC is a runtime switch now (DEF_USE_LCT/DEF_USE_PFT/
!    DEF_USE_PC, MOD_Namelist.F90) -- PFT/PC code under main/ and mksrfdata/
!    is always compiled in, so those two macros no longer exist here.
#undef LULC_USGS
#define LULC_IGBP
! 2.1 Urban model: always compiled in now, DEF_URBAN_RUN
!     (MOD_Namelist.F90, default .false.) picks whether it runs.
#define URBAN_MODEL
#undef URBAN_LCZ

! 3. CoLMDEBUG / RangeCheck / SrfdataDiag used to live here as compile-time
!    macros. They are runtime switches now (DEF_USE_CoLMDEBUG,
!    DEF_USE_RangeCheck, DEF_USE_SrfdataDiag in share/MOD_Namelist.F90,
!    default .false.) so a single binary can carry all three debug code
!    paths and have them toggled on from case.nml instead of being
!    baked in per kernel.

! 4. If defined, MPI parallelization is enabled.
#define  USEMPI
!    Conflict: not used when defined SingPoint.
#if (defined SinglePoint)
#undef USEMPI
#endif

! 5. Hydrological process options.
! 5.1 Campbell_SOIL_MODEL / vanGenuchten_Mualem_SOIL_MODEL used to live
!     here as two mutually exclusive compile-time macros (exactly one
!     always defined, picked by the old 4th script argument). Both
!     code paths are now always compiled in and the choice is a runtime
!     namelist switch instead (DEF_USE_Campbell_SOIL_MODEL,
!     share/MOD_Namelist.F90, default .false. i.e. vanGenuchten).
! 5.2 If defined, lateral flow is modeled.
#define  CatchLateralFlow
!    Conflicts :
#ifndef CATCHMENT
#undef CatchLateralFlow
#endif

! 6. If defined, CaMa-Flood model will be used.
#undef CaMa_Flood

#define GridRiverLakeFlow
!    Conflicts :
#if (defined CATCHMENT || defined SinglePoint)
#undef GridRiverLakeFlow
#endif

! 7. BGC model: always compiled in now (every main/BGC/ module). DEF_USE_BGC
!    (MOD_Namelist.F90, default .false.) picks whether it runs; the old
!    compile-time conflict that required LULC_IGBP_PFT or LULC_IGBP_PC
!    moved to MOD_Namelist.F90 too
!    (DEF_USE_BGC requires DEF_USE_PFT or DEF_USE_PC, validated there).

! 7.1 CROP model: still a compile-time macro (see the header comment
!     above -- N_PFT/N_CFT and their lookup tables in MOD_Const_PFT.F90
!     are parameter-sized differently per choice). DEF_USE_CROP
!     (MOD_Namelist.F90) is a read-only reflection of this macro, not a
!     free runtime switch.
#undef CROP
!    Conflicts : only used when BGC is defined. BGC is a runtime switch
!    now, so this can no longer be checked here at compile time; the
!    equivalent check (DEF_USE_CROP requires DEF_USE_BGC) lives in
!    MOD_Namelist.F90.

! 8. Land use and land cover change mode: always compiled in now
!    (every main/LULCC/ module). DEF_USE_LULCC (MOD_Namelist.F90, default
!    .false.) picks whether it runs -- no existing kernel/preset ever
!    set the old "#define LULCC" here, so this stays .false. by default.

! 12b. If defined, extended canopy interception schemes are enabled.
#define extend_interception

! 13. Water tracer module (isotope / solute / particle / gas families).
!     TRACER used to live here as a compile-time macro (the old script
!     7th argument, TRACERON/TRACEROFF). Every main/TRACER module file is
!     now always compiled in and the choice is a runtime namelist switch
!     instead (DEF_USE_TRACER, share/MOD_Namelist.F90, default .false.) --
!     so that argument slot is gone and this line no longer exists.
!
!     TRACER requiring vanGenuchten (DEF_USE_Campbell_SOIL_MODEL = .false.)
!     used to be a compile-time #error here on (TRACER && Campbell_SOIL_MODEL);
!     Campbell/vanGenuchten became a runtime choice first (see above), so that
!     check already moved to MOD_Namelist.F90 -- it now runs whenever
!     DEF_USE_TRACER is .true., regardless of which soil scheme is picked.
!
!     NOTE: TRACER as a whole does NOT require GridRiverLakeFlow. The tracer
!     subsystem has four families (isotope, solute, particle, gas) and only
!     the river-lake ones need a river network: MOD_Tracer_RiverLake.F90 and
!     MOD_Tracer_Particle_Sediment.F90 guard themselves with
!     #ifdef GridRiverLakeFlow, so they simply are not compiled without it.
!     The other 38 MOD_Tracer_*.F90 modules -- water isotopes, snow tracers,
!     forcing tracers -- are independent of the river network and are
!     perfectly meaningful for SinglePoint runs, where water-isotope
!     observations are common.
!
! 13.1 Methane (one of the four TRACER families: MOD_Tracer_Reactive_Methane*.F90
!      and MOD_Tracer_Reactive_BgcShim.F90) hard-USEs BGC carbon/nitrogen
!      pools. BGC is a runtime switch now too (see 7. above), so unlike
!      before, this is no longer a compile-time gate at all -- main/BGC/ is
!      always compiled in, so the hard USE always resolves. The runtime
!      requirement (methane needs DEF_USE_BGC = .true., which itself needs
!      DEF_USE_PFT or DEF_USE_PC) is enforced by the MOD_Namelist.F90
!      DEF_USE_BGC-requires-PFT-or-PC check, replacing
!      the old compile-time #error that used to live here.
