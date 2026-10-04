#include <define.h>

Module MOD_Ozone

!-----------------------------------------------------------------------
! !DESCRIPTION:
!  This module hold the plant physiological response to the ozone,
!  including vcmax response and stomata response.  Ozone concentration
!  can be either readin through Mod_OzoneData module or set to constant.
!
!  Original:
!  The Community Land Model version 5.0 (CLM5.0)
!
! !REVISIONS:
!  2022, Xingjie Lu: revised the CLM5 code to be compatible with CoLM
!        code structure.
!  2024, Fang Li : used the new ozone stress parameterization scheme
!        based on Li et al. (2024; GMD)
!-----------------------------------------------------------------------

   USE MOD_Precision
   USE MOD_Const_Physical, only: rgas
   USE MOD_Const_PFT, only: isevg, leaf_long, woody
   USE MOD_Grid
   USE MOD_DataType
   USE MOD_SpatialMapping
   USE MOD_Vars_1DForcing, only: forc_ozone
   USE MOD_Namelist, only: DEF_USE_OZONEDATA, DEF_OZONE_KO3
   IMPLICIT NONE

   character(len=256) :: file_ozone

   type(grid_type) :: grid_ozone

   type(block_data_real8_2d) :: f_ozone

   type(spatial_mapping_type) :: mg2p_ozone

   ! FIX upstream-bugs #72: the 3-hourly record currently held in forc_ozone
   integer :: itime_ozone = -1

   SAVE

   PUBLIC :: CalcOzoneStress
   PUBLIC :: ozone_pft_of_lct
   PUBLIC :: init_ozone_data
   PUBLIC :: update_ozone_data

CONTAINS

   ! FIX upstream-bugs #68: LCT patches have no PFT, and MOD_Thermal used to call
   ! LeafTemperature with ivt = 1, so every land cover class took the ozone
   ! parameters of the temperate needleleaf evergreen tree. Each class now takes
   ! the PFT closest in life form and leaf habit; classes without vegetation take
   ! 0 (bare), which leaves o3coefv = o3coefg = 1. Mixed forests keep PFT 1.
   integer FUNCTION ozone_pft_of_lct (lc) RESULT(ivt)

   IMPLICIT NONE
   integer, intent(in) :: lc

#ifdef LULC_USGS
   ! 0 ocean, 1 urban, 2-6 cropland and mosaics, 7 grassland, 8 shrubland,
   ! 9 mixed shrub/grass, 10 savanna, 11 DBF, 12 DNF, 13 EBF, 14 ENF, 15 mixed,
   ! 16 water, 17 herbaceous wetland, 18 wooded wetland, 19 barren,
   ! 20 herbaceous tundra, 21 wooded tundra, 22 mixed tundra, 23 bare tundra, 24 ice
   integer, parameter :: map(0:24) = (/ 0, 13, 15, 15, 15, 15, 15, 13, 10, 10, 14, &
                                        7,  3,  4,  1,  1,  0, 13,  7,  0, 12, 11, 12, 0, 0 /)
#else
   ! 0 ocean, 1 ENF, 2 EBF, 3 DNF, 4 DBF, 5 mixed, 6 closed shrub, 7 open shrub,
   ! 8 woody savanna, 9 savanna, 10 grassland, 11 wetland, 12 cropland, 13 urban,
   ! 14 cropland mosaic, 15 snow/ice, 16 barren, 17 water
   integer, parameter :: map(0:17) = (/ 0, 1, 4, 3, 7, 1, 9, 10, 7, 14, 13, 13, 15, &
                                        13, 15, 0, 0, 0 /)
#endif

      IF (lc >= lbound(map,1) .and. lc <= ubound(map,1)) THEN
         ivt = map(lc)
      ELSE
         ivt = 0
      ENDIF

   END FUNCTION ozone_pft_of_lct


   SUBROUTINE CalcOzoneStress (o3coefv,o3coefg, forc_ozone, forc_psrf, th, ram, &
                              rs, rb, lai, lai_old, ivt, o3uptake, sabv, deltim)
