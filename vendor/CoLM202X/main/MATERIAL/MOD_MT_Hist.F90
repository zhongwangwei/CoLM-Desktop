#include <define.h>

#ifdef MaterialTransport
MODULE MOD_MT_Hist

   USE MOD_Precision
   USE MOD_Vars_Global, only: spval
   USE MOD_NetCDFSerial
   USE MOD_HistGridded
#ifdef UNSTRUCTURED
   USE MOD_HistVector
#endif
#ifdef SinglePoint
   USE MOD_HistSingle
#endif
   USE MOD_MT_Defs, only: nmts, tracers, mass_to_delta, mt_tiny, &
      mt_delta_sanity_max, mt_flux_water_min_for_delta, mt_water_min_for_delta, &
      mt_water_min_for_ratio, &
      tracer_uses_delta_diagnostics, mt_uses_generic_water, &
      tracer_concentration_units, tracer_is_nonvolatile_solute, tracer_is_isotope, &
      tracer_aquifer_actual_water, tracer_aquifer_actual_mass
   USE MOD_MT_Vars

   IMPLICIT NONE
   PRIVATE

   ! Everything else in here is internal. Without this the module exported all
   ! twelve write_history_* procedures plus everything it USEs, so any module
   ! could reach past the two entry points into the writers -- the kind of
   ! coupling that makes the history path impossible to change safely.
   !
   ! The list is exactly what is used from outside today:
   !   mt_hist_accumulate  CoLMMAIN, MOD_MT_SpecialPatches
   !   mt_hist_out         MOD_Hist
   !   the rest                MOD_MT_Reactive_Methane_Hist, which writes
   !                           its own variables through the shared writers
   ! A provider needing another writer should be added here deliberately, which
   ! is the point: the compiler now asks the question.
   PUBLIC :: mt_hist_accumulate
   PUBLIC :: mt_hist_out
   PUBLIC :: HistForm
   PUBLIC :: write_history_mt_ratio_2d
   PUBLIC :: write_history_variable_2d
   PUBLIC :: write_history_variable_3d

   character(len=10) :: HistForm = 'Gridded'

CONTAINS

   SUBROUTINE mt_hist_accumulate (ipatch, snl, maxsnl, nl_soil, &
      ldew_rain, ldew_snow, wliq_soisno, wice_soisno, &
      wa, wdsrf, wetwat, scv)
      IMPLICIT NONE
      integer,  intent(in) :: ipatch, snl, maxsnl, nl_soil
      real(r8), intent(in) :: ldew_rain, ldew_snow
      real(r8), intent(in) :: wliq_soisno(snl+1:nl_soil)
      real(r8), intent(in) :: wice_soisno(snl+1:nl_soil)
      ! Post-step pool states paired with the existing canopy/soil/snow args.
      ! These feed four additional per-pool tracer-ratio diagnostics in
      ! MOD_Hist (f_mt_conc_wa / wdsrf / wetwat / scv). Without them the
      ! aquifer, surface water, wetland and pre-layer thin-snow δ values are
      ! only observable in the restart file — which rarely matches the
      ! history output cadence.
      real(r8), intent(in) :: wa, wdsrf, wetwat, scv

      integer :: imt, j, jsnow
      real(r8) :: layer_water, layer_tracer, actual_aquifer_water, actual_aquifer_mass

      IF (nmts <= 0) RETURN

      ! Canopy water
      a_water_ldew(ipatch) = a_water_ldew(ipatch) + (ldew_rain + ldew_snow)
      DO imt = 1, nmts
         IF (.not. mt_uses_generic_water(imt)) CYCLE
         a_trc_ldew_mass(imt, ipatch) = a_trc_ldew_mass(imt, ipatch) &
            + mt_ldew_rain(imt, ipatch) + mt_ldew_snow(imt, ipatch)
      ENDDO

      ! Soil layers (1:nl_soil) — fixed dimension, safe for MPI gather
      DO j = 1, nl_soil
         layer_water = wliq_soisno(j) + wice_soisno(j)
         a_water_soil(j, ipatch) = a_water_soil(j, ipatch) + layer_water
         DO imt = 1, nmts
            IF (.not. mt_uses_generic_water(imt)) CYCLE
            layer_tracer = mt_wliq_soisno(imt, j, ipatch) + mt_wice_soisno(imt, j, ipatch)
            IF (tracer_is_nonvolatile_solute(imt) .and. &
                layer_water <= mt_water_min_for_ratio) THEN
               a_trc_layer_dry_mass(imt, ipatch) = a_trc_layer_dry_mass(imt, ipatch) + layer_tracer
            ELSE
               a_trc_soil_mass(imt, j, ipatch) = a_trc_soil_mass(imt, j, ipatch) + layer_tracer
            ENDIF
         ENDDO
      ENDDO

      ! Snow layers — separate accumulator with fixed dimension abs(maxsnl).
      ! Snow layer index j ranges from snl+1 to 0 (negative indices).
      ! Map to positive index: jsnow = j - maxsnl  (1-based, from top to bottom)
      ! e.g., maxsnl=-5: j=-4 → jsnow=1, j=-3 → jsnow=2, ..., j=0 → jsnow=5
      IF (snl < 0) THEN
         DO j = snl + 1, 0
            jsnow = j - maxsnl  ! positive index (1 to abs(maxsnl))
            a_water_snow(jsnow, ipatch) = a_water_snow(jsnow, ipatch) &
               + wliq_soisno(j) + wice_soisno(j)
            DO imt = 1, nmts
               IF (.not. mt_uses_generic_water(imt)) CYCLE
               layer_water = wliq_soisno(j) + wice_soisno(j)
               layer_tracer = mt_wliq_soisno(imt, j, ipatch) + mt_wice_soisno(imt, j, ipatch)
               IF (tracer_is_nonvolatile_solute(imt) .and. &
                   layer_water <= mt_water_min_for_ratio) THEN
                  a_trc_layer_dry_mass(imt, ipatch) = &
                     a_trc_layer_dry_mass(imt, ipatch) + layer_tracer
               ELSE
                  a_trc_snow_mass(imt, jsnow, ipatch) = &
                     a_trc_snow_mass(imt, jsnow, ipatch) + layer_tracer
               ENDIF
            ENDDO
         ENDDO
      ENDIF

            ! Aquifer: positive storage and signed debt are different diagnostics.
            ! wa<=0 is a wetland/aquifer debt state, not a physical concentration.
            ! Keep mt_wa state synchronized with wa, but filter sub-mm debt
            ! from concentration diagnostics to avoid tiny-denominator deltas.
            IF (wa > 1._r8) a_water_wa(ipatch) = a_water_wa(ipatch) + wa
            IF (wa < -1._r8) a_water_wa_debt(ipatch) = a_water_wa_debt(ipatch) - wa
            actual_aquifer_water = tracer_aquifer_actual_water(wa, mt_aquifer_ref_water(ipatch))
            IF (mt_aquifer_ref_water(ipatch) > 0._r8 .and. &
                actual_aquifer_water > mt_water_min_for_ratio) &
               a_water_aquifer_actual(ipatch) = a_water_aquifer_actual(ipatch) + actual_aquifer_water
            IF (mt_aquifer_ref_water(ipatch) <= 0._r8 .and. wa > 1._r8) &
               a_water_aquifer_actual(ipatch) = a_water_aquifer_actual(ipatch) + wa
         a_water_wdsrf (ipatch) = a_water_wdsrf (ipatch) + wdsrf
         a_water_wetwat(ipatch) = a_water_wetwat(ipatch) + wetwat
         ! CoLM's `scv` is total snow water equivalent even after layered
         ! snow exists, but `mt_scv` only mirrors the pre-layer thin-snow
         ! pool. Once snl<0, tracer mass lives in mt_wliq/mt_wice snow
         ! layers and is diagnosed via f_mt_conc_soisno_*; do not pair
         ! that total scv water with the thin-snow tracer denominator.
         IF (snl == 0 .and. scv > mt_tiny) THEN
            a_water_scv(ipatch) = a_water_scv(ipatch) + scv
         ENDIF
         DO imt = 1, nmts
            IF (.not. mt_uses_generic_water(imt)) CYCLE
            IF (tracer_is_isotope(imt) .and. mt_aquifer_ref_water(ipatch) > 0._r8 .and. &
                actual_aquifer_water > mt_water_min_for_ratio) THEN
               actual_aquifer_mass = tracer_aquifer_actual_mass(mt_wa(imt, ipatch), &
                  mt_aquifer_ref_mass(imt, ipatch))
               a_trc_aquifer_actual_mass(imt, ipatch) = &
                  a_trc_aquifer_actual_mass(imt, ipatch) + actual_aquifer_mass
               IF (wa < -1._r8) a_trc_wa_debt_mass(imt, ipatch) = &
                  a_trc_wa_debt_mass(imt, ipatch) - wa * actual_aquifer_mass / actual_aquifer_water
            ELSE
            IF (tracer_is_isotope(imt) .and. wa > 1._r8) &
               a_trc_aquifer_actual_mass(imt, ipatch) = &
                  a_trc_aquifer_actual_mass(imt, ipatch) + mt_wa(imt, ipatch)
            IF (wa > 1._r8) THEN
               a_trc_wa_mass(imt, ipatch) = a_trc_wa_mass(imt, ipatch) + mt_wa(imt, ipatch)
            ENDIF
               IF (wa < -1._r8) THEN
                  a_trc_wa_debt_mass(imt, ipatch) = a_trc_wa_debt_mass(imt, ipatch) &
                     + max(-mt_wa(imt, ipatch), 0._r8)
               ENDIF
            ENDIF
            a_trc_wdsrf_mass (imt, ipatch) = a_trc_wdsrf_mass (imt, ipatch) + mt_wdsrf (imt, ipatch)
         a_trc_wetwat_mass(imt, ipatch) = a_trc_wetwat_mass(imt, ipatch) + mt_wetwat(imt, ipatch)
         a_trc_surface_residue_mass(imt, ipatch) = a_trc_surface_residue_mass(imt, ipatch) + &
            mt_surface_residue(imt, ipatch)
         a_trc_subsurface_residue_mass(imt, ipatch) = a_trc_subsurface_residue_mass(imt, ipatch) + &
            mt_subsurface_residue(imt, ipatch)
         a_trc_solid_mass(imt, ipatch) = a_trc_solid_mass(imt, ipatch) + &
            mt_canopy_solid(imt, ipatch) + mt_surface_solid(imt, ipatch) + &
            mt_subsurface_solid(imt, ipatch) + mt_waterstorage_solid(imt, ipatch)
         DO j = max(snl + 1, lbound(mt_solid_soisno, 2)), nl_soil
            a_trc_solid_mass(imt, ipatch) = a_trc_solid_mass(imt, ipatch) + &
               mt_solid_soisno(imt, j, ipatch)
         ENDDO
         IF (snl == 0 .and. scv > mt_tiny) THEN
            a_trc_scv_mass(imt, ipatch) = a_trc_scv_mass(imt, ipatch) + mt_scv(imt, ipatch)
         ENDIF
      ENDDO
   END SUBROUTINE mt_hist_accumulate

   SUBROUTINE mt_hist_out (file_hist, itime_in_file, hist_form_in, &
      sumarea, filter, maxsnl, nl_soil, forcing_has_missing_value, forcmask_pch)
   USE MOD_Block
   USE MOD_DataType
   USE MOD_LandPatch, only: numpatch
   USE MOD_SPMD_Task, only: p_is_worker
   USE MOD_Vars_TimeInvariants, only: patchtype, patchmask
   USE MOD_Namelist, only: DEF_hist_vars
   USE MOD_MT_Lifecycle, only: tracer_lifecycle_land_history

   IMPLICIT NONE

   character(len=*), intent(in) :: file_hist
   integer, intent(in) :: itime_in_file
   character(len=*), intent(in) :: hist_form_in
   type(block_data_real8_2d), intent(inout) :: sumarea
   logical, intent(inout) :: filter(:)
   integer, intent(in) :: maxsnl, nl_soil
   logical, intent(in) :: forcing_has_missing_value
   logical, intent(in) :: forcmask_pch(:)
            real(r8), allocatable :: mt_mass_soisno (:,:)
            real(r8), allocatable :: water_soisno    (:,:)
            real(r8), allocatable :: mt_delta_flux  (:)
            real(r8), allocatable :: snowpack_mass_pat(:)
            real(r8), allocatable :: snowpack_water_pat(:)
            real(r8), allocatable :: ones_arr        (:)
            ! Per-patch (pseudo_mass, pseudo_water) pair for the
            ! leaf-water-weighted bulk delta. pseudo_water = leaf
            ! water moles, pseudo_mass = R_b * moles. Feeding these
            ! to write_history_mt_delta_2d makes the gridded
            ! result mass_to_delta(sum(area * R_b * moles),
            ! sum(area * moles)) -- a true leaf-water-weighted mean
            ! that obeys the sum-then-divide invariant. Reusing
            ! mt_delta_flux for this would mix two different units
            ! across the same buffer.
            real(r8), allocatable :: leaf_mass_pat   (:)
            real(r8), allocatable :: leaf_water_pat  (:)
            character(len=64)  :: mt_varname
            character(len=128) :: mt_longname
            character(len=32)  :: mt_ratio_word, mt_ratio_units
            real(r8) :: delta_loc, leaf_R_loc
            integer :: itrc_loc, ip_loc, j_loc, jsnow_loc
