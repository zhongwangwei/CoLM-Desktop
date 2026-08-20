#include <define.h>

#ifdef GridRiverLakeFlow
#ifdef TRACER
MODULE MOD_Tracer_RiverLake
!-------------------------------------------------------------------------------------
! DESCRIPTION:
!
!   Passive tracer transport module for GridRiverLakeFlow.
!   Supports arbitrary number of tracers (e.g., delta18O, deltaD).
!
!   Physics:
!     - Upwind advection: tracer flux = concentration × water flux
!     - Flux limiter for mass conservation (prevents negative storage)
!     - Fully-mixed assumption within each unit catchment
!
!   Reference: CaMa-Flood tracer module (cmf_ctrl_tracer_mod.F90)
!
! Created by CoLM, Apr 2026
!-------------------------------------------------------------------------------------

   USE MOD_Precision
   USE MOD_SPMD_Task
   USE MOD_Namelist
   ! Share the single authoritative `ntracers` from MOD_Tracer_Defs to
   ! avoid the land/river tracer modules holding two independent copies
   ! that could silently diverge on re-init.
   USE MOD_Tracer_Defs, only: ntracers, tracer_uses_land_water_transport, &
                              tracer_concentration_units, tracer_build_descriptor_identity, &
                              TRACER_DESCRIPTOR_IDENTITY_WIDTH
   IMPLICIT NONE

   !-------------------------------------------------------------------------------------
   ! Module variables
   !-------------------------------------------------------------------------------------
   character(len=32), allocatable :: tracer_names(:)  ! Tracer names

   ! Schema 1 is the first river/lake tracer restart contract that carries
   ! the complete physical descriptor, rather than only count + name hash.
   integer, parameter :: RIVER_TRACER_RESTART_SCHEMA_VERSION = 1
   real(r8), parameter :: TRC_RESTART_NEGATIVE_DUST = 1.0e-12_r8
   ! The coupled donor limiter iterates dimensionless rates in [0,1].  Every
   ! iterate is conservative, but stopping before a fixed point would add
   ! avoidable numerical diffusion, so a material non-convergence is fatal.
   real(r8), parameter :: TRC_LIMITER_RATE_TOL = 1.0e-12_r8
   real(r8), parameter :: TRC_LIMITER_OUT_TINY = 1.0e-30_r8

   ! State variables (prognostic)
   ! trc_mass is built from R_default * water_volume, so its units are
   ! [R*m3] (isotope ratio times cell water volume), NOT kg of heavy water.
   ! trc_conc = trc_mass / volwater is therefore a dimensionless ratio [R].
   real(r8), allocatable :: trc_mass  (:,:)   ! Tracer amount [R*m3] (ntracers, numucat)
   real(r8), allocatable :: trc_conc  (:,:)   ! Tracer ratio  [R]    (ntracers, numucat)

   ! Protected-side (behind-levee) tracer pool paired with
   ! `levsto` in MOD_Grid_RiverLakeLevee. Levee repartition moves water
   ! between visible-side (volwater_ucat, driving trc_mass) and
   ! protected-side storage; without its own tracer pool the mass would
   ! stay pinned to the visible side while water moves, systematically
   ! inflating visible-side concentration (and orphaning mass when
   ! visible-side goes dry).
   real(r8), allocatable :: trc_levsto(:,:)   ! Protected-side tracer amount [R*m3] (ntracers, numucat)

   ! Flux variables (diagnostic, per routing period)
   real(r8), allocatable :: trc_flux_out  (:,:) ! Tracer outflux [R*m3/s] (ntracers, numucat)
   real(r8), allocatable :: trc_dry_drain(:,:) ! Internal sink from dry-cell forced drain [R*m3]
   real(r8), allocatable :: trc_reactive_source(:,:) ! Net reactive source/sink over routing period

   ! Input: tracer amount flux from runoff
   real(r8), allocatable :: acc_trc_inp  (:,:) ! Accumulated tracer input [R*m3] (ntracers, numucat)
   real(r8), allocatable :: trc_inp_buf  (:,:) ! Buffered runoff tracer awaiting release [R*m3]
   real(r8), allocatable :: acc_rnof_ref (:)   ! Accumulated runoff water volume paired with acc_trc_inp [m3]

   ! Bifurcation net flux (saved from last tracer_substep for diagnostics)
   real(r8), allocatable :: trc_bif_net_saved (:,:) ! Per-cell net bif flux [mass/s] (ntracers, numucat)

   ! Near-dry volume floor used only for transport concentration
   ! stabilization; prognostic tracer mass always remains single-pool.
   real(r8), parameter :: trc_v_dry_off = 1.e-6_r8
   ! Display/history diagnostics should not let bathtub-scale residual
   ! volumes dominate isotope delta min/max. This does not alter state.
   real(r8), parameter :: trc_delta_diag_vmin = 1._r8
	   ! History accumulators
	   real(r8), allocatable :: a_trc_storage_mass(:,:) ! Accumulated visible storage tracer [R*m3*s]
	   real(r8), allocatable :: a_water_storage(:)       ! Accumulated visible water storage [m3*s]
	   real(r8), allocatable :: a_trc_levsto_mass(:,:)   ! Accumulated protected storage tracer [R*m3*s]
	   real(r8), allocatable :: a_levsto_water(:)        ! Accumulated protected water storage [m3*s]
	   real(r8), allocatable :: a_trc_out    (:,:) ! Accumulated tracer outflux [mass/s * s] (ntracers, numucat)
   real(r8), allocatable :: a_trc_bifout (:,:) ! Accumulated tracer bif net flux [mass/s * s] (ntracers, numucat)

   ! Routing tracer substep workspace. These buffers are reused across the
   ! many adaptive routing substeps to avoid repeated heap allocate/free churn.
   real(r8), allocatable, save :: conc_next(:)
   real(r8), allocatable, save :: flux_ups(:)
   real(r8), allocatable, save :: trc_flux(:)
   real(r8), allocatable, save :: trc_conc_flux(:)
   real(r8), allocatable, save :: trc_prot_conc_flux(:)
   real(r8), allocatable, save :: conc_dn_pth(:)
   real(r8), allocatable, save :: trc_prot_conc_dn_pth(:)
   real(r8), allocatable, save :: trc_pth_1trc(:)
   real(r8), allocatable, save :: trc_pth_lev(:)
   real(r8), allocatable, save :: trc_pth_levtrc(:,:)
   real(r8), allocatable, save :: bif_recv(:)
   real(r8), allocatable, save :: bif_lev_recv(:)
   real(r8), allocatable, save :: bif_net(:)
   real(r8), allocatable, save :: trc_bif_lev_net(:)
   real(r8), allocatable, save :: has_levee_r8(:)
   real(r8), allocatable, save :: has_levee_dn_pth(:)
   real(r8), allocatable, save :: dt_ucat(:)
   real(r8), allocatable, save :: dt_dn_pth(:)
   real(r8), allocatable, save :: trc_in_mass(:)
   real(r8), allocatable, save :: trc_in_mass_lev(:)
   real(r8), allocatable, save :: trc_out_mass(:)
   real(r8), allocatable, save :: trc_out_mass_lev(:)
   real(r8), allocatable, save :: rate_cell(:)
   real(r8), allocatable, save :: rate_cell_lev(:)
   real(r8), allocatable, save :: rate_next(:)
   real(r8), allocatable, save :: rate_dn_pth(:)
   real(r8), allocatable, save :: rate_dn_pth_lev(:)
   real(r8), allocatable, save :: trc_inp_step(:)
   real(r8), allocatable, save :: trc_dn_out_vis_pth(:)
   real(r8), allocatable, save :: trc_dn_out_lev_pth(:)
   real(r8), allocatable, save :: trc_dn_out_vis_recv(:)
   real(r8), allocatable, save :: trc_dn_out_lev_recv(:)

   PRIVATE :: conc_next, flux_ups, trc_flux, trc_conc_flux, trc_prot_conc_flux
   PRIVATE :: conc_dn_pth, trc_prot_conc_dn_pth, trc_pth_1trc, trc_pth_lev
   PRIVATE :: trc_pth_levtrc, bif_recv, bif_lev_recv, bif_net, trc_bif_lev_net
   PRIVATE :: has_levee_r8, has_levee_dn_pth, dt_ucat, dt_dn_pth
   PRIVATE :: trc_in_mass, trc_in_mass_lev, trc_out_mass, trc_out_mass_lev
   PRIVATE :: rate_cell, rate_cell_lev, rate_next, rate_dn_pth, rate_dn_pth_lev
   PRIVATE :: trc_inp_step, trc_dn_out_vis_pth, trc_dn_out_lev_pth
   PRIVATE :: trc_dn_out_vis_recv, trc_dn_out_lev_recv
   PRIVATE :: ensure_tracer_substep_workspace, release_tracer_substep_workspace
   PRIVATE :: ensure_real1_workspace, ensure_real2_workspace
   PRIVATE :: tracer_bif_path_levee_sides
   PRIVATE :: riverlake_restart_descriptor_compatible
   PRIVATE :: probe_riverlake_restart_vector
   PRIVATE :: validate_riverlake_restart_state

   PUBLIC :: river_lake_tracer_init
   PUBLIC :: tracer_init_from_water
   PUBLIC :: tracer_input_from_runoff
   PUBLIC :: tracer_flush_acc
   PUBLIC :: read_tracer_restart
   PUBLIC :: write_tracer_restart
   PUBLIC :: write_tracer_history
   PUBLIC :: river_lake_tracer_final
   PUBLIC :: check_tracer_state
   PUBLIC :: tracer_substep
   PUBLIC :: tracer_refresh_state
   PUBLIC :: tracer_diag_accumulate_substep
   ! Exposed so the inland-depression overflow fix in
   ! MOD_Grid_RiverLakeFlow can use the same reservoir/levee/floodplain
   ! volume selection as tracer_refresh_state instead of hard-wiring
   ! topo_rivstomax (which disagrees on leveed cells).
   PUBLIC :: get_cell_volume
   PUBLIC :: trc_inp_buf
   PUBLIC :: acc_rnof_ref
   PUBLIC :: trc_levsto
   PUBLIC :: trc_dry_drain
   PUBLIC :: trc_reactive_source
   PUBLIC :: levee_tracer_repartition

