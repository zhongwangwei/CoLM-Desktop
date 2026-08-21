#!/bin/bash
#./create_defineh.bash GRID LULC_IGBP CaMaON CROPON
#
# Soil hydraulic scheme (Campbell vs. vanGenuchten) used to be a 4th
# positional argument here, picking between two compile-time macros. Both
# code paths are now always compiled in and the choice is a runtime
# namelist switch instead (DEF_USE_Campbell_SOIL_MODEL, MOD_Namelist.F90,
# default .false. i.e. vanGenuchten) -- so that argument slot is gone and
# every argument after it moved down by one.
#
# The tracer subsystem (TRACER, formerly a 7th positional argument here:
# TRACERON/TRACEROFF) went the same way. Both code paths (tracer on/off) are
# always compiled in now and the choice is a runtime namelist switch
# instead (DEF_USE_TRACER, MOD_Namelist.F90, default .false.) -- so that
# argument slot is gone too.
#
# LULC/BGC/URBAN_MODEL/LULCC group: the subgrid *structure* (LCT / PFT / PC)
# and BGC/URBAN_MODEL/LULCC are runtime switches now too (DEF_USE_LCT,
# DEF_USE_PFT, DEF_USE_PC, DEF_USE_BGC, DEF_URBAN_RUN, DEF_USE_LULCC --
# MOD_Namelist.F90, defaults matching the old LULC_IGBP/no-BGC/no-URBAN/
# no-LULCC compile baseline). main/BGC/, main/URBAN/, main/LULCC/ and the
# PFT/PC subgrid modules are always compiled in now, so this script's old
# 3rd argument (URBANON/URBANOFF) and 5th argument (BGCON/BGCOFF) are gone,
# and the old 2nd argument's LULC_IGBP_PFT/LULC_IGBP_PC choices collapsed
# into LULC_IGBP (subgrid structure is no longer a compile-time choice).
#
# What did NOT become a runtime switch, and why (see
# docs/plan-macro-runtime.md): land *classification* (LULC_USGS vs
# LULC_IGBP, this script's 2nd argument) and CROP. Both are blocked on the
# same underlying issue -- Fortran `parameter` constants
# (N_land_classification and the USGS/IGBP-keyed lookup tables in
# MOD_Const_LC.F90; N_PFT/N_CFT and the CROP-keyed lookup tables in
# MOD_Const_PFT.F90) have a DIFFERENT compiled value/extent per choice, so
# picking between them is a data-structure decision baked in at compile
# time, not a body-level IF. DEF_USE_USGS/DEF_USE_IGBP/DEF_USE_CROP are
# still namelist-visible (for schema/GUI display and validation), they are
# just read-only reflections of these two compile-time arguments rather
# than free runtime switches.
echo $1 $2 $3 $4

if [ $1 = "GRID" ];then
   GRIDBASE="#define GRIDBASED"
   CATCHMENT="#undef CATCHMENT"
   UNSTRUCTU="#undef UNSTRUCTURED"
   SINGLEPOI="#undef SinglePoint"
else
   if [ $1 = "CATCHMENT" ];then
      GRIDBASE="#undef GRIDBASED"
      CATCHMENT="#define CATCHMENT"
      UNSTRUCTU="#undef UNSTRUCTURED"
      SINGLEPOI="#undef SinglePoint"
   else
      if [ $1 = "UNSTRUCTURED" ];then
         GRIDBASE="#undef GRIDBASED"
         CATCHMENT="#undef CATCHMENT"
         UNSTRUCTU="#define UNSTRUCTURED"
         SINGLEPOI="#undef SinglePoint"
      else
	 if [ $1 = "SinglePoint" ];then
            GRIDBASE="#undef GRIDBASED"
            CATCHMENT="#undef CATCHMENT"
            UNSTRUCTU="#undef UNSTRUCTURED"
            SINGLEPOI="#define SinglePoint"
	 else
   	    echo "Error in argument 1, try (GRID, CATCHMENT, UNSTRUCTURED, SinglePoint)"
	    exit
	 fi
      fi
   fi
fi
#echo $GRIDBASE
#echo $CATCHMENT
#echo $UNSTRUCTU
#echo $SINGLEPOI
if [ $2 = "LULC_USGS" ];then
   LULC_USGS="#define LULC_USGS"
   LULC_IGBP="#undef LULC_IGBP"
