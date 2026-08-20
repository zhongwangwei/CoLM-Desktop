#include <define.h>

#ifdef TRACER
MODULE MOD_Tracer_Precip

   USE MOD_Precision
   USE MOD_Tracer_Defs, only: ntracers, trc_tiny, trc_water_min_for_ratio, &
      tracer_uses_land_water_transport, tracer_equilibrate_dissolved
   USE MOD_Tracer_Forcing, only: tracer_forcing_precip_value
   USE MOD_Tracer_Vars, only: trc_ldew_rain, trc_ldew_snow, a_trc_precip, &
      trc_pg_rain_ground, trc_pg_snow_ground, trc_waterstorage, &
      trc_canopy_solid, trc_surface_solid, trc_waterstorage_solid

   IMPLICIT NONE

CONTAINS

   !---------------------------------------------------------------
   ! Canopy tracer update after LEAF_INTERCEPTION.
   !
   ! Physical process:
   !   1. Precipitation arrives: forc_rain, forc_snow
   !   2. Part intercepted (qintr): mixes with EXISTING canopy water
   !   3. Part passes through (throughfall): keeps precipitation R
   !   4. If canopy saturates, excess drips: carries MIXED canopy R
   !   5. Phase change inside LEAF_INTERCEPTION (NoahMP/MATSIRO/VIC/
   !      JULES) moves ldew_smelt_out from ldew_snow to ldew_rain and/or
   !      ldew_frzc_out from ldew_rain to ldew_snow. Tracer mass has to
   !      migrate coherently, otherwise the rain pool loses signal from
   !      the melt event (and vice versa for freeze) and the snow tracer
   !      gets mis-attributed as drip to trc_pg_snow_ground downstream.
   !
   ! Tracer to ground = throughfall*R_precip + drip*R_mixed
   !   (rain → trc_pg_rain_ground → surface pool via tracer_soil_water;
   !    snow → trc_pg_snow_ground → trc_scv via tracer_newsnow)
   !
   ! Canopy tracer: migrate phase change first (updates "effective"
   ! pre-interception anchor so the rain/snow stages below see a
   ! consistent (ldew_*_old, trc_ldew_*) pair), then for each phase
   ! remove xsc, mix interception with remainder, and finally strip drip.
   !---------------------------------------------------------------
   SUBROUTINE tracer_precip (ipatch, deltim, &
      forc_rain, forc_snow, qintr, qintr_rain, qintr_snow, &
      pg_rain, pg_snow, ldew_rain, ldew_snow, &
      ldew_rain_old, ldew_snow_old, qflx_irrig_sprinkler, &
      gross_intr_rain, gross_intr_snow, &
      xsc_rain_out, xsc_snow_out, &
      ldew_smelt_mass, ldew_frzc_mass, waterstorage_before)

      IMPLICIT NONE
      integer,  intent(in) :: ipatch
      real(r8), intent(in) :: deltim
      real(r8), intent(in) :: forc_rain, forc_snow
      real(r8), intent(in) :: qintr, qintr_rain, qintr_snow
      real(r8), intent(in) :: pg_rain, pg_snow
      real(r8), intent(in) :: ldew_rain, ldew_snow         ! AFTER interception
      real(r8), intent(in) :: ldew_rain_old, ldew_snow_old ! BEFORE interception
      ! Sprinkler irrigation rate (mm/s). LEAF_INTERCEPTION merges this
      ! into the rain stream (MOD_LeafInterception.F90:187,232), so the
      ! "total rain arriving" at the patch is (forc_rain + this). Without
      ! counting it here the tracer input is under-reported and the
      ! throughfall-vs-interception partition uses the wrong denominator.
      real(r8), intent(in) :: qflx_irrig_sprinkler
      ! Gross rain/snow rate entering the canopy mixed pool [mm/s], >=0.
      ! Returned by LEAF_INTERCEPTION alongside qintr (net). Used here to
      ! correctly attribute isotope signatures on canopy-release events:
      ! schemes 3/5/6/7 can have qintr_rain < 0 when stored canopy water
      ! drips out while new rain simultaneously enters the pool. Using
      ! only qintr (net) would attribute the released canopy water to the
      ! fresh-precipitation signature R_input. Using gross_intr_* gives
      ! the right R_mixed for the drip and R_input for the pure-gap part.
      real(r8), intent(in) :: gross_intr_rain
      real(r8), intent(in) :: gross_intr_snow
      ! Pre-mix old-pool release rate [mm/s], >=0. In most schemes a
      ! portion of the OLD canopy pool is flushed BEFORE new precipitation
      ! mixes in (capacity overflow on LAI-driven satcap shrink, phase-
      ! change overflow, snow unloading in NoahMP, etc). That mass carries
      ! the pre-mix canopy signature, NOT R_mixed. Without separating it
      ! out, the tracer drip term would lump xsc with the post-mix tex
      ! and dilute old canopy signatures with fresh precipitation.
      real(r8), intent(in) :: xsc_rain_out
      real(r8), intent(in) :: xsc_snow_out
      ! Canopy rain<->snow phase change mass [grid-scale mm, >=0].
      ! ldew_smelt_mass is the canopy snow -> rain transfer this step
      ! (melt); ldew_frzc_mass is the canopy rain -> snow transfer
      ! (freeze). Each nonzero value triggers an equal-mass tracer
      ! migration between trc_ldew_snow and trc_ldew_rain before the
      ! rain/snow stages run, and the pre-interception anchors
      ! (ldew_*_old_eff, trc_ldew_*) are updated so R_canopy_pre in each
      ! stage reflects the post-migration pool composition. Without this
      ! step the snow-pool tracer would be spuriously classified as drip
      ! and sent to trc_pg_snow_ground (and the rain pool would
      ! under-report the melt signal).
      real(r8), intent(in) :: ldew_smelt_mass
      real(r8), intent(in) :: ldew_frzc_mass
      ! Reservoir water before sprinkler withdrawal. The bulk irrigation
      ! routine has already debited waterstorage by the time this routine runs.
      real(r8), intent(in), optional :: waterstorage_before

      integer  :: itrc
      real(r8) :: R_input, storage_ratio, R_rain_input
      real(r8) :: intercepted, drip, throughfall
      real(r8) :: trc_mixed, water_mixed, R_mixed
      real(r8) :: trc_throughfall, trc_drip
      real(r8) :: trc_rain_ground, trc_snow_ground
      real(r8) :: rain_total           ! forc_rain + sprinkler [mm/s]
      real(r8) :: sprinkler_rate       ! non-negative sprinkler water input [mm/s]
      real(r8) :: sprinkler_water      ! sprinkler water transferred this step [mm]
      real(r8) :: xsc_mass             ! pre-mix release in this step [mm]
      real(r8) :: trc_xsc              ! pre-mix release tracer mass
      real(r8) :: R_canopy_pre         ! canopy signature BEFORE mix
      real(r8) :: ldew_rain_pre_mix    ! post-xsc, pre-mix storage [mm]
      real(r8) :: ldew_snow_pre_mix
      ! Effective pre-interception anchors after phase-change migration.
      ! Equal to ldew_*_old when no phase change happened; otherwise
      ! reflect the post-migration pool so R_canopy_pre stays consistent
      ! with the (migrated) trc_ldew_* values that feed the rain/snow
      ! stages. Clamped to >=0 in case of float noise in the caller.
      real(r8) :: ldew_rain_old_eff
      real(r8) :: ldew_snow_old_eff
      real(r8) :: smelt_mass, frzc_mass ! effective phase-change masses
      real(r8) :: R_snow_pre, R_rain_pre
      real(r8) :: trc_smelt, trc_frzc   ! tracer mass migrated this step
      real(r8) :: canopy_trc_beg, canopy_trc_end, canopy_input_trc
      real(r8) :: canopy_resid, ground_trc_total, desired_ground_total
      real(r8) :: sprinkler_trc
      real(r8) :: canopy_old_water, canopy_release_trc

      IF (ntracers <= 0) RETURN

      ! Interception / pg_rain / ldew already see the sprinkler-boosted
      ! rain stream from LEAF_INTERCEPTION. Use the boosted total here
      ! so the throughfall computation matches (intercepted + throughfall
      ! ≡ rain_total*deltim) and the system-input accounting covers
      ! both natural precipitation and sprinkler irrigation.
      sprinkler_rate = max(qflx_irrig_sprinkler, 0._r8)
      sprinkler_water = sprinkler_rate * deltim
      rain_total = max(forc_rain, 0._r8) + sprinkler_rate

      DO itrc = 1, ntracers
         IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
         R_input = tracer_forcing_precip_value(itrc, ipatch)
         storage_ratio = R_input
         CALL tracer_equilibrate_dissolved(itrc, max(ldew_rain_old, 0._r8), &
            trc_ldew_rain(itrc, ipatch), trc_canopy_solid(itrc, ipatch))
         canopy_trc_beg = trc_ldew_rain(itrc, ipatch) + trc_ldew_snow(itrc, ipatch) &
            + trc_canopy_solid(itrc, ipatch)

         ! Atmospheric precipitation is external input; count into the
         ! precip accumulator. Sprinkler irrigation also reaches the
         ! canopy/ground here via LEAF_INTERCEPTION, but water-side
         ! treats it as an internal transfer from waterstorage (not a
         ! term in forc_prc+forc_prl). Mirror that: debit
         ! trc_waterstorage for sprinkler and keep a_trc_precip purely
         ! atmospheric, so the tracer budget's "external input"
         ! matches the water-side definition.
         a_trc_precip(itrc, ipatch) = a_trc_precip(itrc, ipatch) &
            + (forc_rain + forc_snow) * R_input * deltim

         IF (qflx_irrig_sprinkler > trc_tiny .and. allocated(trc_waterstorage)) THEN
            IF (present(waterstorage_before)) THEN
               CALL tracer_equilibrate_dissolved(itrc, max(waterstorage_before, 0._r8), &
                  trc_waterstorage(itrc, ipatch), trc_waterstorage_solid(itrc, ipatch))
               IF (waterstorage_before > trc_water_min_for_ratio) THEN
                  storage_ratio = trc_waterstorage(itrc, ipatch) / waterstorage_before
               ENDIF
            ENDIF
            sprinkler_trc = sprinkler_water * storage_ratio
            sprinkler_trc = min(max(sprinkler_trc, 0._r8), max(trc_waterstorage(itrc, ipatch), 0._r8))
            trc_waterstorage(itrc, ipatch) = trc_waterstorage(itrc, ipatch) &
               - sprinkler_trc
            ! The canopy must receive exactly the tracer actually removed,
            ! including near-dry and dissolved-inventory-limited withdrawals.
            IF (sprinkler_water > trc_tiny) storage_ratio = sprinkler_trc / sprinkler_water
            IF (present(waterstorage_before)) THEN
               CALL tracer_equilibrate_dissolved(itrc, &
                  max(waterstorage_before - sprinkler_water, 0._r8), &
                  trc_waterstorage(itrc, ipatch), trc_waterstorage_solid(itrc, ipatch))
            ENDIF
         ELSE
            sprinkler_trc = sprinkler_water * storage_ratio
         ENDIF

         R_rain_input = R_input
         IF (rain_total > trc_tiny) THEN
            R_rain_input = (max(forc_rain, 0._r8) * R_input + sprinkler_rate * storage_ratio) / rain_total
         ENDIF

         trc_rain_ground = 0._r8
         trc_snow_ground = 0._r8

         ! ---- Stage 0: canopy phase-change migration ----
         ! NoahMP/MATSIRO/VIC/JULES may have melted part of the canopy
         ! snow pool into the rain pool (ldew_smelt_mass > 0) or frozen
         ! part of the rain pool into the snow pool (ldew_frzc_mass > 0)
         ! BEFORE touching any new precipitation this step. Without
         ! migrating the tracer mass along with the bulk water, the snow
         ! block below would see d_ldew < 0 (for melt) and falsely
         ! classify ldew_smelt_mass as drip to trc_pg_snow_ground, while
         ! the rain pool would silently lose the meltwater's signature.
         ! Clamp each mass to the pool available when that transfer runs;
         ! melt precedes freeze in this aggregate, well-mixed canopy state.
         ldew_rain_old_eff = max(0._r8, ldew_rain_old)
         ldew_snow_old_eff = max(0._r8, ldew_snow_old)
         smelt_mass = min(max(0._r8, ldew_smelt_mass), ldew_snow_old_eff)
         frzc_mass  = max(0._r8, ldew_frzc_mass)

         IF (smelt_mass > 0._r8) THEN
            IF (ldew_snow_old_eff > trc_tiny) THEN
               R_snow_pre = trc_ldew_snow(itrc, ipatch) / ldew_snow_old_eff
            ELSE
               R_snow_pre = R_input
            ENDIF
            trc_smelt = smelt_mass * R_snow_pre
            trc_ldew_snow(itrc, ipatch) = max(0._r8, &
               trc_ldew_snow(itrc, ipatch) - trc_smelt)
            trc_ldew_rain(itrc, ipatch) = trc_ldew_rain(itrc, ipatch) + trc_smelt
            ldew_snow_old_eff = ldew_snow_old_eff - smelt_mass
            ldew_rain_old_eff = ldew_rain_old_eff + smelt_mass
         ENDIF

         frzc_mass = min(frzc_mass, ldew_rain_old_eff)
         IF (frzc_mass > 0._r8) THEN
            ! ponytail: interception exposes aggregate phase mass but no
            ! freeze-event temperature (and PFTs can differ); keep this
            ! transfer conservative until temperature-weighted diagnostics exist.
            IF (ldew_rain_old_eff > trc_tiny) THEN
               R_rain_pre = trc_ldew_rain(itrc, ipatch) / ldew_rain_old_eff
            ELSE
               R_rain_pre = R_input
            ENDIF
            trc_frzc = frzc_mass * R_rain_pre
            trc_ldew_rain(itrc, ipatch) = max(0._r8, &
               trc_ldew_rain(itrc, ipatch) - trc_frzc)
            trc_ldew_snow(itrc, ipatch) = trc_ldew_snow(itrc, ipatch) + trc_frzc
            ldew_rain_old_eff = ldew_rain_old_eff - frzc_mass
            ldew_snow_old_eff = ldew_snow_old_eff + frzc_mass
         ENDIF

         ! LEAF_interception has a carrier-empty branch for lai+sai~=0 that
         ! releases the entire old single-bucket canopy store as rain or snow
         ! according to tleaf, then zeros both ldew phase pools.  Its xsc
         ! diagnostic therefore names the RELEASE phase, not necessarily the
         ! phase in which the old water/tracer entered this routine.  Repack the
         ! complete old canopy inventory into that host-selected phase before
         ! the ordinary xsc calculation.  Otherwise old snow released as rain
         ! remains stranded in trc_ldew_snow (and old rain released as snow is
         ! precipitated into trc_canopy_solid when the now-empty liquid carrier
         ! is equilibrated below).
         canopy_old_water = ldew_rain_old_eff + ldew_snow_old_eff
         IF (max(ldew_rain, 0._r8) + max(ldew_snow, 0._r8) <= trc_water_min_for_ratio .and. &
             canopy_old_water > trc_water_min_for_ratio .and. &
             (((max(xsc_rain_out, 0._r8) * deltim > trc_water_min_for_ratio) .and. &
                (max(xsc_snow_out, 0._r8) * deltim <= trc_water_min_for_ratio)) .or. &
              ((max(xsc_snow_out, 0._r8) * deltim > trc_water_min_for_ratio) .and. &
                (max(xsc_rain_out, 0._r8) * deltim <= trc_water_min_for_ratio))) .and. &
             (max(xsc_rain_out, 0._r8) + max(xsc_snow_out, 0._r8)) * deltim &
                >= canopy_old_water - trc_water_min_for_ratio) THEN
            canopy_release_trc = trc_ldew_rain(itrc, ipatch) + &
               trc_ldew_snow(itrc, ipatch) + trc_canopy_solid(itrc, ipatch)
            trc_canopy_solid(itrc, ipatch) = 0._r8
            IF (max(xsc_rain_out, 0._r8) * deltim > trc_water_min_for_ratio) THEN
               trc_ldew_rain(itrc, ipatch) = canopy_release_trc
               trc_ldew_snow(itrc, ipatch) = 0._r8
               ldew_rain_old_eff = canopy_old_water
               ldew_snow_old_eff = 0._r8
            ELSE
               trc_ldew_rain(itrc, ipatch) = 0._r8
               trc_ldew_snow(itrc, ipatch) = canopy_release_trc
               ldew_rain_old_eff = 0._r8
               ldew_snow_old_eff = canopy_old_water
            ENDIF
         ENDIF

         ! ---- Rain component (2-stage isotope attribution) ----
         ! Stage 1: PRE-MIX release. xsc_rain_out is the fraction of the
         ! OLD canopy pool flushed out before any new rain mixes in
         ! (capacity overflow, phase-change overflow, unloading). It
         ! carries the canopy's pre-mix signature R_canopy_pre, derived
         ! from the entry tracer storage and entry bulk water.
         ! Use the post-phase-change effective old-pool so R_canopy_pre
         ! reflects the actual pool entering the xsc stage.
         xsc_mass = min(max(0._r8, xsc_rain_out) * deltim, ldew_rain_old_eff)
         IF (ldew_rain_old_eff > trc_tiny) THEN
            R_canopy_pre = trc_ldew_rain(itrc, ipatch) / ldew_rain_old_eff
         ELSE
            R_canopy_pre = R_rain_input
         ENDIF
         trc_xsc = xsc_mass * R_canopy_pre
         ldew_rain_pre_mix = ldew_rain_old_eff - xsc_mass

         ! Stage 2: gross rain enters the (post-release) canopy pool at
         ! the mixed atmospheric+sprinkler rain signature. Blend to obtain R_mixed.
         intercepted = max(0._r8, gross_intr_rain) * deltim
         water_mixed = ldew_rain_pre_mix + intercepted
         trc_mixed = (trc_ldew_rain(itrc, ipatch) - trc_xsc) &
                   + intercepted * R_rain_input
         IF (water_mixed > trc_tiny) THEN
            R_mixed = trc_mixed / water_mixed
         ELSE
            R_mixed = R_rain_input
         ENDIF

         ! Pure-gap throughfall (never touched canopy).
         throughfall = max(rain_total * deltim - intercepted, 0._r8)

         ! Actual post-mix drip is defined by the water-side ground flux.
         ! This avoids relying on ldew_rain as a phase-specific final store:
         ! scheme=1 with DEF_VEG_SNOW=false updates only the single ldew
         ! bucket during interception.
         drip = max(max(pg_rain, 0._r8) * deltim - xsc_mass - throughfall, 0._r8)
         drip = min(drip, max(intercepted, 0._r8))

         ! Rain tracer reaching the ground: pure-gap throughfall keeps the
         ! input rain signature, while drip carries the mixed canopy ratio.
         trc_throughfall = throughfall * R_rain_input
         trc_drip = drip * R_mixed

         ! Canopy tracer: blended pool minus drip
         trc_ldew_rain(itrc, ipatch) = max(trc_mixed - drip * R_mixed, 0._r8)

         trc_rain_ground = trc_throughfall + trc_xsc + trc_drip

         ! ---- Snow component (same 2-stage logic) ----
         ! Use ldew_snow_old_eff (post-phase-change effective anchor) and
         ! the migrated trc_ldew_snow so R_canopy_pre matches the pool
         ! that actually feeds the xsc stage.
         xsc_mass = min(max(0._r8, xsc_snow_out) * deltim, ldew_snow_old_eff)
         IF (ldew_snow_old_eff > trc_tiny) THEN
            R_canopy_pre = trc_ldew_snow(itrc, ipatch) / ldew_snow_old_eff
         ELSE
            R_canopy_pre = R_input
         ENDIF
         trc_xsc = xsc_mass * R_canopy_pre
         ldew_snow_pre_mix = ldew_snow_old_eff - xsc_mass

         intercepted = max(0._r8, gross_intr_snow) * deltim
         water_mixed = ldew_snow_pre_mix + intercepted
         trc_mixed = (trc_ldew_snow(itrc, ipatch) - trc_xsc) &
                   + intercepted * R_input
         IF (water_mixed > trc_tiny) THEN
            R_mixed = trc_mixed / water_mixed
         ELSE
            R_mixed = R_input
         ENDIF

         throughfall = max(forc_snow * deltim - intercepted, 0._r8)
         drip = max(max(pg_snow, 0._r8) * deltim - xsc_mass - throughfall, 0._r8)
         drip = min(drip, max(intercepted, 0._r8))

         trc_ldew_snow(itrc, ipatch) = max(trc_mixed - drip * R_mixed, 0._r8)

         trc_snow_ground = throughfall * R_input + trc_xsc + drip * R_mixed

         ! Close the combined canopy interception budget exactly:
         !   Δ(canopy tracer) = rain/snow input tracer - ground tracer.
         ! The phase-specific partition above follows water-side release
         ! diagnostics, but several interception schemes expose only net
         ! pg_* plus clamped xsc/gross terms. That can leave O(1e-8)
         ! residuals which later show up as wetland/soil balance errors.
         ! Preserve the prognostic canopy pools and put the tiny correction
         ! on the internal ground flux consumed by tracer_soil_water /
         ! tracer_wetland; atmospheric a_trc_precip remains unchanged.
         CALL tracer_equilibrate_dissolved(itrc, max(ldew_rain, 0._r8), &
            trc_ldew_rain(itrc, ipatch), trc_canopy_solid(itrc, ipatch))
         canopy_trc_end = trc_ldew_rain(itrc, ipatch) + trc_ldew_snow(itrc, ipatch) &
            + trc_canopy_solid(itrc, ipatch)
         canopy_input_trc = rain_total * deltim * R_rain_input + max(forc_snow, 0._r8) * deltim * R_input
         canopy_resid = canopy_trc_end - canopy_trc_beg - canopy_input_trc &
                      + trc_rain_ground + trc_snow_ground
         IF (abs(canopy_resid) > trc_tiny) THEN
            ground_trc_total = trc_rain_ground + trc_snow_ground
            desired_ground_total = max(ground_trc_total - canopy_resid, 0._r8)
            IF (ground_trc_total > trc_tiny) THEN
               trc_rain_ground = trc_rain_ground * desired_ground_total / ground_trc_total
               trc_snow_ground = trc_snow_ground * desired_ground_total / ground_trc_total
            ELSEIF (rain_total > trc_tiny) THEN
               trc_rain_ground = desired_ground_total
            ELSE
               trc_snow_ground = desired_ground_total
            ENDIF
         ENDIF

         ! Ground rain is a liquid mobile pool. Excess solute precipitates
         ! directly into the surface solid inventory; existing surface solid
         ! may redissolve only up to the rainwater capacity.
         CALL tracer_equilibrate_dissolved(itrc, max(pg_rain, 0._r8) * deltim, &
            trc_rain_ground, trc_surface_solid(itrc, ipatch))

        ! Store rain/snow tracer separately: rain feeds the surface mixed
        ! pool in tracer_soil_water; snow feeds trc_scv in tracer_newsnow.
         trc_pg_rain_ground(itrc, ipatch) = trc_rain_ground
         trc_pg_snow_ground(itrc, ipatch) = trc_snow_ground

      ENDDO

   END SUBROUTINE tracer_precip

END MODULE MOD_Tracer_Precip
#endif