CONTAINS

   !-------------------------------------------------------------------------------------
   ! Initialize tracer module
   !-------------------------------------------------------------------------------------
   SUBROUTINE river_lake_tracer_init ()

   USE MOD_Grid_RiverLakeNetwork, only: numucat
   USE MOD_Tracer_Defs, only: tracer_defs_init, tracers
   IMPLICIT NONE

      integer :: i

      CALL river_lake_tracer_final()
      CALL tracer_defs_init()
      IF (ntracers <= 0) RETURN

      ! MOD_Tracer_Defs owns parsing, sanitisation, defaults, and de-duplication.
      ! River/lake keeps a local copy only because restart/history names use it.
      allocate (tracer_names(ntracers))
      DO i = 1, ntracers
         tracer_names(i) = tracers(i)%name
      ENDDO

      IF (p_is_worker) THEN
         ! Allocate on ALL workers (zero-length if numucat=0) for MPI safety.
         allocate (trc_mass     (ntracers, numucat))
         allocate (trc_conc     (ntracers, numucat))
         allocate (trc_flux_out (ntracers, numucat))
         allocate (trc_dry_drain(ntracers, numucat))
         allocate (trc_reactive_source(ntracers, numucat))
         allocate (acc_trc_inp        (ntracers, numucat))
         allocate (trc_inp_buf        (ntracers, numucat))
         allocate (acc_rnof_ref       (numucat))
	         allocate (trc_bif_net_saved  (ntracers, numucat))
		         allocate (a_trc_storage_mass (ntracers, numucat))
		         allocate (a_water_storage    (numucat))
		         allocate (a_trc_levsto_mass (ntracers, numucat))
		         allocate (a_levsto_water    (numucat))
		         allocate (a_trc_out          (ntracers, numucat))
         allocate (a_trc_bifout       (ntracers, numucat))
         allocate (trc_levsto         (ntracers, numucat))

         trc_mass     = 0._r8
         trc_conc     = 0._r8
         trc_flux_out  = 0._r8
         trc_dry_drain = 0._r8
         trc_reactive_source = 0._r8
         acc_trc_inp  = 0._r8
         trc_inp_buf  = 0._r8
         acc_rnof_ref = 0._r8
	         trc_bif_net_saved = 0._r8
		         a_trc_storage_mass = 0._r8
		         a_water_storage = 0._r8
		         a_trc_levsto_mass = 0._r8
		         a_levsto_water = 0._r8
		         a_trc_out    = 0._r8
         a_trc_bifout = 0._r8
         trc_levsto   = 0._r8
      ENDIF

      IF (p_is_master) THEN
         write(*,'(A,I4,A)') ' Tracer module initialised with ', ntracers, ' tracers:'
         write(*,'(A,*(A,:,", "))') '   Names: ', (trim(tracer_names(i)), i=1, ntracers)
      ENDIF

   END SUBROUTINE river_lake_tracer_init


   !-------------------------------------------------------------------------------------
   ! Cold-start river tracer mass from the current water volume.
   ! Mirrors the land-side tracer_init_from_water logic: when no tracer
   ! restart variable is present in the gridriver restart file but the
   ! hydrology restart provides a non-zero wdsrf_ucat/volresv, seed
   ! trc_mass = volwater * R_init. Without this, any hot-start of water
   ! starts with tracer=0 and bleeds spurious dilution into downstream
   ! concentrations until runoff "re-colours" the reservoir.
   !-------------------------------------------------------------------------------------
   SUBROUTINE tracer_init_from_water (wdsrf, volresv_in, ucat2resv_in, missing_mask)

   USE MOD_Grid_RiverLakeNetwork, only: numucat, lake_type
   USE MOD_Grid_RiverLakeLevee,   only: has_levee, levsto
   USE MOD_Tracer_Defs,           only: tracer_init_water_ratio
   IMPLICIT NONE

   real(r8), intent(in) :: wdsrf(:)
   real(r8), intent(in) :: volresv_in(:)
   integer,  intent(in) :: ucat2resv_in(:)
   ! Optional per-tracer selector. When present, only tracers with
   ! missing_mask(itrc) == .true. are re-seeded from the water volume;
   ! this avoids overwriting tracers that loaded successfully from a
   ! partially-complete restart file (e.g. user renamed one tracer or
   ! added a new tracer to DEF_TRACER_NAMES without re-writing the file).
   ! Without the mask every tracer is cold-started (equivalent to
   ! passing an all-.true. mask).
   logical,  optional, intent(in) :: missing_mask(:)

   integer  :: i, itrc
   real(r8) :: volwater, R_init
   logical  :: do_init
   integer  :: n_init

      IF (.not. p_is_worker) RETURN
      IF (numucat <= 0) RETURN
      IF (.not. allocated(trc_mass)) RETURN

      n_init = 0
      DO itrc = 1, ntracers
         IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
         do_init = .true.
         IF (present(missing_mask)) THEN
            IF (itrc <= size(missing_mask)) do_init = missing_mask(itrc)
         ENDIF
         IF (.not. do_init) CYCLE
         R_init = tracer_init_water_ratio(itrc)
         DO i = 1, numucat
            CALL get_cell_volume(i, wdsrf(i), volresv_in, ucat2resv_in, volwater)
            trc_mass(itrc, i) = max(volwater, 0._r8) * R_init
            ! Seed protected-side pool from levsto so a cold
            ! start with water already behind the levee doesn't produce
            ! a phantom conservation jump on the first repartition.
            IF (DEF_USE_LEVEE .and. allocated(trc_levsto) .and. allocated(has_levee)) THEN
               IF (has_levee(i) .and. lake_type(i) /= 2) THEN
                  trc_levsto(itrc, i) = max(levsto(i), 0._r8) * R_init
               ELSE
                  trc_levsto(itrc, i) = 0._r8
               ENDIF
            ENDIF
         ENDDO
         ! Recompute concentration so diagnostics consistent before first step.
         DO i = 1, numucat
            CALL get_cell_volume(i, wdsrf(i), volresv_in, ucat2resv_in, volwater)
            CALL update_tracer_concentration(itrc, i, volwater)
         ENDDO
         n_init = n_init + 1
      ENDDO

      IF (p_is_master .and. n_init > 0) THEN
         write(*,'(A,I0,A)') '  River tracer cold-started from water volume for ', n_init, ' tracer(s).'
      ENDIF

   END SUBROUTINE tracer_init_from_water


   !-------------------------------------------------------------------------------------
   ! Accumulate heavy-water mass input from runoff.
   ! Called each land-model timestep (before routing accumulation threshold).
   !
   ! rnof_uc_depth(i):           runoff water column [m] (runoff flux * dt,
   !                             area-weighted per ucat cell — NOT a volume).
   ! trc_rnof_ext(itrc, i):      (optional) tracer amount from land tracer
   !                             in matching [R*m] units (depth-based, not mass).
   !
   ! If trc_rnof_ext is present, use it directly (coupled to land tracer system).
   ! Otherwise, compute default heavy-water mass from ref_ratio and init_delta.
   !-------------------------------------------------------------------------------------
   SUBROUTINE tracer_input_from_runoff (rnof_uc_depth, numucat_in, trc_rnof_ext)

      ! Re-uses the per-tracer ref_ratio / init_delta cached by
      ! tracer_defs_init (MOD_Tracer_Defs.F90:82-84). Previously this
      ! routine re-parsed the CSV namelist strings via a local helper on
      ! every land timestep — wasteful and a divergence hazard (two
      ! parse code paths for the same inputs).
      USE MOD_Tracer_Defs, only: tracer_init_water_ratio
      IMPLICIT NONE
      real(r8), intent(in) :: rnof_uc_depth(:)
      integer,  intent(in) :: numucat_in
      real(r8), intent(in), optional :: trc_rnof_ext(:,:)

      integer :: i, itrc
      real(r8), allocatable :: R_default(:)

      ! TRACER may be compiled while the runtime registry intentionally
      ! contains zero tracers (DEF_TRACER_NUM=0).  In that mode river_lake_tracer_init()
      ! returns without allocating the routing buffers, so the input path must
      ! be a no-op instead of touching acc_rnof_ref / acc_trc_inp.
      IF (ntracers <= 0) RETURN
      IF (.not. allocated(acc_rnof_ref)) RETURN
      IF (.not. allocated(acc_trc_inp)) RETURN
      IF (numucat_in <= 0) RETURN

      ! Cache per-tracer R_default once; absent trc_rnof_ext it is used
      ! for every cell and was previously recomputed inside the inner
      ! loop (delta_to_R is pure but costs trig/div per call).
      IF (.not. present(trc_rnof_ext)) THEN
         allocate(R_default(ntracers))
         DO itrc = 1, ntracers
            IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
            R_default(itrc) = tracer_init_water_ratio(itrc)
         ENDDO
      ENDIF

      ! Mirror the water-side `acc_rnof_uc` accumulation
      ! at MOD_Grid_RiverLakeFlow.F90:329 exactly. That line has no
      ! sign filter — both positive and negative rnof contributions are
      ! summed. The previous `IF (rnof_uc_depth > 0)` guard here let
      ! FP-noise negatives drift the water/tracer accumulators apart,
      ! which would make restart recovery and levee pending-pool
      ! repartition use inconsistent water and tracer windows.
      DO i = 1, numucat_in
         acc_rnof_ref(i) = acc_rnof_ref(i) + rnof_uc_depth(i)
         DO itrc = 1, ntracers
            IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
            IF (present(trc_rnof_ext)) THEN
               acc_trc_inp(itrc, i) = acc_trc_inp(itrc, i) + trc_rnof_ext(itrc, i)
            ELSE
               acc_trc_inp(itrc, i) = acc_trc_inp(itrc, i) + R_default(itrc) * rnof_uc_depth(i)
            ENDIF
         ENDDO
      ENDDO

      IF (allocated(R_default)) deallocate(R_default)

   END SUBROUTINE tracer_input_from_runoff


   !-------------------------------------------------------------------------------------
   ! Helper: get water volume for a unit catchment cell, respecting reservoir
   ! state and the prognostic routing volume. For levee cells volwater_ucat
   ! is the visible-side volume; for ordinary cells it is the total volume.
   !-------------------------------------------------------------------------------------
   SUBROUTINE get_cell_volume (icell, wdsrf_cell, volresv_in, ucat2resv_in, volwater)

   USE MOD_Grid_RiverLakeNetwork, only: floodplain_curve, lake_type
   USE MOD_Grid_RiverLakeLevee,   only: has_levee, levsto, levee_visible_volume_from_stage
   USE MOD_Grid_RiverLakeTimeVars, only: volwater_ucat, volwater_ucat_valid
   USE MOD_Vars_Global,           only: spval
   IMPLICIT NONE

   integer,  intent(in)  :: icell
   real(r8), intent(in)  :: wdsrf_cell
   real(r8), intent(in)  :: volresv_in(:)
   integer,  intent(in)  :: ucat2resv_in(:)
	   real(r8), intent(out) :: volwater
	   logical               :: has_levee_cell
	   real(r8), parameter   :: stage_restart_tol = 1.e-5_r8

      has_levee_cell = .false.
      IF (DEF_USE_LEVEE .and. allocated(has_levee)) THEN
         IF (icell >= 1 .and. icell <= size(has_levee)) has_levee_cell = has_levee(icell)
      ENDIF

      IF (lake_type(icell) == 2 .and. size(volresv_in) > 0 .and. size(ucat2resv_in) > 0) THEN
         IF (ucat2resv_in(icell) > 0 .and. ucat2resv_in(icell) <= size(volresv_in) &
            .and. volresv_in(ucat2resv_in(icell)) /= spval) THEN
            volwater = volresv_in(ucat2resv_in(icell))
         ELSE
            volwater = floodplain_curve(icell)%volume(wdsrf_cell)
         ENDIF
      ELSEIF (has_levee_cell) THEN
         IF (volwater_ucat_valid .and. allocated(volwater_ucat)) THEN
            IF (icell <= size(volwater_ucat)) THEN
               ! Backward compatibility: old restarts may carry
               ! volwater_ucat as a zero placeholder even for wet cells.
               ! Reconstruct those from stage instead of treating levee
               ! visible storage as dry.
               IF (volwater_ucat(icell) > 0._r8 .or. wdsrf_cell <= stage_restart_tol) THEN
                  volwater = volwater_ucat(icell)
                  volwater = max(volwater, 0._r8)
                  RETURN
               ENDIF
            ENDIF
         ENDIF

         IF (allocated(levsto)) THEN
            volwater = levee_visible_volume_from_stage(icell, wdsrf_cell, levsto(icell))
         ELSE
            volwater = floodplain_curve(icell)%volume(wdsrf_cell)
         ENDIF
         volwater = max(volwater, 0._r8)
      ELSE
         IF (volwater_ucat_valid .and. allocated(volwater_ucat)) THEN
            IF (icell <= size(volwater_ucat)) THEN
               IF (volwater_ucat(icell) > 0._r8 .or. wdsrf_cell <= stage_restart_tol) THEN
                  volwater = volwater_ucat(icell)
                  volwater = max(volwater, 0._r8)
                  RETURN
               ENDIF
            ENDIF
         ENDIF

         volwater = floodplain_curve(icell)%volume(wdsrf_cell)
         volwater = max(volwater, 0._r8)
      ENDIF

   END SUBROUTINE get_cell_volume


   !-------------------------------------------------------------------------------------
   ! Repartition tracer mass between visible-side (`trc_mass`)
   ! and protected-side (`trc_levsto`) pools in lockstep with the water
   ! redistribution that `levee_fldstg` just performed.
   !
   ! Inputs are the visible-side and protected-side water volumes BEFORE
   ! and AFTER the levee flood-staging call. Two conventions:
   !   d_lev > 0 : water moved visible -> protected. Tracer transferred
   !               at the visible-side's current ratio.
   !   d_lev < 0 : water moved protected -> visible. Tracer transferred
   !               at the protected-side's current ratio.
   ! This preserves asymmetric concentrations across the levee (unlike
   ! a uniform pool-ratio split which homogenises the two compartments).
   !-------------------------------------------------------------------------------------
   SUBROUTINE levee_tracer_repartition (icell, vis_vol_bef, levsto_bef, vis_vol_aft, levsto_aft, &
                                        pending_trc_pool)

   USE MOD_Tracer_Defs, only: trc_tiny
   IMPLICIT NONE
   integer,  intent(in) :: icell
   real(r8), intent(in) :: vis_vol_bef, levsto_bef
   real(r8), intent(in) :: vis_vol_aft, levsto_aft
   ! Optional pool of period-accumulated runoff tracer that is conceptually on
   ! the visible side (its water has already been folded into vis_vol_bef) but
   ! has not yet been merged into trc_mass. Caller passes trc_inp_buf(:, icell)
   ! at the pre-tracer-substep call site so the visible-side ratio reflects
   ! the post-input state. Without this, the ratio is diluted
   ! (denominator includes runoff water but numerator does not include the
   ! matching runoff tracer), leaking the Phase-1 R_init invariant.
   real(r8), intent(inout), optional :: pending_trc_pool(:)

   integer  :: itrc
   real(r8) :: d_lev, ratio, trc_move
   real(r8) :: vis_pending, vis_total
   real(r8) :: debit_mass, debit_pending

      IF (.not. allocated(trc_mass) .or. .not. allocated(trc_levsto)) RETURN
      IF (icell < 1 .or. icell > size(trc_mass, 2)) RETURN

      d_lev = levsto_aft - levsto_bef

      IF (abs(d_lev) < trc_tiny) RETURN

      DO itrc = 1, ntracers
         IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
         vis_pending = 0._r8
         IF (present(pending_trc_pool)) THEN
            IF (itrc <= size(pending_trc_pool)) &
               vis_pending = max(pending_trc_pool(itrc), 0._r8)
         ENDIF
         IF (d_lev > 0._r8) THEN
            ! Visible -> protected. Use post-input ratio (trc_mass + pending) /
            ! vis_vol_bef so the runoff tracer that already shares vis_vol_bef
            ! contributes to the concentration moved across the levee. Debit
            ! trc_mass first; if it would underrun zero, spill the residual
            ! into pending so the visible-side pool stays non-negative without
            ! double-counting.
            vis_total = trc_mass(itrc, icell) + vis_pending
            IF (vis_vol_bef > trc_tiny) THEN
               ratio = vis_total / vis_vol_bef
            ELSE
               ratio = 0._r8
            ENDIF
            trc_move = d_lev * ratio
            ! Cap by total visible mass (signed) so we never transfer more
            ! than the donor pool actually contains.
            IF (trc_move > 0._r8) THEN
               trc_move = min(trc_move, max(vis_total, 0._r8))
            ELSE
               trc_move = max(trc_move, min(vis_total, 0._r8))
            ENDIF
            ! Apportion the debit: take from trc_mass up to its current value,
            ! spill any residual into pending (only if pending is provided).
            IF (trc_move > 0._r8) THEN
               debit_mass    = min(trc_move, max(trc_mass(itrc, icell), 0._r8))
               debit_pending = trc_move - debit_mass
            ELSE
               debit_mass    = max(trc_move, min(trc_mass(itrc, icell), 0._r8))
               debit_pending = trc_move - debit_mass
            ENDIF
            trc_mass  (itrc, icell) = trc_mass  (itrc, icell) - debit_mass
            trc_levsto(itrc, icell) = trc_levsto(itrc, icell) + trc_move
            IF (present(pending_trc_pool) .and. abs(debit_pending) > 0._r8) THEN
               IF (itrc <= size(pending_trc_pool)) THEN
                  pending_trc_pool(itrc) = pending_trc_pool(itrc) - debit_pending
               ENDIF
            ENDIF
         ELSE
            ! Protected -> visible. Protected side has no pending runoff input,
            ! so the original ratio = trc_levsto / levsto_bef is correct.
            IF (levsto_bef > trc_tiny) THEN
               ratio = trc_levsto(itrc, icell) / levsto_bef
            ELSE
               ratio = 0._r8
            ENDIF
            trc_move = abs(d_lev) * ratio
            IF (trc_move > 0._r8) THEN
               trc_move = min(trc_move, max(trc_levsto(itrc, icell), 0._r8))
            ELSE
               trc_move = max(trc_move, min(trc_levsto(itrc, icell), 0._r8))
            ENDIF
            trc_levsto(itrc, icell) = trc_levsto(itrc, icell) - trc_move
            trc_mass  (itrc, icell) = trc_mass  (itrc, icell) + trc_move
         ENDIF
      ENDDO

   END SUBROUTINE levee_tracer_repartition


   !-------------------------------------------------------------------------------------
   ! Refresh concentration from single-pool mass and current water volume.
   !-------------------------------------------------------------------------------------
   SUBROUTINE update_tracer_concentration (itrc, icell, volwater)

   IMPLICIT NONE

   integer,  intent(in) :: itrc, icell
   real(r8), intent(in) :: volwater

      ! Intentionally does NOT clamp trc_mass here, even for tiny
      ! negatives (e.g. -1e-15 R*m³). Clamping state silently would
      ! swallow any future transport / flux-limiter bug that drifts
      ! trc_mass negative progressively — each substep would re-zero
      ! the residual below an absolute threshold and hide the drift.
      ! Instead, display-side reports (check_tracer_state) dust-clamp
      ! for log cleanliness, and report a WARNING when magnitudes
      ! exceed the FP-noise floor. See tracer_mass_fp_dust in
      ! check_tracer_state below for the cosmetic threshold.
      IF (volwater <= trc_v_dry_off) THEN
         trc_conc(itrc, icell) = 0._r8
      ELSE
         trc_conc(itrc, icell) = trc_mass(itrc, icell) / volwater
      ENDIF

   END SUBROUTINE update_tracer_concentration


   !-------------------------------------------------------------------------------------
   ! Refresh tracer concentration from the final hydrologic state.
   ! This is called after the routing loop has updated water volumes so that
   ! diagnostics/history read a concentration consistent with the final step state.
   !-------------------------------------------------------------------------------------
   SUBROUTINE tracer_refresh_state (wdsrf, volresv_in, ucat2resv_in)

   USE MOD_Grid_RiverLakeNetwork, only: numucat, floodplain_curve, lake_type
   USE MOD_Grid_RiverLakeLevee,   only: has_levee
   USE MOD_Grid_RiverLakeTimeVars, only: volwater_ucat
   IMPLICIT NONE

   real(r8), intent(in) :: wdsrf(:)
   real(r8), intent(in) :: volresv_in(:)
   integer,  intent(in) :: ucat2resv_in(:)

   integer :: i, itrc
   real(r8) :: volwater

      IF (.not. p_is_worker) RETURN
      IF (numucat <= 0) RETURN

      DO i = 1, numucat
         CALL get_cell_volume(i, wdsrf(i), volresv_in, ucat2resv_in, volwater)
         DO itrc = 1, ntracers
            IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
            CALL update_tracer_concentration(itrc, i, volwater)
         ENDDO
      ENDDO

   END SUBROUTINE tracer_refresh_state


   !-------------------------------------------------------------------------------------
   ! Accumulate tracer history diagnostics for one routing substep using the final
   ! post-update water state and any post-transport flux corrections already applied
   ! by the flow solver (for example inland-depression overflow adjustments).
   !-------------------------------------------------------------------------------------
   SUBROUTINE tracer_diag_accumulate_substep (dt_all, irivsys, ucatfilter, wdsrf, volresv_in, ucat2resv_in)

	   USE, INTRINSIC :: ieee_arithmetic, only: ieee_is_finite
	   USE MOD_Grid_RiverLakeNetwork, only: numucat
	   USE MOD_Grid_RiverLakeLevee,   only: has_levee, levsto
	   USE MOD_Tracer_Defs, only: trc_tiny
   IMPLICIT NONE

   real(r8), intent(in) :: dt_all(:)
   integer,  intent(in) :: irivsys(:)
   logical,  intent(in) :: ucatfilter(:)
   real(r8), intent(in) :: wdsrf(:)
   real(r8), intent(in) :: volresv_in(:)
   integer,  intent(in) :: ucat2resv_in(:)

   integer :: i, itrc
   real(r8) :: dt_i, volwater, dry_drain

	      IF (.not. p_is_worker) RETURN
	      IF (numucat <= 0) RETURN

      DO i = 1, numucat
         CALL get_cell_volume(i, wdsrf(i), volresv_in, ucat2resv_in, volwater)
         dt_i = 0._r8
         IF (irivsys(i) > 0 .and. irivsys(i) <= size(dt_all)) dt_i = dt_all(irivsys(i))
         IF (volwater <= trc_v_dry_off) THEN
            DO itrc = 1, ntracers
               IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
               ! Dry cleanup is a budgeted sink for non-negative physical mass,
               ! not a way to erase an invalid state before the watchdog sees it.
               IF (.not. ieee_is_finite(trc_mass(itrc, i))) THEN
                  CALL CoLM_stop('non-finite river tracer mass reached dry-cell cleanup')
               ELSEIF (trc_mass(itrc, i) < -TRC_RESTART_NEGATIVE_DUST) THEN
                  CALL CoLM_stop('negative river tracer mass reached dry-cell cleanup')
               ELSEIF (trc_mass(itrc, i) < 0._r8) THEN
                  trc_mass(itrc, i) = 0._r8
               ENDIF
               IF (.not. ieee_is_finite(trc_inp_buf(itrc, i))) &
                  CALL CoLM_stop('non-finite signed river tracer buffer reached dry-cell cleanup')
               ! NEG_RUNOFF_DEBT: a negative pending runoff tracer is a
               ! signed correction tied to future same-cell runoff input, not
               ! a physical negative river outflow.  Dry-cell cleanup drains
               ! only positive orphan mass/pending input and preserves a
               ! negative trc_inp_buf debt so the next positive runoff can
               ! cancel it with the correct source signature.
               dry_drain = max(trc_mass(itrc, i), 0._r8) + max(trc_inp_buf(itrc, i), 0._r8)
               IF (dry_drain > trc_tiny) THEN
                  IF (allocated(trc_dry_drain)) trc_dry_drain(itrc, i) = trc_dry_drain(itrc, i) + dry_drain
                  ! Keep advective outflux and the internal dry-cell sink
                  ! separate. CoLMDEBUG books trc_flux_out at river mouths and
                  ! adds trc_dry_drain globally; folding the sink into both
                  ! arrays counted a dry mouth twice. Preserve the existing
                  ! history diagnostic explicitly without contaminating flux.
                  IF (dt_i > 0._r8 .and. ucatfilter(i)) &
                     a_trc_out(itrc, i) = a_trc_out(itrc, i) + dry_drain
               ENDIF
               trc_mass(itrc, i)    = 0._r8
               trc_inp_buf(itrc, i) = min(trc_inp_buf(itrc, i), 0._r8)
            ENDDO
         ENDIF
         DO itrc = 1, ntracers
            IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
            CALL update_tracer_concentration(itrc, i, volwater)
         ENDDO
         IF (.not. ucatfilter(i)) CYCLE
         IF (irivsys(i) <= 0 .or. irivsys(i) > size(dt_all)) CYCLE
	      IF (dt_i <= 0._r8) CYCLE
		      IF (volwater > trc_v_dry_off) THEN
		         IF (allocated(a_water_storage)) a_water_storage(i) = a_water_storage(i) + volwater * dt_i
		      ENDIF
		      IF (allocated(a_levsto_water) .and. allocated(levsto) .and. allocated(has_levee)) THEN
		         IF (i <= size(levsto) .and. i <= size(has_levee)) THEN
		            IF (has_levee(i) .and. levsto(i) > trc_v_dry_off) THEN
		               a_levsto_water(i) = a_levsto_water(i) + levsto(i) * dt_i
		            ENDIF
		         ENDIF
		      ENDIF

		      DO itrc = 1, ntracers
		         IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
		         IF (volwater > trc_v_dry_off .and. allocated(a_trc_storage_mass)) THEN
		            a_trc_storage_mass(itrc, i) = a_trc_storage_mass(itrc, i) &
		               + trc_mass(itrc, i) * dt_i
		         ENDIF
		         IF (allocated(a_trc_levsto_mass) .and. allocated(trc_levsto) .and. &
		             allocated(levsto) .and. allocated(has_levee)) THEN
		            IF (i <= size(trc_levsto, 2) .and. i <= size(levsto) .and. i <= size(has_levee)) THEN
		               IF (has_levee(i) .and. levsto(i) > trc_v_dry_off) THEN
		                  a_trc_levsto_mass(itrc, i) = a_trc_levsto_mass(itrc, i) &
		                     + trc_levsto(itrc, i) * dt_i
		               ENDIF
		            ENDIF
		         ENDIF
		         a_trc_out   (itrc, i) = a_trc_out   (itrc, i) + trc_flux_out(itrc, i) * dt_i
            a_trc_bifout(itrc, i) = a_trc_bifout(itrc, i) + trc_bif_net_saved(itrc, i) * dt_i
         ENDDO
      ENDDO

   END SUBROUTINE tracer_diag_accumulate_substep


   SUBROUTINE ensure_tracer_substep_workspace (nucat, npth, nlev_bif, bif_active)
      IMPLICIT NONE
      integer, intent(in) :: nucat, npth, nlev_bif
      logical, intent(in) :: bif_active

      CALL ensure_real1_workspace(conc_next, nucat)
      CALL ensure_real1_workspace(flux_ups, nucat)
      CALL ensure_real1_workspace(trc_flux, nucat)
      CALL ensure_real1_workspace(trc_conc_flux, nucat)
      CALL ensure_real1_workspace(trc_prot_conc_flux, nucat)
      CALL ensure_real1_workspace(bif_net, nucat)
      CALL ensure_real1_workspace(trc_bif_lev_net, nucat)
      CALL ensure_real1_workspace(trc_in_mass, nucat)
      CALL ensure_real1_workspace(trc_in_mass_lev, nucat)
      CALL ensure_real1_workspace(trc_out_mass, nucat)
      CALL ensure_real1_workspace(trc_out_mass_lev, nucat)
      CALL ensure_real1_workspace(rate_cell, nucat)
      CALL ensure_real1_workspace(rate_cell_lev, nucat)
      CALL ensure_real1_workspace(rate_next, nucat)
      CALL ensure_real1_workspace(trc_inp_step, nucat)

      IF (bif_active) THEN
         CALL ensure_real1_workspace(conc_dn_pth, npth)
         CALL ensure_real1_workspace(trc_prot_conc_dn_pth, npth)
         CALL ensure_real1_workspace(trc_pth_1trc, npth)
         CALL ensure_real1_workspace(trc_pth_lev, npth)
         CALL ensure_real2_workspace(trc_pth_levtrc, nlev_bif, npth)
         CALL ensure_real1_workspace(bif_recv, nucat)
         CALL ensure_real1_workspace(bif_lev_recv, nucat)
         CALL ensure_real1_workspace(rate_dn_pth, npth)
         CALL ensure_real1_workspace(rate_dn_pth_lev, npth)
         CALL ensure_real1_workspace(has_levee_r8, nucat)
         CALL ensure_real1_workspace(has_levee_dn_pth, npth)
         CALL ensure_real1_workspace(dt_ucat, nucat)
         CALL ensure_real1_workspace(dt_dn_pth, npth)
         CALL ensure_real1_workspace(trc_dn_out_vis_pth, npth)
         CALL ensure_real1_workspace(trc_dn_out_lev_pth, npth)
         CALL ensure_real1_workspace(trc_dn_out_vis_recv, nucat)
         CALL ensure_real1_workspace(trc_dn_out_lev_recv, nucat)
      ENDIF

   END SUBROUTINE ensure_tracer_substep_workspace


   SUBROUTINE ensure_real1_workspace (buf, n)
      IMPLICIT NONE
      real(r8), allocatable, intent(inout) :: buf(:)
      integer, intent(in) :: n

      IF (allocated(buf)) THEN
         IF (size(buf) /= n) deallocate(buf)
      ENDIF
      IF (.not. allocated(buf)) allocate(buf(n))

   END SUBROUTINE ensure_real1_workspace


   SUBROUTINE ensure_real2_workspace (buf, n1, n2)
      IMPLICIT NONE
      real(r8), allocatable, intent(inout) :: buf(:,:)
      integer, intent(in) :: n1, n2

      IF (allocated(buf)) THEN
         IF (size(buf, 1) /= n1 .or. size(buf, 2) /= n2) deallocate(buf)
      ENDIF
      IF (.not. allocated(buf)) allocate(buf(n1, n2))

   END SUBROUTINE ensure_real2_workspace


   SUBROUTINE tracer_bif_path_levee_sides (can_use_levee_tracer, i_up, i_dn, ipth, nucat, &
      upstream_has_levee, downstream_has_levee)
      USE MOD_Grid_RiverLakeLevee, only: has_levee
      IMPLICIT NONE
      logical, intent(in) :: can_use_levee_tracer
      integer, intent(in) :: i_up, i_dn, ipth, nucat
      logical, intent(out) :: upstream_has_levee, downstream_has_levee

      upstream_has_levee = .false.
      downstream_has_levee = .false.
      IF (.not. can_use_levee_tracer .or. .not. allocated(has_levee)) RETURN

      IF (i_up > 0 .and. i_up <= size(has_levee)) upstream_has_levee = has_levee(i_up)
      IF (i_dn > 0 .and. i_dn <= nucat) THEN
         IF (i_dn <= size(has_levee)) downstream_has_levee = has_levee(i_dn)
      ELSEIF (allocated(has_levee_dn_pth)) THEN
         IF (ipth > 0 .and. ipth <= size(has_levee_dn_pth)) &
            downstream_has_levee = has_levee_dn_pth(ipth) > 0.5_r8
      ENDIF
   END SUBROUTINE tracer_bif_path_levee_sides


   SUBROUTINE release_tracer_substep_workspace ()
      IMPLICIT NONE

      IF (allocated(conc_next             )) deallocate(conc_next             )
      IF (allocated(flux_ups              )) deallocate(flux_ups              )
      IF (allocated(trc_flux              )) deallocate(trc_flux              )
      IF (allocated(trc_conc_flux         )) deallocate(trc_conc_flux         )
      IF (allocated(trc_prot_conc_flux    )) deallocate(trc_prot_conc_flux    )
      IF (allocated(conc_dn_pth           )) deallocate(conc_dn_pth           )
      IF (allocated(trc_prot_conc_dn_pth  )) deallocate(trc_prot_conc_dn_pth  )
      IF (allocated(trc_pth_1trc          )) deallocate(trc_pth_1trc          )
      IF (allocated(trc_pth_lev           )) deallocate(trc_pth_lev           )
      IF (allocated(trc_pth_levtrc        )) deallocate(trc_pth_levtrc        )
      IF (allocated(bif_recv              )) deallocate(bif_recv              )
      IF (allocated(bif_lev_recv          )) deallocate(bif_lev_recv          )
      IF (allocated(bif_net               )) deallocate(bif_net               )
      IF (allocated(trc_bif_lev_net       )) deallocate(trc_bif_lev_net       )
      IF (allocated(has_levee_r8          )) deallocate(has_levee_r8          )
      IF (allocated(has_levee_dn_pth      )) deallocate(has_levee_dn_pth      )
      IF (allocated(dt_ucat               )) deallocate(dt_ucat               )
      IF (allocated(dt_dn_pth             )) deallocate(dt_dn_pth             )
      IF (allocated(trc_in_mass           )) deallocate(trc_in_mass           )
      IF (allocated(trc_in_mass_lev       )) deallocate(trc_in_mass_lev       )
      IF (allocated(trc_out_mass          )) deallocate(trc_out_mass          )
      IF (allocated(trc_out_mass_lev      )) deallocate(trc_out_mass_lev      )
      IF (allocated(rate_cell             )) deallocate(rate_cell             )
      IF (allocated(rate_cell_lev         )) deallocate(rate_cell_lev         )
      IF (allocated(rate_next             )) deallocate(rate_next             )
      IF (allocated(rate_dn_pth           )) deallocate(rate_dn_pth           )
      IF (allocated(rate_dn_pth_lev       )) deallocate(rate_dn_pth_lev       )
      IF (allocated(trc_inp_step          )) deallocate(trc_inp_step          )
      IF (allocated(trc_dn_out_vis_pth    )) deallocate(trc_dn_out_vis_pth    )
      IF (allocated(trc_dn_out_lev_pth    )) deallocate(trc_dn_out_lev_pth    )
      IF (allocated(trc_dn_out_vis_recv   )) deallocate(trc_dn_out_vis_recv   )
      IF (allocated(trc_dn_out_lev_recv   )) deallocate(trc_dn_out_lev_recv   )

   END SUBROUTINE release_tracer_substep_workspace


   !-------------------------------------------------------------------------------------
   ! Per-sub-step tracer transport (called inside the DO WHILE routing loop).
   ! Uses instantaneous water fluxes, not time-averaged, so tracer and water
   ! advance in lockstep.  This avoids the "water left but tracer stayed"
   ! artefact of the old single-step-per-period approach.
   !-------------------------------------------------------------------------------------
   SUBROUTINE tracer_substep (dt_ref, dt_all, irivsys, hflux_fc, sum_hflux_riv, wdsrf, ucatfilter, &
      volresv, ucat2resv, is_built_resv, &
      do_bif, bif_hflux_lev_in, npthout_local_in)

   USE, INTRINSIC :: ieee_arithmetic, only: ieee_is_finite
   USE MOD_Grid_RiverLakeNetwork, only: numucat, totalnumucat, ucat_next, &
      floodplain_curve, lake_type, push_ups2ucat, push_next2ucat, &
      npthlev_bif, pth_upst_local, pth_down_local, &
      push_bif_influx, push_bif_dn2pth, rivsys_by_multiple_procs