!-----------------------------------------------------------------------
! !DESCRIPTION:
!  Calculate Ozone Stress on both vcmax and stomata conductance.
!
!  convert o3 from mol/mol to nmol m^-3
!-----------------------------------------------------------------------
   real(r8), intent(out)   :: o3coefv
   real(r8), intent(out)   :: o3coefg
   real(r8), intent(inout) :: forc_ozone !ozone concentration (ppbv)
   real(r8), intent(in)    :: forc_psrf
   real(r8), intent(in)    :: th
   real(r8), intent(in)    :: ram
   real(r8), intent(in)    :: rs
   real(r8), intent(in)    :: rb
   real(r8), intent(in)    :: lai
   real(r8), intent(in)    :: lai_old
   integer , intent(in)    :: ivt
   real(r8), intent(inout) :: o3uptake
   real(r8), intent(in)    :: deltim
   real(r8), intent(in)   :: sabv  !solar radiation absorbed by vegetation (W/m**2)

   real(r8) :: o3concnmolm3   ! o3 concentration (nmol/m^3)
   real(r8) :: o3flux         ! instantaneous o3 flux (nmol m^-2 s^-1)
   real(r8) :: o3fluxcrit     ! instantaneous o3 flux beyond threshold (nmol m^-2 s^-1)
   real(r8) :: o3fluxperdt    ! o3 flux per timestep (mmol m^-2)
   real(r8) :: leafturn       ! leaf turnover time / mortality rate (per hour)
   real(r8) :: decay          ! o3uptake decay rate based on leaf lifetime (mmol m^-2)
   real(r8) :: lai_thresh     ! LAI threshold for LAIs that asymptote and don't
   real(r8) :: o3_flux_threshold !threshold below which o3flux is set to 0 (nmol m^-2 s^-1)

      ! FIX upstream-bugs #71: o3coefv/o3coefg are intent(out); when o3uptake /= 0 and
      ! ivt falls outside the five classes below (e.g. bare ground, ivt = 0) neither
      ! was assigned. Default both to "no stress".
      o3coefv = 1._r8
      o3coefg = 1._r8

      IF(.not. DEF_USE_OZONEDATA)THEN
         forc_ozone = 100._r8  ! ozone partial pressure [ppbv]
      ENDIF

      o3concnmolm3 = forc_ozone * (forc_psrf/(th * 8.314 ))

      ! calculate instantaneous flux
      o3flux = o3concnmolm3/ (DEF_OZONE_KO3*rs+ rb + ram)

      ! set lai_thresh
       IF (isevg(ivt)) THEN
         lai_thresh=0._r8 !so evergreens grow year-round
       ELSE  ! for deciduous vegetation
        IF(ivt == 10)THEN !temperate shrub
         lai_thresh=0.3_r8
        ELSE
         lai_thresh=0.5_r8
        end if
       end if


      ! set o3 flux threshold
      o3_flux_threshold=10._r8
      IF(ivt >= 1 .and. ivt <= 3)THEN  !Needleleaf tree
        o3_flux_threshold=0.8_r8
      ENDIF
      IF(ivt >= 4 .and. ivt <= 8)THEN  !Broadleaf tree
        o3_flux_threshold=1.0_r8
      ENDIF
      IF(ivt >= 9 .and. ivt <= 11)THEN !Shrub
        o3_flux_threshold=6.0_r8
      ENDIF
      IF(ivt >= 12 .and. ivt <= 14)THEN !Grass
        o3_flux_threshold = 1.6_r8
      ENDIF
      IF(ivt >= 15)THEN !Crop
        o3_flux_threshold = 0.5_r8
      ENDIF


      IF (o3flux < o3_flux_threshold) THEN
         o3fluxcrit = 0._r8
      ELSE
         o3fluxcrit = o3flux - o3_flux_threshold
      ENDIF

      ! calculate o3 flux per timestep
      IF(sabv > 0._r8)THEN  !daytime
        o3fluxperdt = o3fluxcrit * deltim * 0.000001_r8
      ELSE
        o3fluxperdt = 0._r8
      ENDIF

      IF (lai > lai_thresh) THEN
       ! o3 uptake decay
         IF (isevg(ivt)) THEN
            leafturn = 2._r8/(leaf_long(ivt)*365._r8*24._r8)
            decay = o3uptake * leafturn * deltim/3600._r8
         ELSE
            decay = o3uptake * max(0._r8,(1._r8-lai_old/lai))
         ENDIF

         !cumulative uptake (mmol m^-2)
         o3uptake = min(90._r8, max(0._r8, o3uptake + o3fluxperdt - decay))

      ELSE
         o3uptake = 0._r8
      ENDIF

      IF (o3uptake == 0._r8) THEN
         ! No o3 damage IF no o3 uptake
         o3coefv = 1._r8
         o3coefg = 1._r8
      ELSE
         ! Determine parameter values for this pft
        IF(ivt >= 1 .and. ivt <= 3)THEN  !Needleleaf tree
          o3coefv = max(0._r8, min(1._r8, 1.005_r8 - 0.0064_r8 * o3uptake))
          o3coefg = max(0._r8, min(1._r8, 0.965_r8 * o3uptake ** (-0.041)))
        ENDIF
        IF(ivt >= 4 .and. ivt <= 8)THEN  !Broadleaf tree
          o3coefv = max(0._r8, min(1._r8, 0.943_r8 * exp(-0.0085*o3uptake)))
          o3coefg = max(0._r8, min(1._r8, 0.943_r8 * exp(-0.0058*o3uptake)))
        ENDIF
        IF(ivt >= 9 .and. ivt <= 11)THEN !Shrub
          o3coefv = max(0._r8, min(1._r8, 1.000_r8-0.074_r8 * log(o3uptake)))
          o3coefg = max(0._r8, min(1._r8, 0.991_r8-0.060_r8 * log(o3uptake)))
        ENDIF
        IF(ivt >= 12 .and. ivt <= 14)THEN !Grass
          o3coefv = max(0._r8, min(1._r8, 0.997_r8 - 0.016_r8 * o3uptake))
          o3coefg = max(0._r8, min(1._r8, 0.989_r8 - 0.045_r8 * log(o3uptake)))
        ENDIF
        IF(ivt >= 15)THEN !Crop
          o3coefv = max(0._r8, min(1._r8, 0.909_r8 - 0.028_r8 * log(o3uptake)))
          o3coefg = max(0._r8, min(1._r8, 1.005_r8 - 0.169_r8 * tanh(o3uptake)))
        ENDIF
     ENDIF

   END SUBROUTINE CalcOzoneStress


   SUBROUTINE init_ozone_data (idate)