#ifdef SinglePoint
            logical :: site_hist_writeback_saved
#endif

      HistForm = hist_form_in

#ifdef SinglePoint
      ! Match the material time writer: bypass the host's shared site cache,
      ! including provider output, then restore the host write-back policy.
      site_hist_writeback_saved = USE_SITE_HistWriteBack
      USE_SITE_HistWriteBack = .false.
#endif

      IF ((p_is_worker) .and. (numpatch > 0)) THEN
         filter(:) = patchtype < 99
         IF (forcing_has_missing_value) filter = filter .and. forcmask_pch
         filter = filter .and. patchmask
      ENDIF
      IF (HistForm == 'Gridded') THEN
         CALL mp2g_hist%get_sumarea (sumarea, filter)
      ENDIF

            IF (nmts > 0) THEN
               IF (p_is_worker) THEN
                  allocate (mt_mass_soisno (maxsnl+1:nl_soil, numpatch))
                  allocate (water_soisno    (maxsnl+1:nl_soil, numpatch))
                  allocate (mt_delta_flux  (numpatch))
                  allocate (snowpack_mass_pat (numpatch))
                  allocate (snowpack_water_pat(numpatch))
                  allocate (ones_arr        (numpatch));  ones_arr = 1._r8
                  allocate (leaf_mass_pat   (numpatch))
                  allocate (leaf_water_pat  (numpatch))
               ELSE
                  allocate (mt_mass_soisno (0,0))
                  allocate (water_soisno    (0,0))
                  allocate (mt_delta_flux  (0))
                  allocate (snowpack_mass_pat (0))
                  allocate (snowpack_water_pat(0))
                  allocate (ones_arr        (0))
                  allocate (leaf_mass_pat   (0))
                  allocate (leaf_water_pat  (0))
               ENDIF

               DO itrc_loc = 1, nmts
                  IF (p_is_worker) THEN
                     IF (numpatch > 0) THEN
                        filter(:) = patchtype < 99
                        IF (forcing_has_missing_value) filter = filter .and. forcmask_pch
                        filter = filter .and. patchmask
                     ENDIF
                  ENDIF
                  IF (.not. mt_uses_generic_water(itrc_loc)) CYCLE
                  ! The pool diagnostics below are named f_mt_conc_* for every
                  ! tracer, but for isotopes they hold a heavy/total ratio, not
                  ! a concentration -- unlike the flux diagnostics, which split
                  ! into f_mt_delta_* and f_mt_conc_* by category. The names
                  ! are kept because renaming them would silently drop these
                  ! fields from every existing DEF_hist_vars list and break
                  ! analysis scripts, so long_name has to carry the distinction:
                  ! say what the ratio is OF, not just that it is one.
                  IF (tracer_uses_delta_diagnostics(itrc_loc)) THEN
                     mt_ratio_word  = 'heavy/total ratio'
                     mt_ratio_units = 'R'
                  ELSE
                     mt_ratio_word  = 'concentration'
                     mt_ratio_units = tracer_concentration_units(itrc_loc)
                  ENDIF

                  IF (tracer_uses_delta_diagnostics(itrc_loc)) THEN
                     write(mt_varname , '(A,A)') 'f_mt_delta_precip_', trim(tracers(itrc_loc)%name)
                     ! a_water_precip also books dew/frost deposition (MOD_MT_Evapo,
                     ! MOD_MT_SoilWater), so say so in the name.
                     write(mt_longname, '(A,A,A)') 'precipitation plus dew/frost deposition material delta (', &
                        trim(tracers(itrc_loc)%name), ')'
                     CALL write_history_mt_delta_2d (DEF_hist_vars%xy_prc .or. DEF_hist_vars%xy_prl, &
                        a_trc_precip(itrc_loc, :), a_water_precip(itrc_loc, :), &
                        tracers(itrc_loc)%ref_ratio, file_hist, trim(mt_varname), &
                        itime_in_file, filter, trim(mt_longname), 'permil', &
                        water_min_override = mt_flux_water_min_for_delta)

                     write(mt_varname , '(A,A)') 'f_mt_delta_runoff_', trim(tracers(itrc_loc)%name)
                     write(mt_longname, '(A,A,A)') 'total runoff material delta (', &
                        trim(tracers(itrc_loc)%name), ')'
                     CALL write_history_mt_delta_2d (DEF_hist_vars%rnof, &
                        a_trc_rnof(itrc_loc, :), a_water_rnof(itrc_loc, :), &
                        tracers(itrc_loc)%ref_ratio, file_hist, trim(mt_varname), &
                        itime_in_file, filter, trim(mt_longname), 'permil', &
                        water_min_override = mt_flux_water_min_for_delta)

                     ! --- Evapotranspiration flux delta ---
                     ! Pair a_trc_evap with gross evaporative water so the
                     ! denominator never goes near zero from dew/frost
                     ! cancellation (a_fevpa is the net flux; not valid as a
                     ! delta denominator). write_history_mt_delta_2d
                     ! maps mass and water separately to the grid and divides
                     ! once at the end -- preserving the
                     ! mass_to_delta(sum(mass), sum(water)) invariant
                     ! documented in MOD_MT_Defs.F90.
                     write(mt_varname , '(A,A)')   'f_mt_delta_evap_', trim(tracers(itrc_loc)%name)
                     write(mt_longname, '(A,A,A)') 'evapotranspiration material delta (', &
                        trim(tracers(itrc_loc)%name), ')'
                        CALL write_history_mt_delta_2d (DEF_hist_vars%fevpa, &
                           a_trc_evap(itrc_loc, :), a_water_evap_gross(itrc_loc, :), &
                           tracers(itrc_loc)%ref_ratio, file_hist, trim(mt_varname), &
                           itime_in_file, filter, trim(mt_longname), 'permil', &
                           water_min_override = mt_flux_water_min_for_delta)

                     write(mt_varname , '(A,A)')   'f_mt_evap_mass_', trim(tracers(itrc_loc)%name)
                     write(mt_longname, '(A,A,A)') 'signed evapotranspiration material mass (', &
                        trim(tracers(itrc_loc)%name), ')'
                     CALL write_history_variable_2d (DEF_hist_vars%fevpa, &
                        a_trc_evap(itrc_loc, :), file_hist, trim(mt_varname), &
                        itime_in_file, sumarea, filter, trim(mt_longname), 'material amount/m2')

                     write(mt_varname , '(A,A)')   'f_mt_delta_soilevap_', trim(tracers(itrc_loc)%name)
                     write(mt_longname, '(A,A,A)') 'soil/surface evaporation material delta (', &
                        trim(tracers(itrc_loc)%name), ')'
                        CALL write_history_mt_delta_2d (DEF_hist_vars%fevpa, &
                           a_trc_soilevap(itrc_loc, :), a_water_soilevap(itrc_loc, :), &
                           tracers(itrc_loc)%ref_ratio, file_hist, trim(mt_varname), &
                           itime_in_file, filter, trim(mt_longname), 'permil', &
                           water_min_override = mt_flux_water_min_for_delta)

                     write(mt_varname , '(A,A)')   'f_mt_delta_canopyevap_', trim(tracers(itrc_loc)%name)
                     write(mt_longname, '(A,A,A)') 'canopy evaporation material delta (', &
                        trim(tracers(itrc_loc)%name), ')'
                        CALL write_history_mt_delta_2d (DEF_hist_vars%fevpa, &
                           a_trc_canopyevap(itrc_loc, :), a_water_canopyevap(itrc_loc, :), &
                           tracers(itrc_loc)%ref_ratio, file_hist, trim(mt_varname), &
                           itime_in_file, filter, trim(mt_longname), 'permil', &
                           water_min_override = mt_flux_water_min_for_delta)

                     write(mt_varname , '(A,A)')   'f_mt_delta_subl_', trim(tracers(itrc_loc)%name)
                     write(mt_longname, '(A,A,A)') 'sublimation material delta (', &
                        trim(tracers(itrc_loc)%name), ')'
                        CALL write_history_mt_delta_2d (DEF_hist_vars%fevpa, &
                           a_trc_subl(itrc_loc, :), a_water_subl(itrc_loc, :), &
                           tracers(itrc_loc)%ref_ratio, file_hist, trim(mt_varname), &
                           itime_in_file, filter, trim(mt_longname), 'permil', &
                           water_min_override = mt_flux_water_min_for_delta)

                     write(mt_varname , '(A,A)')   'f_mt_delta_wetland_evap_', trim(tracers(itrc_loc)%name)
                     write(mt_longname, '(A,A,A)') 'wetland evaporation/sublimation material delta (', &
                        trim(tracers(itrc_loc)%name), ')'
                        CALL write_history_mt_delta_2d (DEF_hist_vars%fevpa, &
                           a_trc_wetland_evap(itrc_loc, :), a_water_wetland_evap(itrc_loc, :), &
                           tracers(itrc_loc)%ref_ratio, file_hist, trim(mt_varname), &
                           itime_in_file, filter, trim(mt_longname), 'permil', &
                           water_min_override = mt_flux_water_min_for_delta)

                     ! --- Transpiration-only flux delta ---
                     ! write_history_mt_delta_2d does sum-then-divide on
                     ! the grid; no per-patch delta needed here.
                     write(mt_varname , '(A,A)')   'f_mt_delta_transp_', trim(tracers(itrc_loc)%name)
                     write(mt_longname, '(A,A,A)') 'transpiration material delta (', &
                        trim(tracers(itrc_loc)%name), ')'
                        CALL write_history_mt_delta_2d (DEF_hist_vars%etr, &
                           a_trc_transp(itrc_loc, :), a_water_transp(itrc_loc, :), &
                           tracers(itrc_loc)%ref_ratio, file_hist, trim(mt_varname), &
                           itime_in_file, filter, trim(mt_longname), 'permil', &
                           water_min_override = mt_flux_water_min_for_delta)

                     ! --- Transpiration source/xylem delta before NSS storage exchange ---
                     write(mt_varname , '(A,A)')   'f_mt_delta_transp_src_', trim(tracers(itrc_loc)%name)
                     write(mt_longname, '(A,A,A)') 'transpiration source/xylem material delta (', &
                        trim(tracers(itrc_loc)%name), ')'
                        CALL write_history_mt_delta_2d (DEF_hist_vars%etr, &
                           a_trc_transp_src(itrc_loc, :), a_water_transp(itrc_loc, :), &
                           tracers(itrc_loc)%ref_ratio, file_hist, trim(mt_varname), &
                           itime_in_file, filter, trim(mt_longname), 'permil', &
                           water_min_override = mt_flux_water_min_for_delta)

                     ! --- Leaf NSS state deltas ---
                  ! e-site: NSS evaporation-site state -- there is no
                  ! conserved per-patch water mass paired with this
                  ! delta (the Peclet number is a mixing factor, not
                  ! a mass), so area-weighted aggregation is the only
                  ! defensible choice. Longname documents that this
                  ! is a state diagnostic, not a mass-weighted bulk
                  ! pool delta.
                  IF (p_is_worker) THEN
                     DO ip_loc = 1, numpatch
                        delta_loc = mt_leaf_delta_e(itrc_loc, ip_loc)
                        IF (delta_loc /= spval .and. &
                            abs(delta_loc) <= mt_delta_sanity_max) THEN
                           mt_delta_flux(ip_loc) = delta_loc
                        ELSE
                           mt_delta_flux(ip_loc) = spval
                        ENDIF
                     ENDDO
                  ENDIF
                  write(mt_varname , '(A,A)')   'f_mt_leaf_delta_e_', trim(tracers(itrc_loc)%name)
                  write(mt_longname, '(A,A,A)') 'leaf evaporation-site NSS delta, area-weighted state (', &
                     trim(tracers(itrc_loc)%name), ')'
                  CALL write_history_variable_2d (DEF_hist_vars%etr, &
                     mt_delta_flux, file_hist, trim(mt_varname), itime_in_file, &
                     sumarea, filter, trim(mt_longname), 'permil', acc_num = ones_arr)

                  ! b-site: bulk leaf water has a matching per-patch
                  ! mass scale (mt_leaf_water_moles, mol/m^2). Build
                  ! pseudo (mass, water) = (R_b * moles, moles) and
                  ! route through write_history_mt_delta_2d so the
                  ! grid result is mass_to_delta(sum(area*R_b*moles),
                  ! sum(area*moles)) -- a leaf-water-weighted bulk
                  ! delta consistent with the sum-then-divide
                  ! invariant. Patches with delta_b == spval, out-of-
                  ! sanity, or zero leaf water are spval'd in mass
                  ! and 0'd in water so the downstream filter at
                  ! water > mt_water_min_for_delta drops them.
                  IF (p_is_worker) THEN
                     DO ip_loc = 1, numpatch
                        delta_loc = mt_leaf_delta_b(itrc_loc, ip_loc)
                        IF (delta_loc /= spval .and. &
                            abs(delta_loc) <= mt_delta_sanity_max .and. &
                            mt_leaf_water_moles(itrc_loc, ip_loc) > 0._r8) THEN
                           leaf_R_loc = (1._r8 + delta_loc * 1.0e-3_r8) * &
                                        tracers(itrc_loc)%ref_ratio
                           leaf_mass_pat (ip_loc) = leaf_R_loc * &
                                                    mt_leaf_water_moles(itrc_loc, ip_loc)
                           leaf_water_pat(ip_loc) = mt_leaf_water_moles(itrc_loc, ip_loc)
                        ELSE
                           leaf_mass_pat (ip_loc) = spval
                           leaf_water_pat(ip_loc) = 0._r8
                        ENDIF
                     ENDDO
                  ENDIF
                  write(mt_varname , '(A,A)')   'f_mt_leaf_delta_b_', trim(tracers(itrc_loc)%name)
                  write(mt_longname, '(A,A,A)') 'bulk leaf-water NSS delta, leaf-water-mole weighted (', &
                     trim(tracers(itrc_loc)%name), ')'
                  CALL write_history_mt_delta_2d (DEF_hist_vars%etr, &
                     leaf_mass_pat, leaf_water_pat, &
                     tracers(itrc_loc)%ref_ratio, file_hist, trim(mt_varname), &
                     itime_in_file, filter, trim(mt_longname), 'permil')
                  ENDIF

                  ! Species-owned tracers (for example CH4) were filtered by
                  ! mt_uses_generic_water above and write history
                  ! through callbacks. Solutes, decaying or not, keep these
                  ! water-pool diagnostics.
                  IF (.not. tracer_uses_delta_diagnostics(itrc_loc)) THEN
                     write(mt_varname , '(A,A)') 'f_mt_conc_precip_', trim(tracers(itrc_loc)%name)
                     ! a_water_precip also books dew/frost deposition, which carries no
                     ! nonvolatile solute: dew-only periods read 0, not missing.
                     IF (tracer_is_nonvolatile_solute(itrc_loc)) THEN
                        write(mt_longname, '(A,A,A)') &
                           'precipitation plus dew/frost deposition material concentration, dew/frost adding water only (', &
                           trim(tracers(itrc_loc)%name), ')'
                     ELSE
                        write(mt_longname, '(A,A,A)') &
                           'precipitation plus dew/frost deposition material concentration (', &
                           trim(tracers(itrc_loc)%name), ')'
                     ENDIF
                     CALL write_history_mt_ratio_2d (DEF_hist_vars%xy_prc .or. DEF_hist_vars%xy_prl, &
                        a_trc_precip(itrc_loc, :), a_water_precip(itrc_loc, :), &
                        file_hist, trim(mt_varname), itime_in_file, filter, &
                        trim(mt_longname), trim(mt_ratio_units))

                     write(mt_varname , '(A,A)') 'f_mt_conc_runoff_', trim(tracers(itrc_loc)%name)
                     write(mt_longname, '(A,A,A)') 'total runoff material concentration (', &
                        trim(tracers(itrc_loc)%name), ')'
                     CALL write_history_mt_ratio_2d (DEF_hist_vars%rnof, &
                        a_trc_rnof(itrc_loc, :), a_water_rnof(itrc_loc, :), &
                        file_hist, trim(mt_varname), itime_in_file, filter, &
                        trim(mt_longname), trim(mt_ratio_units))
                  ENDIF

                  ! Two-way equilibrium exchange with atmospheric vapour.
                  ! Written as a MASS, not a delta: it carries no water
                  ! flux by construction (zero net water, non-zero net
                  ! tracer), so there is nothing to normalise a delta by.
                  ! Signed -- positive means net uptake by the surface.
                  IF (tracer_uses_delta_diagnostics(itrc_loc) .and. &
                      allocated(a_trc_vapor_exchange)) THEN
                     write(mt_varname , '(A,A)') 'f_mt_vapor_exchange_', &
                        trim(tracers(itrc_loc)%name)
                     write(mt_longname, '(A,A,A)') &
                        'zero-water-flux equilibrium vapour exchange, signed (', &
                        trim(tracers(itrc_loc)%name), ')'
                     CALL write_history_variable_2d (DEF_hist_vars%fevpa, &
                        a_trc_vapor_exchange(itrc_loc, :), file_hist, trim(mt_varname), &
                        itime_in_file, sumarea, filter, trim(mt_longname), 'material amount/m2')
                  ENDIF

                  ! Waterless residue is an areal tracer inventory, not a
                  ! concentration. Tie its output to the existing surface-water
                  ! history switch until history namelist gains a dedicated key.
                  IF (tracer_is_nonvolatile_solute(itrc_loc)) THEN
                     write(mt_varname , '(A,A)') 'f_mt_surface_residue_', trim(tracers(itrc_loc)%name)
                     write(mt_longname, '(A,A,A)') 'immobile surface material residue (', &
                        trim(tracers(itrc_loc)%name), ')'
                     CALL write_history_variable_2d (DEF_hist_vars%wdsrf, &
                        a_trc_surface_residue_mass(itrc_loc, :), file_hist, trim(mt_varname), &
                        itime_in_file, sumarea, filter, trim(mt_longname), 'material amount/m2')

                     write(mt_varname , '(A,A)') 'f_mt_subsurface_residue_', trim(tracers(itrc_loc)%name)
                     write(mt_longname, '(A,A,A)') 'immobile subsurface material residue (', &
                        trim(tracers(itrc_loc)%name), ')'
                     CALL write_history_variable_2d (DEF_hist_vars%wa, &
                        a_trc_subsurface_residue_mass(itrc_loc, :), file_hist, trim(mt_varname), &
                        itime_in_file, sumarea, filter, trim(mt_longname), 'material amount/m2')

                     write(mt_varname , '(A,A)') 'f_mt_layer_dry_inventory_', trim(tracers(itrc_loc)%name)
                     write(mt_longname, '(A,A,A)') 'dry snow/soil layer material inventory (', &
                        trim(tracers(itrc_loc)%name), ')'
                     CALL write_history_variable_2d (DEF_hist_vars%wliq_soisno, &
                        a_trc_layer_dry_mass(itrc_loc, :), file_hist, trim(mt_varname), &
                        itime_in_file, sumarea, filter, trim(mt_longname), 'material amount/m2')

                     write(mt_varname , '(A,A)') 'f_mt_solid_inventory_', trim(tracers(itrc_loc)%name)
                     write(mt_longname, '(A,A,A)') 'precipitated solid material inventory (', &
                        trim(tracers(itrc_loc)%name), ')'
                     CALL write_history_variable_2d (DEF_hist_vars%wliq_soisno, &
                        a_trc_solid_mass(itrc_loc, :), file_hist, trim(mt_varname), &
                        itime_in_file, sumarea, filter, trim(mt_longname), 'material amount/m2')
                  ENDIF

                  ! --- Canopy ratio ---
                  write(mt_varname , '(A,A)')   'f_mt_conc_ldew_', trim(tracers(itrc_loc)%name)
                  write(mt_longname, '(5A)') 'canopy material ', trim(mt_ratio_word), &
                     ' (', trim(tracers(itrc_loc)%name), ')'
                  CALL write_history_mt_ratio_2d (DEF_hist_vars%ldew, &
                     a_trc_ldew_mass(itrc_loc, :), a_water_ldew, &
                     file_hist, trim(mt_varname), itime_in_file, &
                     filter, trim(mt_longname), trim(mt_ratio_units))

                  ! --- Soil + snow combined ratio ---
                  ! Snow accumulator is keyed by jsnow = j - maxsnl (positive);
                  ! soil accumulator is keyed by j (1..nl_soil). Map tracer
                  ! mass and water separately, then divide on the history grid.
                  ! This avoids area-fraction dilution when a grid cell has
                  ! multiple patch fractions with different tracer signatures.
                  IF (p_is_worker) THEN
                     mt_mass_soisno = spval
                     water_soisno    = spval
                     DO ip_loc = 1, numpatch
                        DO j_loc = maxsnl+1, 0
                           jsnow_loc = j_loc - maxsnl
                           IF (a_water_snow(jsnow_loc, ip_loc) > mt_tiny) THEN
                              mt_mass_soisno(j_loc, ip_loc) = &
                                 a_trc_snow_mass(itrc_loc, jsnow_loc, ip_loc)
                              water_soisno(j_loc, ip_loc) = a_water_snow(jsnow_loc, ip_loc)
                           ENDIF
                        ENDDO
                        DO j_loc = 1, nl_soil
                           IF (a_water_soil(j_loc, ip_loc) > mt_tiny) THEN
                              mt_mass_soisno(j_loc, ip_loc) = &
                                 a_trc_soil_mass(itrc_loc, j_loc, ip_loc)
                              water_soisno(j_loc, ip_loc) = a_water_soil(j_loc, ip_loc)
                           ENDIF
                        ENDDO
                     ENDDO
                  ENDIF
                  write(mt_varname , '(A,A)')   'f_mt_conc_soisno_', trim(tracers(itrc_loc)%name)
                  write(mt_longname, '(5A)') 'soil/snow layer material ', trim(mt_ratio_word), &
                     ' (', trim(tracers(itrc_loc)%name), ')'
                  CALL write_history_mt_ratio_3d (DEF_hist_vars%wliq_soisno, &
                     mt_mass_soisno, water_soisno, file_hist, trim(mt_varname), itime_in_file, &
                     'soilsnow', maxsnl+1, nl_soil-maxsnl, filter, &
                     trim(mt_longname), trim(mt_ratio_units))

                  ! --- Total snowpack ratio/delta ---
                  ! CoLM's water-side scv is total SWE, while tracer_scv is
                  ! only the pre-layer thin-snow pool. Build a formal snowpack
                  ! diagnostic from the same pieces that own tracer mass:
                  ! layered snow accumulators plus thin-snow scv accumulators.
                  ! This keeps f_mt_conc_scv_* as a thin-snow diagnostic and
                  ! prevents the old thin-tracer / total-SWE denominator mix.
                  IF (p_is_worker) THEN
                     snowpack_mass_pat(:)  = a_trc_scv_mass(itrc_loc, :)
                     snowpack_water_pat(:) = a_water_scv(:)
                     DO ip_loc = 1, numpatch
                        DO jsnow_loc = 1, abs(maxsnl)
                           snowpack_mass_pat(ip_loc) = snowpack_mass_pat(ip_loc) &
                              + a_trc_snow_mass(itrc_loc, jsnow_loc, ip_loc)
                           snowpack_water_pat(ip_loc) = snowpack_water_pat(ip_loc) &
                              + a_water_snow(jsnow_loc, ip_loc)
                        ENDDO
                     ENDDO
                  ENDIF

                  write(mt_varname , '(A,A)')   'f_mt_conc_snowpack_', trim(tracers(itrc_loc)%name)
                  write(mt_longname, '(5A)') 'total snowpack material ', trim(mt_ratio_word), &
                     ' (', trim(tracers(itrc_loc)%name), ')'
                  CALL write_history_mt_ratio_2d (DEF_hist_vars%scv, &
                     snowpack_mass_pat, snowpack_water_pat, &
                     file_hist, trim(mt_varname), itime_in_file, &
                     filter, trim(mt_longname), trim(mt_ratio_units))

                  IF (tracer_uses_delta_diagnostics(itrc_loc)) THEN
                     write(mt_varname , '(A,A)')   'f_mt_delta_snowpack_', trim(tracers(itrc_loc)%name)
                     write(mt_longname, '(A,A,A)') 'total snowpack material delta (', &
                        trim(tracers(itrc_loc)%name), ')'
                     CALL write_history_mt_delta_2d (DEF_hist_vars%scv, &
                        snowpack_mass_pat, snowpack_water_pat, &
                        tracers(itrc_loc)%ref_ratio, file_hist, trim(mt_varname), &
                        itime_in_file, filter, trim(mt_longname), 'permil')
                  ENDIF

                  ! --- Aquifer (wa) ratio ---
                  ! wa can be negative during wetland aquifer-debt episodes;
                  ! the mt_wetland redistribution keeps signed arithmetic.
                  write(mt_varname , '(A,A)')   'f_mt_conc_wa_', trim(tracers(itrc_loc)%name)
                  write(mt_longname, '(5A)') 'aquifer material ', trim(mt_ratio_word), &
                     ' (', trim(tracers(itrc_loc)%name), ')'
                     IF (tracer_is_isotope(itrc_loc)) THEN
                        CALL write_history_mt_ratio_2d (DEF_hist_vars%wa, &
                           a_trc_aquifer_actual_mass(itrc_loc, :), a_water_aquifer_actual, &
                           file_hist, trim(mt_varname), itime_in_file, &
                           filter, trim(mt_longname), trim(mt_ratio_units))
                     ELSE
                        CALL write_history_mt_ratio_2d (DEF_hist_vars%wa, &
                           a_trc_wa_mass(itrc_loc, :), a_water_wa, &
                           file_hist, trim(mt_varname), itime_in_file, &
                           filter, trim(mt_longname), trim(mt_ratio_units))
                     ENDIF

                     write(mt_varname , '(A,A)')   'f_mt_conc_wa_debt_', trim(tracers(itrc_loc)%name)
                     write(mt_longname, '(5A)') 'aquifer debt material ', trim(mt_ratio_word), &
                        ' (', trim(tracers(itrc_loc)%name), ')'
                     CALL write_history_mt_ratio_2d (DEF_hist_vars%wa, &
                        a_trc_wa_debt_mass(itrc_loc, :), a_water_wa_debt, &
                        file_hist, trim(mt_varname), itime_in_file, &
                        filter, trim(mt_longname), trim(mt_ratio_units))

                  ! --- Surface water (wdsrf) ratio ---
                  write(mt_varname , '(A,A)')   'f_mt_conc_wdsrf_', trim(tracers(itrc_loc)%name)
                  write(mt_longname, '(5A)') 'surface water material ', trim(mt_ratio_word), &
                     ' (', trim(tracers(itrc_loc)%name), ')'
                  CALL write_history_mt_ratio_2d (DEF_hist_vars%wdsrf, &
                     a_trc_wdsrf_mass(itrc_loc, :), a_water_wdsrf, &
                     file_hist, trim(mt_varname), itime_in_file, &
                     filter, trim(mt_longname), trim(mt_ratio_units))

                  ! --- Wetland pool (wetwat) ratio ---
                  write(mt_varname , '(A,A)')   'f_mt_conc_wetwat_', trim(tracers(itrc_loc)%name)
                  write(mt_longname, '(5A)') 'wetland pool material ', trim(mt_ratio_word), &
                     ' (', trim(tracers(itrc_loc)%name), ')'
                  IF (p_is_worker) THEN
                     IF (numpatch > 0) THEN
                        filter(:) = patchtype == 2
                        IF (forcing_has_missing_value) THEN
                           filter = filter .and. forcmask_pch
                        ENDIF
                        filter = filter .and. patchmask
                     ENDIF
                  ENDIF
                  CALL write_history_mt_ratio_2d (DEF_hist_vars%wetwat, &
                     a_trc_wetwat_mass(itrc_loc, :), a_water_wetwat, &
                     file_hist, trim(mt_varname), itime_in_file, &
                     filter, trim(mt_longname), trim(mt_ratio_units))

                  IF (p_is_worker) THEN
                     IF (numpatch > 0) THEN
                        filter(:) = patchtype < 99
                        IF (forcing_has_missing_value) filter = filter .and. forcmask_pch
                        filter = filter .and. patchmask
                     ENDIF
                  ENDIF

                  ! --- Pre-layer thin snow (scv) ratio ---
                  ! Only meaningful when snl==0 (accumulated snow below
                  ! layer-creation threshold). CoLM's water-side scv remains
                  ! total SWE after layered snow exists, but tracer mass then
                  ! lives in mt_wliq/mt_wice snow layers; the accumulator
                  ! therefore filters out snl<0 states so this ratio cannot
                  ! divide thin-snow tracer by total layered-snow SWE.
                  write(mt_varname , '(A,A)')   'f_mt_conc_scv_', trim(tracers(itrc_loc)%name)
                  write(mt_longname, '(5A)') 'thin-snow (scv) material ', trim(mt_ratio_word), &
                     ' (', trim(tracers(itrc_loc)%name), ')'
                  CALL write_history_mt_ratio_2d (DEF_hist_vars%scv, &
                     a_trc_scv_mass(itrc_loc, :), a_water_scv, &
                     file_hist, trim(mt_varname), itime_in_file, &
                     filter, trim(mt_longname), trim(mt_ratio_units))

               ENDDO

               deallocate (mt_mass_soisno, water_soisno, mt_delta_flux, &
                           snowpack_mass_pat, snowpack_water_pat, ones_arr, &
                           leaf_mass_pat, leaf_water_pat)
            ENDIF

      CALL tracer_lifecycle_land_history (file_hist, itime_in_file, sumarea, filter, &
         nl_soil, forcing_has_missing_value, forcmask_pch)