#ifdef USEMPI
   USE MOD_Grid_RiverLakeNetwork, only: p_comm_rivsys
#endif
   USE MOD_Grid_RiverLakeLevee, only: has_levee, levsto
   USE MOD_Grid_RiverLakeTimeVars, only: volwater_ucat
   USE MOD_WorkerPushData
   USE MOD_Tracer_Defs, only: trc_tiny, &
      tracer_init_water_ratio, tracer_reactive_decay_fraction
   IMPLICIT NONE

   real(r8), intent(in) :: dt_ref
   real(r8), intent(in) :: dt_all(:)
   integer,  intent(in) :: irivsys(:)
   real(r8), intent(in) :: hflux_fc(:)
   real(r8), intent(in) :: sum_hflux_riv(:)
   real(r8), intent(in) :: wdsrf(:)
   logical,  intent(in) :: ucatfilter(:)
   real(r8), intent(in) :: volresv(:)
   integer,  intent(in) :: ucat2resv(:)
   logical,  intent(in) :: is_built_resv(:)
   logical,  intent(in), optional :: do_bif
   real(r8), intent(in), optional :: bif_hflux_lev_in(:,:)
   integer,  intent(in), optional :: npthout_local_in

   integer  :: i, itrc, ipth, i_up, i_dn, ilev, limiter_iter, limiter_max_iter
   real(r8) :: volwater, volflux, dt_i, dt_donor, trc_pth_fl
   real(r8) :: layer_wflux, trc_rate, limiter_rate_new
   real(r8) :: limiter_delta, limiter_delta_global
   logical  :: upstream_has_levee, downstream_has_levee, can_use_levee_tracer
   logical  :: limiter_converged
   real(r8) :: release, R_fill
   real(r8) :: trc_mass_new
   real(r8) :: decay_fraction, reactive_src
   logical  :: bif_workspace_active
   integer  :: npth_bif, nlev_bif

      npth_bif = 0
      nlev_bif = 0
      bif_workspace_active = .false.
      IF (present(do_bif) .and. present(bif_hflux_lev_in) .and. present(npthout_local_in)) THEN
         npth_bif = npthout_local_in
         nlev_bif = size(bif_hflux_lev_in, 1)
         bif_workspace_active = do_bif
      ENDIF

      ! A Jacobi fixed point can move donor-rate information only one graph
      ! edge per iteration. The water-side BIF aggregate limiter reserves
      ! ordinary gross outflow before authorising BIF outflow, separately for
      ! visible/protected pools. With the same well-mixed concentration used
      ! here, every nonzero BIF sender therefore starts at tracer rate one;
      ! sub-unit-rate dependencies propagate only along the ordinary river
      ! forest. Visible/protected pools give at most 2*N vertices, so 2*N+1 is
      ! a conservative topology-derived bound rather than an arbitrary cycle
      ! iteration budget. A material residual at that bound is fatal.
      IF (totalnumucat < 0 .or. totalnumucat > (huge(limiter_max_iter) - 1) / 2) &
         CALL CoLM_stop('invalid river topology size for tracer donor limiter')
      limiter_max_iter = 2 * totalnumucat + 1

      IF (numucat <= 0 .and. npth_bif <= 0 .and. .not. bif_workspace_active) RETURN

      CALL ensure_tracer_substep_workspace(numucat, npth_bif, nlev_bif, bif_workspace_active)

      can_use_levee_tracer = DEF_USE_LEVEE .and. allocated(has_levee) .and. &
         allocated(trc_levsto) .and. allocated(levsto)

      IF (bif_workspace_active) THEN
         has_levee_r8(:) = 0._r8
         IF (can_use_levee_tracer) THEN
            DO i = 1, numucat
               IF (i <= size(has_levee)) THEN
                  IF (has_levee(i)) has_levee_r8(i) = 1._r8
               ENDIF
            ENDDO
         ENDIF
         CALL worker_push_data (push_bif_dn2pth, has_levee_r8, has_levee_dn_pth, fillvalue = 0._r8)
      ENDIF
      IF (bif_workspace_active) THEN
         dt_ucat(:) = 0._r8
         DO i = 1, numucat
            IF (irivsys(i) > 0 .and. irivsys(i) <= size(dt_all)) dt_ucat(i) = dt_all(irivsys(i))
         ENDDO
         CALL worker_push_data (push_bif_dn2pth, dt_ucat, dt_dn_pth, fillvalue = 0._r8)
      ENDIF

         DO itrc = 1, ntracers
            IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
            R_fill = tracer_init_water_ratio(itrc)

         ! --- 1. Concentration from pre-update single-pool state ---
         DO i = 1, numucat
            CALL get_cell_volume(i, wdsrf(i), volresv, ucat2resv, volwater)
            dt_i = 0._r8
            IF (irivsys(i) > 0 .and. irivsys(i) <= size(dt_all)) dt_i = dt_all(irivsys(i))
            ! The restart/state contract admits only sub-tolerance negative
            ! roundoff. Canonicalize it before forming a concentration: a
            ! negative flux would violate the limiter's nonnegative monotone
            ! map even if the underlying mass error is numerically tiny.
            IF (.not. ieee_is_finite(trc_mass(itrc, i))) THEN
               CALL CoLM_stop('non-finite river tracer mass before transport')
            ELSEIF (trc_mass(itrc, i) < -TRC_RESTART_NEGATIVE_DUST) THEN
               CALL CoLM_stop('negative river tracer mass before transport')
            ELSEIF (trc_mass(itrc, i) < 0._r8) THEN
               trc_mass(itrc, i) = 0._r8
            ENDIF
            IF (allocated(trc_levsto)) THEN
               IF (i <= size(trc_levsto, 2)) THEN
                  IF (.not. ieee_is_finite(trc_levsto(itrc, i))) THEN
                     CALL CoLM_stop('non-finite protected tracer mass before transport')
                  ELSEIF (trc_levsto(itrc, i) < -TRC_RESTART_NEGATIVE_DUST) THEN
                     CALL CoLM_stop('negative protected tracer mass before transport')
                  ELSEIF (trc_levsto(itrc, i) < 0._r8) THEN
                     trc_levsto(itrc, i) = 0._r8
                  ENDIF
               ENDIF
            ENDIF
            ! Release every positive pending input in the first substep,
            ! even when the pre-update cell is dry. Water can arrive during
            ! this same substep; if it remains dry, the post-update dry-drain
            ! path removes the orphan mass. Signed debt stays in the buffer
            ! to cancel later same-cell runoff rather than borrowing from
            ! the current river inventory.
            release = max(trc_inp_buf(itrc, i), 0._r8)
            trc_inp_buf(itrc, i) = trc_inp_buf(itrc, i) - release
            trc_mass(itrc, i) = trc_mass(itrc, i) + release
            CALL update_tracer_concentration(itrc, i, volwater)
            IF (volwater > trc_v_dry_off) THEN
               ! Use the actual well-mixed pool concentration. Inflating the
               ! denominator to an outgoing edge volume breaks the constant-
               ! state invariant whenever same-step inflow balances gross
               ! outflow; the coupled donor limiter now handles that demand.
               trc_conc_flux(i) = trc_mass(itrc, i) / volwater
            ELSE
               ! A dry cell has no physical concentration. Retain the bounded
               ! edge-volume fallback only to transport explicitly queued mass
               ! without division by zero; the limiter remains authoritative.
               volflux = max(abs(hflux_fc(i)) * dt_i, trc_v_dry_off)
               trc_conc_flux(i) = trc_mass(itrc, i) / volflux
            ENDIF
            trc_prot_conc_flux(i) = trc_conc_flux(i)
            IF (can_use_levee_tracer) THEN
               IF (i <= size(has_levee) .and. i <= size(levsto) .and. i <= size(trc_levsto, 2)) THEN
                  IF (has_levee(i)) THEN
                     IF (.not. ieee_is_finite(levsto(i)) .or. levsto(i) < 0._r8) &
                        CALL CoLM_stop('invalid protected water storage before tracer transport')
                     ! A real protected compartment keeps its own signature even
                     ! below the dry threshold. Falling back to visible C here
                     ! can make protected tracer demand exceed protected mass
                     ! and creates a spurious sub-unit BIF donor dependency.
                     IF (levsto(i) > 0._r8) THEN
                        ! Use the true pool volume even below the diagnostic dry
                        ! threshold. The water limiter guarantees Q*dt<=V, so a
                        ! complete tiny-pool drain exports exactly its tracer mass
                        ! instead of leaving an orphan inventory behind.
                        trc_prot_conc_flux(i) = trc_levsto(itrc, i) / levsto(i)
                        IF (.not. ieee_is_finite(trc_prot_conc_flux(i))) &
                           CALL CoLM_stop('non-finite protected tracer concentration')
                     ELSE
                        trc_prot_conc_flux(i) = 0._r8
                     ENDIF
                  ENDIF
               ENDIF
            ENDIF
         ENDDO

         ! --- 3. Get downstream concentration (main channel) ---
         CALL worker_push_data (push_next2ucat, trc_conc_flux, conc_next, fillvalue = R_fill)

         ! --- 4. Upwind main-channel tracer flux ---
         DO i = 1, numucat
            IF (.not. ucatfilter(i)) THEN
               trc_flux(i) = 0._r8
               CYCLE
            ENDIF
            IF (hflux_fc(i) >= 0._r8) THEN
               trc_flux(i) = trc_conc_flux(i) * hflux_fc(i)
            ELSE
               trc_flux(i) = conc_next(i) * hflux_fc(i)
            ENDIF
         ENDDO

         ! --- 5. Aggregate upstream tracer flux ---
         CALL worker_push_data (push_ups2ucat, trc_flux, flux_ups, fillvalue = 0._r8, mode = 'sum')

         ! --- 6. Bifurcation pathway tracer flux ---
         bif_net(:) = 0._r8
         trc_bif_lev_net(:) = 0._r8
         IF (bif_workspace_active) THEN
            CALL worker_push_data (push_bif_dn2pth, trc_conc_flux, conc_dn_pth, fillvalue = 0._r8)
            CALL worker_push_data (push_bif_dn2pth, trc_prot_conc_flux, trc_prot_conc_dn_pth, fillvalue = 0._r8)

            trc_pth_1trc(:) = 0._r8
            trc_pth_lev(:) = 0._r8
            trc_pth_levtrc(:,:) = 0._r8
            DO ipth = 1, npth_bif
               i_up = pth_upst_local(ipth)
               IF (i_up < 1 .or. i_up > numucat) CYCLE
               IF (.not. ucatfilter(i_up)) CYCLE

               i_dn = pth_down_local(ipth)
               CALL tracer_bif_path_levee_sides(can_use_levee_tracer, i_up, i_dn, ipth, numucat, &
                  upstream_has_levee, downstream_has_levee)

               DO ilev = 1, nlev_bif
                  layer_wflux = bif_hflux_lev_in(ilev, ipth)
                  IF (abs(layer_wflux) <= trc_tiny) CYCLE

                  IF (layer_wflux >= 0._r8) THEN
                     IF (ilev > 1 .and. upstream_has_levee) THEN
                        trc_pth_fl = trc_prot_conc_flux(i_up) * layer_wflux
                     ELSE
                        trc_pth_fl = trc_conc_flux(i_up) * layer_wflux
                     ENDIF
                  ELSE
                     IF (ilev > 1 .and. downstream_has_levee) THEN
                        trc_pth_fl = trc_prot_conc_dn_pth(ipth) * layer_wflux
                     ELSE
                        trc_pth_fl = conc_dn_pth(ipth) * layer_wflux
                     ENDIF
                  ENDIF
                  trc_pth_levtrc(ilev, ipth) = trc_pth_fl

                  IF (ilev > 1 .and. upstream_has_levee) THEN
                     trc_bif_lev_net(i_up) = trc_bif_lev_net(i_up) + trc_pth_fl
                  ELSE
                     bif_net(i_up) = bif_net(i_up) + trc_pth_fl
                  ENDIF

                  IF (i_dn > 0 .and. i_dn <= numucat) THEN
                     IF (ilev > 1 .and. downstream_has_levee) THEN
                        trc_bif_lev_net(i_dn) = trc_bif_lev_net(i_dn) - trc_pth_fl
                     ELSE
                        bif_net(i_dn) = bif_net(i_dn) - trc_pth_fl
                     ENDIF
                  ELSE
                     IF (ilev > 1 .and. downstream_has_levee) THEN
                        trc_pth_lev(ipth) = trc_pth_lev(ipth) + trc_pth_fl
                     ELSE
                        trc_pth_1trc(ipth) = trc_pth_1trc(ipth) + trc_pth_fl
                     ENDIF
                  ENDIF
               ENDDO
            ENDDO

            ! Remote bif scatter.  trc_pth_* keep the upstream-sign
            ! pathway flux; the downstream cell's net contribution is
            ! therefore subtracted after push.
            CALL worker_push_data (push_bif_influx, trc_pth_1trc, bif_recv, &
               fillvalue = 0._r8, mode = 'sum')
            CALL worker_push_data (push_bif_influx, trc_pth_lev, bif_lev_recv, &
               fillvalue = 0._r8, mode = 'sum')
            DO i = 1, numucat
               bif_net(i) = bif_net(i) - bif_recv(i)
               trc_bif_lev_net(i) = trc_bif_lev_net(i) - bif_lev_recv(i)
            ENDDO
         ENDIF

         ! --- 7. Coupled mass-based flux limiter ---
         ! Water CFL limits NET storage change, while a tracer donor can have
         ! several simultaneous outgoing faces.  Build the raw gross donor
         ! demand first; the coupled iteration below authorises it only from
         ! mass that upstream donor rates actually deliver in this substep.

         ! 7a: P2STOOUT — total outgoing |tracer| per cell (water-direction)
         ! Guard dt_all(irivsys(i)) the same way as section 0/8 so a
         ! corrupt/uninitialised irivsys doesn't segfault here even if the
         ! water-flow main loop happens to skip it via ucatfilter. See
         ! MOD_Grid_RiverLakeFlow.F90:585 for the primary defense.
         DO i = 1, numucat
            IF (hflux_fc(i) >= 0._r8 .and. &
                irivsys(i) > 0 .and. irivsys(i) <= size(dt_all)) THEN
               trc_out_mass(i) = abs(trc_flux(i)) * dt_all(irivsys(i))
            ELSE
               trc_out_mass(i) = 0._r8
            ENDIF
         ENDDO
         ! Reverse flow: downstream cell is donor → push to its P2STOOUT.
         ! A normal river edge cannot cross river systems: Network assigns an
         ! upstream cell the same rivermouth label as ucat_next, and Flow uses
         ! one dt_all entry per label (reduced on p_comm_rivsys when split).
         ! Therefore this edge/receiver dt is also the downstream donor dt.
         DO i = 1, numucat
            IF (hflux_fc(i) < 0._r8 .and. &
                irivsys(i) > 0 .and. irivsys(i) <= size(dt_all)) THEN
               rate_cell(i) = abs(trc_flux(i)) * dt_all(irivsys(i))
            ELSE
               rate_cell(i) = 0._r8
            ENDIF
         ENDDO
         CALL worker_push_data (push_ups2ucat, rate_cell, rate_next, fillvalue = 0._r8, mode = 'sum')
         DO i = 1, numucat
            trc_out_mass(i) = trc_out_mass(i) + rate_next(i)
            trc_out_mass_lev(i) = 0._r8
         ENDDO
         ! Bif pathways: add sender-side contribution
         IF (bif_workspace_active) THEN
            trc_dn_out_vis_pth(:) = 0._r8
            trc_dn_out_lev_pth(:) = 0._r8
            DO ipth = 1, npth_bif
               i_up = pth_upst_local(ipth)
               IF (i_up < 1 .or. i_up > numucat) CYCLE
               IF (.not. (irivsys(i_up) > 0 .and. irivsys(i_up) <= size(dt_all))) CYCLE
               dt_i = dt_all(irivsys(i_up))
               i_dn = pth_down_local(ipth)
               CALL tracer_bif_path_levee_sides(can_use_levee_tracer, i_up, i_dn, ipth, numucat, &
                  upstream_has_levee, downstream_has_levee)

               DO ilev = 1, nlev_bif
                  layer_wflux = bif_hflux_lev_in(ilev, ipth)
                  IF (abs(layer_wflux) <= trc_tiny) CYCLE
                  trc_pth_fl = trc_pth_levtrc(ilev, ipth)
                  IF (layer_wflux >= 0._r8) THEN
                     IF (ilev > 1 .and. upstream_has_levee) THEN
                        trc_out_mass_lev(i_up) = trc_out_mass_lev(i_up) + abs(trc_pth_fl) * dt_i
                     ELSE
                        trc_out_mass(i_up) = trc_out_mass(i_up) + abs(trc_pth_fl) * dt_i
                     ENDIF
                  ELSE
                     dt_donor = 0._r8
                     IF (i_dn > 0 .and. i_dn <= numucat) THEN
                        IF (irivsys(i_dn) > 0 .and. irivsys(i_dn) <= size(dt_all)) &
                           dt_donor = dt_all(irivsys(i_dn))
                     ELSEIF (bif_workspace_active) THEN
                        dt_donor = dt_dn_pth(ipth)
                     ENDIF
                     IF (dt_donor <= 0._r8) CYCLE
                     IF (ilev > 1 .and. downstream_has_levee) THEN
                        trc_dn_out_lev_pth(ipth) = trc_dn_out_lev_pth(ipth) + abs(trc_pth_fl) * dt_donor
                     ELSE
                        trc_dn_out_vis_pth(ipth) = trc_dn_out_vis_pth(ipth) + abs(trc_pth_fl) * dt_donor
                     ENDIF
                  ENDIF
               ENDDO
            ENDDO
            CALL worker_push_data (push_bif_influx, trc_dn_out_vis_pth, trc_dn_out_vis_recv, &
               fillvalue = 0._r8, mode = 'sum')
            CALL worker_push_data (push_bif_influx, trc_dn_out_lev_pth, trc_dn_out_lev_recv, &
               fillvalue = 0._r8, mode = 'sum')
            DO i = 1, numucat
               trc_out_mass(i) = trc_out_mass(i) + trc_dn_out_vis_recv(i)
               trc_out_mass_lev(i) = trc_out_mass_lev(i) + trc_dn_out_lev_recv(i)
            ENDDO
         ENDIF

         ! 7b: D2RATE fixed point per cell
         ! Start at T(0): only pre-substep donor mass is authorised.  This is
         ! the first iterate of a monotone-from-zero fixed point, not a guess
         ! based on unscaled same-step inflow.  Given a conservative old rate,
         ! T(old) is also conservative because its larger rate can only increase
         ! every receiver's incoming mass.  Therefore every iterate is safe;
         ! convergence recovers same-step pass-through without ever permitting
         ! a downstream cell to spend inflow that its upstream donor later cuts.
         DO i = 1, numucat
            IF (.not. ieee_is_finite(trc_mass(itrc, i)) .or. &
                .not. ieee_is_finite(trc_out_mass(i)) .or. &
                .not. ieee_is_finite(trc_out_mass_lev(i))) &
               CALL CoLM_stop('non-finite river tracer donor limiter state')
            IF (trc_mass(itrc, i) < -TRC_RESTART_NEGATIVE_DUST) &
               CALL CoLM_stop('negative river tracer mass entered donor limiter')
            IF (trc_out_mass(i) < 0._r8 .or. trc_out_mass_lev(i) < 0._r8) &
               CALL CoLM_stop('negative river tracer gross outflow demand')

            IF (trc_out_mass(i) > TRC_LIMITER_OUT_TINY) THEN
               rate_cell(i) = min(max(trc_mass(itrc, i), 0._r8) / trc_out_mass(i), 1._r8)
            ELSE
               rate_cell(i) = 1._r8
            ENDIF
            IF (trc_out_mass_lev(i) > TRC_LIMITER_OUT_TINY) THEN
               IF (.not. allocated(trc_levsto)) THEN
                  CALL CoLM_stop('protected tracer outflow has no protected donor pool')
               ELSEIF (i > size(trc_levsto, 2)) THEN
                  CALL CoLM_stop('protected tracer outflow has no protected donor pool')
               ELSE
                  IF (.not. ieee_is_finite(trc_levsto(itrc, i))) &
                     CALL CoLM_stop('non-finite protected tracer donor limiter state')
                  IF (trc_levsto(itrc, i) < -TRC_RESTART_NEGATIVE_DUST) &
                     CALL CoLM_stop('negative protected tracer mass entered donor limiter')
                  rate_cell_lev(i) = min(max(trc_levsto(itrc, i), 0._r8) / trc_out_mass_lev(i), 1._r8)
               ENDIF
            ELSE
               rate_cell_lev(i) = 1._r8
            ENDIF
         ENDDO

         ! Skip communication entirely when T(0)==1 everywhere.  BIF mode is
         ! globally lock-stepped and includes empty workers.  Without BIF,
         ! independent river systems can take different numbers of substeps;
         ! only a river system actually split across ranks may reduce here.
         limiter_delta = 0._r8
         DO i = 1, numucat
            limiter_delta = max(limiter_delta, 1._r8 - rate_cell(i), &
               1._r8 - rate_cell_lev(i))
         ENDDO
         limiter_delta_global = limiter_delta
