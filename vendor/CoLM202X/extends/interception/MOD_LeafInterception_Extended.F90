#include <define.h>
MODULE MOD_LeafInterception
! NM-1-v2: canopy phase-change heat is carried explicitly via
! canopy_phase_heat/canopy_phase_heat_out, not deferred silently to
! LeafTemperature.
! -----------------------------------------------------------------
! !DESCRIPTION:
! For calculating vegetation canopy precipitation interception.
!
! This MODULE is the coupler for the colm and CaMa-Flood model.

!ANCILLARY FUNCTIONS AND SUBROUTINES
!-------------------
   !* :SUBROUTINE:"LEAF_interception_CoLM2014" : Leaf interception and drainage schemes based on colm2014 version
   !* :SUBROUTINE:"LEAF_interception_CoLM202x" : Leaf interception and drainage schemes besed on new colm version (under development)
   !* :SUBROUTINE:"LEAF_interception_CLM4"     : Leaf interception and drainage schemes modified from CLM4
   !* :SUBROUTINE:"LEAF_interception_CLM5"     : Leaf interception and drainage schemes modified from CLM5
   !* :SUBROUTINE:"LEAF_interception_NOAHMP"   : Leaf interception and drainage schemes modified from Noah-MP
   !* :SUBROUTINE:"LEAF_interception_MATSIRO"  : Leaf interception and drainage schemes modified from MATSIRO 2021 version
   !* :SUBROUTINE:"LEAF_interception_VIC"      : Leaf interception and drainage schemes modified from VIC
   !* :SUBROUTINE:"LEAF_interception_JULES"    : Leaf interception and drainage schemes modified from JULES
   !* :SUBROUTINE:"LEAF_interception_pftwrap"  : wrapper for pft land use classification

!REVISION HISTORY:
!----------------
   ! 2026.01     Zhongwang Wei: Fully revise CLM4,5,Noah-MP,MATSIRO,VIC and JULES schemes.
   ! 2024.04     Hua Yuan: add option to account for vegetation snow process based on Niu et al., 2004
   ! 2023.07     Hua Yuan: remove wrapper PC by using PFT leaf interception
   ! 2023.06     Shupeng Zhang @ SYSU
   ! 2023.02.23  Zhongwang Wei @ SYSU
   ! 2021.12.12  Zhongwang Wei @ SYSU
   ! 2020.10.21  Zhongwang Wei @ SYSU
   ! 2019.06     Hua Yuan: 1) add wrapper for PFT and PC, and 2) remove sigf by using lai+sai
   ! 2014.04     Yongjiu Dai
   ! 2002.08.31  Yongjiu Dai
   USE MOD_Precision
   USE MOD_Const_Physical, only: tfrz, denh2o, denice, cpliq, cpice, hfus
   USE MOD_Namelist, only: DEF_Interception_scheme, DEF_VEG_SNOW, DEF_MATSIRO_CWCAP_SCALE

   IMPLICIT NONE

   real(r8), parameter ::  CICE        = 2.094E06  !specific heat capacity of ice (j/m3/k)
   real(r8), parameter ::  bp          = 20.
   real(r8), parameter ::  CWAT        = 4.188E06  !specific heat capacity of water (j/m3/k)
   real(r8), parameter ::  pcoefs(2,2) = reshape((/20.0_r8, 0.206e-8_r8, 0.0001_r8, 0.9999_r8/), (/2,2/))

   ! Minimum significant precipitation rate threshold [mm/s]
   ! Used across all schemes for numerical stability
   real(r8), parameter ::  PRECIP_THRESHOLD = 1.0e-8_r8

   ! Tolerance for interception water balance checks [mm]
   ! Used by check_interception_balance subroutine under CoLMDEBUG
   real(r8), parameter ::  INTERCEPTION_BALANCE_TOL = 1.0e-5_r8

   !----------------------- Dummy argument --------------------------------
   real(r8) :: satcap                     ! maximum allowed water on canopy [mm]
   real(r8) :: satcap_rain                ! maximum allowed rain on canopy [mm]
   real(r8) :: satcap_snow                ! maximum allowed snow on canopy [mm]
   real(r8) :: lsai                       ! sum of leaf area index and stem area index [-]
   real(r8) :: chiv                       ! leaf angle distribution factor
   real(r8) :: ppc                        ! convective precipitation in time-step [mm]
   real(r8) :: ppl                        ! large-scale precipitation in time-step [mm]
   real(r8) :: p0                         ! precipitation in time-step [mm]
   real(r8) :: fpi                        ! coefficient of interception
   real(r8) :: fpi_rain                   ! coefficient of interception of rain
   real(r8) :: fpi_snow                   ! coefficient of interception of snow
   real(r8) :: alpha_rain                 ! coefficient of interception of rain
   real(r8) :: alpha_snow                 ! coefficient of interception of snow
   real(r8) :: pinf                       ! interception of precipitation in time step [mm]
   real(r8) :: tti_rain                   ! direct rain throughfall in time step [mm]
   real(r8) :: tti_snow                   ! direct snow throughfall in time step [mm]
   real(r8) :: tex_rain                   ! canopy rain drainage in time step [mm]
   real(r8) :: tex_snow                   ! canopy snow drainage in time step [mm]
   real(r8) :: vegt                       ! sigf*lsai
   real(r8) :: xs                         ! proportion of the grid area where the intercepted rainfall
                                          ! plus the preexisting canopy water storage
   real(r8)  :: unl_snow_temp,U10,unl_snow_wind,unl_snow
   real(r8)  :: ap, cp, aa1, bb1, exrain, arg, w
   real(r8)  :: thru_rain, thru_snow
   real(r8)  :: xsc_rain, xsc_snow

   real(r8)  :: fvegc                     ! vegetation fraction
   real(r8)  :: FT                        ! the temperature factor for snow unloading
   real(r8)  :: FV                        ! the wind factor for snow unloading
   real(r8)  :: ICEDRIP                   ! snow unloading
   real(r8)  :: ICEDRIP_OLD              ! old-canopy snow unloading rate [mm/s]
   real(r8)  :: ICEDRIP_NEW              ! same-step new-snow unloading rate [mm/s]

   real(r8)  :: ldew_smelt
   real(r8)  :: ldew_frzc
   real(r8)  :: FP
   real(r8)  :: int_rain
   real(r8)  :: int_snow
CONTAINS

   SUBROUTINE LEAF_interception_CoLM2014 (deltim,dewmx,forc_us,forc_vs,chil,sigf,lai,sai,tair,tleaf,&
                                          prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler,bifall,&
                                          ldew,ldew_rain,ldew_snow,z0m,hu,pg_rain,pg_snow,qintr,qintr_rain,qintr_snow,&
                                          gross_intr_rain,gross_intr_snow,&
                                          xsc_rain_out,xsc_snow_out,&
                                          ldew_smelt_out,ldew_frzc_out,&
                                          canopy_phase_heat_out)
!DESCRIPTION
!===========
   ! Calculation of  interception and drainage of precipitation
   ! the treatment are based on Sellers et al. (1996)

!Original Author:
!-------------------
   !canopy interception scheme modified by Yongjiu Dai based on Sellers et al. (1996)

!References:
!-------------------
   !---Dai, Y., Zeng, X., Dickinson, R.E., Baker, I., Bonan, G.B., BosiloVICh,
   !   M.G., Denning, A.S., Dirmeyer, P.A., Houser, P.R., Niu, G. and Oleson,
   !   K.W., 2003.  The common land model. Bulletin of the American
   !   Meteorological Society, 84(8), pp.1013-1024.

   !---Lawrence, D.M., Thornton, P.E., Oleson, K.W. and Bonan, G.B., 2007.  The
   !   partitioning of evapotranspiration into transpiration, soil evaporation,
   !   and canopy evaporation in a GCM: Impacts on land-atmosphere interaction.
   !   Journal of Hydrometeorology, 8(4), pp.862-880.

   !---Oleson, K., Dai, Y., Bonan, B., BosiloVIChm, M., Dickinson, R.,
   !   Dirmeyer, P., Hoffman, F., Houser, P., Levis, S., Niu, G.Y. and
   !   Thornton, P., 2004.  Technical description of the community land model
   !   (CLM).

   !---Sellers, P.J., Randall, D.A., Collatz, G.J., Berry, J.A., Field, C.B.,
   !   Dazlich, D.A., Zhang, C., Collelo, G.D. and Bounoua, L., 1996. A revised
   !   land surface parameterization (SiB2) for atmospheric GCMs.  Part I:
   !   Model formulation. Journal of climate, 9(4), pp.676-705.

   !---Sellers, P.J., Tucker, C.J., Collatz, G.J., Los, S.O., Justice, C.O.,
   !   Dazlich, D.A. and Randall, D.A., 1996.  A revised land surface
   !   parameterization (SiB2) for atmospheric GCMs. Part II: The generation of
   !   global fields of terrestrial biophysical parameters from satellite data.
   !   Journal of climate, 9(4), pp.706-737.


!ANCILLARY FUNCTIONS AND SUBROUTINES
!-------------------

!REVISION HISTORY
!----------------
   !---2024.04.16  Hua Yuan: add option to account for vegetation snow process based on Niu et al., 2004
   !---2023.02.21  Zhongwang Wei @ SYSU : Snow and rain interception
   !---2021.12.08  Zhongwang Wei @ SYSU
   !---2019.06     Hua Yuan: remove sigf and USE lai+sai for judgement.
   !---2014.04     Yongjiu Dai
   !---2002.08.31  Yongjiu Dai
!=======================================================================

   IMPLICIT NONE

   real(r8), intent(in) :: deltim       !seconds in a time step [second]
   real(r8), intent(in) :: dewmx        !maximum dew [mm]
   real(r8), intent(in) :: forc_us      !wind speed
   real(r8), intent(in) :: forc_vs      !wind speed
   real(r8), intent(in) :: chil         !leaf angle distribution factor
   real(r8), intent(in) :: prc_rain     !convective rainfall [mm/s]
   real(r8), intent(in) :: prc_snow     !convective snowfall [mm/s]
   real(r8), intent(in) :: prl_rain     !large-scale rainfall [mm/s]
   real(r8), intent(in) :: prl_snow     !large-scale snowfall [mm/s]
   real(r8), intent(in) :: qflx_irrig_sprinkler ! irrigation and sprinkler water flux [mm/s]
   real(r8), intent(in) :: bifall       !bulk density of newly fallen dry snow [kg/m3]
   real(r8), intent(in) :: sigf         !fraction of veg cover, excluding snow-covered veg [-]
   real(r8), intent(in) :: lai          !leaf area index [-]
   real(r8), intent(in) :: sai          !stem area index [-]
   real(r8), intent(in) :: tair         !air temperature [K]
   real(r8), intent(in) :: tleaf        !sunlit canopy leaf temperature [K]

   real(r8), intent(inout) :: ldew      !depth of water on foliage [mm]
   real(r8), intent(inout) :: ldew_rain !depth of water on foliage [mm]
   real(r8), intent(inout) :: ldew_snow !depth of water on foliage [mm]
   real(r8), intent(in)    :: z0m       !roughness length
   real(r8), intent(in)    :: hu        !forcing height of U

   real(r8), intent(out) :: pg_rain     !rainfall onto ground including canopy runoff [kg/(m2 s)]
   real(r8), intent(out) :: pg_snow     !snowfall onto ground including canopy runoff [kg/(m2 s)]
   real(r8), intent(out) :: qintr       !interception [kg/(m2 s)]
   real(r8), intent(out) :: qintr_rain  !rainfall interception (mm h2o/s)
   real(r8), intent(out) :: qintr_snow  !snowfall interception (mm h2o/s)
   real(r8), intent(out), optional :: gross_intr_rain !gross rain entering canopy pool [mm/s]
   real(r8), intent(out), optional :: gross_intr_snow !gross snow entering canopy pool [mm/s]
   real(r8), intent(out), optional :: xsc_rain_out    !old canopy rain release [mm/s]
   real(r8), intent(out), optional :: xsc_snow_out    !old canopy snow release [mm/s]
   real(r8), intent(out), optional :: ldew_smelt_out  !canopy snow->rain transfer [mm]
   real(r8), intent(out), optional :: ldew_frzc_out   !canopy rain->snow transfer [mm]
   real(r8), intent(out), optional :: canopy_phase_heat_out !canopy fusion heat flux [W/m2]

!-----------------------------------------------------------------------

      IF (lai+sai > 1e-6) THEN
         lsai   = lai + sai
         vegt   = lsai
         satcap = dewmx*vegt
         satcap_rain = satcap
         satcap_snow = 6.6*(0.27+46./bifall)*vegt  ! Niu et al., 2004
         satcap_snow = 48.*satcap                  ! Simple one without snow density input

         p0  = (prc_rain + prc_snow + prl_rain + prl_snow + qflx_irrig_sprinkler)*deltim
         ppc = (prc_rain + prc_snow)*deltim
         ppl = (prl_rain + prl_snow + qflx_irrig_sprinkler)*deltim

         w = ldew+p0
         IF (tleaf > tfrz) THEN
            xsc_rain = max(0., ldew-satcap)
            xsc_snow = 0.
         ELSE
            xsc_rain = 0.
            xsc_snow = max(0., ldew-satcap)
         ENDIF

         ldew = ldew - (xsc_rain + xsc_snow)

         !TODO-done: account for vegetation snow
         IF ( DEF_VEG_SNOW ) THEN
            xsc_rain  = max(0., ldew_rain-satcap_rain)
            xsc_snow  = max(0., ldew_snow-satcap_snow)
            ldew_rain = ldew_rain - xsc_rain
            ldew_snow = ldew_snow - xsc_snow
            ldew      = ldew_rain + ldew_snow
         ENDIF

         ap = pcoefs(2,1)
         cp = pcoefs(2,2)

         IF (p0 > 1.e-8) THEN
            ap = ppc/p0 * pcoefs(1,1) + ppl/p0 * pcoefs(2,1)
            cp = ppc/p0 * pcoefs(1,2) + ppl/p0 * pcoefs(2,2)

            !----------------------------------------------------------------------
            !      proportional saturated area (xs) and leaf drainage(tex)
            !-----------------------------------------------------------------------
            chiv = chil
            IF ( abs(chiv) .le. 0.01 ) chiv = 0.01
            aa1 = 0.5 - 0.633 * chiv - 0.33 * chiv * chiv
            bb1 = 0.877 * ( 1. - 2. * aa1 )
            exrain = aa1 + bb1

            ! coefficient of interception
            ! set fraction of potential interception to max 0.25 (Lawrence et al. 2007)
            ! assume alpha_rain = alpha_snow
            alpha_rain = 0.25
            fpi = alpha_rain * ( 1.-exp(-exrain*lsai) )
            tti_rain = (prc_rain+prl_rain+qflx_irrig_sprinkler)*deltim * ( 1.-fpi )
            tti_snow = (prc_snow+prl_snow)*deltim * ( 1.-fpi )

            xs = 1.
            IF (p0*fpi>1.e-9) THEN
               arg = (satcap-ldew)/(p0*fpi*ap) - cp/ap
               IF (arg>1.e-9) THEN
                  xs = -1./bp * log( arg )
                  xs = min( xs, 1. )
                  xs = max( xs, 0. )
               ENDIF
            ENDIF

            ! assume no fall down of the intercepted snowfall in a time step
            ! drainage
            tex_rain = (prc_rain+prl_rain+qflx_irrig_sprinkler)*deltim * fpi * (ap/bp*(1.-exp(-bp*xs))+cp*xs) &
                     - max(0., (satcap-ldew)) * xs
            tex_rain = max( tex_rain, 0. )
            ! Ensure physical constraint: tex_rain + tti_rain <= total rain input
            tex_rain = min( tex_rain, (prc_rain+prl_rain+qflx_irrig_sprinkler)*deltim - tti_rain )
            tex_snow = 0.

            ! 04/11/2024, yuan:
            !TODO-done: account for snow on vegetation,
            IF ( DEF_VEG_SNOW ) THEN

               ! re-calculate leaf rain drainage using ldew_rain

               xs = 1.
               IF (p0*fpi>1.e-9) THEN
                  arg = (satcap_rain-ldew_rain)/(p0*fpi*ap) - cp/ap
                  IF (arg>1.e-9) THEN
                     xs = -1./bp * log( arg )
                     xs = min( xs, 1. )
                     xs = max( xs, 0. )
                  ENDIF
               ENDIF

               tex_rain = (prc_rain+prl_rain+qflx_irrig_sprinkler)*deltim * fpi * (ap/bp*(1.-exp(-bp*xs))+cp*xs) &
                        - max(0., (satcap_rain-ldew_rain)) * xs
               tex_rain = max( tex_rain, 0. )
               ! Ensure physical constraint: tex_rain + tti_rain <= total rain input
               tex_rain = min( tex_rain, (prc_rain+prl_rain+qflx_irrig_sprinkler)*deltim - tti_rain )

               ! re-calculate the snow loading rate

               fvegc = 1. - exp(-0.52*lsai)
               FP    = (ppc + ppl) / (10.*ppc + ppl)
               qintr_snow = fvegc * (prc_snow+prl_snow) * FP
               qintr_snow = min (qintr_snow, (satcap_snow-ldew_snow)/deltim * (1.-exp(-(prc_snow+prl_snow)*deltim/satcap_snow)) )
               qintr_snow = max (qintr_snow, 0.)

               ! snow unloading rate

               FT = max(0.0, (tleaf - tfrz) / 1.87e5)
               FV = sqrt(forc_us*forc_us + forc_vs*forc_vs) / 1.56e5
               tex_snow = max(0., ldew_snow/deltim) * (FV+FT)
               tti_snow = (1.0-fvegc)*(prc_snow+prl_snow) + (fvegc*(prc_snow+prl_snow) - qintr_snow)

               ! rate -> mass

               tti_snow = tti_snow * deltim
               tex_snow = tex_snow * deltim
            ENDIF

#if (defined CoLMDEBUG)
            IF (tex_rain+tex_snow+tti_rain+tti_snow-p0 > 1.e-10 .and. .not.DEF_VEG_SNOW) THEN
               write(6,*) 'tex_ + tti_ > p0 in interception code : ',ldew,tex_rain,tex_snow,tti_rain,tti_snow,p0
            ENDIF
#endif

         ELSE
            ! all intercepted by canopy leaves for very small precipitation
            tti_rain = 0.
            tti_snow = 0.
            tex_rain = 0.
            tex_snow = 0.
         ENDIF

         !----------------------------------------------------------------------
         !   total throughfall (thru) and store augmentation
         !----------------------------------------------------------------------

         thru_rain = tti_rain + tex_rain
         thru_snow = tti_snow + tex_snow
         pinf = p0 - (thru_rain + thru_snow)
         ldew = ldew + pinf

         !TODO-done: IF DEF_VEG_SNOW, update ldew_rain, ldew_snow
         IF ( DEF_VEG_SNOW ) THEN
            ldew_rain = ldew_rain + (prc_rain+prl_rain+qflx_irrig_sprinkler)*deltim - thru_rain
            ldew_snow = ldew_snow + (prc_snow+prl_snow)*deltim - thru_snow
            ldew = ldew_rain + ldew_snow
         ENDIF

         pg_rain = (xsc_rain + thru_rain) / deltim
         pg_snow = (xsc_snow + thru_snow) / deltim
         qintr   = pinf / deltim

         qintr_rain = prc_rain + prl_rain + qflx_irrig_sprinkler - thru_rain / deltim
         qintr_snow = prc_snow + prl_snow - thru_snow / deltim

#if (defined CoLMDEBUG)
         w = w - ldew - (pg_rain+pg_snow)*deltim
         IF (abs(w) > INTERCEPTION_BALANCE_TOL) THEN
            write(6,*) 'something wrong in interception code: '
            write(6,*) w, ldew, (pg_rain+pg_snow)*deltim, satcap
            CALL abort
         ENDIF

         IF (DEF_VEG_SNOW .and. abs(ldew-ldew_rain-ldew_snow) > INTERCEPTION_BALANCE_TOL) THEN
            write(6,*) 'something wrong in interception code when DEF_VEG_SNOW: '
            write(6,*) ldew, ldew_rain, ldew_snow
            CALL abort
         ENDIF
#endif

      ELSE
         ! 07/15/2023, yuan: #bug found for ldew value reset.
         !NOTE: this bug should exist in other interception schemes @Zhongwang.
         IF (ldew > 0.) THEN
            IF (tleaf > tfrz) THEN
               pg_rain = prc_rain + prl_rain + qflx_irrig_sprinkler + ldew/deltim
               pg_snow = prc_snow + prl_snow
            ELSE
               pg_rain = prc_rain + prl_rain + qflx_irrig_sprinkler
               pg_snow = prc_snow + prl_snow + ldew/deltim
            ENDIF
         ELSE
            pg_rain = prc_rain + prl_rain + qflx_irrig_sprinkler
            pg_snow = prc_snow + prl_snow
         ENDIF

         ldew       = 0.
         ldew_rain  = 0.
         ldew_snow  = 0.
         qintr      = 0.
         qintr_rain = 0.
         qintr_snow = 0.

      ENDIF

      ! Compatibility outputs for the TRACER-aware dispatcher.  Keep the
      ! CoLM2014 interception physics above identical to upstream; this
      ! bookkeeping only exposes neutral/default tracer diagnostics.
      IF (present(gross_intr_rain))       gross_intr_rain       = max(0._r8, qintr_rain)
      IF (present(gross_intr_snow))       gross_intr_snow       = max(0._r8, qintr_snow)
      IF (present(xsc_rain_out))          xsc_rain_out          = 0._r8
      IF (present(xsc_snow_out))          xsc_snow_out          = 0._r8
      IF (present(ldew_smelt_out))        ldew_smelt_out        = 0._r8
      IF (present(ldew_frzc_out))         ldew_frzc_out         = 0._r8
      IF (present(canopy_phase_heat_out)) canopy_phase_heat_out = 0._r8

   END SUBROUTINE LEAF_interception_CoLM2014

   SUBROUTINE LEAF_interception_CoLM202x (deltim,dewmx,forc_us,forc_vs,chil,sigf,lai,sai,tair,tleaf,&
                                          prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler,&
                                          ldew,ldew_rain,ldew_snow,z0m,hu,pg_rain,pg_snow,&
                                          qintr,qintr_rain,qintr_snow,&
                                          gross_intr_rain,gross_intr_snow,&
                                          xsc_rain_out,xsc_snow_out,&
                                          ldew_smelt_out,ldew_frzc_out,&
                                          canopy_phase_heat_out)
!DESCRIPTION
!===========
   ! Calculation of interception and drainage of precipitation (under development)
   ! the scheme developed by Zhongwang wei @ SYSU (not finished yet)

!Original Author:
!-------------------
   !---Zhongwang Wei @ SYSU

!References:
!-------------------
   !---Zhong, F., Jiang, S., van Dijk, A.I., Ren, L., Schellekens, J. and Miralles, D.G., 2022.
   !   Revisiting large-scale interception patterns constrained by a synthesis of global experimental
   !   data. Hydrology and Earth System Sciences, 26(21), pp.5647-5667.
   !---

!ANCILLARY FUNCTIONS AND SUBROUTINES
!-------------------

!REVISION HISTORY
!----------------
   !---2023.04.30  Zhongwang Wei @ SYSU : Snow and rain interception