#ifdef SinglePoint
      USE_SITE_HistWriteBack = site_hist_writeback_saved
#endif

   END SUBROUTINE mt_hist_out




#ifdef MaterialTransport
   ! Ratio writers (2d, 3d and their vector forms) drop a patch whose
   ! accumulated water is at most mt_water_min_for_delta, as the delta
   ! writers do. With mt_tiny they wrote the ratio of rounding dust: a
   ! NEC January IsoGSM run gave f_mt_conc_wdsrf = 0 (delta -1000) where
   ! the accumulated surface water was 1e-17..1e-14 mm.
   SUBROUTINE write_history_mt_ratio_2d ( is_hist, &
         tracer_mass_acc, water_acc, file_hist, varname, itime_in_file, &
         filter, longname, units)

   USE MOD_Block
   USE MOD_SPMD_Task, only: p_is_worker, p_is_io
#ifdef MaterialTransport
#endif
   USE MOD_Namelist, only: DEF_HIST_CompressLevel

   IMPLICIT NONE

   logical, intent(in) :: is_hist
   real(r8), intent(in) :: tracer_mass_acc(:)
   real(r8), intent(in) :: water_acc(:)
   character(len=*), intent(in) :: file_hist
   character(len=*), intent(in) :: varname
   integer, intent(in) :: itime_in_file
   logical, intent(in) :: filter(:)
   character(len=*), intent(in) :: longname
   character(len=*), intent(in) :: units

   real(r8), allocatable :: tracer_map_vec(:)
   real(r8), allocatable :: water_map_vec(:)
   real(r8), allocatable :: ratio_vec(:)
   type(block_data_real8_2d) :: tracer_xy_2d
   type(block_data_real8_2d) :: water_xy_2d
   type(block_data_real8_2d) :: ratio_xy_2d
   integer :: ip, iblkme, xblk, yblk, xloc, yloc
   integer :: compress

      IF (.not. is_hist) RETURN