#ifdef USEMPI
         IF (bif_workspace_active) THEN
            CALL mpi_allreduce(limiter_delta, limiter_delta_global, 1, MPI_REAL8, MPI_MAX, &
               p_comm_worker, p_err)
         ELSEIF (rivsys_by_multiple_procs) THEN
            CALL mpi_allreduce(limiter_delta, limiter_delta_global, 1, MPI_REAL8, MPI_MAX, &
               p_comm_rivsys, p_err)
         ENDIF
#endif
         IF (.not. ieee_is_finite(limiter_delta_global)) &
            CALL CoLM_stop('non-finite river tracer donor limiter residual')
         limiter_converged = limiter_delta_global <= TRC_LIMITER_RATE_TOL

         DO limiter_iter = 1, limiter_max_iter
            IF (limiter_converged) EXIT

            ! Main-channel actual incoming mass under the current sender rates.
            CALL worker_push_data(push_next2ucat, rate_cell, rate_next, fillvalue = 1._r8)
            trc_in_mass(:) = 0._r8
            trc_in_mass_lev(:) = 0._r8
            trc_inp_step(:) = 0._r8
            DO i = 1, numucat
               dt_i = 0._r8
               IF (irivsys(i) > 0 .and. irivsys(i) <= size(dt_all)) dt_i = dt_all(irivsys(i))
               IF (dt_i <= 0._r8) CYCLE
               IF (hflux_fc(i) >= 0._r8) THEN
                  trc_inp_step(i) = max(trc_flux(i) * rate_cell(i), 0._r8)
               ELSE
                  trc_in_mass(i) = trc_in_mass(i) + &
                     max(-trc_flux(i) * rate_next(i), 0._r8) * dt_i
               ENDIF
            ENDDO
            CALL worker_push_data(push_ups2ucat, trc_inp_step, flux_ups, &
               fillvalue = 0._r8, mode = 'sum')
            DO i = 1, numucat
               dt_i = 0._r8
               IF (irivsys(i) > 0 .and. irivsys(i) <= size(dt_all)) dt_i = dt_all(irivsys(i))
               IF (dt_i > 0._r8) trc_in_mass(i) = trc_in_mass(i) + max(flux_ups(i), 0._r8) * dt_i
            ENDDO

            ! Bifurcation actual incoming mass.  Rebuild gross receiver-side
            ! amounts path by path: signed net BIF flux can hide simultaneous
            ! incoming and outgoing transfers at the same cell.
            IF (bif_workspace_active) THEN
               CALL worker_push_data(push_bif_dn2pth, rate_cell, rate_dn_pth, fillvalue = 1._r8)
               CALL worker_push_data(push_bif_dn2pth, rate_cell_lev, rate_dn_pth_lev, fillvalue = 1._r8)
               trc_pth_1trc(:) = 0._r8
               trc_pth_lev(:) = 0._r8
               DO ipth = 1, npth_bif
                  i_up = pth_upst_local(ipth)
                  IF (i_up < 1 .or. i_up > numucat) CYCLE
                  IF (.not. (irivsys(i_up) > 0 .and. irivsys(i_up) <= size(dt_all))) CYCLE
                  dt_i = dt_all(irivsys(i_up))
                  IF (dt_i <= 0._r8) CYCLE

                  i_dn = pth_down_local(ipth)
                  CALL tracer_bif_path_levee_sides(can_use_levee_tracer, i_up, i_dn, ipth, numucat, &
                     upstream_has_levee, downstream_has_levee)

                  DO ilev = 1, nlev_bif
                     layer_wflux = bif_hflux_lev_in(ilev, ipth)
                     trc_pth_fl = trc_pth_levtrc(ilev, ipth)
                     IF (abs(layer_wflux) <= trc_tiny .or. abs(trc_pth_fl) <= trc_tiny) CYCLE

                     IF (layer_wflux >= 0._r8) THEN
                        IF (ilev > 1 .and. upstream_has_levee) THEN
                           trc_rate = rate_cell_lev(i_up)
                        ELSE
                           trc_rate = rate_cell(i_up)
                        ENDIF
                        trc_pth_fl = trc_pth_fl * trc_rate
                        ! upstream -> downstream: receiver is i_dn.
                        dt_donor = 0._r8
                        IF (i_dn > 0 .and. i_dn <= numucat) THEN
                           IF (irivsys(i_dn) > 0 .and. irivsys(i_dn) <= size(dt_all)) &
                              dt_donor = dt_all(irivsys(i_dn))
                           IF (dt_donor <= 0._r8) CYCLE
                           IF (ilev > 1 .and. downstream_has_levee) THEN
                              trc_in_mass_lev(i_dn) = trc_in_mass_lev(i_dn) + &
                                 max(trc_pth_fl, 0._r8) * dt_donor
                           ELSE
                              trc_in_mass(i_dn) = trc_in_mass(i_dn) + max(trc_pth_fl, 0._r8) * dt_donor
                           ENDIF
                        ELSE
                           dt_donor = dt_dn_pth(ipth)
                           IF (dt_donor <= 0._r8) CYCLE
                           IF (ilev > 1 .and. downstream_has_levee) THEN
                              trc_pth_lev(ipth) = trc_pth_lev(ipth) + max(trc_pth_fl, 0._r8) * dt_donor
                           ELSE
                              trc_pth_1trc(ipth) = trc_pth_1trc(ipth) + max(trc_pth_fl, 0._r8) * dt_donor
                           ENDIF
                        ENDIF
                     ELSE
                        IF (ilev > 1 .and. downstream_has_levee) THEN
                           trc_rate = rate_dn_pth_lev(ipth)
                        ELSE
                           trc_rate = rate_dn_pth(ipth)
                        ENDIF
                        trc_pth_fl = trc_pth_fl * trc_rate
                        ! downstream -> upstream: receiver is local i_up.
                        IF (ilev > 1 .and. upstream_has_levee) THEN
                           trc_in_mass_lev(i_up) = trc_in_mass_lev(i_up) + max(-trc_pth_fl, 0._r8) * dt_i
                        ELSE
                           trc_in_mass(i_up) = trc_in_mass(i_up) + max(-trc_pth_fl, 0._r8) * dt_i
                        ENDIF
                     ENDIF
                  ENDDO
               ENDDO
               CALL worker_push_data(push_bif_influx, trc_pth_1trc, bif_recv, &
                  fillvalue = 0._r8, mode = 'sum')
               CALL worker_push_data(push_bif_influx, trc_pth_lev, bif_lev_recv, &
                  fillvalue = 0._r8, mode = 'sum')
               DO i = 1, numucat
                  trc_in_mass(i) = trc_in_mass(i) + max(bif_recv(i), 0._r8)
                  trc_in_mass_lev(i) = trc_in_mass_lev(i) + max(bif_lev_recv(i), 0._r8)
               ENDDO
            ENDIF

            limiter_delta = 0._r8
            DO i = 1, numucat
               IF (.not. ieee_is_finite(trc_in_mass(i)) .or. &
                   .not. ieee_is_finite(trc_in_mass_lev(i)) .or. &
                   trc_in_mass(i) < 0._r8 .or. trc_in_mass_lev(i) < 0._r8) &
                  CALL CoLM_stop('invalid actual incoming mass in river tracer donor limiter')

               IF (trc_out_mass(i) > TRC_LIMITER_OUT_TINY) THEN
                  limiter_rate_new = min((max(trc_mass(itrc, i), 0._r8) + trc_in_mass(i)) / &
                     trc_out_mass(i), 1._r8)
                  IF (.not. ieee_is_finite(limiter_rate_new)) &
                     CALL CoLM_stop('non-finite visible river tracer donor rate')
                  IF (limiter_rate_new + TRC_LIMITER_RATE_TOL < rate_cell(i)) &
                     CALL CoLM_stop('river tracer donor limiter lost monotonicity')
                  limiter_rate_new = max(rate_cell(i), limiter_rate_new)
                  limiter_delta = max(limiter_delta, limiter_rate_new - rate_cell(i))
                  rate_cell(i) = limiter_rate_new
               ENDIF

               IF (trc_out_mass_lev(i) > TRC_LIMITER_OUT_TINY) THEN
                  limiter_rate_new = min((max(trc_levsto(itrc, i), 0._r8) + trc_in_mass_lev(i)) / &
                     trc_out_mass_lev(i), 1._r8)
                  IF (.not. ieee_is_finite(limiter_rate_new)) &
                     CALL CoLM_stop('non-finite protected river tracer donor rate')
                  IF (limiter_rate_new + TRC_LIMITER_RATE_TOL < rate_cell_lev(i)) &
                     CALL CoLM_stop('protected tracer donor limiter lost monotonicity')
                  limiter_rate_new = max(rate_cell_lev(i), limiter_rate_new)
                  limiter_delta = max(limiter_delta, limiter_rate_new - rate_cell_lev(i))
                  rate_cell_lev(i) = limiter_rate_new
               ENDIF
            ENDDO

            limiter_delta_global = limiter_delta
