! 1. Spatial structure:
!    Select one of the following options.
#define GRIDBASED
#undef CATCHMENT
#undef UNSTRUCTURED
#undef SinglePoint

! 2. Land subgrid type classification:
!    Select one of the following options.
#undef LULC_USGS
#undef LULC_IGBP
#define LULC_IGBP_PFT
#undef LULC_IGBP_PC

! 2.1 3D Urban model (put it temporarily here):
#undef URBAN_MODEL
!    Dependence:  only LULC_IGBP subgrid type for
!    single point URBAN_MODEL right now.
#if (defined URBAN_MODEL && defined SinglePoint)
#define LULC_IGBP
#undef LULC_USGS
#undef LULC_IGBP_PFT
#undef LULC_IGBP_PC
#endif

! 3. If defined, debug information is output.
#undef CoLMDEBUG
! 3.1 If defined, range of variables is checked.
#undef RangeCheck
! 3.1 If defined, surface data in vector is mapped to gridded data for checking.
#undef SrfdataDiag

! 4. If defined, MPI parallelization is enabled.
#define USEMPI
!    Conflict: not used when defined SingPoint.
#if (defined SinglePoint)
#undef USEMPI
#endif

! 5. Hydrological process options.
! 5.1 Two soil hydraulic models can be used.
#undef   Campbell_SOIL_MODEL
#define  vanGenuchten_Mualem_SOIL_MODEL
! 5.2 If defined, lateral flow is modeled.
#define CatchLateralFlow
!    Conflicts :
#ifndef CATCHMENT
#undef CatchLateralFlow
#endif

! 6. If defined, CaMa-Flood model will be used.
#undef CaMa_Flood
#if (defined SinglePoint)
#undef CaMa_Flood
#endif
#ifndef USEMPI
#undef CaMa_Flood
#endif

#define GridRiverLakeFlow
!    Conflicts :
#if (defined CATCHMENT || defined SinglePoint)
#undef GridRiverLakeFlow
#endif

! NOTE: the former standalone river-lake sediment macro has been retired.
! Sediment is now a TRACER 'particle' species and is compiled/activated
! under #ifdef TRACER together with GridRiverLakeFlow.

! 7. If defined, BGC model is used.
#define BGC

!    Conflicts :  only used when LULC_IGBP_PFT is defined.
#ifndef LULC_IGBP_PFT
#ifndef LULC_IGBP_PC
#undef BGC
#endif
#endif
! 7.1 If defined, CROP model is used
#define CROP
!    Conflicts : only used when BGC is defined
#ifndef BGC
#undef CROP
#endif

! 8. If defined, open Land use and land cover change mode.
#undef LULCC

! 9. If defined, data assimilation is used.
#undef DataAssimilation
#if (defined DataAssimilation)
#define LULC_IGBP
#undef LULC_USGS
#undef LULC_IGBP_PFT
#undef LULC_IGBP_PC
#endif

! 10. Interface to AI model.
#undef USESplitAI

! 11. External lake models.
#undef EXTERNAL_LAKE

! 12. Hyperspectral scheme.
#undef HYPERSPECTRAL

! 12b. If defined, extended canopy interception schemes are enabled.
#define extend_interception

! 13. If defined, the tracer subsystem is enabled (isotope, solute,
!     particle, and gas families).
!     This repository template currently enables TRACER; switch to #undef
!     TRACER only for builds that intentionally exclude all tracer species.
#define TRACER
!    Conflicts: TRACER requires VariablySaturatedFlow soil hydrology
!    (vanGenuchten_Mualem_SOIL_MODEL). Campbell_SOIL_MODEL cannot silently
!    disable TRACER because that changes the requested physics at compile time.
#if (defined TRACER) && (defined Campbell_SOIL_MODEL)
#error "TRACER requires vanGenuchten_Mualem_SOIL_MODEL; disable TRACER explicitly before using Campbell_SOIL_MODEL"
#endif
!    Dependency: only the routing-borne TRACER species require GridRiverLakeFlow
!    (in-river isotope transport in MOD_Tracer_RiverLake, and the sediment
!    particle species). Those are compiled only when GridRiverLakeFlow is also
!    defined. Land tracers do not route and build without it, so no hard error
!    is raised here -- SinglePoint forces GridRiverLakeFlow off (section 6),
!    which made TRACER impossible to build for a single point at all.

! 13b. Methane gas provider.
!     Activation is runtime: register a tracer named "CH4" or "METHANE"
!     with type="gas" in the &nl_colm DEF_TRACER_NAMES / DEF_TRACER_TYPES
!     namelist. The methane module is compiled whenever both TRACER and BGC
!     are defined; its lifecycle registrar attaches the CH4 hooks and index.
!     A configured CH4 row without that compiled provider fails at startup.
!     Additional dependency: requires LULC_IGBP_PFT or LULC_IGBP_PC for
!     pftfrac access (per-PFT NPP and root-respiration aggregation).
#if (defined TRACER) && (defined BGC)
#if (!defined LULC_IGBP_PFT && !defined LULC_IGBP_PC)
#error "Methane (TRACER+BGC) requires LULC_IGBP_PFT or LULC_IGBP_PC for pftfrac access."
#endif
#endif
