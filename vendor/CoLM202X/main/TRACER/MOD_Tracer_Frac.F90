#include <define.h>

#ifdef TRACER
MODULE MOD_Tracer_Frac

   USE MOD_Precision
   USE MOD_Namelist, only: DEF_TRACER_USE_FRACTIONATION, &
      DEF_TRACER_NSS_LEAF_WATER_PER_LAI, DEF_TRACER_NSS_LEAF_PATH_LENGTH, &
      DEF_TRACER_NSS_LEAF_RB, DEF_TRACER_ICE_SUPERSAT_SLOPE, &
      DEF_TRACER_CG_RELHUM_MAX, DEF_TRACER_OPEN_WATER_KINETIC
   USE MOD_Tracer_Defs, only: tracers, ntracers, tracer_is_isotope, trc_tiny
   USE MOD_Tracer_Isotope_Registry, only: isotope_fractionation_registered, &
      isotope_alpha_liq_vap, isotope_alpha_ice_vap, isotope_diffusivity_ratio_air, &
      isotope_leaf_liquid_diffusivity, isotope_mj79_relative_factor
   USE MOD_Tracer_Isotope_Registrations, only: ensure_isotope_physics_registered

   IMPLICIT NONE
   SAVE

   real(r8), parameter :: tfrz = 273.15_r8
   real(r8), parameter :: water_moles_per_mm = 1000._r8 / 18.01528_r8
   real(r8), parameter :: liquid_water_molar_density = 55.5e3_r8
   real(r8), parameter :: universal_gas_constant = 8.31446261815324_r8
   ! Symmetric magnitude bound on the Craig-Gordon ratio, in units of the
   ! equilibrium vapour ratio.  This is a NUMERICAL guard only: the 1/(1-h)
   ! factor amplifies any inconsistency between the host's evaporation flux
   ! and the humidity diagnosed here, and near the humidity cap that
   ! amplification reaches 1/(1-h) ~ 100.  It replaces the former one-sided
   ! 0.75*R_eq floor, which was not a guard but a physical restriction: it
   ! made net heavy-isotope uptake (R_E < 0, real whenever h*R_a exceeds the
   ! equilibrium vapour ratio) impossible to express.
   real(r8), parameter :: craig_gordon_max_ratio_amplification = 10._r8
   ! Craig-Gordon kinetic fractionation exponent for evaporation from a
   ! turbulent liquid/open-water/soil surface.  The previous n=1 bare
   ! diffusivity ratio is retained only for ice/snow sublimation, where the
   ! stagnant diffusive limit is the safer default until a separate
   ! snow-surface resistance parameterization is added.
   real(r8), parameter :: craig_gordon_kinetic_exponent_liquid = 2._r8 / 3._r8
   real(r8), parameter :: craig_gordon_kinetic_exponent_ice = 1._r8
   ! Below this temperature the Jouzel-Merlivat (1984) supersaturation
   ! kinetic term is applied in full; between here and tfrz it is blended
   ! linearly into the equilibrium limit, following the IsoGSM reference
   ! implementation (gsml/CLD1/lrgscl.F:210-224).
   real(r8), parameter :: jm84_full_kinetic_temp = 253.15_r8
   ! Merlivat & Jouzel (1979) wind-dependent kinetic fractionation for a
   ! FREE WATER SURFACE (lakes, ponds, water bodies).  Two turbulence
   ! regimes; note the rough regime fractionates LESS, because the added
   ! turbulence thins the diffusive sublayer that the kinetic effect lives
   ! in.  Values as in IsoGSM gsml/ISOTOPE/frkin.F:12-23.  These are much
   ! smaller than the n=2/3 diffusivity exponent appropriate to soil pores
   ! (~6 vs ~19 permil for 18O), which is why open water needs its own law.
   real(r8), parameter :: mj79_smooth_k = 0.006_r8
   real(r8), parameter :: mj79_rough_slope = 0.000285_r8
   real(r8), parameter :: mj79_rough_offset = 0.00082_r8
   real(r8), parameter :: mj79_wind_threshold = 7._r8

   PUBLIC :: tracer_fractionation_active
   PUBLIC :: tracer_alpha_liq_vap, tracer_alpha_ice_vap, tracer_alpha_ice_liq
   PUBLIC :: tracer_jm84_effective_alpha, tracer_ice_deposition_alpha
   PUBLIC :: tracer_alpha_ice_vap_deposition
   PUBLIC :: tracer_rayleigh_freezing_loss
   PUBLIC :: tracer_diffusivity_ratio_air
   PUBLIC :: tracer_alpha_kinetic_leaf, tracer_alpha_kinetic_soil
   PUBLIC :: tracer_soil_kinetic_alpha_core
   PUBLIC :: tracer_soil_effective_diffusivity, tracer_soil_diffusive_transfer
   PUBLIC :: tracer_soil_vapor_equivalent_diffusivity
   PUBLIC :: tracer_snow_vapor_equivalent_diffusivity
   PUBLIC :: tracer_liquid_self_diffusivity
   PUBLIC :: tracer_equilibration_exchange
   PUBLIC :: tracer_alpha_kinetic_craig_gordon
   PUBLIC :: tracer_mj79_kinetic_alpha, tracer_alpha_kinetic_open_water
   PUBLIC :: tracer_craig_gordon_evap_ratio, tracer_craig_gordon_ratio_core
   PUBLIC :: tracer_equilibrium_deposition_ratio
   PUBLIC :: tracer_transpiration_nss_ratio
   PUBLIC :: tracer_saturation_vapor_pressure, tracer_surface_relhum