#ifdef USEMPI
            IF (bif_workspace_active) THEN
               CALL mpi_allreduce(limiter_delta, limiter_delta_global, 1, MPI_REAL8, MPI_MAX, &
                  p_comm_worker, p_err)
            ELSEIF (rivsys_by_multiple_procs) THEN
               CALL mpi_allreduce(limiter_delta, limiter_delta_global, 1, MPI_REAL8, MPI_MAX, &
                  p_comm_rivsys, p_err)
            ENDIF
#endif
            IF (.not. ieee_is_finite(limiter_delta_global)) &
               CALL CoLM_stop('non-finite river tracer donor limiter residual')
            limiter_converged = limiter_delta_global <= TRC_LIMITER_RATE_TOL
         ENDDO

         IF (.not. limiter_converged) THEN
            WRITE(*,'(A,I0,A,I0,A,ES12.4)') 'ERROR tracer donor limiter: tracer=', itrc, &
               ' iterations=', limiter_max_iter, ' residual=', limiter_delta_global
            CALL CoLM_stop('river tracer donor limiter did not converge')
         ENDIF

         ! 7c: Get downstream cell rate (for reverse main flow)
         CALL worker_push_data (push_next2ucat, rate_cell, rate_next, fillvalue = 1._r8)

         ! 7d: Scale main-channel flux by sender rate (water direction)
         DO i = 1, numucat
            IF (hflux_fc(i) >= 0._r8) THEN
               trc_flux(i) = trc_flux(i) * rate_cell(i)
            ELSE
               trc_flux(i) = trc_flux(i) * rate_next(i)
            ENDIF
         ENDDO

         ! 7e: Scale bif pathway flux by sender rate (water direction)
         IF (bif_workspace_active) THEN
            CALL worker_push_data (push_bif_dn2pth, rate_cell, rate_dn_pth, fillvalue = 1._r8)
            CALL worker_push_data (push_bif_dn2pth, rate_cell_lev, rate_dn_pth_lev, fillvalue = 1._r8)
            DO ipth = 1, npth_bif
               i_up = pth_upst_local(ipth)
               IF (i_up < 1 .or. i_up > numucat) CYCLE
               i_dn = pth_down_local(ipth)
               CALL tracer_bif_path_levee_sides(can_use_levee_tracer, i_up, i_dn, ipth, numucat, &
                  upstream_has_levee, downstream_has_levee)
               DO ilev = 1, nlev_bif
                  layer_wflux = bif_hflux_lev_in(ilev, ipth)
                  IF (abs(layer_wflux) <= trc_tiny) CYCLE
                  IF (layer_wflux >= 0._r8) THEN
                     IF (ilev > 1 .and. upstream_has_levee) THEN
                        trc_rate = rate_cell_lev(i_up)
                     ELSE
                        trc_rate = rate_cell(i_up)
                     ENDIF
                  ELSE
                     IF (ilev > 1 .and. downstream_has_levee) THEN
                        trc_rate = rate_dn_pth_lev(ipth)
                     ELSE
                        trc_rate = rate_dn_pth(ipth)
                     ENDIF
                  ENDIF
                  trc_pth_levtrc(ilev, ipth) = trc_pth_levtrc(ilev, ipth) * trc_rate
               ENDDO
            ENDDO
            ! Rebuild visible/protected bif nets from scaled layer fluxes.
            bif_net(:) = 0._r8
            trc_bif_lev_net(:) = 0._r8
            trc_pth_1trc(:) = 0._r8
            trc_pth_lev(:) = 0._r8
            DO ipth = 1, npth_bif
               i_up = pth_upst_local(ipth)
               IF (i_up < 1 .or. i_up > numucat) CYCLE
               i_dn = pth_down_local(ipth)
               CALL tracer_bif_path_levee_sides(can_use_levee_tracer, i_up, i_dn, ipth, numucat, &
                  upstream_has_levee, downstream_has_levee)

               DO ilev = 1, nlev_bif
                  trc_pth_fl = trc_pth_levtrc(ilev, ipth)
                  IF (abs(trc_pth_fl) <= trc_tiny) CYCLE
                  IF (ilev > 1 .and. upstream_has_levee) THEN
                     trc_bif_lev_net(i_up) = trc_bif_lev_net(i_up) + trc_pth_fl
                  ELSE
                     bif_net(i_up) = bif_net(i_up) + trc_pth_fl
                  ENDIF

                  IF (i_dn > 0 .and. i_dn <= numucat) THEN
                     IF (ilev > 1 .and. downstream_has_levee) THEN
                        trc_bif_lev_net(i_dn) = trc_bif_lev_net(i_dn) - trc_pth_fl
                     ELSE
                        bif_net(i_dn) = bif_net(i_dn) - trc_pth_fl
                     ENDIF
                  ELSE
                     IF (ilev > 1 .and. downstream_has_levee) THEN
                        trc_pth_lev(ipth) = trc_pth_lev(ipth) + trc_pth_fl
                     ELSE
                        trc_pth_1trc(ipth) = trc_pth_1trc(ipth) + trc_pth_fl
                     ENDIF
                  ENDIF
               ENDDO
            ENDDO
            CALL worker_push_data (push_bif_influx, trc_pth_1trc, bif_recv, &
               fillvalue = 0._r8, mode = 'sum')
            CALL worker_push_data (push_bif_influx, trc_pth_lev, bif_lev_recv, &
               fillvalue = 0._r8, mode = 'sum')
            DO i = 1, numucat
               bif_net(i) = bif_net(i) - bif_recv(i)
               trc_bif_lev_net(i) = trc_bif_lev_net(i) - bif_lev_recv(i)
            ENDDO
         ENDIF

         ! Re-aggregate upstream tracer flux after limiting
         CALL worker_push_data (push_ups2ucat, trc_flux, flux_ups, fillvalue = 0._r8, mode = 'sum')

         ! --- 8. Update tracer mass ---
         DO i = 1, numucat
            IF (.not. ucatfilter(i)) CYCLE
            dt_i = 0._r8
            IF (irivsys(i) > 0 .and. irivsys(i) <= size(dt_all)) dt_i = dt_all(irivsys(i))
            IF (dt_i <= 0._r8) CYCLE

            trc_mass_new = trc_mass(itrc, i) &
               + (- trc_flux(i) + flux_ups(i) - bif_net(i)) * dt_i
            IF (.not. ieee_is_finite(trc_mass_new)) &
               CALL CoLM_stop('non-finite river tracer mass after coupled donor limiter')
            IF (trc_mass_new < -TRC_RESTART_NEGATIVE_DUST) &
               CALL CoLM_stop('negative river tracer mass after coupled donor limiter')

            ! Transport owns only conservative edge transfers.  Never snap the
            ! result back to R_fill: doing so is an unbooked source/sink and can
            ! erase a legitimate restart or upstream isotope gradient.
            trc_mass(itrc, i) = max(trc_mass_new, 0._r8)
            IF (allocated(trc_levsto)) THEN
               IF (i <= size(trc_levsto, 2)) THEN
                  trc_levsto(itrc, i) = trc_levsto(itrc, i) - trc_bif_lev_net(i) * dt_i
                  IF (.not. ieee_is_finite(trc_levsto(itrc, i))) &
                     CALL CoLM_stop('non-finite protected tracer mass after coupled donor limiter')
                  IF (trc_levsto(itrc, i) < -TRC_RESTART_NEGATIVE_DUST) &
                     CALL CoLM_stop('negative protected tracer mass after coupled donor limiter')
                  trc_levsto(itrc, i) = max(trc_levsto(itrc, i), 0._r8)
               ENDIF
            ENDIF

            decay_fraction = tracer_reactive_decay_fraction(itrc, dt_i)
            IF (decay_fraction > 0._r8) THEN
               reactive_src = 0._r8
               CALL decay_river_pool(trc_mass(itrc, i), decay_fraction, reactive_src)
               CALL decay_river_pool(trc_inp_buf(itrc, i), decay_fraction, reactive_src)
               IF (allocated(trc_levsto)) THEN
                  IF (i <= size(trc_levsto, 2)) THEN
                     CALL decay_river_pool(trc_levsto(itrc, i), decay_fraction, reactive_src)
                  ENDIF
               ENDIF
               IF (allocated(trc_reactive_source)) THEN
                  trc_reactive_source(itrc, i) = trc_reactive_source(itrc, i) + reactive_src
               ENDIF
            ENDIF
         ENDDO

         ! --- 9. Save flux for diagnostics (only for active cells) ---
         DO i = 1, numucat
            IF (ucatfilter(i)) THEN
               trc_flux_out(itrc, i) = trc_flux(i)
               trc_bif_net_saved(itrc, i) = bif_net(i) + trc_bif_lev_net(i)
            ENDIF
         ENDDO

      ENDDO  ! itrc

      ! --- 10. Final concentration from the pre-water-update state ---
      ! Dry-cell drain is handled after the water update in
      ! tracer_diag_accumulate_substep. Draining here would erase tracer
      ! delivered into a cell that is dry at the start of the substep but
      ! becomes wet after the hydrologic volume update.
      DO i = 1, numucat
         CALL get_cell_volume(i, wdsrf(i), volresv, ucat2resv, volwater)
         DO itrc = 1, ntracers
            IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
            CALL update_tracer_concentration(itrc, i, volwater)
         ENDDO
      ENDDO

   END SUBROUTINE tracer_substep

   SUBROUTINE decay_river_pool (pool, fraction, source_sink)
      USE MOD_Tracer_Defs, only: trc_tiny
      IMPLICIT NONE
      real(r8), intent(inout) :: pool
      real(r8), intent(in)    :: fraction
      real(r8), intent(inout) :: source_sink
      real(r8) :: before

      IF (pool <= trc_tiny) RETURN
      before = pool
      pool = pool * (1._r8 - fraction)
      source_sink = source_sink + pool - before
   END SUBROUTINE decay_river_pool

   logical FUNCTION riverlake_restart_descriptor_compatible (file_restart)
      USE MOD_NetCDFSerial, only: ncio_var_exist, ncio_inquire_varsize, ncio_read_serial
      IMPLICIT NONE
      character(len=*), intent(in) :: file_restart

      integer :: flags(4), status, schema, descriptor_count, identity_count, complete
      integer, allocatable :: expected(:,:), ondisk(:,:), varsize(:)

      ! status: 1=current and matching, 0=legacy/incompatible (cold start),
      !        -1=malformed/incomplete metadata (fatal). A complete descriptor
      ! from another schema or tracer configuration is safe but incompatible;
      ! inconsistent ranks, fixed width, or internal counts indicate damage.
      CALL tracer_build_descriptor_identity(expected, transport_only=.true.)
      flags = 0
      status = 0
      IF (p_is_master) THEN
         IF (ncio_var_exist(file_restart, 'trc_river_restart_schema', readflag=.false.)) flags(1) = 1
         IF (ncio_var_exist(file_restart, 'trc_river_descriptor_count', readflag=.false.)) flags(2) = 1
         IF (ncio_var_exist(file_restart, 'trc_river_descriptor_identity', readflag=.false.)) flags(3) = 1
         IF (ncio_var_exist(file_restart, 'trc_river_restart_complete', readflag=.false.)) flags(4) = 1
      ENDIF
