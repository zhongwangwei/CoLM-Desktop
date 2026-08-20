#include <define.h>

#ifdef TRACER
MODULE MOD_Tracer_Conservation

   USE MOD_Precision
   USE MOD_Namelist, only: DEF_TRACER_BALANCE_ABORT_NBAD, DEF_TRACER_RESID_ABORT_NBAD
   USE MOD_Tracer_Defs, only: ntracers, trc_tiny, tracer_init_water_ratio, &
      tracer_reactive_decay_fraction, tracer_can_use_fixed_signature, &
      tracer_has_dissolved_limit, tracer_uses_land_water_transport
   USE MOD_Tracer_Frac, only: tracer_fractionation_active
   USE MOD_Tracer_Vars

   IMPLICIT NONE

   ! Hard balance tolerance for the water-corrected per-patch tracer residual.
   ! Keep ordinary land patches strict without using an unrealistically tiny
   ! absolute floor: double-precision budget sums over many storage/flux
   ! components can leave bounded roundoff residuals even when accounting is
   ! closed.  This floor is intentionally below the old 5e-5 value so stale-slot
   ! or missing-flux errors are not hidden by a broad absolute tolerance.
   ! Urban simple patches use the same threshold as ordinary land patches;
   ! their impermeable qgtop<0 evaporation branch is mirrored explicitly in
   ! MOD_Tracer_SoilWater instead of hidden behind a local tolerance waiver.
   real(r8), parameter :: trc_balance_abs_tol = 1.0e-12_r8
   real(r8), parameter :: trc_balance_rel_tol = 1.0e-10_r8
   integer, parameter :: n_storage_diag = 12
   integer, parameter :: n_flux_diag = 7

   ! Per-step accumulator snapshots (saved at start of each timestep)
   real(r8), allocatable, save :: snap_precip(:,:)  ! (ntracers, numpatch)
   real(r8), allocatable, save :: snap_vapor_exchange(:,:)
   real(r8), allocatable, save :: snap_evap  (:,:)
   real(r8), allocatable, save :: snap_rnof  (:,:)
   real(r8), allocatable, save :: snap_rsur  (:,:)
   real(r8), allocatable, save :: snap_rsub  (:,:)
   real(r8), allocatable, save :: snap_qinfl (:,:)
   real(r8), allocatable, save :: snap_qcharge(:,:)
   real(r8), allocatable, save :: snap_storage_comp(:,:,:) ! (component, tracer, patch)

   ! Worker-local worst-patch tracker for tracer_balance_check. Previously
   ! the routine only printed when `ipatch <= 1 .and. itrc == 1`, so every
   ! other failing patch was invisible. These counters accumulate across
   ! patches within a step; caller drains them via tracer_balance_report
   ! at step end.
   real(r8), save :: balance_worst_err   = 0._r8
   real(r8), save :: balance_worst_diag(19) = 0._r8
   real(r8), save :: balance_worst_sbeg(n_storage_diag) = 0._r8
   real(r8), save :: balance_worst_send(n_storage_diag) = 0._r8
   real(r8), save :: balance_worst_sds (n_storage_diag) = 0._r8
   real(r8), save :: balance_worst_fcomp(n_flux_diag) = 0._r8
   integer,  save :: balance_worst_ipatch = 0
   integer,  save :: balance_worst_itrc   = 0
   integer,  save :: balance_worst_ptype  = -1
   integer,  save :: balance_nbad         = 0
   ! Fixed-signature tracers have a second, independent contract: each water-
   ! carrying boundary flux must equal the matching water flux times R_init.
   ! Keep this separate from mass conservation so neither check can hide the
   ! other by substituting theoretical fluxes for the amounts actually booked.
   real(r8), save :: signature_worst_abs      = 0._r8
   real(r8), save :: signature_worst_tol      = 0._r8
   real(r8), save :: signature_worst_terms(3) = 0._r8 ! input, evap, runoff
   integer,  save :: signature_worst_ipatch = 0
   integer,  save :: signature_worst_itrc   = 0
   integer,  save :: signature_worst_ptype  = -1
   integer,  save :: signature_nbad         = 0
   ! P1 diagnostic: the per-layer trc_wliq<->wliq_soisno reconciliation
   ! (MOD_Tracer_SoilWater) injects/removes tracer booked as numerical_source_sink.
   ! This is an explicit solver-closure term, not an unbooked transport leak, so
   ! the hard transport-balance check subtracts it like reactive_source_sink.
   ! It still has its own independent hard cap below; otherwise a large
   ! near-dry collapse could be "balanced" by the residual account forever.
   real(r8), parameter :: trc_resid_warn_frac = 1.0e-6_r8
   real(r8), parameter :: trc_resid_abs_tol   = 5.0e-5_r8
   real(r8), parameter :: trc_resid_rel_tol   = 1.0e-6_r8
   real(r8), save :: resid_worst_abs      = 0._r8
   real(r8), save :: resid_hard_worst_abs = 0._r8
   real(r8), save :: resid_hard_worst_tol = 0._r8
   integer,  save :: resid_hard_worst_ipatch = 0
   integer,  save :: resid_hard_worst_itrc   = 0
   integer,  save :: resid_hard_worst_ptype  = -1
   integer,  save :: resid_nbad           = 0
   integer,  save :: resid_hard_nbad      = 0

   PUBLIC :: deallocate_tracer_conservation
   PUBLIC :: tracer_apply_reactive_processes
   PUBLIC :: tracer_balance_report

