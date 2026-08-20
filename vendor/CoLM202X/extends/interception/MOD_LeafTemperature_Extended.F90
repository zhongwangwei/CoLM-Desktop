#include <define.h>

MODULE MOD_LeafTemperature
! NM-1-v2: canopy phase-change heat is explicitly propagated through
! canopy_phase_heat/canopy_phase_heat_out and balanced in the leaf
! temperature energy channel.

!-----------------------------------------------------------------------
   USE MOD_Precision
   USE MOD_Namelist, only: DEF_USE_CBL_HEIGHT, DEF_USE_PLANTHYDRAULICS, DEF_USE_OZONESTRESS, &
                           DEF_RSS_SCHEME, DEF_Interception_scheme, DEF_MATSIRO_CWCAP_SCALE, &
                           DEF_SPLIT_SOILSNOW, DEF_VEG_SNOW
   USE MOD_SPMD_Task
   USE MOD_PHSRootfluxBalance, only: balance_phs_rootflux

   IMPLICIT NONE

   SAVE

! PUBLIC MEMBER FUNCTIONS:
   PUBLIC :: LeafTemperature

! PRIVATE MEMBER FUNCTIONS:
   PRIVATE :: dewfraction
   PRIVATE :: effective_canopy_area
!-----------------------------------------------------------------------

CONTAINS

!-----------------------------------------------------------------------

   SUBROUTINE LeafTemperature ( &
              ipatch     ,ivt        ,deltim     ,csoilc     ,dewmx      ,htvp       ,&
              lai        ,sai        ,htop       ,hbot       ,sqrtdi     ,effcon     ,&
              vmax25     ,c3c4       ,slti       ,hlti       ,shti       ,hhti       ,trda       ,&
              trdm       ,trop       ,g1         ,g0         ,gradm      ,binter     ,&
              extkn      ,extkb      ,extkd      ,hu         ,ht         ,hq         ,&
              us         ,vs         ,thm        ,th         ,thv        ,qm         ,&
              psrf       ,rhoair     ,parsun     ,parsha     ,sabv       ,frl        ,&
              fsun       ,thermk     ,rstfacsun  ,rstfacsha  ,gssun      ,gssha      ,&
              po2m       ,pco2m      ,z0h_g      ,obug       ,ustarg     ,zlnd       ,&
              zsno       ,fsno       ,sigf       ,etrc       ,tg         ,qg         ,&
              rss        ,t_soil     ,t_snow     ,q_soil     ,q_snow     ,dqgdT      ,&
              emg        ,tl         ,ldew       ,ldew_rain  ,ldew_snow  ,fwet_snow  ,&
              taux       ,tauy       ,fseng      ,fseng_soil ,fseng_snow ,fevpg      ,&
              fevpg_soil ,fevpg_snow ,cgrnd      ,cgrndl     ,cgrnds     ,tref       ,&
              qref       ,rst        ,assim      ,respc      ,fsenl      ,fevpl      ,&
              etr        ,dlrad      ,ulrad      ,z0m        ,zol        ,rib        ,&
              ustar      ,qstar      ,tstar      ,fm         ,fh         ,fq         ,&
              rootfr     ,&
!Plant Hydraulic variables
              kmax_sun   ,kmax_sha   ,kmax_xyl   ,kmax_root  ,psi50_sun  ,psi50_sha  ,&
              psi50_xyl  ,psi50_root ,ck         ,vegwp      ,gs0sun     ,gs0sha     ,&
              assimsun   ,etrsun     ,assimsha   ,etrsha     ,&
!Ozone stress variables
              o3coefv_sun,o3coefv_sha,o3coefg_sun,o3coefg_sha,&
              lai_old    ,o3uptakesun,o3uptakesha,forc_ozone ,&
!End ozone stress variables
!WUE stomata model parameter
              lambda     ,&
!End WUE stomata model parameter
              hpbl       ,&
              qintr_rain ,qintr_snow ,t_precip   ,lfevpl     ,hprl       ,dheatl     ,smp        ,&
              hk         ,hksati     ,rootflux   ,canopy_phase_heat                  ,&
              canopy_smelt_mass_out, canopy_frzc_mass_out, raw_trc_out)

