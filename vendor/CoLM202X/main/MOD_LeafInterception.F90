#include <define.h>
MODULE MOD_LeafInterception
! -----------------------------------------------------------------
! !DESCRIPTION:
! For calculating vegetation canopy precipitation interception.
!
! This MODULE is the coupler for the colm and CaMa-Flood model.

!ANCILLARY FUNCTIONS AND SUBROUTINES
!-------------------
   !* :SUBROUTINE:"LEAF_interception_CoLM2014" : Leaf interception and drainage schemes based on colm2014 version
   !* :SUBROUTINE:"LEAF_interception_pftwrap"  : wrapper for pft land use classification

!REVISION HISTORY:
!----------------
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
   USE MOD_Namelist, only: DEF_VEG_SNOW

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

      xsc_rain = 0._r8
      xsc_snow = 0._r8

      IF (lai+sai > 1e-6) THEN
         lsai   = lai + sai
         vegt   = lsai
         satcap = dewmx*vegt
         satcap_rain = satcap
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
               xsc_rain = max(0._r8, ldew)
               pg_rain = prc_rain + prl_rain + qflx_irrig_sprinkler + ldew/deltim
               pg_snow = prc_snow + prl_snow
            ELSE
               xsc_snow = max(0._r8, ldew)
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

      ! Optional diagnostics for TRACER-aware callers.  They do not change
      ! the CoLM2014 interception physics above and are absent from the
      ! original/default call sites (e.g. URBAN).
      IF (present(gross_intr_rain))       gross_intr_rain       = max(0._r8, qintr_rain)
      IF (present(gross_intr_snow))       gross_intr_snow       = max(0._r8, qintr_snow)
      IF (present(xsc_rain_out))          xsc_rain_out          = xsc_rain / deltim
      IF (present(xsc_snow_out))          xsc_snow_out          = xsc_snow / deltim
      IF (present(ldew_smelt_out))        ldew_smelt_out        = 0._r8
      IF (present(ldew_frzc_out))         ldew_frzc_out         = 0._r8
      IF (present(canopy_phase_heat_out)) canopy_phase_heat_out = 0._r8

   END SUBROUTINE LEAF_interception_CoLM2014

   SUBROUTINE LEAF_interception_wrap(deltim,dewmx,forc_us,forc_vs,chil,sigf,lai,sai,tair,tleaf, &
                               prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler,bifall, &
                                                       ldew,ldew_rain,ldew_snow,z0m,hu,pg_rain, &
                                                            pg_snow,qintr,qintr_rain,qintr_snow, &
                                                            gross_intr_rain,gross_intr_snow, &
                                                            xsc_rain_out,xsc_snow_out, &
                                                            ldew_smelt_out,ldew_frzc_out, &
                                                            canopy_phase_heat_out)
   IMPLICIT NONE
   real(r8), intent(in)    :: deltim, dewmx, forc_us, forc_vs, chil
   real(r8), intent(in)    :: prc_rain, prc_snow, prl_rain, prl_snow
   real(r8), intent(in)    :: qflx_irrig_sprinkler, bifall
   real(r8), intent(in)    :: sigf, lai, sai, tair
   real(r8), intent(inout) :: tleaf
   real(r8), intent(inout) :: ldew, ldew_rain, ldew_snow
   real(r8), intent(in)    :: z0m, hu
   real(r8), intent(out)   :: pg_rain, pg_snow, qintr, qintr_rain, qintr_snow
   real(r8), intent(out), optional :: gross_intr_rain, gross_intr_snow
   real(r8), intent(out), optional :: xsc_rain_out, xsc_snow_out
   real(r8), intent(out), optional :: ldew_smelt_out, ldew_frzc_out
   real(r8), intent(out), optional :: canopy_phase_heat_out

      CALL LEAF_interception_CoLM2014 (deltim,dewmx,forc_us,forc_vs,chil,sigf,lai,sai,tair,tleaf,&
         prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler,bifall,&
         ldew,ldew_rain,ldew_snow,z0m,hu,pg_rain,pg_snow,qintr,qintr_rain,qintr_snow,&
         gross_intr_rain,gross_intr_snow,xsc_rain_out,xsc_snow_out,&
         ldew_smelt_out,ldew_frzc_out,canopy_phase_heat_out)

   END SUBROUTINE LEAF_interception_wrap