CONTAINS

   !---------------------------------------------------------------
   ! Release the save-level snap_* snapshots. Must be called from
   ! land_tracer_final so that a subsequent land_tracer_init on the same
   ! process (regression harness, embedded driver) does not re-use
   ! stale arrays sized to the previous numpatch — the lazy
   ! `IF (.not. allocated) allocate` guard in tracer_save_storage
   ! only fires on the first call ever, so without this deallocate
   ! a changed numpatch would either truncate or overrun snap_*.
   !---------------------------------------------------------------
   SUBROUTINE deallocate_tracer_conservation ()
      IMPLICIT NONE
      IF (allocated(snap_precip)) deallocate(snap_precip)
      IF (allocated(snap_vapor_exchange)) deallocate(snap_vapor_exchange)
      IF (allocated(snap_evap  )) deallocate(snap_evap  )
      IF (allocated(snap_rnof  )) deallocate(snap_rnof  )
      IF (allocated(snap_rsur  )) deallocate(snap_rsur  )
      IF (allocated(snap_rsub  )) deallocate(snap_rsub  )
      IF (allocated(snap_qinfl )) deallocate(snap_qinfl )
      IF (allocated(snap_qcharge)) deallocate(snap_qcharge)
      IF (allocated(snap_storage_comp)) deallocate(snap_storage_comp)
   END SUBROUTINE deallocate_tracer_conservation

   SUBROUTINE tracer_save_storage (ipatch, snl, nl_soil, waterstorage)
      IMPLICIT NONE
      integer, intent(in) :: ipatch, snl, nl_soil
      ! Optional: patch-level irrigation reservoir (mm of water equivalent).
      ! Present only when DEF_USE_IRRIGATION is on under CROP. When passed,
      ! trc_waterstorage is re-synced to waterstorage*R_init (Phase-1
      ! invariant) and included in the storage balance; this mirrors the
      ! water-side convention at CoLMMAIN:806 that treats waterstorage as
      ! part of the column inventory (so irrigation becomes an internal
      ! transfer instead of an external atmospheric input).
      real(r8), intent(in), optional :: waterstorage
         integer :: itrc, j, lb_store
      real(r8) :: R_init
      real(r8) :: storage_comp(n_storage_diag)
      logical  :: fixed_signature_storage

         IF (ntracers <= 0) RETURN
         lb_store = max(lbound(trc_wliq_soisno, 2), snl + 1)

      ! Allocate snapshots on first call
      IF (.not. allocated(snap_precip)) THEN
         allocate(snap_precip(ntracers, size(trc_storage_beg,2))); snap_precip = 0._r8
         allocate(snap_vapor_exchange(ntracers, size(trc_storage_beg,2)))
         snap_vapor_exchange = 0._r8
         allocate(snap_evap  (ntracers, size(trc_storage_beg,2))); snap_evap   = 0._r8
         allocate(snap_rnof  (ntracers, size(trc_storage_beg,2))); snap_rnof   = 0._r8
         allocate(snap_rsur  (ntracers, size(trc_storage_beg,2))); snap_rsur   = 0._r8
         allocate(snap_rsub  (ntracers, size(trc_storage_beg,2))); snap_rsub   = 0._r8
         allocate(snap_qinfl (ntracers, size(trc_storage_beg,2))); snap_qinfl  = 0._r8
         allocate(snap_qcharge(ntracers, size(trc_storage_beg,2))); snap_qcharge = 0._r8
         allocate(snap_storage_comp(n_storage_diag, ntracers, size(trc_storage_beg,2)))
         snap_storage_comp = 0._r8
      ENDIF

      DO itrc = 1, ntracers
         IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
            trc_rnof_step(itrc, ipatch) = 0._r8
            IF (allocated(trc_reactive_source_step)) trc_reactive_source_step(itrc, ipatch) = 0._r8
            IF (allocated(trc_numerical_residual_step)) trc_numerical_residual_step(itrc, ipatch) = 0._r8

         ! Phase-1 re-sync of the irrigation reservoir tracer to current
         ! waterstorage. Under no-fractionation tests all refills arrive at
         ! R_init so the ratio is constant. When fractionation is active, do
         ! not overwrite the reservoir isotope state with the initial delta.
         fixed_signature_storage = tracer_can_use_fixed_signature(itrc) .and. &
            .not. tracer_fractionation_active(itrc) .and. &
            .not. tracer_has_dissolved_limit(itrc)
         IF (allocated(trc_runtime_forced)) THEN
            fixed_signature_storage = fixed_signature_storage .and. .not. trc_runtime_forced(itrc)
         ENDIF
         IF (present(waterstorage) .and. allocated(trc_waterstorage) .and. &
             fixed_signature_storage) THEN
            R_init = tracer_init_water_ratio(itrc)
            trc_waterstorage(itrc, ipatch) = max(waterstorage, 0._r8) * R_init
         ENDIF

         ! Save current storage split by component:
         ! 1 ldew, 2 active snow/soil liquid, 3 active snow/soil ice,
         ! 4 wa, 5 wdsrf, 6 wetwat, 7 scv, 8 waterstorage,
         ! 9 leaf NSS isostorage.  Use the active lower bound (snl+1)
         ! so stale inactive snow slots left by snow layer topology changes
         ! are not double-counted after they have been merged into soil.
         storage_comp = 0._r8
         storage_comp(1) = trc_ldew_rain(itrc, ipatch) + trc_ldew_snow(itrc, ipatch)
            DO j = lb_store, nl_soil
               storage_comp(2) = storage_comp(2) + trc_wliq_soisno(itrc, j, ipatch)
               storage_comp(3) = storage_comp(3) + trc_wice_soisno(itrc, j, ipatch)
            ENDDO
         storage_comp(4) = trc_wa(itrc, ipatch)
         storage_comp(5) = trc_wdsrf(itrc, ipatch)
         storage_comp(6) = trc_wetwat(itrc, ipatch)
         storage_comp(7) = trc_scv(itrc, ipatch)
         IF (allocated(trc_waterstorage)) THEN
            storage_comp(8) = trc_waterstorage(itrc, ipatch)
         ENDIF
         IF (allocated(trc_leaf_iso_storage)) THEN
            storage_comp(9) = trc_leaf_iso_storage(itrc, ipatch)
         ENDIF
         IF (allocated(trc_surface_residue)) THEN
            storage_comp(10) = trc_surface_residue(itrc, ipatch)
         ENDIF
         IF (allocated(trc_subsurface_residue)) THEN
            storage_comp(11) = trc_subsurface_residue(itrc, ipatch)
         ENDIF
         IF (allocated(trc_solid_soisno)) THEN
            DO j = lb_store, nl_soil
               storage_comp(12) = storage_comp(12) + trc_solid_soisno(itrc, j, ipatch)
            ENDDO
         ENDIF
         IF (allocated(trc_canopy_solid)) &
            storage_comp(12) = storage_comp(12) + trc_canopy_solid(itrc, ipatch)
         IF (allocated(trc_surface_solid)) &
            storage_comp(12) = storage_comp(12) + trc_surface_solid(itrc, ipatch)
         IF (allocated(trc_subsurface_solid)) &
            storage_comp(12) = storage_comp(12) + trc_subsurface_solid(itrc, ipatch)
         IF (allocated(trc_waterstorage_solid)) &
            storage_comp(12) = storage_comp(12) + trc_waterstorage_solid(itrc, ipatch)
         snap_storage_comp(:, itrc, ipatch) = storage_comp
         trc_storage_beg(itrc, ipatch) = sum(storage_comp)

         ! Snapshot accumulators at start of this step
         snap_precip(itrc, ipatch) = a_trc_precip(itrc, ipatch)
         IF (allocated(a_trc_vapor_exchange)) &
            snap_vapor_exchange(itrc, ipatch) = a_trc_vapor_exchange(itrc, ipatch)
         snap_evap  (itrc, ipatch) = a_trc_evap  (itrc, ipatch)
         snap_rnof  (itrc, ipatch) = a_trc_rnof  (itrc, ipatch)
         snap_rsur  (itrc, ipatch) = a_trc_rsur  (itrc, ipatch)
         snap_rsub  (itrc, ipatch) = a_trc_rsub  (itrc, ipatch)
         snap_qinfl (itrc, ipatch) = a_trc_qinfl (itrc, ipatch)
         snap_qcharge(itrc, ipatch) = a_trc_qcharge(itrc, ipatch)
      ENDDO
   END SUBROUTINE tracer_save_storage

   ! Contract: this routine applies only generic land-water reactive pools
   ! that live in MOD_Tracer_Vars and share the ordinary water-transport
   ! storage/flux budget.  species-owned reactive state (for example CH4's
   ! conc_methane* prognostic pools) must keep its own source/sink accounting
   ! and restart callbacks; it must not be double-booked here through
   ! trc_reactive_source_step.
   SUBROUTINE tracer_apply_reactive_processes (ipatch, snl, nl_soil, deltim)
      IMPLICIT NONE
      integer,  intent(in) :: ipatch, snl, nl_soil
      real(r8), intent(in) :: deltim
         integer :: itrc, j, lb_store
      real(r8) :: decay_fraction, source_sink

         IF (ntracers <= 0) RETURN
         IF (.not. allocated(trc_reactive_source_step)) RETURN
         lb_store = max(lbound(trc_wliq_soisno, 2), snl + 1)

      DO itrc = 1, ntracers
         IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
         decay_fraction = tracer_reactive_decay_fraction(itrc, deltim)
         IF (decay_fraction <= 0._r8) CYCLE

         source_sink = 0._r8
         CALL decay_pool(trc_ldew_rain(itrc, ipatch), decay_fraction, source_sink)
         CALL decay_pool(trc_ldew_snow(itrc, ipatch), decay_fraction, source_sink)
            DO j = lb_store, nl_soil
               CALL decay_pool(trc_wliq_soisno(itrc, j, ipatch), decay_fraction, source_sink)
               CALL decay_pool(trc_wice_soisno(itrc, j, ipatch), decay_fraction, source_sink)
            ENDDO
         CALL decay_pool(trc_wa(itrc, ipatch), decay_fraction, source_sink)
         CALL decay_pool(trc_wdsrf(itrc, ipatch), decay_fraction, source_sink)
         CALL decay_pool(trc_wetwat(itrc, ipatch), decay_fraction, source_sink)
         CALL decay_pool(trc_scv(itrc, ipatch), decay_fraction, source_sink)
         IF (allocated(trc_waterstorage)) THEN
            CALL decay_pool(trc_waterstorage(itrc, ipatch), decay_fraction, source_sink)
         ENDIF
         IF (allocated(trc_leaf_iso_storage)) THEN
            CALL decay_pool(trc_leaf_iso_storage(itrc, ipatch), decay_fraction, source_sink)
         ENDIF
         IF (allocated(trc_surface_residue)) THEN
            CALL decay_pool(trc_surface_residue(itrc, ipatch), decay_fraction, source_sink)
         ENDIF
         IF (allocated(trc_subsurface_residue)) THEN
            CALL decay_pool(trc_subsurface_residue(itrc, ipatch), decay_fraction, source_sink)
         ENDIF
         IF (allocated(trc_solid_soisno)) THEN
            DO j = lb_store, nl_soil
               CALL decay_pool(trc_solid_soisno(itrc, j, ipatch), decay_fraction, source_sink)
            ENDDO
         ENDIF
         IF (allocated(trc_canopy_solid)) &
            CALL decay_pool(trc_canopy_solid(itrc, ipatch), decay_fraction, source_sink)
         IF (allocated(trc_surface_solid)) &
            CALL decay_pool(trc_surface_solid(itrc, ipatch), decay_fraction, source_sink)
         IF (allocated(trc_subsurface_solid)) &
            CALL decay_pool(trc_subsurface_solid(itrc, ipatch), decay_fraction, source_sink)
         IF (allocated(trc_waterstorage_solid)) &
            CALL decay_pool(trc_waterstorage_solid(itrc, ipatch), decay_fraction, source_sink)

         trc_reactive_source_step(itrc, ipatch) = trc_reactive_source_step(itrc, ipatch) &
            + source_sink
      ENDDO

   CONTAINS

      SUBROUTINE decay_pool (pool, fraction, source_sink)
         real(r8), intent(inout) :: pool
         real(r8), intent(in)    :: fraction
         real(r8), intent(inout) :: source_sink
         real(r8) :: before

         ! Negative tracer pools can occur only in signed water-debt states
         ! such as wa. Do not turn those numerical debts into chemical source.
         IF (pool <= trc_tiny) RETURN
         before = pool
         pool = pool * (1._r8 - fraction)
         source_sink = source_sink + pool - before
      END SUBROUTINE decay_pool

   END SUBROUTINE tracer_apply_reactive_processes

   SUBROUTINE tracer_balance_check (ipatch, snl, nl_soil, deltim, xerr_tracer, &
                                    patchtype_in, water_err_in, water_dS_in, &
                                    water_input_in, water_output_in, &
                                    water_evap_in, water_rnof_in)
      IMPLICIT NONE
      integer,  intent(in)  :: ipatch, snl, nl_soil
      real(r8), intent(in)  :: deltim
      real(r8), intent(out) :: xerr_tracer
      ! Optional patchtype passthrough so TRC_BAL report can identify the
      ! offending branch (0=soil, 1=urban, 2=wetland, 3=glacier, 4=waterbody).
      ! Kept optional so old call sites (if any) still compile.
      integer,  intent(in), optional :: patchtype_in
      real(r8), intent(in), optional :: water_err_in
      real(r8), intent(in), optional :: water_dS_in
      real(r8), intent(in), optional :: water_input_in
      real(r8), intent(in), optional :: water_output_in
      real(r8), intent(in), optional :: water_evap_in
      real(r8), intent(in), optional :: water_rnof_in

      integer  :: itrc, j, lb_store
      real(r8) :: storage_end, step_input, step_evap, step_rnof, step_output, err
      real(r8) :: step_input_check, step_output_check
      real(r8) :: step_rsur, step_rsub, step_qinfl, step_qcharge
      real(r8) :: step_vapor_exchange
      real(r8) :: R_init, water_err, water_err_R, err_minus_water, check_err
      real(r8) :: reactive_source_sink, numerical_source_sink
      real(r8) :: water_dS, water_input, water_output, water_evap, water_rnof
      real(r8) :: dS_minus_water_R, in_minus_water_R, out_minus_water_R
      real(r8) :: evap_minus_water_R, rnof_minus_water_R
      real(r8) :: balance_scale, balance_tol, resid_scale, resid_tol
      real(r8) :: signature_error, signature_scale, signature_tol
      real(r8) :: storage_comp_end(n_storage_diag)
      real(r8) :: storage_comp_beg(n_storage_diag)
      real(r8) :: storage_comp_dS (n_storage_diag)
      logical  :: fixed_signature_step, water_corrected_check

      xerr_tracer = 0._r8
      IF (ntracers <= 0) RETURN
      lb_store = max(lbound(trc_wliq_soisno, 2), snl + 1)

      DO itrc = 1, ntracers
         IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
         ! Current storage split by component; keep this in the same
         ! component order as tracer_save_storage.
         storage_comp_end = 0._r8
         storage_comp_end(1) = trc_ldew_rain(itrc, ipatch) + trc_ldew_snow(itrc, ipatch)
         DO j = lb_store, nl_soil
            storage_comp_end(2) = storage_comp_end(2) + trc_wliq_soisno(itrc, j, ipatch)
            storage_comp_end(3) = storage_comp_end(3) + trc_wice_soisno(itrc, j, ipatch)
         ENDDO
         storage_comp_end(4) = trc_wa(itrc, ipatch)
         storage_comp_end(5) = trc_wdsrf(itrc, ipatch)
         storage_comp_end(6) = trc_wetwat(itrc, ipatch)
         storage_comp_end(7) = trc_scv(itrc, ipatch)
         IF (allocated(trc_waterstorage)) THEN
            storage_comp_end(8) = trc_waterstorage(itrc, ipatch)
         ENDIF
         IF (allocated(trc_leaf_iso_storage)) THEN
            storage_comp_end(9) = trc_leaf_iso_storage(itrc, ipatch)
         ENDIF
         IF (allocated(trc_surface_residue)) THEN
            storage_comp_end(10) = trc_surface_residue(itrc, ipatch)
         ENDIF
         IF (allocated(trc_subsurface_residue)) THEN
            storage_comp_end(11) = trc_subsurface_residue(itrc, ipatch)
         ENDIF
         IF (allocated(trc_solid_soisno)) THEN
            DO j = lb_store, nl_soil
               storage_comp_end(12) = storage_comp_end(12) + trc_solid_soisno(itrc, j, ipatch)
            ENDDO
         ENDIF
         IF (allocated(trc_canopy_solid)) &
            storage_comp_end(12) = storage_comp_end(12) + trc_canopy_solid(itrc, ipatch)
         IF (allocated(trc_surface_solid)) &
            storage_comp_end(12) = storage_comp_end(12) + trc_surface_solid(itrc, ipatch)
         IF (allocated(trc_subsurface_solid)) &
            storage_comp_end(12) = storage_comp_end(12) + trc_subsurface_solid(itrc, ipatch)
         IF (allocated(trc_waterstorage_solid)) &
            storage_comp_end(12) = storage_comp_end(12) + trc_waterstorage_solid(itrc, ipatch)
         storage_end = sum(storage_comp_end)
         storage_comp_beg = snap_storage_comp(:, itrc, ipatch)
         storage_comp_dS = storage_comp_end - storage_comp_beg

         R_init = tracer_init_water_ratio(itrc)
         water_corrected_check = tracer_can_use_fixed_signature(itrc)
         fixed_signature_step = water_corrected_check .and. .not. tracer_fractionation_active(itrc)
         IF (allocated(trc_runtime_forced)) THEN
            fixed_signature_step = fixed_signature_step .and. .not. trc_runtime_forced(itrc)
         ENDIF

         ! Per-step fluxes = current accumulator - snapshot at step start.
         ! Conservation always uses the amounts actually booked. Fixed-
         ! signature agreement with water_flux*R_init is checked independently
         ! below; substituting theoretical values here would hide accounting
         ! errors in precipitation or evaporation.
         step_input  = a_trc_precip(itrc, ipatch) - snap_precip(itrc, ipatch)
         step_evap   = a_trc_evap(itrc, ipatch) - snap_evap(itrc, ipatch)
         step_rsur   = a_trc_rsur(itrc, ipatch) - snap_rsur(itrc, ipatch)
         step_rsub   = a_trc_rsub(itrc, ipatch) - snap_rsub(itrc, ipatch)
         step_qinfl  = a_trc_qinfl(itrc, ipatch) - snap_qinfl(itrc, ipatch)
         step_qcharge = a_trc_qcharge(itrc, ipatch) - snap_qcharge(itrc, ipatch)
         step_rnof   = 0._r8
         step_output = step_evap
