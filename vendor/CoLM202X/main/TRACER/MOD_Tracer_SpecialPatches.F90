#include <define.h>

#ifdef TRACER
MODULE MOD_Tracer_SpecialPatches

   USE MOD_Precision
   USE MOD_Tracer_Defs, only: ntracers, tracers, trc_tiny, trc_water_min_for_ratio, &
      trc_delta_sanity_max, tracer_init_water_ratio, &
      tracer_can_use_fixed_signature, tracer_uses_land_water_transport, tracer_is_nonvolatile_solute, &
      tracer_has_dissolved_limit, tracer_equilibrate_dissolved
   USE MOD_Namelist, only: DEF_TRACER_SUBL_SKIN_MM
   USE MOD_Tracer_Forcing, only: tracer_forcing_precip_value, tracer_forcing_vapor_value
   USE MOD_Tracer_EvapLimit, only: tracer_atmospheric_tracer_loss
   USE MOD_Tracer_Frac, only: tracer_fractionation_active, tracer_surface_relhum, &
      tracer_alpha_kinetic_craig_gordon, tracer_craig_gordon_evap_ratio, &
      tracer_alpha_kinetic_open_water, &
      tracer_equilibrium_deposition_ratio
   USE MOD_Tracer_Conservation, only: tracer_save_storage, tracer_balance_check, &
      tracer_apply_reactive_processes
   USE MOD_Tracer_Hist, only: tracer_hist_accumulate
   USE MOD_Tracer_Vars, only: trc_wliq_soisno, trc_wice_soisno, trc_scv, trc_wdsrf, &
      trc_ldew_rain, trc_ldew_snow, trc_rnof_step, a_trc_precip, tracer_book_evap_loss, &
      TRC_EVAP_KIND_SOILEVAP, TRC_EVAP_KIND_SUBL, &
      a_trc_rsur, a_trc_rnof, trc_wetwat, trc_waterstorage, trc_storage_beg, &
      trc_surface_residue, trc_subsurface_residue, trc_runtime_forced, &
      trc_solid_soisno, trc_canopy_solid, trc_surface_solid, &
      trc_subsurface_solid, trc_waterstorage_solid, sync_tracer_patch_ratio

   IMPLICIT NONE
   PRIVATE

   PUBLIC :: tracer_glacier_patch
   PUBLIC :: tracer_waterbody_patch