else
   if [ $2 = "LULC_IGBP" ];then
      LULC_USGS="#undef LULC_USGS"
      LULC_IGBP="#define LULC_IGBP"
   else
      echo "Error in argument 2, try (LULC_USGS, LULC_IGBP)"
      exit
   fi
fi

#echo $LULC_USGS
#echo $LULC_IGBP

if [ $3 = "CaMaON" ];then
   CaMa="#define CaMa_Flood"
else
   if [ $3 = "CaMaOFF" ];then
      CaMa="#undef CaMa_Flood"
   else
      echo "Error in argument 3, try (CaMaON, CaMaOFF)"
      exit
   fi
fi
#echo $CaMa

if [ $4 = "CROPON" ];then
   CROP="#define CROP"
else
   if [ $4 = "CROPOFF" ];then
      CROP="#undef CROP"
   else
      echo "Error in argument 4, try (CROPON, CROPOFF)"
   fi
fi

cat>include/define.h<<EOF
! 1. Spatial structure:
!    Select one of the following options.
$GRIDBASE
$CATCHMENT
$UNSTRUCTU
$SINGLEPOI

! 2. Land TYPE classification: still a compile-time choice (see the
!    header comment above -- N_land_classification and its lookup tables
!    in MOD_Const_LC.F90 are parameter-sized differently per choice).
!    The subgrid *structure* that used to live here as LULC_IGBP_PFT/
!    LULC_IGBP_PC is a runtime switch now (DEF_USE_LCT/DEF_USE_PFT/
!    DEF_USE_PC, MOD_Namelist.F90) -- main/ and mksrfdata/'s PFT/PC code
!    is always compiled in, so those two macros no longer exist here.
$LULC_USGS
$LULC_IGBP
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
!     always defined, picked by this script's old 4th argument). Both
!     code paths are now always compiled in and the choice is a runtime
!     namelist switch instead (DEF_USE_Campbell_SOIL_MODEL,
!     share/MOD_Namelist.F90, default .false. i.e. vanGenuchten).
! 5.2 If defined, lateral flow is modeled.
#define  LATERAL_FLOW
!    Conflicts :
#ifndef CATCHMENT
#undef LATERAL_FLOW
#endif

! 6. If defined, CaMa-Flood model will be used.
$CaMa

#define GridRiverLakeFlow
!    Conflicts :
#if (defined CATCHMENT || defined SinglePoint)
#undef GridRiverLakeFlow
#endif

! 7. BGC model: always compiled in now (every main/BGC/ module). DEF_USE_BGC
!    (MOD_Namelist.F90, default .false.) picks whether it runs; the old
!    compile-time "Conflicts: only used when LULC_IGBP_PFT or
!    LULC_IGBP_PC is defined" cascade moved to MOD_Namelist.F90 too
!    (DEF_USE_BGC requires DEF_USE_PFT or DEF_USE_PC, validated there).

! 7.1 CROP model: still a compile-time macro (see the header comment
!     above -- N_PFT/N_CFT and their lookup tables in MOD_Const_PFT.F90
!     are parameter-sized differently per choice). DEF_USE_CROP
!     (MOD_Namelist.F90) is a read-only reflection of this macro, not a
!     free runtime switch.
$CROP
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
!     TRACER used to live here as a compile-time macro (this script's old
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
!     "#ifdef GridRiverLakeFlow", so they simply are not compiled without it.
!     The other 38 MOD_Tracer_*.F90 modules -- water isotopes, snow tracers,
!     forcing tracers -- are independent of the river network and are
!     perfectly meaningful for SinglePoint runs, where water-isotope
!     observations are common.
!
! 13.1 Methane (one of TRACER's four families: MOD_Tracer_Reactive_Methane*.F90
!      and MOD_Tracer_Reactive_BgcShim.F90) hard-USEs BGC carbon/nitrogen
!      pools. BGC is a runtime switch now too (see 7. above), so unlike
!      before, this is no longer a compile-time gate at all -- main/BGC/ is
!      always compiled in, so the hard USE always resolves. The runtime
!      requirement (methane needs DEF_USE_BGC = .true., which itself needs
!      DEF_USE_PFT or DEF_USE_PC) is enforced in MOD_Namelist.F90's
!      "DEF_USE_BGC requires DEF_USE_PFT or DEF_USE_PC" check, replacing
!      the old compile-time #error that used to live here.
EOF