#ifndef CatchLateralFlow
         step_rnof   = a_trc_rnof(itrc, ipatch) - snap_rnof(itrc, ipatch)
         step_output = step_output + step_rnof
#endif
         step_input_check = step_input
         step_output_check = step_output

         ! Two-way equilibrium exchange with atmospheric vapour moves tracer
         ! across the patch boundary with NO accompanying water flux, so it
         ! is included in mass conservation but excluded from the separate
         ! fixed-signature check. Signed: positive = net uptake.
         step_vapor_exchange = 0._r8
         IF (allocated(a_trc_vapor_exchange)) THEN
            step_vapor_exchange = a_trc_vapor_exchange(itrc, ipatch) &
                                - snap_vapor_exchange(itrc, ipatch)
         ENDIF
         step_input_check = step_input_check + step_vapor_exchange

         ! Conservation: Δstorage = input - output + source_sink.
         ! Reactive and numerical source/sink terms are explicit state mutations
         ! booked by their owning processes.  Subtract both from the hard check
         ! so solver-closure residuals are reported separately instead of being
         ! misclassified as unbooked transport leaks.
         reactive_source_sink = 0._r8
         IF (allocated(trc_reactive_source_step)) THEN
            reactive_source_sink = trc_reactive_source_step(itrc, ipatch)
         ENDIF
         numerical_source_sink = 0._r8
         IF (allocated(trc_numerical_residual_step)) THEN
            numerical_source_sink = trc_numerical_residual_step(itrc, ipatch)
         ENDIF
         err = storage_end - trc_storage_beg(itrc, ipatch) - step_input_check + step_output_check
         trc_balance_err(itrc, ipatch) = err - reactive_source_sink - numerical_source_sink
         IF (present(water_err_in)) THEN
            water_err = water_err_in
         ELSE
            water_err = 0._r8
         ENDIF
         water_err_R = water_err * R_init
         err_minus_water = err - water_err_R
         ! The hard tracer check should ignore host water-budget non-closure
         ! for isotope tracers. Fractionation and runtime atmospheric forcing
         ! keep their process-owned flux signatures above, but they should not
         ! make the hard tracer check count host water non-closure as tracer
         ! non-conservation. A real tracer accounting bug remains visible in
         ! err_minus_water.
         IF (present(water_err_in) .and. water_corrected_check) THEN
            check_err = err_minus_water
         ELSE
            check_err = err
         ENDIF
         check_err = check_err - reactive_source_sink - numerical_source_sink
         IF (present(water_dS_in)) THEN
            water_dS = water_dS_in
         ELSE
            water_dS = 0._r8
         ENDIF
         IF (present(water_input_in)) THEN
            water_input = water_input_in
         ELSE
            water_input = 0._r8
         ENDIF
         IF (present(water_output_in)) THEN
            water_output = water_output_in
         ELSE
            water_output = 0._r8
         ENDIF
         IF (present(water_evap_in)) THEN
            water_evap = water_evap_in
         ELSE
            water_evap = 0._r8
         ENDIF
         IF (present(water_rnof_in)) THEN
            water_rnof = water_rnof_in
         ELSE
            water_rnof = 0._r8
         ENDIF
         a_water_precip(itrc, ipatch) = a_water_precip(itrc, ipatch) + max(water_input, 0._r8)
         a_water_rnof(itrc, ipatch) = a_water_rnof(itrc, ipatch) + max(water_rnof, 0._r8)
         dS_minus_water_R  = (storage_end - trc_storage_beg(itrc, ipatch)) &
                           - water_dS * R_init
         in_minus_water_R  = step_input  - water_input  * R_init
         out_minus_water_R = step_output - water_output * R_init
         evap_minus_water_R = step_evap - water_evap * R_init
         rnof_minus_water_R = step_rnof - water_rnof * R_init

         balance_scale = max(abs(storage_end), abs(trc_storage_beg(itrc, ipatch)), &
            abs(step_input_check), abs(step_output_check), &
            abs(reactive_source_sink), abs(numerical_source_sink))
         balance_tol = trc_balance_abs_tol + trc_balance_rel_tol * balance_scale
         resid_scale = max(1._r8, abs(storage_end), abs(trc_storage_beg(itrc, ipatch)), &
            abs(step_input_check), abs(step_output_check), abs(reactive_source_sink))
         resid_tol = max(trc_resid_abs_tol, trc_resid_rel_tol * resid_scale)

         signature_error = 0._r8
         signature_scale = 0._r8
         IF (fixed_signature_step) THEN
            IF (present(water_input_in)) THEN
               signature_error = max(signature_error, abs(in_minus_water_R))
               signature_scale = max(signature_scale, abs(step_input), abs(water_input * R_init))
            ENDIF
            IF (present(water_evap_in)) THEN
               signature_error = max(signature_error, abs(evap_minus_water_R))
               signature_scale = max(signature_scale, abs(step_evap), abs(water_evap * R_init))
            ENDIF