!=======================================================================

   IMPLICIT NONE

   real(r8), intent(in) :: deltim       !seconds in a time step [second]
   real(r8), intent(in) :: dewmx        !maximum dew [mm]
   real(r8), intent(in) :: forc_us      !wind speed
   real(r8), intent(in) :: forc_vs      !wind speed
   real(r8), intent(in) :: chil         !leaf angle distribution factor
   real(r8), intent(in) :: prc_rain     !convective rainfall [mm/s]
   real(r8), intent(in) :: prc_snow     !convective snowfall [mm/s]
   real(r8), intent(in) :: prl_rain     !large-scale rainfall [mm/s]
   real(r8), intent(in) :: prl_snow     !large-scale snowfall [mm/s]
   real(r8), intent(in) :: qflx_irrig_sprinkler ! irrigation and sprinkler water flux [mm/s]
   real(r8), intent(in) :: sigf         !fraction of veg cover, excluding snow-covered veg [-]
   real(r8), intent(in) :: lai          !leaf area index [-]
   real(r8), intent(in) :: sai          !stem area index [-]
   real(r8), intent(in) :: tair         !air temperature [K]
   real(r8), intent(in) :: tleaf        !sunlit canopy leaf temperature [K]

   real(r8), intent(inout) :: ldew      !depth of water on foliage [mm]
   real(r8), intent(inout) :: ldew_rain !depth of water on foliage [mm]
   real(r8), intent(inout) :: ldew_snow !depth of water on foliage [mm]
   real(r8), intent(in)    :: z0m       !roughness length
   real(r8), intent(in)    :: hu        !forcing height of U

   real(r8), intent(out) :: pg_rain     !rainfall onto ground including canopy runoff [kg/(m2 s)]
   real(r8), intent(out) :: pg_snow     !snowfall onto ground including canopy runoff [kg/(m2 s)]
   real(r8), intent(out) :: qintr       !interception [kg/(m2 s)]
   real(r8), intent(out) :: qintr_rain  !rainfall interception (mm h2o/s) [NET storage-related flux; can be <0 when canopy drains faster than new rain intercepts. Use gross_intr_rain for gross interception]
   real(r8), intent(out) :: qintr_snow  !snowfall interception (mm h2o/s) [NET storage-related flux; can be <0 (e.g. snow unloading/blowing with no new snow). Use gross_intr_snow for gross interception]
   real(r8), intent(out) :: gross_intr_rain !gross rain entering canopy mixed pool (mm h2o/s, >=0)
   real(r8), intent(out) :: gross_intr_snow !gross snow entering canopy mixed pool (mm h2o/s, >=0)
   real(r8), intent(out) :: xsc_rain_out    !pre-mix rain release rate from old canopy pool (mm h2o/s, >=0)
   real(r8), intent(out) :: xsc_snow_out    !pre-mix snow release rate from old canopy pool (mm h2o/s, >=0)
   ! Phase-change tracer transfer (grid-scale mm, >=0). CoLM202x does not
   ! perform canopy rain<->snow phase change in this routine.
   real(r8), intent(out) :: ldew_smelt_out
   real(r8), intent(out) :: ldew_frzc_out
   ! Canopy phase-change fusion heat flux [W/m^2]. Zero for CoLM202x.
   real(r8), intent(out) :: canopy_phase_heat_out

   ! Local clamps for precipitation inputs [mm/s]. See CoLM2014 block for rationale.
   real(r8) :: rain_clamp, snow_clamp

      ldew_smelt_out        = 0._r8
      ldew_frzc_out         = 0._r8
      canopy_phase_heat_out = 0._r8

      IF (lai+sai > 1e-6) THEN
         lsai   = lai + sai
         vegt   = lsai
         satcap = dewmx*vegt

         ! Clamp precipitation inputs once at scheme entry and use the
         ! clamped rates everywhere downstream.
         rain_clamp = MAX(0.0_r8, prc_rain + prl_rain + qflx_irrig_sprinkler)
         snow_clamp = MAX(0.0_r8, prc_snow + prl_snow)
         p0  = (rain_clamp + snow_clamp) * deltim
         ppc = MAX(0.0_r8, prc_rain + prc_snow) * deltim
         ppl = MAX(0.0_r8, p0 - ppc)

         ! C2 + back-compat: seed components from ldew if the
         ! restart/init state only populated ldew (CoLM202x before C2 did
         ! not maintain components). Otherwise trust the components and
         ! resync ldew to their sum.
         IF (ldew > 1.e-12_r8 .and. (ldew_rain + ldew_snow) < 1.e-12_r8) THEN
            IF (tleaf > tfrz) THEN
               ldew_rain = ldew
               ldew_snow = 0._r8
            ELSE
               ldew_rain = 0._r8
               ldew_snow = ldew
            ENDIF
         ELSE
            ldew = ldew_rain + ldew_snow
         ENDIF

         w = ldew+p0

         ! CoLM202x uses a SINGLE combined bucket (satcap = dewmx*vegt).
         ! The previous per-component comparison allowed (ldew_rain,
         ! ldew_snow) to each sit at satcap, doubling the effective total
         ! capacity. Compute combined excess and split it by the current
         ! phase ratio to preserve single-bucket semantics while keeping
         ! rain/snow components phase-consistent.
         IF (ldew > satcap .and. ldew > 1.e-12_r8) THEN
            xsc_rain = (ldew - satcap) * ldew_rain / ldew
            xsc_snow = (ldew - satcap) * ldew_snow / ldew
         ELSE
            xsc_rain = 0._r8
            xsc_snow = 0._r8
         ENDIF
         ldew_rain = ldew_rain - xsc_rain
         ldew_snow = ldew_snow - xsc_snow
         ldew = ldew_rain + ldew_snow

         ap = pcoefs(2,1)
         cp = pcoefs(2,2)

         IF (p0 > 1.e-8) THEN
            ap = ppc/p0 * pcoefs(1,1) + ppl/p0 * pcoefs(2,1)
            cp = ppc/p0 * pcoefs(1,2) + ppl/p0 * pcoefs(2,2)
            !----------------------------------------------------------------------
            !      proportional saturated area (xs) and leaf drainage(tex)
            !-----------------------------------------------------------------------
            chiv = chil
            IF ( abs(chiv) .le. 0.01 ) chiv = 0.01
            aa1 = 0.5 - 0.633 * chiv - 0.33 * chiv * chiv
            bb1 = 0.877 * ( 1. - 2. * aa1 )
            exrain = aa1 + bb1

            ! coefficient of interception
            ! set fraction of potential interception to max 0.25 (Lawrence et al. 2007)
            alpha_rain = 0.25
            fpi = alpha_rain * ( 1.-exp(-exrain*lsai) )
            tti_rain = rain_clamp*deltim * ( 1.-fpi )
            tti_snow = snow_clamp*deltim * ( 1.-fpi )

            xs = 1.
            IF (p0*fpi>1.e-9) THEN
               arg = (satcap-ldew)/(p0*fpi*ap) - cp/ap
               IF (arg>1.e-9) THEN
                  xs = -1./bp * log( arg )
                  xs = min( xs, 1. )
                  xs = max( xs, 0. )
               ENDIF
            ENDIF

            ! assume no fall down of the intercepted snowfall in a time step drainage
            tex_rain = rain_clamp*deltim * fpi * (ap/bp*(1.-exp(-bp*xs))+cp*xs) &
                     - max(0., (satcap-ldew)) * xs
            tex_rain = max( tex_rain, 0. )
            ! Ensure physical constraint: tex_rain + tti_rain <= total rain input
            tex_rain = min( tex_rain, rain_clamp*deltim - tti_rain )
            tex_snow = 0.

#if (defined CoLMDEBUG)
            IF (tex_rain+tex_snow+tti_rain+tti_snow-p0 > 1.e-10) THEN
               write(6,*) 'tex_ + tti_ > p0 in interception code : '
            ENDIF
#endif

         ELSE
            ! all intercepted by canopy leves for very small precipitation
            tti_rain = 0.
            tti_snow = 0.
            tex_rain = 0.
            tex_snow = 0.
         ENDIF

         !----------------------------------------------------------------------
         !   total throughfall (thru) and store augmentation
         !----------------------------------------------------------------------

         thru_rain = tti_rain + tex_rain
         thru_snow = tti_snow + tex_snow
         pinf = p0 - (thru_rain + thru_snow)
         ldew = ldew + pinf

         ! Maintain rain/snow components (was missing — only `ldew` was
         ! updated, leaving components stale). Because CoLM202x does not
         ! separate snow drainage (tex_snow stays 0), all rain excess goes
         ! into the rain component; snow interception is purely absorbed
         ! into ldew_snow with no drainage out.
         ldew_rain = ldew_rain + rain_clamp*deltim - thru_rain
         ldew_snow = ldew_snow + snow_clamp*deltim - thru_snow
         ldew_rain = max(0._r8, ldew_rain)
         ldew_snow = max(0._r8, ldew_snow)
         ldew = ldew_rain + ldew_snow

         pg_rain = (xsc_rain + thru_rain) / deltim
         pg_snow = (xsc_snow + thru_snow) / deltim
         qintr   = pinf / deltim

         qintr_rain = rain_clamp - (thru_rain / deltim)
         qintr_snow = snow_clamp - (thru_snow / deltim)

         ! Gross interception rate (see CoLM2014 block for rationale).
         gross_intr_rain = max(0._r8, rain_clamp &
                                       - tti_rain / deltim)
         gross_intr_snow = max(0._r8, snow_clamp - tti_snow / deltim)

         ! Pre-mix old-pool release rate (see CoLM2014 block for rationale).
         xsc_rain_out = xsc_rain / deltim
         xsc_snow_out = xsc_snow / deltim

#if (defined CoLMDEBUG)
         w = w - ldew - (pg_rain+pg_snow)*deltim
         IF (abs(w) > INTERCEPTION_BALANCE_TOL) THEN
            write(6,*) 'something wrong in interception code : '
            write(6,*) w, ldew, (pg_rain+pg_snow)*deltim, satcap
            CALL abort
         ENDIF

         CALL check_interception_balance('CoLM202x', &
              ldew, ldew_rain, ldew_snow, pg_rain, pg_snow, &
              qintr, qintr_rain, qintr_snow)
#endif

      ELSE
         ! Release by phase instead of tleaf-based mixing. CoLM202x now
         ! tracks ldew_rain and ldew_snow separately, so phase conservation
         ! is straightforward here.
         ! Clamp raw precipitation to prevent negative noise from propagating
         ! to pg_* in the no-vegetation branch, matching the in-branch clamp
         ! and JULES's no-veg treatment.
         ! Preserve pre-existing canopy water signature for tracer attribution.
         ! The ldew_rain / ldew_snow released here is OLD canopy water
         ! (R_canopy_pre), not fresh throughfall (R_input). Route it through
         ! xsc_*_out BEFORE ldew is reset so tracer_precip classifies it
         ! correctly (otherwise it would be lumped into throughfall and diluted
         ! by fresh precip signature).
         CALL release_no_vegetation_canopy_storage(deltim,tleaf, &
              prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler, &
              ldew,ldew_rain,ldew_snow,pg_rain,pg_snow, &
              qintr,qintr_rain,qintr_snow,gross_intr_rain,gross_intr_snow, &
              xsc_rain_out,xsc_snow_out)
      ENDIF
   END SUBROUTINE LEAF_interception_CoLM202x

   SUBROUTINE LEAF_interception_CLM4 (deltim,dewmx,forc_us,forc_vs,chil,sigf,lai,sai,tair,tleaf,&
                                       prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler,&
                                       ldew,ldew_rain,ldew_snow,z0m,hu,pg_rain,&
                                       pg_snow,qintr,qintr_rain,qintr_snow,&
                                       gross_intr_rain,gross_intr_snow,&
                                       xsc_rain_out,xsc_snow_out,&
                                       ldew_smelt_out,ldew_frzc_out,&
                                       canopy_phase_heat_out)
!DESCRIPTION
!===========
   ! Canopy interception following CLM4.5 official implementation
   ! - Interception efficiency: fpi = 0.25*(1-exp(-0.5*LSAI))
   ! - Drainage method: Simple bucket overflow (when storage exceeds capacity)
   ! - Verified against CLM4.5 source code: CTSM-clm4_5_18_r272/src_clm40/biogeophys/Hydrology1Mod.F90
   !
   ! Key features:
   ! - No pre-drainage step (unlike some earlier CoLM versions)
   ! - No spatial heterogeneity consideration (uniform canopy capacity)
   ! - Immediate overflow drainage when capacity is exceeded

!Original Author:
!-------------------
   !Lawrence, D.M.

!References:
!-------------------
   !---Lawrence, D.M., Thornton, P.E., Oleson, K.W. and Bonan, G.B., 2007.
   !   The partitioning of evapotranspiration into transpiration, soil evaporation,
   !   and canopy evaporation in a GCM: Impacts on land-atmosphere interaction. Journal of Hydrometeorology, 8(4), pp.862-880.
   !---Oleson, K.W., Lawrence, D.M., Bonan, G.B., Drewniak, B., Huang, M., Koven, C.D., Levis, S., Li, F., Riley, W.J., Subin, Z.M. and Swenson, S.C., 2013.
   !   Technical description of version 4.5 of the Community Land Model (CLM). NCAR Technical Note NCAR/TN-503+ STR.

!ANCILLARY FUNCTIONS AND SUBROUTINES
!-------------------

!REVISION HISTORY
!----------------
   ! 2023.02.21  Zhongwang Wei @ SYSU : Snow and rain interception
   ! 2021.12.08  Zhongwang Wei @ SYSU
   ! 2014.04     Yongjiu Dai
   ! 2002.08.31  Yongjiu Dai
!=======================================================================

   IMPLICIT NONE

   real(r8), intent(in) :: deltim       !seconds in a time step [second]
   real(r8), intent(in) :: dewmx        !maximum dew [mm]
   real(r8), intent(in) :: forc_us      !wind speed
   real(r8), intent(in) :: forc_vs      !wind speed
   real(r8), intent(in) :: chil         !leaf angle distribution factor
   real(r8), intent(in) :: prc_rain     !convective rainfall [mm/s]
   real(r8), intent(in) :: prc_snow     !convective snowfall [mm/s]
   real(r8), intent(in) :: prl_rain     !large-scale rainfall [mm/s]
   real(r8), intent(in) :: prl_snow     !large-scale snowfall [mm/s]
   real(r8), intent(in) :: qflx_irrig_sprinkler !irrigation and sprinkler water flux [mm/s]
   real(r8), intent(in) :: sigf         !fraction of veg cover, excluding snow-covered veg [-]
   real(r8), intent(in) :: lai          !leaf area index [-]
   real(r8), intent(in) :: sai          !stem area index [-]
   real(r8), intent(in) :: tair         !air temperature [K]
   real(r8), intent(in) :: tleaf        !sunlit canopy leaf temperature [K]

   real(r8), intent(inout) :: ldew      !depth of water on foliage [mm]
   real(r8), intent(inout) :: ldew_rain !depth of water on foliage [mm]
   real(r8), intent(inout) :: ldew_snow !depth of water on foliage [mm]
   real(r8), intent(in)    :: z0m       !roughness length
   real(r8), intent(in)    :: hu        !forcing height of U

   real(r8), intent(out) :: pg_rain     !rainfall onto ground including canopy runoff [kg/(m2 s)]
   real(r8), intent(out) :: pg_snow     !snowfall onto ground including canopy runoff [kg/(m2 s)]
   real(r8), intent(out) :: qintr       !interception [kg/(m2 s)]
   real(r8), intent(out) :: qintr_rain  !rainfall interception (mm h2o/s) [NET storage-related flux; can be <0 when canopy drains faster than new rain intercepts. Use gross_intr_rain for gross interception]
   real(r8), intent(out) :: qintr_snow  !snowfall interception (mm h2o/s) [NET storage-related flux; can be <0 (e.g. snow unloading/blowing with no new snow). Use gross_intr_snow for gross interception]
   real(r8), intent(out) :: gross_intr_rain !gross rain entering canopy mixed pool (mm h2o/s, >=0)
   real(r8), intent(out) :: gross_intr_snow !gross snow entering canopy mixed pool (mm h2o/s, >=0)
   real(r8), intent(out) :: xsc_rain_out    !pre-mix rain release rate from old canopy pool (mm h2o/s, >=0)
   real(r8), intent(out) :: xsc_snow_out    !pre-mix snow release rate from old canopy pool (mm h2o/s, >=0)
   ! Phase-change tracer transfer (grid-scale mm, >=0). CLM4 does not
   ! perform canopy rain<->snow phase change in this routine.
   real(r8), intent(out) :: ldew_smelt_out
   real(r8), intent(out) :: ldew_frzc_out
   ! Canopy phase-change fusion heat flux [W/m^2]. Zero for CLM4.
   real(r8), intent(out) :: canopy_phase_heat_out

   ! Local clamps for precipitation inputs [mm/s].
   real(r8) :: rain_clamp, snow_clamp
   ! Helpers for combined rain+snow overflow.
   real(r8) :: post_ldew_int, overflow, frac_rain_c4, frac_snow_c4

      ldew_smelt_out        = 0._r8
      ldew_frzc_out         = 0._r8
      canopy_phase_heat_out = 0._r8

      IF (lai+sai > 1e-6) THEN
         lsai   = lai + sai
         vegt   = lsai
         ! Guard restart/float-noise negative canopy state before
         ! (satcap-ldew) comparisons.
         ldew_rain = max(0._r8, ldew_rain)
         ldew_snow = max(0._r8, ldew_snow)
         satcap = dewmx*vegt

         ! Clamp precipitation inputs once at scheme entry.
         rain_clamp = MAX(0.0_r8, prc_rain + prl_rain + qflx_irrig_sprinkler)
         snow_clamp = MAX(0.0_r8, prc_snow + prl_snow)
         p0  = (rain_clamp + snow_clamp) * deltim
         ppc = MAX(0.0_r8, prc_rain + prc_snow) * deltim
         ppl = MAX(0.0_r8, p0 - ppc)

         ! C1 + back-compat: seed components from ldew if the
         ! restart/init state only populated ldew (CLM4 before C1 did not
         ! maintain components). Otherwise trust the components and resync
         ! ldew to their sum.
         IF (ldew > 1.e-12_r8 .and. (ldew_rain + ldew_snow) < 1.e-12_r8) THEN
            IF (tleaf > tfrz) THEN
               ldew_rain = ldew
               ldew_snow = 0._r8
            ELSE
               ldew_rain = 0._r8
               ldew_snow = ldew
            ENDIF
         ELSE
            ldew = ldew_rain + ldew_snow
         ENDIF

         w = ldew+p0

         ! CLM4 uses a SINGLE combined bucket (satcap = dewmx*vegt). The
         ! previous per-component comparison allowed (ldew_rain, ldew_snow)
         ! to each sit at satcap, doubling the effective total capacity.
         ! Compute combined excess and split it by the current phase ratio
         ! to preserve single-bucket semantics while keeping rain/snow
         ! components phase-consistent.
         IF (ldew > satcap .and. ldew > 1.e-12_r8) THEN
            xsc_rain = (ldew - satcap) * ldew_rain / ldew
            xsc_snow = (ldew - satcap) * ldew_snow / ldew
         ELSE
            xsc_rain = 0._r8
            xsc_snow = 0._r8
         ENDIF
         ldew_rain = ldew_rain - xsc_rain
         ldew_snow = ldew_snow - xsc_snow
         ldew = ldew_rain + ldew_snow

         IF (p0 > 1.e-8) THEN
            exrain =0.5
            ! coefficient of interception
            ! set fraction of potential interception to max 0.25 (Lawrence et al. 2007)
            alpha_rain = 0.25
            fpi = alpha_rain * ( 1.-exp(-exrain*lsai) )
            tti_rain = rain_clamp*deltim * ( 1.-fpi )
            tti_snow = snow_clamp*deltim * ( 1.-fpi )

            ! Align with original CLM4 Hydrology1Mod.F90 qflx_candrip
            ! logic. CLM4 uses a single
            ! shared canopy bucket (satcap = dewmx*vegt) for both rain
            ! and snow. When post-interception storage exceeds satcap,
            ! the excess drips AT THIS STEP, split between rain and
            ! snow by the current step's precipitation fraction (CLM4
            ! fracrain/fracsnow).
            !
            ! Previously:
            !   tex_rain = rain*fpi*dt + ldew - satcap      (only rain)
            !   tex_snow = 0                                  (ALWAYS)
            ! which allowed `ldew + snow*fpi*dt > satcap` to persist
            ! through the step. Snow would only drain on the NEXT step's
            ! entry xsc path — meaning current-step pg_snow was under-
            ! reported and canopy fwet_snow / Niu pull / albedo / qsubl
            ! all used a transient over-full snow bucket.
            !
            ! New: compute combined overflow and split per phase.
            post_ldew_int = ldew + (rain_clamp + snow_clamp) * deltim * fpi
            overflow = max(0._r8, post_ldew_int - satcap)
            IF (overflow > 0._r8 .and. (rain_clamp + snow_clamp) > 1.e-12_r8) THEN
               frac_rain_c4 = rain_clamp / (rain_clamp + snow_clamp)
               frac_snow_c4 = 1._r8 - frac_rain_c4
            ELSE
               frac_rain_c4 = 0._r8
               frac_snow_c4 = 0._r8
            ENDIF
            tex_rain = overflow * frac_rain_c4
            tex_snow = overflow * frac_snow_c4

            ! Physical constraint: tex + tti <= total precip input (per phase).
            tex_rain = min(tex_rain, max(0._r8, rain_clamp*deltim - tti_rain))
            tex_snow = min(tex_snow, max(0._r8, snow_clamp*deltim - tti_snow))

#if (defined CoLMDEBUG)
            IF (tex_rain+tex_snow+tti_rain+tti_snow-p0 > 1.e-10) THEN
               write(6,*) 'tex_ + tti_ > p0 in interception code : '
            ENDIF
#endif


         ELSE
            ! all intercepted by canopy leaves for very small precipitation
            tti_rain = 0.
            tti_snow = 0.
            tex_rain = 0.
            tex_snow = 0.
         ENDIF

         !----------------------------------------------------------------------
         !   total throughfall (thru) and store augmentation
         !----------------------------------------------------------------------
         thru_rain = tti_rain + tex_rain
         thru_snow = tti_snow + tex_snow
         pinf = p0 - (thru_rain + thru_snow)
         ldew = ldew + pinf

         ! Maintain rain/snow components so ldew_rain + ldew_snow == ldew
         ! at exit. xsc_* was already subtracted from ldew_rain/ldew_snow
         ! before the main block, so here we only apply the net
         ! precipitation - throughfall balance per phase.
         ldew_rain = ldew_rain + rain_clamp*deltim - thru_rain
         ldew_snow = ldew_snow + snow_clamp*deltim - thru_snow
         ldew_rain = max(0._r8, ldew_rain)
         ldew_snow = max(0._r8, ldew_snow)
         ldew = ldew_rain + ldew_snow

         pg_rain = (xsc_rain + thru_rain) / deltim
         pg_snow = (xsc_snow + thru_snow) / deltim
         qintr   = pinf / deltim

         qintr_rain = rain_clamp - (thru_rain / deltim)
         qintr_snow = snow_clamp - (thru_snow / deltim)

         ! Gross interception rate (see CoLM2014 block for rationale).
         gross_intr_rain = max(0._r8, rain_clamp &
                                       - tti_rain / deltim)
         gross_intr_snow = max(0._r8, snow_clamp - tti_snow / deltim)

         ! Pre-mix old-pool release rate (see CoLM2014 block for rationale).
         xsc_rain_out = xsc_rain / deltim
         xsc_snow_out = xsc_snow / deltim

#if (defined CoLMDEBUG)
         w = w - ldew - (pg_rain+pg_snow)*deltim
         IF (abs(w) > INTERCEPTION_BALANCE_TOL) THEN
            write(6,*) 'something wrong in interception code : '
            write(6,*) w, ldew, (pg_rain+pg_snow)*deltim, satcap
            CALL abort
         ENDIF
#endif

      ELSE
         ! Release canopy water per phase. CLM4 now tracks
         ! ldew_rain/ldew_snow separately, so a phase-preserving release
         ! is correct regardless of tleaf.
         ! Clamp raw precipitation to prevent negative noise from propagating
         ! to pg_* in the no-vegetation branch, matching the in-branch clamp
         ! and JULES's no-veg treatment.
         ! Preserve pre-existing canopy water signature for tracer attribution.
         ! The ldew_rain / ldew_snow released here is OLD canopy water
         ! (R_canopy_pre), not fresh throughfall (R_input). Route it through
         ! xsc_*_out BEFORE ldew is reset so tracer_precip classifies it
         ! correctly (otherwise it would be lumped into throughfall and diluted
         ! by fresh precip signature).
         CALL release_no_vegetation_canopy_storage(deltim,tleaf, &
              prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler, &
              ldew,ldew_rain,ldew_snow,pg_rain,pg_snow, &
              qintr,qintr_rain,qintr_snow,gross_intr_rain,gross_intr_snow, &
              xsc_rain_out,xsc_snow_out)
      ENDIF

   END SUBROUTINE LEAF_interception_CLM4

   SUBROUTINE LEAF_interception_CLM5 (deltim,dewmx,forc_us,forc_vs,chil,sigf,lai,sai,tair,tleaf,&
                                    prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler,&
                                    ldew,ldew_rain,ldew_snow,z0m,hu,pg_rain,pg_snow,&
                                    qintr,qintr_rain,qintr_snow,&
                                    gross_intr_rain,gross_intr_snow,&
                                    xsc_rain_out,xsc_snow_out,&
                                    ldew_smelt_out,ldew_frzc_out,&
                                    canopy_phase_heat_out)

!DESCRIPTION
!===========
   ! Canopy interception following CLM5.0 official implementation
   ! - Separate treatment for rain and snow interception
   ! - Rain interception: fpi = tanh(LSAI) [or 0.25*(1-exp(-0.5*LSAI))]
   ! - Snow interception: fpi = 1-exp(-0.5*LSAI)
   ! - Liquid water capacity: 0.1*(LAI+SAI) mm
   ! - Snow capacity: 6.0*(LAI+SAI) mm
   ! - Simple bucket overflow drainage based on temperature
   ! - Snow unloading due to wind and temperature
   ! - Verified against CLM5 source: CanopyHydrologyMod.F90
   !
   ! Key features:
   ! - No pre-drainage step (fixed from earlier version)
   ! - Drainage based on storage, not interception (critical fix)
   ! - Temperature-dependent rain/snow drainage
   ! - Physics-based snow unloading

!Original Author:
!-------------------
   !Lawrence, D.M.

!References:
!-------------------
   !---Lawrence, D.M., Thornton, P.E., Oleson, K.W. and Bonan, G.B., 2007.
   !   The partitioning of evapotranspiration into transpiration, soil evaporation,
   !   and canopy evaporation in a GCM: Impacts on land-atmosphere interaction. Journal of Hydrometeorology, 8(4), pp.862-880.
   !---Lawrence, D.M., Fisher, R.A., Koven, C.D., Oleson, K.W., Swenson, S.C., Bonan, G., Collier, N., Ghimire, B.,
   !   van Kampenhout, L., Kennedy, D. and Kluzek, E., 2019. The Community Land Model version 5:
   !   Description of new features, benchmarking, and impact of forcing uncertainty.
   !   Journal of Advances in Modeling Earth Systems, 11(12), pp.4245-4287.
   !---Fan, Y., Meijide, A., Lawrence, D.M., Roupsard, O., Carlson, K.M., Chen, H.Y.,
   !   Röll, A., Niu, F. and Knohl, A., 2019. Reconciling canopy interception parameterization
   !   and rainfall forcing frequency in the Community Land Model for simulating evapotranspiration
   !   of rainforests and oil palm plantations in Indonesia. Journal of Advances in Modeling Earth Systems, 11(3), pp.732-751.


!ANCILLARY FUNCTIONS AND SUBROUTINES
!-------------------

!REVISION HISTORY
!----------------
   ! 2023.02.21  Zhongwang Wei @ SYSU
   ! 2021.12.08  Zhongwang Wei @ SYSU
