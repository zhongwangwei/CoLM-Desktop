#include <define.h>

#ifdef TRACER
MODULE MOD_Tracer_Snow

   USE MOD_Precision
   USE MOD_Tracer_Defs, only: ntracers, trc_tiny, tracer_uses_land_water_transport, &
      tracer_equilibrate_dissolved
	   USE MOD_Tracer_Vars, only: trc_wliq_soisno, trc_wice_soisno, trc_pg_snow_ground, &
	      trc_scv, trc_wetwat, trc_solid_soisno

   IMPLICIT NONE

CONTAINS

   SUBROUTINE tracer_newsnow (ipatch, patchtype, snl, snl_old, pg_snow, deltim, &
      scv, scv_bef, wetwat_val, &
      wliq_soisno, wice_soisno, wice_soisno_bef)
      IMPLICIT NONE
      integer,  intent(in) :: ipatch, patchtype, snl, snl_old
      real(r8), intent(in) :: pg_snow                    ! snow throughfall [mm/s]
      real(r8), intent(in) :: deltim
      real(r8), intent(in) :: scv                        ! post-newsnow scv [mm]
      real(r8), intent(in) :: scv_bef                    ! pre-newsnow scv [mm]
      real(r8), intent(in) :: wetwat_val                 ! current wetwat [mm]
      ! These arrays are only valid when snl < 0
      real(r8), intent(in), optional :: wliq_soisno(:)
      real(r8), intent(in), optional :: wice_soisno(:)
      real(r8), intent(in), optional :: wice_soisno_bef(:)

      integer  :: itrc, j
      real(r8) :: R_snow_step, R_layer, d_wice, snow_mass_step
      real(r8) :: total_tracer, layer_water, ice_water, liq_water
      real(r8) :: ice_tracer, liq_tracer

      IF (ntracers <= 0) RETURN

      DO itrc = 1, ntracers
         IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
         snow_mass_step = max(pg_snow, 0._r8) * deltim
         IF (snow_mass_step > trc_tiny) THEN
            R_snow_step = trc_pg_snow_ground(itrc, ipatch) / snow_mass_step
         ELSE
            R_snow_step = 0._r8
         ENDIF

#if (defined CoLMDEBUG)
         ! Invariant: when the pre-newsnow state has layered snow
         ! (snl_old < 0), trc_scv must be zero — pre-layer accumulation
         ! stops once a real snow layer exists, and Cases B/C below will
         ! unconditionally reset trc_scv. A non-zero residual here means
         ! somebody else wrote trc_scv while snl_old<0 (a real bug
         ! upstream, not a simple rounding artefact). Flag it once per
	         ! tracer per occurrence so the root cause can be traced; Cases
	         ! B/C now fold any residual into the top snow layer before reset.
         IF (snl_old < 0 .and. abs(trc_scv(itrc, ipatch)) > trc_tiny) THEN
            write(*,'(A,I8,A,I3,A,E12.5)') &
               ' WARNING tracer_newsnow: trc_scv residual under layered snow ipatch=', &
               ipatch, ' itrc=', itrc, ' trc_scv_pre=', trc_scv(itrc, ipatch)
         ENDIF
#endif

         IF (snl_old == 0 .and. snl < 0) THEN
            ! Case A: First snow layer just created from accumulated scv.
            ! Partition the accumulated snow tracer across the new layer's
            ! ice/liquid pools.  Do not put all trc_scv into ice and then add
            ! liquid tracer again; that duplicates tracer mass when the first
            ! layer is born with liquid water.
            total_tracer = trc_scv(itrc, ipatch) + trc_pg_snow_ground(itrc, ipatch)
            j = snl + 1  ! = 0
            ice_water = 0._r8
            liq_water = 0._r8
            IF (present(wice_soisno)) THEN
               IF (size(wice_soisno) > 0) ice_water = max(wice_soisno(1), 0._r8)
            ENDIF
            IF (present(wliq_soisno)) THEN
               IF (size(wliq_soisno) > 0) liq_water = max(wliq_soisno(1), 0._r8)
            ENDIF
            layer_water = ice_water + liq_water
            IF (layer_water <= trc_tiny) layer_water = max(scv, trc_tiny)
            R_layer = total_tracer / layer_water
            ice_tracer = min(max(ice_water * R_layer, 0._r8), max(total_tracer, 0._r8))
            liq_tracer = min(max(liq_water * R_layer, 0._r8), max(total_tracer - ice_tracer, 0._r8))
            IF (ice_water <= trc_tiny .and. liq_water <= trc_tiny) ice_tracer = max(total_tracer, 0._r8)
            trc_wice_soisno(itrc, j, ipatch) = ice_tracer
            trc_wliq_soisno(itrc, j, ipatch) = liq_tracer
            ! Clear trc_scv: all transferred to the snow layer
            trc_scv(itrc, ipatch) = 0._r8

         ELSEIF (snl < 0 .and. snl < snl_old) THEN
            ! Case B: New layer added to existing snow pack (deeper snow).
            ! This is a less common case (snow pack growing a new layer).
            ! R_snow_step is correct ONLY when the new layer's mass came from
            ! fresh snowfall this step. SnowLayersCombineDivide handles
            ! split/combine via dedicated tracer hooks (see file-end
            ! comment), so reaching this branch via re-distribution would
            ! quietly use fresh-snow R on existing-pack water and erase the
            ! aged R signal. A CoLMDEBUG warning fires when the new
            ! layer mass disagrees with pg_snow*dt by more than FP noise
            ! — covers the regression risk if a future split path forgets
            ! to route through the hooks.
            j = snl + 1