#ifndef CatchLateralFlow
            IF (present(water_rnof_in)) THEN
               signature_error = max(signature_error, abs(rnof_minus_water_R))
               signature_scale = max(signature_scale, abs(step_rnof), abs(water_rnof * R_init))
            ENDIF
#endif
         ENDIF
         signature_tol = trc_balance_abs_tol + trc_balance_rel_tol * signature_scale
         IF (fixed_signature_step .and. signature_error > signature_tol) THEN
            signature_nbad = signature_nbad + 1
            IF (signature_error > signature_worst_abs) THEN
               signature_worst_abs = signature_error
               signature_worst_tol = signature_tol
               signature_worst_terms = (/ in_minus_water_R, evap_minus_water_R, rnof_minus_water_R /)
               signature_worst_ipatch = ipatch
               signature_worst_itrc = itrc
               IF (present(patchtype_in)) THEN
                  signature_worst_ptype = patchtype_in
               ELSE
                  signature_worst_ptype = -1
               ENDIF
            ENDIF
         ENDIF

         IF (abs(check_err) > balance_tol) THEN
            ! Track worst-ever failure within this step (worker-local).
            ! tracer_balance_report surfaces the accumulated bulk at end of step.
            IF (abs(check_err) > abs(balance_worst_err)) THEN
               balance_worst_err    = check_err
               balance_worst_diag(1) = err
               balance_worst_diag(2) = storage_end - trc_storage_beg(itrc, ipatch)
               balance_worst_diag(3) = step_input
               balance_worst_diag(4) = step_output
               balance_worst_diag(5) = water_err
               balance_worst_diag(6) = water_err_R
               balance_worst_diag(7) = err_minus_water
               balance_worst_diag(8) = water_dS
               balance_worst_diag(9) = water_input
               balance_worst_diag(10) = water_output
               balance_worst_diag(11) = dS_minus_water_R
               balance_worst_diag(12) = in_minus_water_R
               balance_worst_diag(13) = out_minus_water_R
               balance_worst_diag(14) = step_evap
               balance_worst_diag(15) = step_rnof
               balance_worst_diag(16) = water_evap
               balance_worst_diag(17) = water_rnof
               balance_worst_diag(18) = evap_minus_water_R
               balance_worst_diag(19) = rnof_minus_water_R
               balance_worst_sbeg  = storage_comp_beg
               balance_worst_send  = storage_comp_end
               balance_worst_sds   = storage_comp_dS
               balance_worst_fcomp = (/ step_input, step_evap, step_rsur, step_rsub, &
                  step_rnof, step_qinfl, step_qcharge /)
               balance_worst_ipatch = ipatch
               balance_worst_itrc   = itrc
               IF (present(patchtype_in)) THEN
                  balance_worst_ptype = patchtype_in
               ELSE
                  balance_worst_ptype = -1
               ENDIF
            ENDIF
            balance_nbad = balance_nbad + 1
         ENDIF

         ! P1: surface a non-trivial numerical residual. This is context only;
         ! the residual is explicitly booked and excluded from the hard tol.
         IF (abs(numerical_source_sink) > trc_resid_warn_frac * &
                max(abs(storage_end), abs(trc_storage_beg(itrc, ipatch)))) THEN
            resid_nbad = resid_nbad + 1
            resid_worst_abs = max(resid_worst_abs, abs(numerical_source_sink))
         ENDIF
         IF (abs(numerical_source_sink) > resid_tol) THEN
            resid_hard_nbad = resid_hard_nbad + 1
            IF (abs(numerical_source_sink) > resid_hard_worst_abs) THEN
               resid_hard_worst_abs = abs(numerical_source_sink)
               resid_hard_worst_tol = resid_tol
               resid_hard_worst_ipatch = ipatch
               resid_hard_worst_itrc   = itrc
               IF (present(patchtype_in)) THEN
                  resid_hard_worst_ptype = patchtype_in
               ELSE
                  resid_hard_worst_ptype = -1
               ENDIF
            ENDIF
         ENDIF

         ! deltim is intent(in) and never checked. Every Makeoptions builds
         ! with -ffpe-trap=zero, so a caller passing 0 would not produce a
         ! harmless Inf in a diagnostic -- it would abort the model inside the
         ! conservation check, pointing the backtrace at the wrong place
         ! entirely. Report the raw residual instead; a rate is meaningless
         ! without a timestep anyway.
         IF (deltim > 0._r8) THEN
            xerr_tracer = max(xerr_tracer, abs(check_err) / deltim)
         ELSE
            xerr_tracer = max(xerr_tracer, abs(check_err))
         ENDIF
      ENDDO
   END SUBROUTINE tracer_balance_check

   !---------------------------------------------------------------
   ! Drain worker-local balance counters. Call once per CoLM step
   ! after all patches have run tracer_balance_check. Reduces the
   ! worst-err across workers so a single-patch failure elsewhere
   ! still lands in the per-rank log. Zeros the state so next step
   ! starts fresh.
   !
   ! This runs from inside CoLMDRIVER (worker-only gated at
   ! CoLM.F90:528). Reducing over p_comm_glb here would hang — master
   ! and IO never enter. Use p_comm_worker so the collective stays on
   ! the same ranks that produced the counts. Worker rank 0 prints the
   ! summary (not p_is_master, which lives on a different comm).
   !---------------------------------------------------------------
   SUBROUTINE tracer_balance_report ()
      USE MOD_SPMD_Task, only: CoLM_stop