!=======================================================================

   IMPLICIT NONE

   real(r8), intent(in) :: deltim       !seconds in a time step [second]
   real(r8), intent(in) :: dewmx        !maximum dew [mm]
   real(r8), intent(in) :: forc_us      !wind speed
   real(r8), intent(in) :: forc_vs      !wind speed
   real(r8), intent(in) :: chil         !leaf angle distribution factor
   real(r8), intent(in) :: prc_rain     !convective rainfall [mm/s]
   real(r8), intent(in) :: prc_snow     !convective snowfall [mm/s]
   real(r8), intent(in) :: prl_rain     !large-scale rainfall [mm/s]
   real(r8), intent(in) :: prl_snow     !large-scale snowfall [mm/s]
   real(r8), intent(in) :: qflx_irrig_sprinkler !irrigation and sprinkler water flux [mm/s]
   real(r8), intent(in) :: sigf         !fraction of veg cover, excluding snow-covered veg [-]
   real(r8), intent(in) :: lai          !leaf area index [-]
   real(r8), intent(in) :: sai          !stem area index [-]
   real(r8), intent(in) :: tair         !air temperature [K]
   real(r8), intent(in) :: tleaf        !sunlit canopy leaf temperature [K]

   real(r8), intent(inout) :: ldew      !depth of water on foliage [mm]
   real(r8), intent(inout) :: ldew_rain !depth of water on foliage [mm]
   real(r8), intent(inout) :: ldew_snow !depth of water on foliage [mm]
   real(r8), intent(in)    :: z0m       !roughness length
   real(r8), intent(in)    :: hu        !forcing height of U

   real(r8), intent(out) :: pg_rain     !rainfall onto ground including canopy runoff [kg/(m2 s)]
   real(r8), intent(out) :: pg_snow     !snowfall onto ground including canopy runoff [kg/(m2 s)]
   real(r8), intent(out) :: qintr       !interception [kg/(m2 s)]
   real(r8), intent(out) :: qintr_rain  !rainfall interception (mm h2o/s) [NET storage-related flux; can be <0 when canopy drains faster than new rain intercepts. Use gross_intr_rain for gross interception]
   real(r8), intent(out) :: qintr_snow  !snowfall interception (mm h2o/s) [NET storage-related flux; can be <0 (e.g. snow unloading/blowing with no new snow). Use gross_intr_snow for gross interception]
   real(r8), intent(out) :: gross_intr_rain !gross rain entering canopy mixed pool (mm h2o/s, >=0)
   real(r8), intent(out) :: gross_intr_snow !gross snow entering canopy mixed pool (mm h2o/s, >=0)
   real(r8), intent(out) :: xsc_rain_out    !pre-mix rain release rate from old canopy pool (mm h2o/s, >=0)
   real(r8), intent(out) :: xsc_snow_out    !pre-mix snow release rate from old canopy pool (mm h2o/s, >=0)
   ! Phase-change tracer transfer (grid-scale mm, >=0). CLM5 defers canopy
   ! rain<->snow phase change to LeafTemperature / THERMAL, so this routine
   ! always reports zero.
   real(r8), intent(out) :: ldew_smelt_out
   real(r8), intent(out) :: ldew_frzc_out
   ! Canopy phase-change fusion heat flux [W/m^2]. Zero for CLM5.
   real(r8), intent(out) :: canopy_phase_heat_out
   real(r8) :: xsnorun, xliqrun,qflx_prec_intr_rain,qflx_prec_intr_snow
   ! Local clamps for precipitation inputs [mm/s].
   real(r8) :: rain_clamp, snow_clamp

      ldew_smelt_out        = 0._r8
      ldew_frzc_out         = 0._r8
      canopy_phase_heat_out = 0._r8

      IF (lai+sai > 1e-6) THEN
         lsai   = lai + sai
         vegt   = lsai
         ! Guard restart/float-noise negative canopy state before
         ! (satcap-ldew) / fwet / xliqrun uses.
         ldew_rain = max(0._r8, ldew_rain)
         ldew_snow = max(0._r8, ldew_snow)
         ! Clamp precipitation inputs once at scheme entry.
         rain_clamp = MAX(0.0_r8, prc_rain + prl_rain + qflx_irrig_sprinkler)
         snow_clamp = MAX(0.0_r8, prc_snow + prl_snow)
         p0  = (rain_clamp + snow_clamp) * deltim

         ! Ensure ldew is consistent with components at entry
         ! CLM5 operates on ldew_rain/ldew_snow and sets ldew = ldew_rain + ldew_snow at exit
         ! At entry from initialization or restart, ldew may be inconsistent
         ldew = ldew_rain + ldew_snow

         w = ldew+p0  ! For mass balance check

         ! Canopy capacity - CLM5 official values
         ! Verified against CanopyHydrologyMod.F90 lines 320, 329-330
         satcap_rain = dewmx*vegt        ! liquid water capacity = 0.1*(LAI+SAI)
         satcap_snow = satcap_rain*60.0  ! snow capacity = 6.0*(LAI+SAI)

         ! Initialize drainage and unloading outside IF p0 block so they
         ! remain defined on dry steps.
         tex_rain = 0._r8
         tex_snow = 0._r8
         unl_snow = 0._r8

         IF(p0 > 1.e-8) THEN
            ! Interception efficiency - CLM5 formulas
            ! Rain: CLM5 line 323 (tanh option) or line 325 (exponential option)
            ! Snow: CLM5 line 332
            ! Note: CoLM uses tanh for rain; CLM5 default is exponential (CLM4.5)
            alpha_rain = 1.0
            alpha_snow = 1.0
            fpi_rain   = alpha_rain * tanh(lsai)
            fpi_snow   = alpha_snow * ( 1.-exp(-0.5*lsai) )

            ! Direct throughfall - CLM5 lines 334, 337, 341
            tti_rain   = rain_clamp*deltim * ( 1.-fpi_rain )
            tti_snow   = snow_clamp*deltim * ( 1.-fpi_snow )

            ! Intercepted precipitation - CLM5 line 345
            qflx_prec_intr_rain = rain_clamp*deltim * fpi_rain
            qflx_prec_intr_snow = snow_clamp*deltim * fpi_snow

            ! Water storage of intercepted precipitation - CLM5 lines 347-348
            ! Add interception to storage BEFORE calculating drainage
            ldew_rain = max(0., ldew_rain + qflx_prec_intr_rain)
            ldew_snow = max(0., ldew_snow + qflx_prec_intr_snow)

         ELSE
            ! No precipitation - no interception. Drainage and unloading
            ! are handled unconditionally below.
            tti_rain = 0._r8
            tti_snow = 0._r8
         ENDIF

         ! Bucket overflow drainage is independent of the current precipitation
         ! step. Previously this lived inside IF(p0 > 1.e-8), so a dry step
         ! that inherited an oversaturated canopy (e.g. LAI shrink after a
         ! prior wet step, or restart inconsistency) would retain ldew >
         ! satcap indefinitely. Lift the drain out so it fires every step.
         ! On wet steps the post-interception ldew is already capped by the
         ! earlier add + satcap comparison, so the unconditional drain is a
         ! no-op.
         ! Simple bucket overflow drainage - CLM5 lines 367-379.
         ! Check BOTH phases independently. Previous code tied drainage to
         ! tleaf > tfrz (matching CLM5 original), but that left the non-
         ! dominant phase above capacity indefinitely during phase transitions
         ! (e.g. tleaf crosses to > tfrz but ldew_snow still exceeds
         ! satcap_snow). Deliberate CoLM improvement over CLM5 original
         ! L366-378.
         xliqrun = max(0._r8, (ldew_rain - satcap_rain)/deltim)
         IF (xliqrun > 0._r8) THEN
            tex_rain  = tex_rain + xliqrun * deltim
            ldew_rain = satcap_rain
         ENDIF
         xsnorun = max(0._r8, (ldew_snow - satcap_snow)/deltim)
         IF (xsnorun > 0._r8) THEN
            tex_snow  = tex_snow + xsnorun * deltim
            ldew_snow = satcap_snow
         ENDIF

         ! Snow unloading aligned with CLM5 original
         ! (CanopyHydrologyMod.F90:425-435). Previously CoLM had:
         !   1) wind factor without the 0.5 coefficient → unloaded 2× too
         !      fast (Ec-low bias).
         !   2) temperature factor (tleaf - tfrz)/1.87e5 instead of
         !      (forc_t - 270.15)/1.87e5 → threshold off by 3K, weaker
         !      temp unloading (Ec-high bias).
         !   3) unloading nested inside IF(p0 > 1.e-8) → dry steps left
         !      canopy snow stuck (Ec-high bias, the dominant of the three).
         ! Now matches original CLM5 exactly.
         IF (ldew_snow > 1.e-8) THEN
            U10           = sqrt(forc_us*forc_us + forc_vs*forc_vs)
            unl_snow_temp = max(0._r8, ldew_snow * (tair - 270.15_r8) / 1.87e5_r8)
            unl_snow_wind = max(0._r8, 0.5_r8 * U10 * ldew_snow / 1.56e5_r8)
            unl_snow      = unl_snow_temp + unl_snow_wind
            unl_snow      = min(unl_snow, ldew_snow)
            ldew_snow     = ldew_snow - unl_snow
         ENDIF

         !----------------------------------------------------------------------
         !   Total water reaching ground and interception
         !----------------------------------------------------------------------
         thru_rain = tti_rain + tex_rain
         thru_snow = tti_snow + tex_snow + unl_snow
         ldew      = ldew_rain + ldew_snow

         pg_rain = thru_rain / deltim
         pg_snow = thru_snow / deltim
         qintr   = (p0 - thru_rain - thru_snow) / deltim
         qintr_rain = rain_clamp - (thru_rain / deltim)
         qintr_snow = snow_clamp - (thru_snow / deltim)

         ! Gross interception rate: CLM5 computes qflx_prec_intr_* explicitly
         ! as the mass entering the canopy pool before drainage. Use it
         ! directly when precipitation is present.
         IF (p0 > 1.e-8) THEN
            gross_intr_rain = qflx_prec_intr_rain / deltim
            gross_intr_snow = qflx_prec_intr_snow / deltim
         ELSE
            gross_intr_rain = 0._r8
            gross_intr_snow = 0._r8
         ENDIF

         ! CLM5 has no pre-mix release: new rain/snow is added into the
         ! canopy pool BEFORE any drainage (xliqrun/xsnorun) or unload,
         ! so the overflow always carries R_mixed rather than the old
         ! pre-mix canopy signature. xsc_rain_out = xsc_snow_out = 0.
         xsc_rain_out = 0._r8
         xsc_snow_out = 0._r8

#if (defined CoLMDEBUG)
         ! Mass balance check
         w = w - ldew - (pg_rain+pg_snow)*deltim
         IF (abs(w) > INTERCEPTION_BALANCE_TOL) THEN
            write(6,*) 'Mass balance error in CLM5 interception:'
            write(6,*) 'Error:', w, 'ldew:', ldew, 'outflow:', (pg_rain+pg_snow)*deltim
            write(6,*) 'satcap_rain:', satcap_rain, 'satcap_snow:', satcap_snow
            CALL abort
         ENDIF

         CALL check_interception_balance('CLM5', &
              ldew, ldew_rain, ldew_snow, pg_rain, pg_snow, &
              qintr, qintr_rain, qintr_snow)
#endif

      ELSE
         ! 07/15/2023, Hua Yuan: bug found for ldew value reset when vegetation disappears
         ! 2026-01-16 improvement: Maintain phase conservation for rain/snow separated schemes
         ! Yuan's original fix released water based on temperature, which violates phase conservation
         ! for schemes that separate rain and snow storage (ldew_rain vs ldew_snow)
         !
         ! Yuan's original code (2023-07-15):
         ! IF (ldew > 0.) THEN
         !    IF (tleaf > tfrz) THEN
         !       pg_rain = prc_rain + prl_rain + qflx_irrig_sprinkler + ldew/deltim
         !       pg_snow = prc_snow + prl_snow
         !    ELSE
         !       pg_rain = prc_rain + prl_rain + qflx_irrig_sprinkler
         !       pg_snow = prc_snow + prl_snow + ldew/deltim
         !    ENDIF
         ! ELSE
         !    pg_rain = prc_rain + prl_rain + qflx_irrig_sprinkler
         !    pg_snow = prc_snow + prl_snow
         ! ENDIF
         !
         ! Improved version: Release liquid and solid water separately to
         ! preserve phase states.
         ! Clamp raw precipitation to prevent negative noise from propagating
         ! to pg_* in the no-vegetation branch, matching the in-branch clamp
         ! and JULES's no-veg treatment.
         ! Preserve pre-existing canopy water signature for tracer attribution.
         ! The ldew_rain / ldew_snow released here is OLD canopy water
         ! (R_canopy_pre), not fresh throughfall (R_input). Route it through
         ! xsc_*_out BEFORE ldew is reset so tracer_precip classifies it
         ! correctly (otherwise it would be lumped into throughfall and diluted
         ! by fresh precip signature).
         CALL release_no_vegetation_canopy_storage(deltim,tleaf, &
              prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler, &
              ldew,ldew_rain,ldew_snow,pg_rain,pg_snow, &
              qintr,qintr_rain,qintr_snow,gross_intr_rain,gross_intr_snow, &
              xsc_rain_out,xsc_snow_out)
      ENDIF

   END SUBROUTINE LEAF_interception_CLM5

   SUBROUTINE LEAF_interception_NOAHMP(deltim,dewmx,forc_us,forc_vs,chil,sigf,lai,sai,tair,tleaf, &
                                       prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler,&
                                       ldew,ldew_rain,ldew_snow,z0m,hu,pg_rain,pg_snow,qintr,qintr_rain,qintr_snow,&
                                       gross_intr_rain,gross_intr_snow,&
                                       xsc_rain_out,xsc_snow_out,&
                                       ldew_smelt_out,ldew_frzc_out,&
                                       canopy_phase_heat_out)
!DESCRIPTION
!===========
   ! Interception and drainage of precipitation
   ! the treatment are modified from Noah-MP 5.0

!Original Author:
!-------------------
   !---Guo-Yue Niu

!References:
!-------------------
   !---Yang, M., Zuo, R., Li, X. and Wang, L., 2019. Improvement test for the canopy interception parameterization scheme
   !   in the community land model. Sola, 15, pp.166-171.
   !---Niu, G.Y., Yang, Z.L., Mitchell, K.E., Chen, F., Ek, M.B., Barlage, M., Kumar, A.,
   !   Manning, K., Niyogi, D., Rosero, E. and Tewari, M., 2011. The community Noah land
   !   surface model with multiparameterization options (Noah‐MP): 1. Model description and evaluation
   !   with local‐scale measurements. Journal of Geophysical Research: Atmospheres, 116(D12).
   !---He, C., Valayamkunnath, P., Barlage, M., Chen, F., Gochis, D., Cabell, R., Schneider, T.,
   !   Rasmussen, R., Niu, G.Y., Yang, Z.L. and Niyogi, D., 2023. Modernizing the open-source
   !   community Noah-MP land surface model (version 5.0) with enhanced modularity,
   !   interoperability, and applicability. EGUsphere, 2023, pp.1-31.

!ANCILLARY FUNCTIONS AND SUBROUTINES
!-------------------

!REVISION HISTORY
!----------------
   ! 2026.02.11  Zhongwang Wei @ SYSU - Added input clamping, comment fixes
   ! 2023.02.21  Zhongwang Wei @ SYSU
   ! 2021.12.08  Zhongwang Wei @ SYSU
!=======================================================================

   IMPLICIT NONE

   real(r8), intent(in)    :: deltim     !seconds in a time step [second]
   real(r8), intent(in)    :: dewmx      !maximum dew [mm]
   real(r8), intent(in)    :: forc_us    !wind speed
   real(r8), intent(in)    :: forc_vs    !wind speed
   real(r8), intent(in)    :: chil       !leaf angle distribution factor
   real(r8), intent(in)    :: prc_rain   !convective rainfall [mm/s]
   real(r8), intent(in)    :: prc_snow   !convective snowfall [mm/s]
   real(r8), intent(in)    :: prl_rain   !large-scale rainfall [mm/s]
   real(r8), intent(in)    :: prl_snow   !large-scale snowfall [mm/s]
   real(r8), intent(in)    :: qflx_irrig_sprinkler !irrigation and sprinkler water flux [mm/s]
   real(r8), intent(in)    :: sigf       !fraction of veg cover, excluding snow-covered veg [-]
   real(r8), intent(in)    :: lai        !leaf area index [-]
   real(r8), intent(in)    :: sai        !stem area index [-]
   real(r8), intent(in)    :: tair       !air temperature [K]
   real(r8), intent(inout) :: tleaf      !sunlit canopy leaf temperature [K]

   real(r8), intent(inout) :: ldew       !depth of water on foliage [mm]
   real(r8), intent(inout) :: ldew_rain  !depth of liquid on foliage [mm]
   real(r8), intent(inout) :: ldew_snow  !depth of solid (frozen) on foliage [mm]
   real(r8), intent(in)    :: z0m        !roughness length
   real(r8), intent(in)    :: hu         !forcing height of U

   real(r8), intent(out)   :: pg_rain    !rainfall onto ground including canopy runoff [kg/(m2 s)]
   real(r8), intent(out)   :: pg_snow    !snowfall onto ground including canopy runoff [kg/(m2 s)]
   real(r8), intent(out)   :: qintr      !interception [kg/(m2 s)]
   real(r8), intent(out)   :: qintr_rain !rainfall interception (mm h2o/s) [NET storage-related flux; can be <0 when canopy drains faster than new rain intercepts. Use gross_intr_rain for gross interception]
   real(r8), intent(out)   :: qintr_snow !snowfall interception (mm h2o/s) [NET storage-related flux; can be <0 (e.g. snow unloading/blowing with no new snow). Use gross_intr_snow for gross interception]
   real(r8), intent(out)   :: gross_intr_rain !gross rain entering canopy mixed pool (mm h2o/s, >=0)
   real(r8), intent(out)   :: gross_intr_snow !gross snow entering canopy mixed pool (mm h2o/s, >=0)
   real(r8), intent(out)   :: xsc_rain_out    !pre-mix rain release rate from old canopy pool (mm h2o/s, >=0)
   real(r8), intent(out)   :: xsc_snow_out    !pre-mix snow release rate from old canopy pool (mm h2o/s, >=0)
   ! Phase-change tracer transfer (grid-scale mm, >=0). Used by tracer_precip
   ! to move trc_ldew_snow -> trc_ldew_rain (melt) or trc_ldew_rain ->
   ! trc_ldew_snow (freeze). Without this, canopy melt/freeze silently
   ! diverts snow-pool tracer mass into trc_pg_snow_ground (and rain pool
   ! mis-reports its isotope signature after melt).
   real(r8), intent(out)   :: ldew_smelt_out  !canopy snow->rain mass transferred by melt this step [mm, >=0]
   real(r8), intent(out)   :: ldew_frzc_out   !canopy rain->snow mass transferred by freeze this step [mm, >=0]
   ! Fusion heat flux carried by canopy rain<->snow phase change this step
   ! [W/m^2]. Sign convention: +canopy_phase_heat means canopy GAINS energy
   ! (freeze releases fusion heat); -canopy_phase_heat means canopy LOSES
   ! energy (melt absorbs fusion heat). MOD_Thermal + LeafTemperature/PC
   ! add this term to the canopy energy residual (err) and the global
   ! errore check so energy balance closes when phase change is done here.
   ! Without this, audit ROLLBACK NM-1 left fusion heat silently unaccounted,
   ! biasing tleaf warm -> fwet high -> Ec/ET high.
   real(r8), intent(out)   :: canopy_phase_heat_out   !canopy phase-change fusion heat flux [W/m^2, + heats canopy, - cools]

   ! Local variables
   real(r8)                :: PrecipAreaFrac !fraction of gridcell receiving precipitation [-]
   real(r8)                :: BDFALL
   ! Local clamps for precipitation inputs [mm/s].
   real(r8)                :: rain_clamp, snow_clamp
   ! Local fwet_snow used to pull tleaf toward tfrz after explicit
   ! melt/freeze (Niu et al. 2004 parameterization).
   real(r8)                :: fwet_snow
      ! Initialize phase-change tracer outputs (default: no phase change).
      ldew_smelt_out        = 0._r8
      ldew_frzc_out         = 0._r8
      canopy_phase_heat_out = 0._r8

      IF (lai+sai > 1e-6) THEN
         lsai   = lai + sai
         vegt   = lsai
         ! Guard restart/float-noise negative canopy state before
         ! (satcap-ldew) / fwet / xsc uses.
         ldew_rain = max(0._r8, ldew_rain)
         ldew_snow = max(0._r8, ldew_snow)
         ! Calculate vegetation fraction from LAI (alternative to input VegFrac)
         fvegc=max(0.05,1.0-exp(-0.52*lsai))

         ! Maximum canopy water - Noah-MP lines 82, 105
         ! Note: Official Noah-MP uses VegFrac as input variable
         ! CoLM uses fvegc calculated from LAI, which is also physically reasonable
         satcap_rain = fvegc * dewmx*vegt
         BDFALL      = 67.92+51.25*EXP(MIN(2.5,(tleaf-273.15))/2.59)
         satcap_snow = fvegc * 6.6*(0.27+46./BDFALL) * lsai
         satcap_snow = max(0.0,satcap_snow)

         ! Clamp precipitation inputs once at scheme entry.
         rain_clamp = MAX(0.0_r8, prc_rain + prl_rain + qflx_irrig_sprinkler)
         snow_clamp = MAX(0.0_r8, prc_snow + prl_snow)

         ! Noah-MP receives the full precipitation forcing.  Its fvegc
         ! partition below already accounts for precipitation through canopy
         ! gaps; applying the generic interception alpha here would count the
         ! vegetation cover twice and suppress interception.
         p0  = (rain_clamp + snow_clamp) * deltim
         ! CoLM forcing supplies total atmospheric precipitation and splits it
         ! internally into synthetic 1/3 convective and 2/3 large-scale
         ! components.  Noah-MP's total-precipitation fallback is 10%/90%; use
         ! that split here, while leaving uniform sprinkler water in ppl.
         ppc = 0.1_r8 * MAX(0.0_r8, prc_rain + prc_snow + &
                                      prl_rain + prl_snow) * deltim
         ppl = MAX(0.0_r8, p0 - ppc)

         ! Estimate PrecipAreaFrac based on precipitation type - Noah-MP line 47
         ! Convective precipitation typically covers ~10% of gridcell
         ! Stratiform precipitation typically covers ~100% of gridcell
         IF (qflx_irrig_sprinkler > 0._r8) THEN
            ! Noah-MP treats sprinkler irrigation as spatially uniform.
            PrecipAreaFrac = 1.0_r8
         ELSEIF (p0 > 0._r8) THEN
            PrecipAreaFrac = p0 / (10.0_r8*ppc + ppl)
            PrecipAreaFrac = max(0.1, min(1.0, PrecipAreaFrac))  ! constrain to [0.1, 1.0]
         ELSE
            PrecipAreaFrac = 1.0
         ENDIF

         ! Ensure ldew is consistent with components at entry
         ! NoahMP modifies ldew in-place; if ldew != ldew_rain + ldew_snow
         ! at entry (e.g., from initialization or restart), mass balance drifts
         ldew = ldew_rain + ldew_snow

         w   = ldew+p0

         ! Initialize excess water variables
         xsc_rain    = 0.0
         xsc_snow    = 0.0

         ! Entry-time overflow release.  This mirrors the original Noah-MP /
         ! older CoLM implementation and is needed when restart/phenology or
         ! a previous phase change leaves ldew_* above the current diagnosed
         ! capacity.  Without this drain, a small-area / low-LAI canopy can
         ! retain liquid water many times larger than satcap_rain and inflate
         ! fwet/Ec until evaporation slowly removes it.
         xsc_rain  = max(0._r8, ldew_rain - satcap_rain)
         xsc_snow  = max(0._r8, ldew_snow - satcap_snow)
         ldew_rain = ldew_rain - xsc_rain
         ldew_snow = ldew_snow - xsc_snow

         ! phase change and excess
         ! Absorb fusion heat into tleaf via the fwet_snow-weighted
         ! pull-toward-freezing.
         !
         ! THIS BLOCK IS FAITHFUL TO UPSTREAM NOAHMP:
         ! Original NoahMP (CanopyHydrologyMod.F90:119) does exactly
         !   TemperatureCanopy = CanopyWetFrac*ConstFreezePoint
         !                     + (1.0 - CanopyWetFrac)*TemperatureCanopy
         ! after melt/freeze, where CanopyWetFrac = (CanopyIce/CanopyIceMax)^0.667.
         ! Previously CoLM's NoahMP scheme had the tleaf update commented
         ! out, silently losing fusion energy and biasing tleaf warm.
         ! Same formula as MOD_LeafTemperature.F90:1302 (Niu et al. 2004).
         IF (tleaf > tfrz) THEN
            IF (ldew_snow>1.e-8) THEN
               ldew_smelt = MIN(ldew_snow,(tleaf-tfrz)*CICE*ldew_snow/DENICE/(HFUS))
               ldew_smelt = MAX(ldew_smelt,0.0)
               ldew_snow  = ldew_snow-ldew_smelt
               ldew_rain  = ldew_rain+ldew_smelt
               ldew_smelt_out = ldew_smelt_out + ldew_smelt
               ! Fusion heat absorbed by the canopy for this melt (in [mm]
               ! water-equivalent = kg/m^2): energy = ldew_smelt * HFUS.
               ! Flux (W/m^2) with canopy-loses-energy sign convention:
               canopy_phase_heat_out = canopy_phase_heat_out - HFUS * ldew_smelt / deltim
               xsc_rain   = xsc_rain + MAX(0._r8, ldew_rain - satcap_rain)
               ldew_rain  = ldew_rain - MAX(0._r8, ldew_rain - satcap_rain)
               ! Fusion heat now exported via canopy_phase_heat_out;
               ! LeafTemperature's qmelt/qfrz mass block is gated off for
               ! scheme=4 (phase_change_owned_by_interception), so no
               ! double-relocation and no silent energy loss. Previous
               ! ROLLBACK NM-1 comment retained for archaeology: removing
               ! the Niu (2004) in-line tl pull was correct given that
               ! dheatl only tracks iter-internal dtl, but the fix is in
               ! MOD_Thermal's errore term, not here.
            ENDIF
         ELSE
            IF (ldew_rain>1.e-8) THEN
               ldew_frzc  = MIN(ldew_rain,(tfrz-tleaf)*CWAT*ldew_rain/DENH2O/(HFUS))
               ldew_frzc  = MAX(ldew_frzc,0.0)
               ldew_snow  = ldew_snow+ldew_frzc
               ldew_rain  = ldew_rain-ldew_frzc
               ldew_frzc_out = ldew_frzc_out + ldew_frzc
               ! Fusion heat released to the canopy (canopy gains energy):
               canopy_phase_heat_out = canopy_phase_heat_out + HFUS * ldew_frzc / deltim
               xsc_snow   = xsc_snow + MAX(0._r8, ldew_snow - satcap_snow)
               ldew_snow  = ldew_snow - MAX(0._r8, ldew_snow - satcap_snow)
            ENDIF
         ENDIF
         ! Resync ldew with components after phase change (CoLM2014 pattern)
         ldew = ldew_rain + ldew_snow

         int_snow = 0._r8
         ICEDRIP = 0._r8
         ICEDRIP_OLD = 0._r8
         ICEDRIP_NEW = 0._r8

         IF (p0 > 0._r8) THEN

            ! Throughfall: direct precipitation through vegetation gaps - Noah-MP lines 91, 119
            tti_rain = rain_clamp*deltim * ( 1.-fvegc )
            tti_snow = snow_clamp*deltim * ( 1.-fvegc )

            ! Interception and drip calculation - Noah-MP lines 86-90, 109-118
            ! Interception rate [mm/s]
            int_rain = fvegc * rain_clamp * PrecipAreaFrac  ! max interception capability
            int_rain = min(int_rain, (satcap_rain-ldew_rain)/deltim * &
                          (1.0-exp(-rain_clamp*deltim/satcap_rain)))
            int_rain = max(0., int_rain)

            int_snow = fvegc * snow_clamp * PrecipAreaFrac  ! max interception capability
            int_snow = min(int_snow, (satcap_snow-ldew_snow)/deltim * &
                          (1.0-exp(-snow_clamp*deltim/satcap_snow)))
            int_snow = max(0., int_snow)

            ! Drip: excess precipitation on vegetation that cannot be intercepted
            tex_rain = rain_clamp*fvegc*deltim  - int_rain*deltim
            tex_snow = snow_clamp*fvegc*deltim - int_snow*deltim