#ifdef SinglePoint
      IF (HistForm == 'Single') THEN
         allocate(ratio_vec(size(water_acc)))
         ratio_vec = spval
         DO ip = 1, size(water_acc)
            IF (abs(water_acc(ip)) > mt_water_min_for_delta .and. tracer_mass_acc(ip) /= spval) THEN
               ratio_vec(ip) = tracer_mass_acc(ip) / water_acc(ip)
            ENDIF
         ENDDO
         CALL single_write_2d (ratio_vec, file_hist, varname, itime_in_file, &
            longname, units)
         deallocate(ratio_vec)
         RETURN
      ENDIF
#endif
#ifdef UNSTRUCTURED
      IF (HistForm == 'Vector') THEN
         CALL write_history_mt_ratio_vector_2d (tracer_mass_acc, water_acc, &
            file_hist, varname, itime_in_file, filter, longname, units)
         RETURN
      ENDIF
#endif
      IF (HistForm /= 'Gridded') RETURN

      allocate(tracer_map_vec(size(tracer_mass_acc)))
      allocate(water_map_vec(size(water_acc)))
      tracer_map_vec = 0._r8
      water_map_vec = 0._r8

      IF (p_is_worker) THEN
         DO ip = 1, size(water_acc)
            IF (abs(water_acc(ip)) > mt_water_min_for_delta .and. tracer_mass_acc(ip) /= spval) THEN
               tracer_map_vec(ip) = tracer_mass_acc(ip)
               water_map_vec(ip) = water_acc(ip)
            ENDIF
         ENDDO
      ENDIF

      IF (p_is_io) THEN
         CALL allocate_block_data (ghist, tracer_xy_2d)
         CALL allocate_block_data (ghist, water_xy_2d)
         CALL allocate_block_data (ghist, ratio_xy_2d)
      ENDIF

      CALL mp2g_hist%pset2grid (tracer_map_vec, tracer_xy_2d, spv = spval, msk = filter)
      CALL mp2g_hist%pset2grid (water_map_vec, water_xy_2d, spv = spval, msk = filter)

      IF (p_is_io) THEN
         DO iblkme = 1, gblock%nblkme
            xblk = gblock%xblkme(iblkme)
            yblk = gblock%yblkme(iblkme)

            DO yloc = 1, ghist%ycnt(yblk)
               DO xloc = 1, ghist%xcnt(xblk)
                  IF (water_xy_2d%blk(xblk,yblk)%val(xloc,yloc) /= spval .and. &
                      tracer_xy_2d%blk(xblk,yblk)%val(xloc,yloc) /= spval .and. &
                      abs(water_xy_2d%blk(xblk,yblk)%val(xloc,yloc)) > mt_tiny) THEN
                     ratio_xy_2d%blk(xblk,yblk)%val(xloc,yloc) = &
                        tracer_xy_2d%blk(xblk,yblk)%val(xloc,yloc) &
                        / water_xy_2d%blk(xblk,yblk)%val(xloc,yloc)
                  ELSE
                     ratio_xy_2d%blk(xblk,yblk)%val(xloc,yloc) = spval
                  ENDIF
               ENDDO
            ENDDO
         ENDDO
      ENDIF

      compress = DEF_HIST_CompressLevel
      CALL hist_write_var_real8_2d (file_hist, varname, ghist, itime_in_file, &
         ratio_xy_2d, compress, longname, units)

      deallocate(tracer_map_vec)
      deallocate(water_map_vec)

   END SUBROUTINE write_history_mt_ratio_2d


   SUBROUTINE write_history_mt_ratio_3d ( is_hist, &
         tracer_mass_acc, water_acc, file_hist, varname, itime_in_file, &
         dim1name, lb1, ndim1, filter, longname, units)

   USE MOD_Block
   USE MOD_SPMD_Task, only: p_is_worker, p_is_io
   USE MOD_Vars_1DAccFluxes, only: nac