#ifdef USEMPI
      USE MOD_SPMD_Task, only: p_comm_worker, p_iam_worker, p_err, p_is_worker
#endif
      IMPLICIT NONE
      real(r8) :: worst_abs, reduced_abs
      integer  :: nbad_total
      real(r8) :: resid_abs_total
      real(r8) :: resid_hard_abs_total, resid_hard_tol_total
      integer  :: resid_nbad_total
      integer  :: resid_hard_nbad_total
      real(r8) :: signature_abs_total, signature_tol_total
      real(r8) :: signature_terms_total(3)
      integer  :: signature_nbad_total
      integer  :: winner_rank
      integer  :: resid_winner_rank
      integer  :: signature_winner_rank
      integer  :: resid_hard_ipatch_total, resid_hard_itrc_total, resid_hard_ptype_total
      integer  :: signature_ipatch_total, signature_itrc_total, signature_ptype_total
      logical  :: print_me
      real(r8) :: maxima_local(4), maxima_total(4)
      integer  :: counts_local(4), counts_total(4)
      ! MAXLOC reduction: pair |err| with rank so we can ship ipatch/itrc
      ! from the rank that owns the worst patch. Standard MPI_MAXLOC on
      ! real+int works with MPI_2DOUBLE_PRECISION wrapping (rank as real).
      real(r8) :: in_pair(2), out_pair(2)