#if (defined CoLMDEBUG)
            IF (tex_rain+tex_snow+tti_rain+tti_snow-p0 > 1.e-10) THEN
               write(6,*) 'tex_ + tti_ > p0 in interception code : '
            ENDIF
#endif
         ELSE
            ! all intercepted by canopy leaves for very small precipitation
            tti_rain = 0.
            tti_snow = 0.
            tex_rain = 0.
            tex_snow = 0.
         ENDIF

         ! Snow unloading after interception, following original Noah-MP
         ! ordering and using canopy temperature (TemperatureCanopy).
         ! Split unloading into old-pool release (xsc_snow) and same-step
         ! new-snow drip (tex_snow) so tracer semantics remain consistent.
         IF (ldew_snow > 1.e-8) THEN
            FT = MAX(0.0,(tleaf - 270.15) / 1.87E5)
            FV = SQRT(forc_us*forc_us + forc_vs*forc_vs) / 1.56E5
            ! Upstream Noah-MP bases unloading on canopy ice present at the
            ! start of the step; newly intercepted snow is not in the source.
            ICEDRIP = MAX(0._r8, ldew_snow) * (FV+FT)
            ICEDRIP = MIN(ICEDRIP, ldew_snow/deltim + int_snow)
            ICEDRIP_OLD = MIN(ICEDRIP, ldew_snow/deltim)
            ICEDRIP_NEW = MAX(0._r8, ICEDRIP - ICEDRIP_OLD)
            xsc_snow = xsc_snow + ICEDRIP_OLD * deltim
            ldew_snow = ldew_snow - ICEDRIP_OLD * deltim
            tex_snow = tex_snow + ICEDRIP_NEW * deltim
         ENDIF

         !BDFALL = 67.92+51.25*EXP(MIN(2.5,(SFCTMP-TFRZ))/2.59)

         !----------------------------------------------------------------------
         !   total throughfall (thru) and store augmentation
         !----------------------------------------------------------------------

         thru_rain = tti_rain + tex_rain
         thru_snow = tti_snow + tex_snow
         pinf = p0 - (thru_rain + thru_snow)

         ! Update rain/snow components following CoLM2014 pattern (lines 322-324)
         ldew_rain = ldew_rain + rain_clamp*deltim - thru_rain
         ldew_snow = ldew_snow + snow_clamp*deltim - thru_snow
         ldew = ldew_rain + ldew_snow

         pg_rain = (xsc_rain + thru_rain) / deltim
         pg_snow = (xsc_snow + thru_snow) / deltim
         qintr   = pinf / deltim

         qintr_rain = rain_clamp - (thru_rain / deltim)
         qintr_snow = snow_clamp - (thru_snow / deltim)

         ! Gross interception rate: NoahMP's tti_* is the (1-fvegc) gap
         ! throughfall; the rest of precip hits canopy and enters the
         ! mixed pool (some of which drips as tex_*).
         gross_intr_rain = max(0._r8, rain_clamp &
                                       - tti_rain / deltim)
         gross_intr_snow = max(0._r8, snow_clamp - tti_snow / deltim)

         ! Pre-mix release: accumulated from ICEDRIP (snow unloading) and
         ! phase-change overflow, both of which flushed the OLD canopy
         ! pool before new precip mixing. Downstream tex_rain/tex_snow are
         ! post-mix drips and stay in pg_rain at R_mixed via tracer.
         xsc_rain_out = xsc_rain / deltim
         xsc_snow_out = xsc_snow / deltim

#if (defined CoLMDEBUG)
         w = w - ldew - (pg_rain+pg_snow)*deltim
         IF (abs(w) > INTERCEPTION_BALANCE_TOL) THEN
            write(6,*) 'something wrong in interception code : '
            write(6,*) w, ldew, (pg_rain+pg_snow)*deltim  !, satcap
            CALL abort
         ENDIF

         CALL check_interception_balance('NoahMP', &
              ldew, ldew_rain, ldew_snow, pg_rain, pg_snow, &
              qintr, qintr_rain, qintr_snow)
#endif

      ELSE
         ! 07/15/2023, Hua Yuan: bug found for ldew value reset when vegetation disappears
         ! Release canopy water separately by phase to preserve phase
         ! conservation.
         ! Clamp raw precipitation to prevent negative noise from propagating
         ! to pg_* in the no-vegetation branch, matching the in-branch clamp
         ! and JULES's no-veg treatment.
         ! Preserve pre-existing canopy water signature for tracer attribution.
         ! The ldew_rain / ldew_snow released here is OLD canopy water
         ! (R_canopy_pre), not fresh throughfall (R_input). Route it through
         ! xsc_*_out BEFORE ldew is reset so tracer_precip classifies it
         ! correctly (otherwise it would be lumped into throughfall and diluted
         ! by fresh precip signature).
         CALL release_no_vegetation_canopy_storage(deltim,tleaf, &
              prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler, &
              ldew,ldew_rain,ldew_snow,pg_rain,pg_snow, &
              qintr,qintr_rain,qintr_snow,gross_intr_rain,gross_intr_snow, &
              xsc_rain_out,xsc_snow_out)

      ENDIF

   END SUBROUTINE LEAF_interception_NOAHMP


   SUBROUTINE LEAF_interception_MATSIRO (deltim,dewmx,forc_us,forc_vs,chil,sigf,lai,sai,tair,tleaf, &
                                         prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler,&
                                         ldew,ldew_rain,ldew_snow,z0m,hu,pg_rain,pg_snow,qintr,&
                                         qintr_rain,qintr_snow,&
                                         gross_intr_rain,gross_intr_snow,&
                                         xsc_rain_out,xsc_snow_out,&
                                         ldew_smelt_out,ldew_frzc_out,&
                                         canopy_phase_heat_out)
!DESCRIPTION
!===========
   ! Interception and drainage of precipitation
   ! the treatment are modified from MATSIRO 6 (under development)

!Original Author:
!-------------------
   !---MATSIRO6 document writing team∗

!References:
!-------------------
   !---Tatebe, H., Ogura, T., Nitta, T., Komuro, Y., Ogochi, K., Takemura, T., Sudo, K., Sekiguchi, M.,
   !   Abe, M., Saito, F. and Chikira, M., 2019. Description and basic evaluation of simulated mean state,
   !   internal variability, and climate sensitivity in MIROC6. Geoscientific Model Development, 12(7), pp.2727-2765. 116(D12).
   !---Takata, K., Emori, S. and Watanabe, T., 2003. Development of the minimal advanced treatments of surface interaction and
   !   runoff. Global and planetary Change, 38(1-2), pp.209-222.
   !---Guo, Q., Kino, K., Li, S., Nitta, T., Takeshima, A., Suzuki, K.T., Yoshida, N. and Yoshimura, K., 2021.
   !   Description of MATSIRO6.

!ANCILLARY FUNCTIONS AND SUBROUTINES
!-------------------

!REVISION HISTORY
!----------------
   ! 2026.02.11  Zhongwang Wei @ SYSU - Added input clamping, comment fixes
   ! 2023.02.21  Zhongwang Wei @ SYSU
   ! 2021.12.08  Zhongwang Wei @ SYSU
!=======================================================================

   IMPLICIT NONE

   real(r8), intent(in) :: deltim       !seconds in a time step [second]
   real(r8), intent(in) :: dewmx        !maximum dew [mm]
   real(r8), intent(in) :: forc_us      !wind speed
   real(r8), intent(in) :: forc_vs      !wind speed
   real(r8), intent(in) :: chil         !leaf angle distribution factor
   real(r8), intent(in) :: prc_rain     !convective rainfall [mm/s]
   real(r8), intent(in) :: prc_snow     !convective snowfall [mm/s]
   real(r8), intent(in) :: prl_rain     !large-scale rainfall [mm/s]
   real(r8), intent(in) :: prl_snow     !large-scale snowfall [mm/s]
   real(r8), intent(in) :: qflx_irrig_sprinkler !irrigation and sprinkler water flux [mm/s]
   real(r8), intent(in) :: sigf         !fraction of veg cover, excluding snow-covered veg [-]
   real(r8), intent(in) :: lai          !leaf area index [-]
   real(r8), intent(in) :: sai          !stem area index [-]
   real(r8), intent(in) :: tair         !air temperature [K]
   real(r8), intent(inout) :: tleaf     !sunlit canopy leaf temperature [K]

   real(r8), intent(inout) :: ldew      !depth of water on foliage [mm]
   real(r8), intent(inout) :: ldew_rain !depth of liquid on foliage [mm]
   real(r8), intent(inout) :: ldew_snow !depth of solid (frozen) on foliage [mm]
   real(r8), intent(in)    :: z0m       !roughness length
   real(r8), intent(in)    :: hu        !forcing height of  U


   real(r8), intent(out) :: pg_rain     !rainfall onto ground including canopy runoff [kg/(m2 s)]
   real(r8), intent(out) :: pg_snow     !snowfall onto ground including canopy runoff [kg/(m2 s)]
   real(r8), intent(out) :: qintr       !interception [kg/(m2 s)]
   real(r8), intent(out) :: qintr_rain  !rainfall interception (mm h2o/s) [NET storage-related flux; can be <0 when canopy drains faster than new rain intercepts. Use gross_intr_rain for gross interception]
   real(r8), intent(out) :: qintr_snow  !snowfall interception (mm h2o/s) [NET storage-related flux; can be <0 (e.g. snow unloading/blowing with no new snow). Use gross_intr_snow for gross interception]
   real(r8), intent(out) :: gross_intr_rain !gross rain entering canopy mixed pool (mm h2o/s, >=0)
   real(r8), intent(out) :: gross_intr_snow !gross snow entering canopy mixed pool (mm h2o/s, >=0)
   real(r8), intent(out) :: xsc_rain_out    !pre-mix rain release rate from old canopy pool (mm h2o/s, >=0)
   real(r8), intent(out) :: xsc_snow_out    !pre-mix snow release rate from old canopy pool (mm h2o/s, >=0)
   ! Phase-change tracer transfer (grid-scale mm, >=0). See NoahMP block
   ! for rationale; single-cap phase change here guarantees the transferred
   ! mass stays inside the canopy pool, so tracer_precip must move the
   ! corresponding tracer mass between trc_ldew_snow and trc_ldew_rain.
   real(r8), intent(out) :: ldew_smelt_out  !canopy snow->rain mass transferred by melt this step [mm, >=0]
   real(r8), intent(out) :: ldew_frzc_out   !canopy rain->snow mass transferred by freeze this step [mm, >=0]
   ! Canopy phase-change fusion heat flux [W/m^2]. See NoahMP block above
   ! for sign convention (+heats canopy / -cools). Must be exported to
   ! MOD_Thermal so the canopy energy residual closes (MATSIRO's
   ! original cwmelt -> cflxbl path is emulated via this bulk flux).
   real(r8), intent(out) :: canopy_phase_heat_out
   !local
   real(r8) :: fint, Ac, dewmx_MATSIRO,ldew_rain_s, ldew_snow_s,ldew_rain_n, ldew_snow_n
   real(r8) :: tex_rain_n,tex_rain_s,tex_snow_n,tex_snow_s,tti_rain_n,tti_rain_s,tti_snow_n,tti_snow_s
   real(r8) :: fwet_snow                ! Canopy wet fraction for phase-heat tleaf pull
   ! Local clamps for precipitation inputs [mm/s].
   ! MATSIRO separates convective and stratiform, so keep component
   ! clamps as well for storm/non-storm formulas.
   real(r8) :: rain_clamp, snow_clamp
   real(r8) :: rain_strat_clamp  ! prl_rain + irrigation (stratiform, uniform) [mm/s]
   real(r8) :: rain_conv_clamp   ! prc_rain (convective, concentrated in Ac) [mm/s]
   real(r8) :: snow_strat_clamp, snow_conv_clamp
      !the canopy water capacity per leaf area index is set to 0.2mm
      dewmx_MATSIRO = 0.2_r8 * DEF_MATSIRO_CWCAP_SCALE
      !the fraction of the convective precipitation area is assumed to be uniform (0.1)
      Ac            = 0.1

      ! Initialize phase-change tracer outputs (default: no phase change).
      ldew_smelt_out        = 0._r8
      ldew_frzc_out         = 0._r8
      canopy_phase_heat_out = 0._r8

      IF (lai > 1e-6_r8) THEN
         lsai   = lai + sai
         vegt   = lsai
         ! Guard restart/float-noise negative canopy state before
         ! single-cap xsc / phase-change / Rutter drip.
         ldew_rain = max(0._r8, ldew_rain)
         ldew_snow = max(0._r8, ldew_snow)
         ! Clamp precipitation inputs once at scheme entry.
         ! Component clamps preserved separately for the storm/non-storm
         ! split (L1533-1571 use them directly).
         ! CoLM supplies a synthetic 1/3 convective, 2/3 large-scale split
         ! even when the forcing contains total precipitation only. Recombine
         ! it here, then use a MATSIRO-local 10%/90% fallback. Sprinkler water
         ! remains uniform (large-scale) precipitation.
         rain_conv_clamp  = 0.1_r8 * MAX(0.0_r8, prc_rain + prl_rain)
         rain_strat_clamp = 0.9_r8 * MAX(0.0_r8, prc_rain + prl_rain) &
                            + MAX(0.0_r8, qflx_irrig_sprinkler)
         snow_conv_clamp  = 0.1_r8 * MAX(0.0_r8, prc_snow + prl_snow)
         snow_strat_clamp = 0.9_r8 * MAX(0.0_r8, prc_snow + prl_snow)
         rain_clamp = rain_strat_clamp + rain_conv_clamp
         snow_clamp = snow_strat_clamp + snow_conv_clamp

         p0  = (rain_clamp + snow_clamp) * deltim
         ! Storage capacity follows original MATSIRO6 (matsiro.f90 cnwcap):
         !   cwcap = cnw_wcmax * LAI, where cnw_wcmax = 0.2 mm
         ! LAI only (not LAI+SAI) — stems do not hold canopy water in MATSIRO.
         !
         ! Original MATSIRO uses a SINGLE cwcap shared by rain+snow
         ! (matsiro.f90:3564). Two-bucket caps doubled the effective canopy
         ! storage (0.4*LAI instead of 0.2*LAI), inflating Ec via more
         ! retained water. Keep both satcap_rain and satcap_snow variables
         ! for interface continuity but assign them the same single cwcap
         ! so every downstream comparison implicitly enforces the shared-cap
         ! semantics.
         satcap_rain = dewmx_MATSIRO*lai        ! = single cwcap
         satcap_snow = satcap_rain              ! shared cap, not independent

         ! Ensure ldew is consistent with components at entry
         ! MATSIRO modifies ldew in-place; inconsistency at entry propagates to output
         ldew = ldew_rain + ldew_snow

         w = ldew+p0

         ! Single-cap pre-mix overflow. Previously each component was clipped
         ! to its own satcap (= dewmx*LAI), so total ldew could sit at
         ! 2*cwcap. Now clip combined (rain+snow) vs cwcap and split the
         ! excess by current phase ratio (same pattern as CLM4 at L798-807).
         IF (ldew > satcap_rain .and. ldew > 1.e-12_r8) THEN
            xsc_rain = (ldew - satcap_rain) * ldew_rain / ldew
            xsc_snow = (ldew - satcap_rain) * ldew_snow / ldew
         ELSE
            xsc_rain = 0._r8
            xsc_snow = 0._r8
         ENDIF
         ldew_rain = ldew_rain - xsc_rain
         ldew_snow = ldew_snow - xsc_snow
         ldew      = ldew_rain + ldew_snow   ! <= satcap_rain now

         ! phase change and excess
         ! Absorb fusion heat into tleaf to prevent the silent energy loss
         ! that previously biased tleaf warm (during melt) and inflated
         ! downstream Ec.
         !
         ! NOTE — APPROXIMATION, NOT FAITHFUL TO UPSTREAM MATSIRO:
         ! The original MATSIRO (matsiro.f90 matcnw L3430-3431) does NOT
         ! use this fwet*tfrz + (1-fwet)*tleaf formula. Instead it
         ! accumulates cwmelt and exports cflxbl = -emelt * cwmelt *
         ! dwatr / dt as a canopy heat flux that the parent energy
         ! balance applies to canopy temperature.
         !
         ! Faithfully reproducing that path would require adding a
         ! canopy_phase_heat output parameter and wiring it to CoLMMAIN's
         ! energy budget. Until that follow-up lands, we mirror the Niu
         ! et al. (2004) pull-toward-freezing already used in
         ! MOD_LeafTemperature.F90:1302 (CoLM's own scheme-agnostic
         ! relaxation). Net effect on bulk Ec is similar — fusion energy
         ! is no longer thrown away — but the exact tleaf trajectory
         ! during phase events is a CoLM approximation, not original
         ! MATSIRO.
         !
         ! Since total mass is preserved by phase change, the single-cap
         ! constraint ldew <= satcap_rain established above still holds
         ! afterwards (no additional xsc accumulation needed).
         IF (tleaf > tfrz) THEN
            IF (ldew_snow>1.e-8) THEN
               ldew_smelt = MIN(ldew_snow,(tleaf-tfrz)*CICE*ldew_snow/DENICE/(HFUS))
               ldew_smelt = MAX(ldew_smelt,0.0)
               ldew_snow  = ldew_snow-ldew_smelt
               ldew_rain  = ldew_rain+ldew_smelt
               ldew_smelt_out = ldew_smelt_out + ldew_smelt
               canopy_phase_heat_out = canopy_phase_heat_out - HFUS * ldew_smelt / deltim
               ! MATSIRO's cwmelt -> cflxbl fusion flux is now exported via
               ! canopy_phase_heat_out for MOD_Thermal to consume in the
               ! canopy energy balance. LeafTemperature's qmelt/qfrz block
               ! is gated off for scheme=5 to prevent double-site phase
               ! change. See MOD_Thermal.F90 errore term.
            ENDIF
         ELSE
            IF (ldew_rain>1.e-8) THEN
               ldew_frzc  = MIN(ldew_rain,(tfrz-tleaf)*CWAT*ldew_rain/DENH2O/(HFUS))
               ldew_frzc  = MAX(ldew_frzc,0.0)
               ldew_snow  = ldew_snow+ldew_frzc
               ldew_rain  = ldew_rain-ldew_frzc
               ldew_frzc_out = ldew_frzc_out + ldew_frzc
               canopy_phase_heat_out = canopy_phase_heat_out + HFUS * ldew_frzc / deltim
               ! Freeze branch (canopy gains fusion heat).
            ENDIF
         ENDIF
         ! Resync ldew with components after phase change (CoLM2014 pattern)
         ldew = ldew_rain + ldew_snow

         IF (p0 > 1.e-8) THEN
            ! Interception efficiency - MATSIRO formulation
            ! MATSIRO uses simple linear saturation following Takata et al. (2003)
            ! Reference: Takata, K., Emori, S., and Watanabe, T. (2003). "Development of the
            ! minimal advanced treatments of surface interaction and runoff", Global and
            ! Planetary Change, 38, 209-222, doi:10.1016/S0921-8181(03)00030-4
            ! Verified against official MATSIRO source code (matsiro.f90):
            !   fctint = min(grlai(ud), 1.d0)
            ! LAI only (not LAI+SAI): stems do not intercept in MATSIRO.
            ! When LAI ≤ 1: efficiency equals LAI
            ! When LAI > 1: efficiency saturates at 100%
            fpi_rain  = min(1.0, lai)
            fpi_snow  = min(1.0, lai)

            !-----------------------------------------------------------------------
            ! Storm area
            !-----------------------------------------------------------------------
            ! Use clamped component rates (stratiform vs convective) so
            ! negative forcing noise cannot bleed into the storm/non-storm
            ! split of tti/ldew.
            ldew_rain_s = ldew_rain + (rain_strat_clamp * fpi_rain + rain_conv_clamp * fpi_rain / Ac)  * deltim
            ldew_snow_s = ldew_snow + (snow_strat_clamp * fpi_snow + snow_conv_clamp * fpi_snow / Ac)  * deltim
            !
            tti_rain_s  = (rain_strat_clamp + rain_conv_clamp/Ac) * (1.d0-fpi_rain) * deltim
            tti_snow_s  = (snow_strat_clamp + snow_conv_clamp/Ac) * (1.d0-fpi_snow) * deltim

            !
            ! Rutter exponential drainage formula (Rutter et al. 1975)
            ! tex = overflow + k1 * exp(k2 * storage)
            !
            ! Physical constants from Rutter et al. (1975):
            ! - cwb_adrp1 = 1.14e-11 [m/s]: Base dripping coefficient
            !   Represents minimum drainage rate when canopy is near saturation
            ! - cwb_adrp2 = 3.7e3 [1/m]: Exponential saturation factor
            !   Controls how rapidly drainage increases with storage
            !   Higher values = more sensitive to storage amount
            ! - min(50.0, ...): Overflow protection to prevent EXP(large_number)
            !   Caps exponent at 50 to avoid numerical overflow
            !   (exp(50) ≈ 5e21, near double precision limit)
            !
            ! Unit conversion: 1.14e-11 [m/s] × 1000 [mm/m] = 1.14e-8 [mm/s]
            !
            ! Rain throughfall compares ldew_rain_s + ldew_snow_s vs cwcap
            ! (matsiro.f90:3392 `winps`). Base Rutter drip still uses
            ! min(ldew_rain_s, cwcap) because only the liquid pool produces
            ! continuous dripping in the original formulation.
            tex_rain_s  = max(ldew_rain_s + ldew_snow_s - satcap_rain, 0.d0) &
                        + (1.14d-11)*1000.*deltim*exp(min(50.0d0, min(ldew_rain_s,satcap_rain)/1000.* 3.7d3))
            tex_rain_s  = min(tex_rain_s, ldew_rain_s)
            ldew_rain_s = ldew_rain_s - tex_rain_s

            ! Snow drainage using same Rutter formula (see rain drainage comments above).
            ! Snow throughfall compares ldew_snow_s only vs cwcap
            ! (matsiro.f90:3397 `snfls`) — asymmetric vs rain by design.
            tex_snow_s  = max(ldew_snow_s - satcap_snow, 0.d0) + (1.14d-11)*1000.*deltim*exp(min(50.0d0, min(ldew_snow_s,satcap_snow)/1000.0* 3.7d3))
            tex_snow_s  = min(tex_snow_s, ldew_snow_s)
            ldew_snow_s = ldew_snow_s - tex_snow_s

            !-------------------------------------------------------------------------
            ! Non-storm area
            !-------------------------------------------------------------------------
            ldew_rain_n = ldew_rain + rain_strat_clamp * fpi_rain  * deltim
            ldew_snow_n = ldew_snow + snow_strat_clamp * fpi_snow  * deltim

            !
            tti_rain_n  = rain_strat_clamp * (1.d0-fpi_rain) * deltim
            tti_snow_n  = snow_strat_clamp * (1.d0-fpi_snow) * deltim

            ! Rutter drainage for non-storm area (single-cap semantics, see
            ! M-Bug-A note in the storm block above; matsiro.f90:3407 `winpn`).
            tex_rain_n  = max(ldew_rain_n + ldew_snow_n - satcap_rain, 0.d0) &
                        + (1.14d-11)*1000.*deltim*exp(min(50.0d0, min(ldew_rain_n,satcap_rain)/1000.* 3.7d3))
            tex_rain_n  = min(tex_rain_n, ldew_rain_n)
            ldew_rain_n = ldew_rain_n - tex_rain_n

            ! Snow drainage for non-storm area (matsiro.f90:3413 `snfln`).
            tex_snow_n  =  max(ldew_snow_n - satcap_snow, 0.d0) + (1.14d-11)*1000.*deltim*exp(min(50.0d0, min(ldew_snow_n,satcap_snow)/1000.* 3.7d3))
            tex_snow_n  =  min(tex_snow_n, ldew_snow_n)
            ldew_snow_n =  ldew_snow_n - tex_snow_n
            !-------------------------------------------------------------------------
            !-------------------------------------------------------------------------
            ! Average
            !-------------------------------------------------------------------------
            ldew_rain = ldew_rain_n + (ldew_rain_s - ldew_rain_n) * Ac
            ldew_snow = ldew_snow_n + (ldew_snow_s - ldew_snow_n) * Ac
            ldew_rain = max(0.0,ldew_rain)
            ldew_snow = max(0.0,ldew_snow)

            tti_rain  = tti_rain_n*(1-Ac)+tti_rain_s*Ac
            tti_snow  = tti_snow_n+(tti_snow_s-tti_snow_n) * Ac
            tti_rain  = max(0.0,tti_rain)
            tti_snow  = max(0.0,tti_snow)

            tex_rain  = tex_rain_n+(tex_rain_s-tex_rain_n)*Ac
            tex_snow  = tex_snow_n+(tex_snow_s-tex_snow_n)*Ac
            tex_rain  = max(0.0,tex_rain)
            tex_snow  = max(0.0,tex_snow)
            !-------------------------------------------------------------------------