#ifdef USEMPI
      CALL mpi_bcast(flags, 4, MPI_INTEGER, p_address_master, p_comm_glb, p_err)
#endif

      ! No commit marker means a pre-transaction/legacy schema. Even if an
      ! earlier build wrote descriptor-like fields, their atomicity cannot be
      ! established, so never scatter their state.
      IF (flags(4) == 0) THEN
         IF (p_is_master) WRITE(*,'(A)') &
            '  NOTE read_tracer_restart: restart has no tracer commit marker; cold-starting river tracers.'
         riverlake_restart_descriptor_compatible = .false.
         deallocate(expected)
         RETURN
      ENDIF

      IF (flags(1) /= 1 .or. flags(2) /= 1) THEN
         status = -1
      ELSEIF (p_is_master) THEN
         status = 1
         identity_count = -1

         CALL ncio_inquire_varsize(file_restart, 'trc_river_restart_schema', varsize)
         IF (.not. allocated(varsize)) THEN
            status = -1
         ELSE
            IF (size(varsize) /= 0) status = -1
            deallocate(varsize)
         ENDIF
         CALL ncio_inquire_varsize(file_restart, 'trc_river_descriptor_count', varsize)
         IF (.not. allocated(varsize)) THEN
            status = -1
         ELSE
            IF (size(varsize) /= 0) status = -1
            deallocate(varsize)
         ENDIF
         CALL ncio_inquire_varsize(file_restart, 'trc_river_restart_complete', varsize)
         IF (.not. allocated(varsize)) THEN
            status = -1
         ELSE
            IF (size(varsize) /= 0) status = -1
            deallocate(varsize)
         ENDIF

         IF (status == 1) THEN
            CALL ncio_read_serial(file_restart, 'trc_river_restart_complete', complete)
            IF (complete /= 1) status = -1
         ENDIF
         IF (status == 1) THEN
            CALL ncio_read_serial(file_restart, 'trc_river_restart_schema', schema)
            CALL ncio_read_serial(file_restart, 'trc_river_descriptor_count', descriptor_count)
            IF (descriptor_count < 0 .or. descriptor_count > 1000) status = -1
         ENDIF
         IF (status == 1) THEN
            IF (schema /= RIVER_TRACER_RESTART_SCHEMA_VERSION) THEN
               status = 0
            ELSEIF (descriptor_count == 0) THEN
               ! Coherent empty transaction: provider-only configurations have
               ! no generic river state and therefore no zero-length identity
               ! variable. A stale identity variable in a reused NetCDF target
               ! is ignored because committed count=0 is authoritative.
               status = merge(1, 0, size(expected, 2) == 0)
            ELSEIF (flags(3) /= 1) THEN
               status = -1
            ELSE
               CALL ncio_inquire_varsize(file_restart, 'trc_river_descriptor_identity', varsize)
               IF (.not. allocated(varsize)) THEN
                  status = -1
               ELSE
                  IF (size(varsize) /= 2) THEN
                     status = -1
                  ELSEIF (varsize(1) /= TRACER_DESCRIPTOR_IDENTITY_WIDTH) THEN
                     status = -1
                  ELSE
                     identity_count = varsize(2)
                     IF (identity_count /= descriptor_count) status = -1
                  ENDIF
                  deallocate(varsize)
               ENDIF
               IF (status == 1) THEN
                  CALL ncio_read_serial(file_restart, 'trc_river_descriptor_identity', ondisk)
                  IF (.not. allocated(ondisk)) THEN
                     status = -1
                  ELSE
                     IF (size(ondisk, 1) /= TRACER_DESCRIPTOR_IDENTITY_WIDTH .or. &
                         size(ondisk, 2) /= descriptor_count) THEN
                        status = -1
                     ELSEIF (descriptor_count /= size(expected, 2)) THEN
                        status = 0
                     ELSEIF (.not. all(ondisk == expected)) THEN
                        status = 0
                     ENDIF
                     deallocate(ondisk)
                  ENDIF
               ENDIF
            ENDIF
         ENDIF
      ENDIF
#ifdef USEMPI
      CALL mpi_bcast(status, 1, MPI_INTEGER, p_address_master, p_comm_glb, p_err)
#endif

      IF (status < 0) THEN
         CALL CoLM_stop('malformed/incomplete river tracer restart descriptor metadata')
      ELSEIF (status == 0) THEN
         IF (p_is_master) WRITE(*,'(A)') &
            '  NOTE read_tracer_restart: tracer descriptor changed; cold-starting river tracers.'
      ENDIF
      riverlake_restart_descriptor_compatible = status == 1
      deallocate(expected)
   END FUNCTION riverlake_restart_descriptor_compatible

   SUBROUTINE probe_riverlake_restart_vector (file_restart, varname, required, present_on_disk)
      USE MOD_NetCDFSerial, only: ncio_var_exist, ncio_inquire_varsize
      USE MOD_Grid_RiverLakeNetwork, only: totalnumucat
      IMPLICIT NONE
      character(len=*), intent(in) :: file_restart, varname
      logical, intent(in) :: required
      logical, intent(out) :: present_on_disk

      integer :: status
      integer, allocatable :: varsize(:)

      ! status: 1=present with the exact ucatch-vector shape, 0=absent,
      !        -1=present but malformed. Probe every required state field before
      ! the first scatter so a complete=1 file is loaded atomically or not at all.
      status = 0
      IF (p_is_master) THEN
         IF (ncio_var_exist(file_restart, trim(varname), readflag=.false.)) THEN
            status = 1
            CALL ncio_inquire_varsize(file_restart, trim(varname), varsize)
            IF (.not. allocated(varsize)) THEN
               status = -1
            ELSE
               IF (size(varsize) /= 1) THEN
                  status = -1
               ELSEIF (varsize(1) /= totalnumucat) THEN
                  status = -1
               ENDIF
               deallocate(varsize)
            ENDIF
         ENDIF
      ENDIF
#ifdef USEMPI
      CALL mpi_bcast(status, 1, MPI_INTEGER, p_address_master, p_comm_glb, p_err)
#endif

      IF (status < 0 .or. (required .and. status == 0)) THEN
         IF (p_is_master) WRITE(*,'(3A)') &
            'ERROR read_tracer_restart: required/current river tracer vector "', &
            trim(varname), '" is absent or has the wrong rank/ucatch extent.'
         CALL CoLM_stop('incomplete or malformed committed river tracer restart')
      ENDIF
      present_on_disk = status == 1
   END SUBROUTINE probe_riverlake_restart_vector

   SUBROUTINE validate_riverlake_restart_state ()
      USE, INTRINSIC :: ieee_arithmetic, only: ieee_is_finite
      USE MOD_Grid_RiverLakeNetwork, only: numucat
      IMPLICIT NONE

      integer :: invalid_counts(7)
      integer :: i, itrc

      ! [nonfinite mass, negative mass, nonfinite signed buffer,
      !  nonfinite signed tracer accumulator, nonfinite runoff reference,
      !  nonfinite levee mass, negative levee mass]
      invalid_counts = 0
      IF (p_is_worker .and. numucat > 0) THEN
         DO i = 1, numucat
            IF (.not. ieee_is_finite(acc_rnof_ref(i))) invalid_counts(5) = invalid_counts(5) + 1
            DO itrc = 1, ntracers
               IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
               IF (.not. ieee_is_finite(trc_mass(itrc, i))) THEN
                  invalid_counts(1) = invalid_counts(1) + 1
               ELSEIF (trc_mass(itrc, i) < -TRC_RESTART_NEGATIVE_DUST) THEN
                  invalid_counts(2) = invalid_counts(2) + 1
               ENDIF
               IF (.not. ieee_is_finite(trc_inp_buf(itrc, i))) &
                  invalid_counts(3) = invalid_counts(3) + 1
               IF (.not. ieee_is_finite(acc_trc_inp(itrc, i))) &
                  invalid_counts(4) = invalid_counts(4) + 1
               IF (.not. ieee_is_finite(trc_levsto(itrc, i))) THEN
                  invalid_counts(6) = invalid_counts(6) + 1
               ELSEIF (trc_levsto(itrc, i) < -TRC_RESTART_NEGATIVE_DUST) THEN
                  invalid_counts(7) = invalid_counts(7) + 1
               ENDIF
            ENDDO
         ENDDO
      ENDIF
#ifdef USEMPI
      CALL mpi_allreduce(MPI_IN_PLACE, invalid_counts, size(invalid_counts), &
         MPI_INTEGER, MPI_SUM, p_comm_glb, p_err)
#endif

      IF (any(invalid_counts > 0)) THEN
         IF (p_is_master) THEN
            WRITE(*,'(A,7(I0,1X))') 'ERROR read_tracer_restart invalid state counts '// &
               '[mass_nan mass_neg buffer_nan accinp_nan runoff_nan levee_nan levee_neg]: ', invalid_counts
         ENDIF
         CALL CoLM_stop('invalid river/lake tracer restart state')
      ENDIF

      ! Canonicalize only sub-picomass negative roundoff admitted by the
      ! domain check above. Signed runoff buffers/accumulators are prognostic
      ! correction state and must never be clipped.
      IF (p_is_worker .and. numucat > 0) THEN
         DO itrc = 1, ntracers
            IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
            WHERE (trc_mass(itrc, :) < 0._r8) trc_mass(itrc, :) = 0._r8
            WHERE (trc_levsto(itrc, :) < 0._r8) trc_levsto(itrc, :) = 0._r8
         ENDDO
      ENDIF
   END SUBROUTINE validate_riverlake_restart_state


   !-------------------------------------------------------------------------------------
   ! Flush accumulated tracer diagnostics
   !-------------------------------------------------------------------------------------
   SUBROUTINE tracer_flush_acc ()

   USE MOD_Grid_RiverLakeNetwork, only: numucat
   IMPLICIT NONE

      ! Reset the *history* accumulators only. acc_trc_inp / acc_rnof_ref are
      ! routing-period state, not history state, and are reset at the end of
      ! each routing period in MOD_Grid_RiverLakeFlow.F90:1263-1264. Resetting
      ! acc_rnof_ref here too would leave acc_trc_inp / acc_rnof_ref with
      ! mismatched windows when history flushes happen between routing periods
      ! (acctime_rnof_max > deltim), corrupting restart recovery.
	      IF (numucat > 0) THEN
		         IF (allocated(a_trc_storage_mass)) a_trc_storage_mass = 0._r8
		         IF (allocated(a_water_storage)) a_water_storage = 0._r8
		         IF (allocated(a_trc_levsto_mass)) a_trc_levsto_mass = 0._r8
		         IF (allocated(a_levsto_water)) a_levsto_water = 0._r8
		         IF (allocated(a_trc_out   )) a_trc_out    = 0._r8
         IF (allocated(a_trc_bifout)) a_trc_bifout = 0._r8
      ENDIF

   END SUBROUTINE tracer_flush_acc

   !-------------------------------------------------------------------------------------
   ! Read tracer restart
   !-------------------------------------------------------------------------------------
   SUBROUTINE read_tracer_restart (file_restart, found_restart, missing_mask)

   USE MOD_Vector_ReadWrite
   USE MOD_Grid_RiverLakeNetwork, only: numucat, totalnumucat, ucat_data_address, &
      ucat_gdid, ucat_next
   USE MOD_Grid_RiverLakeLevee, only: has_levee_bf => has_levee
   IMPLICIT NONE

   character(len=*), intent(in) :: file_restart
   ! .true. when no generic transport row exists, or when the complete current
   ! transaction was loaded. Legacy/descriptor-incompatible files cold-start
   ! every generic row and report .false. only when such rows are configured.
   logical, optional, intent(out) :: found_restart
   ! Retained for caller ABI compatibility. A cold start marks every generic
   ! transport row; committed transactions never default individual rows.
   logical, optional, intent(out) :: missing_mask(:)

	   integer :: itrc
	   integer :: ii_bf, itrc_bf
	   logical :: network_meta_matches, field_present
	   logical :: has_transport_tracer, descriptor_compatible
	   character(len=64) :: varname
	   real(r8), allocatable :: tmpvec(:)

	      IF (present(missing_mask)) missing_mask = .false.
	      IF (ntracers <= 0) THEN
	         IF (present(found_restart)) found_restart = .true.
	         RETURN
	      ENDIF
	      ! Provider-owned tracers have no generic river vectors, but an existing
	      ! generic-river transaction must still be structurally valid.  Validate
	      ! its commit metadata before the provider-only early return so complete=0
	      ! or partial schema/count metadata cannot be silently accepted.  A
	      ! coherent empty, legacy, or descriptor-incompatible transaction remains
	      ! harmless here because no generic state will be probed or consumed.
	      descriptor_compatible = riverlake_restart_descriptor_compatible(file_restart)
	      has_transport_tracer = .false.
	      DO itrc = 1, ntracers
	         IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
	         has_transport_tracer = .true.
	      ENDDO
	      IF (.not. has_transport_tracer) THEN
	         IF (present(found_restart)) found_restart = .true.
	         RETURN
	      ENDIF

      ! Count/name-only metadata cannot prove that the loaded numbers retain
      ! the same units and physics. Older schemas are therefore an explicit
      ! whole-domain cold start; a complete current descriptor is required
      ! before any river tracer state is scattered.
	      IF (.not. descriptor_compatible) THEN
         IF (present(missing_mask)) THEN
            DO itrc = 1, ntracers
               IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
               missing_mask(itrc) = .true.
            ENDDO
         ENDIF
         IF (p_is_worker) THEN
            IF (allocated(trc_mass)) trc_mass = 0._r8
            IF (allocated(trc_conc)) trc_conc = 0._r8
            IF (allocated(trc_inp_buf)) trc_inp_buf = 0._r8
            IF (allocated(acc_trc_inp)) acc_trc_inp = 0._r8
            IF (allocated(acc_rnof_ref)) acc_rnof_ref = 0._r8
            IF (allocated(trc_levsto)) trc_levsto = 0._r8
         ENDIF
         IF (present(found_restart)) found_restart = .false.
         RETURN
      ENDIF

      ! A committed current transaction must contain every required vector with
      ! the exact ucatch extent. Complete this whole preflight before the first
      ! scatter so no rank can observe a partially loaded tracer state.
      CALL probe_riverlake_restart_vector(file_restart, 'trc_numucat_meta', .true., field_present)
      CALL probe_riverlake_restart_vector(file_restart, 'trc_ucat_gdid_meta', .true., field_present)
      CALL probe_riverlake_restart_vector(file_restart, 'trc_ucat_next_meta', .true., field_present)
      DO itrc = 1, ntracers
         IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
         write(varname, '(A,A)') 'trc_mass_', trim(tracer_names(itrc))
         CALL probe_riverlake_restart_vector(file_restart, trim(varname), .true., field_present)
         write(varname, '(A,A)') 'trc_inpbuf_', trim(tracer_names(itrc))
         CALL probe_riverlake_restart_vector(file_restart, trim(varname), .true., field_present)
         write(varname, '(A,A)') 'trc_accinp_', trim(tracer_names(itrc))
         CALL probe_riverlake_restart_vector(file_restart, trim(varname), .true., field_present)
         write(varname, '(A,A)') 'trc_levsto_', trim(tracer_names(itrc))
         CALL probe_riverlake_restart_vector(file_restart, trim(varname), .true., field_present)
      ENDDO
      CALL probe_riverlake_restart_vector(file_restart, 'acc_rnof_ref', .true., field_present)

      ! vector_read_and_scatter contains mpi_barrier(p_comm_glb), so
      ! ALL ranks (master, workers, IO) must enter this routine.
      IF (p_is_worker .and. numucat > 0) THEN
         allocate (tmpvec(numucat))
      ELSE
         allocate (tmpvec(0))
      ENDIF

      ! Stable grid-cell identity plus downstream global UCID catch both
      ! reordered cells and changed routing topology at equal network size.
      network_meta_matches = .true.
      CALL vector_read_and_scatter(file_restart, tmpvec, numucat, &
         'trc_ucat_gdid_meta', ucat_data_address)
      IF (p_is_worker .and. numucat > 0) &
         network_meta_matches = all(nint(tmpvec(:)) == ucat_gdid(:))
      CALL vector_read_and_scatter(file_restart, tmpvec, numucat, &
         'trc_ucat_next_meta', ucat_data_address)
      IF (p_is_worker .and. numucat > 0) &
         network_meta_matches = network_meta_matches .and. all(nint(tmpvec(:)) == ucat_next(:))