#ifdef USEMPI
      include 'mpif.h'
#endif

      IF (ntracers <= 0) RETURN

      worst_abs = abs(balance_worst_err)
      nbad_total  = balance_nbad
      resid_abs_total  = resid_worst_abs
      resid_nbad_total = resid_nbad
      resid_hard_abs_total  = resid_hard_worst_abs
      resid_hard_tol_total  = resid_hard_worst_tol
      resid_hard_nbad_total = resid_hard_nbad
      resid_hard_ipatch_total = resid_hard_worst_ipatch
      resid_hard_itrc_total   = resid_hard_worst_itrc
      resid_hard_ptype_total  = resid_hard_worst_ptype
      signature_abs_total     = signature_worst_abs
      signature_tol_total     = signature_worst_tol
      signature_terms_total   = signature_worst_terms
      signature_nbad_total    = signature_nbad
      signature_ipatch_total  = signature_worst_ipatch
      signature_itrc_total    = signature_worst_itrc
      signature_ptype_total   = signature_worst_ptype
      winner_rank = 0
      resid_winner_rank = 0
      signature_winner_rank = 0

#ifdef USEMPI
      reduced_abs = worst_abs
      IF (p_is_worker) THEN
         maxima_local = (/ worst_abs, resid_worst_abs, resid_hard_worst_abs, signature_worst_abs /)
         maxima_total = maxima_local
         counts_local = (/ balance_nbad, resid_nbad, resid_hard_nbad, signature_nbad /)
         counts_total = counts_local
         CALL mpi_reduce(maxima_local, maxima_total, 4, MPI_REAL8, MPI_MAX, 0, &
                         p_comm_worker, p_err)
         CALL mpi_allreduce(counts_local, counts_total, 4, MPI_INTEGER, MPI_SUM, &
                            p_comm_worker, p_err)

         reduced_abs          = maxima_total(1)
         resid_abs_total      = maxima_total(2)
         resid_hard_abs_total = maxima_total(3)
         signature_abs_total  = maxima_total(4)
         nbad_total            = counts_total(1)
         resid_nbad_total      = counts_total(2)
         resid_hard_nbad_total = counts_total(3)
         signature_nbad_total  = counts_total(4)

         IF (resid_hard_nbad_total > 0) THEN
            in_pair = (/ resid_hard_worst_abs, real(p_iam_worker, r8) /)
            CALL mpi_allreduce(in_pair, out_pair, 1, MPI_2DOUBLE_PRECISION, &
                               MPI_MAXLOC, p_comm_worker, p_err)
            resid_winner_rank = nint(out_pair(2))
            IF (resid_winner_rank /= 0) THEN
               IF (p_iam_worker == resid_winner_rank) THEN
                  CALL mpi_send(resid_hard_worst_ipatch, 1, MPI_INTEGER, 0, 101, p_comm_worker, p_err)
                  CALL mpi_send(resid_hard_worst_itrc,   1, MPI_INTEGER, 0, 102, p_comm_worker, p_err)
                  CALL mpi_send(resid_hard_worst_ptype,  1, MPI_INTEGER, 0, 103, p_comm_worker, p_err)
                  CALL mpi_send(resid_hard_worst_tol,    1, MPI_REAL8,   0, 104, p_comm_worker, p_err)
               ELSEIF (p_iam_worker == 0) THEN
                  CALL mpi_recv(resid_hard_ipatch_total, 1, MPI_INTEGER, resid_winner_rank, 101, &
                                p_comm_worker, MPI_STATUS_IGNORE, p_err)
                  CALL mpi_recv(resid_hard_itrc_total,   1, MPI_INTEGER, resid_winner_rank, 102, &
                                p_comm_worker, MPI_STATUS_IGNORE, p_err)
                  CALL mpi_recv(resid_hard_ptype_total,  1, MPI_INTEGER, resid_winner_rank, 103, &
                                p_comm_worker, MPI_STATUS_IGNORE, p_err)
                  CALL mpi_recv(resid_hard_tol_total,    1, MPI_REAL8,   resid_winner_rank, 104, &
                                p_comm_worker, MPI_STATUS_IGNORE, p_err)
               ENDIF
            ENDIF
         ENDIF

         IF (signature_nbad_total > 0) THEN
            in_pair = (/ signature_worst_abs, real(p_iam_worker, r8) /)
            CALL mpi_allreduce(in_pair, out_pair, 1, MPI_2DOUBLE_PRECISION, &
                               MPI_MAXLOC, p_comm_worker, p_err)
            signature_winner_rank = nint(out_pair(2))
            IF (signature_winner_rank /= 0) THEN
               IF (p_iam_worker == signature_winner_rank) THEN
                  CALL mpi_send(signature_worst_ipatch, 1, MPI_INTEGER, 0, 105, p_comm_worker, p_err)
                  CALL mpi_send(signature_worst_itrc,   1, MPI_INTEGER, 0, 106, p_comm_worker, p_err)
                  CALL mpi_send(signature_worst_ptype,  1, MPI_INTEGER, 0, 107, p_comm_worker, p_err)
                  CALL mpi_send(signature_worst_tol,    1, MPI_REAL8,   0, 108, p_comm_worker, p_err)
                  CALL mpi_send(signature_worst_terms,  3, MPI_REAL8,   0, 109, p_comm_worker, p_err)
               ELSEIF (p_iam_worker == 0) THEN
                  CALL mpi_recv(signature_ipatch_total, 1, MPI_INTEGER, signature_winner_rank, 105, &
                                p_comm_worker, MPI_STATUS_IGNORE, p_err)
                  CALL mpi_recv(signature_itrc_total,   1, MPI_INTEGER, signature_winner_rank, 106, &
                                p_comm_worker, MPI_STATUS_IGNORE, p_err)
                  CALL mpi_recv(signature_ptype_total,  1, MPI_INTEGER, signature_winner_rank, 107, &
                                p_comm_worker, MPI_STATUS_IGNORE, p_err)
                  CALL mpi_recv(signature_tol_total,    1, MPI_REAL8, signature_winner_rank, 108, &
                                p_comm_worker, MPI_STATUS_IGNORE, p_err)
                  CALL mpi_recv(signature_terms_total,  3, MPI_REAL8, signature_winner_rank, 109, &
                                p_comm_worker, MPI_STATUS_IGNORE, p_err)
               ENDIF
            ENDIF
         ENDIF

         IF (nbad_total > 0) THEN
            in_pair = (/ worst_abs, real(p_iam_worker, r8) /)
            CALL mpi_allreduce(in_pair, out_pair, 1, MPI_2DOUBLE_PRECISION, &
                               MPI_MAXLOC, p_comm_worker, p_err)
            winner_rank = nint(out_pair(2))
            IF (winner_rank /= 0) THEN
               IF (p_iam_worker == winner_rank) THEN
                  CALL mpi_send(balance_worst_ipatch, 1, MPI_INTEGER, 0, 91, p_comm_worker, p_err)
                  CALL mpi_send(balance_worst_itrc,   1, MPI_INTEGER, 0, 92, p_comm_worker, p_err)
                  CALL mpi_send(balance_worst_ptype,  1, MPI_INTEGER, 0, 93, p_comm_worker, p_err)
                  CALL mpi_send(balance_worst_diag,  19, MPI_REAL8,   0, 94, p_comm_worker, p_err)
                  CALL mpi_send(balance_worst_sds, n_storage_diag, MPI_REAL8, 0, 95, p_comm_worker, p_err)
                  CALL mpi_send(balance_worst_sbeg, n_storage_diag, MPI_REAL8, 0, 96, p_comm_worker, p_err)
                  CALL mpi_send(balance_worst_send, n_storage_diag, MPI_REAL8, 0, 97, p_comm_worker, p_err)
                  CALL mpi_send(balance_worst_fcomp, n_flux_diag, MPI_REAL8, 0, 98, p_comm_worker, p_err)
               ELSEIF (p_iam_worker == 0) THEN
                  CALL mpi_recv(balance_worst_ipatch, 1, MPI_INTEGER, winner_rank, 91, &
                                p_comm_worker, MPI_STATUS_IGNORE, p_err)
                  CALL mpi_recv(balance_worst_itrc,   1, MPI_INTEGER, winner_rank, 92, &
                                p_comm_worker, MPI_STATUS_IGNORE, p_err)
                  CALL mpi_recv(balance_worst_ptype,  1, MPI_INTEGER, winner_rank, 93, &
                                p_comm_worker, MPI_STATUS_IGNORE, p_err)
                  CALL mpi_recv(balance_worst_diag,  19, MPI_REAL8,   winner_rank, 94, &
                                p_comm_worker, MPI_STATUS_IGNORE, p_err)
                  CALL mpi_recv(balance_worst_sds, n_storage_diag, MPI_REAL8, winner_rank, 95, &
                                p_comm_worker, MPI_STATUS_IGNORE, p_err)
                  CALL mpi_recv(balance_worst_sbeg, n_storage_diag, MPI_REAL8, winner_rank, 96, &
                                p_comm_worker, MPI_STATUS_IGNORE, p_err)
                  CALL mpi_recv(balance_worst_send, n_storage_diag, MPI_REAL8, winner_rank, 97, &
                                p_comm_worker, MPI_STATUS_IGNORE, p_err)
                  CALL mpi_recv(balance_worst_fcomp, n_flux_diag, MPI_REAL8, winner_rank, 98, &
                                p_comm_worker, MPI_STATUS_IGNORE, p_err)
               ENDIF
            ENDIF
         ENDIF
      ENDIF
      print_me = (p_is_worker .and. p_iam_worker == 0)
