#!/bin/bash
#./create_defineh.bash GRID LULC_IGBP_PFT URBANON vanGenu CaMaON BGCON CROPON TRACERON
echo $1 $2 $3 $4 $5 $6 $7 ${8:-TRACEROFF}

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

if [ $4 = "Campbell" ];then
   CAMPBELL="#define Campbell_SOIL_MODEL"
   VENGENU="#undef vanGenuchten_Mualem_SOIL_MODEL"
else
   if [ $4 = "vanGenu" ];then
      CAMPBELL="#undef Campbell_SOIL_MODEL"
      VENGENU="#define vanGenuchten_Mualem_SOIL_MODEL"
   else
      echo "Error in argument 4, try (Campbell, vanGenu)"
      exit
   fi
fi

#echo $CAMPBELL
#echo $VENGENU

if [ $5 = "CaMaON" ];then
   CaMa="#define CaMa_Flood"
else
   if [ $5 = "CaMaOFF" ];then
      CaMa="#undef CaMa_Flood"
   else
      echo "Error in argument 5, try (CaMaON, CaMaOFF)"
      exit
   fi
fi
#echo $CaMa

if [ $6 = "BGCON" ];then
   BGC="#define BGC"
else
   if [ $6 = "BGCOFF" ];then
      BGC="#undef BGC"
   else
      echo "Error in argument 6, try (BGCON, BGCOFF)"
      exit
   fi
fi
#echo $BGC

if [ $7 = "CROPON" ];then
   CROP="#define CROP"
else
   if [ $7 = "CROPOFF" ];then
      CROP="#undef CROP"
   else
      echo "Error in argument 7, try (CROPON, CROPOFF)"
   fi
fi

if [ "${8:-TRACEROFF}" = "TRACERON" ];then
   TRACER="#define TRACER"
else
   if [ "${8:-TRACEROFF}" = "TRACEROFF" ];then
      TRACER="#undef TRACER"
   else
      echo "Error in argument 8, try (TRACERON, TRACEROFF)"
      exit
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

! 3. If defined, debug information is output.
#define CoLMDEBUG
! 3.1 If defined, range of variables is checked.
#define RangeCheck
! 3.2 If defined, surface data in vector is mapped to gridded data for checking.
#undef SrfdataDiag

! 4. If defined, MPI parallelization is enabled.
#define  USEMPI
!    Conflict: not used when defined SingPoint.
#if (defined SinglePoint)
#undef USEMPI
#endif

! 5. Hydrological process options.
! 5.1 Two soil hydraulic models can be used.
$CAMPBELL
$VENGENU
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

! 13. If defined, water tracer module is enabled.
$TRACER
#if (defined TRACER) && (defined Campbell_SOIL_MODEL)
#error "TRACER requires vanGenuchten_Mualem_SOIL_MODEL; disable TRACER explicitly before using Campbell_SOIL_MODEL"
#endif
! NOTE: TRACER as a whole does NOT require GridRiverLakeFlow. The tracer
! subsystem has four families (isotope, solute, particle, gas) and only the
! river-lake ones need a river network: MOD_Tracer_RiverLake.F90 and
! MOD_Tracer_Particle_Sediment.F90 already guard themselves with
! "#ifdef GridRiverLakeFlow", so they simply are not compiled without it.
! The other 38 MOD_Tracer_*.F90 modules -- water isotopes, snow tracers,
! forcing tracers -- are independent of the river network and are perfectly
! meaningful for SinglePoint runs, where water-isotope observations are common.
!
! A blanket #error here made TRACER impossible for every SinglePoint build,
! because SinglePoint unconditionally undefines GridRiverLakeFlow above.
#if (defined TRACER) && (defined BGC)
#if (!defined LULC_IGBP_PFT && !defined LULC_IGBP_PC)
#error "Methane (TRACER+BGC) requires LULC_IGBP_PFT or LULC_IGBP_PC for pftfrac access."
#endif
#endif
EOF