#ifdef USEMPI
      CALL mpi_allreduce(MPI_IN_PLACE, network_meta_matches, 1, MPI_LOGICAL, MPI_LAND, p_comm_glb, p_err)
#endif
      IF (.not. network_meta_matches) THEN
         IF (p_is_master) WRITE(*,'(A)') &
            'ERROR: river/lake tracer restart belongs to a different catchment network; aborting.'
         CALL CoLM_stop()
      ENDIF

      CALL vector_read_and_scatter(file_restart, tmpvec, numucat, &
         'trc_numucat_meta', ucat_data_address)
      field_present = .true.
      IF (p_is_worker .and. numucat > 0) &
         field_present = all(nint(tmpvec(:)) == totalnumucat)
#ifdef USEMPI
      CALL mpi_allreduce(MPI_IN_PLACE, field_present, 1, MPI_LOGICAL, MPI_LAND, p_comm_glb, p_err)
#endif
      IF (.not. field_present) THEN
         IF (p_is_master) WRITE(*,'(A)') &
            'ERROR: river/lake tracer restart catchment-count metadata is inconsistent.'
         CALL CoLM_stop()
      ENDIF

      ! The full descriptor is the single tracer-identity source of truth.
      ! Network size/identity are independently guarded above; duplicating
      ! count/name hashes here creates a second, weaker schema that can drift.

      DO itrc = 1, ntracers
         IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
         write(varname, '(A,A)') 'trc_mass_', trim(tracer_names(itrc))
         CALL vector_read_and_scatter(file_restart, tmpvec, numucat, trim(varname), ucat_data_address)
         IF (p_is_worker .and. numucat > 0) trc_mass(itrc, :) = tmpvec(:)

         write(varname, '(A,A)') 'trc_inpbuf_', trim(tracer_names(itrc))
         CALL vector_read_and_scatter(file_restart, tmpvec, numucat, trim(varname), ucat_data_address)
         IF (p_is_worker .and. numucat > 0) trc_inp_buf(itrc, :) = tmpvec(:)

         write(varname, '(A,A)') 'trc_levsto_', trim(tracer_names(itrc))
         CALL vector_read_and_scatter(file_restart, tmpvec, numucat, trim(varname), ucat_data_address)
         IF (p_is_worker .and. numucat > 0) trc_levsto(itrc, :) = tmpvec(:)

         write(varname, '(A,A)') 'trc_accinp_', trim(tracer_names(itrc))
         CALL vector_read_and_scatter(file_restart, tmpvec, numucat, trim(varname), ucat_data_address)
         IF (p_is_worker .and. numucat > 0) acc_trc_inp(itrc, :) = tmpvec(:)

      ENDDO

      CALL vector_read_and_scatter(file_restart, tmpvec, numucat, 'acc_rnof_ref', ucat_data_address)
      IF (p_is_worker .and. numucat > 0) acc_rnof_ref(:) = tmpvec(:)

      ! Reject corrupt loaded rows before the levee-configuration migration can
      ! fold protected mass into the visible pool and thereby hide its origin.
      ! This same validator is run again after levee migration below.
      CALL validate_riverlake_restart_state()

      ! The water restart reader folds protected storage into visible storage
      ! whenever the current configuration has no levee for a cell.  Mirror
      ! that transfer for tracer mass; simply zeroing trc_levsto would either
      ! strand mass (global LEVEE off) or lose it (per-cell mask change).
      IF (p_is_worker .and. numucat > 0 .and. allocated(trc_mass) &
          .and. allocated(trc_levsto) .and. allocated(has_levee_bf)) THEN
         DO ii_bf = 1, numucat
            IF (ii_bf <= size(has_levee_bf)) THEN
               IF (.not. has_levee_bf(ii_bf)) THEN
                  DO itrc_bf = 1, ntracers
                     IF (.not. tracer_uses_land_water_transport(itrc_bf)) CYCLE
                     trc_mass(itrc_bf, ii_bf) = trc_mass(itrc_bf, ii_bf) &
                        + trc_levsto(itrc_bf, ii_bf)
                     trc_levsto(itrc_bf, ii_bf) = 0._r8
                  ENDDO
               ENDIF
            ENDIF
         ENDDO
      ENDIF

      ! Levee migration may have added two large finite pools. Recheck for
      ! overflow after the transformation before transport can consume state.
      CALL validate_riverlake_restart_state()

	      deallocate (tmpvec)

      IF (present(found_restart)) found_restart = .true.

   END SUBROUTINE read_tracer_restart


   !-------------------------------------------------------------------------------------
   ! Write tracer restart
   !-------------------------------------------------------------------------------------
   SUBROUTINE write_tracer_restart (file_restart)

   USE MOD_Vector_ReadWrite
   USE MOD_NetCDFSerial, only: ncio_define_dimension, ncio_write_serial
   USE MOD_Grid_RiverLakeNetwork, only: numucat, totalnumucat, ucat_data_address, &
      ucat_gdid, ucat_next
   IMPLICIT NONE

   character(len=*), intent(in) :: file_restart

   integer :: itrc
   character(len=64) :: varname
   real(r8), allocatable :: tmpvec(:)
   integer, allocatable :: descriptor_identity(:,:)

      ! Guard: tracer module may not be initialised (e.g. mkinidata).
      ! tracer_names is allocated on ALL ranks by river_lake_tracer_init (before the
      ! p_is_worker guard), so this check is safe for master/IO.
      ! Do NOT check allocated(trc_mass) here — trc_mass is worker-only,
      ! but vector_gather_and_write has mpi_barrier(p_comm_glb) that
      ! requires ALL ranks to participate.
      IF (.not. allocated(tracer_names)) RETURN
      ! Never return on a worker-only allocation predicate: every rank must
      ! enter the gather collectives below in the same order. Initialization
      ! allocates zero-length routing state on empty workers.

      ! A count/name hash cannot distinguish tracers whose units, ownership, or
      ! physical parameters changed. Build the exact shared descriptor once;
      ! it is committed only after every distributed state field is durable.
      CALL tracer_build_descriptor_identity(descriptor_identity, transport_only=.true.)
      IF (size(descriptor_identity, 2) <= 0) THEN
         ! Provider-only configurations carry no generic state. Commit an
         ! explicit empty transaction instead of leaving complete=0: a later
         ! run that adds a transport tracer must cold-start cleanly rather than
         ! misclassifying this intentional absence as an interrupted write.
         IF (p_is_master) THEN
            CALL ncio_write_serial(file_restart, 'trc_river_restart_complete', 0)
            CALL ncio_write_serial(file_restart, 'trc_river_descriptor_count', 0)
            CALL ncio_write_serial(file_restart, 'trc_river_restart_schema', &
               RIVER_TRACER_RESTART_SCHEMA_VERSION)
            CALL ncio_write_serial(file_restart, 'trc_river_restart_complete', 1)
         ENDIF
         deallocate(descriptor_identity)
         RETURN
      ENDIF

      ! Never persist a poisoned checkpoint. This is the same collective state
      ! contract enforced after reads, so all ranks must enter before master I/O.
      CALL validate_riverlake_restart_state()
      IF (p_is_master) THEN
         CALL ncio_write_serial(file_restart, 'trc_river_restart_complete', 0)
      ENDIF

      IF (p_is_worker .and. numucat > 0) THEN
         allocate (tmpvec(numucat))
      ELSE
         allocate (tmpvec(0))
      ENDIF

      ! Restart metadata guards. Store as ucatch vectors so the existing
      ! vector I/O path can scatter and verify them collectively on read.
      IF (p_is_worker .and. numucat > 0) tmpvec(:) = real(totalnumucat, r8)
      CALL vector_gather_and_write ( &
         tmpvec, numucat, totalnumucat, ucat_data_address, file_restart, 'trc_numucat_meta', 'ucatch')

      IF (p_is_worker .and. numucat > 0) tmpvec(:) = real(ucat_gdid(:), r8)
      CALL vector_gather_and_write ( &
         tmpvec, numucat, totalnumucat, ucat_data_address, file_restart, 'trc_ucat_gdid_meta', 'ucatch')

      IF (p_is_worker .and. numucat > 0) tmpvec(:) = real(ucat_next(:), r8)
      CALL vector_gather_and_write ( &
         tmpvec, numucat, totalnumucat, ucat_data_address, file_restart, 'trc_ucat_next_meta', 'ucatch')

      DO itrc = 1, ntracers
         IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
         IF (p_is_worker .and. numucat > 0) THEN
            tmpvec(:) = trc_mass(itrc, :)
         ENDIF

         write(varname, '(A,A)') 'trc_mass_', trim(tracer_names(itrc))
         CALL vector_gather_and_write ( &
            tmpvec, numucat, totalnumucat, ucat_data_address, file_restart, trim(varname), 'ucatch')

         IF (p_is_worker .and. numucat > 0) THEN
            tmpvec(:) = trc_inp_buf(itrc, :)
         ENDIF
         write(varname, '(A,A)') 'trc_inpbuf_', trim(tracer_names(itrc))
         CALL vector_gather_and_write ( &
            tmpvec, numucat, totalnumucat, ucat_data_address, file_restart, trim(varname), 'ucatch')

         ! Per-tracer routing-period accumulator. Paired with acc_rnof_ref
         ! below; both must persist so a restart written mid-period
         ! recovers the queued land-tracer input instead of dropping it.
         IF (p_is_worker .and. numucat > 0) THEN
            tmpvec(:) = acc_trc_inp(itrc, :)
         ENDIF
         write(varname, '(A,A)') 'trc_accinp_', trim(tracer_names(itrc))
         CALL vector_gather_and_write ( &
            tmpvec, numucat, totalnumucat, ucat_data_address, file_restart, trim(varname), 'ucatch')

         ! Always persist the protected-side row, even when levees are disabled
         ! (then it is zero). This makes schema completeness independent of a
         ! runtime levee toggle and preserves mass when a leveed restart is read
         ! by a non-leveed configuration that folds the pool into visible water.
         IF (p_is_worker .and. numucat > 0) THEN
            tmpvec(:) = trc_levsto(itrc, :)
         ENDIF
         write(varname, '(A,A)') 'trc_levsto_', trim(tracer_names(itrc))
         CALL vector_gather_and_write ( &
            tmpvec, numucat, totalnumucat, ucat_data_address, file_restart, trim(varname), 'ucatch')
      ENDDO

      ! Shared runoff reference paired with acc_trc_inp for the active
      ! routing period. Written once (not per tracer).
      IF (p_is_worker .and. numucat > 0) THEN
         tmpvec(:) = acc_rnof_ref(:)
      ENDIF
      CALL vector_gather_and_write ( &
         tmpvec, numucat, totalnumucat, ucat_data_address, file_restart, 'acc_rnof_ref', 'ucatch')

      ! Commit metadata last. If any state/descriptor write above is interrupted,
      ! complete remains zero and the reader refuses the mixed-generation file.
      IF (p_is_master) THEN
         CALL ncio_define_dimension(file_restart, 'trc_river_descriptor_field', &
            TRACER_DESCRIPTOR_IDENTITY_WIDTH)
         CALL ncio_define_dimension(file_restart, 'trc_river_transport_tracer', &
            size(descriptor_identity, 2))
         CALL ncio_write_serial(file_restart, 'trc_river_descriptor_count', &
            size(descriptor_identity, 2))
         CALL ncio_write_serial(file_restart, 'trc_river_descriptor_identity', &
            descriptor_identity, 'trc_river_descriptor_field', 'trc_river_transport_tracer')
         CALL ncio_write_serial(file_restart, 'trc_river_restart_schema', &
            RIVER_TRACER_RESTART_SCHEMA_VERSION)
         CALL ncio_write_serial(file_restart, 'trc_river_restart_complete', 1)
      ENDIF

      deallocate (tmpvec)
      deallocate (descriptor_identity)

   END SUBROUTINE write_tracer_restart


   !-------------------------------------------------------------------------------------
   ! Write tracer history output
   !-------------------------------------------------------------------------------------
   SUBROUTINE write_tracer_history (file_hist_ucat, itime_in_file_ucat, acctime_ucat_hist)

	   USE MOD_Grid_RiverLakeNetwork, only: numucat, allups_mask_ucat
	   ! Route history is dispatched, not written directly, so 'one' and
	   ! 'block' cannot drift apart variable by variable.
	   USE MOD_Grid_RiverLakeHistRoute, only: route_hist_write_ucat
	   USE MOD_Tracer_Defs, only: tracer_uses_delta_diagnostics, tracers, trc_tiny, &
	      trc_delta_sanity_max
	   USE MOD_Vars_Global, only: spval
	   IMPLICIT NONE

   character(len=*), intent(in) :: file_hist_ucat
   integer,  intent(in) :: itime_in_file_ucat
   real(r8), intent(in) :: acctime_ucat_hist(:)  ! Per-unit-catchment accumulated history time [s]

	   integer :: itrc, i
	   character(len=64) :: varname
	   character(len=128) :: longname
	   character(len=32) :: conc_word, conc_units, mass_units, flux_units
	   real(r8), allocatable :: tmpvec(:)
	   real(r8) :: ratio_loc, delta_loc
	   real(r8), parameter :: trc_hist_fp_dust = 1.0e-12_r8

	      IF (p_is_worker .and. numucat > 0) THEN
	         allocate (tmpvec(numucat))
	      ELSE
	         allocate (tmpvec(0))
	      ENDIF
      ! Output-side FP-dust clamp threshold. Same rationale as
      ! check_tracer_state: trc_mass can carry sub-picomass alternating
      ! residue from upwind flux add/subtract, which propagates to
      ! trc_conc / a_trc_out / a_trc_bifout. The underlying
      ! state is preserved intact (so transport / limiter regressions
      ! remain visible via the WARNING watchdog); only this display
      ! slice hides the noise floor.
      DO itrc = 1, ntracers
         IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
         IF (tracer_uses_delta_diagnostics(itrc)) THEN
            conc_word  = 'ratio'
            conc_units = 'R'
            mass_units = 'R*m3'
            flux_units = 'R*m3/s'
         ELSE
            conc_word  = 'concentration'
            conc_units = tracer_concentration_units(itrc)
            mass_units = 'tracer'
            flux_units = 'tracer/s'
         ENDIF

         ! --- Tracer concentration ---
         ! Water-weighted mean: sum(mass*dt) / sum(volume*dt), the same quotient
         ! the delta diagnostic below is built from.  A plain time average of the
         ! per-step ratio would disagree with that delta for the very same field
         ! (for isotopes conc_units is 'R', i.e. they report the same quantity),
         ! and would fold dry sub-steps -- where trc_conc is forced to zero and
         ! concentration is undefined, not zero -- into the mean.  Both
         ! accumulators below are gated on volwater > trc_v_dry_off, so dry time
         ! leaves numerator and denominator alike.
         IF (p_is_worker .and. numucat > 0) THEN
            tmpvec(:) = spval
            IF (allocated(a_trc_storage_mass) .and. allocated(a_water_storage)) THEN
               DO i = 1, numucat
                  IF (i > size(acctime_ucat_hist)) CYCLE
                  IF (acctime_ucat_hist(i) <= 0._r8) CYCLE
                  IF (a_water_storage(i) <= trc_delta_diag_vmin * acctime_ucat_hist(i)) CYCLE
                  tmpvec(i) = a_trc_storage_mass(itrc, i) / a_water_storage(i)
               ENDDO
            ENDIF
            WHERE (abs(tmpvec) < trc_hist_fp_dust) tmpvec = 0._r8
         ENDIF

         write(varname, '(A,A)') 'f_trc_conc_', trim(tracer_names(itrc))
         write(longname, '(5A)') 'tracer ', trim(conc_word), &
            ' (', trim(tracer_names(itrc)), ')'

	         CALL route_hist_write_ucat (tmpvec, trim(varname), &
	            longname = trim(longname), &
	            units = trim(conc_units))

	         IF (tracer_uses_delta_diagnostics(itrc)) THEN
	            IF (p_is_worker .and. numucat > 0) THEN
	               tmpvec(:) = spval
	               IF (tracers(itrc)%ref_ratio > trc_tiny) THEN
	                  DO i = 1, numucat
	                     IF (allocated(allups_mask_ucat)) THEN
	                        IF (i > size(allups_mask_ucat)) CYCLE
	                        IF (allups_mask_ucat(i) < 0.5_r8) CYCLE
	                     ENDIF
		                     IF (i > size(acctime_ucat_hist)) CYCLE
		                     IF (acctime_ucat_hist(i) <= 0._r8) CYCLE
		                     IF (.not. allocated(a_water_storage)) CYCLE
		                     IF (.not. allocated(a_trc_storage_mass)) CYCLE
		                     IF (a_water_storage(i) <= trc_delta_diag_vmin * acctime_ucat_hist(i)) CYCLE
		                     ratio_loc = a_trc_storage_mass(itrc, i) / a_water_storage(i)
		                     IF (ratio_loc <= trc_tiny) CYCLE
	                     delta_loc = (ratio_loc / tracers(itrc)%ref_ratio - 1.0_r8) * 1000.0_r8
	                     IF (abs(delta_loc) <= trc_delta_sanity_max) tmpvec(i) = delta_loc
	                  ENDDO
	               ENDIF
	            ENDIF

	            write(varname, '(A,A)') 'f_trc_delta_', trim(tracer_names(itrc))
	            write(longname, '(A,A,A)') 'tracer delta (', trim(tracer_names(itrc)), ')'
	            CALL route_hist_write_ucat (tmpvec, trim(varname), &
	               longname = trim(longname), &
	               units = 'permil')
	         ENDIF

	         ! --- Tracer outflux ---
         IF (p_is_worker .and. numucat > 0) THEN
            tmpvec(:) = spval
            DO i = 1, numucat
               IF (i <= size(acctime_ucat_hist)) THEN
                  IF (acctime_ucat_hist(i) > 0._r8) tmpvec(i) = a_trc_out(itrc, i) / acctime_ucat_hist(i)
               ENDIF
            ENDDO
            WHERE (abs(tmpvec) < trc_hist_fp_dust) tmpvec = 0._r8
         ENDIF

         write(varname, '(A,A)') 'f_trc_flux_', trim(tracer_names(itrc))
         write(longname, '(A,A,A)') 'tracer outflux (', trim(tracer_names(itrc)), ')'

	         CALL route_hist_write_ucat (tmpvec, trim(varname), &
	            longname = trim(longname), &
	            units = trim(flux_units))

		         IF (DEF_USE_LEVEE) THEN
		            ! --- Protected-side tracer storage behind levees ---
		            IF (p_is_worker .and. numucat > 0) THEN
		               tmpvec(:) = 0._r8
		               IF (allocated(a_trc_levsto_mass)) THEN
		                  tmpvec(:) = spval
		                  DO i = 1, numucat
		                     IF (i <= size(acctime_ucat_hist)) THEN
		                        IF (acctime_ucat_hist(i) > 0._r8) &
		                           tmpvec(i) = a_trc_levsto_mass(itrc, i) / acctime_ucat_hist(i)
		                     ENDIF
		                  ENDDO
		               ENDIF
		               WHERE (abs(tmpvec) < trc_hist_fp_dust) tmpvec = 0._r8
		            ENDIF

		            write(varname, '(A,A)') 'f_trc_levsto_', trim(tracer_names(itrc))
		            write(longname, '(A,A,A)') 'protected-side levee tracer storage (', trim(tracer_names(itrc)), ')'

		            CALL route_hist_write_ucat (tmpvec, trim(varname), &
		               longname = trim(longname), &
		               units = trim(mass_units))

		            IF (tracer_uses_delta_diagnostics(itrc)) THEN
		               IF (p_is_worker .and. numucat > 0) THEN
		                  tmpvec(:) = spval
		                  IF (tracers(itrc)%ref_ratio > trc_tiny .and. &
		                      allocated(a_trc_levsto_mass) .and. allocated(a_levsto_water)) THEN
		                     DO i = 1, numucat
		                        IF (allocated(allups_mask_ucat)) THEN
		                           IF (i > size(allups_mask_ucat)) CYCLE
		                           IF (allups_mask_ucat(i) < 0.5_r8) CYCLE
		                        ENDIF
		                        IF (i > size(acctime_ucat_hist)) CYCLE
		                        IF (acctime_ucat_hist(i) <= 0._r8) CYCLE
		                        IF (a_levsto_water(i) <= trc_delta_diag_vmin * acctime_ucat_hist(i)) CYCLE
		                        ratio_loc = a_trc_levsto_mass(itrc, i) / a_levsto_water(i)
		                        IF (ratio_loc <= trc_tiny) CYCLE
		                        delta_loc = (ratio_loc / tracers(itrc)%ref_ratio - 1.0_r8) * 1000.0_r8
		                        IF (abs(delta_loc) <= trc_delta_sanity_max) tmpvec(i) = delta_loc
		                     ENDDO
		                  ENDIF
		               ENDIF

		               write(varname, '(A,A)') 'f_trc_levdelta_', trim(tracer_names(itrc))
		               write(longname, '(A,A,A)') 'protected-side levee tracer delta (', trim(tracer_names(itrc)), ')'
		               CALL route_hist_write_ucat (tmpvec, trim(varname), &
		                  longname = trim(longname), &
		                  units = 'permil')
		            ENDIF
		         ENDIF

		         IF (DEF_USE_BIFURCATION) THEN
		            ! --- Tracer bifurcation net flux ---
		            IF (p_is_worker .and. numucat > 0) THEN
		               tmpvec(:) = spval
		               DO i = 1, numucat
		                  IF (i <= size(acctime_ucat_hist)) THEN
		                     IF (acctime_ucat_hist(i) > 0._r8) tmpvec(i) = a_trc_bifout(itrc, i) / acctime_ucat_hist(i)
		                  ENDIF
		               ENDDO
		               WHERE (abs(tmpvec) < trc_hist_fp_dust) tmpvec = 0._r8
		            ENDIF

		            write(varname, '(A,A)') 'f_trc_bifout_', trim(tracer_names(itrc))
		            write(longname, '(A,A,A)') 'tracer net bifurcation outflux (', trim(tracer_names(itrc)), ')'

		            CALL route_hist_write_ucat (tmpvec, trim(varname), &
		               longname = trim(longname), &
		               units = trim(flux_units))
		         ENDIF

      ENDDO

	      deallocate (tmpvec)

	   END SUBROUTINE write_tracer_history


   !-------------------------------------------------------------------------------------
   ! Diagnostic check: print min/max of tracer state variables
   !-------------------------------------------------------------------------------------