#ifdef MaterialTransport
#endif
   USE MOD_Namelist, only: DEF_HIST_CompressLevel

   IMPLICIT NONE

   logical, intent(in) :: is_hist
   real(r8), intent(in) :: tracer_mass_acc(:,:)
   real(r8), intent(in) :: water_acc(:,:)
   character(len=*), intent(in) :: file_hist
   character(len=*), intent(in) :: varname
   integer, intent(in) :: itime_in_file
   character(len=*), intent(in) :: dim1name
   integer, intent(in) :: lb1, ndim1
   logical, intent(in) :: filter(:)
   character(len=*), intent(in) :: longname
   character(len=*), intent(in) :: units

   real(r8), allocatable :: tracer_map_vec(:,:)
   real(r8), allocatable :: water_map_vec(:,:)
   type(block_data_real8_3d) :: tracer_xy_3d
   type(block_data_real8_3d) :: water_xy_3d
   type(block_data_real8_3d) :: ratio_xy_3d
   integer :: ip, i1, iblkme, xblk, yblk, xloc, yloc
   integer :: lb_vec, ub_vec
   integer :: compress
   real(r8), allocatable :: ratio_vec(:,:)

      IF (.not. is_hist) RETURN
#ifdef SinglePoint
      IF (HistForm == 'Single') THEN
         lb_vec = lbound(tracer_mass_acc, 1)
         ub_vec = ubound(tracer_mass_acc, 1)
         allocate(ratio_vec(lb_vec:ub_vec, size(water_acc, 2)))
         ratio_vec = spval
         DO ip = 1, size(water_acc, 2)
            DO i1 = lb_vec, ub_vec
               IF (abs(water_acc(i1, ip)) > mt_water_min_for_delta .and. tracer_mass_acc(i1, ip) /= spval) THEN
                  ! single_write_3d divides by nac internally like ordinary
                  ! accumulated variables. Ratio is already a final diagnostic,
                  ! so pre-scale here to leave the written value unchanged.
                  ratio_vec(i1, ip) = tracer_mass_acc(i1, ip) / water_acc(i1, ip) * real(nac, r8)
               ENDIF
            ENDDO
         ENDDO
         CALL single_write_3d (ratio_vec, file_hist, varname, itime_in_file, &
            dim1name, ndim1, longname, units)
         deallocate(ratio_vec)
         RETURN
      ENDIF