! NOTE: The check "tex+tti > p0" is not applicable to MATSIRO scheme.
! Rutter exponential drainage drains pre-existing canopy water (ldew),
! so tex+tti can legitimately exceed p0. The real mass balance check
! is performed below (w residual check with abort).

         ELSE
            ! On no-precipitation steps, the original MATSIRO design still
            ! drains pre-existing canopy water via Rutter's exponential
            ! formula (matsiro.f90 cnwbdg does not gate drainage on
            ! precipitation). The previous zeroing here made canopy water
            ! stay put indefinitely on dry steps. Run the same Rutter formula
            ! on the stored ldew_rain/ldew_snow so drainage proceeds whether
            ! or not precipitation is falling.
            ! Rain drainage uses combined ldew_rain+ldew_snow vs cwcap;
            ! snow drainage uses ldew_snow only.
            tex_rain = max(ldew_rain + ldew_snow - satcap_rain, 0.0_r8) &
                     + (1.14d-11)*1000._r8*deltim &
                        * exp(min(50.0d0, min(ldew_rain, satcap_rain)/1000._r8*3.7d3))
            tex_rain = min(tex_rain, ldew_rain)
            ldew_rain = ldew_rain - tex_rain

            tex_snow = max(ldew_snow - satcap_snow, 0.0_r8) &
                     + (1.14d-11)*1000._r8*deltim &
                        * exp(min(50.0d0, min(ldew_snow, satcap_snow)/1000._r8*3.7d3))
            tex_snow = min(tex_snow, ldew_snow)
            ldew_snow = ldew_snow - tex_snow

            tti_rain = 0._r8
            tti_snow = 0._r8
         ENDIF

         !BDFALL = 67.92+51.25*EXP(MIN(2.5,(SFCTMP-TFRZ))/2.59)

         !----------------------------------------------------------------------
         !   total throughfall (thru) and store augmentation
         !----------------------------------------------------------------------

         thru_rain = tti_rain + tex_rain
         thru_snow = tti_snow + tex_snow
         pinf = p0 - (thru_rain + thru_snow)

         ! Resync ldew with components following CoLM2014 pattern (line 324)
         ! In the precip case, ldew_rain/ldew_snow were updated via weighted average
         ! In the no-precip case, tex_* drained pre-existing storage
         ldew = ldew_rain + ldew_snow

         pg_rain = (xsc_rain + thru_rain) / deltim
         pg_snow = (xsc_snow + thru_snow) / deltim
         qintr   = pinf / deltim

         qintr_rain = rain_clamp - (thru_rain / deltim)
         qintr_snow = snow_clamp - (thru_snow / deltim)

         ! Gross interception rate: MATSIRO's grid-averaged tti_* is the
         ! (1-fpi)*rain gap-direct component; gross = rain_total - tti/dt.
         gross_intr_rain = max(0._r8, rain_clamp &
                                       - tti_rain / deltim)
         gross_intr_snow = max(0._r8, snow_clamp - tti_snow / deltim)

         ! Pre-mix old-pool release rate: capacity overflow + phase-change
         ! residual accumulated into xsc_rain/xsc_snow BEFORE interception.
         xsc_rain_out = xsc_rain / deltim
         xsc_snow_out = xsc_snow / deltim
#if (defined CoLMDEBUG)
         w = w - ldew - (pg_rain+pg_snow)*deltim
         IF (abs(w) > INTERCEPTION_BALANCE_TOL) THEN
            write(6,*) 'something wrong in interception code : '
            write(6,*) w, ldew, (pg_rain+pg_snow)*deltim !, satcap
            CALL abort
         ENDIF

         CALL check_interception_balance('MATSIRO', &
              ldew, ldew_rain, ldew_snow, pg_rain, pg_snow, &
              qintr, qintr_rain, qintr_snow)
#endif

      ELSE
         ! No vegetation: all precipitation passes through, release any
         ! stored water.
         ! Clamp raw precipitation to prevent negative noise from propagating
         ! to pg_* in the no-vegetation branch, matching the in-branch clamp
         ! and JULES's no-veg treatment.
         ! Preserve pre-existing canopy water signature for tracer attribution.
         ! The ldew_rain / ldew_snow released here is OLD canopy water
         ! (R_canopy_pre), not fresh throughfall (R_input). Route it through
         ! xsc_*_out BEFORE ldew is reset so tracer_precip classifies it
         ! correctly (otherwise it would be lumped into throughfall and diluted
         ! by fresh precip signature).
         CALL release_no_vegetation_canopy_storage(deltim,tleaf, &
              prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler, &
              ldew,ldew_rain,ldew_snow,pg_rain,pg_snow, &
              qintr,qintr_rain,qintr_snow,gross_intr_rain,gross_intr_snow, &
              xsc_rain_out,xsc_snow_out)
      ENDIF
   END SUBROUTINE LEAF_interception_MATSIRO

   SUBROUTINE LEAF_interception_VIC (deltim,dewmx,forc_us,forc_vs,chil,sigf,fsno,lai,sai,tair,tleaf, &
                                       prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler,&
                                       ldew,ldew_rain,ldew_snow,z0m,hu,pg_rain,&
                                       pg_snow,qintr,qintr_rain,qintr_snow,&
                                       gross_intr_rain,gross_intr_snow,&
                                       xsc_rain_out,xsc_snow_out,&
                                       ldew_smelt_out,ldew_frzc_out,&
                                       canopy_phase_heat_out)
!DESCRIPTION
!===========
   ! Calculation of  interception and drainage of precipitation
   ! the treatment are based on VIC 5.0 (under development)

!Original Author:
!-------------------
   !---Hamman, J.J. AND Liang X.

!References:
!-------------------
   !---Hamman, J.J., Nijssen, B., Bohn, T.J., Gergel, D.R. and Mao, Y., 2018.
   !   The Variable Infiltration Capacity model version 5 (VIC-5): Infrastructure
   !   improvements for new applications and reproducibility. Geoscientific Model Development,
   !   11(8), pp.3481-3496.
   !---Liang, X., Lettenmaier, D.P., Wood, E.F. and Burges, S.J., 1994.
   !   A simple hydrologically based model of land surface water and energy fluxes
   !   for general circulation models. Journal of Geophysical Research: Atmospheres, 99(D7),
   !   pp.14415-14428.

!ANCILLARY FUNCTIONS AND SUBROUTINES
!-------------------

!REVISION HISTORY
!----------------
   ! 2026.02.11  Zhongwang Wei @ SYSU - Added input clamping, comment fixes
   ! 2023.02.21  Zhongwang Wei @ SYSU
   ! 2021.12.08  Zhongwang Wei @ SYSU
!=======================================================================


   IMPLICIT NONE

   real(r8), intent(in) :: deltim       !seconds in a time step [second]
   real(r8), intent(in) :: dewmx        !maximum dew [mm]
   real(r8), intent(in) :: forc_us      !wind speed
   real(r8), intent(in) :: forc_vs      !wind speed
   real(r8), intent(in) :: chil         !leaf angle distribution factor
   real(r8), intent(in) :: prc_rain     !convective rainfall [mm/s]
   real(r8), intent(in) :: prc_snow     !convective snowfall [mm/s]
   real(r8), intent(in) :: prl_rain     !large-scale rainfall [mm/s]
   real(r8), intent(in) :: prl_snow     !large-scale snowfall [mm/s]
   real(r8), intent(in) :: qflx_irrig_sprinkler !irrigation and sprinkler water flux [mm/s]
   real(r8), intent(in) :: sigf         !fraction of veg cover, excluding snow-covered veg [-]
   real(r8), intent(in) :: fsno         !fraction of ground covered by snow [-]
   real(r8), intent(in) :: lai          !leaf area index [-]
   real(r8), intent(in) :: sai          !stem area index [-]
   real(r8), intent(in) :: tair         !air temperature [K]
   real(r8), intent(inout) :: tleaf     !sunlit canopy leaf temperature [K]

   real(r8), intent(inout) :: ldew      !depth of water on foliage [mm]
   real(r8), intent(inout) :: ldew_rain !depth of liquid on foliage [mm]
   real(r8), intent(inout) :: ldew_snow !depth of solid (frozen) on foliage [mm]
   real(r8), intent(in) :: z0m          !roughness length
   real(r8), intent(in) :: hu           !forcing height of U


   real(r8), intent(out) :: pg_rain     !rainfall onto ground including canopy runoff [kg/(m2 s)]
   real(r8), intent(out) :: pg_snow     !snowfall onto ground including canopy runoff [kg/(m2 s)]
   real(r8), intent(out) :: qintr       !interception [kg/(m2 s)]
   real(r8), intent(out) :: qintr_rain  !rainfall interception (mm h2o/s) [NET storage-related flux; can be <0 when canopy drains faster than new rain intercepts. Use gross_intr_rain for gross interception]
   real(r8), intent(out) :: qintr_snow  !snowfall interception (mm h2o/s) [NET storage-related flux; can be <0 (e.g. snow unloading/blowing with no new snow). Use gross_intr_snow for gross interception]
   real(r8), intent(out) :: gross_intr_rain !gross rain entering canopy mixed pool (mm h2o/s, >=0)
   real(r8), intent(out) :: gross_intr_snow !gross snow entering canopy mixed pool (mm h2o/s, >=0)
   real(r8), intent(out) :: xsc_rain_out    !pre-mix rain release rate from old canopy pool (mm h2o/s, >=0)
   real(r8), intent(out) :: xsc_snow_out    !pre-mix snow release rate from old canopy pool (mm h2o/s, >=0)
   real(r8), intent(out) :: ldew_smelt_out  !canopy snow->rain mass transferred by melt this step [mm, >=0]
   real(r8), intent(out) :: ldew_frzc_out   !canopy rain->snow mass transferred by freeze this step [mm, >=0]
   ! Canopy phase-change fusion heat flux [W/m^2].
   ! Emulates VIC's RefreezeEnergy / canopy energy-balance path as a bulk
   ! flux for MOD_Thermal to consume.
   real(r8), intent(out) :: canopy_phase_heat_out

   real(r8) :: Imax1,Lr,Snow,Rain,DeltaSnowInt,Wind,BlownSnow
   real(r8) :: MaxInt,Overload,IntRainFract,IntSnowFract,ldew_smelt,MaxWaterInt
   ! Local clamps for precipitation inputs [mm/s].
   real(r8) :: rain_clamp, snow_clamp
   ! Canopy wet fraction for fusion-heat tleaf pull
   real(r8) :: fwet_snow
   ! Actual mass entering the VIC vegetation-tile canopy pool this step
   ! (DeltaSnowInt-post-BlownSnow for snow; capacity-limited inflow for
   ! rain). Used to emit gross_intr_* that tracer_precip can trust,
   ! instead of diagnosing it from total precipitation.
   real(r8) :: actual_rain_int, actual_snow_int
   logical  :: vic_snow_intercept_context
   ! DEF_VEG_SNOW=T makes the input lai already sigf-scaled in CoLMMAIN.
   ! VIC runs with F=1 inside the vegetation tile, so restore tile LAI.
   real(r8) :: lai_perveg

      ! Initialize phase-change tracer outputs (default: no phase change).
      ldew_smelt_out        = 0._r8
      ldew_frzc_out         = 0._r8
      canopy_phase_heat_out = 0._r8

      ! Original VIC evaluates interception inside the vegetation tile with
      ! F=1. CoLM applies patch/PFT area weights outside this routine.
      IF (lai+sai > 1e-6) THEN
         lsai   = lai + sai
         vegt   = lsai
         ! Guard restart/float-noise negative canopy state.
         ldew_rain = max(0._r8, ldew_rain)
         ldew_snow = max(0._r8, ldew_snow)

         ! Clamp precipitation inputs once at scheme entry.
         rain_clamp = MAX(0.0_r8, prc_rain + prl_rain + qflx_irrig_sprinkler)
         snow_clamp = MAX(0.0_r8, prc_snow + prl_snow)

         ! Ensure ldew is consistent with components at entry.
         ! VIC sets ldew = ldew_rain + ldew_snow at exit; inconsistency at entry
         ! from initialization or restart causes mass balance check to fail
         ldew = ldew_rain + ldew_snow

         ! VIC applies the 1 mm liquid cleanup only in snow_intercept(),
         ! not in the warm rain-only canopy_evap() path. Snapshot the snow
         ! context before phase change can melt the last intercepted snow.
         vic_snow_intercept_context = &
            (fsno > 1.e-10_r8 .OR. ldew_snow > 1.e-10_r8 .OR. snow_clamp > 1.e-10_r8)

         ! Default DEF_VEG_SNOW=F already supplies vegetation-tile LAI. When
         ! enabled, undo CoLM's snow-burial scaling for faithful VIC F=1.
         IF (DEF_VEG_SNOW) THEN
            lai_perveg = lai / max(sigf, 1.e-10_r8)
         ELSE
            lai_perveg = lai
         ENDIF

         ! Capacities follow original VIC (vic_run.c:222-223, initialize_parameters.c:45):
         !   Wdmax = LAI * VEG_LAI_WATER_FACTOR,  VEG_LAI_WATER_FACTOR = 0.1 mm/LAI
         ! LAI only (not LAI+SAI) — stems do not hold canopy water in VIC.
         Imax1  = 4.0 * lai_perveg * 0.0005 * 1000.0   ! Structural max (branch) capacity [mm]
         MaxInt = 0.1_r8 * lai_perveg ! Wet canopy capacity (VIC Wdmax) [mm]
         ! VIC Lr (LAI ratio for snow capacity) temperature dependence
         ! Thresholds are in Kelvin: 272.15 K = -1°C, 270.15 K = -3°C
         ! Formula design: Lr = 4.0 at -1°C, Lr = 1.0 at -3°C, linear in between
         ! Reference: VIC snow_intercept.c, Storck et al. 2002
         ! BUG-FIX: previous version used -272.15/-270.15 (sign error), which
         !          forced Lr to always equal 4.0 for any physical canopy temperature
         IF (tleaf > 272.15_r8) THEN
            Lr = 4.0_r8
         ELSEIF (tleaf >= 270.15_r8) THEN
            Lr = 1.5_r8*(tleaf - 273.15_r8) + 5.5_r8
         ELSE
            Lr = 1.0_r8
         ENDIF

         ! Snow interception capacity, VIC snow_intercept.c:161
         !   MaxSnowInt = VEG_LAI_SNOW_MULTIPLIER * LAI * Lr
         ! LAI only (not LAI+SAI) — consistent with VIC original.
         ! Use vegetation-tile LAI (see above).
         satcap_snow = 0.0005 * Lr * lai_perveg * 1000.0  ! [mm]
         !/* Calculate total liquid water capacity on branches and in intercepted snow */
         ! VIC physical design: Total liquid water capacity includes two components:
         ! 1. Liquid water held in snow matrix (mature/ripe snow at 0°C)
         ! 2. Liquid water on leaf surfaces
         !
         ! Physical basis: Intercepted snow is a porous medium that can retain liquid water
         ! in its interstitial spaces when it reaches 0°C (mature/ripe snow state).
         ! This is a fundamental concept in snow hydrology (Colbeck 1972, Jordan 1991).
         !
         ! Formula: satcap_rain = SNOW_LIQUID_WATER_CAPACITY * ldew_snow + MaxInt
         ! - Term 1 (0.035*ldew_snow): Irreducible liquid water content in snow matrix
         !   The 0.035 coefficient represents typical irreducible water saturation (~3.5% by mass)
         !   This same parameter is used for ground snowpack in VIC (snow_melt.c, ice_melt.c)
         ! - Term 2 (MaxInt=0.1*lsai): Liquid water on leaf/branch surfaces
         !
         ! Physical meaning: When intercepted snow becomes ripe (0°C), it can simultaneously hold:
         ! - Solid ice framework (ldew_snow)
         ! - Liquid water in pore spaces (up to 3.5% of snow mass)
         ! - Additional liquid water on vegetation surfaces (MaxInt)
         ! When snow completely melts (ldew_snow=0), capacity reverts to leaf-only (MaxInt)
         !
         ! Reference: Andreadis et al. (2009) "Modeling snow accumulation and ablation
         ! processes in forested environments", WRR, doi:10.1029/2008WR007042
         !
         ! Rain capacity = snow matrix water retention + leaf surface capacity
         ! When snow melts completely (ldew_snow→0), capacity reverts to just MaxInt
         satcap_rain = 0.035 * ldew_snow + MaxInt  ! in mm

         ! Input clamping: prevent negative precipitation (numerical noise)
         ! from causing mass balance failures.
         p0  = (rain_clamp + snow_clamp) * deltim
         w = ldew+p0

         xsc_rain   = max(0., ldew_rain-satcap_rain)
         ! VIC's temperature-dependent MaxSnowInt limits only NEW snow
         ! interception; it is not a hard cap on existing canopy snow.
         ! Existing snow is released only by the separate Imax1 structural
         ! overload (or the thin-storage/energy-balance paths).
         xsc_snow   = 0._r8

         ldew_rain  = ldew_rain-xsc_rain
         ! phase change and excess
         ! Absorb fusion heat into tleaf to prevent the silent energy loss
         ! that previously biased tleaf warm (during melt) and inflated
         ! downstream Ec.
         !
         ! NOTE — APPROXIMATION, NOT FAITHFUL TO UPSTREAM VIC:
         ! Original VIC (snow_intercept.c L413-498) handles canopy phase
         ! change via solve_canopy_energy_bal: a root_brent solve on
         ! Tfoliage that yields RefreezeEnergy, which in turn drives
         ! PotSnowMelt and refreeze of liquid water. The phase event is
         ! fully embedded in the canopy energy balance, with Tfoliage
         ! pulled toward 0°C only when RefreezeEnergy < 0 (intercepted
         ! snow ripe).
         !
         ! Reproducing that path in CoLM requires embedding a canopy
         ! energy balance inside LEAF_interception_VIC, which is outside
         ! the current architecture (energy balance is in
         ! MOD_LeafTemperature). As a pragmatic substitute we apply the
         ! Niu et al. (2004) pull-toward-freezing already used in
         ! MOD_LeafTemperature.F90:1302. Bulk Ec effect is similar —
         ! fusion energy no longer disappears — but the per-step tleaf
         ! trajectory during phase events is a CoLM approximation, not
         ! original VIC.
         IF (tleaf > tfrz) THEN
            IF (ldew_snow>1.e-8) THEN
               ldew_smelt = MIN(ldew_snow,(tleaf-tfrz)*CICE*ldew_snow/DENICE/(HFUS))
               ldew_smelt = MAX(ldew_smelt,0.0)
               ldew_snow  = ldew_snow-ldew_smelt
               ldew_rain  = ldew_rain+ldew_smelt
               ! Accumulate vegetation-tile melt mass.
               ldew_smelt_out = ldew_smelt_out + ldew_smelt
               ! Vegetation-tile fusion heat:
               canopy_phase_heat_out = canopy_phase_heat_out - HFUS * ldew_smelt / deltim
               xsc_rain   = xsc_rain  + MAX(0., ldew_rain-satcap_rain)
               ldew_rain  = ldew_rain - MAX(0., ldew_rain-satcap_rain)
               ! VIC's RefreezeEnergy / solve_canopy_energy_bal path is
               ! emulated by exporting the bulk fusion flux
               ! canopy_phase_heat_out. LeafTemperature's qmelt/qfrz block
               ! is gated off for scheme=6 to prevent double-site phase
               ! change.
            ENDIF
         ELSE
            IF (ldew_rain>1.e-8) THEN
               ldew_frzc  = MIN(ldew_rain,(tfrz-tleaf)*CWAT*ldew_rain/DENH2O/(HFUS))
               ldew_frzc  = MAX(ldew_frzc,0.0)
               ldew_snow  = ldew_snow+ldew_frzc
               ldew_rain  = ldew_rain-ldew_frzc
               ! Accumulate vegetation-tile freeze mass.
               ldew_frzc_out = ldew_frzc_out + ldew_frzc
               canopy_phase_heat_out = canopy_phase_heat_out + HFUS * ldew_frzc / deltim
            ENDIF
         ENDIF

         ! satcap_rain depends on ldew_snow (snow-matrix water retention =
         ! 0.035 * ldew_snow + MaxInt). The initial value was computed from
         ! pre-phase ldew_snow; if the phase-change block just melted or
         ! froze a significant mass, satcap_rain is stale. Recompute and
         ! drain any new oversaturation into xsc_rain so downstream
         ! interception (Rutter/capacity-based) sees a consistent capacity.
         satcap_rain = 0.035_r8 * ldew_snow + MaxInt
         IF (ldew_rain > satcap_rain) THEN
            xsc_rain  = xsc_rain + (ldew_rain - satcap_rain)
            ldew_rain = satcap_rain
         ENDIF

         ! Thin-storage throughfall cutoff (VIC snow_intercept.c L207-210,
         ! L239-242, threshold SNOW_MIN_SWQ_EB_THRES = 0.001 m = 1.0 mm
         ! per vegetation). Without this, micro canopy storage (< 1.0 mm)
         ! lingers for many timesteps and keeps contributing to canopy
         ! evaporation demand, inflating Ec. VIC explicitly drops such
         ! amounts back as throughfall when no fresh precipitation is
         ! arriving. Route through xsc_* (pre-mix release) to preserve
         ! tracer semantics: these are OLD canopy waters released BEFORE
         ! new precipitation mixes in.
         ! Storage is already in VIC vegetation-tile coordinates, so the
         ! 1.0 mm threshold matches the upstream definition directly.
         IF (snow_clamp < 1.e-8_r8 .AND. ldew_snow < 1.0_r8 .AND. ldew_snow > 0._r8) THEN
            xsc_snow  = xsc_snow + ldew_snow
            ldew_snow = 0._r8
         ENDIF
         IF (vic_snow_intercept_context .AND. rain_clamp < 1.e-8_r8 &
             .AND. ldew_rain < 1.0_r8 .AND. ldew_rain > 0._r8) THEN
            xsc_rain  = xsc_rain + ldew_rain
            ldew_rain = 0._r8
         ENDIF

         ! Note: ldew will be resynced as ldew = ldew_rain + ldew_snow at output (line ~1806)
         ! No in-place ldew update needed here (CoLM2014 pattern: resync at end)

         ! Default to 0; the positive-precipitation branch updates.
         actual_rain_int = 0._r8
         actual_snow_int = 0._r8

         IF (p0 > 0._r8) THEN
            ! VIC physical interception algorithm (snow_intercept.c lines 165-176, 224-236)
            ! Snow: Dynamic capacity-based model
            ! Rain: VIC capacity-based interception

            ! Snow interception: VIC physical algorithm
            ! Interception efficiency decreases as canopy snow load approaches capacity
            ! This prevents unphysical continuous interception when branches are saturated
            Snow = snow_clamp*deltim
            IF (satcap_snow > 1.e-6 .and. Snow > 1.e-8) THEN
               ! DeltaSnowInt = (1 - IntSnow/MaxSnowInt) * SnowFall
               ! Physical meaning: Interception efficiency = available capacity / max capacity
               DeltaSnowInt = (1.0 - ldew_snow/satcap_snow) * Snow

               ! Ensure intercepted amount doesn't exceed available capacity
               IF (DeltaSnowInt + ldew_snow > satcap_snow) THEN
                  DeltaSnowInt = satcap_snow - ldew_snow
               ENDIF

               ! Ensure non-negative
               IF (DeltaSnowInt < 0.0) THEN
                  DeltaSnowInt = 0.0
               ENDIF

               ! BlownSnow strictly per VIC snow_intercept.c:189-195 — acts
               ! on NEW interception (DeltaSnowInt) only, not on historically
               ! accumulated canopy snow (ldew_snow). Previously CoLM applied
               ! BlownSnow to ldew_snow AFTER interception + overflow,
               ! unphysically shedding multi-day accumulated snow in one
               ! gust. Now matches the original sequence: compute
               ! DeltaSnowInt, subtract wind-blown fraction, then check
               ! Imax1 and update ldew_snow. VIC uses foliage temperature and
               ! wind attenuated to the overstory, not reference-height wind.
               Wind = vic_canopy_wind_speed(SQRT(forc_us*forc_us + forc_vs*forc_vs), z0m, hu)
               IF (tleaf-tfrz < -3.0_r8 .and. Wind > 1.0_r8 .and. DeltaSnowInt > 0._r8) THEN
                  BlownSnow = (0.2_r8*Wind - 0.2_r8) * DeltaSnowInt
                  BlownSnow = min(DeltaSnowInt, BlownSnow)
                  DeltaSnowInt = DeltaSnowInt - BlownSnow
               ENDIF

               ! Structural Imax1 pre-rejection (snow_intercept.c:199-201).
               ! If the canopy cannot bear (current snow + this-step
               ! intercept), refuse ALL new interception — all falls as
               ! throughfall this step. This is separate from the post-
               ! interception structural overloading check below which
               ! redistributes once capacity is already hit.
               IF (ldew_snow + DeltaSnowInt > Imax1) THEN
                  DeltaSnowInt = 0._r8
               ENDIF
            ELSE
               DeltaSnowInt = 0.0
            ENDIF

            ! VIC throughfall inside the vegetation tile (F=1).
            ! After the BlownSnow split fix: wind-blown portion has already been subtracted
            ! from DeltaSnowInt, so it now correctly flows into tti_snow
            ! (gap throughfall carrying R_input signature), matching
            ! snow_intercept.c:204 semantics. Previously it flowed through
            ! tex_snow (canopy drip at R_mixed) which was tracer-inaccurate.
            tti_snow = Snow - DeltaSnowInt
            ldew_snow = ldew_snow + DeltaSnowInt
            ! DeltaSnowInt (post-BlownSnow, post-Imax1 rejection) is the
            ! snow mass actually admitted into the canopy mixed
            ! pool this step.
            actual_snow_int = DeltaSnowInt

            ! Rain interception: Original VIC capacity-based algorithm
            ! Physical mechanism: Rain is intercepted based on available canopy storage capacity,
            ! not a fixed efficiency function. When capacity is available, rain is intercepted;
            ! when saturated, excess drains as throughfall.
            ! Reference: Andreadis et al. (2009) WRR, VIC snow_intercept.c lines 218-236
            ! This differs from CLM5's empirical efficiency approach (tanh function)
            Rain = rain_clamp * deltim

            ! Recompute MaxWaterInt using the POST-snow-interception ldew_snow.
            ! Original VIC (snow_intercept.c:213 then L222) adds the newly
            ! intercepted snow to IntSnow FIRST, then computes MaxWaterInt.
            ! Previously CoLM used satcap_rain computed pre-snow-interception,
            ! which undercounts the 0.035*DeltaSnowInt contribution of the
            ! fresh snow matrix to liquid-water retention capacity.
            MaxWaterInt = 0.035_r8 * ldew_snow + MaxInt

            ! Capacity-based interception (VIC original algorithm).
            ! If there is available capacity, intercept rain; otherwise it
            ! becomes throughfall. Record actual mass admitted into
            ! the canopy pool (used later for gross_intr_rain).
            IF (ldew_rain + Rain <= MaxWaterInt) THEN
               ! All rain can be intercepted (capacity not exceeded)
               actual_rain_int = Rain
               ldew_rain = ldew_rain + Rain
               tti_rain = 0._r8
            ELSE
               ! Capacity exceeded: excess becomes throughfall
               actual_rain_int = max(0._r8, MaxWaterInt - ldew_rain)
               tti_rain = ldew_rain + Rain - MaxWaterInt
               ! Storage saturated at maximum capacity
               ldew_rain = MaxWaterInt
            ENDIF

            tex_rain    = max(0.0,ldew_rain-MaxWaterInt)
            tex_snow    = 0._r8

            ldew_rain   = ldew_rain - tex_rain

            ! BlownSnow moved into the snow-interception block above
            ! (snow_intercept.c:189-195 original placement). It now reduces
            ! DeltaSnowInt BEFORE the new mass enters the canopy pool, so
            ! historically accumulated ldew_snow is NOT shed by a single
            ! gust. Previous location (here, post-overflow) incorrectly
            ! used ldew_snow as the base.

         ELSE
            ! No precipitation this step; existing storage is handled below.
            tti_rain = 0.
            tti_snow = 0.
            tex_rain = 0.
            tex_snow = 0.
         ENDIF ! end precipitation-dependent interception

         ! MaxSnowInt is only an interception-efficiency scale. Existing
         ! canopy snow is unloaded only when the total rain+snow load exceeds
         ! VIC's structural bearing capacity Imax1. Keep this outside the
         ! precipitation branch so overload is also handled in dry timesteps.
         IF (ldew_rain + ldew_snow > Imax1) THEN
            Overload = (ldew_snow + ldew_rain) - Imax1
            IntRainFract = ldew_rain / (ldew_rain + ldew_snow)
            IntSnowFract = 1._r8 - IntRainFract
            ldew_rain = ldew_rain - Overload * IntRainFract
            ldew_snow = ldew_snow - Overload * IntSnowFract
            tex_rain  = tex_rain  + Overload * IntRainFract
            tex_snow  = tex_snow  + Overload * IntSnowFract
         ENDIF