#if (defined LULC_IGBP_PFT || defined LULC_IGBP_PC)
   SUBROUTINE LEAF_interception_pftwrap (ipatch,deltim,dewmx,forc_us,forc_vs,forc_t,&
                               prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler,bifall,&
                               ldew,ldew_rain,ldew_snow,z0m,hu,pg_rain,pg_snow,qintr,qintr_rain,qintr_snow,&
                               gross_intr_rain,gross_intr_snow,&
                               xsc_rain_out,xsc_snow_out,&
                               ldew_smelt_out,ldew_frzc_out,&
                               canopy_phase_heat_out,canopy_phase_heat_p_out)
   USE MOD_Precision
   USE MOD_LandPFT
   USE MOD_Vars_PFTimeInvariants
   USE MOD_Vars_PFTimeVariables
   USE MOD_Vars_1DPFTFluxes
   USE MOD_Const_PFT
   IMPLICIT NONE
   integer,  intent(in)    :: ipatch
   real(r8), intent(in)    :: deltim, dewmx, forc_us, forc_vs, forc_t
   real(r8), intent(in)    :: z0m, hu
   real(r8), intent(inout) :: ldew_rain, ldew_snow
   real(r8), intent(in)    :: prc_rain, prc_snow, prl_rain, prl_snow
   real(r8), intent(in)    :: qflx_irrig_sprinkler, bifall
   real(r8), intent(inout) :: ldew
   real(r8), intent(out)   :: pg_rain, pg_snow, qintr, qintr_rain, qintr_snow
   real(r8), intent(out), optional :: gross_intr_rain, gross_intr_snow
   real(r8), intent(out), optional :: xsc_rain_out, xsc_snow_out
   real(r8), intent(out), optional :: ldew_smelt_out, ldew_frzc_out
   real(r8), intent(out), optional :: canopy_phase_heat_out
   real(r8), intent(out), optional :: canopy_phase_heat_p_out(:)

   integer i, p, ps, pe
   real(r8) pg_rain_tmp, pg_snow_tmp
   real(r8) gross_rain_tmp, gross_snow_tmp, xsc_rain_tmp, xsc_snow_tmp
   real(r8) smelt_tmp, frzc_tmp, heat_tmp
   real(r8) gross_rain_i, gross_snow_i, xsc_rain_i, xsc_snow_i
   real(r8) smelt_i, frzc_i, heat_i

      pg_rain_tmp = 0._r8
      pg_snow_tmp = 0._r8
      gross_rain_tmp = 0._r8
      gross_snow_tmp = 0._r8
      xsc_rain_tmp = 0._r8
      xsc_snow_tmp = 0._r8
      smelt_tmp = 0._r8
      frzc_tmp = 0._r8
      heat_tmp = 0._r8

      ps = patch_pft_s(ipatch)
      pe = patch_pft_e(ipatch)
      IF (present(canopy_phase_heat_p_out)) canopy_phase_heat_p_out(:) = 0._r8

      DO i = ps, pe
         p = pftclass(i)
         CALL LEAF_interception_CoLM2014 (deltim,dewmx,forc_us,forc_vs,chil_p(p),sigf_p(i),lai_p(i),sai_p(i),forc_t,tleaf_p(i),&
            prc_rain,prc_snow,prl_rain,prl_snow,qflx_irrig_sprinkler,bifall,&
            ldew_p(i),ldew_rain_p(i),ldew_snow_p(i),z0m_p(i),hu,pg_rain,pg_snow,qintr_p(i),qintr_rain_p(i),qintr_snow_p(i),&
            gross_rain_i,gross_snow_i,xsc_rain_i,xsc_snow_i,smelt_i,frzc_i,heat_i)
         pg_rain_tmp = pg_rain_tmp + pg_rain*pftfrac(i)
         pg_snow_tmp = pg_snow_tmp + pg_snow*pftfrac(i)
         gross_rain_tmp = gross_rain_tmp + gross_rain_i*pftfrac(i)
         gross_snow_tmp = gross_snow_tmp + gross_snow_i*pftfrac(i)
         xsc_rain_tmp = xsc_rain_tmp + xsc_rain_i*pftfrac(i)
         xsc_snow_tmp = xsc_snow_tmp + xsc_snow_i*pftfrac(i)
         smelt_tmp = smelt_tmp + smelt_i*pftfrac(i)
         frzc_tmp = frzc_tmp + frzc_i*pftfrac(i)
         heat_tmp = heat_tmp + heat_i*pftfrac(i)
         IF (present(canopy_phase_heat_p_out)) canopy_phase_heat_p_out(i - ps + 1) = heat_i
      ENDDO

      pg_rain = pg_rain_tmp
      pg_snow = pg_snow_tmp
      ldew    = sum( ldew_p(ps:pe) * pftfrac(ps:pe))
      ldew_rain = sum( ldew_rain_p(ps:pe) * pftfrac(ps:pe))
      ldew_snow = sum( ldew_snow_p(ps:pe) * pftfrac(ps:pe))
      qintr   = sum(qintr_p(ps:pe) * pftfrac(ps:pe))
      qintr_rain = sum(qintr_rain_p(ps:pe) * pftfrac(ps:pe))
      qintr_snow = sum(qintr_snow_p(ps:pe) * pftfrac(ps:pe))
      IF (present(gross_intr_rain)) gross_intr_rain = gross_rain_tmp
      IF (present(gross_intr_snow)) gross_intr_snow = gross_snow_tmp
      IF (present(xsc_rain_out)) xsc_rain_out = xsc_rain_tmp
      IF (present(xsc_snow_out)) xsc_snow_out = xsc_snow_tmp
      IF (present(ldew_smelt_out)) ldew_smelt_out = smelt_tmp
      IF (present(ldew_frzc_out)) ldew_frzc_out = frzc_tmp
      IF (present(canopy_phase_heat_out)) canopy_phase_heat_out = heat_tmp

   END SUBROUTINE LEAF_interception_pftwrap
#endif


END MODULE MOD_LeafInterception