#endif
#ifdef UNSTRUCTURED
      IF (HistForm == 'Vector') THEN
         CALL write_history_mt_ratio_vector_3d (tracer_mass_acc, water_acc, &
            file_hist, varname, itime_in_file, dim1name, lb1, ndim1, filter, &
            longname, units)
         RETURN
      ENDIF
#endif
      IF (HistForm /= 'Gridded') RETURN

      lb_vec = lbound(tracer_mass_acc, 1)
      ub_vec = ubound(tracer_mass_acc, 1)
      allocate(tracer_map_vec(lb_vec:ub_vec, size(tracer_mass_acc, 2)))
      allocate(water_map_vec  (lb_vec:ub_vec, size(water_acc, 2)))
      tracer_map_vec = spval
      water_map_vec  = spval

      IF (p_is_worker) THEN
         DO ip = 1, size(water_acc, 2)
            DO i1 = lb_vec, ub_vec
               IF (water_acc(i1, ip) > mt_water_min_for_delta .and. tracer_mass_acc(i1, ip) /= spval) THEN
                  tracer_map_vec(i1, ip) = tracer_mass_acc(i1, ip)
                  water_map_vec  (i1, ip) = water_acc(i1, ip)
               ENDIF
            ENDDO
         ENDDO
      ENDIF

      IF (p_is_io) THEN
         CALL allocate_block_data (ghist, tracer_xy_3d, ndim1, lb1)
         CALL allocate_block_data (ghist, water_xy_3d,   ndim1, lb1)
         CALL allocate_block_data (ghist, ratio_xy_3d,   ndim1, lb1)
      ENDIF

      CALL mp2g_hist%pset2grid (tracer_map_vec, tracer_xy_3d, spv = spval, msk = filter)
      CALL mp2g_hist%pset2grid (water_map_vec,   water_xy_3d, spv = spval, msk = filter)

      IF (p_is_io) THEN
         DO iblkme = 1, gblock%nblkme
            xblk = gblock%xblkme(iblkme)
            yblk = gblock%yblkme(iblkme)

            DO yloc = 1, ghist%ycnt(yblk)
               DO xloc = 1, ghist%xcnt(xblk)
                  DO i1 = ratio_xy_3d%lb1, ratio_xy_3d%ub1
                     IF (water_xy_3d%blk(xblk,yblk)%val(i1,xloc,yloc) /= spval .and. &
                         tracer_xy_3d%blk(xblk,yblk)%val(i1,xloc,yloc) /= spval .and. &
                         water_xy_3d%blk(xblk,yblk)%val(i1,xloc,yloc) > mt_tiny) THEN
                        ratio_xy_3d%blk(xblk,yblk)%val(i1,xloc,yloc) = &
                           tracer_xy_3d%blk(xblk,yblk)%val(i1,xloc,yloc) &
                           / water_xy_3d%blk(xblk,yblk)%val(i1,xloc,yloc)
                     ELSE
                        ratio_xy_3d%blk(xblk,yblk)%val(i1,xloc,yloc) = spval
                     ENDIF
                  ENDDO
               ENDDO
            ENDDO
         ENDDO
      ENDIF

      compress = DEF_HIST_CompressLevel
      CALL hist_write_var_real8_3d (file_hist, varname, dim1name, ghist, &
         itime_in_file, ratio_xy_3d, compress, longname, units)

      deallocate(tracer_map_vec)
      deallocate(water_map_vec)

   END SUBROUTINE write_history_mt_ratio_3d


   SUBROUTINE write_history_mt_delta_2d ( is_hist, &
         tracer_mass_acc, water_acc, ref_ratio, file_hist, varname, itime_in_file, &
         filter, longname, units, water_min_override)

   USE MOD_Block
   USE MOD_SPMD_Task, only: p_is_worker, p_is_io
#ifdef MaterialTransport
#endif
   USE MOD_Namelist, only: DEF_HIST_CompressLevel

   IMPLICIT NONE

   logical, intent(in) :: is_hist
   real(r8), intent(in) :: tracer_mass_acc(:)
   real(r8), intent(in) :: water_acc(:)
   real(r8), intent(in) :: ref_ratio
   character(len=*), intent(in) :: file_hist
   character(len=*), intent(in) :: varname
   integer, intent(in) :: itime_in_file
      logical, intent(in) :: filter(:)
      character(len=*), intent(in) :: longname
      character(len=*), intent(in) :: units
      real(r8), intent(in), optional :: water_min_override

      real(r8), allocatable :: tracer_map_vec(:)
      real(r8), allocatable :: water_map_vec(:)
      real(r8), allocatable :: delta_vec(:)
      type(block_data_real8_2d) :: tracer_xy_2d
      type(block_data_real8_2d) :: water_xy_2d
      type(block_data_real8_2d) :: delta_xy_2d
      integer :: ip, iblkme, xblk, yblk, xloc, yloc
      integer :: compress
      real(r8) :: delta_loc, water_min

      IF (.not. is_hist) RETURN
      water_min = mt_water_min_for_delta
      IF (present(water_min_override)) water_min = water_min_override
#ifdef SinglePoint
      IF (HistForm == 'Single') THEN
         allocate(delta_vec(size(water_acc)))
         delta_vec = spval
         DO ip = 1, size(water_acc)
            IF (water_acc(ip) > water_min .and. tracer_mass_acc(ip) /= spval) THEN
               delta_loc = mass_to_delta(tracer_mass_acc(ip), water_acc(ip), ref_ratio)
               IF (delta_loc /= spval .and. abs(delta_loc) <= mt_delta_sanity_max) THEN
                  delta_vec(ip) = delta_loc
               ENDIF
            ENDIF
         ENDDO
         CALL single_write_2d (delta_vec, file_hist, varname, itime_in_file, &
            longname, units)
         deallocate(delta_vec)
         RETURN
      ENDIF
#endif
#ifdef UNSTRUCTURED
      IF (HistForm == 'Vector') THEN
         CALL write_history_mt_delta_vector_2d (tracer_mass_acc, water_acc, ref_ratio, &
            file_hist, varname, itime_in_file, filter, longname, units, water_min)
         RETURN
      ENDIF
#endif
      IF (HistForm /= 'Gridded') RETURN

      allocate(tracer_map_vec(size(tracer_mass_acc)))
         allocate(water_map_vec(size(water_acc)))
         tracer_map_vec = 0._r8
         water_map_vec = 0._r8

         IF (p_is_worker) THEN
            DO ip = 1, size(water_acc)
               IF (water_acc(ip) > water_min .and. &
                   tracer_mass_acc(ip) /= spval) THEN
               tracer_map_vec(ip) = tracer_mass_acc(ip)
               water_map_vec(ip) = water_acc(ip)
            ENDIF
         ENDDO
      ENDIF

      IF (p_is_io) THEN
         CALL allocate_block_data (ghist, tracer_xy_2d)
         CALL allocate_block_data (ghist, water_xy_2d)
         CALL allocate_block_data (ghist, delta_xy_2d)
      ENDIF

      CALL mp2g_hist%pset2grid (tracer_map_vec, tracer_xy_2d, spv = spval, msk = filter)
      CALL mp2g_hist%pset2grid (water_map_vec, water_xy_2d, spv = spval, msk = filter)

      IF (p_is_io) THEN
         DO iblkme = 1, gblock%nblkme
            xblk = gblock%xblkme(iblkme)
            yblk = gblock%yblkme(iblkme)

            DO yloc = 1, ghist%ycnt(yblk)
               DO xloc = 1, ghist%xcnt(xblk)
                  IF (water_xy_2d%blk(xblk,yblk)%val(xloc,yloc) > mt_tiny) THEN
                     delta_loc = mass_to_delta( &
                        tracer_xy_2d%blk(xblk,yblk)%val(xloc,yloc), &
                        water_xy_2d%blk(xblk,yblk)%val(xloc,yloc), &
                        ref_ratio)
                     IF (delta_loc /= spval .and. abs(delta_loc) <= mt_delta_sanity_max) THEN
                        delta_xy_2d%blk(xblk,yblk)%val(xloc,yloc) = delta_loc
                     ELSE
                        delta_xy_2d%blk(xblk,yblk)%val(xloc,yloc) = spval
                     ENDIF
                  ELSE
                     delta_xy_2d%blk(xblk,yblk)%val(xloc,yloc) = spval
                  ENDIF
               ENDDO
            ENDDO
         ENDDO
      ENDIF

      compress = DEF_HIST_CompressLevel
      CALL hist_write_var_real8_2d (file_hist, varname, ghist, itime_in_file, &
         delta_xy_2d, compress, longname, units)

      deallocate(tracer_map_vec)
      deallocate(water_map_vec)

   END SUBROUTINE write_history_mt_delta_2d