CONTAINS

   SUBROUTINE tracer_glacier_patch(ipatch, maxsnl, nl_soil, deltim, &
      prc_rain, prl_rain, prc_snow, prl_snow, rnof, qseva, qsubl, qsdew, qfros, &
      endwb, totwb, glacier_overflow_mass, errorw, wdsrf, scv, t_grnd, forc_q, &
      forc_psrf, wliq_soisno, wice_soisno)

      IMPLICIT NONE
      integer,  intent(in) :: ipatch, maxsnl, nl_soil
      real(r8), intent(in) :: deltim
      real(r8), intent(in) :: prc_rain, prl_rain, prc_snow, prl_snow
      real(r8), intent(in) :: rnof, qseva, qsubl, qsdew, qfros
      real(r8), intent(in) :: endwb, totwb, glacier_overflow_mass, errorw
      real(r8), intent(in) :: wdsrf, scv, t_grnd, forc_q, forc_psrf
      real(r8), intent(in) :: wliq_soisno(maxsnl+1:nl_soil)
      real(r8), intent(in) :: wice_soisno(maxsnl+1:nl_soil)

      integer  :: itrc, j_trc, snl_trc
      real(r8) :: R_init, R_precip, R_vapor, R_final
      real(r8) :: R_dew, R_frost, R_runoff
      real(r8) :: precip_mass, rnof_mass
      real(r8) :: evap_mass, dep_mass
      real(r8) :: evap_liq_mass, evap_ice_mass, dep_liq_mass, dep_ice_mass
      real(r8) :: water_dS, water_end, water_beg, water_input
      real(r8) :: water_before_output, water_after_evap, water_after_liq
      real(r8) :: trc_input, trc_evap, trc_evap_liq, trc_evap_ice, trc_after_liq
      real(r8) :: trc_rnof, trc_available, trc_final
      real(r8) :: trc_held_storage, surface_residue_beg
      real(r8) :: surface_liquid_end, surface_carrier, surface_residue_export
      real(r8) :: xerr_tracer
      logical  :: mixed_signature, fixed_signature, frac_active, nonvolatile_solute

      DO itrc = 1, ntracers
         IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
         ! Glacier physics exposes a single bulk snow carrier (scv).  Fold any
         ! layered state left by the preceding patch/step into that carrier
         ! before the balance snapshot instead of clearing it outside the
         ! accounting window.
         trc_scv(itrc, ipatch) = trc_scv(itrc, ipatch) + &
            sum(trc_wliq_soisno(itrc, maxsnl+1:0, ipatch)) + &
            sum(trc_wice_soisno(itrc, maxsnl+1:0, ipatch))
         trc_wliq_soisno(itrc, maxsnl+1:0, ipatch) = 0._r8
         trc_wice_soisno(itrc, maxsnl+1:0, ipatch) = 0._r8
         ! These carrier pools do not exist on glacier patches.  Quarantine
         ! conservative mass left by a patch-class transition before clearing
         ! the incompatible dissolved states; it will re-enter only through a
         ! resolved glacier surface carrier below.
         IF (tracer_is_nonvolatile_solute(itrc)) THEN
            CALL move_special_subsurface_residue_to_surface(itrc, ipatch)
            IF (tracer_has_dissolved_limit(itrc)) THEN
               trc_surface_solid(itrc, ipatch) = trc_surface_solid(itrc, ipatch) + &
                  trc_surface_residue(itrc, ipatch) + trc_ldew_rain(itrc, ipatch) + &
                  trc_ldew_snow(itrc, ipatch) + trc_wetwat(itrc, ipatch) + &
                  trc_canopy_solid(itrc, ipatch)
               trc_surface_residue(itrc, ipatch) = 0._r8
               trc_surface_solid(itrc, ipatch) = trc_surface_solid(itrc, ipatch) + &
                  sum(trc_solid_soisno(itrc, maxsnl+1:0, ipatch))
               trc_solid_soisno(itrc, maxsnl+1:0, ipatch) = 0._r8
               IF (allocated(trc_waterstorage)) THEN
                  trc_surface_solid(itrc, ipatch) = trc_surface_solid(itrc, ipatch) + &
                     trc_waterstorage(itrc, ipatch) + trc_waterstorage_solid(itrc, ipatch)
               ENDIF
            ELSE
               trc_surface_residue(itrc, ipatch) = trc_surface_residue(itrc, ipatch) + &
                  trc_ldew_rain(itrc, ipatch) + trc_ldew_snow(itrc, ipatch) + &
                  trc_wetwat(itrc, ipatch)
               IF (allocated(trc_waterstorage)) THEN
                  trc_surface_residue(itrc, ipatch) = trc_surface_residue(itrc, ipatch) + &
                     trc_waterstorage(itrc, ipatch)
               ENDIF
            ENDIF
         ENDIF
         trc_ldew_rain(itrc, ipatch) = 0._r8
         trc_ldew_snow(itrc, ipatch) = 0._r8
         trc_wetwat   (itrc, ipatch) = 0._r8
         trc_canopy_solid(itrc, ipatch) = 0._r8
         IF (allocated(trc_waterstorage)) trc_waterstorage(itrc, ipatch) = 0._r8
         trc_waterstorage_solid(itrc, ipatch) = 0._r8
      ENDDO

      snl_trc = 0
      CALL tracer_save_storage(ipatch, snl_trc, nl_soil)
      precip_mass = (prc_rain + prl_rain + prc_snow + prl_snow) * deltim
      rnof_mass   = max(rnof, 0._r8) * deltim
      evap_liq_mass = max(qseva, 0._r8) * deltim
      evap_ice_mass = max(qsubl, 0._r8) * deltim
      dep_liq_mass  = max(qsdew, 0._r8) * deltim
      dep_ice_mass  = max(qfros, 0._r8) * deltim
      evap_mass = evap_liq_mass + evap_ice_mass
      dep_mass  = dep_liq_mass  + dep_ice_mass
      water_input = precip_mass + dep_mass
      water_dS = endwb - totwb - glacier_overflow_mass
      water_end = max(wdsrf, 0._r8) + max(scv, 0._r8)
      DO j_trc = 1, nl_soil
         water_end = water_end + max(wliq_soisno(j_trc), 0._r8) + max(wice_soisno(j_trc), 0._r8)
      ENDDO
      water_beg = water_end - water_dS

      DO itrc = 1, ntracers
         IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
         trc_rnof_step(itrc, ipatch) = 0._r8
         R_init = tracer_init_water_ratio(itrc)
         frac_active = tracer_fractionation_active(itrc)
         nonvolatile_solute = tracer_is_nonvolatile_solute(itrc)
         surface_residue_beg = trc_surface_residue(itrc, ipatch)
         trc_held_storage = trc_subsurface_residue(itrc, ipatch) + surface_residue_beg + &
            solid_inventory(itrc, ipatch, 1, nl_soil)
         fixed_signature = tracer_can_use_fixed_signature(itrc) .and. .not. frac_active
         IF (allocated(trc_runtime_forced)) THEN
            fixed_signature = fixed_signature .and. .not. trc_runtime_forced(itrc)
         ENDIF
         mixed_signature = .not. fixed_signature

         IF (mixed_signature) THEN
            R_precip = tracer_forcing_precip_value(itrc, ipatch)
            R_vapor  = tracer_forcing_vapor_value (itrc, ipatch)
            IF (nonvolatile_solute) THEN
               R_dew = 0._r8
               R_frost = 0._r8
            ELSE
               R_dew = R_vapor
               R_frost = R_vapor
               IF (frac_active) THEN
                  R_dew = tracer_equilibrium_deposition_ratio(itrc, R_vapor, t_grnd, .false.)
                  R_frost = tracer_equilibrium_deposition_ratio(itrc, R_vapor, t_grnd, .true.)
               ENDIF
            ENDIF
            trc_input = precip_mass * R_precip + dep_liq_mass * R_dew + dep_ice_mass * R_frost
            trc_available = max(trc_storage_beg(itrc, ipatch) - &
               trc_held_storage + trc_input, 0._r8)
            water_before_output = water_beg + water_input
            IF (nonvolatile_solute) THEN
               trc_evap_liq = 0._r8
               trc_evap_ice = 0._r8
            ELSE
               trc_evap_liq = tracer_atmospheric_tracer_loss(trc_available, water_before_output, &
                  evap_liq_mass, t_grnd, .false., glacier_evap_ratio_for, trc_tiny, &
                  tracer_ratio_cap(itrc), nonvolatile_solute)
               trc_after_liq = trc_available - trc_evap_liq
               water_after_liq = water_before_output - evap_liq_mass
               trc_evap_ice = tracer_atmospheric_tracer_loss(trc_after_liq, water_after_liq, &
                  evap_ice_mass, t_grnd, .true., glacier_evap_ratio_for, trc_tiny, &
                  tracer_ratio_cap(itrc), nonvolatile_solute, skin_mass = DEF_TRACER_SUBL_SKIN_MM)
            ENDIF
            trc_evap = trc_evap_liq + trc_evap_ice
            water_after_evap = water_before_output - evap_mass
            trc_final = max(trc_available - trc_evap, 0._r8)
            CALL tracer_equilibrate_dissolved(itrc, water_after_evap, trc_final, &
               trc_surface_solid(itrc, ipatch))
            IF (water_after_evap > trc_water_min_for_ratio) THEN
               R_runoff = trc_final / water_after_evap
            ELSE
               R_runoff = 0._r8
            ENDIF
            trc_rnof = min(rnof_mass * R_runoff, trc_final)
            surface_liquid_end = max(wdsrf, 0._r8) + max(wliq_soisno(1), 0._r8)
            surface_carrier = surface_liquid_end + rnof_mass
            surface_residue_export = 0._r8
            IF (nonvolatile_solute .and. .not. tracer_has_dissolved_limit(itrc) .and. &
                surface_residue_beg > trc_tiny .and. &
                rnof_mass > trc_tiny .and. &
                surface_carrier > trc_water_min_for_ratio) THEN
               surface_residue_export = min(surface_residue_beg * &
                  rnof_mass / surface_carrier, surface_residue_beg)
               surface_residue_beg = surface_residue_beg - surface_residue_export
               trc_rnof = trc_rnof + surface_residue_export
            ENDIF
            ! surface_residue_export comes from held mass, not trc_available.
            trc_final = max(trc_final - trc_rnof + surface_residue_export, 0._r8)
            CALL tracer_equilibrate_dissolved(itrc, water_end, trc_final, &
               trc_surface_solid(itrc, ipatch))
            IF (water_end > trc_water_min_for_ratio) THEN
               R_final = trc_final / water_end
               IF (nonvolatile_solute) trc_surface_residue(itrc, ipatch) = surface_residue_beg
            ELSE
               R_final = 0._r8
               IF (nonvolatile_solute) THEN
                  trc_surface_residue(itrc, ipatch) = surface_residue_beg + trc_final
               ENDIF
            ENDIF
         ELSE
            trc_input = water_input * R_init
            trc_evap_liq = evap_liq_mass * R_init
            trc_evap_ice = evap_ice_mass * R_init
            trc_evap = trc_evap_liq + trc_evap_ice
            trc_rnof  = rnof_mass * R_init
            R_final   = R_init
         ENDIF

         IF (trc_input > 0._r8) a_trc_precip(itrc, ipatch) = a_trc_precip(itrc, ipatch) + trc_input
         CALL tracer_book_evap_loss(itrc, ipatch, trc_evap_liq, evap_liq_mass, TRC_EVAP_KIND_SOILEVAP)
         CALL tracer_book_evap_loss(itrc, ipatch, trc_evap_ice, evap_ice_mass, TRC_EVAP_KIND_SUBL)
         IF (trc_rnof > 0._r8) THEN
            trc_rnof_step(itrc, ipatch) = trc_rnof
            a_trc_rsur(itrc, ipatch) = a_trc_rsur(itrc, ipatch) + trc_rnof
            a_trc_rnof(itrc, ipatch) = a_trc_rnof(itrc, ipatch) + trc_rnof
         ENDIF
         CALL sync_tracer_patch_ratio(itrc, ipatch, snl_trc, maxsnl, nl_soil, &
            wliq_soisno, wice_soisno, 0._r8, wdsrf, scv, R_final)
         IF (nonvolatile_solute .and. .not. tracer_has_dissolved_limit(itrc) .and. &
             trc_surface_residue(itrc, ipatch) > trc_tiny) THEN
            IF (wdsrf > trc_water_min_for_ratio) THEN
               trc_wdsrf(itrc, ipatch) = trc_wdsrf(itrc, ipatch) + &
                  trc_surface_residue(itrc, ipatch)
               trc_surface_residue(itrc, ipatch) = 0._r8
            ELSEIF (wliq_soisno(1) > trc_water_min_for_ratio) THEN
               trc_wliq_soisno(itrc, 1, ipatch) = &
                  trc_wliq_soisno(itrc, 1, ipatch) + &
                  trc_surface_residue(itrc, ipatch)
               trc_surface_residue(itrc, ipatch) = 0._r8
            ENDIF
         ENDIF
      ENDDO

      CALL tracer_apply_reactive_processes(ipatch, snl_trc, nl_soil, deltim)
      CALL tracer_balance_check(ipatch, snl_trc, nl_soil, deltim, xerr_tracer, &
         patchtype_in = 3, water_err_in = errorw, water_dS_in = water_dS, &
         water_input_in = precip_mass + dep_mass, water_output_in = evap_mass + rnof_mass, &
         water_evap_in = evap_mass, water_rnof_in = rnof_mass)
      CALL tracer_hist_accumulate(ipatch, snl_trc, maxsnl, nl_soil, 0._r8, 0._r8, &
         wliq_soisno(snl_trc+1:nl_soil), wice_soisno(snl_trc+1:nl_soil), 0._r8, wdsrf, 0._r8, scv)

   CONTAINS

      real(r8) FUNCTION glacier_evap_ratio_for (source_ratio, temp_k, from_ice)
         real(r8), intent(in) :: source_ratio, temp_k
         logical,  intent(in) :: from_ice
         real(r8) :: relhum, alpha_k, alpha_k_liq, alpha_k_ice

         glacier_evap_ratio_for = source_ratio
         IF (.not. tracer_fractionation_active(itrc)) RETURN
         alpha_k_liq = tracer_alpha_kinetic_craig_gordon(itrc, .false.)
         alpha_k_ice = tracer_alpha_kinetic_craig_gordon(itrc, .true.)
         alpha_k = merge(alpha_k_ice, alpha_k_liq, from_ice)
         relhum = tracer_surface_relhum(forc_q, forc_psrf, temp_k, from_ice)
         glacier_evap_ratio_for = tracer_craig_gordon_evap_ratio(itrc, source_ratio, R_vapor, &
            temp_k, relhum, alpha_k, from_ice)
      END FUNCTION glacier_evap_ratio_for

   END SUBROUTINE tracer_glacier_patch

   SUBROUTINE tracer_waterbody_patch(ipatch, maxsnl, nl_soil, snl, deltim, &
      forc_rain, forc_snow, lake_deficit, rnof, qseva, qsubl, qsdew, qfros, &
      endwb, totwb, errorw, wa, wdsrf, scv, t_grnd, forc_q, forc_psrf, &
      forc_us, forc_vs, &
      wliq_soisno, wice_soisno, use_dynamic_lake)

      IMPLICIT NONE
      integer,  intent(in) :: ipatch, maxsnl, nl_soil, snl
      real(r8), intent(in) :: deltim
      real(r8), intent(in) :: forc_rain, forc_snow, lake_deficit, rnof
      real(r8), intent(in) :: qseva, qsubl, qsdew, qfros
      real(r8), intent(in) :: endwb, totwb, errorw, wa, wdsrf, scv, t_grnd, forc_q, forc_psrf
      ! Wind is needed because open-water kinetic fractionation is
      ! wind-dependent (Merlivat & Jouzel 1979).
      real(r8), intent(in) :: forc_us, forc_vs
      real(r8), intent(in) :: wliq_soisno(maxsnl+1:nl_soil)
      real(r8), intent(in) :: wice_soisno(maxsnl+1:nl_soil)
      logical,  intent(in) :: use_dynamic_lake

      integer  :: itrc, j_trc
      real(r8) :: R_init, R_precip, R_vapor, R_pool, R_final
      real(r8) :: R_dew, R_frost, R_runoff
      real(r8) :: atm_precip_mass, deficit_mass, precip_mass, rnof_mass
      real(r8) :: evap_mass, dep_mass
      real(r8) :: evap_liq_mass, evap_ice_mass, dep_liq_mass, dep_ice_mass
      real(r8) :: water_dS, water_end, water_beg, water_input
      real(r8) :: water_before_output, water_after_evap, water_after_liq
      real(r8) :: trc_input, trc_evap, trc_evap_liq, trc_evap_ice, trc_after_liq
      real(r8) :: trc_rnof, trc_available, trc_final
      real(r8) :: trc_held_storage, surface_residue_beg
      real(r8) :: surface_liquid_end, surface_carrier, surface_residue_export
      real(r8) :: xerr_tracer
      logical  :: mixed_signature, fixed_signature, frac_active, nonvolatile_solute

      DO itrc = 1, ntracers
         IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
         ! Waterbody patches have no canopy, wetland, or irrigation-reservoir
         ! carrier in this branch. Preserve conservative transition mass in a
         ! surface quarantine instead of reporting a carrierless concentration.
         IF (tracer_is_nonvolatile_solute(itrc)) THEN
            CALL move_special_subsurface_residue_to_surface(itrc, ipatch)
            IF (tracer_has_dissolved_limit(itrc)) THEN
               trc_surface_solid(itrc, ipatch) = trc_surface_solid(itrc, ipatch) + &
                  trc_surface_residue(itrc, ipatch) + trc_ldew_rain(itrc, ipatch) + &
                  trc_ldew_snow(itrc, ipatch) + trc_wetwat(itrc, ipatch) + &
                  trc_canopy_solid(itrc, ipatch)
               trc_surface_residue(itrc, ipatch) = 0._r8
               trc_surface_solid(itrc, ipatch) = trc_surface_solid(itrc, ipatch) + &
                  sum(trc_solid_soisno(itrc, maxsnl+1:0, ipatch))
               trc_solid_soisno(itrc, maxsnl+1:0, ipatch) = 0._r8
               IF (allocated(trc_waterstorage)) THEN
                  trc_surface_solid(itrc, ipatch) = trc_surface_solid(itrc, ipatch) + &
                     trc_waterstorage(itrc, ipatch) + trc_waterstorage_solid(itrc, ipatch)
               ENDIF
            ELSE
               trc_surface_residue(itrc, ipatch) = trc_surface_residue(itrc, ipatch) + &
                  trc_ldew_rain(itrc, ipatch) + trc_ldew_snow(itrc, ipatch) + &
                  trc_wetwat(itrc, ipatch)
               IF (allocated(trc_waterstorage)) THEN
                  trc_surface_residue(itrc, ipatch) = trc_surface_residue(itrc, ipatch) + &
                     trc_waterstorage(itrc, ipatch)
               ENDIF
            ENDIF
         ENDIF
         trc_ldew_rain(itrc, ipatch) = 0._r8
         trc_ldew_snow(itrc, ipatch) = 0._r8
         trc_wetwat   (itrc, ipatch) = 0._r8
         trc_canopy_solid(itrc, ipatch) = 0._r8
         IF (allocated(trc_waterstorage)) trc_waterstorage(itrc, ipatch) = 0._r8
         trc_waterstorage_solid(itrc, ipatch) = 0._r8
      ENDDO

      CALL tracer_save_storage(ipatch, maxsnl, nl_soil)
      atm_precip_mass = (forc_rain + forc_snow) * deltim
      IF (use_dynamic_lake) THEN
         precip_mass = atm_precip_mass
      ELSE
         precip_mass = (forc_rain + forc_snow + lake_deficit) * deltim
      ENDIF
      deficit_mass = precip_mass - atm_precip_mass
      rnof_mass   = max(rnof, 0._r8) * deltim
      evap_liq_mass = max(qseva, 0._r8) * deltim
      evap_ice_mass = max(qsubl, 0._r8) * deltim
      dep_liq_mass  = max(qsdew, 0._r8) * deltim
      dep_ice_mass  = max(qfros, 0._r8) * deltim
      evap_mass = evap_liq_mass + evap_ice_mass
      dep_mass  = dep_liq_mass  + dep_ice_mass
      water_input = precip_mass + dep_mass
      water_dS = endwb - totwb
      water_end = wa + max(wdsrf, 0._r8)
      DO j_trc = maxsnl + 1, nl_soil
         IF (j_trc >= snl + 1) THEN
            water_end = water_end + max(wliq_soisno(j_trc), 0._r8) + max(wice_soisno(j_trc), 0._r8)
         ENDIF
      ENDDO
      IF (snl >= 0) water_end = water_end + max(scv, 0._r8)
      water_beg = water_end - water_dS

      DO itrc = 1, ntracers
         IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
         trc_rnof_step(itrc, ipatch) = 0._r8
         R_init = tracer_init_water_ratio(itrc)
         frac_active = tracer_fractionation_active(itrc)
         nonvolatile_solute = tracer_is_nonvolatile_solute(itrc)
         surface_residue_beg = trc_surface_residue(itrc, ipatch)
         trc_held_storage = trc_subsurface_residue(itrc, ipatch) + surface_residue_beg + &
            solid_inventory(itrc, ipatch, maxsnl + 1, nl_soil)
         fixed_signature = tracer_can_use_fixed_signature(itrc) .and. .not. frac_active
         IF (allocated(trc_runtime_forced)) THEN
            fixed_signature = fixed_signature .and. .not. trc_runtime_forced(itrc)
         ENDIF
         mixed_signature = .not. fixed_signature

         IF (mixed_signature) THEN
            R_precip = tracer_forcing_precip_value(itrc, ipatch)
            R_vapor  = tracer_forcing_vapor_value (itrc, ipatch)
            IF (water_beg > trc_water_min_for_ratio) THEN
               R_pool = max(trc_storage_beg(itrc, ipatch) - &
                  trc_held_storage, 0._r8) / water_beg
            ELSE
               R_pool = R_precip
            ENDIF
            IF (nonvolatile_solute) THEN
               R_dew = 0._r8
               R_frost = 0._r8
            ELSE
               R_dew = R_vapor
               R_frost = R_vapor
               IF (frac_active) THEN
                  R_dew = tracer_equilibrium_deposition_ratio(itrc, R_vapor, t_grnd, .false.)
                  R_frost = tracer_equilibrium_deposition_ratio(itrc, R_vapor, t_grnd, .true.)
               ENDIF
            ENDIF
            trc_input = atm_precip_mass * R_precip + dep_liq_mass * R_dew + &
               dep_ice_mass * R_frost + deficit_mass * R_pool
            trc_available = max(trc_storage_beg(itrc, ipatch) - &
               trc_held_storage + trc_input, 0._r8)
            water_before_output = water_beg + water_input
            IF (nonvolatile_solute) THEN
               trc_evap_liq = 0._r8
               trc_evap_ice = 0._r8
            ELSE
               trc_evap_liq = tracer_atmospheric_tracer_loss(trc_available, water_before_output, &
                  evap_liq_mass, t_grnd, .false., waterbody_evap_ratio_for, trc_tiny, &
                  tracer_ratio_cap(itrc), nonvolatile_solute)
               trc_after_liq = trc_available - trc_evap_liq
               water_after_liq = water_before_output - evap_liq_mass
               trc_evap_ice = tracer_atmospheric_tracer_loss(trc_after_liq, water_after_liq, &
                  evap_ice_mass, t_grnd, .true., waterbody_evap_ratio_for, trc_tiny, &
                  tracer_ratio_cap(itrc), nonvolatile_solute, skin_mass = DEF_TRACER_SUBL_SKIN_MM)
            ENDIF
            trc_evap = trc_evap_liq + trc_evap_ice
            water_after_evap = water_before_output - evap_mass
            trc_final = max(trc_available - trc_evap, 0._r8)
            CALL tracer_equilibrate_dissolved(itrc, water_after_evap, trc_final, &
               trc_surface_solid(itrc, ipatch))
            IF (water_after_evap > trc_water_min_for_ratio) THEN
               R_runoff = trc_final / water_after_evap
            ELSE
               R_runoff = 0._r8
            ENDIF
            trc_rnof = min(rnof_mass * R_runoff, trc_final)
            surface_liquid_end = max(wdsrf, 0._r8) + &
               max(wliq_soisno(snl+1), 0._r8)
            surface_carrier = surface_liquid_end + rnof_mass
            surface_residue_export = 0._r8
            IF (nonvolatile_solute .and. .not. tracer_has_dissolved_limit(itrc) .and. &
                surface_residue_beg > trc_tiny .and. &
                rnof_mass > trc_tiny .and. &
                surface_carrier > trc_water_min_for_ratio) THEN
               surface_residue_export = min(surface_residue_beg * &
                  rnof_mass / surface_carrier, surface_residue_beg)
               surface_residue_beg = surface_residue_beg - surface_residue_export
               trc_rnof = trc_rnof + surface_residue_export
            ENDIF
            ! surface_residue_export comes from held mass, not trc_available.
            trc_final = max(trc_final - trc_rnof + surface_residue_export, 0._r8)
            CALL tracer_equilibrate_dissolved(itrc, water_end, trc_final, &
               trc_surface_solid(itrc, ipatch))
            IF (water_end > trc_water_min_for_ratio) THEN
               R_final = trc_final / water_end
               IF (nonvolatile_solute) trc_surface_residue(itrc, ipatch) = surface_residue_beg
            ELSE
               R_final = 0._r8
               IF (nonvolatile_solute) THEN
                  trc_surface_residue(itrc, ipatch) = surface_residue_beg + trc_final
               ENDIF
            ENDIF
         ELSE
            trc_input = water_input * R_init
            trc_evap_liq = evap_liq_mass * R_init
            trc_evap_ice = evap_ice_mass * R_init
            trc_evap = trc_evap_liq + trc_evap_ice
            trc_rnof  = rnof_mass * R_init
            R_final   = R_init
         ENDIF

         IF (trc_input > 0._r8) a_trc_precip(itrc, ipatch) = a_trc_precip(itrc, ipatch) + trc_input
         CALL tracer_book_evap_loss(itrc, ipatch, trc_evap_liq, evap_liq_mass, TRC_EVAP_KIND_SOILEVAP)
         CALL tracer_book_evap_loss(itrc, ipatch, trc_evap_ice, evap_ice_mass, TRC_EVAP_KIND_SUBL)
         IF (trc_rnof > 0._r8) THEN
            trc_rnof_step(itrc, ipatch) = trc_rnof
            a_trc_rsur(itrc, ipatch) = a_trc_rsur(itrc, ipatch) + trc_rnof
            a_trc_rnof(itrc, ipatch) = a_trc_rnof(itrc, ipatch) + trc_rnof
         ENDIF
         CALL sync_tracer_patch_ratio(itrc, ipatch, snl, maxsnl, nl_soil, &
            wliq_soisno, wice_soisno, wa, wdsrf, scv, R_final)
         IF (nonvolatile_solute .and. .not. tracer_has_dissolved_limit(itrc) .and. &
             trc_surface_residue(itrc, ipatch) > trc_tiny) THEN
            IF (wdsrf > trc_water_min_for_ratio) THEN
               trc_wdsrf(itrc, ipatch) = trc_wdsrf(itrc, ipatch) + &
                  trc_surface_residue(itrc, ipatch)
               trc_surface_residue(itrc, ipatch) = 0._r8
            ELSEIF (wliq_soisno(snl+1) > trc_water_min_for_ratio) THEN
               trc_wliq_soisno(itrc, snl+1, ipatch) = &
                  trc_wliq_soisno(itrc, snl+1, ipatch) + &
                  trc_surface_residue(itrc, ipatch)
               trc_surface_residue(itrc, ipatch) = 0._r8
            ENDIF
         ENDIF
      ENDDO

      CALL tracer_apply_reactive_processes(ipatch, maxsnl, nl_soil, deltim)
      CALL tracer_balance_check(ipatch, maxsnl, nl_soil, deltim, xerr_tracer, &
         patchtype_in = 4, water_err_in = errorw, water_dS_in = water_dS, &
         water_input_in = precip_mass + dep_mass, water_output_in = evap_mass + rnof_mass, &
         water_evap_in = evap_mass, water_rnof_in = rnof_mass)
      CALL tracer_hist_accumulate(ipatch, snl, maxsnl, nl_soil, 0._r8, 0._r8, &
         wliq_soisno(snl+1:nl_soil), wice_soisno(snl+1:nl_soil), wa, wdsrf, 0._r8, scv)

   CONTAINS

      real(r8) FUNCTION waterbody_evap_ratio_for (source_ratio, temp_k, from_ice)
         real(r8), intent(in) :: source_ratio, temp_k
         logical,  intent(in) :: from_ice
         real(r8) :: relhum, alpha_k, alpha_k_liq, alpha_k_ice

         waterbody_evap_ratio_for = source_ratio
         IF (.not. tracer_fractionation_active(itrc)) RETURN
         alpha_k_liq = tracer_alpha_kinetic_open_water(itrc, &
            sqrt(max(forc_us*forc_us + forc_vs*forc_vs, 0._r8)))
         alpha_k_ice = tracer_alpha_kinetic_craig_gordon(itrc, .true.)
         alpha_k = merge(alpha_k_ice, alpha_k_liq, from_ice)
         relhum = tracer_surface_relhum(forc_q, forc_psrf, temp_k, from_ice)
         waterbody_evap_ratio_for = tracer_craig_gordon_evap_ratio(itrc, source_ratio, R_vapor, &
            temp_k, relhum, alpha_k, from_ice)
      END FUNCTION waterbody_evap_ratio_for

   END SUBROUTINE tracer_waterbody_patch

   SUBROUTINE move_special_subsurface_residue_to_surface (itrc, ipatch)
      integer, intent(in) :: itrc, ipatch

      IF (trc_subsurface_residue(itrc, ipatch) <= trc_tiny) RETURN
      IF (tracer_has_dissolved_limit(itrc)) THEN
         trc_surface_solid(itrc, ipatch) = trc_surface_solid(itrc, ipatch) + &
            trc_subsurface_residue(itrc, ipatch)
      ELSE
         trc_surface_residue(itrc, ipatch) = trc_surface_residue(itrc, ipatch) + &
            trc_subsurface_residue(itrc, ipatch)
      ENDIF
      trc_subsurface_residue(itrc, ipatch) = 0._r8
   END SUBROUTINE move_special_subsurface_residue_to_surface

   real(r8) FUNCTION solid_inventory (itrc, ipatch, lb, nl_soil)
      integer, intent(in) :: itrc, ipatch, lb, nl_soil

      solid_inventory = trc_canopy_solid(itrc, ipatch) + &
         trc_surface_solid(itrc, ipatch) + trc_subsurface_solid(itrc, ipatch) + &
         trc_waterstorage_solid(itrc, ipatch)
      IF (lb <= nl_soil) solid_inventory = solid_inventory + &
         sum(trc_solid_soisno(itrc, lb:nl_soil, ipatch))
   END FUNCTION solid_inventory


   real(r8) FUNCTION tracer_ratio_cap (itrc)
      integer, intent(in) :: itrc

      tracer_ratio_cap = merge(tracers(itrc)%ref_ratio * (1._r8 + trc_delta_sanity_max / 1000._r8), &
         0._r8, tracer_fractionation_active(itrc))
   END FUNCTION tracer_ratio_cap

END MODULE MOD_Tracer_SpecialPatches
#endif