!-----------------------------------------------------------------------
! !DESCRIPTION:
!  open ozone netcdf file from DEF_dir_rawdata, read latitude and
!  longitude info.  Initialize Ozone data read in.
!-----------------------------------------------------------------------

   USE MOD_SPMD_Task
   USE MOD_Namelist
   USE MOD_TimeManager
   USE MOD_Grid
   USE MOD_NetCDFSerial
   USE MOD_NetCDFBlock
   USE MOD_LandPatch
   USE MOD_RangeCheck
   IMPLICIT NONE

   integer, intent(in) :: idate(3)

   ! Local Variables
   real(r8), allocatable :: lat(:), lon(:)
   integer :: itime
   integer :: iyear, month, mday
   character(len=8) :: syear, smonth

!     CALL julian2monthday(idate(1),idate(2),month,mday)
!     iyear = idate(1)
!     IF(idate(1) .lt. 2013)iyear = 2013
!     IF(idate(1) .gt. 2021)iyear = 2021
!     write(syear,"(I4.4)")  iyear
!     write(smonth,"(I2.2)") month
      IF (trim(DEF_file_Ozone) == 'null') THEN
         file_ozone = trim(DEF_dir_runtime) // '/Ozone/Global/OZONE-setgrid.nc'
      ELSE
         file_ozone = trim(DEF_file_Ozone)
      ENDIF