CONTAINS

   logical FUNCTION tracer_fractionation_active (itrc)
      integer, intent(in) :: itrc

      tracer_fractionation_active = .false.
      IF (.not. DEF_TRACER_USE_FRACTIONATION) RETURN
      IF (.not. tracer_is_isotope(itrc)) RETURN
      CALL ensure_isotope_physics_registered ()
      tracer_fractionation_active = isotope_fractionation_registered(itrc)
   END FUNCTION tracer_fractionation_active

   real(r8) FUNCTION tracer_alpha_liq_vap (itrc, temp_k)
      integer,  intent(in) :: itrc
      real(r8), intent(in) :: temp_k

      CALL ensure_isotope_physics_registered ()
      tracer_alpha_liq_vap = isotope_alpha_liq_vap(itrc, temp_k)
   END FUNCTION tracer_alpha_liq_vap

   real(r8) FUNCTION tracer_alpha_ice_vap (itrc, temp_k)
      integer,  intent(in) :: itrc
      real(r8), intent(in) :: temp_k

      CALL ensure_isotope_physics_registered ()
      tracer_alpha_ice_vap = isotope_alpha_ice_vap(itrc, temp_k)
   END FUNCTION tracer_alpha_ice_vap

   real(r8) FUNCTION tracer_alpha_ice_liq (itrc, temp_k)
      integer,  intent(in) :: itrc
      real(r8), intent(in) :: temp_k

      ! alpha = R_ice / R_liquid, derived from the existing equilibrium
      ! vapor references so phase-change and vapor processes stay consistent.
      tracer_alpha_ice_liq = tracer_alpha_ice_vap(itrc, temp_k) / &
         max(tracer_alpha_liq_vap(itrc, temp_k), trc_tiny)
   END FUNCTION tracer_alpha_ice_liq

   real(r8) FUNCTION tracer_rayleigh_freezing_loss (itrc, pool_trc, pool_water, &
      freeze_water, temp_k)
      integer,  intent(in) :: itrc
      real(r8), intent(in) :: pool_trc
      real(r8), intent(in) :: pool_water
      real(r8), intent(in) :: freeze_water
      real(r8), intent(in) :: temp_k
      real(r8) :: alpha_il
      real(r8) :: liquid_fraction, remaining_water
      real(r8) :: source_ratio, remaining_ratio, remaining_trc

      tracer_rayleigh_freezing_loss = 0._r8
      IF (pool_water <= trc_tiny .or. freeze_water <= trc_tiny) RETURN
      IF (pool_trc <= trc_tiny) RETURN

      IF (freeze_water >= pool_water * (1._r8 - 1.e-12_r8)) THEN
         tracer_rayleigh_freezing_loss = max(pool_trc, 0._r8)
         RETURN
      ENDIF

      source_ratio = max(pool_trc, 0._r8) / pool_water
      IF (.not. tracer_fractionation_active(itrc)) THEN
         tracer_rayleigh_freezing_loss = min(freeze_water * source_ratio, max(pool_trc, 0._r8))
         RETURN
      ENDIF

      alpha_il = tracer_alpha_ice_liq(itrc, temp_k)
      IF (alpha_il <= trc_tiny .or. alpha_il /= alpha_il) THEN
         tracer_rayleigh_freezing_loss = min(freeze_water * source_ratio, max(pool_trc, 0._r8))
         RETURN
      ENDIF

      ! Equilibrium freezing of a finite mixed liquid pool:
      ! R_liq = R0 * f**(alpha_ice_liq - 1), with f = remaining liquid fraction.
      ! The tracer transferred to ice is the mass removed from the liquid pool.
      remaining_water = pool_water - freeze_water
      liquid_fraction = max(remaining_water / pool_water, trc_tiny)
      remaining_ratio = source_ratio * liquid_fraction ** (alpha_il - 1._r8)
      remaining_trc = remaining_water * remaining_ratio
      tracer_rayleigh_freezing_loss = min(max(pool_trc - remaining_trc, 0._r8), &
         max(pool_trc, 0._r8))
   END FUNCTION tracer_rayleigh_freezing_loss

   real(r8) FUNCTION tracer_diffusivity_ratio_air (itrc)
      integer, intent(in) :: itrc

      CALL ensure_isotope_physics_registered ()
      tracer_diffusivity_ratio_air = isotope_diffusivity_ratio_air(itrc)
   END FUNCTION tracer_diffusivity_ratio_air

   !---------------------------------------------------------------
   ! Jouzel & Merlivat (1984) effective solid-vapour fractionation.
   !
   ! Ice growing into a supersaturated vapour cannot maintain isotopic
   ! equilibrium: the heavier isotopologue diffuses more slowly toward the
   ! crystal surface, so the effective alpha lies BELOW alpha_eq.
   !   S         = 1 - slope * T[C]                  (S > 1 below 0 C)
   !   alpha_kin = S / (alpha_eq * (D/D_i) * (S-1) + 1)
   !   alpha_eff = alpha_eq * alpha_kin
   ! Pure function of its arguments so it can be verified standalone.
   !---------------------------------------------------------------
   real(r8) FUNCTION tracer_jm84_effective_alpha (alpha_eq, diff_ratio, tc, slope)
      real(r8), intent(in) :: alpha_eq
      real(r8), intent(in) :: diff_ratio
      real(r8), intent(in) :: tc        ! temperature in Celsius
      real(r8), intent(in) :: slope
      real(r8) :: s, denom

      tracer_jm84_effective_alpha = alpha_eq
      IF (slope <= 0._r8 .or. tc >= 0._r8) RETURN

      s = 1._r8 - slope * tc
      denom = alpha_eq * diff_ratio * (s - 1._r8) + 1._r8
      IF (denom <= 0._r8) RETURN
      tracer_jm84_effective_alpha = alpha_eq * s / denom
   END FUNCTION tracer_jm84_effective_alpha

   !---------------------------------------------------------------
   ! Temperature-branched deposition alpha:
   !   T >= tfrz                 : equilibrium (S = 1, term inert)
   !   T <= jm84_full_kinetic_temp: full supersaturation kinetics
   !   in between                : linear blend of the equilibrium value at
   !                               tfrz and the EFFECTIVE value at the cold
   !                               end, so both joins are continuous.
   ! Pure function of its arguments (callers supply the three equilibrium
   ! alphas) so the branch continuity can be verified standalone.
   !---------------------------------------------------------------
   real(r8) FUNCTION tracer_ice_deposition_alpha (temp_k, slope, diff_ratio, &
      alpha_eq_at_t, alpha_eq_at_tfrz, alpha_eq_at_tcold)
      real(r8), intent(in) :: temp_k
      real(r8), intent(in) :: slope
      real(r8), intent(in) :: diff_ratio
      real(r8), intent(in) :: alpha_eq_at_t
      real(r8), intent(in) :: alpha_eq_at_tfrz
      real(r8), intent(in) :: alpha_eq_at_tcold
      real(r8) :: alpha_cold, span

      tracer_ice_deposition_alpha = alpha_eq_at_t
      IF (slope <= 0._r8) RETURN
      IF (temp_k >= tfrz) RETURN

      IF (temp_k <= jm84_full_kinetic_temp) THEN
         tracer_ice_deposition_alpha = tracer_jm84_effective_alpha(alpha_eq_at_t, &
            diff_ratio, temp_k - tfrz, slope)
         RETURN
      ENDIF

      alpha_cold = tracer_jm84_effective_alpha(alpha_eq_at_tcold, diff_ratio, &
         jm84_full_kinetic_temp - tfrz, slope)
      span = tfrz - jm84_full_kinetic_temp
      tracer_ice_deposition_alpha = (alpha_eq_at_tfrz * (temp_k - jm84_full_kinetic_temp) &
         + alpha_cold * (tfrz - temp_k)) / span
   END FUNCTION tracer_ice_deposition_alpha

   real(r8) FUNCTION tracer_alpha_ice_vap_deposition (itrc, temp_k)
      integer,  intent(in) :: itrc
      real(r8), intent(in) :: temp_k

      tracer_alpha_ice_vap_deposition = tracer_ice_deposition_alpha(temp_k, &
         DEF_TRACER_ICE_SUPERSAT_SLOPE, tracer_diffusivity_ratio_air(itrc), &
         tracer_alpha_ice_vap(itrc, temp_k), &
         tracer_alpha_ice_vap(itrc, tfrz), &
         tracer_alpha_ice_vap(itrc, jm84_full_kinetic_temp))
   END FUNCTION tracer_alpha_ice_vap_deposition

   real(r8) FUNCTION tracer_alpha_kinetic_craig_gordon (itrc, from_ice)
      integer, intent(in) :: itrc
      logical, intent(in) :: from_ice

      IF (from_ice) THEN
         tracer_alpha_kinetic_craig_gordon = &
            tracer_diffusivity_ratio_air(itrc) ** craig_gordon_kinetic_exponent_ice
      ELSE
         tracer_alpha_kinetic_craig_gordon = &
            tracer_diffusivity_ratio_air(itrc) ** craig_gordon_kinetic_exponent_liquid
      ENDIF
   END FUNCTION tracer_alpha_kinetic_craig_gordon

   !---------------------------------------------------------------
   ! Merlivat & Jouzel (1979) kinetic factor for a free water surface.
   !
   ! k is the fractional reduction of the heavy-isotope flux relative to
   ! the light one; in the Craig-Gordon convention (where R_E is DIVIDED by
   ! alpha_k) that is alpha_k = 1/(1-k).  `relative_factor` carries the
   ! published species scaling: 1.0 for 18O, 0.88 for HDO.
   !
   ! Pure function of its arguments so it can be verified standalone.
   !---------------------------------------------------------------
   real(r8) FUNCTION tracer_mj79_kinetic_alpha (wind, relative_factor)
      real(r8), intent(in) :: wind
      real(r8), intent(in) :: relative_factor
      real(r8) :: u, k

      u = max(wind, 0._r8)
      IF (u >= mj79_wind_threshold) THEN
         k = mj79_rough_slope * u + mj79_rough_offset
      ELSE
         k = mj79_smooth_k
      ENDIF
      k = k * max(relative_factor, 0._r8)
      ! Keep the factor finite for absurd wind forcing; k stays << 1 for any
      ! physical wind speed, so this never binds in practice.
      k = min(max(k, 0._r8), 0.5_r8)
      tracer_mj79_kinetic_alpha = 1._r8 / (1._r8 - k)
   END FUNCTION tracer_mj79_kinetic_alpha

   !---------------------------------------------------------------
   ! Kinetic factor for evaporation from an open water surface.
   !
   ! Lakes and water bodies are not soil: their evaporating interface is a
   ! free water surface with a wind-dependent diffusive sublayer, so the
   ! n=2/3 pore-diffusion exponent overstates the effect roughly threefold.
   ! Defaults to Merlivat & Jouzel (1979); 'EXPONENT' restores the previous
   ! behaviour for sensitivity experiments.
   !---------------------------------------------------------------
   real(r8) FUNCTION tracer_alpha_kinetic_open_water (itrc, wind)
      integer,  intent(in) :: itrc
      real(r8), intent(in) :: wind

      IF (trim(DEF_TRACER_OPEN_WATER_KINETIC) == 'EXPONENT') THEN
         tracer_alpha_kinetic_open_water = tracer_alpha_kinetic_craig_gordon(itrc, .false.)
         RETURN
      ENDIF

      CALL ensure_isotope_physics_registered ()
      tracer_alpha_kinetic_open_water = tracer_mj79_kinetic_alpha(wind, &
         isotope_mj79_relative_factor(itrc))
   END FUNCTION tracer_alpha_kinetic_open_water

   real(r8) FUNCTION tracer_alpha_kinetic_leaf (itrc, ra, rb, rc)
      integer,  intent(in) :: itrc
      real(r8), intent(in) :: ra, rb, rc
      real(r8) :: ra1, rb1, rc1, denom, diff_ratio

      ra1 = max(ra, 0._r8)
      rb1 = max(rb, 0._r8)
      rc1 = max(rc, 0._r8)
      denom = ra1 + rb1 + rc1
      tracer_alpha_kinetic_leaf = 1._r8
      IF (denom <= trc_tiny) RETURN

      diff_ratio = tracer_diffusivity_ratio_air(itrc)
      tracer_alpha_kinetic_leaf = (ra1 + rb1 * diff_ratio ** (2._r8 / 3._r8) + &
         rc1 * diff_ratio) / denom
   END FUNCTION tracer_alpha_kinetic_leaf

   !---------------------------------------------------------------
   ! Resistance-weighted kinetic factor for soil evaporation.
   !
   !   alpha_k = (ra * D**(2/3) + rs * D**1) / (ra + rs)
   !
   ! The two segments carry the exponent appropriate to their transport
   ! regime, exactly as the leaf form does: the aerodynamic path is
   ! turbulent (n = 2/3), while the soil-surface/pore path is stagnant
   ! molecular diffusion (n = 1).  Limits:
   !   rs -> 0        wet soil, turbulent limit  -> D**(2/3)  (~19 permil)
   !   rs -> infinity dry soil, diffusive limit  -> D         (~28.5 permil)
   ! A fixed n = 2/3 pins soil evaporation at the wet-soil value even as the
   ! evaporation front retreats into the pores.
   !
   ! Pure function of its arguments so it can be verified standalone.
   !---------------------------------------------------------------
   ! NOTE on the `ra` exponent, which differs from tracer_alpha_kinetic_leaf
   ! above (that one weights ra by 1, i.e. no fractionation).  This is
   ! deliberate, not an oversight:
   ! - For leaves, Farquhar & Cernusak define ra as the purely turbulent
   !   resistance ABOVE the canopy; the molecular sublayer is carried by the
   !   separate leaf boundary term rb, which does get n = 2/3.
   ! - For bare soil there is no such intermediate term: CoLM's aerodynamic
   !   resistance to the ground already absorbs the quasi-laminar sublayer.
   !   Weighting it by 1 would send the wet-soil limit to alpha_k = 1, i.e.
   !   zero kinetic fractionation from a saturated surface, contradicting the
   !   observed ~15-20 permil.  n = 2/3 keeps the wet limit at ~19 permil.
   real(r8) FUNCTION tracer_soil_kinetic_alpha_core (ra, rs, diff_ratio)
      real(r8), intent(in) :: ra, rs
      real(r8), intent(in) :: diff_ratio
      real(r8) :: ra1, rs1, denom

      ra1 = max(ra, 0._r8)
      rs1 = max(rs, 0._r8)
      denom = ra1 + rs1
      tracer_soil_kinetic_alpha_core = 1._r8
      IF (denom <= 0._r8) RETURN
      IF (diff_ratio <= 0._r8) RETURN

      tracer_soil_kinetic_alpha_core = &
         (ra1 * diff_ratio ** craig_gordon_kinetic_exponent_liquid &
        + rs1 * diff_ratio ** craig_gordon_kinetic_exponent_ice) / denom
   END FUNCTION tracer_soil_kinetic_alpha_core

   !---------------------------------------------------------------
   ! Effective liquid-phase isotope diffusivity in soil [m2/s].
   !
   !   D_eff = D_liquid * theta**(7/3) / porsl**2
   !
   ! Millington-Quirk: theta*tortuosity with tortuosity = theta**(4/3)/phi**2.
   ! The theta**(7/3) dependence is steep on purpose -- it is what stops a
   ! dry layer from exchanging, and what confines the diffusive smoothing to
   ! the wet part of the column.
   !
   ! Pure function of its arguments so it can be verified standalone.
   !---------------------------------------------------------------
   real(r8) FUNCTION tracer_soil_effective_diffusivity (water_mm, dz, porsl, diff_liquid)
      real(r8), intent(in) :: water_mm      ! liquid water in the layer [mm]
      real(r8), intent(in) :: dz            ! layer thickness [m]
      real(r8), intent(in) :: porsl         ! porosity [-]
      real(r8), intent(in) :: diff_liquid   ! free-water self diffusivity [m2/s]
      real(r8) :: theta

      tracer_soil_effective_diffusivity = 0._r8
      IF (water_mm <= 0._r8 .or. dz <= 0._r8) RETURN
      IF (porsl <= 0._r8 .or. diff_liquid <= 0._r8) RETURN

      ! mm of water over dz metres of soil -> volumetric water content.
      theta = water_mm / (1000._r8 * dz)
      theta = min(theta, porsl)
      IF (theta <= 0._r8) RETURN

      tracer_soil_effective_diffusivity = diff_liquid * theta ** (7._r8 / 3._r8) &
         / (porsl * porsl)
   END FUNCTION tracer_soil_effective_diffusivity

   !---------------------------------------------------------------
   ! Vapour-phase isotope diffusion in soil pores, expressed as an
   ! EQUIVALENT LIQUID diffusivity [m2/s] so it can simply be added to
   ! tracer_soil_effective_diffusivity and reuse the same flux form and
   ! overshoot limiter.
   !
   !   D_vap_equiv = D_vap_air * tau_gas / diff_ratio
   !                 * rho_v_sat / (alpha_liq_vap * rho_water)
   !
   ! Why it matters: liquid diffusion alone cannot produce the Barnes &
   ! Allison (1983) profile.  Once the surface dries, the evaporation front
   ! retreats into the soil and transport above it goes through the PORE AIR.
   ! Vapour diffusivity is ~1e4x the liquid value while pore vapour density is
   ! ~2e-5 of liquid water, so the two are comparable overall -- and their
   ! moisture dependences are OPPOSITE (tau_gas uses AIR-filled porosity), so
   ! vapour takes over exactly where the liquid film shuts down.  For
   ! porsl = 0.45 the vapour term is ~0.5% of liquid at theta = 0.40 and ~30x
   ! liquid at theta = 0.05.
   !
   ! Assumptions, both deliberate:
   ! - Pore air is treated as saturated (relative humidity 1).  Via Kelvin,
   !   h > 0.993 for matric potentials down to about -1 MPa, so this only
   !   overstates rho_v in very dry soil.
   ! - Pore vapour is in local isotopic equilibrium with its own layer's
   !   liquid, R_v = R_liq / alpha_liq_vap.  That is what lets the term
   !   collapse into a diffusivity instead of needing a prognostic vapour
   !   isotope field.  It drops the thermally driven part of vapour transport
   !   (gradients in rho_v itself), which is second order for the layer
   !   temperature contrasts CoLM resolves.
   !
   ! Pure function of its arguments so it can be verified standalone.
   !---------------------------------------------------------------
   real(r8) FUNCTION tracer_soil_vapor_equivalent_diffusivity (water_mm, ice_mm, dz, porsl, &
      temp_k, psrf, diff_ratio, alpha_liq_vap)
      real(r8), intent(in) :: water_mm        ! liquid water in the layer [mm]
      ! Ice occupies pore space exactly as liquid does, so it must come out of
      ! the air-filled porosity too.  Charging only the liquid content would
      ! overstate vapour transport in frozen soil by more than an order of
      ! magnitude (porsl=0.45, theta_liq=0.10, theta_ice=0.25 -> 18.6x).
      real(r8), intent(in) :: ice_mm          ! ice in the layer [mm w.e.]
      real(r8), intent(in) :: dz              ! layer thickness [m]
      real(r8), intent(in) :: porsl           ! porosity [-]
      real(r8), intent(in) :: temp_k          ! layer temperature [K]
      real(r8), intent(in) :: psrf            ! surface pressure [Pa]
      real(r8), intent(in) :: diff_ratio      ! D_light/D_heavy in air [-]
      real(r8), intent(in) :: alpha_liq_vap   ! equilibrium liquid/vapour [-]
      real(r8), parameter :: d_vap_air_ref = 2.12e-5_r8   ! at 273.15 K, 1 atm [m2/s]
      real(r8), parameter :: p_ref = 101325._r8           ! [Pa]
      real(r8), parameter :: rv_water = 461.5_r8          ! vapour gas constant
      real(r8), parameter :: rho_water = 1000._r8         ! [kg/m3]
      real(r8) :: theta, theta_ice, filled, air_porosity
      real(r8) :: tau_gas, d_vap_air, rho_v_sat, tk

      tracer_soil_vapor_equivalent_diffusivity = 0._r8
      IF (dz <= 0._r8 .or. porsl <= 0._r8) RETURN
      IF (psrf <= 0._r8 .or. diff_ratio <= 0._r8 .or. alpha_liq_vap <= 0._r8) RETURN

      theta     = max(water_mm, 0._r8) / (1000._r8 * dz)
      theta_ice = max(ice_mm,   0._r8) / (1000._r8 * dz)
      filled = min(theta + theta_ice, porsl)
      air_porosity = porsl - filled
      IF (air_porosity <= 0._r8) RETURN

      tau_gas = air_porosity ** (7._r8 / 3._r8) / (porsl * porsl)
      tk = max(temp_k, 150._r8)
      d_vap_air = d_vap_air_ref * (tk / 273.15_r8) ** 2 * (p_ref / psrf)
      rho_v_sat = tracer_saturation_vapor_pressure(tk, .false.) / (rv_water * tk)

      tracer_soil_vapor_equivalent_diffusivity = d_vap_air * tau_gas / diff_ratio &
         * rho_v_sat / (alpha_liq_vap * rho_water)
   END FUNCTION tracer_soil_vapor_equivalent_diffusivity

   !---------------------------------------------------------------
   ! Vapour-phase isotope diffusion within snow / firn, again expressed as an
   ! EQUIVALENT diffusivity [m2/s] so it plugs into the same flux form.
   !
   !   D_eq = D_vap_air * phi**(7/3) / diff_ratio
   !          * rho_v_sat(over ice) / (alpha_ice_vap * rho_water)
   !
   ! Isotopes hardly move through the ice lattice; a pack smooths its own
   ! vertical signal through the pore vapour (Whillans & Grootes 1985; Johnsen
   ! et al. 2000).  Two differences from the soil form:
   ! - pore air is saturated with respect to ICE, and the gradient acts on the
   !   ice carrier, so alpha_ice_vap replaces alpha_liq_vap;
   ! - porosity comes from snow density against the density of ice.  Ice and
   !   liquid are converted with their own densities (917 vs 1000 kg/m3),
   !   because a given mm of water equivalent occupies ~9% more volume as ice.
   ! There is no soil-porosity denominator: for snow the whole layer volume is
   ! potentially pore space, so Millington-Quirk reduces to phi**(7/3).
   !
   ! At rho = 300 kg/m3 and -10 C this gives ~1.6e-11 m2/s, inside the
   ! published firn range (~1e-11 to 5e-11).  Note the implied timescale is
   ! years for a seasonal layer thickness, so this matters for multi-year
   ! packs and glaciers far more than for a snowpack that melts out annually.
   !
   ! Pure function of its arguments so it can be verified standalone.
   !---------------------------------------------------------------
   real(r8) FUNCTION tracer_snow_vapor_equivalent_diffusivity (wliq_mm, wice_mm, dz, &
      temp_k, psrf, diff_ratio, alpha_ice_vap)
      real(r8), intent(in) :: wliq_mm         ! liquid water in the layer [mm]
      real(r8), intent(in) :: wice_mm         ! ice in the layer [mm w.e.]
      real(r8), intent(in) :: dz              ! layer thickness [m]
      real(r8), intent(in) :: temp_k
      real(r8), intent(in) :: psrf            ! surface pressure [Pa]
      real(r8), intent(in) :: diff_ratio      ! D_light/D_heavy in air [-]
      real(r8), intent(in) :: alpha_ice_vap   ! equilibrium ice/vapour [-]
      real(r8), parameter :: d_vap_air_ref = 2.12e-5_r8
      real(r8), parameter :: p_ref = 101325._r8
      real(r8), parameter :: rv_water = 461.5_r8
      real(r8), parameter :: rho_water = 1000._r8
      real(r8), parameter :: rho_ice = 917._r8
      real(r8) :: solid_fraction, liquid_fraction, phi
      real(r8) :: tau_gas, d_vap_air, rho_v_sat, tk

      tracer_snow_vapor_equivalent_diffusivity = 0._r8
      IF (dz <= 0._r8 .or. psrf <= 0._r8) RETURN
      IF (diff_ratio <= 0._r8 .or. alpha_ice_vap <= 0._r8) RETURN

      solid_fraction  = max(wice_mm, 0._r8) / (rho_ice   * dz)
      liquid_fraction = max(wliq_mm, 0._r8) / (rho_water * dz)
      phi = 1._r8 - min(solid_fraction + liquid_fraction, 1._r8)
      IF (phi <= 0._r8) RETURN

      tau_gas = phi ** (7._r8 / 3._r8)
      tk = max(temp_k, 150._r8)
      d_vap_air = d_vap_air_ref * (tk / 273.15_r8) ** 2 * (p_ref / psrf)
      ! Saturation over ICE: at -10 C it is ~10% below the water curve.
      rho_v_sat = tracer_saturation_vapor_pressure(tk, .true.) / (rv_water * tk)

      tracer_snow_vapor_equivalent_diffusivity = d_vap_air * tau_gas / diff_ratio &
         * rho_v_sat / (alpha_ice_vap * rho_water)
   END FUNCTION tracer_snow_vapor_equivalent_diffusivity

   !---------------------------------------------------------------
   ! Diffusive tracer transfer between two adjacent soil layers over one
   ! step.  Returns the tracer mass moved from UPPER to LOWER (signed, in the
   ! same units as the layer tracer storage); the caller applies it as an
   ! equal-and-opposite pair, so mass is conserved by construction.
   !
   !   transfer = D_harmonic * (R_upper - R_lower) / dz_mid * 1000 * dt
   !
   ! The 1000 converts the m/s flux density into mm/s, matching CoLM's
   ! mm-based water and tracer storage.  Layer diffusivities are combined as
   ! resistances in series (thickness-weighted harmonic mean).
   !
   ! The result is limited to the amount that would equalise the two ratios,
   ! so an explicit step can approach equilibrium but never overshoot it.
   ! Without that limiter a thin layer plus a long timestep violates the
   ! diffusive CFL condition and the scheme rings.
   !
   ! Pure function of its arguments so it can be verified standalone.
   !---------------------------------------------------------------
   real(r8) FUNCTION tracer_soil_diffusive_transfer (ratio_upper, ratio_lower, &
      water_upper, water_lower, d_eff_upper, d_eff_lower, dz_upper, dz_lower, deltim)
      real(r8), intent(in) :: ratio_upper, ratio_lower
      real(r8), intent(in) :: water_upper, water_lower
      real(r8), intent(in) :: d_eff_upper, d_eff_lower
      real(r8), intent(in) :: dz_upper, dz_lower
      real(r8), intent(in) :: deltim
      real(r8) :: d_harmonic, dz_mid, gradient, equalise

      tracer_soil_diffusive_transfer = 0._r8
      IF (water_upper <= 0._r8 .or. water_lower <= 0._r8) RETURN
      IF (d_eff_upper <= 0._r8 .or. d_eff_lower <= 0._r8) RETURN
      IF (dz_upper <= 0._r8 .or. dz_lower <= 0._r8 .or. deltim <= 0._r8) RETURN

      gradient = ratio_upper - ratio_lower
      IF (gradient == 0._r8) RETURN

      d_harmonic = (dz_upper + dz_lower) / &
         (dz_upper / d_eff_upper + dz_lower / d_eff_lower)
      dz_mid = 0.5_r8 * (dz_upper + dz_lower)

      tracer_soil_diffusive_transfer = d_harmonic * gradient / dz_mid &
         * 1000._r8 * deltim

      ! Amount that makes both ratios equal; never move more than that.
      equalise = gradient / (1._r8 / water_upper + 1._r8 / water_lower)
      IF (abs(tracer_soil_diffusive_transfer) > abs(equalise)) THEN
         tracer_soil_diffusive_transfer = equalise
      ENDIF
   END FUNCTION tracer_soil_diffusive_transfer

   !---------------------------------------------------------------
   ! Two-way equilibrium exchange between a wet surface and ambient vapour.
   !
   ! A wet leaf keeps trading molecules with the surrounding vapour even when
   ! the NET water flux is zero, relaxing its ratio toward the equilibrium
   ! value alpha_eq * R_vapor:
   !
   !   R_new = R_old + f * (alpha_eq * R_vapor - R_old)
   !   gain  = pool_water * (R_new - R_old)
   !
   ! `f` is the equilibration degree over one step (0 = no exchange, 1 = full
   ! equilibrium), the same device IsoGSM uses for falling raindrops
   ! (gsml/CLD1/lrgscl.F, eqf = 0.95, with the resolved drop-spectrum
   ! alternative in ISOTOPE/eqm_deg.F).
   !
   ! The return value is a SIGNED tracer mass crossing the atmosphere/surface
   ! boundary with NO accompanying water flux, so callers must book it into a
   ! dedicated accumulator -- it is a real boundary flux, not an internal
   ! redistribution, and the balance check has to see it.
   !
   ! Pure function of its arguments so it can be verified standalone.
   !---------------------------------------------------------------
   real(r8) FUNCTION tracer_equilibration_exchange (pool_trc, pool_water, &
      vapor_ratio, alpha_eq, equilibration_fraction)
      real(r8), intent(in) :: pool_trc
      real(r8), intent(in) :: pool_water
      real(r8), intent(in) :: vapor_ratio
      real(r8), intent(in) :: alpha_eq
      real(r8), intent(in) :: equilibration_fraction
      real(r8) :: f, ratio_old, ratio_equilibrium

      tracer_equilibration_exchange = 0._r8
      IF (pool_water <= 0._r8) RETURN
      IF (alpha_eq <= 0._r8 .or. vapor_ratio < 0._r8) RETURN

      f = min(max(equilibration_fraction, 0._r8), 1._r8)
      IF (f <= 0._r8) RETURN

      ratio_old = pool_trc / pool_water
      ratio_equilibrium = alpha_eq * vapor_ratio
      tracer_equilibration_exchange = pool_water * f * (ratio_equilibrium - ratio_old)

      ! Never drive the pool's inventory negative.
      IF (tracer_equilibration_exchange < 0._r8) THEN
         tracer_equilibration_exchange = &
            -min(-tracer_equilibration_exchange, max(pool_trc, 0._r8))
      ENDIF
   END FUNCTION tracer_equilibration_exchange

   real(r8) FUNCTION tracer_alpha_kinetic_soil (itrc, ra, rs)
      integer,  intent(in) :: itrc
      real(r8), intent(in) :: ra, rs

      tracer_alpha_kinetic_soil = tracer_soil_kinetic_alpha_core(ra, rs, &
         tracer_diffusivity_ratio_air(itrc))
   END FUNCTION tracer_alpha_kinetic_soil

   !---------------------------------------------------------------
   ! Craig-Gordon evaporate ratio, pure form.
   !
   !   R_E = (R_s/alpha_eq - h*R_a) / (alpha_k * (1-h))
   !
   ! The 1/(1-h) factor is NOT a singularity in the tracer FLUX: the host
   ! water flux carries the same (1-h) factor, so the product E*R_E stays
   ! finite as h -> 1.  IsoGSM gets that cancellation structurally, by
   ! evaluating the isotope flux from the same bulk gradient as the water
   ! flux (gsml/moninp.F:369-373).  Here the ratio is formed explicitly, so
   ! the cancellation has to survive the humidity cap rather than being
   ! truncated by it -- hence the cap now lives in the namelist and sits
   ! near saturation instead of at 0.95.
   !
   ! The result may legitimately be NEGATIVE: once h*R_a exceeds the
   ! equilibrium vapour ratio, the net isotope flux reverses sign while the
   ! net water flux is still evaporation (the surface takes up heavy
   ! isotopes from an ambient vapour heavier than its own equilibrium
   ! vapour).  Callers must treat the return value as a signed ratio, not
   ! as a loss magnitude.
   !
   ! Pure function of its arguments so it can be verified standalone.
   !---------------------------------------------------------------
   real(r8) FUNCTION tracer_craig_gordon_ratio_core (source_ratio, vapor_ratio, &
      alpha_eq, alpha_k, relhum, relhum_max)
      real(r8), intent(in) :: source_ratio
      real(r8), intent(in) :: vapor_ratio
      real(r8), intent(in) :: alpha_eq
      real(r8), intent(in) :: alpha_k
      real(r8), intent(in) :: relhum
      real(r8), intent(in) :: relhum_max
      real(r8) :: h, one_minus_h, equilibrium_ratio, bound

      tracer_craig_gordon_ratio_core = source_ratio
      IF (alpha_eq <= 0._r8 .or. alpha_k <= 0._r8) RETURN

      equilibrium_ratio = source_ratio / alpha_eq
      tracer_craig_gordon_ratio_core = equilibrium_ratio

      h = min(max(relhum, 0._r8), max(min(relhum_max, 0.999999_r8), 0._r8))
      one_minus_h = 1._r8 - h
      IF (one_minus_h <= 0._r8) RETURN

      tracer_craig_gordon_ratio_core = &
         (equilibrium_ratio - h * vapor_ratio) / (alpha_k * one_minus_h)

      bound = craig_gordon_max_ratio_amplification * abs(equilibrium_ratio)
      tracer_craig_gordon_ratio_core = &
         min(max(tracer_craig_gordon_ratio_core, -bound), bound)
   END FUNCTION tracer_craig_gordon_ratio_core

   real(r8) FUNCTION tracer_craig_gordon_evap_ratio (itrc, source_ratio, vapor_ratio, &
      temp_k, relhum, alpha_k, from_ice)
      integer,  intent(in) :: itrc
      real(r8), intent(in) :: source_ratio
      real(r8), intent(in) :: vapor_ratio
      real(r8), intent(in) :: temp_k
      real(r8), intent(in) :: relhum
      real(r8), intent(in) :: alpha_k
      logical,  intent(in) :: from_ice

      real(r8) :: alpha_eq

      tracer_craig_gordon_evap_ratio = source_ratio
      IF (.not. tracer_fractionation_active(itrc)) RETURN

      IF (from_ice) THEN
         alpha_eq = tracer_alpha_ice_vap(itrc, temp_k)
      ELSE
         alpha_eq = tracer_alpha_liq_vap(itrc, temp_k)
      ENDIF
      IF (alpha_eq <= trc_tiny .or. alpha_eq /= alpha_eq) RETURN

      tracer_craig_gordon_evap_ratio = tracer_craig_gordon_ratio_core(source_ratio, &
         vapor_ratio, alpha_eq, max(alpha_k, trc_tiny), relhum, DEF_TRACER_CG_RELHUM_MAX)
      IF (tracer_craig_gordon_evap_ratio /= tracer_craig_gordon_evap_ratio) THEN
         tracer_craig_gordon_evap_ratio = source_ratio
      ENDIF
   END FUNCTION tracer_craig_gordon_evap_ratio

   real(r8) FUNCTION tracer_equilibrium_deposition_ratio (itrc, vapor_ratio, temp_k, from_ice)
      integer,  intent(in) :: itrc
      real(r8), intent(in) :: vapor_ratio
      real(r8), intent(in) :: temp_k
      logical,  intent(in) :: from_ice

      tracer_equilibrium_deposition_ratio = vapor_ratio
      IF (.not. tracer_fractionation_active(itrc)) RETURN

      IF (from_ice) THEN
         ! Vapour -> ice is not a pure equilibrium transfer below 0 C: the
         ! crystal grows into supersaturated air, so use the Jouzel-Merlivat
         ! effective alpha (reduces to alpha_eq at/above freezing and when
         ! DEF_TRACER_ICE_SUPERSAT_SLOPE = 0).
         tracer_equilibrium_deposition_ratio = &
            tracer_alpha_ice_vap_deposition(itrc, temp_k) * vapor_ratio
      ELSE
         tracer_equilibrium_deposition_ratio = tracer_alpha_liq_vap(itrc, temp_k) * vapor_ratio
      ENDIF
   END FUNCTION tracer_equilibrium_deposition_ratio

   SUBROUTINE tracer_transpiration_nss_ratio (itrc, source_ratio, vapor_ratio, &
      temp_k, relhum, psrf, transp_water, deltim, leaf_area, aerodynamic_resistance, &
      stomatal_resistance, &
      prev_delta_e, prev_delta_b, prev_peclet, prev_leaf_moles, &
      trans_ratio, new_delta_e, new_delta_b, new_peclet, new_leaf_moles)
      integer,  intent(in) :: itrc
      real(r8), intent(in) :: source_ratio
      real(r8), intent(in) :: vapor_ratio
      real(r8), intent(in) :: temp_k
      real(r8), intent(in) :: relhum
      real(r8), intent(in) :: psrf
      real(r8), intent(in) :: transp_water
      real(r8), intent(in) :: deltim
      real(r8), intent(in) :: leaf_area
      real(r8), intent(in) :: aerodynamic_resistance
      real(r8), intent(in) :: stomatal_resistance
      real(r8), intent(in) :: prev_delta_e
      real(r8), intent(in) :: prev_delta_b
      real(r8), intent(in) :: prev_peclet
      real(r8), intent(in) :: prev_leaf_moles
      real(r8), intent(out) :: trans_ratio
      real(r8), intent(out) :: new_delta_e
      real(r8), intent(out) :: new_delta_b
      real(r8), intent(out) :: new_peclet
      real(r8), intent(out) :: new_leaf_moles

      real(r8) :: h, one_minus_h, alpha_eq, alpha_k, eps_eq, eps_k
      real(r8) :: delta_x, delta_v, delta_es, delta_t
      real(r8) :: leaf_moles, transp_moles, transp_moles_leaf_s
      real(r8) :: prev_e, prev_p, prev_w, liquid_diff, peclet_number
      real(r8) :: leaf_moles_ratio
      real(r8) :: relax_b, denom, tk
      real(r8) :: gross_moles, conductance_gross_moles
      real(r8) :: vapor_molar_density_sat, total_resistance
	      real(r8) :: prev_leaf_water, new_leaf_water, prev_bulk_ratio, new_bulk_ratio
	      real(r8) :: storage_tracer_change, storage_tracer_change_used
	      real(r8) :: storage_scale, storage_bound, target_leaf_storage
	      real(r8), parameter :: max_storage_jump_for_fallback = 10._r8
	      real(r8), parameter :: max_storage_tendency_fraction = 0.95_r8

      trans_ratio = source_ratio
      new_delta_e = tracer_ratio_to_delta(itrc, source_ratio)
      new_delta_b = new_delta_e
      new_peclet = 1._r8
      new_leaf_moles = 0._r8

      IF (.not. tracer_fractionation_active(itrc)) RETURN
      IF (source_ratio <= trc_tiny .or. vapor_ratio <= trc_tiny) RETURN
      IF (transp_water <= trc_tiny .or. deltim <= trc_tiny) RETURN

      leaf_moles = max(leaf_area, 0._r8) * max(DEF_TRACER_NSS_LEAF_WATER_PER_LAI, 0._r8) &
         * water_moles_per_mm
      new_leaf_moles = leaf_moles
      IF (leaf_moles <= trc_tiny) RETURN

      tk = max(temp_k, 150._r8)
      ! Share the Craig-Gordon humidity cap.  delta_es below is LINEAR in h,
      ! so this cap is not a singularity guard: it truncates real depletion
      ! exactly as the surface cap did, and a different value here would make
      ! the leaf and surface closures disagree about the same air.
      h = min(max(relhum, 0._r8), max(min(DEF_TRACER_CG_RELHUM_MAX, 0.999999_r8), 0._r8))
      one_minus_h = max(1._r8 - h, 1.e-6_r8)
      alpha_eq = tracer_alpha_liq_vap(itrc, tk)
      eps_eq = (alpha_eq - 1._r8) * 1000._r8
      alpha_k = tracer_alpha_kinetic_leaf(itrc, aerodynamic_resistance, &
         DEF_TRACER_NSS_LEAF_RB / max(leaf_area, trc_tiny), stomatal_resistance)
      eps_k = (alpha_k - 1._r8) * 1000._r8

      delta_x = tracer_ratio_to_delta(itrc, source_ratio)
      delta_v = tracer_ratio_to_delta(itrc, vapor_ratio)
      delta_es = delta_x + eps_k + eps_eq + h * (delta_v - eps_k - delta_x)

      transp_moles = transp_water * water_moles_per_mm
      ! Péclet transport is defined per unit leaf area, whereas
      ! transp_water is supplied per unit ground area.
      transp_moles_leaf_s = transp_moles / (deltim * max(leaf_area, trc_tiny))
      liquid_diff = tracer_leaf_liquid_diffusivity(itrc, tk)
      IF (liquid_diff > trc_tiny .and. DEF_TRACER_NSS_LEAF_PATH_LENGTH > 0._r8) THEN
         peclet_number = transp_moles_leaf_s * DEF_TRACER_NSS_LEAF_PATH_LENGTH / &
            (liquid_water_molar_density * liquid_diff)
         IF (peclet_number > 1.e-8_r8) THEN
            new_peclet = (1._r8 - exp(-peclet_number)) / peclet_number
         ELSE
            new_peclet = 1._r8 - 0.5_r8 * peclet_number
         ENDIF
      ENDIF
      new_peclet = min(max(new_peclet, 0._r8), 1._r8)

	      IF (prev_leaf_moles > trc_tiny) THEN
	         leaf_moles_ratio = prev_leaf_moles / max(leaf_moles, trc_tiny)
	         IF (leaf_moles_ratio > 0.25_r8 .and. leaf_moles_ratio < 4._r8) THEN
	            prev_w = prev_leaf_moles
	         ELSE
	            ! Ignore a storage-size jump from changed NSS parameters or legacy restart files.
	            prev_w = leaf_moles
	         ENDIF
	         prev_e = prev_delta_e
	         prev_p = min(max(prev_peclet, 0._r8), 1._r8)
      ELSE
         prev_w = leaf_moles
         prev_e = delta_x
         prev_p = new_peclet
      ENDIF

	      ! Farquhar-Cernusak NSS form: the turnover time is controlled by
	      ! the one-way gross flux through stomata/boundary (g_t * w_i), not
	      ! by net transpiration when canopy coupling or water stress makes
	      ! E/(1-h) inconsistent with the same humidity used here.
	      gross_moles = transp_moles / one_minus_h
	      conductance_gross_moles = 0._r8
	      ! CoLM supplies stomatal_resistance on a canopy/ground-area basis.
	      ! DEF_TRACER_NSS_LEAF_RB is a leaf boundary resistance, so convert
	      ! only that term to the same ground-area basis before adding them.
	      total_resistance = max(aerodynamic_resistance, 0._r8) + &
	         max(stomatal_resistance, 0._r8) + &
	         max(DEF_TRACER_NSS_LEAF_RB, 0._r8) / max(leaf_area, trc_tiny)
	      IF (total_resistance > trc_tiny .and. psrf > trc_tiny) THEN
	         vapor_molar_density_sat = tracer_saturation_vapor_pressure(tk, .false.) / &
	            (universal_gas_constant * tk)
	         conductance_gross_moles = deltim * vapor_molar_density_sat / total_resistance
	      ENDIF
	      gross_moles = max(gross_moles, conductance_gross_moles)
	      relax_b = alpha_k * alpha_eq / max(gross_moles, trc_tiny)
      denom = 1._r8 + relax_b * leaf_moles * new_peclet
      IF (denom > trc_tiny) THEN
         new_delta_e = (delta_es + relax_b * (leaf_moles * new_peclet * delta_x + &
            prev_w * prev_p * (prev_e - delta_x))) / denom
      ELSE
         new_delta_e = delta_es
      ENDIF
      new_delta_b = delta_x + new_peclet * (new_delta_e - delta_x)

      ! Diagnose the transpiration flux from the leaf isotope storage
      ! budget, not directly from the Craig-Gordon end-member. Over one
      ! step:
      !   E * R_T = E * R_x - d(W_leaf * R_leaf_bulk)
      ! This keeps daily transpiration close to source water whenever
      ! leaf water storage is approximately cyclic; any departure is then
      ! the explicit net storage tendency rather than a hidden flux bias.
      IF (prev_leaf_moles > trc_tiny) THEN
         prev_leaf_water = prev_w / water_moles_per_mm
         prev_bulk_ratio = tracer_delta_to_ratio(itrc, prev_delta_b)
      ELSE
         prev_leaf_water = leaf_moles / water_moles_per_mm
         prev_bulk_ratio = source_ratio
      ENDIF
      new_leaf_water = leaf_moles / water_moles_per_mm
      new_bulk_ratio = tracer_delta_to_ratio(itrc, new_delta_b)
	      storage_tracer_change = new_leaf_water * new_bulk_ratio - prev_leaf_water * prev_bulk_ratio
	      storage_tracer_change_used = storage_tracer_change
	      storage_scale = max(transp_water * source_ratio, trc_tiny)
	      storage_bound = max_storage_tendency_fraction * storage_scale
	      IF (abs(storage_tracer_change) > max_storage_jump_for_fallback * storage_scale) THEN
	         ! A tiny transpiration step cannot physically determine a large
	         ! leaf-storage isotope tendency (usually LAI/restart/NSS memory
	         ! mismatch). Freeze the budget tendency for this step and adjust
	         ! the diagnostic bulk leaf delta to the same storage state.
	         storage_tracer_change_used = 0._r8
	      ELSE
	         storage_tracer_change_used = min(max(storage_tracer_change_used, -storage_bound), &
	            storage_bound)
	      ENDIF
	      IF (abs(storage_tracer_change_used - storage_tracer_change) > trc_tiny) THEN
	         target_leaf_storage = prev_leaf_water * prev_bulk_ratio + storage_tracer_change_used
	         IF (new_leaf_water > trc_tiny .and. target_leaf_storage > trc_tiny) THEN
	            new_bulk_ratio = target_leaf_storage / new_leaf_water
	            new_delta_b = tracer_ratio_to_delta(itrc, new_bulk_ratio)
	            IF (new_peclet > 1.e-6_r8) THEN
	               new_delta_e = delta_x + (new_delta_b - delta_x) / new_peclet
	            ELSE
	               new_delta_e = new_delta_b
	            ENDIF
	         ELSE
	            storage_tracer_change_used = 0._r8
	            new_delta_e = delta_x
	            new_delta_b = delta_x
	         ENDIF
	      ENDIF
	      trans_ratio = (transp_water * source_ratio - storage_tracer_change_used) / transp_water
	      IF (trans_ratio /= trans_ratio .or. trans_ratio <= 0._r8) THEN
	         trans_ratio = source_ratio
	         new_delta_e = prev_delta_e
	         new_delta_b = prev_delta_b
	      ENDIF
      IF (new_delta_b /= new_delta_b) new_delta_b = prev_delta_b
   END SUBROUTINE tracer_transpiration_nss_ratio

   real(r8) FUNCTION tracer_surface_relhum (qair, psrf, temp_k, over_ice)
      real(r8), intent(in) :: qair
      real(r8), intent(in) :: psrf
      real(r8), intent(in) :: temp_k
      logical,  intent(in) :: over_ice
      real(r8) :: eair, esat, qsafe, psafe

      qsafe = max(qair, 0._r8)
      psafe = max(psrf, 1._r8)
      eair = qsafe * psafe / (0.622_r8 + 0.378_r8 * qsafe)
      esat = tracer_saturation_vapor_pressure(temp_k, over_ice)
      IF (esat > trc_tiny) THEN
         tracer_surface_relhum = min(max(eair / esat, 0._r8), 0.999999_r8)
      ELSE
         tracer_surface_relhum = 0._r8
      ENDIF
   END FUNCTION tracer_surface_relhum

   real(r8) FUNCTION tracer_saturation_vapor_pressure (temp_k, over_ice)
      real(r8), intent(in) :: temp_k
      logical,  intent(in) :: over_ice
      real(r8) :: tc, tk

      tk = max(temp_k, 150._r8)
      tc = tk - tfrz
      IF (over_ice) THEN
         tracer_saturation_vapor_pressure = 611.2_r8 * exp(22.46_r8 * tc / (272.62_r8 + tc))
      ELSE
         tracer_saturation_vapor_pressure = 611.2_r8 * exp(17.67_r8 * tc / (243.5_r8 + tc))
      ENDIF
   END FUNCTION tracer_saturation_vapor_pressure

   ! Liquid self-diffusivity of the isotopologue in free water [m2/s].
   ! Same physical property the leaf-water Peclet term uses, so it shares the
   ! registered per-species function; named separately because soil diffusion
   ! is not a leaf process.  Returns 0 for tracers with no registered
   ! isotope physics, which makes callers no-ops for them.
   real(r8) FUNCTION tracer_liquid_self_diffusivity (itrc, temp_k)
      integer,  intent(in) :: itrc
      real(r8), intent(in) :: temp_k

      tracer_liquid_self_diffusivity = tracer_leaf_liquid_diffusivity(itrc, temp_k)
   END FUNCTION tracer_liquid_self_diffusivity

   real(r8) FUNCTION tracer_leaf_liquid_diffusivity (itrc, temp_k)
      integer,  intent(in) :: itrc
      real(r8), intent(in) :: temp_k

      CALL ensure_isotope_physics_registered ()
      tracer_leaf_liquid_diffusivity = isotope_leaf_liquid_diffusivity(itrc, temp_k)
   END FUNCTION tracer_leaf_liquid_diffusivity

   real(r8) FUNCTION tracer_ratio_to_delta (itrc, ratio)
      integer,  intent(in) :: itrc
      real(r8), intent(in) :: ratio
      real(r8) :: ref_ratio

      tracer_ratio_to_delta = 0._r8
      ref_ratio = 0._r8
      IF (itrc >= 1 .and. itrc <= ntracers .and. allocated(tracers)) ref_ratio = tracers(itrc)%ref_ratio
      IF (ref_ratio > trc_tiny) tracer_ratio_to_delta = (ratio / ref_ratio - 1._r8) * 1000._r8
   END FUNCTION tracer_ratio_to_delta

   real(r8) FUNCTION tracer_delta_to_ratio (itrc, delta)
      integer,  intent(in) :: itrc
      real(r8), intent(in) :: delta
      real(r8) :: ref_ratio

      tracer_delta_to_ratio = 0._r8
      ref_ratio = 0._r8
      IF (itrc >= 1 .and. itrc <= ntracers .and. allocated(tracers)) ref_ratio = tracers(itrc)%ref_ratio
      IF (ref_ratio > trc_tiny) tracer_delta_to_ratio = ref_ratio * (1._r8 + delta / 1000._r8)
   END FUNCTION tracer_delta_to_ratio


END MODULE MOD_Tracer_Frac
#endif