!=======================================================================
! !DESCRIPTION:
!  Foliage energy conservation is given by foliage energy budget equation
!                       Rnet - Hf - LEf = 0
!  The equation is solved by Newton-Raphson iteration, in which this
!  iteration includes the calculation of the photosynthesis and stomatal
!  resistance, and the integration of turbulent flux profiles. The
!  sensible and latent heat transfer between foliage and atmosphere and
!  ground is linked by the equations:
!                       Ha = Hf + Hg and Ea = Ef + Eg
!
!  Original author: Yongjiu Dai, August 15, 2001
!
! !REVISIONS:
!
!  09/2014, Hua Yuan: imbalanced energy due to T/q adjustment is
!           allocated to sensible heat flux.
!
!  10/2017, Hua Yuan: added options for z0, displa, rb and rd
!           calculation (Dai, Y., Yuan, H., Xin, Q., Wang, D.,
!           Shangguan, W., Zhang, S., et al. (2019). Different
!           representations of canopy structure—A large source of
!           uncertainty in global land surface modeling. Agricultural
!           and Forest Meteorology, 269-270, 119-135.
!           https://doi.org/10.1016/j.agrformet.2019.02.006
!
!  10/2019, Hua Yuan: change only the leaf temperature from two-leaf
!           to one-leaf (due to large differences may exist between
!           sunlit/shaded leaf temperature.
!
!  01/2021, Xingjie Lu and Nan Wei: added plant hydraulic process
!           interface.
!
!  01/2021, Nan Wei: added interaction btw prec and canopy.
!
!  05/2023, Shaofeng Liu: add option to call moninobuk_leddy, the
!           LargeEddy surface turbulence scheme (LZD2022); make a proper
!           update of um.
!
!  04/2024, Hua Yuan: add option to account for vegetation snow process.
!
!=======================================================================

   USE MOD_Precision
   USE MOD_Vars_Global
   USE MOD_Const_Physical, only: vonkar, grav, hvap, hsub, cpair, stefnc, cpliq, cpice, &
                                 hfus, tfrz, denice, denh2o
   USE MOD_FrictionVelocity
   USE MOD_CanopyLayerProfile
   USE MOD_TurbulenceLEddy
   USE MOD_AssimStomataConductance
   USE MOD_UserSpecifiedForcing, only: HEIGHT_mode
   USE MOD_Vars_TimeInvariants, only: patchclass
   USE MOD_Const_LC, only: z0mr, displar
   USE MOD_PlantHydraulic, only:PlantHydraulicStress_twoleaf, getvegwp_twoleaf
   USE MOD_Ozone, only: CalcOzoneStress
   USE MOD_Qsadv

   IMPLICIT NONE

!-------------------------- Dummy Arguments ----------------------------

   integer,  intent(in) :: ipatch,ivt
   real(r8), intent(in) :: &
        deltim,     &! seconds in a time step [second]
        csoilc,     &! drag coefficient for soil under canopy [-]
        dewmx,      &! maximum dew
        htvp         ! latent heat of evaporation (/sublimation) [J/kg]

! vegetation parameters
   real(r8), intent(inout) :: &
        sai          ! stem area index  [-]
   real(r8), intent(in) :: &
        sqrtdi,     &! inverse sqrt of leaf dimension [m**-0.5]
        htop,       &! PFT crown top height [m]
        hbot,       &! PFT crown bot height [m]

        effcon,     &! quantum efficiency of RuBP regeneration (mol CO2 / mol quanta)
        vmax25,     &! maximum carboxylation rate at 25 C at canopy top
                     ! the range : 30.e-6 <-> 100.e-6 (mol co2 m-2 s-1)
        shti,       &! slope of high temperature inhibition function     (s1)
        hhti,       &! 1/2 point of high temperature inhibition function (s2)
        slti,       &! slope of low temperature inhibition function      (s3)
        hlti,       &! 1/2 point of low temperature inhibition function  (s4)
        trda,       &! temperature coefficient in gs-a model             (s5)
        trdm,       &! temperature coefficient in gs-a model             (s6)
        trop,       &! temperature coefficient in gs-a model         (273+25)
        g1,         &! conductance-photosynthesis slope parameter for medlyn model
        g0,         &! conductance-photosynthesis intercept for medlyn model
        gradm,      &! conductance-photosynthesis slope parameter
        binter,     &! conductance-photosynthesis intercept
!Ozone WUE stomata model parameter
        lambda,     &! Marginal water cost of carbon gain ((mol h2o) (mol co2)-1)
!End WUE stomata model parameter
        extkn        ! coefficient of leaf nitrogen allocation
   integer , intent(in) :: &
        c3c4 ! 1 for c3, 0 for c4
   real(r8), intent(in) :: & ! for plant hydraulic scheme
        kmax_sun,   &! Plant Hydraulics Parameters
        kmax_sha,   &! Plant Hydraulics Parameters
        kmax_xyl,   &! Plant Hydraulics Parameters
        kmax_root,  &! Plant Hydraulics Parameters
        psi50_sun,  &! water potential at 50% loss of sunlit leaf tissue conductance (mmH2O)
        psi50_sha,  &! water potential at 50% loss of shaded leaf tissue conductance (mmH2O)
        psi50_xyl,  &! water potential at 50% loss of xylem tissue conductance (mmH2O)
        psi50_root, &! water potential at 50% loss of root tissue conductance (mmH2O)
        ck           ! shape-fitting parameter for vulnerability curve (-)
   real(r8), intent(inout) :: &
        vegwp(1:nvegwcs),&! vegetation water potential
        gs0sun,     &! maximum stomata conductance of sunlit leaf
        gs0sha       ! maximum stomata conductance of shaded leaf

! input variables
   real(r8), intent(in) :: &
        hu,         &! observational height of wind [m]
        ht,         &! observational height of temperature [m]
        hq,         &! observational height of humidity [m]
        us,         &! wind component in eastward direction [m/s]
        vs,         &! wind component in northward direction [m/s]
        thm,        &! intermediate variable (tm+0.0098*ht)
        th,         &! potential temperature (kelvin)
        thv,        &! virtual potential temperature (kelvin)
        qm,         &! specific humidity at reference height [kg/kg]
        psrf,       &! pressure at reference height [pa]
        rhoair,     &! density air [kg/m**3]

        lai,        &! adjusted leaf area index for seasonal variation [-]
        parsun,     &! par absorbed per unit lai [w/m**2]
        parsha,     &! par absorbed per unit lai [w/m**2]
        sabv,       &! solar radiation absorbed by vegetation [W/m2]
        frl,        &! atmospheric infrared (longwave) radiation [W/m2]
        fsun,       &! sunlit fraction of canopy

        extkb,      &! (k, g(mu)/mu) direct solar extinction coefficient
        extkd,      &! diffuse and scattered diffuse PAR extinction coefficient
        thermk,     &! canopy gap fraction for tir radiation

        po2m,       &! atmospheric partial pressure  o2 (pa)
        pco2m,      &! atmospheric partial pressure co2 (pa)

        z0h_g,      &! bare soil roughness length, sensible heat [m]
        obug,       &! bare soil obu
        ustarg,     &! bare soil ustar
        zlnd,       &! roughness length for soil [m]
        zsno,       &! roughness length for snow [m]
        fsno,       &! fraction of snow cover on ground

        sigf,       &! fraction of veg cover, excluding snow-covered veg [-]
        etrc,       &! maximum possible transpiration rate (mm/s)
        tg,         &! ground surface temperature [K]
        t_soil,     &! ground surface soil temperature [K]
        t_snow,     &! ground surface snow temperature [K]
        qg,         &! specific humidity at ground surface [kg/kg]
        q_soil,     &! specific humidity at ground soil surface [kg/kg]
        q_snow,     &! specific humidity at ground snow surface [kg/kg]
        dqgdT,      &! temperature derivative of "qg"
        rss,        &! soil surface resistance [s/m]
        emg          ! vegetation emissivity

   real(r8), intent(in) :: &
        t_precip,   &! snowfall/rainfall temperature [kelvin]
        qintr_rain, &! rainfall interception (mm h2o/s)
        qintr_snow, &! snowfall interception (mm h2o/s)
        smp     (1:nl_soil), &! soil matrix potential
        rootfr  (1:nl_soil), &! root fraction
        hksati  (1:nl_soil), &! hydraulic conductivity at saturation [mm h2o/s]
        hk      (1:nl_soil)   ! soil hydraulic conductance
   real(r8), intent(in) :: &
        hpbl         ! atmospheric boundary layer height [m]

   real(r8), intent(inout) :: &
        tl,         &! leaf temperature [K]
        ldew,       &! depth of water on foliage [mm]
        ldew_rain,  &! depth of rain on foliage [mm]
        ldew_snow    ! depth of snow on foliage [mm]

   real(r8), intent(out) :: &
        fwet_snow    ! vegetation snow fractional cover [-]

   real(r8), intent(inout) :: &
!Ozone stress variables
        lai_old    ,&! lai in last time step
        o3uptakesun,&! Ozone does, sunlit leaf (mmol O3/m^2)
        o3uptakesha,&! Ozone does, shaded leaf (mmol O3/m^2)
        forc_ozone
!End ozone stress variables

   real(r8), intent(out) :: &
        taux,       &! wind stress: E-W [kg/m/s**2]
        tauy,       &! wind stress: N-S [kg/m/s**2]
        fseng,      &! sensible heat flux from ground [W/m2]
        fseng_soil, &! sensible heat flux from ground soil [W/m2]
        fseng_snow, &! sensible heat flux from ground snow [W/m2]
        fevpg,      &! evaporation heat flux from ground [mm/s]
        fevpg_soil, &! evaporation heat flux from ground soil [mm/s]
        fevpg_snow, &! evaporation heat flux from ground snow [mm/s]
        cgrnd,      &! deriv. of soil energy flux wrt to soil temp [w/m2/k]
        cgrndl,     &! deriv, of soil sensible heat flux wrt soil temp [w/m2/k]
        cgrnds,     &! deriv of soil latent heat flux wrt soil temp [w/m**2/k]
        tref,       &! 2 m height air temperature (kelvin)
        qref,       &! 2 m height air specific humidity
        rstfacsun,  &! factor of soil water stress to transpiration on sunlit leaf
        rstfacsha,  &! factor of soil water stress to transpiration on shaded leaf
        gssun,      &! stomata conductance of sunlit leaf
        gssha,      &! stomata conductance of shaded leaf
        rootflux(1:nl_soil)  ! root water uptake from different layers

   ! Canopy fusion heat flux imported from LEAF_interception [W/m^2].
   ! Non-zero only for DEF_Interception_scheme in {4,5,6,7}: interception
   ! runs the canopy rain<->snow phase change there, and the resulting
   ! fusion heat is accounted here so the canopy energy residual (err)
   ! stays near zero. Sign: +heats canopy (freeze), -cools canopy (melt).
   real(r8), intent(in) :: canopy_phase_heat

   ! Optional patch-level export of canopy phase-change
   ! mass (mm of water-equivalent over deltim). qmelt drives ldew_snow→
   ! ldew_rain at lines 1473-1480; qfrz drives ldew_rain→ldew_snow at
   ! lines 1482-1489. THERMAL forwards these to MOD_Tracer_Evapo so it
   ! can stop inferring the phase-change amount from the d_rain/d_snow
   ! sign-pattern heuristic (which under-counts when canopy melt and
   ! rain-pool evaporation coexist in one step).
   real(r8), intent(out), optional :: canopy_smelt_mass_out
   real(r8), intent(out), optional :: canopy_frzc_mass_out
   real(r8), intent(out), optional :: raw_trc_out

   real(r8), intent(inout) :: &
        assimsun,   &! sunlit leaf assimilation rate [umol co2 /m**2/ s] [+]
        etrsun,     &! transpiration rate of sunlit leaf [mm/s]
        assimsha,   &! shaded leaf assimilation rate [umol co2 /m**2/ s] [+]
        etrsha       ! transpiration rate of shaded leaf [mm/s]

   real(r8), intent(out) :: &
        rst,        &! stomatal resistance
        assim,      &! rate of assimilation
        respc,      &! rate of respiration
        fsenl,      &! sensible heat from leaves [W/m2]
        fevpl,      &! evaporation+transpiration from leaves [mm/s]
        lfevpl,     &! latent heat flux from leaves [W/m2]
        etr,        &! transpiration rate [mm/s]
        dlrad,      &! downward longwave radiation blow the canopy [W/m2]
        ulrad,      &! upward longwave radiation above the canopy [W/m2]
        hprl,       &! precipitation sensible heat from canopy
        dheatl,     &! vegetation heat change [W/m2]

        z0m,        &! effective roughness [m]
        zol,        &! dimensionless height (z/L) used in Monin-Obukhov theory
        rib,        &! bulk Richardson number in surface layer
        ustar,      &! friction velocity [m/s]
        tstar,      &! temperature scaling parameter
        qstar,      &! moisture scaling parameter
        fm,         &! integral of profile function for momentum
        fh,         &! integral of profile function for heat
        fq           ! integral of profile function for moisture

   real(r8), intent(inout) :: &
!Ozone stress variables
        o3coefv_sun,&! Ozone stress factor for photosynthesis on sunlit leaf
        o3coefv_sha,&! Ozone stress factor for photosynthesis on sunlit leaf
        o3coefg_sun,&! Ozone stress factor for stomata on shaded leaf
        o3coefg_sha  ! Ozone stress factor for stomata on shaded leaf
!End ozone stress variables

!-------------------------- Local Variables ----------------------------
! assign iteration parameters
   integer, parameter :: itmax  = 40   !maximum number of iteration
   integer, parameter :: itmin  = 6    !minimum number of iteration
   real(r8),parameter :: delmax = 3.0  !maximum change in leaf temperature [K]
   real(r8),parameter :: dtmin  = 0.01 !max limit for temperature convergence [K]
   real(r8),parameter :: dlemin = 0.1  !max limit for energy flux convergence [w/m2]

   real(r8) dtl(0:itmax+1)     !difference of tl between two iterative step

   real(r8) :: &
        displa,     &! displacement height [m]
        hu_,        &! adjusted observational height of wind [m]
        ht_,        &! adjusted observational height of temperature [m]
        hq_,        &! adjusted observational height of humidity [m]
        zldis,      &! reference height "minus" zero displacement height [m]
        zii,        &! convective boundary layer height [m]
        z0mv,       &! roughness length, momentum [m]
        z0hv,       &! roughness length, sensible heat [m]
        z0qv,       &! roughness length, latent heat [m]
        zeta,       &! dimensionless height used in Monin-Obukhov theory
        beta,       &! coefficient of convective velocity [-]
        wc,         &! convective velocity [m/s]
        wc2,        &! wc**2
        dth,        &! diff of virtual temp. between ref. height and surface
        dthv,       &! diff of vir. poten. temp. between ref. height and surface
        dqh,        &! diff of humidity between ref. height and surface
        obu,        &! monin-obukhov length (m)
        um,         &! wind speed including the stability effect [m/s]
        ur,         &! wind speed at reference height [m/s]
        uaf,        &! velocity of air within foliage [m/s]
        fh2m,       &! relation for temperature at 2m
        fq2m,       &! relation for specific humidity at 2m
        fm10m,      &! integral of profile function for momentum at 10m
        thvstar,    &! virtual potential temperature scaling parameter
        taf,        &! air temperature within canopy space [K]
        qaf,        &! humidity of canopy air [kg/kg]
        eah,        &! canopy air vapor pressure (pa)
        pco2g,      &! co2 pressure (pa) at ground surface (pa)
        pco2a,      &! canopy air co2 pressure (pa)

        fdry,       &! fraction of foliage that is green and dry [-]
        fwet,       &! fraction of foliage covered by water [-]
        cf,         &! heat transfer coefficient from leaves [-]
        rb,         &! leaf boundary layer resistance [s/m]
        rbsun,      &! Sunlit leaf boundary layer resistance [s/m]
        rbsha,      &! Shaded leaf boundary layer resistance [s/m]
        rd,         &! aerodynamical resistance between ground and canopy air
        ram,        &! aerodynamical resistance [s/m]
        rah,        &! thermal resistance [s/m]
        raw,        &! moisture resistance [s/m]
        clai,       &! canopy heat capacity [Jm-2K-1]
        cah,        &! heat conductance for air [m/s]
        cgh,        &! heat conductance for ground [m/s]
        cfh,        &! heat conductance for leaf [m/s]
        caw,        &! latent heat conductance for air [m/s]
        cgw,        &! latent heat conductance for ground [m/s]
        cfw,        &! latent heat conductance for leaf [m/s]
        wtshi,      &! sensible heat resistance for air, grd and leaf [-]
        wtsqi,      &! latent heat resistance for air, grd and leaf [-]
        wta0,       &! normalized heat conductance for air [-]
        wtg0,       &! normalized heat conductance for ground [-]
        wtl0,       &! normalized heat conductance for air and leaf [-]
        wtaq0,      &! normalized latent heat conductance for air [-]
        wtgq0,      &! normalized heat conductance for ground [-]
        wtlq0,      &! normalized latent heat cond. for air and leaf [-]

        ei,         &! vapor pressure on leaf surface [pa]
        deidT,      &! derivative of "ei" on "tl" [pa/K]
        qsatl,      &! leaf specific humidity [kg/kg]
        qsatldT,    &! derivative of "qsatl" on "tlef"

        del,        &! absolute change in leaf temp in current iteration [K]
        del2,       &! change in leaf temperature in previous iteration [K]
        dele,       &! change in heat fluxes from leaf [W/m2]
        dele2,      &! change in heat fluxes from leaf in previous iteration [W/m2]
        det,        &! maximum leaf temp. change in two consecutive iter [K]
        dee,        &! maximum leaf heat fluxes change in two consecutive iter [W/m2]

        obuold,     &! monin-obukhov length from previous iteration
        tlbef,      &! leaf temperature from previous iteration [K]
        ecidif,     &! excess energies [W/m2]
        err,        &! balance error

        rssun,      &! sunlit leaf stomatal resistance [s/m]
        rssha,      &! shaded leaf stomatal resistance [s/m]
        fsha,       &! shaded fraction of canopy
        laisun,     &! sunlit leaf area index, one-sided
        laisha,     &! shaded leaf area index, one-sided
        respcsun,   &! sunlit leaf respiration rate [umol co2 /m**2/ s] [+]
        respcsha,   &! shaded leaf respiration rate [umol co2 /m**2/ s] [+]
        rsoil,      &! soil respiration
        gah2o,      &! conductance between canopy and atmosphere
        gdh2o,      &! conductance between canopy and ground
        tprcor       ! tf*psur*100./1.013e5

   integer it, nmozsgn

   real(r8) delta, fac
   real(r8) evplwet, evplwet_dtl, etr_dtl, elwmax, elwdif, etr0, sumrootr
   real(r8) irab, dirab_dtl, fsenl_dtl, fevpl_dtl
   real(r8) w, csoilcn, z0mg, cintsun(3), cintsha(3)
   real(r8) fevpl_bef, fevpl_noadj, dtl_noadj, htvpl, erre
   real(r8) qevpl, qdewl, qsubl, qfrol, qmelt, qfrz
   real(r8) tl_pre_phase   ! snapshot of tl before Niu (2004) pull
   ! True when the active interception scheme owns
   ! the canopy rain<->snow phase change (and exports the fusion heat
   ! via canopy_phase_heat). Schemes 4/5/6/7 currently set this.
   logical  phase_change_owned_by_interception
   real(r8) flux_deficit                    ! C4b: component-level evap deficit [mm]
   real(r8) phase_flux_deficit              ! Bug 3: unmet same-phase latent demand [mm/s]

   real(r8) lt, egvf

   real(r8) :: sqrtdragc !sqrt(drag coefficient)
   real(r8) :: fai       !canopy frontal area index
   real(r8) :: a_k71     !exponential extinction factor for u/k decline within canopy (Kondo 1971)
   real(r8) :: fqt, fht, fmtop
   real(r8) :: catch_JULES !JULES liquid canopy capacity [mm] (catch0 + dcatch_dlai*LAI)
   real(r8) :: catch_JULES_snow !JULES snow canopy capacity [mm] (snowloadlai*LAI)
   real(r8) :: epot_JULES  !JULES potential evap [kg/m2/s]
   real(r8) :: epdt_JULES  !JULES potential evap × dt [kg/m2 = mm]
   real(r8) :: fraca_JULES !JULES supply-limited wet fraction [-]
   real(r8) :: fraca_JULES_rain !JULES liquid-only fraca using ldew_rain [-]
   real(r8) :: fraca_JULES_snow !JULES snow-only fraca using ldew_snow [-]
   real(r8) :: lai_perveg_JULES !per-veg LAI for DEF_VEG_SNOW coordinate fix [-]
   real(r8) :: wet_area    ! scheme-specific wet-canopy evaporating area [m2/m2]
   real(r8) :: wet_area_cfw ! scheme-specific canopy area used in cfw [m2/m2]
   real(r8) :: wet_cond    ! scheme-specific wet-canopy conductance [m/s]
   real(r8) :: wet_cond_cfw ! scheme-specific wet conductance used in cfw [m/s]
   real(r8) :: wetfrac_VIC !VIC (S/Wmax)^(2/3) [-]
   real(r8) :: wetfrac_VIC_base !phase-aware VIC wet fraction before supply limiting [-]
   real(r8) :: ldew_VIC_evap !water available to scheme-specific evaporation this step [mm]
   real(r8) :: ldew_JULES_rain_evap !JULES liquid storage present before current rain [mm]
   real(r8) :: satcap_rain_VIC_evap !grid-scale VIC liquid capacity [mm]
   real(r8) :: satcap_MATSIRO_evap !MATSIRO canopy capacity used with step-start storage [mm]
   real(r8) :: ec_pot_VIC  !VIC potential wet-canopy evap [kg/m2/s]
   real(r8) :: f_VIC       !VIC supply-limit factor min(1, W/Ec_pot*dt) [-]
   real(r8) :: evp_weight  !Scheme-specific weight replacing (1-delta*(1-fwet)) [-]
   real(r8) :: dry_factor  !Scheme-specific (1 - wet fraction) for transpiration [-]
   ! sigf floors used to convert grid-scale ldew to
   ! per-vegetation before comparing with per-vegetation canopy capacity.
   real(r8) :: sigf_safe_JULES
   real(r8) :: utop, ueff, ktop
   real(r8) :: phih, z0qg, z0hg
   real(r8) :: hsink, displasink
   real(r8) gb_mol
   real(r8),dimension(nl_soil) :: k_soil_root    ! radial root and soil conductance
   real(r8),dimension(nl_soil) :: k_ax_root      ! axial root conductance

   integer,  parameter :: zd_opt = 3             ! z0 and d with vertical profile consideration
   integer,  parameter :: rb_opt = 3             ! rb with vertical profile consideration
   integer,  parameter :: rd_opt = 3             ! rd with vertical profile consideration

!-----------------------------------------------------------------------

! initialization of errors and  iteration parameters
      it     = 1    !counter for leaf temperature iteration
      del    = 0.0  !change in leaf temperature from previous iteration
      dele   = 0.0  !latent head flux from leaf for previous iteration

      dtl(0) = 0.
      fevpl_bef = 0.

      fht  = 0.     !integral of profile function for heat
      fqt  = 0.     !integral of profile function for moisture

!-----------------------------------------------------------------------
! scaling-up coefficients from leaf to canopy
!-----------------------------------------------------------------------

      fsha   = 1. -fsun
      laisun = lai*fsun
      laisha = lai*fsha

! scaling-up coefficients from leaf to canopy
      cintsun(1) = (1.-exp(-(0.110+extkb)*lai))/(0.110+extkb)
      cintsun(2) = (1.-exp(-(extkb+extkd)*lai))/(extkb+extkd)
      cintsun(3) = (1.-exp(-extkb*lai))/extkb

      cintsha(1) = (1.-exp(-0.110*lai))/0.110 - cintsun(1)
      cintsha(2) = (1.-exp(-extkd*lai))/extkd - cintsun(2)
      cintsha(3) = lai - cintsun(3)

!-----------------------------------------------------------------------
! get fraction of wet and dry canopy surface (fwet & fdry)
! initial saturated vapor pressure and humidity and their derivation
!-----------------------------------------------------------------------

      !clai = 4.2 * 1000. * 0.2
      clai = 0.0

      ! 0.2mm*LSAI, account for leaf (plus dew) heat capacity
      IF ( DEF_VEG_SNOW ) THEN
         clai = 0.2*(lai+sai)*cpliq + ldew_rain*cpliq + ldew_snow*cpice
      ENDIF

      CALL dewfraction (sigf,lai,sai,dewmx,tl,ldew,ldew_rain,ldew_snow,fwet,fdry)
      wetfrac_VIC_base = fwet
      ldew_VIC_evap = ldew
      ldew_JULES_rain_evap = ldew_rain

      ! Original MATSIRO computes canopy evaporation from step-start glwc,
      ! before matcnw adds precipitation. CoLM intercepts first, so undo the
      ! net storage flux for the evaporation-side fwet and supply limit.
      IF (DEF_Interception_scheme == 5) THEN
         ldew_VIC_evap = max(0._r8, ldew - (qintr_rain + qintr_snow) * deltim)
         satcap_MATSIRO_evap = 0.2_r8 * DEF_MATSIRO_CWCAP_SCALE * max(lai, 0._r8)
         fwet = 0._r8
         IF (ldew_VIC_evap > 0._r8 .AND. satcap_MATSIRO_evap > 1.e-10_r8) THEN
            fwet = min(ldew_VIC_evap / satcap_MATSIRO_evap, 1._r8)
         ENDIF
         fdry = (1._r8 - fwet) * lai / max(lai + sai, 1.e-10_r8)
         ldew_VIC_evap = min(ldew, ldew_VIC_evap)
      ENDIF

      ! For warm-rain steps, VIC canopy_evap() computes evaporation
      ! before adding current precipitation. CoLM interception runs first, so
      ! reconstruct the step-start storage from the net intercepted-rain rate.
      ! Snow cases stay on the post-interception state used by snow_intercept().
      IF (DEF_Interception_scheme == 6 &
          .AND. fsno <= 1.e-10_r8 .AND. ldew_snow <= 1.e-10_r8 &
          .AND. abs(qintr_snow) <= 1.e-10_r8 &
          .AND. abs(canopy_phase_heat) <= 1.e-10_r8) THEN
         ldew_VIC_evap = max(0._r8, ldew_rain - qintr_rain * deltim)
         satcap_rain_VIC_evap = canopy_rain_capacity_for_fwet(sigf, lai, sai, dewmx)
         wetfrac_VIC_base = 0._r8
         IF (ldew_VIC_evap > 0._r8 .AND. satcap_rain_VIC_evap > 1.e-10_r8) THEN
            wetfrac_VIC_base = min((ldew_VIC_evap / satcap_rain_VIC_evap)**.666666666666_r8, 1._r8)
         ENDIF
         fwet = wetfrac_VIC_base
         ! Daily VIC may evaporate newly supplied rain. The intercepted
         ! store is the mass still available after CoLM's earlier overflow.
         IF (deltim >= 86400._r8) ldew_VIC_evap = ldew_rain
      ENDIF

      ! Original JULES evaporates the step-start canopy store before sieve()
      ! adds current precipitation. CoLM calls interception first, so recover
      ! that liquid store for snow-free steps from the exact net storage flux.
      IF (DEF_Interception_scheme == 7 &
          .AND. fsno <= 1.e-10_r8 .AND. ldew_snow <= 1.e-10_r8 &
          .AND. abs(qintr_snow) <= 1.e-10_r8 &
          .AND. abs(canopy_phase_heat) <= 1.e-10_r8) THEN
         ldew_JULES_rain_evap = min(ldew_rain, &
              max(0._r8, ldew_rain - qintr_rain * deltim))
         ldew_VIC_evap = ldew_JULES_rain_evap
      ENDIF

      CALL qsadv(tl,psrf,ei,deiDT,qsatl,qsatlDT)

!-----------------------------------------------------------------------
! initial for fluxes profile
!-----------------------------------------------------------------------

      nmozsgn = 0    !number of times moz changes sign
      obuold = 0.    !monin-obukhov length from previous iteration
      zii  = 1000.   !m (pbl height)
      beta = 1.      !- (in computing W_*)
      z0mg = (1.-fsno)*zlnd + fsno*zsno
      z0hg = z0mg
      z0qg = z0mg

      z0m    = htop * z0mr(patchclass(ipatch))
      displa = htop * displar(patchclass(ipatch))

      z0mv = z0m; z0hv = z0m; z0qv = z0m

      ! Modify aerodynamic parameters for sparse/dense canopy (X. Zeng)
      lt     = min(lai+sai, 2.)
      egvf   = (1._r8 - exp(-lt)) / (1._r8 - exp(-2.))
      displa = egvf * displa
      z0mv   = exp(egvf * log(z0mv) + (1._r8 - egvf) * log(z0mg))

      z0hv   = z0mv
      z0qv   = z0mv

! 10/17/2017, yuan: z0m and displa with vertical profile solution
      IF (zd_opt == 3) THEN

         CALL cal_z0_displa(lai+sai, htop, 1._r8, z0mv, displa)

         ! NOTE: adjusted for small displa
         displasink = max(htop/2., displa)
         hsink = z0mv + displasink

         z0hv   = z0mv
         z0qv   = z0mv

      ENDIF

      fai    = 1. - exp(-0.5*(lai+sai))
      sqrtdragc = min( (0.003+0.3*fai)**0.5, 0.3 )

      a_k71 = htop/(htop-displa)/(vonkar/sqrtdragc)

      taf = 0.5 * (tg + thm)
      qaf = 0.5 * (qm + qg)

      pco2a = pco2m
      tprcor = 44.6*273.16*psrf/1.013e5
      rsoil = 0.  !respiration (mol m-2 s-1)
!     rsoil = 1.22e-6*exp(308.56*(1./56.02-1./(tg-227.13)))
!     rsoil = rstfac * 0.23 * 15. * 2.**((tg-273.16-10.)/10.) * 1.e-6
!     rsoil = 5.22 * 1.e-6
      rsoil = 0.22 * 1.e-6

      ur  = max(0.1, sqrt(us*us+vs*vs))    !limit set to 0.1
      dth = thm - taf
      dqh = qm  - qaf
      dthv  = dth*(1.+0.61*qm) + 0.61*th*dqh

      hu_ = hu; ht_ = ht; hq_ = hq;

      IF (trim(HEIGHT_mode) == 'absolute') THEN

         IF (hu <= htop+1) THEN
            hu_ = htop + 1.
            IF (taux == spval) & ! only print warning for the first time-step
               write(6,*) 'Warning: the obs height of u less than htop+1, set it to htop+1.'
         ENDIF

         IF (ht <= htop+1) THEN
            ht_ = htop + 1.
            IF (taux == spval) & ! only print warning for the first time-step
               write(6,*) 'Warning: the obs height of t less than htop+1, set it to htop+1.'
         ENDIF

         IF (hq <= htop+1) THEN
            hq_ = htop + 1.
            IF (taux == spval) & ! only print warning for the first time-step
               write(6,*) 'Warning: the obs height of q less than htop+1, set it to htop+1.'
         ENDIF

      ELSE ! relative height
         hu_ = htop + hu
         ht_ = htop + ht
         hq_ = htop + hq
      ENDIF

      zldis = hu_ - displa

      IF(zldis <= 0.0) THEN
         write(6,*) 'the obs height of u less than the zero displacement heght'
         CALL abort
      ENDIF

      CALL moninobukini(ur,th,thm,thv,dth,dqh,dthv,zldis,z0mv,um,obu)

! ======================================================================
!     BEGIN stability iteration
! ======================================================================

      DO WHILE (it .le. itmax)

         tlbef = tl

         del2  = del
         dele2 = dele

         IF (tl > tfrz) THEN
            htvpl = hvap
         ELSE
            htvpl = hsub
         ENDIF

!-----------------------------------------------------------------------
! Aerodynamical resistances
!-----------------------------------------------------------------------
! Evaluate stability-dependent variables using moz from prior iteration
         IF (rd_opt == 3) THEN
            IF (DEF_USE_CBL_HEIGHT) THEN
              CALL moninobukm_leddy(hu_,ht_,hq_,displa,z0mv,z0hv,z0qv,obu,um, &
                                    displasink,z0mv,hpbl,ustar,fh2m,fq2m, &
                                    htop,fmtop,fm,fh,fq,fht,fqt,phih)
            ELSE
              CALL moninobukm(hu_,ht_,hq_,displa,z0mv,z0hv,z0qv,obu,um, &
                              displasink,z0mv,ustar,fh2m,fq2m, &
                              htop,fmtop,fm,fh,fq,fht,fqt,phih)
            ENDIF
            ! Aerodynamic resistance
            ram = 1./(ustar*ustar/um)
            rah = 1./(vonkar/(fh-fht)*ustar)
            raw = 1./(vonkar/(fq-fqt)*ustar)
         ELSE
            IF (DEF_USE_CBL_HEIGHT) THEN
               CALL moninobuk_leddy(hu_,ht_,hq_,displa,z0mv,z0hv,z0qv,obu,um,hpbl, &
                                    ustar,fh2m,fq2m,fm10m,fm,fh,fq)
            ELSE
               CALL moninobuk(hu_,ht_,hq_,displa,z0mv,z0hv,z0qv,obu,um,&
                              ustar,fh2m,fq2m,fm10m,fm,fh,fq)
            ENDIF
            ! Aerodynamic resistance
            ram = 1./(ustar*ustar/um)
            rah = 1./(vonkar/fh*ustar)
            raw = 1./(vonkar/fq*ustar)
         ENDIF

         z0hg = z0mg/exp(0.13 * (ustar*z0mg/1.5e-5)**0.45)
         z0qg = z0hg

! Bulk boundary layer resistance of leaves
          uaf = ustar
          cf = 0.01*sqrtdi/sqrt(uaf)
          rb = 1/(cf*uaf)

! 11/17/2017, yuan: 3D rb calculation (with vertical profile consideration)
! 03/13/2020, yuan: added analytical solution
         IF (rb_opt == 3) THEN
            utop = ustar/vonkar * fmtop
            ueff = ueffect(utop, htop, z0mg, z0mg, a_k71, 1._r8, 1._r8)
            cf = 0.01*sqrtdi*sqrt(ueff)
            rb = 1./cf
         ENDIF

!        rd = 1./(csoilc*uaf)                 ! BATS legacy
!        w = exp(-0.5*(lai+sai))              ! Dickinson's modification :
!        csoilc = ( 1.-w + w*um/uaf)/rah      ! "rah" here is the resistance over
!        rd = 1./(csoilc*uaf)                 ! bare ground fraction

! modified by Xubin Zeng's suggestion at 08-07-2002
         w = exp(-(lai+sai))
         csoilcn = (vonkar/(0.13*(z0mg*uaf/1.5e-5)**0.45))*w + csoilc*(1.-w)
         rd = 1./(csoilcn*uaf)

! 11/17/2017, yuan: 3D rd calculation with vertical profile solution
! 03/13/2020, yuan: added analytical solution
         IF (rd_opt == 3) THEN
            ktop = vonkar * (htop-displa) * ustar / phih
            rd = frd(ktop, htop, z0qg, hsink, z0qg, displa/htop, &
               z0qg, obug, ustar, z0mg, a_k71, 1._r8, 1._r8)
         ENDIF

!-----------------------------------------------------------------------
! stomatal resistances
!-----------------------------------------------------------------------

         IF(lai .gt. 0.001) THEN

            eah = qaf * psrf / ( 0.622 + 0.378 * qaf )    !pa

            ! If use PHS, calculate maximum stomata conductance (minimum stomata resistance)
            ! by setting rstfac = 1. (no water stress). When use PHS, stomata only calculate
            ! non-stress stomata conductance, assimilation rate and leaf respiration
            IF (DEF_USE_PLANTHYDRAULICS) THEN
                 rstfacsun = 1.
                 rstfacsha = 1.
            ENDIF

            ! leaf to canopy level
            rbsun = rb / laisun
            rbsha = rb / laisha

            ! Sunlit leaves
            CALL stomata  (vmax25   ,effcon   ,c3c4     ,slti     ,hlti     ,&
                 shti     ,hhti     ,trda     ,trdm     ,trop     ,&
                 g1       ,g0       ,gradm    ,binter   ,thm      ,&
                 psrf     ,po2m     ,pco2m    ,pco2a    ,eah      ,&
                 ei       ,tl       ,parsun   ,&
            !Ozone stress variables
                 o3coefv_sun   ,o3coefg_sun   ,&
            !End ozone stress variables
            !Ozone WUE stomata model parameter
                 lambda   ,&
            !End WUE stomata model parameter
                 rbsun    ,raw      ,rstfacsun,cintsun  ,&
                 assimsun ,respcsun ,rssun    )

            ! Shaded leaves
            CALL stomata  (vmax25   ,effcon   ,c3c4     ,slti     ,hlti     ,&
                 shti     ,hhti     ,trda     ,trdm     ,trop     ,&
                 g1       ,g0       ,gradm    ,binter   ,thm      ,&
                 psrf     ,po2m     ,pco2m    ,pco2a    ,eah      ,&
                 ei       ,tl       ,parsha   ,&
            ! Ozone stress variables
                 o3coefv_sha    ,o3coefg_sha  ,&
            ! End ozone stress variables
            ! Ozone WUE stomata model parameter
                 lambda   ,&
            ! End WUE stomata model parameter
                 rbsha    ,raw      ,rstfacsha,cintsha  ,&
                 assimsha ,respcsha ,rssha    )

            IF (DEF_USE_PLANTHYDRAULICS) THEN

               gs0sun = min( 1.e6, 1./(rssun*tl/tprcor) )/ laisun * 1.e6 * o3coefg_sun
               gs0sha = min( 1.e6, 1./(rssha*tl/tprcor) )/ laisha * 1.e6 * o3coefg_sha

               IF (DEF_Interception_scheme == 6) THEN
                  wetfrac_VIC = wetfrac_VIC_base
                  wet_area    = 1._r8
                  ec_pot_VIC  = rhoair * wetfrac_VIC * wet_area/rb &
                              * max(0._r8, qsatl - qaf)
                  IF (ec_pot_VIC * deltim > 1.e-10_r8 .AND. ldew_VIC_evap > 0._r8) THEN
                     f_VIC = min(1.0_r8, ldew_VIC_evap / (ec_pot_VIC * deltim))
                  ELSE
                     f_VIC = 1.0_r8
                  ENDIF
                  fwet = f_VIC * wetfrac_VIC
               ELSEIF (DEF_Interception_scheme == 7) THEN
                  ! Keep JULES wet-canopy suppression consistent between
                  ! the thermal Ec branch and the existing single-fwet PHS path.
                  sigf_safe_JULES = max(sigf, 0.01_r8)
                  ! DEF_VEG_SNOW coordinate: upstream makes
                  ! lai=tlai*sigf when DEF_VEG_SNOW=T (CoLMMAIN.F90:2083/2099),
                  ! but the per-veg catch_JULES / fraca_JULES coordinate
                  ! expects un-scaled LAI. Divide back out for the catch
                  ! formulas so (ldew/sigf)/catch stays per-veg/per-veg.
                  IF (DEF_VEG_SNOW) THEN
                     lai_perveg_JULES = lai / sigf_safe_JULES
                  ELSE
                     lai_perveg_JULES = lai
                  ENDIF
                  ! Rain capacity (0.5+0.05*LAI ≈ 1 mm)
                  ! used to be applied to TOTAL ldew (rain+snow). With
                  ! snow capacity 22× larger (4.4*LAI), fraca_JULES would
                  ! hit 1 too easily when ldew_snow is large, inflating Ec
                  ! (routed to qsubl when tl<tfrz). Split per phase, then
                  ! combine as independent probabilities.
                  catch_JULES      = 0.5_r8 + 0.05_r8 * lai_perveg_JULES
                  catch_JULES_snow = 4.4_r8 * lai_perveg_JULES
                  epot_JULES  = rhoair / max(raw, 1.e-10_r8) * (qsatl - qaf)
                  IF (epot_JULES > 0._r8) THEN
                     epdt_JULES = epot_JULES * deltim
                     IF (ldew_JULES_rain_evap > 0._r8 .AND. (epdt_JULES + catch_JULES) > 1.e-10_r8) THEN
                        fraca_JULES_rain = min((ldew_JULES_rain_evap / sigf_safe_JULES) / &
                                                   (epdt_JULES + catch_JULES), 1.0_r8)
                     ELSE
                        fraca_JULES_rain = 0.0_r8
                     ENDIF
                     IF (ldew_snow > 0._r8 .AND. (epdt_JULES + catch_JULES_snow) > 1.e-10_r8) THEN
                        fraca_JULES_snow = min((ldew_snow / sigf_safe_JULES) / (epdt_JULES + catch_JULES_snow), 1.0_r8)
                     ELSE
                        fraca_JULES_snow = 0.0_r8
                     ENDIF
                  ELSE
                     fraca_JULES_rain = 1.0_r8
                     fraca_JULES_snow = 1.0_r8
                  ENDIF
                  fraca_JULES = fraca_JULES_rain + fraca_JULES_snow &
                              - fraca_JULES_rain * fraca_JULES_snow
                  fwet = fraca_JULES
               ENDIF

               sai = amax1(sai,0.1)
               ! PHS update actual stomata conductance (resistance), assimilation rate
               ! and leaf respiration. above stomatal resistances are for the canopy,
               ! the stomatal resistances and the "rb" in the following calculations are
               ! the average for single leaf. thus,
               CALL PlantHydraulicStress_twoleaf (       nl_soil    ,nvegwcs    ,&
                     z_soi      ,dz_soi     ,rootfr     ,psrf       ,qsatl      ,&
                     qaf        ,tl         ,rb         ,rss        ,raw        ,&
                     rd         ,rstfacsun  ,rstfacsha  ,cintsun    ,cintsha    ,&
                     laisun     ,laisha     ,rhoair     ,fwet       ,sai        ,&
                     kmax_sun   ,kmax_sha   ,kmax_xyl   ,kmax_root  ,psi50_sun  ,&
                     psi50_sha  ,psi50_xyl  ,psi50_root ,htop       ,ck         ,&
                     smp        ,hk         ,hksati     ,vegwp      ,etrsun     ,&
                     etrsha     ,rootflux   ,qg         ,qm         ,gs0sun     ,&
                     gs0sha     ,k_soil_root,k_ax_root  ,gssun      ,gssha       )

               etr  = etrsun + etrsha
               gssun = gssun * laisun
               gssha = gssha * laisha

               CALL update_photosyn(tl, po2m, pco2m, pco2a, parsun, psrf, rstfacsun, rb, gssun, &
                                    effcon, vmax25, c3c4, gradm, trop, slti, hlti, shti, hhti, trda, &
                                    trdm, cintsun, assimsun, respcsun)

               CALL update_photosyn(tl, po2m, pco2m, pco2a, parsha, psrf, rstfacsha, rb, gssha, &
                                    effcon, vmax25, c3c4, gradm, trop, slti, hlti, shti, hhti, trda, &
                                    trdm, cintsha, assimsha, respcsha)

               rssun = tprcor/tl * 1.e6 / gssun
               rssha = tprcor/tl * 1.e6 / gssha
            ENDIF

         ELSE
            rssun = 2.e20; assimsun = 0.; respcsun = 0.
            rssha = 2.e20; assimsha = 0.; respcsha = 0.
            gssun = 0._r8
            gssha = 0._r8

            ! 07/2023, yuan: a bug for imbalanced water, rootflux only change
            ! in DEF_USE_PLANTHYDRAULICS case in this routine.
            IF (DEF_USE_PLANTHYDRAULICS) THEN
               etr    = 0.
               etrsun = 0._r8
               etrsha = 0._r8
               rootflux = 0.
            ENDIF
         ENDIF

! above stomatal resistances are for the canopy, the stomatal resistances
! and the "rb" in the following calculations are the average for single leaf. thus,
         rssun = rssun * laisun
         rssha = rssha * laisha

!-----------------------------------------------------------------------
! dimensional and non-dimensional sensible and latent heat conductances
! for canopy and soil flux calculations.
!-----------------------------------------------------------------------

         delta = 0.0
         IF(qsatl-qaf .gt. 0.) delta = 1.0

         cah = 1. / rah
         cgh = 1. / rd
         cfh = (lai + sai) / rb

         caw = 1. / raw
         IF (qg < qaf) THEN
            cgw = 1. / rd !dew case. no soil resistance
         ELSE
            IF (DEF_RSS_SCHEME .eq. 4) THEN
               cgw = rss / rd
            ELSE
               cgw = 1. / (rd + rss)
            ENDIF
         ENDIF

         ! -----------------------------------------------------------
         ! Compute scheme-specific evp_weight / dry_factor BEFORE etr.
         ! cfw follows the same scheme-specific wet-area semantics used by
         ! PlantHydraulicStress_twoleaf so the outer qaf solve stays aligned.
         ! -----------------------------------------------------------
         wet_area_cfw = lai + sai
         wet_cond_cfw = wet_area_cfw / rb

         IF (DEF_Interception_scheme == 6) THEN
            ! Upstream VIC wet-canopy evaporation is a bulk Penman flux:
            ! no extra LAI area multiplier beyond Wdmax = 0.1*LAI.
            wet_area = 1._r8
            wet_area_cfw = wet_area
            wet_cond = wet_area / rb
            wet_cond_cfw = wet_cond
            IF (DEF_USE_PLANTHYDRAULICS) THEN
               evp_weight = fwet
            ELSE
               wetfrac_VIC = wetfrac_VIC_base
               ec_pot_VIC  = rhoair * wetfrac_VIC * wet_area / rb &
                           * max(0._r8, qsatl - qaf)
               IF (ec_pot_VIC * deltim > 1.e-10_r8 .AND. ldew_VIC_evap > 0._r8) THEN
                  f_VIC = min(1.0_r8, ldew_VIC_evap / (ec_pot_VIC * deltim))
               ELSE
                  f_VIC = 1.0_r8
               ENDIF
               evp_weight = f_VIC * wetfrac_VIC
               fwet = evp_weight
            ENDIF
            dry_factor = 1._r8 - fwet                    ! PHS-compatible

         ELSEIF (DEF_Interception_scheme == 7) THEN
            ! Upstream JULES uses bulk fraca/ecan with aerodynamic exchange,
            ! not a leaf-boundary wet conductance based on rb.
            wet_area = 1._r8
            wet_area_cfw = wet_area
            wet_cond = 1._r8 / max(raw, 1.e-10_r8)
            wet_cond_cfw = wet_cond
            IF (DEF_USE_PLANTHYDRAULICS) THEN
               evp_weight = fwet
            ELSE
               sigf_safe_JULES = max(sigf, 0.01_r8)
               ! See PHS branch above for rationale.
               IF (DEF_VEG_SNOW) THEN
                  lai_perveg_JULES = lai / sigf_safe_JULES
               ELSE
                  lai_perveg_JULES = lai
               ENDIF
               catch_JULES      = 0.5_r8 + 0.05_r8 * lai_perveg_JULES
               catch_JULES_snow = 4.4_r8 * lai_perveg_JULES
               ! Use canopy-air qaf, not atmospheric qm (applied symmetrically to JULES),
               ! so single-layer and PC versions (MOD_LeafTemperaturePC.F90:1580) agree.
               epot_JULES  = rhoair * wet_cond * (qsatl - qaf)
               IF (epot_JULES > 0._r8) THEN
                  epdt_JULES = epot_JULES * deltim
                  IF (ldew_JULES_rain_evap > 0._r8 .AND. (epdt_JULES + catch_JULES) > 1.e-10_r8) THEN
                     fraca_JULES_rain = min((ldew_JULES_rain_evap / sigf_safe_JULES) / &
                                                (epdt_JULES + catch_JULES), 1.0_r8)
                  ELSE
                     fraca_JULES_rain = 0.0_r8
                  ENDIF
                  IF (ldew_snow > 0._r8 .AND. (epdt_JULES + catch_JULES_snow) > 1.e-10_r8) THEN
                     fraca_JULES_snow = min((ldew_snow / sigf_safe_JULES) / (epdt_JULES + catch_JULES_snow), 1.0_r8)
                  ELSE
                     fraca_JULES_snow = 0.0_r8
                  ENDIF
               ELSE
                  fraca_JULES_rain = 1.0_r8
                  fraca_JULES_snow = 1.0_r8
               ENDIF
               fraca_JULES = fraca_JULES_rain + fraca_JULES_snow &
                           - fraca_JULES_rain * fraca_JULES_snow
               evp_weight = fraca_JULES
               fwet = evp_weight
            ENDIF
            dry_factor = 1._r8 - fwet                    ! JULES: Ec/Et independent

         ELSEIF (DEF_Interception_scheme == 4) THEN
            ! NoahMP applies VegFrac to the grid-cell flux, but not to its
            ! within-canopy humidity conductance.
            wet_area_cfw = min(6._r8, max(lai + sai, 0._r8))
            wet_area = max(0.05_r8, 1._r8 - exp(-0.52_r8 * &
                       max(lai + sai, 0._r8))) * wet_area_cfw
            wet_cond = wet_area / rb
            wet_cond_cfw = wet_area_cfw / rb
            evp_weight = 1._r8 - delta*(1._r8 - fwet)
            dry_factor = 1._r8 - fwet

         ELSEIF (DEF_Interception_scheme == 5) THEN
            ! MATSIRO canopy water is LAI-only throughout fctint/cwcap/fcwet.
            ! Keep both the wet-canopy output area and the cfw coupling area
            ! on min(1, LAI) so CoLM stays consistent with upstream MATSIRO.
            evp_weight = fwet
            wet_area   = min(1._r8, max(lai, 0._r8))
            wet_area_cfw = wet_area
            wet_cond = wet_area / rb
            wet_cond_cfw = wet_area_cfw / rb
            dry_factor = 1._r8 - fwet
         ELSE
            evp_weight = 1._r8 - delta*(1._r8 - fwet)
            wet_area = lai + sai
            wet_cond = wet_area / rb
            dry_factor = 1._r8 - fwet                    ! CoLM default
         ENDIF

         ! cfw uses the same scheme-specific wet conductance semantics as the
         ! wet-canopy flux itself so the qaf solve stays consistent.
         IF (DEF_Interception_scheme == 5) THEN
            cfw = fwet*wet_cond_cfw + (1.-fwet)*delta* &
                  ( laisun/(rb+rssun) + laisha/(rb+rssha) )
         ELSE
            cfw = (1.-delta*(1.-fwet))*wet_cond_cfw + (1.-fwet)*delta* &
                  ( laisun/(rb+rssun) + laisha/(rb+rssha) )
         ENDIF

         wtshi = 1. / ( cah + cgh + cfh )
         wtsqi = 1. / ( caw + cgw + cfw )

         wta0 = cah * wtshi
         wtg0 = cgh * wtshi
         wtl0 = cfh * wtshi

         wtaq0 = caw * wtsqi
         wtgq0 = cgw * wtsqi
         wtlq0 = cfw * wtsqi

!-----------------------------------------------------------------------
! IR radiation, sensible and latent heat fluxes and their derivatives
!-----------------------------------------------------------------------
! the partial derivatives of areodynamical resistance are ignored
! which cannot be determined analytically
         fac = 1. - thermk

! longwave absorption and their derivatives
         ! 10/16/2017, yuan: added reflected longwave by the ground

IF (.not.DEF_SPLIT_SOILSNOW) THEN
         irab = (frl - 2. * stefnc * tl**4 + emg*stefnc*tg**4 ) * fac &
              + (1-emg)*thermk*fac*frl + (1-emg)*(1-thermk)*fac*stefnc*tl**4
ELSE
         irab = (frl - 2. * stefnc * tl**4 &
              + (1.-fsno)*emg*stefnc*t_soil**4 &
              + fsno*emg*stefnc*t_snow**4 ) * fac &
              + (1-emg)*thermk*fac*frl + (1-emg)*(1-thermk)*fac*stefnc*tl**4
ENDIF
         dirab_dtl = - 8. * stefnc * tl**3 * fac &
                   + 4.*(1-emg)*(1-thermk)*fac*stefnc*tl**3

! sensible heat fluxes and their derivatives
         fsenl = rhoair * cpair * cfh * ( (wta0 + wtg0)*tl - wta0*thm - wtg0*tg )
         fsenl_dtl = rhoair * cpair * cfh * (wta0 + wtg0)

! latent heat fluxes and their derivatives
         ! evp_weight / wet_area / dry_factor were already computed above,
         ! before cfw, so canopy-air humidity coupling and Ec/Tr stay aligned.

         etr = rhoair * dry_factor * delta &
             * ( laisun/(rb+rssun) + laisha/(rb+rssha) ) &
             * ( (wtaq0 + wtgq0)*qsatl - wtaq0*qm - wtgq0*qg )

         etrsun = rhoair * dry_factor * delta &
             * ( laisun/(rb+rssun) ) * ( (wtaq0 + wtgq0)*qsatl - wtaq0*qm - wtgq0*qg )
         etrsha = rhoair * dry_factor * delta &
             * ( laisha/(rb+rssha) ) * ( (wtaq0 + wtgq0)*qsatl - wtaq0*qm - wtgq0*qg )

         etr_dtl = rhoair * dry_factor * delta &
                 * ( laisun/(rb+rssun) + laisha/(rb+rssha) ) &
                 * (wtaq0 + wtgq0)*qsatlDT

         IF (.not. DEF_USE_PLANTHYDRAULICS) THEN
            IF(etr.ge.etrc)THEN
               etr = etrc
               etr_dtl = 0.
            ENDIF
         ELSE
            IF(rstfacsun .lt. 1.e-2 .or. etrsun .le. 0.)etrsun = 0._r8
            IF(rstfacsha .lt. 1.e-2 .or. etrsha .le. 0.)etrsha = 0._r8
            etr = etrsun + etrsha
            CALL balance_phs_rootflux(ipatch, p_iam_glb, etr, rootflux, rootfr, &
                                      'post-PHS')
         ENDIF

         ! evp_weight was computed above (before etr). Use it directly here.
         evplwet = rhoair * evp_weight * wet_cond &
                 * ( (wtaq0 + wtgq0)*qsatl - wtaq0*qm - wtgq0*qg )
         evplwet_dtl = rhoair * evp_weight * wet_cond &
                     * (wtaq0 + wtgq0)*qsatlDT

         IF(evplwet.ge.ldew_VIC_evap/deltim)THEN
            evplwet = ldew_VIC_evap/deltim
            evplwet_dtl = 0.
         ENDIF

         fevpl = etr + evplwet
         fevpl_dtl = etr_dtl + evplwet_dtl

         ! 07/09/2014, yuan: added for energy balance
         erre = 0.
         fevpl_noadj = fevpl
         IF ( fevpl*fevpl_bef < 0. ) THEN
            erre  = -0.9*fevpl
            fevpl =  0.1*fevpl
         ENDIF

!-----------------------------------------------------------------------
! difference of temperatures by quasi-newton-raphson method for the non-linear system equations
! MARK#dtl
!-----------------------------------------------------------------------

         ! Use htvpl (= hvap if tl>tfrz, else hsub) instead
         ! of hardcoded hvap so canopy snow sublimation is energetically
         ! accounted for at L_sublim (~2.83e6 J/kg) vs L_vap (~2.5e6 J/kg).
         ! htvpl is recomputed at the start of each iteration (L599-602)
         ! against the current tl.
         ! qintr_rain/qintr_snow are NET storage-rate
         ! fluxes and can go negative (canopy unloading / phase-change
         ! overflow / thin-storage cutoff) in scheme=6. The enthalpy term
         ! cpliq*qintr*(t_precip-tl) only represents incoming precipitation
         ! at t_precip; outgoing water leaves at tl, contributing 0.
         ! Clamp to max(0,·) so negative net flux does not spuriously
         ! inject t_precip-tl energy.
         dtl(it) = (sabv + irab - fsenl - htvpl*fevpl + canopy_phase_heat &
                 + cpliq*max(0._r8,qintr_rain)*(t_precip-tl) &
                 + cpice*max(0._r8,qintr_snow)*(t_precip-tl)) &
                 / (clai/deltim - dirab_dtl + fsenl_dtl + htvpl*fevpl_dtl  &
                 + cpliq*max(0._r8,qintr_rain) + cpice*max(0._r8,qintr_snow))

         dtl_noadj = dtl(it)

         ! check magnitude of change in leaf temperature limit to maximum allowed value

         ! 06/12/2014, yuan: .lt. -> .le.
         IF(it .le. itmax) THEN

         ! put brakes on large temperature excursions
           IF(abs(dtl(it)).gt.delmax)THEN
               dtl(it) = delmax*dtl(it)/abs(dtl(it))
           ENDIF

         ! 06/12/2014, yuan: .lt. -> .le.
         ! NOTE: could be a bug IF dtl*dtl==0, changed from lt->le
           IF((it.ge.2) .and. (dtl(it-1)*dtl(it).le.0.))THEN
               dtl(it) = 0.5*(dtl(it-1) + dtl(it))
           ENDIF

         ENDIF

         tl = tlbef + dtl(it)

!-----------------------------------------------------------------------
! square roots differences of temperatures and fluxes for use as the condition of convergences
!-----------------------------------------------------------------------

         del  = sqrt( dtl(it)*dtl(it) )
         dele = dtl(it) * dtl(it) * ( dirab_dtl**2 + fsenl_dtl**2 + (htvpl*fevpl_dtl)**2 )   ! NM-3
         dele = sqrt(dele)

!-----------------------------------------------------------------------
!  saturated vapor pressures and canopy air temperature, canopy air humidity
!-----------------------------------------------------------------------
! Recalculate leaf saturated vapor pressure (ei_)for updated leaf temperature
! and adjust specific humidity (qsatl_) proportionately
         CALL qsadv(tl,psrf,ei,deiDT,qsatl,qsatlDT)

! update vegetation/ground surface temperature, canopy air temperature,
! canopy air humidity
         taf = wta0*thm + wtg0*tg + wtl0*tl
         qaf = wtaq0*qm + wtgq0*qg + wtlq0*qsatl

! update co2 partial pressure within canopy air
         gah2o = 1.0/raw * tprcor/thm                     !mol m-2 s-1
         IF (DEF_RSS_SCHEME .eq. 4) THEN
             gdh2o = rss/rd  * tprcor/thm                 !mol m-2 s-1
         ELSE
             gdh2o = 1.0/(rd+rss)  * tprcor/thm           !mol m-2 s-1
         ENDIF
         pco2a = pco2m - 1.37*psrf/max(0.446,gah2o) * &
                (assimsun + assimsha  - respcsun -respcsha - rsoil)

!-----------------------------------------------------------------------
! Update monin-obukhov length and wind speed including the stability effect
!-----------------------------------------------------------------------

         dth = thm - taf
         dqh = qm  - qaf

         tstar = vonkar/(fh-fht)*dth
         qstar = vonkar/(fq-fqt)*dqh

         thvstar = tstar*(1.+0.61*qm)+0.61*th*qstar
         zeta = zldis*vonkar*grav*thvstar / (ustar**2*thv)
         IF(zeta .ge. 0.)THEN                             !stable
            zeta = min(2.,max(zeta,1.e-6))
         ELSE                                             !unstable
            zeta = max(-100.,min(zeta,-1.e-6))
         ENDIF
         obu = zldis/zeta

         IF(zeta .ge. 0.)THEN
           um = max(ur,.1)
         ELSE
           IF (DEF_USE_CBL_HEIGHT) THEN !//TODO: Shaofeng, 2023.05.18
             zii = max(5.*hu_,hpbl)
           ENDIF !//TODO: Shaofeng, 2023.05.18
           wc = (-grav*ustar*thvstar*zii/thv)**(1./3.)
          wc2 = beta*beta*(wc*wc)
           um = sqrt(ur*ur+wc2)
         ENDIF

         IF(obuold*obu .lt. 0.) nmozsgn = nmozsgn+1
         IF(nmozsgn .ge. 4) obu = zldis/(-0.01)
         obuold = obu

!-----------------------------------------------------------------------
! Test for convergence
!-----------------------------------------------------------------------

         it = it+1

         IF(it .gt. itmin) THEN
            fevpl_bef = fevpl
            det = max(del,del2)
            ! 10/03/2017, yuan: possible bugs here, solution:
            ! define dee, change del => dee
            dee = max(dele,dele2)
            IF(det .lt. dtmin .and. dee .lt. dlemin) EXIT
         ENDIF

      ENDDO

      IF(DEF_USE_OZONESTRESS)THEN
         CALL CalcOzoneStress(o3coefv_sun,o3coefg_sun,forc_ozone,psrf,th,ram,&
                              rssun,rb,lai,lai_old,ivt,o3uptakesun,sabv,deltim)
         CALL CalcOzoneStress(o3coefv_sha,o3coefg_sha,forc_ozone,psrf,th,ram,&
                              rssha,rb,lai,lai_old,ivt,o3uptakesha,sabv,deltim)
         lai_old  = lai
         assimsun = assimsun * o3coefv_sun
         assimsha = assimsha * o3coefv_sha
!         rssun    = rssun / o3coefg_sun
!         rssha    = rssha / o3coefg_sha
      ELSE
         o3coefv_sun = 1.0_r8
         o3coefg_sun = 1.0_r8
         o3coefv_sha = 1.0_r8
         o3coefg_sha = 1.0_r8
      ENDIF

! ======================================================================
!     END stability iteration
! ======================================================================

      z0m = z0mv
      zol = zeta
      rib = min(5.,zol*ustar**2/(vonkar**2/fh*um**2))

! canopy fluxes and total assimilation amd respiration

      IF(lai .gt. 0.001) THEN
         rst = 1./(laisun/rssun + laisha/rssha)
      ELSE
         rssun = 2.0e4 ; rssha = 2.0e4
         assimsun = 0. ; assimsha = 0.
         respcsun = 0. ; respcsha = 0.
         rst = 2.0e4
      ENDIF
      assim = assimsun + assimsha
      respc = respcsun + respcsha! + rsoil

! canopy fluxes and total assimilation amd respiration
      fsenl = fsenl + fsenl_dtl*dtl(it-1) &
            ! yuan: add the imbalanced energy below due to T adjustment to sensible heat
            ! NM-3: htvpl reflects sublim vs vap latent heat per current tl.
            ! VIC-qintr: clamp qintr_* to max(0,·) — must match dtl denom.
            + (dtl_noadj-dtl(it-1)) * (clai/deltim - dirab_dtl + fsenl_dtl + htvpl*fevpl_dtl &
            + cpliq * max(0._r8,qintr_rain) + cpice * max(0._r8,qintr_snow)) &
            ! yuan: add the imbalanced energy below due to q adjustment to sensible heat
            + htvpl*erre

      etr0  = etr
      etr   = etr + etr_dtl*dtl(it-1)

         IF (DEF_USE_PLANTHYDRAULICS) THEN
            !TODO@yuan: rootflux may not be consistent with etr,
            !           water imbalance could happen.
            IF (abs(etr0) .ge. 1.e-15) THEN
                rootflux = rootflux * etr / etr0
            ELSE
                rootflux = rootflux + dz_soi / sum(dz_soi) * etr_dtl* dtl(it-1)
            ENDIF

            ! Keep the layer-summed root uptake consistent with canopy
            ! transpiration after the final temperature increment.  The
            ! implementation is isolated in extends/interception so the
            ! extended-interception PHS closure adjustment stays easy to audit.
            CALL balance_phs_rootflux(ipatch, p_iam_glb, etr, rootflux, rootfr, &
                                      'post-leaf-temperature')
         ENDIF

      evplwet = evplwet + evplwet_dtl*dtl(it-1)
      fevpl   = fevpl_noadj
      fevpl   = fevpl   +   fevpl_dtl*dtl(it-1)

      elwmax  = ldew_VIC_evap/deltim
      elwdif  = max(0., evplwet-elwmax)
      evplwet = min(evplwet, elwmax)

      fevpl = fevpl - elwdif
      fsenl = fsenl + htvpl*elwdif   ! NM-3: latent excess redirect uses scheme-correct latent heat

      taux = - rhoair*us/ram
      tauy = - rhoair*vs/ram

!-----------------------------------------------------------------------
! fluxes from ground to canopy space
!-----------------------------------------------------------------------

      fseng = cpair*rhoair*cgh*(tg-taf)
! 03/07/2020, yuan: calculate fseng_soil/snow
      !NOTE: taf = wta0*thm + wtg0*tg + wtl0*tl
      fseng_soil = cpair*rhoair*cgh*((1.-wtg0)*t_soil - wta0*thm - wtl0*tl)
      fseng_snow = cpair*rhoair*cgh*((1.-wtg0)*t_snow - wta0*thm - wtl0*tl)

! 03/07/2020, yuan: calculate fevpg_soil/snow
      !NOTE: qaf = wtaq0*qm + wtgq0*qg + wtlq0*qsatl
      fevpg = rhoair*cgw*(qg-qaf)
      fevpg_soil = rhoair*cgw*((1.-wtgq0)*q_soil - wtaq0*qm - wtlq0*qsatl)
      fevpg_snow = rhoair*cgw*((1.-wtgq0)*q_snow - wtaq0*qm - wtlq0*qsatl)

!-----------------------------------------------------------------------
! downward (upward) longwave radiation below (above) the canopy and prec. sensible heat
!-----------------------------------------------------------------------

      ! 10/16/2017, yuan: added reflected longwave by the ground
      dlrad = thermk * frl &
            + stefnc * fac * tlbef**3 * (tlbef + 4.*dtl(it-1))

IF (.not.DEF_SPLIT_SOILSNOW) THEN
      ulrad = stefnc * ( fac * tlbef**3 * (tlbef + 4.*dtl(it-1)) &
            + thermk*emg*tg**4 ) &
            + (1-emg)*thermk*thermk*frl &
            + (1-emg)*thermk*fac*stefnc*tlbef**4 &
            + 4.*(1-emg)*thermk*fac*stefnc*tlbef**3*dtl(it-1)
ELSE
      ulrad = stefnc * ( fac * tlbef**3 * (tlbef + 4.*dtl(it-1)) &
            + (1.-fsno)*thermk*emg*t_soil**4 &
            + fsno*thermk*emg*t_snow**4 ) &
            + (1-emg)*thermk*thermk*frl &
            + (1-emg)*thermk*fac*stefnc*tlbef**4 &
            + 4.*(1-emg)*thermk*fac*stefnc*tlbef**3*dtl(it-1)
ENDIF
      ! precipitation sensible heat from canopy
      ! Clamp to max(0,·) — see dtl formula above.
      hprl   = cpliq * max(0._r8,qintr_rain)*(t_precip-tl) &
             + cpice * max(0._r8,qintr_snow)*(t_precip-tl)

      ! vegetation heat change
      dheatl = clai/deltim*dtl(it-1)

!-----------------------------------------------------------------------
! Derivative of soil energy flux with respect to soil temperature (cgrnd)
!-----------------------------------------------------------------------

      cgrnds = cpair*rhoair*cgh*(1.-wtg0)
      cgrndl = rhoair*cgw*(1.-wtgq0)*dqgdT
      cgrnd  = cgrnds + cgrndl*htvp

!-----------------------------------------------------------------------
! balance check
! (the computational error was created by the assumed 'dtl' in MARK#dtl)
!-----------------------------------------------------------------------

      ! Add canopy_phase_heat so the canopy energy
      ! residual accounts for fusion heat consumed/released by the canopy
      ! rain<->snow phase change that LEAF_interception performed inside
      ! schemes 4-7. Zero for other schemes / steps without canopy phase
      ! change.
      err = sabv + irab + dirab_dtl*dtl(it-1) - fsenl - htvpl*fevpl + hprl &
          + canopy_phase_heat &
          ! account for vegetation heat change
          - dheatl

#if (defined CoLMDEBUG)
      IF(abs(err) .gt. .2) &
      write(6,*) 'energy imbalance in LeafTemperature.F90',it-1,&
         err,sabv,irab,fsenl,htvpl*fevpl,hprl,dheatl,canopy_phase_heat
#endif

!-----------------------------------------------------------------------
! Update dew accumulation (kg/m2)
!-----------------------------------------------------------------------
      IF (DEF_Interception_scheme .ge. 1 .and. DEF_Interception_scheme .le. 8) THEN
         ldew = max(0., ldew-evplwet*deltim)

         ! account for vegetation snow and update ldew_rain, ldew_snow, ldew
         IF ( DEF_VEG_SNOW ) THEN
            CALL partition_canopy_latent_flux(tl, tfrz, deltim, evplwet, ldew_rain, ldew_snow, &
                                              qevpl, qdewl, qsubl, qfrol, phase_flux_deficit)
            IF (phase_flux_deficit > 0._r8) THEN
               evplwet = evplwet - phase_flux_deficit
               fevpl   = fevpl - phase_flux_deficit
               fsenl   = fsenl + htvpl * phase_flux_deficit   ! NM-3
            ENDIF

            ldew_rain = ldew_rain + (qdewl-qevpl)*deltim
            ldew_snow = ldew_snow + (qfrol-qsubl)*deltim
            ! Close the energy-balance loop when the
            ! cross-phase redirect drives a component below zero. The
            ! bulk elwmax clamp (L~1120) sizes evplwet against total
            ! ldew at entry, but component-level inconsistency (stale
            ! restart, or single-phase qsubl/qevpl cap) can still leave
            ! the flux over-committed vs actual phase storage. Convert
            ! the deficit back from latent to sensible heat, matching
            ! the L~1124-1125 elwdif pattern so mass and energy stay
            ! consistent with what the canopy could physically supply.
            IF (ldew_rain < 0._r8 .or. ldew_snow < 0._r8) THEN
               flux_deficit = max(0._r8, -ldew_rain) + max(0._r8, -ldew_snow)
               evplwet = evplwet - flux_deficit / deltim
               fevpl   = fevpl   - flux_deficit / deltim
               fsenl   = fsenl   + htvpl * flux_deficit / deltim   ! NM-3
            ENDIF
            ldew_rain = max(0._r8, ldew_rain)
            ldew_snow = max(0._r8, ldew_snow)

            ldew = ldew_rain + ldew_snow
         ENDIF
      ELSE
         CALL abort
      ENDIF

      ! When DEF_VEG_SNOW is false, only ldew is updated above
      ! (via ldew = max(0., ldew - evplwet*deltim)), but ldew_rain/ldew_snow
      ! remain unchanged. Downstream interception routines (schemes 1, 3-8)
      ! resync ldew = ldew_rain + ldew_snow at entry, which would silently
      ! revert the evaporation adjustment.
      !
      ! DEF_VEG_SNOW=false signals "do not track phase"
      ! — components only exist so that ldew_rain+ldew_snow == ldew holds
      ! at the scheme interface. We therefore reconcile components using
      ! the current temperature (tl) rather than scaling by an old ratio:
      ! this avoids long-term phase drift while keeping the total mass
      ! identical. Matches the no-veg-path phase-conservation behaviour.
      IF (.not. DEF_VEG_SNOW) THEN
         IF (tl > tfrz) THEN
            ldew_rain = ldew
            ldew_snow = 0.
         ELSE
            ldew_rain = 0.
            ldew_snow = ldew
         ENDIF
      ENDIF

      ! Canopy rain<->snow phase change is done by
      ! LEAF_interception itself for schemes 4/5/6/7 (NoahMP / MATSIRO /
      ! VIC / JULES), and the fusion heat has already been accounted via
      ! canopy_phase_heat in the err residual above. Running the qmelt /
      ! qfrz / tl-pull block below for those schemes would (a) do a
      ! second phase-change site on the residual ldew_snow / ldew_rain
      ! and (b) double-count the tl pull. Gate the block so it only runs
      ! for schemes where interception does NOT own the phase change.
      phase_change_owned_by_interception = &
         (DEF_Interception_scheme >= 4 .and. DEF_Interception_scheme <= 7)

      ! Default the optional phase-change exports to 0 so
      ! every return path leaves them well-defined for the caller.
      IF (present(canopy_smelt_mass_out)) canopy_smelt_mass_out = 0._r8
      IF (present(canopy_frzc_mass_out)) canopy_frzc_mass_out = 0._r8

      IF ( DEF_VEG_SNOW .and. .not. phase_change_owned_by_interception ) THEN
         ! update fwet_snow
         fwet_snow = canopy_snow_wetfrac(sigf, lai, sai, dewmx, tl, ldew_snow)

         ! phase change
         ! Capture tl change from Niu (2004) pull
         ! into dheatl so the canopy-side energy balance check at
         ! MOD_Thermal.F90:1366-1383 closes. Previously the post-iteration
         ! tl update silently leaked clai*(tl_post - tl_pre)/deltim from
         ! the energy budget (~3.5 W/m² in cold-canopy melt episodes),
         ! tripping errore > 0.5 W/m² when CoLMDEBUG was on.

         qmelt = 0.
         qfrz  = 0.

         !TODO: double check below
         IF (ldew_snow.gt.1.e-6 .and. tl.gt.tfrz) THEN
            qmelt = min(ldew_snow/deltim,(tl-tfrz)*cpice*ldew_snow/(deltim*hfus))
            ldew_snow = max(0.,ldew_snow - qmelt*deltim)
            ldew_rain = max(0.,ldew_rain + qmelt*deltim)
            tl_pre_phase = tl
            tl = fwet_snow*tfrz + (1.-fwet_snow)*tl  !Niu et al., 2004
            dheatl = dheatl + clai/deltim * (tl - tl_pre_phase)
            ! Export the actual mass that crossed (post the
            ! ldew_snow >= 0 clamp) so tracer_evapo can charge the right
            ! amount instead of inferring from d_rain/d_snow.
            IF (present(canopy_smelt_mass_out)) canopy_smelt_mass_out = qmelt * deltim
         ENDIF

         IF (ldew_rain.gt.1.e-6 .and. tl.lt.tfrz) THEN
            qfrz  = min(ldew_rain/deltim,(tfrz-tl)*cpliq*ldew_rain/(deltim*hfus))
            ldew_rain = max(0.,ldew_rain - qfrz*deltim)
            ldew_snow = max(0.,ldew_snow + qfrz*deltim)
            tl_pre_phase = tl
            tl = fwet_snow*tfrz + (1.-fwet_snow)*tl  !Niu et al., 2004
            dheatl = dheatl + clai/deltim * (tl - tl_pre_phase)
            IF (present(canopy_frzc_mass_out)) canopy_frzc_mass_out = qfrz * deltim
         ENDIF
      ELSEIF ( DEF_VEG_SNOW ) THEN
         ! Still need fwet_snow for downstream fwet consumers under
         ! DEF_VEG_SNOW=T, but the mass / energy phase-change transfer
         ! is owned by LEAF_interception for schemes 4/5/6/7.
         fwet_snow = canopy_snow_wetfrac(sigf, lai, sai, dewmx, tl, ldew_snow)
      ENDIF

      ! Export the canopy latent-energy term exactly as solved here.
      ! MOD_Thermal must not re-infer this from the post-adjustment tl.
      lfevpl = htvpl * fevpl

!-----------------------------------------------------------------------
! 2 m height air temperature
!-----------------------------------------------------------------------
      tref = thm + vonkar/(fh-fht)*dth * (fh2m/vonkar - fh/vonkar)
      qref =  qm + vonkar/(fq-fqt)*dqh * (fq2m/vonkar - fq/vonkar)
      IF (present(raw_trc_out)) raw_trc_out = max(raw, 0._r8)

   END SUBROUTINE LeafTemperature
!----------------------------------------------------------------------

   SUBROUTINE partition_canopy_latent_flux(tleaf, tfrz, deltim, evplwet, ldew_rain, ldew_snow, &
                                           qevpl, qdewl, qsubl, qfrol, phase_flux_deficit)

   USE MOD_Precision

   IMPLICIT NONE

   real(r8), intent(in)  :: tleaf
   real(r8), intent(in)  :: tfrz
   real(r8), intent(in)  :: deltim
   real(r8), intent(in)  :: evplwet
   real(r8), intent(in)  :: ldew_rain
   real(r8), intent(in)  :: ldew_snow
   real(r8), intent(out) :: qevpl
   real(r8), intent(out) :: qdewl
   real(r8), intent(out) :: qsubl
   real(r8), intent(out) :: qfrol
   real(r8), intent(out) :: phase_flux_deficit

      phase_flux_deficit = 0._r8
      IF (tleaf > tfrz) THEN
         qevpl = max(evplwet, 0._r8)
         qdewl = abs(min(evplwet, 0._r8))
         qsubl = 0._r8
         qfrol = 0._r8

         IF (qevpl > ldew_rain/deltim) THEN
            phase_flux_deficit = qevpl - ldew_rain/deltim
            qevpl = ldew_rain/deltim
         ENDIF
      ELSE
         qevpl = 0._r8
         qdewl = 0._r8
         qsubl = max(evplwet, 0._r8)
         qfrol = abs(min(evplwet, 0._r8))

         IF (qsubl > ldew_snow/deltim) THEN
            phase_flux_deficit = qsubl - ldew_snow/deltim
            qsubl = ldew_snow/deltim
         ENDIF
      ENDIF

   END SUBROUTINE partition_canopy_latent_flux

   SUBROUTINE dewfraction (sigf,lai,sai,dewmx,tleaf,ldew,ldew_rain,ldew_snow,fwet,fdry)
   !DESCRIPTION
   !===========
      ! determine fraction of foliage covered by water and
      ! fraction of foliage that is dry and transpiring

   !Original Author:
   !-------------------
      !---Yongjiu Dai

   !References:
   !-------------------
      !---Dai, Y., Zeng, X., Dickinson, R.E., Baker, I., Bonan, G.B., BosiloVICh, M.G., Denning,
      !   A.S., Dirmeyer, P.A., Houser, P.R., Niu, G. and Oleson, K.W., 2003.  The common land
      !   model. Bulletin of the American Meteorological Society, 84(8), pp.1013-1024.

   !ANCILLARY FUNCTIONS AND SUBROUTINES
   !-------------------

   !REVISION HISTORY
   !----------------
      !---2024.04.16  Hua Yuan: add option to account for vegetation snow process
      !---2021.12.08  Zhongwang Wei @ SYSU
      !---2018.06     Hua Yuan: remove sigf, to compatible with PFT
      !---1999.09.15  Yongjiu Dai
   !=======================================================================

   USE MOD_Precision
   ! dewfraction has its own USE MOD_Precision + IMPLICIT
   ! NONE block which shadows host association on some compilers
   ! (gfortran/Intel both flagged tfrz as implicit). Import tfrz
   ! explicitly here so the scheme=4 temperature tie-break resolves.
   USE MOD_Const_Physical, only: tfrz

   IMPLICIT NONE

   real(r8), intent(in)  :: sigf      !fraction of veg cover, excluding snow-covered veg [-]
   real(r8), intent(in)  :: lai       !leaf area index  [-]
   real(r8), intent(in)  :: sai       !stem area index  [-]
   real(r8), intent(in)  :: dewmx     !maximum allowed dew [0.1 mm]
   real(r8), intent(in)  :: tleaf     !canopy temperature used by scheme-specific snow capacity [K]
   real(r8), intent(in)  :: ldew      !depth of water on foliage [kg/m2/s]
   real(r8), intent(in)  :: ldew_rain !depth of rain on foliage [kg/m2/s]
   real(r8), intent(in)  :: ldew_snow !depth of snow on foliage [kg/m2/s]
   real(r8), intent(out) :: fwet      !fraction of foliage covered by water&snow [-]
   real(r8), intent(out) :: fdry      !fraction of foliage that is green and dry [-]

   real(r8) :: lsai                   !lai + sai
   real(r8) :: dewmxi                 !inverse of maximum allowed dew [1/mm]
   real(r8) :: vegt                   !sigf*lsai, NOTE: remove sigf
   real(r8) :: fwet_rain              !fraction of foliage covered by water [-]
   real(r8) :: fwet_snow              !fraction of foliage covered by snow [-]
   real(r8) :: satcap_rain_eff        !scheme-specific liquid capacity used by fwet [-]
   real(r8) :: lai_eff                !LAI clamped to avoid division by zero [-]
   real(r8) :: fwet_max               !scheme-specific cap on fwet [-]
   real(r8) :: dewmx_MATSIRO            ! MATSIRO canopy water capacity per LAI [mm]
   real(r8), parameter :: fwet_max_CLM5 = 0.05_r8 ! CLM5.0 max leaf wetted fraction, Lawrence et al. 2019

      !-----------------------------------------------------------------------
      ! Fwet is the fraction of all vegetation surfaces which are wet
      ! including stem area which contribute to evaporation
      lsai   = lai + sai ! effective leaf area index
      dewmxi = 1.0/dewmx
      ! 06/2018, yuan: remove sigf, to compatible with PFT
      vegt   =  lsai
      dewmx_MATSIRO = 0.2_r8 * DEF_MATSIRO_CWCAP_SCALE

      IF (DEF_Interception_scheme == 5) THEN
         ! ----------------------------------------------------------------
         ! MATSIRO scheme: linear fwet on TOTAL canopy water (rain+snow),
         ! using LAI only (not LAI+SAI). Follows original MATSIRO canwet():
         !   fcwet = glwc / (LAI * cnw_wcmax),  cnw_wcmax = 0.2 mm
         ! Original MATSIRO does not split rain/snow in fwet — cwcap is a
         ! single value and glwc = glwcl + glwci. So we ignore DEF_VEG_SNOW
         ! here and always use the combined ldew to stay faithful to the
         ! original scheme.
         ! ----------------------------------------------------------------
         lai_eff = max(lai, 1.e-6_r8)
         fwet = 0
         IF (ldew > 0.) THEN
            fwet = ldew / (lai_eff * dewmx_MATSIRO)
            fwet = min(fwet, 1.0)
         ENDIF

      ELSEIF (DEF_Interception_scheme == 4) THEN
         ! Noah-MP uses canopy ice whenever any is present; otherwise it uses
         ! canopy liquid water. Do not select the phase from leaf temperature
         ! or divide the combined store by a single-phase capacity.
         fwet = 0
         IF (ldew_snow > 0._r8) THEN
            satcap_rain_eff = canopy_snow_capacity_for_fwet(sigf, lai, sai, dewmx, tleaf)
            IF (satcap_rain_eff > 1.e-10_r8) THEN
               fwet = min((ldew_snow / satcap_rain_eff)**.666666666666_r8, 1.0_r8)
            ENDIF
         ELSEIF (ldew_rain > 0._r8) THEN
            satcap_rain_eff = canopy_rain_capacity_for_fwet(sigf, lai, sai, dewmx)
            IF (satcap_rain_eff > 1.e-10_r8) THEN
               fwet = min((ldew_rain / satcap_rain_eff)**.666666666666_r8, 1.0_r8)
            ENDIF
         ENDIF

      ELSE
         ! ----------------------------------------------------------------
         ! Default CoLM scheme: fwet = (S / Smax)^(2/3)
         ! CLM5 (scheme=3) additionally clamps fwet to 0.05 per
         ! CanopyHydrologyMod.F90 L722 (Lawrence et al. 2019, Fan et al. 2019)
         ! ----------------------------------------------------------------
         IF (DEF_Interception_scheme == 3) THEN
            fwet_max = fwet_max_CLM5
         ELSE
            fwet_max = 1.0_r8
         ENDIF

         fwet = 0
         IF (ldew > 0.) THEN
            fwet = ((dewmxi/vegt)*ldew)**.666666666666
            ! Check for maximum limit of fwet
            fwet = min(fwet, fwet_max)
         ENDIF

         ! account for vegetation snow
         ! calculate fwet_rain, fwet_snow, fwet
         ! CLM4 (scheme=2) uses a single
         ! shared canopy bucket (MOD_LeafInterception.F90:792,
         ! satcap = dewmx*vegt). The single-bucket fwet already
         ! computed above against dewmx*vegt is the correct value for
         ! scheme=2. Skip the double-bucket phase-split union formula
         ! for scheme=2 only — it would otherwise (a) compress fwet
         ! even when ldew==satcap, and (b) route snow through
         ! canopy_snow_capacity_for_fwet DEFAULT = 48*dewmx*lsai (no
         ! CASE(2)), compounding the under-estimation ~48x.
         IF ( (DEF_VEG_SNOW .or. DEF_Interception_scheme == 6) .and. &
              DEF_Interception_scheme /= 2 ) THEN

            fwet_rain = 0
            IF(ldew_rain > 0.) THEN
               satcap_rain_eff = canopy_rain_capacity_for_fwet(sigf, lai, sai, dewmx)
               IF (satcap_rain_eff > 1.e-10_r8) THEN
                  fwet_rain = (ldew_rain / satcap_rain_eff)**.666666666666
                  ! Check for maximum limit of fwet_rain
                  fwet_rain = min(fwet_rain, fwet_max)
               ENDIF
            ENDIF

            fwet_snow = 0
            IF(ldew_snow > 0.) THEN
               fwet_snow = canopy_snow_wetfrac(sigf, lai, sai, dewmx, tleaf, ldew_snow)
               fwet_snow = min(fwet_snow, fwet_max)
            ENDIF

            IF (DEF_Interception_scheme == 4) THEN
               IF (ldew_snow > 0._r8) THEN
                  fwet = fwet_snow
               ELSE
                  fwet = fwet_rain
               ENDIF
            ELSE
               fwet = fwet_rain + fwet_snow - fwet_rain*fwet_snow
            ENDIF
            fwet = min(fwet, fwet_max)
         ENDIF
      ENDIF

      ! fdry is the fraction of lai which is dry because only leaves can
      ! transpire. Adjusted for stem area which does not transpire
      fdry = (1.-fwet)*lai/lsai

   END SUBROUTINE dewfraction

   FUNCTION canopy_rain_capacity_for_fwet(sigf, lai, sai, dewmx) RESULT(satcap_rain_eff)

   USE MOD_Precision

   IMPLICIT NONE

   real(r8), intent(in) :: sigf
   real(r8), intent(in) :: lai
   real(r8), intent(in) :: sai
   real(r8), intent(in) :: dewmx
   real(r8)             :: satcap_rain_eff
   real(r8)             :: lsai
   real(r8)             :: fvegc
   real(r8)             :: sigf_safe
   real(r8)             :: lai_perveg

      lsai = max(lai + sai, 0._r8)
      sigf_safe = max(sigf, 0.01_r8)
      lai_perveg = lai
      IF (DEF_VEG_SNOW) lai_perveg = lai / sigf_safe

      SELECT CASE (DEF_Interception_scheme)
      CASE (4)
         fvegc = max(0.05_r8, 1.0_r8 - exp(-0.52_r8 * lsai))
         satcap_rain_eff = fvegc * dewmx * lsai
      CASE (6)
         ! VIC is evaluated inside the vegetation tile with F=1.
         satcap_rain_eff = 0.1_r8 * max(lai_perveg, 0._r8)
      CASE (7)
         ! Under DEF_VEG_SNOW the caller passes lai=tlai*sigf.
         ! JULES catch is per-veg (0.5 + 0.05*LAI_true); the grid-scale
         ! effective capacity used against grid-scale ldew is
         ! sigf * catch_perveg, so lai inside the (0.5+0.05*·) term must
         ! be un-scaled. Otherwise capacity shrinks as sigf² and fwet_rain
         ! is inflated.
         IF (DEF_VEG_SNOW) THEN
            satcap_rain_eff = sigf_safe * (0.5_r8 + 0.05_r8 * max(lai / sigf_safe, 0._r8))
         ELSE
            satcap_rain_eff = sigf_safe * (0.5_r8 + 0.05_r8 * max(lai, 0._r8))
         ENDIF
      CASE DEFAULT
         satcap_rain_eff = dewmx * lsai
      END SELECT

      satcap_rain_eff = max(satcap_rain_eff, 0._r8)

   END FUNCTION canopy_rain_capacity_for_fwet

   FUNCTION canopy_snow_capacity_for_fwet(sigf, lai, sai, dewmx, tleaf) RESULT(satcap_snow_eff)

   USE MOD_Precision

   IMPLICIT NONE

   real(r8), intent(in) :: sigf
   real(r8), intent(in) :: lai
   real(r8), intent(in) :: sai
   real(r8), intent(in) :: dewmx
   real(r8), intent(in) :: tleaf
   real(r8)             :: satcap_snow_eff
   real(r8)             :: lsai
   real(r8)             :: fvegc
   real(r8)             :: BDFALL
   real(r8)             :: Lr
   real(r8)             :: sigf_safe
   real(r8)             :: dewmx_MATSIRO
   real(r8)             :: lai_perveg

      lsai = max(lai + sai, 0._r8)
      sigf_safe = max(sigf, 0.01_r8)
      dewmx_MATSIRO = 0.2_r8 * DEF_MATSIRO_CWCAP_SCALE
      lai_perveg = lai
      IF (DEF_VEG_SNOW) lai_perveg = lai / sigf_safe

      SELECT CASE (DEF_Interception_scheme)
      CASE (3)
         satcap_snow_eff = 60._r8 * dewmx * lsai
      CASE (4)
         fvegc = max(0.05_r8, 1.0_r8 - exp(-0.52_r8 * lsai))
         BDFALL = 67.92_r8 + 51.25_r8 * exp(min(2.5_r8, (tleaf - 273.15_r8)) / 2.59_r8)
         satcap_snow_eff = fvegc * 6.6_r8*(0.27_r8 + 46._r8/BDFALL) * lsai
      CASE (5)
         satcap_snow_eff = dewmx_MATSIRO * max(lai, 0._r8)
      CASE (6)
         IF (tleaf > 272.15_r8) THEN
            Lr = 4.0_r8
         ELSEIF (tleaf >= 270.15_r8) THEN
            Lr = 1.5_r8 * (tleaf - 273.15_r8) + 5.5_r8
         ELSE
            Lr = 1.0_r8
         ENDIF
         satcap_snow_eff = 0.5_r8 * Lr * max(lai_perveg, 0._r8)
      CASE (7)
         ! See canopy_rain_capacity_for_fwet CASE(7) above.
         IF (DEF_VEG_SNOW) THEN
            satcap_snow_eff = sigf_safe * 4.4_r8 * max(lai / sigf_safe, 0._r8)
         ELSE
            satcap_snow_eff = sigf_safe * 4.4_r8 * max(lai, 0._r8)
         ENDIF
      CASE DEFAULT
         satcap_snow_eff = 48._r8 * dewmx * lsai
      END SELECT

      satcap_snow_eff = max(satcap_snow_eff, 0._r8)

   END FUNCTION canopy_snow_capacity_for_fwet

   FUNCTION canopy_snow_wetfrac(sigf, lai, sai, dewmx, tleaf, ldew_snow) RESULT(fwet_snow)

   USE MOD_Precision

   IMPLICIT NONE

   real(r8), intent(in) :: sigf
   real(r8), intent(in) :: lai
   real(r8), intent(in) :: sai
   real(r8), intent(in) :: dewmx
   real(r8), intent(in) :: tleaf
   real(r8), intent(in) :: ldew_snow
   real(r8)             :: fwet_snow
   real(r8)             :: satcap_snow_eff

      satcap_snow_eff = canopy_snow_capacity_for_fwet(sigf, lai, sai, dewmx, tleaf)
      fwet_snow = 0._r8
      IF (ldew_snow > 0._r8 .AND. satcap_snow_eff > 1.e-10_r8) THEN
         fwet_snow = min((ldew_snow / satcap_snow_eff)**(2.0_r8/3.0_r8), 1.0_r8)
      ENDIF

   END FUNCTION canopy_snow_wetfrac

   FUNCTION effective_canopy_area(area_in) RESULT(area_eff)
   ! Beer-Lambert saturation of the evaporating canopy area used in Ec.
   ! Fixes input/output asymmetry: CoLM interception already uses
   ! fpi = alpha*(1-exp(-k*LSAI)) and fvegc = 1-exp(-0.52*LSAI) on
   ! the capture side; this applies the same closure on the evap side
   ! so dense canopies (LAI>3) don't linearly inflate Ec.
   ! k=0.5 matches random leaf angle (Campbell & Norman 1998, Monteith
   ! 1965) and is consistent with the 0.52 used in fvegc for snow
   ! unloading — the standard Monteith extinction coefficient.
   ! Asymptote 1/k = 2.0 matches bulk wet-canopy conductance order
   ! (MATSIRO cdvc·rb_leaf ≈ 2, VIC rb_leaf/ra_bulk ≈ 2).
   USE MOD_Precision
   IMPLICIT NONE
   real(r8), intent(in) :: area_in
   real(r8)             :: area_eff
   real(r8), parameter  :: k_ext = 0.5_r8
      IF (area_in <= 0._r8) THEN
         area_eff = 0._r8
      ELSE
         area_eff = (1._r8 - exp(-k_ext * area_in)) / k_ext
      ENDIF
   END FUNCTION effective_canopy_area

END MODULE MOD_LeafTemperature