#ifdef UNSTRUCTURED
   SUBROUTINE write_history_mt_ratio_vector_2d (tracer_mass_acc, water_acc, &
      file_hist, varname, itime_in_file, filter, longname, units)

      IMPLICIT NONE
      real(r8), intent(in) :: tracer_mass_acc(:)
      real(r8), intent(in) :: water_acc(:)
      character(len=*), intent(in) :: file_hist
      character(len=*), intent(in) :: varname
      integer, intent(in) :: itime_in_file
      logical, intent(in) :: filter(:)
      character(len=*), intent(in) :: longname
      character(len=*), intent(in) :: units

      CALL write_history_mt_vector_2d (tracer_mass_acc, water_acc, 0._r8, &
         .false., mt_water_min_for_delta, file_hist, varname, itime_in_file, filter, &
         longname, units)
   END SUBROUTINE write_history_mt_ratio_vector_2d

   SUBROUTINE write_history_mt_delta_vector_2d (tracer_mass_acc, water_acc, &
      ref_ratio, file_hist, varname, itime_in_file, filter, longname, units, water_min)

      IMPLICIT NONE
      real(r8), intent(in) :: tracer_mass_acc(:)
      real(r8), intent(in) :: water_acc(:)
      real(r8), intent(in) :: ref_ratio
      character(len=*), intent(in) :: file_hist
      character(len=*), intent(in) :: varname
      integer, intent(in) :: itime_in_file
      logical, intent(in) :: filter(:)
      character(len=*), intent(in) :: longname
      character(len=*), intent(in) :: units
      real(r8), intent(in) :: water_min

      CALL write_history_mt_vector_2d (tracer_mass_acc, water_acc, ref_ratio, &
         .true., water_min, file_hist, varname, itime_in_file, filter, &
         longname, units)
   END SUBROUTINE write_history_mt_delta_vector_2d

   SUBROUTINE write_history_mt_vector_2d (tracer_mass_acc, water_acc, ref_ratio, &
      write_delta, water_min, file_hist, varname, itime_in_file, filter, longname, units)

      USE MOD_SPMD_Task
      USE MOD_Namelist, only: DEF_HIST_CompressLevel
      USE MOD_LandElm
      USE MOD_ElmVector
      USE MOD_LandPatch, only: elm_patch
      IMPLICIT NONE

      real(r8), intent(in) :: tracer_mass_acc(:)
      real(r8), intent(in) :: water_acc(:)
      real(r8), intent(in) :: ref_ratio
      logical, intent(in) :: write_delta
      real(r8), intent(in) :: water_min
      character(len=*), intent(in) :: file_hist
      character(len=*), intent(in) :: varname
      integer, intent(in) :: itime_in_file
      logical, intent(in) :: filter(:)
      character(len=*), intent(in) :: longname
      character(len=*), intent(in) :: units

      integer :: numset, totalnumset, iset, istt, iend, iwork, mesg(2)
      integer :: isrc, ndata, compress
      logical, allocatable :: mask(:)
      real(r8), allocatable :: frac(:)
      real(r8), allocatable :: mt_local(:), water_local(:), out_vec(:)
      real(r8), allocatable :: mt_global(:), water_global(:)
      real(r8), allocatable :: send_pair(:,:), recv_pair(:,:)
      real(r8) :: delta_loc

      IF (p_is_worker) THEN
         numset = numelm
         IF (numset > 0) THEN
            allocate(mt_local(numset), water_local(numset))
            mt_local = 0._r8
            water_local = 0._r8

            DO iset = 1, numset
               istt = elm_patch%substt(iset)
               iend = elm_patch%subend(iset)
               IF ((istt > 0) .and. (iend >= istt)) THEN
                  allocate(mask(istt:iend), frac(istt:iend))
                  IF (write_delta) THEN
                     mask = (water_acc(istt:iend) > water_min) .and. &
                        (tracer_mass_acc(istt:iend) /= spval) .and. filter(istt:iend)
                  ELSE
                     mask = (abs(water_acc(istt:iend)) > water_min) .and. &
                        (tracer_mass_acc(istt:iend) /= spval) .and. filter(istt:iend)
                  ENDIF
                  IF (any(mask)) THEN
                     frac = elm_patch%subfrc(istt:iend)
                     mt_local(iset) = sum(frac * tracer_mass_acc(istt:iend), mask = mask)
                     water_local(iset) = sum(frac * water_acc(istt:iend), mask = mask)
                  ENDIF
                  deallocate(mask, frac)
               ENDIF
            ENDDO
         ENDIF

#ifdef USEMPI
         mesg = (/p_iam_glb, numset/)
         CALL mpi_send (mesg, 2, MPI_INTEGER, p_address_master, mpi_tag_mesg, p_comm_glb, p_err)
         IF (numset > 0) THEN
            allocate(send_pair(2, numset))
            send_pair(1, :) = mt_local
            send_pair(2, :) = water_local
            CALL mpi_send (send_pair, 2*numset, MPI_REAL8, p_address_master, &
               mpi_tag_data, p_comm_glb, p_err)
            deallocate(send_pair)
         ENDIF
#endif
      ENDIF

      IF (p_is_master) THEN
         totalnumset = totalnumelm
         allocate(out_vec(totalnumset), mt_global(totalnumset), water_global(totalnumset))
         out_vec = spval
         mt_global = 0._r8
         water_global = 0._r8

#ifdef USEMPI
         DO iwork = 0, p_np_worker-1
            CALL mpi_recv (mesg, 2, MPI_INTEGER, MPI_ANY_SOURCE, &
               mpi_tag_mesg, p_comm_glb, p_stat, p_err)
            isrc = mesg(1)
            ndata = mesg(2)
            IF (ndata > 0) THEN
               allocate(recv_pair(2, ndata))
               CALL mpi_recv (recv_pair, 2*ndata, MPI_REAL8, isrc, &
                  mpi_tag_data, p_comm_glb, p_stat, p_err)
               mt_global(elm_data_address(p_itis_worker(isrc))%val) = recv_pair(1, :)
               water_global(elm_data_address(p_itis_worker(isrc))%val) = recv_pair(2, :)
               deallocate(recv_pair)
            ENDIF
         ENDDO
#else
         IF (allocated(mt_local)) THEN
            mt_global(elm_data_address(0)%val) = mt_local
            water_global(elm_data_address(0)%val) = water_local
         ENDIF
#endif

         DO iset = 1, totalnumset
            IF (write_delta) THEN
               IF (water_global(iset) > water_min) THEN
                  delta_loc = mass_to_delta(mt_global(iset), water_global(iset), ref_ratio)
                  IF (delta_loc /= spval .and. abs(delta_loc) <= mt_delta_sanity_max) THEN
                     out_vec(iset) = delta_loc
                  ENDIF
               ENDIF
            ELSEIF (abs(water_global(iset)) > water_min) THEN
               out_vec(iset) = mt_global(iset) / water_global(iset)
            ENDIF
         ENDDO

         compress = DEF_HIST_CompressLevel
         IF (itime_in_file >= 1) THEN
            CALL ncio_write_serial_time (file_hist, varname, itime_in_file, out_vec, &
               'element', 'time', compress)
         ELSE
            CALL ncio_write_serial (file_hist, varname, out_vec, 'element', compress)
         ENDIF
         IF (itime_in_file <= 1) THEN
            CALL ncio_put_attr (file_hist, varname, 'long_name', longname)
            CALL ncio_put_attr (file_hist, varname, 'units', units)
            CALL ncio_put_attr (file_hist, varname, 'missing_value', spval)
         ENDIF
      ENDIF

      IF (allocated(out_vec)) deallocate(out_vec)
      IF (allocated(mt_local)) deallocate(mt_local)
      IF (allocated(mt_global)) deallocate(mt_global)
      IF (allocated(water_local)) deallocate(water_local)
      IF (allocated(water_global)) deallocate(water_global)

#ifdef USEMPI
      ! Fixed tags are reused for every history field.  Keep a field-boundary
      ! synchronization so a fast worker cannot send the next field while the
      ! master is still receiving the current field from a slower worker.
      CALL mpi_barrier (p_comm_glb, p_err)
#endif
   END SUBROUTINE write_history_mt_vector_2d

   SUBROUTINE write_history_mt_ratio_vector_3d (tracer_mass_acc, water_acc, &
      file_hist, varname, itime_in_file, dim1name, lb1, ndim1, filter, longname, units)

      USE MOD_SPMD_Task
      USE MOD_Namelist, only: DEF_HIST_CompressLevel
      USE MOD_LandElm
      USE MOD_ElmVector
      USE MOD_LandPatch, only: elm_patch
      IMPLICIT NONE

      real(r8), intent(in) :: tracer_mass_acc(lb1:,:)
      real(r8), intent(in) :: water_acc(lb1:,:)
      character(len=*), intent(in) :: file_hist
      character(len=*), intent(in) :: varname
      integer, intent(in) :: itime_in_file
      character(len=*), intent(in) :: dim1name
      integer, intent(in) :: lb1, ndim1
      logical, intent(in) :: filter(:)
      character(len=*), intent(in) :: longname
      character(len=*), intent(in) :: units

      integer :: numset, totalnumset, iset, istt, iend, iwork, mesg(2)
      integer :: isrc, ndata, compress, i1, k, ub1
      logical, allocatable :: mask(:)
      real(r8), allocatable :: frac(:)
      real(r8), allocatable :: mt_local(:,:), water_local(:,:), out_vec(:,:)
      real(r8), allocatable :: mt_global(:,:), water_global(:,:)
      real(r8), allocatable :: send_pair(:,:,:), recv_pair(:,:,:)

      ub1 = lb1 + ndim1 - 1
      IF (p_is_worker) THEN
         numset = numelm
         IF (numset > 0) THEN
            allocate(mt_local(ndim1, numset), water_local(ndim1, numset))
            mt_local = 0._r8
            water_local = 0._r8
            DO iset = 1, numset
               istt = elm_patch%substt(iset)
               iend = elm_patch%subend(iset)
               IF ((istt > 0) .and. (iend >= istt)) THEN
                  allocate(mask(istt:iend), frac(istt:iend))
                  frac = elm_patch%subfrc(istt:iend)
                  DO i1 = lb1, ub1
                     k = i1 - lb1 + 1
                     mask = (abs(water_acc(i1, istt:iend)) > mt_water_min_for_delta) .and. &
                        (tracer_mass_acc(i1, istt:iend) /= spval) .and. filter(istt:iend)
                     IF (any(mask)) THEN
                        mt_local(k, iset) = sum(frac * tracer_mass_acc(i1, istt:iend), mask = mask)
                        water_local(k, iset) = sum(frac * water_acc(i1, istt:iend), mask = mask)
                     ENDIF
                  ENDDO
                  deallocate(mask, frac)
               ENDIF
            ENDDO
         ENDIF

#ifdef USEMPI
         mesg = (/p_iam_glb, numset/)
         CALL mpi_send (mesg, 2, MPI_INTEGER, p_address_master, mpi_tag_mesg, p_comm_glb, p_err)
         IF (numset > 0) THEN
            allocate(send_pair(2, ndim1, numset))
            send_pair(1, :, :) = mt_local
            send_pair(2, :, :) = water_local
            CALL mpi_send (send_pair, 2*ndim1*numset, MPI_REAL8, p_address_master, &
               mpi_tag_data, p_comm_glb, p_err)
            deallocate(send_pair)
         ENDIF
#endif
      ENDIF

      IF (p_is_master) THEN
         totalnumset = totalnumelm
         allocate(out_vec(ndim1, totalnumset), mt_global(ndim1, totalnumset), &
            water_global(ndim1, totalnumset))
         out_vec = spval
         mt_global = 0._r8
         water_global = 0._r8