#ifdef RangeCheck
   SUBROUTINE check_tracer_state ()

   USE MOD_SPMD_Task
   USE MOD_RangeCheck
   USE MOD_Grid_RiverLakeNetwork, only: numucat
   USE MOD_Grid_RiverLakeTimeVars, only: wdsrf_ucat, volresv_state => volresv
   USE MOD_Grid_Reservoir, only: ucat2resv_state => ucat2resv
   USE MOD_Tracer_Defs, only: tracers, trc_tiny, trc_delta_sanity_max, &
      tracer_uses_delta_diagnostics
   USE MOD_Vars_Global, only: spval
   IMPLICIT NONE

   integer :: itrc
   integer :: icell_worst, nbad, i
   character(len=64) :: label
   character(len=16) :: trc_mass_units, trc_conc_units, trc_flux_units
   real(r8), allocatable :: tmp(:)
   real(r8), allocatable :: volresv_check(:)
   integer, allocatable :: ucat2resv_check(:)
   real(r8) :: worst_neg_mass
   real(r8) :: volwater, delta_loc
   real(r8), parameter :: trc_mass_fp_dust  = 1.0e-12_r8   ! [R*m3]
   ! Same magnitude, different dimension. Declared separately so the flux
   ! path cannot be read as comparing a rate against an absolute mass, and
   ! so the two can be tuned independently if the residue ever differs.
   real(r8), parameter :: trc_flux_fp_dust  = 1.0e-12_r8   ! [R*m3/s]
   real(r8), parameter :: trc_mass_neg_warn = -1.0e-6_r8

      ! Workers that never ran river_lake_tracer_init have no data to check.
      ! Master and IO ranks don't allocate trc_mass but MUST still enter
      ! this routine: master participates in check_vector_data's recv/print
      ! side, and all ranks share the same call count to avoid orphaning
      ! worker-to-master mpi_send messages.
      IF (p_is_worker .and. (.not. allocated(trc_mass) .or. .not. allocated(trc_conc))) RETURN

      IF (p_is_master) THEN
         write(*,'(/,A)') 'Checking Tracer Variables ...'
      ENDIF

      IF (p_is_worker .and. numucat > 0) THEN
         allocate (tmp(numucat))
      ELSE
         allocate (tmp(0))
      ENDIF

      ! Display-only thresholds (state itself is never modified here):
      !   trc_mass_fp_dust: absolute trc_mass below this is FP residue
      !       from alternating substep add/subtract (order 1e-15 R*m³);
      !       dust-clamp for log cleanliness only. Well below any
      !       physical input (rnof*R_init is typically >= 1e-9).
      !   trc_mass_neg_warn: trc_mass below this signed threshold is
      !       NOT noise — indicates transport/limiter drift. Emit a
      !       diagnostic once so the root cause can be traced instead
      !       of silently swallowed by a state-side clamp.
      IF (p_is_worker) THEN
         IF (allocated(volresv_state)) THEN
            allocate (volresv_check(size(volresv_state)))
            volresv_check(:) = volresv_state(:)
         ELSE
            allocate (volresv_check(0))
         ENDIF
         IF (allocated(ucat2resv_state)) THEN
            allocate (ucat2resv_check(size(ucat2resv_state)))
            ucat2resv_check(:) = ucat2resv_state(:)
         ELSE
            allocate (ucat2resv_check(0))
         ENDIF
      ENDIF

      DO itrc = 1, ntracers
         IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
         IF (tracer_uses_delta_diagnostics(itrc)) THEN
            trc_mass_units = 'R*m3'
            trc_conc_units = 'R'
            trc_flux_units = 'R*m3/s'
         ELSE
            trc_mass_units = 'tracer'
            trc_conc_units = tracer_concentration_units(itrc)
            trc_flux_units = 'tracer/s'
         ENDIF

         IF (p_is_worker .and. numucat > 0) THEN
            tmp = trc_mass(itrc,:)
            ! Significant-negative watchdog (state unchanged)
            worst_neg_mass = 0._r8
            icell_worst    = 0
            nbad           = 0
            IF (numucat > 0) THEN
               icell_worst = minloc(tmp, dim=1)
               IF (icell_worst >= 1 .and. icell_worst <= numucat) THEN
                  IF (tmp(icell_worst) < trc_mass_neg_warn) THEN
                     worst_neg_mass = tmp(icell_worst)
                     nbad = count(tmp < trc_mass_neg_warn)
                  ENDIF
               ENDIF
            ENDIF
            IF (worst_neg_mass < trc_mass_neg_warn) THEN
               write(*,'(A,A,A,I6,A,I8,A,E12.5)') &
                  ' WARNING check_tracer_state: ', trim(tracer_names(itrc)), &
                  ' trc_mass < -1e-6 in ', nbad, ' cell(s); worst @ucat=', &
                  icell_worst, ' mass=', worst_neg_mass
            ENDIF
            ! Cosmetic dust clamp for the min/max line only
            WHERE (abs(tmp) < trc_mass_fp_dust) tmp = 0._r8
         ENDIF
         write(label,'(5A)') 'trc_mass_', trim(tracer_names(itrc)), ' [', trim(trc_mass_units), ']'
         CALL check_vector_data (label, tmp)

         IF (p_is_worker .and. numucat > 0) THEN
            tmp = trc_conc(itrc,:)
            ! Cosmetic dust clamp on the conc display path, keyed on the MASS
            ! that produced each concentration rather than on the
            ! concentration itself. What this suppresses is a trc_mass of
            ! FP-residue size propagating into trc_conc, with the sign flipped
            ! by division -- so the test belongs on trc_mass.
            !
            ! It used to compare trc_conc against trc_mass_fp_dust directly.
            ! That threshold is an absolute mass in R*m3; trc_conc is a ratio
            ! or a concentration per unit water, so the comparison had no
            ! defined meaning and the cells it caught depended on cell volume.
            ! Display only either way -- trc_conc itself is never modified.
            WHERE (abs(trc_mass(itrc,:)) < trc_mass_fp_dust) tmp = 0._r8
         ENDIF
         write(label,'(5A)') 'trc_conc_', trim(tracer_names(itrc)), ' [', trim(trc_conc_units), ']'
         CALL check_vector_data (label, tmp)

         IF (tracer_uses_delta_diagnostics(itrc)) THEN
            IF (p_is_worker .and. numucat > 0) THEN
               tmp = spval

               IF (tracers(itrc)%ref_ratio > trc_tiny) THEN
                  ! tmp is rebuilt element by element in the loop below; the
                  ! whole-array delta that used to be computed here was
                  ! overwritten on the next line without ever being read.
                  tmp = spval
                  DO i = 1, numucat
                     volwater = 0._r8
                     IF (allocated(wdsrf_ucat)) THEN
                        IF (i <= size(wdsrf_ucat)) THEN
                           CALL get_cell_volume(i, wdsrf_ucat(i), volresv_check, ucat2resv_check, volwater)
                        ENDIF
                     ENDIF

                     IF (trc_conc(itrc, i) > trc_tiny) THEN
                        delta_loc = (trc_conc(itrc, i) / tracers(itrc)%ref_ratio - 1.0_r8) * 1000.0_r8

                        IF (volwater > trc_delta_diag_vmin) THEN
                           IF (abs(delta_loc) <= trc_delta_sanity_max) tmp(i) = delta_loc
                        ENDIF
                     ENDIF
                  ENDDO
               ENDIF
            ENDIF
            write(label,'(A,A,A)') 'trc_delta_', trim(tracer_names(itrc)), ' [permil]'
            CALL check_vector_data (label, tmp)
         ENDIF

         IF (p_is_worker .and. numucat > 0) THEN
            tmp = trc_flux_out(itrc,:)
            ! FP-dust clamp on the flux display path (same rationale
            ! as trc_mass / trc_conc above). Real flux magnitudes are
            ! many orders above the residue so this only hides
            ! the alternating +/- 1e-15 numerical noise.
            WHERE (abs(tmp) < trc_flux_fp_dust) tmp = 0._r8
         ENDIF
         write(label,'(5A)') 'trc_outflux_', trim(tracer_names(itrc)), ' [', trim(trc_flux_units), ']'
         CALL check_vector_data (label, tmp)

         IF (p_is_worker .and. numucat > 0) tmp = acc_trc_inp(itrc,:)
         write(label,'(5A)') 'trc_inp_', trim(tracer_names(itrc)), ' [', trim(trc_mass_units), ']'
         CALL check_vector_data (label, tmp)

         IF (p_is_worker .and. numucat > 0) tmp = trc_inp_buf(itrc,:)
         write(label,'(5A)') 'trc_inpbuf_', trim(tracer_names(itrc)), ' [', trim(trc_mass_units), ']'
         CALL check_vector_data (label, tmp)

         ! Surface protected-side pool so the levee-on case has a
         ! min/max line alongside trc_mass. Same FP-dust clamp rationale.
         IF (p_is_worker .and. numucat > 0) THEN
            IF (allocated(trc_levsto)) THEN
               tmp = trc_levsto(itrc,:)
               WHERE (abs(tmp) < trc_mass_fp_dust) tmp = 0._r8
            ELSE
               tmp = 0._r8
            ENDIF
         ENDIF
         write(label,'(5A)') 'trc_levsto_', trim(tracer_names(itrc)), ' [', trim(trc_mass_units), ']'
         CALL check_vector_data (label, tmp)
      ENDDO
      IF (allocated(volresv_check)) deallocate (volresv_check)
      IF (allocated(ucat2resv_check)) deallocate (ucat2resv_check)

      deallocate (tmp)

   END SUBROUTINE check_tracer_state
#else
   SUBROUTINE check_tracer_state ()
   IMPLICIT NONE
   END SUBROUTINE check_tracer_state
#endif


   !-------------------------------------------------------------------------------------
   ! Deallocate tracer module
   !-------------------------------------------------------------------------------------
   SUBROUTINE river_lake_tracer_final ()

   IMPLICIT NONE

      IF (allocated(tracer_names )) deallocate (tracer_names )
      IF (allocated(trc_mass     )) deallocate (trc_mass     )
      IF (allocated(trc_conc     )) deallocate (trc_conc     )
      IF (allocated(trc_flux_out )) deallocate (trc_flux_out )
      IF (allocated(acc_trc_inp       )) deallocate (acc_trc_inp       )
      IF (allocated(trc_inp_buf       )) deallocate (trc_inp_buf       )
      IF (allocated(acc_rnof_ref      )) deallocate (acc_rnof_ref      )
	      IF (allocated(trc_bif_net_saved)) deallocate (trc_bif_net_saved)
		      IF (allocated(a_trc_storage_mass)) deallocate (a_trc_storage_mass)
		      IF (allocated(a_water_storage   )) deallocate (a_water_storage   )
		      IF (allocated(a_trc_levsto_mass )) deallocate (a_trc_levsto_mass )
		      IF (allocated(a_levsto_water    )) deallocate (a_levsto_water    )
		      IF (allocated(a_trc_out        )) deallocate (a_trc_out        )
      IF (allocated(a_trc_bifout     )) deallocate (a_trc_bifout     )
	      IF (allocated(trc_dry_drain    )) deallocate (trc_dry_drain    )
	      IF (allocated(trc_reactive_source)) deallocate (trc_reactive_source)
	      IF (allocated(trc_levsto       )) deallocate (trc_levsto       )
      CALL release_tracer_substep_workspace()

   END SUBROUTINE river_lake_tracer_final

END MODULE MOD_Tracer_RiverLake
#endif
#endif