! NOTE: The check "tex+tti > p0" is not applicable to VIC scheme.
! VIC's tex includes drainage of pre-existing canopy water (ldew) from
! capacity overflow and structural overloading. Additionally, tti includes
! bare-fraction precipitation. The real mass balance check is below.


         thru_rain = tti_rain + tex_rain
         thru_snow = tti_snow + tex_snow

         ! VIC safety check: When snow completely melts, liquid water capacity
         ! reverts from (0.035*ldew_snow + MaxInt) to just (MaxInt)
         ! Must drain excess water that can no longer be held
         ! Reference: VIC snow_intercept.c lines 522-526
         !
         ! This safety drain happens AFTER actual_rain_int (new rain) has
         ! been injected into ldew_rain and AFTER Imax1 structural overflow,
         ! so ldew_rain here is a POST-MIX pool (fresh interception + any
         ! canopy melt water already blended in). Routing the excess through
         ! xsc_rain would make tracer_precip tag it with R_canopy_pre
         ! (pre-mix canopy signature), but physics says it should carry
         ! R_mixed because the pool is already mixed. Route through
         ! thru_rain instead — tracer_precip derives drip =
         ! max(intercepted - d_ldew, 0) from the ldew_rain change, so the
         ! excess is automatically classified as post-mix drip at R_mixed.
         ! The excess liquid is released directly within the vegetation tile.
         IF (ldew_snow < 1.e-6 .and. ldew_rain > MaxInt) THEN
            thru_rain = thru_rain + (ldew_rain - MaxInt)
            ldew_rain = MaxInt
         ENDIF

         ! Update total vegetation-tile canopy water storage.
         ldew = ldew_rain + ldew_snow
         pinf = p0 - (thru_rain + thru_snow)

         pg_rain = (xsc_rain + thru_rain) / deltim
         pg_snow = (xsc_snow + thru_snow) / deltim
         qintr   = pinf / deltim

         qintr_rain = rain_clamp - (thru_rain / deltim)
         qintr_snow = snow_clamp - (thru_snow / deltim)

         ! gross_intr_* must be the ACTUAL mass entering the canopy mixed
         ! pool, otherwise tracer_precip (MOD_Tracer_Precip.F90:170,181,182)
         ! misclassifies gap throughfall as canopy drip at R_mixed. Snow
         ! uses DeltaSnowInt (post-BlownSnow, post-Imax1 rejection); rain
         ! uses the capacity-limited inflow from the VIC branch above.
         gross_intr_rain = actual_rain_int / deltim
         gross_intr_snow = actual_snow_int / deltim

         ! Pre-mix old-pool release. These accumulate the initial capacity
         ! overflow plus any
         ! phase-change overflow, all of which flushed OLD canopy water
         ! before new precip mixed in. tex_rain/BlownSnow/Overload are
         ! post-mix drip and flow through pg_rain at R_mixed in tracer.
         xsc_rain_out = xsc_rain / deltim
         xsc_snow_out = xsc_snow / deltim

#if (defined CoLMDEBUG)
         w = w - ldew - (pg_rain+pg_snow)*deltim
         IF (abs(w) > INTERCEPTION_BALANCE_TOL) THEN
            write(6,*) 'something wrong in interception code : '
            write(6,*) w, ldew, (pg_rain+pg_snow)*deltim !, satcap
            CALL abort
         ENDIF

         CALL check_interception_balance('VIC', &
              ldew, ldew_rain, ldew_snow, pg_rain, pg_snow, &
              qintr, qintr_rain, qintr_snow)
#endif

      ELSE
         ! No vegetation: all precipitation passes through, release any
         ! stored water.
         ! Clamp raw precipitation to prevent negative noise from propagating
         ! to pg_* in the no-vegetation branch, matching the in-branch clamp
         ! and JULES's no-veg treatment.
         ! Preserve pre-existing canopy water signature for tracer attribution.
         ! The ldew_rain / ldew_snow released here is OLD canopy water
         ! (R_canopy_pre), not fresh throughfall (R_input). Route it through
         ! xsc_*_out BEFORE ldew is reset so tracer_precip classifies it
         ! correctly (otherwise it would be lumped into throughfall and diluted
         ! by fresh precip signature).
         CALL release_no_vegetation_canopy_storage(deltim,tleaf, &
              prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler, &
              ldew,ldew_rain,ldew_snow,pg_rain,pg_snow, &
              qintr,qintr_rain,qintr_snow,gross_intr_rain,gross_intr_snow, &
              xsc_rain_out,xsc_snow_out)
      ENDIF
   END SUBROUTINE LEAF_interception_VIC

   FUNCTION vic_canopy_wind_speed(forcing_wind, z0m, wind_height) RESULT(canopy_wind)
      IMPLICIT NONE

      real(r8), intent(in) :: forcing_wind, z0m, wind_height
      real(r8) :: canopy_wind
      real(r8), parameter :: z0m_height_ratio = 0.1_r8
      real(r8), parameter :: displacement_ratio = 0.667_r8
      real(r8), parameter :: wind_attenuation = 0.5_r8
      real(r8) :: canopy_height, displacement, reference_height
      real(r8) :: zw, log_reference, uw, uh, attenuation

      canopy_wind = max(0._r8, forcing_wind)
      IF (canopy_wind <= 0._r8 .or. z0m <= 1.e-8_r8 .or. wind_height <= 0._r8) RETURN

      ! CoLM uses z0m/height=0.1 and displacement/height=0.667. Apply the
      ! VIC CalcAerodynamic overstory profile with its default attenuation.
      canopy_height = z0m / z0m_height_ratio
      displacement = displacement_ratio * canopy_height
      IF (displacement < wind_height) THEN
         reference_height = wind_height
      ELSE
         reference_height = displacement + wind_height + z0m
      ENDIF

      zw = 1.5_r8 * canopy_height - 0.5_r8 * displacement
      log_reference = log((reference_height - displacement) / z0m)
      IF (log_reference <= 1.e-8_r8) RETURN

      uw = log((zw - displacement) / z0m) / log_reference
      uh = uw - (1._r8 - (canopy_height - displacement) / &
           (zw - displacement)) / log_reference
      attenuation = uh * exp(wind_attenuation * &
           ((z0m + displacement) / canopy_height - 1._r8))
      canopy_wind = canopy_wind * min(1._r8, max(0._r8, attenuation))
   END FUNCTION vic_canopy_wind_speed

   SUBROUTINE LEAF_interception_JULES(deltim,dewmx,forc_us,forc_vs,chil,sigf,lai,sai,tair,tleaf, &
                                       prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler,&
                                       ldew,ldew_rain,ldew_snow,z0m,hu,pg_rain,pg_snow,qintr,qintr_rain,qintr_snow,&
                                       gross_intr_rain,gross_intr_snow,&
                                       xsc_rain_out,xsc_snow_out,&
                                       ldew_smelt_out,ldew_frzc_out,&
                                       canopy_phase_heat_out)
   !DESCRIPTION
   !===========
      ! Official JULES canopy interception scheme
      ! Rain: Rutter (1971) penetration model (sieve_jls_mod.F90)
      ! Snow: Exponential saturation model with unloading (canopysnow_mod.F90)
      !
      ! 2026-02 Fixes:
      ! - Added vegetation fraction (sigf) scaling: interception occurs on vegetated area only.
      ! - Added non-negative clamping for precipitation inputs to ensure mass balance.
      ! - Fixed mass balance check: ldew resync before sigf division (VIC pattern).

   !Original Author:
   !-------------------
      !---Rutter et al. (1971, 1975) - Rain interception model
      !---JULES team (Best et al. 2011) - Snow interception model
      !---Zhongwang Wei @ SYSU - CoLM implementation

   !References:
   !-------------------
      !---Rutter et al. (1971): A predictive model of rainfall interception in forests, 1.
      !   Derivation of the model from observations in a plantation of Corsican pine.
      !   Agricultural Meteorology, 9, 367-384.
      !---Rutter et al. (1975): A predictive model of rainfall interception in forests, 2.
      !   Generalization of the model and comparison with observations in some coniferous
      !   and hardwood stands. Journal of Applied Ecology, 12, 367-380.
      !---Best et al. (2011): The Joint UK Land Environment Simulator (JULES), model description -
      !   Part 1: Energy and water fluxes. Geosci. Model Dev. 4:677-699.
      !---Clark et al. (2011): The Joint UK Land Environment Simulator (JULES), model description -
      !   Part 2: Carbon fluxes and vegetation dynamics. Geosci. Model Dev. 4:701-722.

   !ANCILLARY FUNCTIONS AND SUBROUTINES
   !-------------------

   !REVISION HISTORY
   !----------------
      ! 2026.02.11  Zhongwang Wei @ SYSU - Added sigf scaling, input clamping, mass balance fix
      ! 2026.01.16  Zhongwang Wei @ SYSU - Converted to official JULES Rutter model
      ! 2023.02.21  Zhongwang Wei @ SYSU
      ! 2021.12.08  Zhongwang Wei @ SYSU
   !=======================================================================

   IMPLICIT NONE

   real(r8), intent(in)    :: deltim     !seconds in a time step [second]
   real(r8), intent(in)    :: dewmx      !maximum dew [mm] (unused in JULES; retained for interface compatibility)
   real(r8), intent(in)    :: forc_us    !wind speed [m/s]
   real(r8), intent(in)    :: forc_vs    !wind speed [m/s]
   real(r8), intent(in)    :: chil       !leaf angle distribution factor (unused in JULES)
   real(r8), intent(in)    :: prc_rain   !convective rainfall [mm/s]
   real(r8), intent(in)    :: prc_snow   !convective snowfall [mm/s]
   real(r8), intent(in)    :: prl_rain   !large-scale rainfall [mm/s]
   real(r8), intent(in)    :: prl_snow   !large-scale snowfall [mm/s]
   real(r8), intent(in)    :: qflx_irrig_sprinkler !irrigation and sprinkler water flux [mm/s]
   real(r8), intent(in)    :: sigf       !fraction of veg cover, excluding snow-covered veg [-]
   real(r8), intent(in)    :: lai        !leaf area index [-]
   real(r8), intent(in)    :: sai        !stem area index [-]
   real(r8), intent(in)    :: tair       !air temperature [K]
   real(r8), intent(inout) :: tleaf      !sunlit canopy leaf temperature [K] (modified by NM-1 fix: absorbs fusion heat during canopy phase change)

   real(r8), intent(inout) :: ldew       !depth of water on foliage [mm]
   real(r8), intent(inout) :: ldew_rain  !depth of liquid on foliage [mm]
   real(r8), intent(inout) :: ldew_snow  !depth of solid on foliage [mm]
   real(r8), intent(in)    :: z0m        !roughness length (unused in JULES)
   real(r8), intent(in)    :: hu         !forcing height of U (unused in JULES)

   real(r8), intent(out)   :: pg_rain    !rainfall onto ground including canopy runoff [kg/(m2 s)]
   real(r8), intent(out)   :: pg_snow    !snowfall onto ground including canopy runoff [kg/(m2 s)]
   real(r8), intent(out)   :: qintr      !interception [kg/(m2 s)]
   real(r8), intent(out)   :: qintr_rain !rainfall interception (mm h2o/s) [NET storage-related flux; can be <0 when canopy drains faster than new rain intercepts. Use gross_intr_rain for gross interception]
   real(r8), intent(out)   :: qintr_snow !snowfall interception (mm h2o/s) [NET storage-related flux; can be <0 (e.g. snow unloading/blowing with no new snow). Use gross_intr_snow for gross interception]
   real(r8), intent(out)   :: gross_intr_rain !gross rain entering canopy mixed pool (mm h2o/s, >=0)
   real(r8), intent(out)   :: gross_intr_snow !gross snow entering canopy mixed pool (mm h2o/s, >=0)
   real(r8), intent(out)   :: xsc_rain_out    !pre-mix rain release rate from old canopy pool (mm h2o/s, >=0)
   real(r8), intent(out)   :: xsc_snow_out    !pre-mix snow release rate from old canopy pool (mm h2o/s, >=0)
   ! Phase-change tracer transfer (grid-scale mm, >=0). JULES's phase
   ! change runs in per-veg coordinates; per-veg contributions accumulate
   ! below and are scaled to grid-scale with sigf_safe at the end of the
   ! vegetated branch (mirroring how ldew_rain/ldew_snow are rescaled).
   real(r8), intent(out)   :: ldew_smelt_out  !canopy snow->rain mass transferred by melt this step [mm, >=0, grid-scale]
   real(r8), intent(out)   :: ldew_frzc_out   !canopy rain->snow mass transferred by freeze this step [mm, >=0, grid-scale]
   ! Canopy phase-change fusion heat flux [W/m^2, grid-scale]. Accumulated
   ! per-veg and scaled to grid-scale with sigf_safe at the end of the
   ! vegetated branch. See NoahMP block for sign convention. JULES's
   ! original canopysnow_mod imports melt_surft externally (computed in
   ! surface_flux); here we instead export the bulk fusion flux so
   ! MOD_Thermal's canopy energy balance closes at CoLM granularity.
   real(r8), intent(out)   :: canopy_phase_heat_out

   ! Local variables
   real(r8)                :: snowinterceptfact    ! Snow interception efficiency (0.7)
   real(r8)                :: snowunloadfact       ! Snow unloading factor due to melt (0.4)
   real(r8)                :: unload_backgrnd      ! Total background unloading rate [s⁻¹]
   real(r8)                :: area                 ! Precipitation area fraction for current event
   real(r8)                :: can_cpy_rain         ! Canopy capacity for rain [mm]
   real(r8)                :: can_cpy_snow         ! Canopy capacity for snow [mm]
   real(r8)                :: r_rain               ! Total rain rate [mm/s] (clamped, non-negative)
   real(r8)                :: r_rain_ls            ! Large-scale rain + sprinkler [mm/s]
   real(r8)                :: r_rain_con           ! Convective rain [mm/s]
   real(r8)                :: r_snow               ! Snow rate [mm/s] (clamped, non-negative)
   real(r8)                :: can_ratio            ! Canopy saturation ratio (can_wcnt/can_cpy)
   real(r8)                :: aexp                 ! Exponential term in Rutter model
   real(r8)                :: tfall_rain           ! Rain throughfall [mm/s]
   real(r8)                :: tfall_rain_ls        ! Large-scale rain throughfall [mm/s]
   real(r8)                :: tfall_rain_con       ! Convective rain throughfall [mm/s]
   real(r8)                :: tfall_snow           ! Snow throughfall [mm/s]
   real(r8)                :: intercept_rain       ! Rain interception in timestep [mm]
   real(r8)                :: intercept_snow       ! Snow interception in timestep [mm]
   real(r8)                :: unload_snow          ! Snow unloading in timestep [mm]
   real(r8)                :: ldew_snow_pre        ! Pre-unload canopy snow store [mm]
   real(r8)                :: melt_rate            ! Canopy snow melt rate [mm/s]
   real(r8)                :: melt_factor          ! Dimensionless melt energy ratio: CICE/(DENICE*HFUS)
   real(r8)                :: frz_factor           ! Dimensionless freeze energy ratio: CWAT/(DENH2O*HFUS)
   real(r8)                :: smallp               ! Small positive number
   real(r8)                :: p0_l                 ! total precipitation (local)
   real(r8)                :: w_l                   ! mass balance check variable (local)
   real(r8)                :: ldew_frzc            ! freezing water amount
   real(r8)                :: xsc_rain, xsc_snow   ! excess water drained after phase change
   real(r8)                :: sigf_safe            ! safe vegetation fraction (>= 0.01)
   real(r8)                :: thru_rain, thru_snow ! grid-scale throughfall [mm]
   real(r8)                :: can_cpy_snow_pv      ! per-veg snow capacity for fwet_snow tleaf pull
   real(r8)                :: fwet_snow            ! Canopy wet fraction for fusion-heat tleaf pull
   ! DEF_VEG_SNOW=T makes the input lai pre-multiplied by sigf
   ! (CoLMMAIN.F90:2083/2099). JULES physics operates per-veg
   ! (ldew/sigf_safe), so restore per-veg LAI for capacity formulas.
   real(r8)                :: lai_perveg_J

      ! Initialize phase-change tracer outputs (default: no phase change).
      ldew_smelt_out        = 0._r8
      ldew_frzc_out         = 0._r8
      canopy_phase_heat_out = 0._r8

      ! JULES operates on per-vegetation storage. Same rationale as VIC above:
      ! sparse vegetation (0 < sigf < 0.01) would hit the sigf_safe floor and
      ! bias grid-scale throughfall. Route those patches to the no-veg branch.
      IF (lai+sai > 1e-6 .AND. sigf >= 0.01_r8) THEN
         !======================================================================
         ! Input Clamping (Mass Balance Safety)
         !======================================================================
         ! CoLM forcing supplies one total precipitation field and creates a
         ! synthetic 1/3 convective + 2/3 large-scale split. Recombine that
         ! total here, then use the same 10%/90% assumption as the other
         ! total-precipitation interception schemes. Sprinkler water remains
         ! in the spatially uniform large-scale component.
         r_rain_con = 0.1_r8 * MAX(0.0_r8, prc_rain + prl_rain)
         r_rain_ls  = 0.9_r8 * MAX(0.0_r8, prc_rain + prl_rain) &
                    + MAX(0.0_r8, qflx_irrig_sprinkler)
         r_rain = r_rain_ls + r_rain_con
         r_snow = MAX(0.0_r8, prc_snow + prl_snow)

         ! Clamp canopy state: negative values from restart or upstream bugs
         ! would be amplified by sigf division and cause mass balance abort
         ldew_rain = MAX(0.0_r8, ldew_rain)
         ldew_snow = MAX(0.0_r8, ldew_snow)

         !======================================================================
         ! JULES Parameters - Official values from JULES source code
         !======================================================================
         snowinterceptfact = 0.7       ! Snow interception efficiency (jules_snow_mod.F90)
         snowunloadfact    = 0.4       ! Snow unloading factor (canopysnow_mod.F90)
         ! Upstream JULES defaults both configurable background unloading
         ! coefficients to zero. Melt-driven unloading remains active below.
         unload_backgrnd   = 0.0_r8
         ! Canopy capacities follow original JULES pftparm defaults
         ! (rose-app.conf: catch0_io=0.5, dcatch_dlai_io=0.05, snowloadlai=4.4)
         !   rain: catch = catch0 + dcatch_dlai * LAI   (LAI only, not LAI+SAI)
         !   snow: catch = snowloadlai * LAI
         ! Derive per-veg LAI so capacity matches the per-veg ldew coordinate.
         ! DEF_VEG_SNOW=T pre-multiplied lai by sigf in CoLMMAIN.F90:2083/2099;
         ! divide back out here. Outer guard ensures sigf >= 0.01 here, so
         ! sigf_safe (computed below) equals sigf.
         IF (DEF_VEG_SNOW) THEN
            lai_perveg_J = lai / max(sigf, 0.01_r8)
         ELSE
            lai_perveg_J = lai
         ENDIF
         can_cpy_rain      = 0.5_r8 + 0.05_r8 * lai_perveg_J   ! Rain capacity [mm], JULES pftparm
         can_cpy_snow      = 4.4_r8 * lai_perveg_J             ! Snow capacity [mm], snowloadlai
         smallp = EPSILON(1.0_r8)      ! Machine epsilon for numerical stability

         !======================================================================
         ! Precipitation totals and mass balance reference (GRID-SCALE)
         !======================================================================
         ! Use clamped rates for consistency.
         p0_l  = (r_rain + r_snow) * deltim

         ! Ensure ldew is consistent with components at entry (GRID-SCALE)
         ! Must be done BEFORE sigf division to keep w_l in grid-scale units
         ldew = ldew_rain + ldew_snow

         ! Mass balance reference: grid-scale storage + grid-scale precipitation
         w_l = ldew + p0_l

         !======================================================================
         ! Vegetation Fraction Scaling (sigf)
         !======================================================================
         ! JULES physics operates on the vegetated area only.
         ! Convert grid-averaged storage to per-vegetation values.
         ! (Matching VIC pattern: divide before physics, multiply after)
         ! Note: outer guard guarantees sigf > 1e-6; floor at 0.01 prevents
         ! extreme amplification when sigf is very small but positive.
         sigf_safe = max(sigf, 0.01_r8)
         ldew_rain = ldew_rain / sigf_safe
         ldew_snow = ldew_snow / sigf_safe

         !======================================================================
         ! Phase change (melting/freezing) - Do BEFORE interception
         !======================================================================
         ! Pre-compute dimensionless energy ratios to avoid large intermediate
         ! products (e.g. CICE*ldew_snow ~1e8) that can trap under -ffpe-trap.
         ! melt_factor = CICE / (DENICE * HFUS) ≈ 0.00684 [K⁻¹]
         ! frz_factor  = CWAT / (DENH2O * HFUS) ≈ 0.01256 [K⁻¹]
         melt_factor = CICE / (DENICE * HFUS)
         frz_factor  = CWAT / (DENH2O * HFUS)

         ! Absorb fusion heat into tleaf to prevent the silent energy loss
         ! that previously biased tleaf warm (during melt) and inflated
         ! downstream Ec.
         !
         ! NOTE — APPROXIMATION, NOT FAITHFUL TO UPSTREAM JULES:
         ! Original JULES does NOT do canopy phase change inside the
         ! interception modules (sieve_jls_mod.F90 / canopysnow_mod.F90).
         ! Instead, canopysnow_mod RECEIVES an externally-computed
         ! melt_surft from the surface_flux scheme (which solves the
         ! canopy energy balance separately) and only USES it for snow
         ! unloading: unload = snowunloadfact * melt_surft * dt + ...
         ! Canopy temperature (tstar_tile) is updated in surface_flux,
         ! not here.
         !
         ! Reproducing that path requires removing explicit melt/freeze
         ! from this routine and routing all canopy phase change through
         ! MOD_LeafTemperature. As a pragmatic substitute, since CoLM
         ! already does explicit melt/freeze here for ldew_smelt and
         ! ldew_frzc accounting, we apply the Niu et al. (2004)
         ! pull-toward-freezing already used in
         ! MOD_LeafTemperature.F90:1302 to keep fusion energy in the
         ! tleaf budget rather than silently lost. Bulk Ec effect is
         ! similar but the per-step tleaf trajectory during phase events
         ! is a CoLM approximation, not original JULES.
         !
         ! can_cpy_snow is per-veg snow capacity.
         can_cpy_snow_pv = max(can_cpy_snow, 1.e-10_r8)
         IF (tleaf > tfrz) THEN
            ! Canopy snow melting
            IF (ldew_snow > 1.e-8) THEN
               melt_rate = MIN(ldew_snow/deltim, &
                    (tleaf - tfrz) * melt_factor * ldew_snow / deltim)
               melt_rate = MAX(melt_rate, 0.0_r8)
               ldew_snow = ldew_snow - melt_rate * deltim
               ldew_snow = MAX(ldew_snow, 0.0_r8)  ! prevent -eps from FP rounding
               ldew_rain = ldew_rain + melt_rate * deltim
               ! Accumulate per-veg melt mass; scaled to grid-scale below.
               ldew_smelt_out = ldew_smelt_out + melt_rate * deltim
               ! Per-veg fusion heat; grid-scale conversion below.
               canopy_phase_heat_out = canopy_phase_heat_out - HFUS * melt_rate
               ! The canopy fusion heat is now exported via
               ! canopy_phase_heat_out so MOD_Thermal's energy balance
               ! closes. LeafTemperature's qmelt/qfrz block is gated off
               ! for scheme=7 so there is no second phase-change site.
            ELSE
               melt_rate = 0.0_r8
            ENDIF
         ELSE
            ! Canopy rain freezing
            IF (ldew_rain > 1.e-8) THEN
               ldew_frzc = MIN(ldew_rain, &
                    (tfrz - tleaf) * frz_factor * ldew_rain)
               ldew_frzc = MAX(ldew_frzc, 0.0_r8)
               ldew_snow = ldew_snow + ldew_frzc
               ldew_rain = ldew_rain - ldew_frzc
               ldew_rain = MAX(ldew_rain, 0.0_r8)  ! prevent -eps from FP rounding
               ! Accumulate per-veg freeze mass; scaled to grid-scale below.
               ldew_frzc_out = ldew_frzc_out + ldew_frzc
               canopy_phase_heat_out = canopy_phase_heat_out + HFUS * ldew_frzc / deltim
               ! Freeze branch (canopy gains fusion heat).
            ENDIF
            melt_rate = 0.0_r8
         ENDIF

         !======================================================================
         ! Drain excess water after phase change
         ! When snow melts to rain, ldew_rain can greatly exceed can_cpy_rain
         ! When rain freezes to snow, ldew_snow can exceed can_cpy_snow
         !======================================================================
         xsc_rain = 0.0
         xsc_snow = 0.0
         IF (ldew_rain > can_cpy_rain) THEN
            xsc_rain  = ldew_rain - can_cpy_rain
            ldew_rain = can_cpy_rain
         ENDIF
         IF (ldew_snow > can_cpy_snow) THEN
            xsc_snow  = ldew_snow - can_cpy_snow
            ldew_snow = can_cpy_snow
         ENDIF

         !======================================================================
         ! RAIN INTERCEPTION: Rutter (1971) Penetration Model
         ! From JULES sieve_jls_mod.F90 lines 125-142
         !======================================================================
         intercept_rain = 0.0_r8
         tfall_rain_ls  = r_rain_ls
         tfall_rain_con = r_rain_con

         IF (can_cpy_rain > 0.0_r8) THEN
            ! Upstream JULES calls sieve sequentially for condensation,
            ! large-scale rain, then convective rain. CoLM currently has
            ! no condensation input at this stage, but we preserve the
            ! LS -> CON canopy-filling order so mixed rain events no
            ! longer collapse into a single area-weighted call.
            IF (r_rain_ls > smallp) THEN
               area = 1.0_r8
               aexp = exp(max(-50.0_r8, -area * can_cpy_rain / (r_rain_ls * deltim)))
               can_ratio = ldew_rain / can_cpy_rain
               can_ratio = MAX(0.0_r8, MIN(can_ratio, 1.0_r8))
               tfall_rain_ls = r_rain_ls * ((1.0_r8 - can_ratio) * aexp + can_ratio)
               intercept_rain = intercept_rain + (r_rain_ls - tfall_rain_ls) * deltim
               ldew_rain = ldew_rain + (r_rain_ls - tfall_rain_ls) * deltim
            ENDIF

            IF (r_rain_con > smallp) THEN
               area = 0.3_r8
               aexp = exp(max(-50.0_r8, -area * can_cpy_rain / (r_rain_con * deltim)))
               can_ratio = ldew_rain / can_cpy_rain
               can_ratio = MAX(0.0_r8, MIN(can_ratio, 1.0_r8))
               tfall_rain_con = r_rain_con * ((1.0_r8 - can_ratio) * aexp + can_ratio)
               intercept_rain = intercept_rain + (r_rain_con - tfall_rain_con) * deltim
               ldew_rain = ldew_rain + (r_rain_con - tfall_rain_con) * deltim
            ENDIF
         ENDIF

         tfall_rain = tfall_rain_ls + tfall_rain_con

         ! Post-Rutter drainage: discrete timestep can overshoot capacity
         IF (ldew_rain > can_cpy_rain) THEN
            tfall_rain = tfall_rain + (ldew_rain - can_cpy_rain) / deltim
            ldew_rain  = can_cpy_rain
         ENDIF

         !======================================================================
         ! SNOW INTERCEPTION: Exponential Saturation Model with Unloading
         ! From JULES canopysnow_mod.F90 lines 131-145
         !======================================================================
         ! Upstream JULES computes intercept and unload from the same
         ! pre-interception snow_can state, then applies the net update once.
         ! Keep that parallel-update semantics here.
         ldew_snow_pre = ldew_snow
         unload_snow = snowunloadfact * melt_rate * deltim &
                     + unload_backgrnd * ldew_snow_pre * deltim
         unload_snow = MAX(MIN(unload_snow, ldew_snow_pre), 0.0)
         ! unload_snow is OLD canopy snow shaken off by wind/melt, released
         ! BEFORE any new snow mixes in. Route it through xsc_snow (per-veg,
         ! pre-mix) so tracer_precip (MOD_Tracer_Precip.F90:167) attributes
         ! it as R_canopy_pre rather than R_mixed canopy drip. Bulk pg_snow
         ! is unchanged: the offsetting subtraction is applied to
         ! tfall_snow below.
         xsc_snow = xsc_snow + unload_snow

         ! Guard on can_cpy_snow > smallp: with the JULES convention
         ! can_cpy_snow = 4.4*LAI, leafless patches (LAI=0, SAI>0) would
         ! trigger divide-by-zero at the EXP denominator below.
         IF (r_snow > smallp .AND. can_cpy_snow > smallp) THEN
            ! Snow interception (JULES lines 131-132), evaluated on the
            ! same pre-unload canopy snow state as unload_snow.
            intercept_snow = snowinterceptfact * (can_cpy_snow - ldew_snow_pre) * &
                             (1.0 - EXP(MAX(-50.0_r8, -r_snow * deltim / can_cpy_snow)))
            intercept_snow = MAX(0.0, intercept_snow)

            ! Apply intercept and unload together, matching upstream's
            ! parallel update on snow_can_pre.
            ldew_snow = ldew_snow_pre + intercept_snow - unload_snow

            ! Snowfall to ground = snowfall - intercepted
            ! Unloaded snow is accounted via xsc_snow (OLD canopy release,
            ! pre-mix), not tfall_snow. tfall_snow now holds only the
            ! R_input component (gap throughfall from fresh snowfall minus
            ! interception).
            tfall_snow = r_snow - intercept_snow / deltim

            ! Post-interception drainage
            IF (ldew_snow > can_cpy_snow) THEN
               tfall_snow = tfall_snow + (ldew_snow - can_cpy_snow) / deltim
               ldew_snow  = can_cpy_snow
            ENDIF
         ELSE
            intercept_snow = 0.0
            ldew_snow = ldew_snow_pre - unload_snow
            ! No snowfall; unloaded snow already routed to xsc_snow above.
            ! tfall_snow reflects only bare-gap path.
            tfall_snow = r_snow
         ENDIF
         ldew_snow = MAX(ldew_snow, 0.0_r8)

         !======================================================================
         ! Output fluxes: Per-vegetation → Grid-scale (VIC pattern)
         !======================================================================
         ! tfall_rain/tfall_snow are per-vegetation throughfall rates [mm/s]
         ! xsc_rain/xsc_snow are per-vegetation excess [mm]
         ! Combine: vegetated area throughfall + bare ground direct precipitation

         ! Grid-scale throughfall [mm] (for mass balance check)
         ! IMPORTANT: Use sigf_safe consistently (not sigf) to ensure exact mass balance
         ! When sigf < 0.01, sigf_safe = 0.01 != sigf, mixing them creates a residual
         thru_rain = (tfall_rain * deltim + xsc_rain) * sigf_safe + r_rain * deltim * (1.0 - sigf_safe)
         thru_snow = (tfall_snow * deltim + xsc_snow) * sigf_safe + r_snow * deltim * (1.0 - sigf_safe)

         ! Convert throughfall to rate [mm/s]
         pg_rain = thru_rain / deltim
         pg_snow = thru_snow / deltim

         ! Rescale state variables back to grid-averaged
         ldew_rain = ldew_rain * sigf_safe
         ldew_snow = ldew_snow * sigf_safe

         ! Update total canopy water (grid-scale)
         ldew = ldew_rain + ldew_snow

         ! Interception = total input - total output - storage change
         ! Using the VIC approach: qintr = (p0 - thru) / deltim
         qintr = (p0_l - thru_rain - thru_snow) / deltim

         ! Phase-separated interception rates
         ! NOTE: These can be NEGATIVE when pre-existing canopy storage drains
         ! (e.g., excess from phase change or prior timestep). Negative values
         ! represent net canopy release, not a mass balance error.
         ! Algebraic identity: qintr_rain + qintr_snow == qintr (exact).
         qintr_rain = r_rain - pg_rain
         qintr_snow = r_snow - pg_snow

         ! Gross interception rate (grid-scale): rain/snow that physically
         ! entered the canopy mixed pool. In JULES per-veg: intercept_rain
         ! and intercept_snow are the Rutter/exp-model captures before
         ! post-drainage, [mm] per deltim. Scale to grid with sigf_safe.
         gross_intr_rain = intercept_rain * sigf_safe / deltim
         gross_intr_snow = intercept_snow * sigf_safe / deltim

         ! Pre-mix old-pool release (grid-scale): xsc_rain / xsc_snow
         ! accumulate from the post-phase-change capacity drain (melt/
         ! freeze that pushed a component past can_cpy), all BEFORE the
         ! new-rain Rutter interception step. Scale per-veg → grid via
         ! sigf_safe. Post-Rutter overflow (adds into tfall_rain) is a
         ! post-mix drip at R_mixed and is NOT counted here.
         xsc_rain_out = xsc_rain * sigf_safe / deltim
         xsc_snow_out = xsc_snow * sigf_safe / deltim

         ! Convert per-veg phase-change masses to grid-scale [mm] so
         ! tracer_precip can migrate the corresponding trc_ldew_* mass
         ! using the same units as the ldew_*_old_trc snapshot.
         ldew_smelt_out        = ldew_smelt_out        * sigf_safe
         ldew_frzc_out         = ldew_frzc_out         * sigf_safe
         ! Same per-veg -> grid-scale conversion for the fusion heat flux.
         canopy_phase_heat_out = canopy_phase_heat_out * sigf_safe

