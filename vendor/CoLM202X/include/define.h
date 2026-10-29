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
#if (defined GridRiverLakeFlow && !defined USEMPI)
#error GridRiverLakeFlow requires USEMPI
#endif
#ifdef GridRiverLakeSediment
#error GridRiverLakeSediment is not coupled to this version yet
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

! 13. Material transport (water isotopes, solutes, river sediment, methane).
!     Upstream names this macro `MaterialTransport` (it used to be `TRACER`,
!     the old script 7th argument TRACERON/TRACEROFF). Here it is a
!     **runtime** choice again: every main/MATERIAL/ module is always
!     compiled in and each family is switched on from the namelist
!     (DEF_USE_ISOTOPE / DEF_SOLUTE_NAMES / DEF_USE_METHANE, in
!     main/MATERIAL/MOD_MT_Namelist.F90, all off by default). With every
!     family off the results are unchanged. No macro is defined here: the
!     preprocessing guards upstream writes around material-transport code
!     were expanded away exactly like the old TRACER ones.
!
!     Upstream's compile-time rules, kept here as documentation because
!     this build no longer branches on the macro (the runtime checks live
!     in MOD_MT_Namelist.F90 / MOD_Namelist.F90):
!       * needs vanGenuchten_Mualem (not Campbell) -- runtime:
!         DEF_USE_Campbell_SOIL_MODEL must be .false.;
!       * not supported with CATCHMENT (unit-catchment) grids;
!       * not supported with URBAN_MODEL;
!       * turns GridRiverLakeFlow on and CaMa_Flood off.
!
!     NOTE: material transport as a whole does NOT require GridRiverLakeFlow.
!     It has four families (isotope, solute, particle, gas) and only the
!     river-lake ones need a river network: MOD_MT_RiverLake.F90 and
!     MOD_MT_Sediment.F90 guard themselves with #ifdef GridRiverLakeFlow,
!     so they simply are not compiled without it. The isotope/solute/snow/
!     forcing modules are independent of the river network and work for
!     SinglePoint runs, where water-isotope observations are common.
!
! 13.1 Methane (MOD_MT_Methane*.F90) hard-USEs BGC carbon/nitrogen pools.
!      BGC is a runtime switch too (see 7. above), so this is not a
!      compile-time gate: main/BGC/ is always compiled in. The runtime
!      requirement (DEF_USE_METHANE needs DEF_USE_BGC = .true., which itself
!      needs DEF_USE_PFT or DEF_USE_PC) is enforced by MOD_Namelist.F90.