!      file_ozone = '/share/home/dq010/CoLM/data/rawdata/CROP-NITRIF/CoLMruntime/Ozone//Global/OZONE-setgrid.nc'

      CALL ncio_read_bcast_serial (file_ozone, 'lat', lat)
      CALL ncio_read_bcast_serial (file_ozone, 'lon', lon)

      CALL grid_ozone%define_by_center (lat, lon)

      CALL allocate_block_data (grid_ozone, f_ozone)

      CALL mg2p_ozone%build_arealweighted (grid_ozone, landpatch)

      ! FIX upstream-bugs #72: read the 3-hour window that contains the start time
      ! (was (idate(3)-1800)/10800+..., i.e. the window before it for a 00:00 start).
      itime = ozone_record (idate(1), idate(2), idate(3))
      itime_ozone = itime

      CALL ncio_read_block_time (file_ozone, 'OZONE', grid_ozone, itime, f_ozone)
      IF (DEF_USE_RangeCheck) THEN
      CALL check_block_data ('Ozone', f_ozone)
      ENDIF

      ! The first time-step precedes the first three-hour update boundary.
      ! Populate patch forcing immediately instead of leaving it uninitialized.
      CALL mg2p_ozone%grid2pset (f_ozone, forc_ozone)
      IF (DEF_USE_RangeCheck) THEN
      CALL check_vector_data ('Ozone', forc_ozone)
      ENDIF

   END SUBROUTINE init_ozone_data

   SUBROUTINE update_ozone_data (time, deltim)

!-----------------------------------------------------------------------
! !DESCRIPTION:
!  read ozone data during simulation
!-----------------------------------------------------------------------

   USE MOD_TimeManager
   USE MOD_Namelist
   USE MOD_NetCDFBlock
   USE MOD_RangeCheck
   IMPLICIT NONE

   type(timestamp), intent(in) :: time
   real(r8), intent(in) :: deltim

   ! Local Variables
   type(timestamp) :: time_next
   integer :: month, mday
   integer :: iyear, itime
   character(len=8) :: syear, smonth

      IF (trim(DEF_file_Ozone) == 'null') THEN
         file_ozone = trim(DEF_dir_runtime) // '/Ozone/Global/OZONE-setgrid.nc'
      ELSE
         file_ozone = trim(DEF_file_Ozone)
      ENDIF
!      file_ozone = '/share/home/dq010/CoLM/data/rawdata/CROP-NITRIF/CoLMruntime/Ozone/Global/OZONE-setgrid.nc'
      ! FIX upstream-bugs #72: use the 3-hour window that contains the current step
      ! (its start time), and read it whenever it differs from the one in memory.
      ! The old test/index read (time%sec-int(deltim))/10800, one window late, and
      ! gave itime = 0 for a 10800 s step at the start of a year.
      itime = ozone_record (time%year, time%day, time%sec)
      IF (itime /= itime_ozone) THEN
         itime_ozone = itime
         CALL ncio_read_block_time (file_ozone, 'OZONE', grid_ozone, itime, f_ozone)
         IF (DEF_USE_RangeCheck) THEN
         CALL check_block_data ('Ozone', f_ozone)
         ENDIF

         CALL mg2p_ozone%grid2pset (f_ozone, forc_ozone)
         IF (DEF_USE_RangeCheck) THEN
         CALL check_vector_data ('Ozone', forc_ozone)
         ENDIF
      ENDIF

   END SUBROUTINE update_ozone_data

   integer FUNCTION ozone_record (year, day, sec)
!-----------------------------------------------------------------------
! !DESCRIPTION:
!  1-based 3-hourly record (8 per day, 365-day climatology) of the window that
!  contains a time stamp given in CoLM's end form (sec in (0, 86400]): the end of
!  a day is the start of the next one. Day 366 of a leap year reuses day 365.
!-----------------------------------------------------------------------
   USE MOD_TimeManager, only: isleapyear
   IMPLICIT NONE
   integer, intent(in) :: year, day, sec
   integer :: d, s, maxday

      d = day
      s = sec
      IF (s >= 86400) THEN
         s = s - 86400
         d = d + 1
         maxday = 365
         IF (isleapyear(year)) maxday = 366
         IF (d > maxday) d = 1
      ENDIF
      ozone_record = s / 10800 + (min(d,365) - 1) * 8 + 1

   END FUNCTION ozone_record

END MODULE MOD_Ozone
! ---------- EOP ------------