#if (defined CoLMDEBUG)
         ! Mass balance check: w_l (grid-scale old storage + precip) should equal
         ! new grid-scale storage + grid-scale ground flux
         w_l = w_l - ldew - (pg_rain + pg_snow) * deltim
         IF (abs(w_l) > 1.e-6) THEN
            write(6,*) 'JULES interception mass balance error: ', w_l
            write(6,*) 'ldew=', ldew, ' pg*dt=', (pg_rain+pg_snow)*deltim
            CALL abort
         ENDIF

         CALL check_interception_balance('JULES', &
              ldew, ldew_rain, ldew_snow, pg_rain, pg_snow, &
              qintr, qintr_rain, qintr_snow)
#endif

      ELSE
         ! No vegetation: all precipitation passes through, release any
         ! stored water. Clamp raw precipitation to prevent negative pg
         ! (matching vegetated branch).
         ! Preserve pre-existing canopy water signature for tracer attribution.
         ! The ldew_rain / ldew_snow released here is OLD canopy water
         ! (R_canopy_pre), not fresh throughfall (R_input). Route it through
         ! xsc_*_out BEFORE ldew is reset so tracer_precip classifies it
         ! correctly (otherwise it would be lumped into throughfall and diluted
         ! by fresh precip signature).
         CALL release_no_vegetation_canopy_storage(deltim,tleaf, &
              prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler, &
              ldew,ldew_rain,ldew_snow,pg_rain,pg_snow, &
              qintr,qintr_rain,qintr_snow,gross_intr_rain,gross_intr_snow, &
              xsc_rain_out,xsc_snow_out)
      ENDIF
   END SUBROUTINE LEAF_interception_JULES

   SUBROUTINE LEAF_interception_wrap(deltim,dewmx,forc_us,forc_vs,chil,sigf,fsno,lai,sai,tair,tleaf, &
                               prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler,bifall, &
                                                       ldew,ldew_rain,ldew_snow,z0m,hu,pg_rain, &
                                                            pg_snow,qintr,qintr_rain,qintr_snow, &
                                                            gross_intr_rain,gross_intr_snow, &
                                                            xsc_rain_out,xsc_snow_out, &
                                                            ldew_smelt_out,ldew_frzc_out, &
                                                            canopy_phase_heat_out)
!DESCRIPTION
!===========
   !wrapper for calculation of canopy interception using USGS or IGBP land cover classification

!ANCILLARY FUNCTIONS AND SUBROUTINES
!-------------------

!Original Author:
!-------------------
   !---Shupeng Zhang

!References:


!REVISION HISTORY
!----------------

   IMPLICIT NONE

   real(r8), intent(in)    :: deltim     !seconds in a time step [second]
   real(r8), intent(in)    :: dewmx      !maximum dew [mm]
   real(r8), intent(in)    :: forc_us    !wind speed
   real(r8), intent(in)    :: forc_vs    !wind speed
   real(r8), intent(in)    :: chil       !leaf angle distribution factor
   real(r8), intent(in)    :: prc_rain   !convective rainfall [mm/s]
   real(r8), intent(in)    :: prc_snow   !convective snowfall [mm/s]
   real(r8), intent(in)    :: prl_rain   !large-scale rainfall [mm/s]
   real(r8), intent(in)    :: prl_snow   !large-scale snowfall [mm/s]
   real(r8), intent(in)    :: qflx_irrig_sprinkler !irrigation and sprinkler water [mm/s]
   real(r8), intent(in)    :: bifall     !bulk density of newly fallen dry snow [kg/m3]
   real(r8), intent(in)    :: sigf       !fraction of veg cover, excluding snow-covered veg [-]
   real(r8), intent(in)    :: fsno       !fraction of ground covered by snow [-]
   real(r8), intent(in)    :: lai        !leaf area index [-]
   real(r8), intent(in)    :: sai        !stem area index [-]
   real(r8), intent(in)    :: tair       !air temperature [K]
   real(r8), intent(inout) :: tleaf      !sunlit canopy leaf temperature [K]

   real(r8), intent(inout) :: ldew       !depth of water on foliage [mm]
   real(r8), intent(inout) :: ldew_rain  !depth of liquid on foliage [mm]
   real(r8), intent(inout) :: ldew_snow  !depth of liquid on foliage [mm]
   real(r8), intent(in)    :: z0m        !roughness length
   real(r8), intent(in)    :: hu         !forcing height of U


   real(r8), intent(out)   :: pg_rain    !rainfall onto ground including canopy runoff [kg/(m2 s)]
   real(r8), intent(out)   :: pg_snow    !snowfall onto ground including canopy runoff [kg/(m2 s)]
   real(r8), intent(out)   :: qintr      !interception [kg/(m2 s)]
   real(r8), intent(out)   :: qintr_rain !rainfall interception (mm h2o/s) [NET storage-related flux; can be <0 when canopy drains faster than new rain intercepts. Use gross_intr_rain for gross interception]
   real(r8), intent(out)   :: qintr_snow !snowfall interception (mm h2o/s) [NET storage-related flux; can be <0 (e.g. snow unloading/blowing with no new snow). Use gross_intr_snow for gross interception]
   real(r8), intent(out)   :: gross_intr_rain !gross rain entering canopy pool (mm h2o/s, >=0)
   real(r8), intent(out)   :: gross_intr_snow !gross snow entering canopy pool (mm h2o/s, >=0)
   real(r8), intent(out)   :: xsc_rain_out    !pre-mix rain release from old pool (mm h2o/s, >=0)
   real(r8), intent(out)   :: xsc_snow_out    !pre-mix snow release from old pool (mm h2o/s, >=0)
   ! Phase-change tracer transfer (grid-scale mm, >=0). See per-scheme
   ! subroutines for when they are nonzero (NoahMP/MATSIRO/VIC/JULES only).
   real(r8), intent(out)   :: ldew_smelt_out
   real(r8), intent(out)   :: ldew_frzc_out
   ! Canopy phase-change fusion heat flux [W/m^2]. Sign: + heats canopy,
   ! - cools canopy. Non-zero only for NoahMP/MATSIRO/VIC/JULES; MOD_Thermal
   ! consumes it in the canopy energy balance.
   real(r8), intent(out)   :: canopy_phase_heat_out

      IF (DEF_Interception_scheme==1) THEN
         CALL LEAF_interception_CoLM2014 (deltim,dewmx,forc_us,forc_vs,chil,sigf,lai,sai,tair,tleaf,&
                                             prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler,bifall,&
                                             ldew,ldew_rain,ldew_snow,z0m,hu,pg_rain,&
                                             pg_snow,qintr,qintr_rain,qintr_snow,&
                                             gross_intr_rain,gross_intr_snow,&
                                             xsc_rain_out,xsc_snow_out,&
                                             ldew_smelt_out,ldew_frzc_out,&
                                             canopy_phase_heat_out)
      ELSEIF (DEF_Interception_scheme==2) THEN
         CALL LEAF_interception_CLM4 (deltim,dewmx,forc_us,forc_vs,chil,sigf,lai,sai,tair,tleaf,&
                                             prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler,&
                                             ldew,ldew_rain,ldew_snow,z0m,hu,pg_rain,&
                                             pg_snow,qintr,qintr_rain,qintr_snow,&
                                             gross_intr_rain,gross_intr_snow,&
                                             xsc_rain_out,xsc_snow_out,&
                                             ldew_smelt_out,ldew_frzc_out,&
                                             canopy_phase_heat_out)
      ELSEIF (DEF_Interception_scheme==3) THEN
         CALL LEAF_interception_CLM5(deltim,dewmx,forc_us,forc_vs,chil,sigf,lai,sai,tair,tleaf,&
                                             prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler,&
                                             ldew,ldew_rain,ldew_snow,z0m,hu,pg_rain,&
                                             pg_snow,qintr,qintr_rain,qintr_snow,&
                                             gross_intr_rain,gross_intr_snow,&
                                             xsc_rain_out,xsc_snow_out,&
                                             ldew_smelt_out,ldew_frzc_out,&
                                             canopy_phase_heat_out)
      ELSEIF (DEF_Interception_scheme==4) THEN
         CALL LEAF_interception_NoahMP (deltim,dewmx,forc_us,forc_vs,chil,sigf,lai,sai,tair,tleaf,&
                                             prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler,&
                                             ldew,ldew_rain,ldew_snow,z0m,hu,pg_rain,&
                                             pg_snow,qintr,qintr_rain,qintr_snow,&
                                             gross_intr_rain,gross_intr_snow,&
                                             xsc_rain_out,xsc_snow_out,&
                                             ldew_smelt_out,ldew_frzc_out,&
                                             canopy_phase_heat_out)
      ELSEIF  (DEF_Interception_scheme==5) THEN
         CALL LEAF_interception_matsiro (deltim,dewmx,forc_us,forc_vs,chil,sigf,lai,sai,tair,tleaf,&
                                             prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler,&
                                             ldew,ldew_rain,ldew_snow,z0m,hu,pg_rain,&
                                             pg_snow,qintr,qintr_rain,qintr_snow,&
                                             gross_intr_rain,gross_intr_snow,&
                                             xsc_rain_out,xsc_snow_out,&
                                             ldew_smelt_out,ldew_frzc_out,&
                                             canopy_phase_heat_out)

      ELSEIF  (DEF_Interception_scheme==6) THEN
         CALL LEAF_interception_vic (deltim,dewmx,forc_us,forc_vs,chil,sigf,fsno,lai,sai,tair,tleaf,&
                                             prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler,&
                                             ldew,ldew_rain,ldew_snow,z0m,hu,pg_rain,&
                                             pg_snow,qintr,qintr_rain,qintr_snow,&
                                             gross_intr_rain,gross_intr_snow,&
                                             xsc_rain_out,xsc_snow_out,&
                                             ldew_smelt_out,ldew_frzc_out,&
                                             canopy_phase_heat_out)

      ELSEIF  (DEF_Interception_scheme==7) THEN
         CALL LEAF_interception_JULES (deltim,dewmx,forc_us,forc_vs,chil,sigf,lai,sai,tair,tleaf,&
                                             prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler,&
                                             ldew,ldew_rain,ldew_snow,z0m,hu,pg_rain,&
                                             pg_snow,qintr,qintr_rain,qintr_snow,&
                                             gross_intr_rain,gross_intr_snow,&
                                             xsc_rain_out,xsc_snow_out,&
                                             ldew_smelt_out,ldew_frzc_out,&
                                             canopy_phase_heat_out)

      ELSEIF  (DEF_Interception_scheme==8) THEN
         CALL LEAF_interception_colm202x (deltim,dewmx,forc_us,forc_vs,chil,sigf,lai,sai,tair,tleaf,&
                                             prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler,&
                                             ldew,ldew_rain,ldew_snow,z0m,hu,pg_rain,&
                                             pg_snow,qintr,qintr_rain,qintr_snow,&
                                             gross_intr_rain,gross_intr_snow,&
                                             xsc_rain_out,xsc_snow_out,&
                                             ldew_smelt_out,ldew_frzc_out,&
                                             canopy_phase_heat_out)
      ELSE
         ! Fail early on invalid scheme IDs. Without this, intent(out)
         ! pg_rain/pg_snow/qintr*/gross_intr*/xsc_*_out are left undefined
         ! and consumed immediately by CoLMMAIN.F90:867 (qdrip = pg_rain +
         ! pg_snow) and tracer_precip. The pre-existing abort in
         ! MOD_LeafTemperaturePC.F90:1900 is too late — it runs after
         ! THERMAL, long after interception has poisoned downstream.
         ! Ensure intent(out) phase-change outputs are defined before abort()
         ! to silence uninitialized-access diagnostics on compilers that
         ! check intent(out) contracts even on the failure path.
         ldew_smelt_out        = 0._r8
         ldew_frzc_out         = 0._r8
         canopy_phase_heat_out = 0._r8
         write(6,*) 'LEAF_interception_wrap: invalid DEF_Interception_scheme=', &
                    DEF_Interception_scheme, ' (must be 1..8)'
         CALL abort
      ENDIF

   END SUBROUTINE LEAF_interception_wrap

#if (defined LULC_IGBP_PFT || defined LULC_IGBP_PC)
   SUBROUTINE LEAF_interception_pftwrap (ipatch,deltim,dewmx,forc_us,forc_vs,forc_t,fsno,&
                               prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler,bifall,&
                               ldew,ldew_rain,ldew_snow,z0m,hu,pg_rain,pg_snow,qintr,qintr_rain,qintr_snow,&
                               gross_intr_rain,gross_intr_snow,&
                               xsc_rain_out,xsc_snow_out,&
                               ldew_smelt_out,ldew_frzc_out,&
                               canopy_phase_heat_out,canopy_phase_heat_p_out)