#if (defined CoLMDEBUG)
            IF (present(wice_soisno)) THEN
               IF (abs(wice_soisno(1) - max(pg_snow, 0._r8) * deltim) > 1.e-6_r8) THEN
                  write(*,'(A,I8,A,I3,A,E12.5,A,E12.5)') &
                     ' WARNING tracer_newsnow Case B: wice_soisno(1) /= pg_snow*dt ipatch=', &
                     ipatch, ' itrc=', itrc, &
                     ' wice=', wice_soisno(1), &
                     ' pg_snow*dt=', max(pg_snow, 0._r8) * deltim
               ENDIF
            ENDIF
#endif
            IF (present(wice_soisno)) THEN
               trc_wice_soisno(itrc, j, ipatch) = wice_soisno(1) * R_snow_step
            ENDIF
	            IF (present(wliq_soisno)) THEN
	               IF (size(wliq_soisno) > 0) THEN
	                  trc_wliq_soisno(itrc, j, ipatch) = wliq_soisno(1) * R_snow_step
	               ENDIF
	            ENDIF
	            IF (abs(trc_scv(itrc, ipatch)) > trc_tiny) THEN
	               trc_wice_soisno(itrc, j, ipatch) = trc_wice_soisno(itrc, j, ipatch) &
	                  + trc_scv(itrc, ipatch)
	            ENDIF
	            ! trc_scv should be 0 when layers already exist
	            trc_scv(itrc, ipatch) = 0._r8

         ELSEIF (snl < 0 .and. snl == snl_old) THEN
            ! Case C: Snow added to existing top layer (no new node).
            ! Do not route through trc_scv here: once layered snow exists,
            ! the water-side pg_snow increment goes directly into the top
            ! snow ice layer. A previous scv -> d_wice-gated -> clear path
            ! could silently drop ground-snow tracer if tracer_precip's
            ! canopy residual correction and the water increment differed by
            ! more than trc_tiny.
            j = snl + 1
            IF (present(wice_soisno) .and. present(wice_soisno_bef)) THEN
               d_wice = wice_soisno(1) - wice_soisno_bef(1)
	               IF (trc_pg_snow_ground(itrc, ipatch) > trc_tiny) THEN
	                  trc_wice_soisno(itrc, j, ipatch) = trc_wice_soisno(itrc, j, ipatch) &
	                     + trc_pg_snow_ground(itrc, ipatch)
	               ENDIF
#if (defined CoLMDEBUG)
               IF (abs(d_wice - snow_mass_step) > 1.e-6_r8 .and. &
                   trc_pg_snow_ground(itrc, ipatch) > trc_tiny) THEN
                  write(*,'(A,I8,A,I3,A,E12.5,A,E12.5,A,E12.5)') &
                     ' WARNING tracer_newsnow Case C: d_wice /= pg_snow*dt ipatch=', &
                     ipatch, ' itrc=', itrc, ' d_wice=', d_wice, &
                     ' pg_snow*dt=', snow_mass_step, &
                     ' trc_pg_snow_ground=', trc_pg_snow_ground(itrc, ipatch)
	               ENDIF
#endif
	               IF (abs(trc_scv(itrc, ipatch)) > trc_tiny) THEN
	                  trc_wice_soisno(itrc, j, ipatch) = trc_wice_soisno(itrc, j, ipatch) &
	                     + trc_scv(itrc, ipatch)
	               ENDIF
            ENDIF
            ! trc_scv should be 0 when layers exist
            trc_scv(itrc, ipatch) = 0._r8

         ELSE
            ! snl == 0: no snow layer yet. Accumulate in trc_scv until
            ! the first real snow layer is created.
            !
            ! scv sink handling (warm wetland melt, thin-snow melt) is done
            ! AFTER THERMAL in CoLMMAIN, not here. tracer_newsnow only
            ! accumulates and transfers on layer creation.
            !
            ! One exception: warm wetland zeroing scv happens INSIDE newsnow
            ! (before THERMAL), so handle it here.
            trc_scv(itrc, ipatch) = trc_scv(itrc, ipatch) + trc_pg_snow_ground(itrc, ipatch)
            IF (patchtype == 2 .and. scv < trc_tiny) THEN
               ! Warm wetland: newsnow transferred scv → wetwat, scv=0
               trc_wetwat(itrc, ipatch) = trc_wetwat(itrc, ipatch) + trc_scv(itrc, ipatch)
               trc_scv(itrc, ipatch) = 0._r8
            ENDIF
         ENDIF
         IF (snl < 0 .and. present(wliq_soisno)) THEN
            j = snl + 1
            IF (size(wliq_soisno) > 0) THEN
               CALL tracer_equilibrate_dissolved(itrc, max(wliq_soisno(1), 0._r8), &
                  trc_wliq_soisno(itrc, j, ipatch), trc_solid_soisno(itrc, j, ipatch))
            ENDIF
         ENDIF
      ENDDO
   END SUBROUTINE tracer_newsnow

   ! tracer_snow_layer_adj was removed: the prior post-hoc redistribution
   ! (total-tracer / total-water share) homogenised the snow column every
   ! time snowlayerscombine / snowlayersdivide fired, erasing vertical
   ! isotope gradients that the model is otherwise supposed to preserve.
   ! The fix carries trc_wliq / trc_wice / trc_scv through the exact same
   ! per-layer topology as water via the new optional hooks on
   ! MOD_SnowLayersCombineDivide's combine/divide routines.

END MODULE MOD_Tracer_Snow
#endif