#ifdef USEMPI
         DO iwork = 0, p_np_worker-1
            CALL mpi_recv (mesg, 2, MPI_INTEGER, MPI_ANY_SOURCE, &
               mpi_tag_mesg, p_comm_glb, p_stat, p_err)
            isrc = mesg(1)
            ndata = mesg(2)
            IF (ndata > 0) THEN
               allocate(recv_pair(2, ndim1, ndata))
               CALL mpi_recv (recv_pair, 2*ndim1*ndata, MPI_REAL8, isrc, &
                  mpi_tag_data, p_comm_glb, p_stat, p_err)
               DO k = 1, ndim1
                  mt_global(k, elm_data_address(p_itis_worker(isrc))%val) = recv_pair(1, k, :)
                  water_global(k, elm_data_address(p_itis_worker(isrc))%val) = recv_pair(2, k, :)
               ENDDO
               deallocate(recv_pair)
            ENDIF
         ENDDO
#else
         IF (allocated(mt_local)) THEN
            mt_global(:, elm_data_address(0)%val) = mt_local
            water_global(:, elm_data_address(0)%val) = water_local
         ENDIF
#endif

         DO iset = 1, totalnumset
            DO k = 1, ndim1
               IF (abs(water_global(k, iset)) > mt_tiny) THEN
                  out_vec(k, iset) = mt_global(k, iset) / water_global(k, iset)
               ENDIF
            ENDDO
         ENDDO

         CALL ncio_define_dimension (file_hist, dim1name, ndim1)
         compress = DEF_HIST_CompressLevel
         IF (itime_in_file >= 1) THEN
            CALL ncio_write_serial_time (file_hist, varname, itime_in_file, out_vec, &
               dim1name, 'element', 'time', compress)
         ELSE
            CALL ncio_write_serial (file_hist, varname, out_vec, dim1name, 'element', compress)
         ENDIF
         IF (itime_in_file <= 1) THEN
            CALL ncio_put_attr (file_hist, varname, 'long_name', longname)
            CALL ncio_put_attr (file_hist, varname, 'units', units)
            CALL ncio_put_attr (file_hist, varname, 'missing_value', spval)
         ENDIF
      ENDIF

      IF (allocated(out_vec)) deallocate(out_vec)
      IF (allocated(mt_local)) deallocate(mt_local)
      IF (allocated(mt_global)) deallocate(mt_global)
      IF (allocated(water_local)) deallocate(water_local)
      IF (allocated(water_global)) deallocate(water_global)

#ifdef USEMPI
      ! See the 2-D gather above: the trailing barrier separates messages
      ! that intentionally reuse mpi_tag_mesg/mpi_tag_data across fields.
      CALL mpi_barrier (p_comm_glb, p_err)
#endif
   END SUBROUTINE write_history_mt_ratio_vector_3d
#endif
#endif

   SUBROUTINE write_history_variable_2d ( is_hist, &
         acc_vec, file_hist, varname, itime_in_file, sumarea, filter, &
         longname, units, acc_num, input_mode)

   USE MOD_Vars_1DAccFluxes, only: nac

   IMPLICIT NONE

   logical, intent(in) :: is_hist

   real(r8),         intent(inout) :: acc_vec(:)
   character(len=*), intent(in)    :: file_hist
   character(len=*), intent(in)    :: varname
   integer,          intent(in)    :: itime_in_file
   character(len=*), intent(in)    :: longname
   character(len=*), intent(in)    :: units

   type(block_data_real8_2d), intent(in) :: sumarea
   logical, intent(in) :: filter(:)
   real(r8), intent(in), optional  :: acc_num(:)

   character(len=*), intent(in), optional :: input_mode

      IF (.not. is_hist) RETURN

#ifndef SinglePoint
      IF ( .not. present(acc_num) ) THEN
         IF (p_is_worker) &
            WHERE (acc_vec /= spval) acc_vec = acc_vec / nac
      ELSE
         IF (p_is_worker) THEN
            WHERE (acc_vec/=spval .and. acc_num>0)
               acc_vec = acc_vec / acc_num
            ELSEWHERE (acc_vec/=spval .and. acc_num<=0)
               acc_vec = spval
            END WHERE
         ENDIF
      ENDIF
#else
      IF ( .not. present(acc_num) ) THEN
         WHERE (acc_vec /= spval)  acc_vec = acc_vec / nac
      ELSE
         WHERE (acc_vec/=spval .and. acc_num>0)
               acc_vec = acc_vec / acc_num
            ELSEWHERE (acc_vec/=spval .and. acc_num<=0)
               acc_vec = spval
            END WHERE
      ENDIF
#endif

      select CASE (HistForm)
      CASE ('Gridded')
         IF (present(input_mode)) THEN
            CALL flux_map_and_write_2d ( &
               acc_vec, file_hist, varname, itime_in_file, sumarea, filter, longname, units, input_mode)
         ELSE
            CALL flux_map_and_write_2d ( &
               acc_vec, file_hist, varname, itime_in_file, sumarea, filter, longname, units)
         ENDIF
#ifdef UNSTRUCTURED
      CASE ('Vector')
         IF (present(input_mode)) THEN
            CALL aggregate_to_vector_and_write_2d ( &
               acc_vec, file_hist, varname, itime_in_file, filter, longname, units, input_mode)
         ELSE
            CALL aggregate_to_vector_and_write_2d ( &
               acc_vec, file_hist, varname, itime_in_file, filter, longname, units)
         ENDIF
#endif
#ifdef SinglePoint
      CASE ('Single')
         CALL single_write_2d ( &
            acc_vec, file_hist, varname, itime_in_file, longname, units)
#endif
      END select

   END SUBROUTINE write_history_variable_2d


#ifdef URBAN_MODEL
   SUBROUTINE write_history_variable_urb_2d ( is_hist, &
         acc_vec, file_hist, varname, itime_in_file, sumarea, filter, &
         longname, units)

   IMPLICIT NONE

   logical, intent(in) :: is_hist

   real(r8), intent(inout) :: acc_vec(:)
   character(len=*), intent(in) :: file_hist
   character(len=*), intent(in) :: varname
   integer,          intent(in) :: itime_in_file
   character(len=*), intent(in) :: longname
   character(len=*), intent(in) :: units

   type(block_data_real8_2d), intent(in) :: sumarea
   logical, intent(in) :: filter(:)

      IF (.not. is_hist) RETURN

      select CASE (HistForm)
      CASE ('Gridded')
         CALL flux_map_and_write_urb_2d ( &
            acc_vec, file_hist, varname, itime_in_file, sumarea, filter, longname, units)
#ifdef UNSTRUCTURED
      CASE ('Vector')
         !TODO: currently, it is not applicable to urban variables
         CALL aggregate_to_vector_and_write_2d ( &
            acc_vec, file_hist, varname, itime_in_file, filter, longname, units)
#endif
#ifdef SinglePoint
      CASE ('Single')
         CALL single_write_2d ( &
            acc_vec, file_hist, varname, itime_in_file, longname, units)
#endif
      END select

   END SUBROUTINE write_history_variable_urb_2d
#endif


   SUBROUTINE write_history_variable_3d ( is_hist, &
         acc_vec, file_hist, varname, itime_in_file, dim1name, lb1, ndim1, &
         sumarea, filter, longname, units, acc_num)

   USE MOD_Vars_1DAccFluxes, only: nac

   IMPLICIT NONE

   logical, intent(in) :: is_hist

   real(r8), intent(inout) :: acc_vec(:,:)
   character(len=*), intent(in) :: file_hist
   character(len=*), intent(in) :: varname
   integer, intent(in) :: itime_in_file
   character(len=*), intent(in) :: dim1name
   integer, intent(in) :: lb1, ndim1

   type(block_data_real8_2d), intent(in) :: sumarea
   logical, intent(in) :: filter(:)
   character (len=*), intent(in) :: longname
   character (len=*), intent(in) :: units
   real(r8), intent(in), optional :: acc_num(:)

   ! Local variables
   integer :: iblkme, xblk, yblk, xloc, yloc, i1
   integer :: compress
   integer :: j

      IF (.not. is_hist) RETURN

      ! The lower-level 3D writers (flux_map_and_write_3d, aggregate_..., single_...)
      ! always divide acc_vec by `nac`. When a per-patch acc_num counter is supplied
      ! (e.g. methane accumulators that only tick on active patches), pre-scale by
      ! nac/acc_num so the lower-level division yields acc_vec/acc_num.
      IF (present(acc_num)) THEN
#ifndef SinglePoint
         IF (p_is_worker) THEN
            DO j = 1, size(acc_vec, 1)
               WHERE (acc_vec(j,:) /= spval .and. acc_num > 0._r8)
                  acc_vec(j,:) = acc_vec(j,:) * real(nac, r8) / acc_num
               ELSEWHERE (acc_vec(j,:) /= spval .and. acc_num <= 0._r8)
                  acc_vec(j,:) = spval
               END WHERE
            ENDDO
         ENDIF
#else
         DO j = 1, size(acc_vec, 1)
            WHERE (acc_vec(j,:) /= spval .and. acc_num > 0._r8)
               acc_vec(j,:) = acc_vec(j,:) * real(nac, r8) / acc_num
            ELSEWHERE (acc_vec(j,:) /= spval .and. acc_num <= 0._r8)
               acc_vec(j,:) = spval
            END WHERE
         ENDDO
#endif
      ENDIF

      select CASE (HistForm)
      CASE ('Gridded')
         CALL flux_map_and_write_3d ( &
            acc_vec, file_hist, varname, itime_in_file, dim1name, lb1, ndim1, &
            sumarea, filter, longname, units)
#ifdef UNSTRUCTURED
      CASE ('Vector')
         CALL aggregate_to_vector_and_write_3d ( &
            acc_vec, file_hist, varname, itime_in_file, dim1name, lb1, ndim1, &
            filter, longname, units)
#endif
#ifdef SinglePoint
      CASE ('Single')
         CALL single_write_3d (acc_vec, file_hist, varname, itime_in_file, &
            dim1name, ndim1, longname, units)
#endif
      END select

   END SUBROUTINE write_history_variable_3d

END MODULE MOD_MT_Hist
#endif