! -----------------------------------------------------------------
! !DESCRIPTION:
! wrapper for calculation of canopy interception for PFTs within a land cover type.
!
! Created by Hua Yuan, 06/2019
!
! !REVISION HISTORY:
! 2023.02.21 Zhongwang Wei @ SYSU: add different options of canopy interception for PFTs
!
! -----------------------------------------------------------------

   USE MOD_Precision
   USE MOD_LandPFT
   USE MOD_Const_Physical, only: tfrz
   USE MOD_Vars_PFTimeInvariants
   USE MOD_Vars_PFTimeVariables
   USE MOD_Vars_1DPFTFluxes
   USE MOD_Const_PFT
   IMPLICIT NONE

   integer,  intent(in)    :: ipatch     !patch index
   real(r8), intent(in)    :: deltim     !seconds in a time step [second]
   real(r8), intent(in)    :: dewmx      !maximum dew [mm]
   real(r8), intent(in)    :: forc_us    !wind speed
   real(r8), intent(in)    :: forc_vs    !wind speed
   real(r8), intent(in)    :: forc_t     !air temperature
   real(r8), intent(in)    :: fsno       !fraction of ground covered by snow [-]
   real(r8), intent(in)    :: z0m        !roughness length
   real(r8), intent(in)    :: hu         !forcing height of U
   real(r8), intent(inout) :: ldew_rain  !depth of water on foliage [mm]
   real(r8), intent(inout) :: ldew_snow  !depth of water on foliage [mm]
   real(r8), intent(in)    :: prc_rain   !convective ranfall [mm/s]
   real(r8), intent(in)    :: prc_snow   !convective snowfall [mm/s]
   real(r8), intent(in)    :: prl_rain   !large-scale rainfall [mm/s]
   real(r8), intent(in)    :: prl_snow   !large-scale snowfall [mm/s]
   real(r8), intent(in)    :: qflx_irrig_sprinkler !irrigation and sprinkler water [mm/s]
   real(r8), intent(in)    :: bifall     ! bulk density of newly fallen dry snow [kg/m3]

   real(r8), intent(inout) :: ldew       !depth of water on foliage [mm]
   real(r8), intent(out)   :: pg_rain    !rainfall onto ground including canopy runoff [kg/(m2 s)]
   real(r8), intent(out)   :: pg_snow    !snowfall onto ground including canopy runoff [kg/(m2 s)]
   real(r8), intent(out)   :: qintr      !interception [kg/(m2 s)]
   real(r8), intent(out)   :: qintr_rain !rainfall interception (mm h2o/s) [NET storage-related flux; can be <0 when canopy drains faster than new rain intercepts. Use gross_intr_rain for gross interception]
   real(r8), intent(out)   :: qintr_snow !snowfall interception (mm h2o/s) [NET storage-related flux; can be <0 (e.g. snow unloading/blowing with no new snow). Use gross_intr_snow for gross interception]
   real(r8), intent(out)   :: gross_intr_rain !gross rain entering canopy pool (mm h2o/s, >=0)
   real(r8), intent(out)   :: gross_intr_snow !gross snow entering canopy pool (mm h2o/s, >=0)
   real(r8), intent(out)   :: xsc_rain_out    !pre-mix rain release from old pool (mm h2o/s, >=0)
   real(r8), intent(out)   :: xsc_snow_out    !pre-mix snow release from old pool (mm h2o/s, >=0)
   ! Phase-change tracer transfer (grid-scale mm, >=0). See per-scheme
   ! subroutines for when they are nonzero (NoahMP/MATSIRO/VIC/JULES only).
   real(r8), intent(out)   :: ldew_smelt_out
   real(r8), intent(out)   :: ldew_frzc_out
   ! Canopy phase-change fusion heat flux [W/m^2]. See wrap counterpart.
   real(r8), intent(out)   :: canopy_phase_heat_out
   ! Per-PFT canopy phase-change fusion heat flux [W/m^2]. The scalar
   ! canopy_phase_heat_out remains the pftfrac-weighted patch aggregate.
   real(r8), intent(out)   :: canopy_phase_heat_p_out(:)

   integer i, p, ps, pe
#ifdef CROP
   integer  :: irrig_flag  ! 1 if sprinker, 2 if others
#endif
   real(r8) pg_rain_tmp, pg_snow_tmp
   real(r8) gross_intr_rain_pft, gross_intr_snow_pft  ! per-PFT scalar, reused per iteration
   real(r8) gross_intr_rain_tmp, gross_intr_snow_tmp  ! area-weighted aggregate
   real(r8) xsc_rain_pft, xsc_snow_pft                ! per-PFT scalar, reused per iteration
   real(r8) xsc_rain_tmp, xsc_snow_tmp                ! area-weighted aggregate
   real(r8) ldew_smelt_pft, ldew_frzc_pft             ! per-PFT phase-change mass, reused per iteration
   real(r8) ldew_smelt_tmp, ldew_frzc_tmp             ! area-weighted aggregate
   real(r8) canopy_phase_heat_pft                     ! per-PFT fusion heat flux, reused per iteration
   real(r8) canopy_phase_heat_tmp                     ! area-weighted aggregate [W/m^2]

      pg_rain_tmp = 0.
      pg_snow_tmp = 0.
      gross_intr_rain_tmp = 0._r8
      gross_intr_snow_tmp = 0._r8
      xsc_rain_tmp = 0._r8
      xsc_snow_tmp = 0._r8
      ldew_smelt_tmp = 0._r8
      ldew_frzc_tmp  = 0._r8
      canopy_phase_heat_tmp = 0._r8

      ps = patch_pft_s(ipatch)
      pe = patch_pft_e(ipatch)
      canopy_phase_heat_p_out(:) = 0._r8

      IF (DEF_Interception_scheme==1) THEN
         DO i = ps, pe
            p = pftclass(i)
            CALL LEAF_interception_CoLM2014 (deltim,dewmx,forc_us,forc_vs,chil_p(p),sigf_p(i),lai_p(i),sai_p(i),forc_t,tleaf_p(i),&
                                                prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler,bifall,&
                                                ldew_p(i),ldew_rain_p(i),ldew_snow_p(i),z0m_p(i),hu,pg_rain,pg_snow,qintr_p(i),qintr_rain_p(i),qintr_snow_p(i),&
                                                gross_intr_rain_pft,gross_intr_snow_pft,&
                                                xsc_rain_pft,xsc_snow_pft,&
                                                ldew_smelt_pft,ldew_frzc_pft,&
                                                canopy_phase_heat_pft)
            pg_rain_tmp = pg_rain_tmp + pg_rain*pftfrac(i)
            pg_snow_tmp = pg_snow_tmp + pg_snow*pftfrac(i)
            gross_intr_rain_tmp = gross_intr_rain_tmp + gross_intr_rain_pft*pftfrac(i)
            gross_intr_snow_tmp = gross_intr_snow_tmp + gross_intr_snow_pft*pftfrac(i)
            xsc_rain_tmp = xsc_rain_tmp + xsc_rain_pft*pftfrac(i)
            xsc_snow_tmp = xsc_snow_tmp + xsc_snow_pft*pftfrac(i)
            ldew_smelt_tmp = ldew_smelt_tmp + ldew_smelt_pft*pftfrac(i)
            ldew_frzc_tmp  = ldew_frzc_tmp  + ldew_frzc_pft *pftfrac(i)
            canopy_phase_heat_p_out(i-ps+1) = canopy_phase_heat_pft
            canopy_phase_heat_tmp = canopy_phase_heat_tmp + canopy_phase_heat_pft*pftfrac(i)
         ENDDO
      ELSEIF (DEF_Interception_scheme==2) THEN
         DO i = ps, pe
            p = pftclass(i)
            CALL LEAF_interception_clm4 (deltim,dewmx,forc_us,forc_vs,chil_p(p),sigf_p(i),lai_p(i),sai_p(i),forc_t,tleaf_p(i),&
                                             prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler,&
                                             ldew_p(i),ldew_rain_p(i),ldew_snow_p(i),z0m_p(i),hu,pg_rain,pg_snow,qintr_p(i),qintr_rain_p(i),qintr_snow_p(i),&
                                                gross_intr_rain_pft,gross_intr_snow_pft,&
                                                xsc_rain_pft,xsc_snow_pft,&
                                                ldew_smelt_pft,ldew_frzc_pft,&
                                                canopy_phase_heat_pft)
            pg_rain_tmp = pg_rain_tmp + pg_rain*pftfrac(i)
            pg_snow_tmp = pg_snow_tmp + pg_snow*pftfrac(i)
            gross_intr_rain_tmp = gross_intr_rain_tmp + gross_intr_rain_pft*pftfrac(i)
            gross_intr_snow_tmp = gross_intr_snow_tmp + gross_intr_snow_pft*pftfrac(i)
            xsc_rain_tmp = xsc_rain_tmp + xsc_rain_pft*pftfrac(i)
            xsc_snow_tmp = xsc_snow_tmp + xsc_snow_pft*pftfrac(i)
            ldew_smelt_tmp = ldew_smelt_tmp + ldew_smelt_pft*pftfrac(i)
            ldew_frzc_tmp  = ldew_frzc_tmp  + ldew_frzc_pft *pftfrac(i)
            canopy_phase_heat_p_out(i-ps+1) = canopy_phase_heat_pft
            canopy_phase_heat_tmp = canopy_phase_heat_tmp + canopy_phase_heat_pft*pftfrac(i)
         ENDDO
      ELSEIF (DEF_Interception_scheme==3) THEN
         DO i = ps, pe
            p = pftclass(i)
            CALL LEAF_interception_clm5 (deltim,dewmx,forc_us,forc_vs,chil_p(p),sigf_p(i),lai_p(i),sai_p(i),forc_t,tleaf_p(i),&
                                             prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler,&
                                             ldew_p(i),ldew_rain_p(i),ldew_snow_p(i),z0m_p(i),hu,pg_rain,pg_snow,qintr_p(i),qintr_rain_p(i),qintr_snow_p(i),&
                                                gross_intr_rain_pft,gross_intr_snow_pft,&
                                                xsc_rain_pft,xsc_snow_pft,&
                                                ldew_smelt_pft,ldew_frzc_pft,&
                                                canopy_phase_heat_pft)
            pg_rain_tmp = pg_rain_tmp + pg_rain*pftfrac(i)
            pg_snow_tmp = pg_snow_tmp + pg_snow*pftfrac(i)
            gross_intr_rain_tmp = gross_intr_rain_tmp + gross_intr_rain_pft*pftfrac(i)
            gross_intr_snow_tmp = gross_intr_snow_tmp + gross_intr_snow_pft*pftfrac(i)
            xsc_rain_tmp = xsc_rain_tmp + xsc_rain_pft*pftfrac(i)
            xsc_snow_tmp = xsc_snow_tmp + xsc_snow_pft*pftfrac(i)
            ldew_smelt_tmp = ldew_smelt_tmp + ldew_smelt_pft*pftfrac(i)
            ldew_frzc_tmp  = ldew_frzc_tmp  + ldew_frzc_pft *pftfrac(i)
            canopy_phase_heat_p_out(i-ps+1) = canopy_phase_heat_pft
            canopy_phase_heat_tmp = canopy_phase_heat_tmp + canopy_phase_heat_pft*pftfrac(i)
         ENDDO
      ELSEIF (DEF_Interception_scheme==4) THEN
         DO i = ps, pe
            p = pftclass(i)
            CALL LEAF_interception_NoahMP (deltim,dewmx,forc_us,forc_vs,chil_p(p),sigf_p(i),lai_p(i),sai_p(i),forc_t,tleaf_p(i),&
                                             prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler,&
                                             ldew_p(i),ldew_rain_p(i),ldew_snow_p(i),z0m_p(i),hu,pg_rain,pg_snow,qintr_p(i),qintr_rain_p(i),qintr_snow_p(i),&
                                                gross_intr_rain_pft,gross_intr_snow_pft,&
                                                xsc_rain_pft,xsc_snow_pft,&
                                                ldew_smelt_pft,ldew_frzc_pft,&
                                                canopy_phase_heat_pft)
            pg_rain_tmp = pg_rain_tmp + pg_rain*pftfrac(i)
            pg_snow_tmp = pg_snow_tmp + pg_snow*pftfrac(i)
            gross_intr_rain_tmp = gross_intr_rain_tmp + gross_intr_rain_pft*pftfrac(i)
            gross_intr_snow_tmp = gross_intr_snow_tmp + gross_intr_snow_pft*pftfrac(i)
            xsc_rain_tmp = xsc_rain_tmp + xsc_rain_pft*pftfrac(i)
            xsc_snow_tmp = xsc_snow_tmp + xsc_snow_pft*pftfrac(i)
            ldew_smelt_tmp = ldew_smelt_tmp + ldew_smelt_pft*pftfrac(i)
            ldew_frzc_tmp  = ldew_frzc_tmp  + ldew_frzc_pft *pftfrac(i)
            canopy_phase_heat_p_out(i-ps+1) = canopy_phase_heat_pft
            canopy_phase_heat_tmp = canopy_phase_heat_tmp + canopy_phase_heat_pft*pftfrac(i)
         ENDDO
      ELSEIF (DEF_Interception_scheme==5) THEN
         DO i = ps, pe
            p = pftclass(i)
            CALL LEAF_interception_MATSIRO (deltim,dewmx,forc_us,forc_vs,chil_p(p),sigf_p(i),lai_p(i),sai_p(i),forc_t,tleaf_p(i),&
                                             prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler,&
                                             ldew_p(i),ldew_rain_p(i),ldew_snow_p(i),z0m_p(i),hu,pg_rain,pg_snow,qintr_p(i),qintr_rain_p(i),qintr_snow_p(i),&
                                                gross_intr_rain_pft,gross_intr_snow_pft,&
                                                xsc_rain_pft,xsc_snow_pft,&
                                                ldew_smelt_pft,ldew_frzc_pft,&
                                                canopy_phase_heat_pft)
            pg_rain_tmp = pg_rain_tmp + pg_rain*pftfrac(i)
            pg_snow_tmp = pg_snow_tmp + pg_snow*pftfrac(i)
            gross_intr_rain_tmp = gross_intr_rain_tmp + gross_intr_rain_pft*pftfrac(i)
            gross_intr_snow_tmp = gross_intr_snow_tmp + gross_intr_snow_pft*pftfrac(i)
            xsc_rain_tmp = xsc_rain_tmp + xsc_rain_pft*pftfrac(i)
            xsc_snow_tmp = xsc_snow_tmp + xsc_snow_pft*pftfrac(i)
            ldew_smelt_tmp = ldew_smelt_tmp + ldew_smelt_pft*pftfrac(i)
            ldew_frzc_tmp  = ldew_frzc_tmp  + ldew_frzc_pft *pftfrac(i)
            canopy_phase_heat_p_out(i-ps+1) = canopy_phase_heat_pft
            canopy_phase_heat_tmp = canopy_phase_heat_tmp + canopy_phase_heat_pft*pftfrac(i)
         ENDDO
      ELSEIF (DEF_Interception_scheme==6) THEN
         DO i = ps, pe
            p = pftclass(i)
            CALL LEAF_interception_VIC (deltim,dewmx,forc_us,forc_vs,chil_p(p),sigf_p(i),fsno,lai_p(i),sai_p(i),forc_t,tleaf_p(i),&
                                             prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler,&
                                             ldew_p(i),ldew_rain_p(i),ldew_snow_p(i),z0m_p(i),hu,pg_rain,pg_snow,qintr_p(i),qintr_rain_p(i),qintr_snow_p(i),&
                                                gross_intr_rain_pft,gross_intr_snow_pft,&
                                                xsc_rain_pft,xsc_snow_pft,&
                                                ldew_smelt_pft,ldew_frzc_pft,&
                                                canopy_phase_heat_pft)
            pg_rain_tmp = pg_rain_tmp + pg_rain*pftfrac(i)
            pg_snow_tmp = pg_snow_tmp + pg_snow*pftfrac(i)
            gross_intr_rain_tmp = gross_intr_rain_tmp + gross_intr_rain_pft*pftfrac(i)
            gross_intr_snow_tmp = gross_intr_snow_tmp + gross_intr_snow_pft*pftfrac(i)
            xsc_rain_tmp = xsc_rain_tmp + xsc_rain_pft*pftfrac(i)
            xsc_snow_tmp = xsc_snow_tmp + xsc_snow_pft*pftfrac(i)
            ldew_smelt_tmp = ldew_smelt_tmp + ldew_smelt_pft*pftfrac(i)
            ldew_frzc_tmp  = ldew_frzc_tmp  + ldew_frzc_pft *pftfrac(i)
            canopy_phase_heat_p_out(i-ps+1) = canopy_phase_heat_pft
            canopy_phase_heat_tmp = canopy_phase_heat_tmp + canopy_phase_heat_pft*pftfrac(i)
         ENDDO
      ELSEIF (DEF_Interception_scheme==7) THEN
         DO i = ps, pe
            p = pftclass(i)
            CALL LEAF_interception_JULES (deltim,dewmx,forc_us,forc_vs,chil_p(p),sigf_p(i),lai_p(i),sai_p(i),forc_t,tleaf_p(i),&
                                             prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler,&
                                             ldew_p(i),ldew_rain_p(i),ldew_snow_p(i),z0m_p(i),hu,pg_rain,pg_snow,qintr_p(i),qintr_rain_p(i),qintr_snow_p(i),&
                                                gross_intr_rain_pft,gross_intr_snow_pft,&
                                                xsc_rain_pft,xsc_snow_pft,&
                                                ldew_smelt_pft,ldew_frzc_pft,&
                                                canopy_phase_heat_pft)
            pg_rain_tmp = pg_rain_tmp + pg_rain*pftfrac(i)
            pg_snow_tmp = pg_snow_tmp + pg_snow*pftfrac(i)
            gross_intr_rain_tmp = gross_intr_rain_tmp + gross_intr_rain_pft*pftfrac(i)
            gross_intr_snow_tmp = gross_intr_snow_tmp + gross_intr_snow_pft*pftfrac(i)
            xsc_rain_tmp = xsc_rain_tmp + xsc_rain_pft*pftfrac(i)
            xsc_snow_tmp = xsc_snow_tmp + xsc_snow_pft*pftfrac(i)
            ldew_smelt_tmp = ldew_smelt_tmp + ldew_smelt_pft*pftfrac(i)
            ldew_frzc_tmp  = ldew_frzc_tmp  + ldew_frzc_pft *pftfrac(i)
            canopy_phase_heat_p_out(i-ps+1) = canopy_phase_heat_pft
            canopy_phase_heat_tmp = canopy_phase_heat_tmp + canopy_phase_heat_pft*pftfrac(i)
         ENDDO
      ELSEIF (DEF_Interception_scheme==8) THEN
         DO i = ps, pe
            p = pftclass(i)
            CALL LEAF_interception_CoLM202x (deltim,dewmx,forc_us,forc_vs,chil_p(p),sigf_p(i),lai_p(i),sai_p(i),forc_t,tleaf_p(i),&
                                             prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler,&
                                             ldew_p(i),ldew_rain_p(i),ldew_snow_p(i),z0m_p(i),hu,pg_rain,pg_snow,qintr_p(i),qintr_rain_p(i),qintr_snow_p(i),&
                                                gross_intr_rain_pft,gross_intr_snow_pft,&
                                                xsc_rain_pft,xsc_snow_pft,&
                                                ldew_smelt_pft,ldew_frzc_pft,&
                                                canopy_phase_heat_pft)
            pg_rain_tmp = pg_rain_tmp + pg_rain*pftfrac(i)
            pg_snow_tmp = pg_snow_tmp + pg_snow*pftfrac(i)
            gross_intr_rain_tmp = gross_intr_rain_tmp + gross_intr_rain_pft*pftfrac(i)
            gross_intr_snow_tmp = gross_intr_snow_tmp + gross_intr_snow_pft*pftfrac(i)
            xsc_rain_tmp = xsc_rain_tmp + xsc_rain_pft*pftfrac(i)
            xsc_snow_tmp = xsc_snow_tmp + xsc_snow_pft*pftfrac(i)
            ldew_smelt_tmp = ldew_smelt_tmp + ldew_smelt_pft*pftfrac(i)
            ldew_frzc_tmp  = ldew_frzc_tmp  + ldew_frzc_pft *pftfrac(i)
            canopy_phase_heat_p_out(i-ps+1) = canopy_phase_heat_pft
            canopy_phase_heat_tmp = canopy_phase_heat_tmp + canopy_phase_heat_pft*pftfrac(i)
         ENDDO
      ELSE
         ! Fail early on invalid scheme IDs.
         ! See wrap counterpart above for rationale.
         ldew_smelt_out        = 0._r8
         ldew_frzc_out         = 0._r8
         canopy_phase_heat_out = 0._r8
         write(6,*) 'LEAF_interception_pftwrap: invalid DEF_Interception_scheme=', &
                    DEF_Interception_scheme, ' (must be 1..8)'
         CALL abort
      ENDIF

      pg_rain = pg_rain_tmp
      pg_snow = pg_snow_tmp
      ldew    = sum( ldew_p(ps:pe) * pftfrac(ps:pe))
      ldew_rain = sum( ldew_rain_p(ps:pe) * pftfrac(ps:pe))
      ldew_snow = sum( ldew_snow_p(ps:pe) * pftfrac(ps:pe))
      qintr   = sum(qintr_p(ps:pe) * pftfrac(ps:pe))
      qintr_rain = sum(qintr_rain_p(ps:pe) * pftfrac(ps:pe))
      qintr_snow = sum(qintr_snow_p(ps:pe) * pftfrac(ps:pe))
      gross_intr_rain = gross_intr_rain_tmp
      gross_intr_snow = gross_intr_snow_tmp
      xsc_rain_out = xsc_rain_tmp
      xsc_snow_out = xsc_snow_tmp
      ldew_smelt_out = ldew_smelt_tmp
      ldew_frzc_out  = ldew_frzc_tmp
      canopy_phase_heat_out = canopy_phase_heat_tmp


   END SUBROUTINE LEAF_interception_pftwrap
#endif

   SUBROUTINE release_no_vegetation_canopy_storage(deltim,tleaf, &
         prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler, &
         ldew,ldew_rain,ldew_snow,pg_rain,pg_snow, &
         qintr,qintr_rain,qintr_snow,gross_intr_rain,gross_intr_snow, &
         xsc_rain_out,xsc_snow_out)

      real(r8), intent(in) :: deltim, tleaf
      real(r8), intent(in) :: prc_rain, prc_snow, prl_rain, prl_snow
      real(r8), intent(in) :: qflx_irrig_sprinkler
      real(r8), intent(inout) :: ldew, ldew_rain, ldew_snow
      real(r8), intent(out) :: pg_rain, pg_snow
      real(r8), intent(out) :: qintr, qintr_rain, qintr_snow
      real(r8), intent(out) :: gross_intr_rain, gross_intr_snow
      real(r8), intent(out) :: xsc_rain_out, xsc_snow_out

      IF (ldew > 1.e-12_r8 .and. (ldew_rain + ldew_snow) < 1.e-12_r8) THEN
         IF (tleaf > tfrz) THEN
            ldew_rain = ldew
            ldew_snow = 0._r8
         ELSE
            ldew_rain = 0._r8
            ldew_snow = ldew
         ENDIF
      ENDIF

      ldew_rain = max(0._r8, ldew_rain)
      ldew_snow = max(0._r8, ldew_snow)
      ldew      = ldew_rain + ldew_snow

      xsc_rain_out = ldew_rain / deltim
      xsc_snow_out = ldew_snow / deltim

      pg_rain = max(0._r8, prc_rain + prl_rain + qflx_irrig_sprinkler) + xsc_rain_out
      pg_snow = max(0._r8, prc_snow + prl_snow) + xsc_snow_out

      ldew            = 0._r8
      ldew_rain       = 0._r8
      ldew_snow       = 0._r8
      qintr           = 0._r8
      qintr_rain      = 0._r8
      qintr_snow      = 0._r8
      gross_intr_rain = 0._r8
      gross_intr_snow = 0._r8

   END SUBROUTINE release_no_vegetation_canopy_storage

   SUBROUTINE check_interception_balance(scheme_name, &
         ldew, ldew_rain, ldew_snow, pg_rain, pg_snow, &
         qintr, qintr_rain, qintr_snow)

   ! Validates interception water balance consistency.
   ! Called from CoLMDEBUG blocks after each scheme completes.

      character(len=*), intent(in) :: scheme_name
      real(r8), intent(in) :: ldew, ldew_rain, ldew_snow
      real(r8), intent(in) :: pg_rain, pg_snow
      real(r8), intent(in) :: qintr, qintr_rain, qintr_snow

      ! Check A: component consistency (ldew == ldew_rain + ldew_snow)
      IF (abs(ldew - (ldew_rain + ldew_snow)) > INTERCEPTION_BALANCE_TOL) THEN
         write(6,*) 'Component consistency error in ', scheme_name, ':'
         write(6,*) 'ldew=', ldew, ' ldew_rain+ldew_snow=', ldew_rain+ldew_snow
         write(6,*) 'diff=', ldew - (ldew_rain + ldew_snow)
         CALL abort
      ENDIF

      ! Check B: non-negativity
      IF (ldew < -INTERCEPTION_BALANCE_TOL .or. &
          ldew_rain < -INTERCEPTION_BALANCE_TOL .or. &
          ldew_snow < -INTERCEPTION_BALANCE_TOL .or. &
          pg_rain < -INTERCEPTION_BALANCE_TOL .or. &
          pg_snow < -INTERCEPTION_BALANCE_TOL) THEN
         write(6,*) 'Negative value error in ', scheme_name, ':'
         write(6,*) 'ldew=', ldew, ' ldew_rain=', ldew_rain, ' ldew_snow=', ldew_snow
         write(6,*) 'pg_rain=', pg_rain, ' pg_snow=', pg_snow
         CALL abort
      ENDIF

      ! Check C: flux consistency (qintr == qintr_rain + qintr_snow)
      IF (abs(qintr - (qintr_rain + qintr_snow)) > INTERCEPTION_BALANCE_TOL) THEN
         write(6,*) 'Flux consistency error in ', scheme_name, ':'
         write(6,*) 'qintr=', qintr, ' qintr_rain+qintr_snow=', qintr_rain+qintr_snow
         write(6,*) 'diff=', qintr - (qintr_rain + qintr_snow)
         CALL abort
      ENDIF

   END SUBROUTINE check_interception_balance

END MODULE MOD_LeafInterception
