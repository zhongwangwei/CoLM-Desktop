#!/bin/bash
#./create_defineh.bash GRID LULC_IGBP_PFT URBANON CaMaON BGCON CROPON
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
echo $1 $2 $3 $4 $5 $6

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
   LULC_IGBP_PFT="#undef LULC_IGBP_PFT"
   LULC_IGBP_PC="#undef LULC_IGBP_PC"
else
   if [ $2 = "LULC_IGBP" ];then
      LULC_USGS="#undef LULC_USGS"
      LULC_IGBP="#define LULC_IGBP"
      LULC_IGBP_PFT="#undef LULC_IGBP_PFT"
      LULC_IGBP_PC="#undef LULC_IGBP_PC"
   else
      if [ $2 = "LULC_IGBP_PFT" ];then
         LULC_USGS="#undef LULC_USGS"
         LULC_IGBP="#undef LULC_IGBP"
         LULC_IGBP_PFT="#define LULC_IGBP_PFT"
         LULC_IGBP_PC="#undef LULC_IGBP_PC"
      else
	 if [ $2 = "LULC_IGBP_PC" ];then
            LULC_USGS="#undef LULC_USGS"
            LULC_IGBP="#undef LULC_IGBP"
            LULC_IGBP_PFT="#undef LULC_IGBP_PFT"
            LULC_IGBP_PC="#define LULC_IGBP_PC"
	 else
	    echo "Error in argument 2, try (LULC_USGS, LULC_IGBP, LULC_IGBP_PFT, LULC_IGBP_PC)"
	    exit
	 fi
      fi
   fi
fi

#echo $LULC_USGS
#echo $LULC_IGBP
#echo $LULC_IGBP_PFT
#echo $LULC_IGBP_PC

if [ $3 = "URBANON" ];then
   URBAN="#define URBAN_MODEL"
else
   if [ $3 = "URBANOFF" ];then
      URBAN="#undef URBAN_MODEL"
   else
      echo "Error in argument 3, try (URBANON, URBANOFF)"
      exit
   fi 
fi
#echo $URBAN

if [ $4 = "CaMaON" ];then
   CaMa="#define CaMa_Flood"
else
   if [ $4 = "CaMaOFF" ];then
      CaMa="#undef CaMa_Flood"
   else
      echo "Error in argument 4, try (CaMaON, CaMaOFF)"
      exit
   fi
fi
#echo $CaMa

if [ $5 = "BGCON" ];then
   BGC="#define BGC"
else
   if [ $5 = "BGCOFF" ];then
      BGC="#undef BGC"
   else
      echo "Error in argument 5, try (BGCON, BGCOFF)"
      exit
   fi
fi
#echo $BGC

if [ $6 = "CROPON" ];then
   CROP="#define CROP"
else
   if [ $6 = "CROPOFF" ];then
      CROP="#undef CROP"
   else
      echo "Error in argument 6, try (CROPON, CROPOFF)"
   fi
fi

cat>include/define.h<<EOF
! 1. Spatial structure:
!    Select one of the following options.
$GRIDBASE
$CATCHMENT
$UNSTRUCTU
$SINGLEPOI

! 2. Land TYPE classification :
!    Select one of the following options.
$LULC_USGS
$LULC_IGBP
$LULC_IGBP_PFT
$LULC_IGBP_PC
! 2.1 Urban model setting (put it temporarily here):
$URBAN
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

! 7. If defined, BGC model is used.
$BGC

!    Conflicts :  only used when LULC_IGBP_PFT or LULC_IGBP_PC is defined.
#ifndef LULC_IGBP_PFT
#ifndef LULC_IGBP_PC
#undef BGC
#endif
#endif

! 7.1 If defined, CROP model is used
$CROP
!    Conflicts : only used when BGC is defined
#ifndef BGC
#undef CROP
#endif

! 8. If defined, open Land use and land cover change mode.
#undef LULCC

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
!      and MOD_Tracer_Reactive_BgcLink.F90) still needs BGC at compile time --
!      it hard-USEs BGC carbon/nitrogen pools -- so unlike TRACER itself this
!      stays a real compile-time gate (Makefile's METHANE_ENABLED / these
!      files' own "#ifdef BGC"; TRACER dropped out of the condition here since
!      it is no longer a macro). The PFT/PC requirement below is therefore
!      keyed on BGC alone now: whenever BGC survives the "Conflicts" #undef
!      above (i.e. only when LULC_IGBP_PFT or LULC_IGBP_PC is defined), the
!      condition below can never actually trigger -- it is retained as a
!      documented invariant, not a live trap, exactly as before this file
!      also required TRACER on this same combination.
#ifdef BGC
#if (!defined LULC_IGBP_PFT && !defined LULC_IGBP_PC)
#error "Methane (BGC) requires LULC_IGBP_PFT or LULC_IGBP_PC for pftfrac access."
#endif
#endif
EOF