#else
      reduced_abs = worst_abs
      print_me    = .true.
#endif

      IF (print_me .and. nbad_total > 0) THEN
         ! "nbad_entries" = (patch, tracer) pairs exceeding tol.
         ! Upper bound on unique patches is nbad_entries / ntracers
         ! (each patch contributes at most ntracers entries).
         ! ptype: 0=soil 1=urban 2=wetland 3=glacier 4=waterbody, -1=unknown.
         WRITE(*,'(A,I8,A,E12.5,A,I8,A,I4,A,I3,A,I4,A,E12.5,A,E12.5,A,E12.5,&
                  &A,E12.5,A,E12.5,A,E12.5,A,E12.5,A,E12.5,A,E12.5,A,E12.5,&
                  &A,E12.5,A,E12.5,A,E12.5,A,E12.5,A,E12.5,A,E12.5,A,E12.5,&
                  &A,E12.5,A,E12.5,A,E12.5)') &
            'TRC_BAL step report: nbad_entries=', nbad_total, &
            ' worst_abs_err=', reduced_abs, &
            ' @ipatch=', balance_worst_ipatch, &
            ' itrc=', balance_worst_itrc, &
            ' ptype=', balance_worst_ptype, &
            ' owner=', winner_rank, &
            ' err=', balance_worst_diag(1), &
            ' dS=', balance_worst_diag(2), &
            ' in=', balance_worst_diag(3), &
            ' out=', balance_worst_diag(4), &
            ' water_err=', balance_worst_diag(5), &
            ' water_err_R=', balance_worst_diag(6), &
            ' err_minus_water=', balance_worst_diag(7), &
            ' water_dS=', balance_worst_diag(8), &
            ' water_input=', balance_worst_diag(9), &
            ' water_output=', balance_worst_diag(10), &
            ' dS_minus_water_R=', balance_worst_diag(11), &
            ' in_minus_water_R=', balance_worst_diag(12), &
            ' out_minus_water_R=', balance_worst_diag(13), &
            ' step_evap=', balance_worst_diag(14), &
            ' step_rnof=', balance_worst_diag(15), &
            ' water_evap=', balance_worst_diag(16), &
            ' water_rnof=', balance_worst_diag(17), &
            ' evap_minus_water_R=', balance_worst_diag(18), &
            ' rnof_minus_water_R=', balance_worst_diag(19)
         WRITE(*,'(A,I8,A,I4,A,I3,A,E12.5,A,E12.5,A,E12.5,A,E12.5,A,E12.5,&
                  &A,E12.5,A,E12.5,A,E12.5,A,E12.5,A,E12.5,A,E12.5,A,E12.5)') &
            'TRC_BAL_DCOMP @ipatch=', balance_worst_ipatch, &
            ' itrc=', balance_worst_itrc, &
            ' ptype=', balance_worst_ptype, &
            ' d_ldew=', balance_worst_sds(1), &
            ' d_soil_liq=', balance_worst_sds(2), &
            ' d_soil_ice=', balance_worst_sds(3), &
            ' d_wa=', balance_worst_sds(4), &
            ' d_wdsrf=', balance_worst_sds(5), &
            ' d_wetwat=', balance_worst_sds(6), &
            ' d_scv=', balance_worst_sds(7), &
            ' d_waterstorage=', balance_worst_sds(8), &
            ' d_leaf_iso=', balance_worst_sds(9), &
            ' d_surface_residue=', balance_worst_sds(10), &
            ' d_subsurface_residue=', balance_worst_sds(11), &
            ' d_solid=', balance_worst_sds(12)
         WRITE(*,'(A,I8,A,I4,A,I3,A,E12.5,A,E12.5,A,E12.5,A,E12.5,A,E12.5,&
                  &A,E12.5,A,E12.5)') &
            'TRC_BAL_FCOMP @ipatch=', balance_worst_ipatch, &
            ' itrc=', balance_worst_itrc, &
            ' ptype=', balance_worst_ptype, &
            ' precip=', balance_worst_fcomp(1), &
            ' evap=', balance_worst_fcomp(2), &
            ' rsur=', balance_worst_fcomp(3), &
            ' rsub=', balance_worst_fcomp(4), &
            ' rnof=', balance_worst_fcomp(5), &
            ' qinfl=', balance_worst_fcomp(6), &
            ' qcharge=', balance_worst_fcomp(7)
         WRITE(*,'(A,I8,A,I4,A,I3,A,E12.5,A,E12.5,A,E12.5,A,E12.5,A,E12.5,&
                  &A,E12.5,A,E12.5,A,E12.5,A,E12.5,A,E12.5,A,E12.5,A,E12.5)') &
            'TRC_BAL_SEND @ipatch=', balance_worst_ipatch, &
            ' itrc=', balance_worst_itrc, &
            ' ptype=', balance_worst_ptype, &
            ' ldew=', balance_worst_send(1), &
            ' soil_liq=', balance_worst_send(2), &
            ' soil_ice=', balance_worst_send(3), &
            ' wa=', balance_worst_send(4), &
            ' wdsrf=', balance_worst_send(5), &
            ' wetwat=', balance_worst_send(6), &
            ' scv=', balance_worst_send(7), &
            ' waterstorage=', balance_worst_send(8), &
            ' leaf_iso=', balance_worst_send(9), &
            ' surface_residue=', balance_worst_send(10), &
            ' subsurface_residue=', balance_worst_send(11), &
            ' solid=', balance_worst_send(12)
      ENDIF

      IF (print_me .and. resid_nbad_total > 0) THEN
         ! Fires independently of nbad_total to identify entries where the
         ! numerical residual was explicitly excluded from hard balance accounting.
         WRITE(*,'(A,I8,A,E12.5)') &
            'TRC_BAL residual note (excluded from tol check): n_entries=', &
            resid_nbad_total, ' worst_abs=', resid_abs_total
      ENDIF

      IF (print_me .and. resid_hard_nbad_total > DEF_TRACER_RESID_ABORT_NBAD) THEN
         WRITE(*,'(A,I8,A,E12.5,A,E12.5,A,I8,A,I4,A,I3,A,I4)') &
            'TRC_BAL residual hard failure: n_entries=', resid_hard_nbad_total, &
            ' worst_abs=', resid_hard_abs_total, &
            ' tol=', resid_hard_tol_total, &
            ' @ipatch=', resid_hard_ipatch_total, &
            ' itrc=', resid_hard_itrc_total, &
            ' ptype=', resid_hard_ptype_total, &
            ' owner=', resid_winner_rank
         CALL CoLM_stop ('TRC_BAL hard failure: numerical residual exceeds independent threshold')
      ENDIF

      IF (print_me .and. signature_nbad_total > 0) THEN
         WRITE(*,'(A,I8,A,E12.5,A,E12.5,A,I8,A,I4,A,I3,A,I4,A,E12.5,A,E12.5,A,E12.5)') &
            'TRC_SIG step report: nbad_entries=', signature_nbad_total, &
            ' worst_abs=', signature_abs_total, &
            ' tol=', signature_tol_total, &
            ' @ipatch=', signature_ipatch_total, &
            ' itrc=', signature_itrc_total, &
            ' ptype=', signature_ptype_total, &
            ' owner=', signature_winner_rank, &
            ' input_minus_water_R=', signature_terms_total(1), &
            ' evap_minus_water_R=', signature_terms_total(2), &
            ' rnof_minus_water_R=', signature_terms_total(3)
      ENDIF

      IF (print_me .and. signature_nbad_total > DEF_TRACER_BALANCE_ABORT_NBAD) THEN
         CALL CoLM_stop ('TRC_SIG hard failure: fixed-signature flux differs from water flux times R_init')
      ENDIF

      IF (print_me .and. nbad_total > DEF_TRACER_BALANCE_ABORT_NBAD) THEN
         CALL CoLM_stop ('TRC_BAL hard failure: tracer balance error exceeds aggregate threshold')
      ENDIF

      balance_worst_err    = 0._r8
      balance_worst_diag   = 0._r8
      balance_worst_sbeg   = 0._r8
      balance_worst_send   = 0._r8
      balance_worst_sds    = 0._r8
      balance_worst_fcomp  = 0._r8
      balance_worst_ipatch = 0
      balance_worst_itrc   = 0
      balance_worst_ptype  = -1
      balance_nbad         = 0
      signature_worst_abs      = 0._r8
      signature_worst_tol      = 0._r8
      signature_worst_terms    = 0._r8
      signature_worst_ipatch   = 0
      signature_worst_itrc     = 0
      signature_worst_ptype    = -1
      signature_nbad           = 0
      resid_worst_abs      = 0._r8
      resid_nbad           = 0
      resid_hard_worst_abs = 0._r8
      resid_hard_worst_tol = 0._r8
      resid_hard_worst_ipatch = 0
      resid_hard_worst_itrc   = 0
      resid_hard_worst_ptype  = -1
      resid_hard_nbad      = 0
   END SUBROUTINE tracer_balance_report

END MODULE MOD_Tracer_Conservation
#endif
