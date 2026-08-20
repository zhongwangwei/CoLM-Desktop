#include <define.h>

#ifdef TRACER
MODULE MOD_Tracer_Vars

   USE MOD_Precision
   USE MOD_Namelist, only: DEF_TRACER_LULCC_ABORT_NBAD
   USE MOD_SPMD_Task, only: p_is_io, CoLM_stop
   USE MOD_Tracer_Defs, only: ntracers, tracers, &
      tracer_uses_land_water_transport, tracer_equilibrate_dissolved

   IMPLICIT NONE
   SAVE
   PRIVATE

   logical :: lulcc_area_fallback_warned = .false.
   real(r8), parameter :: lulcc_mass_abs_tol = 1.e-8_r8
   real(r8), parameter :: lulcc_mass_rel_tol = 1.e-10_r8

   real(r8), allocatable :: trc_ldew_rain  (:,:)
   real(r8), allocatable :: trc_ldew_snow  (:,:)
   real(r8), allocatable :: trc_wliq_soisno(:,:,:)
   real(r8), allocatable :: trc_wice_soisno(:,:,:)
   real(r8), allocatable :: trc_wa         (:,:)
   real(r8), allocatable :: trc_wdsrf      (:,:)
   real(r8), allocatable :: trc_wetwat     (:,:)
   ! Waterless surface residue for nonvolatile conservative solutes.
   ! This is a numerical immobile store, not a precipitated mineral phase.
   real(r8), allocatable :: trc_surface_residue(:,:)
   ! Positive solute orphaned when aquifer water vanishes. It redissolves only
   ! when the aquifer returns, preventing first-rain release at the surface.
   real(r8), allocatable :: trc_subsurface_residue(:,:)
   ! Immobile precipitated solute. Layer, surface, aquifer, and irrigation
   ! reservoir inventories stay separate so saturation cannot move mass between
   ! hydrologically disconnected pools.
   real(r8), allocatable :: trc_solid_soisno(:,:,:)
   real(r8), allocatable :: trc_canopy_solid(:,:)
   real(r8), allocatable :: trc_surface_solid(:,:)
   real(r8), allocatable :: trc_subsurface_solid(:,:)
   real(r8), allocatable :: trc_waterstorage_solid(:,:)

   ! Tracer mirror of MOD_Vars_TimeVariables::waterstorage. Irrigation
   ! (drip/flood/paddy/sprinkler) is an internal transfer from the
   ! per-patch waterstorage reservoir to soil / canopy; water-side
   ! balance (CoLMMAIN:806) treats it that way, so the atmospheric-input
   ! term (forc_prc+forc_prl) does NOT include irrigation. Without a
   ! matching tracer reservoir the tracer side would have to record
   ! irrigation as external input, creating a systematic offset between
   ! the water and tracer conservation accountings. trc_waterstorage
   ! carries the isotope composition of the reservoir and is debited
   ! by exactly the same volume the water side withdraws each step.
   real(r8), allocatable :: trc_waterstorage(:,:)     ! (ntracers, numpatch)

   ! Snow water equivalent tracer for pre-layer accumulation [kg/m2]
   ! Tracks tracer in scv before a snow layer is created (snl=0).
   ! When the first snow layer forms, trc_scv is transferred to trc_wice_soisno.
   real(r8), allocatable :: trc_scv        (:,:)  ! (ntracers, numpatch)

   ! Throughfall tracer reaching ground [kg/m2 per step]
   ! Rain and snow kept separate: rain enters the surface mixed pool via
   ! tracer_soil_water; snow enters trc_scv via tracer_newsnow (mirrors
   ! water side where gwat only carries pg_rain). A previous "trc_pg_to_ground
   ! = rain+snow" combined var double-counted snow tracer when used in the
   ! surface pool — removed to prevent reintroduction.
   real(r8), allocatable :: trc_pg_rain_ground(:,:)    ! (ntracers, numpatch)
   real(r8), allocatable :: trc_pg_snow_ground(:,:)    ! (ntracers, numpatch)
   real(r8), allocatable :: trc_rnof_step(:,:)         ! (ntracers, numpatch), this land step only

   ! Tracer carried by snowmelt sm from thin-snow (snl==0) scv during THERMAL.
   ! Set in CoLMMAIN after THERMAL based on the actual trc_scv reduction;
   ! consumed by tracer_soil_water when snl==0 .and. sm>0. Replaces the old
   ! "sm * R_precip" injection which used a fixed init delta instead of the
   ! real scv ratio.
   real(r8), allocatable :: trc_sm_carry(:,:)          ! (ntracers, numpatch)

   ! Non-steady-state leaf-water isotope memory for transpiration.
   ! These are diagnostic state variables for the isotope fractionation
   ! calculation; hydrology does not carry a prognostic leaf-tissue water pool.
   real(r8), allocatable :: trc_leaf_delta_e(:,:)      ! evaporation-site leaf water delta [permil]
   real(r8), allocatable :: trc_leaf_delta_b(:,:)      ! bulk leaf water delta [permil]
   real(r8), allocatable :: trc_leaf_peclet (:,:)      ! previous Peclet mixing factor [-]
   real(r8), allocatable :: trc_leaf_water_moles(:,:)  ! previous leaf water content [mol/m2]
   real(r8), allocatable :: trc_leaf_iso_storage(:,:)  ! NSS isotope storage anomaly [R*mm]
   logical,  allocatable :: trc_runtime_forced(:)      ! tracer has runtime atmospheric forcing

   ! LULCC snapshots for prognostic land-water tracer pools. These are
   ! saved before LULCC rebuilds landpatch and remapped after the new patch
   ! layout exists, mirroring the reactive/methane LULCC path.
   logical :: land_tracer_lulcc_snapshot_valid = .false.
   real(r8), allocatable :: lulcc_trc_ldew_rain_old(:,:)
   real(r8), allocatable :: lulcc_trc_ldew_snow_old(:,:)
   real(r8), allocatable :: lulcc_trc_wliq_soisno_old(:,:,:)
   real(r8), allocatable :: lulcc_trc_wice_soisno_old(:,:,:)
   real(r8), allocatable :: lulcc_trc_wa_old(:,:)
   real(r8), allocatable :: lulcc_trc_wdsrf_old(:,:)
   real(r8), allocatable :: lulcc_trc_wetwat_old(:,:)
   real(r8), allocatable :: lulcc_trc_surface_residue_old(:,:)
   real(r8), allocatable :: lulcc_trc_subsurface_residue_old(:,:)
   real(r8), allocatable :: lulcc_trc_solid_soisno_old(:,:,:)
   real(r8), allocatable :: lulcc_trc_canopy_solid_old(:,:)
   real(r8), allocatable :: lulcc_trc_surface_solid_old(:,:)
   real(r8), allocatable :: lulcc_trc_subsurface_solid_old(:,:)
   real(r8), allocatable :: lulcc_trc_waterstorage_solid_old(:,:)
   real(r8), allocatable :: lulcc_trc_waterstorage_old(:,:)
   real(r8), allocatable :: lulcc_trc_scv_old(:,:)
   real(r8), allocatable :: lulcc_trc_leaf_delta_e_old(:,:)
   real(r8), allocatable :: lulcc_trc_leaf_delta_b_old(:,:)
   real(r8), allocatable :: lulcc_trc_leaf_peclet_old(:,:)
   real(r8), allocatable :: lulcc_trc_leaf_water_moles_old(:,:)
   real(r8), allocatable :: lulcc_trc_leaf_iso_storage_old(:,:)

   real(r8), allocatable :: a_trc_precip   (:,:)
   ! Signed tracer mass exchanged with atmospheric vapour with NO net water
   ! flux (two-way equilibrium exchange on wet surfaces).  It is a genuine
   ! boundary flux, so it must be tracked separately from a_trc_precip: the
   ! latter is paired with a water flux and is cross-checked against
   ! water_input * R_init on fixed-signature tracers.
   real(r8), allocatable :: a_trc_vapor_exchange(:,:)
   real(r8), allocatable :: a_water_precip (:,:)
	   integer, parameter :: TRC_EVAP_KIND_TOTAL       = 0
	   integer, parameter :: TRC_EVAP_KIND_TRANSP      = 1
	   integer, parameter :: TRC_EVAP_KIND_SOILEVAP    = 2
	   integer, parameter :: TRC_EVAP_KIND_CANOPYEVAP  = 3
	   integer, parameter :: TRC_EVAP_KIND_SUBL        = 4
	   integer, parameter :: TRC_EVAP_KIND_WETLAND     = 5
	   real(r8), allocatable :: a_trc_evap     (:,:)
	   real(r8), allocatable :: a_water_evap_gross(:,:)
	   real(r8), allocatable :: a_trc_transp   (:,:)
	   real(r8), allocatable :: a_trc_transp_src(:,:)
	   real(r8), allocatable :: a_water_transp (:,:)
	   real(r8), allocatable :: a_trc_soilevap (:,:)
	   real(r8), allocatable :: a_water_soilevap(:,:)
	   real(r8), allocatable :: a_trc_canopyevap(:,:)
	   real(r8), allocatable :: a_water_canopyevap(:,:)
	   real(r8), allocatable :: a_trc_subl(:,:)
	   real(r8), allocatable :: a_water_subl(:,:)
	   real(r8), allocatable :: a_trc_wetland_evap(:,:)
	   real(r8), allocatable :: a_water_wetland_evap(:,:)
   real(r8), allocatable :: a_trc_rsur     (:,:)
   real(r8), allocatable :: a_trc_rsub     (:,:)
   real(r8), allocatable :: a_trc_rnof     (:,:)
   real(r8), allocatable :: a_water_rnof   (:,:)
   real(r8), allocatable :: a_trc_qinfl    (:,:)
   real(r8), allocatable :: a_trc_qcharge  (:,:)

   real(r8), allocatable :: trc_storage_beg(:,:)
   real(r8), allocatable :: trc_balance_err(:,:)
	   ! Net reactive source/sink applied during the current land step.
	   ! Positive adds tracer to storage; negative removes tracer.
	   real(r8), allocatable :: trc_reactive_source_step(:,:)
	   ! Explicit numerical source/sink used only to reconcile solver residuals
	   ! that have no physical flux signature. Positive adds tracer to storage.
	   real(r8), allocatable :: trc_numerical_residual_step(:,:)

   real(r8), allocatable :: a_trc_ldew_mass (:,:)
   real(r8), allocatable :: a_water_ldew    (:)
   real(r8), allocatable :: a_trc_soil_mass (:,:,:)  ! (ntracers, nl_soil, numpatch)
   real(r8), allocatable :: a_water_soil    (:,:)     ! (nl_soil, numpatch)
   real(r8), allocatable :: a_trc_snow_mass (:,:,:)  ! (ntracers, abs(maxsnl), numpatch) snow layers
   real(r8), allocatable :: a_water_snow    (:,:)     ! (abs(maxsnl), numpatch) snow layers

   ! Surface / aquifer / wetland / thin-snow diagnostic accumulators.
   ! tracer_balance_check already includes these four pools in the conservation
   ! sum, but the history writer only had canopy + soil + snow, so users could
   ! not see the δ values of wa (aquifer), wdsrf (surface water), wetwat
   ! (wetland pool) or scv (pre-layer thin snow). These accumulators let
   ! MOD_Hist emit a per-pool tracer ratio diagnostic alongside the existing
   ! ldew / soisno outputs.
	   real(r8), allocatable :: a_trc_wa_mass    (:,:)   ! (ntracers, numpatch)
	   real(r8), allocatable :: a_water_wa       (:)     ! (numpatch)
	   real(r8), allocatable :: a_trc_wa_debt_mass(:,:)  ! signed-debt magnitude diagnostics
	   real(r8), allocatable :: a_water_wa_debt   (:)    ! aquifer debt magnitude
   real(r8), allocatable :: a_trc_wdsrf_mass (:,:)
   real(r8), allocatable :: a_water_wdsrf    (:)
   real(r8), allocatable :: a_trc_wetwat_mass(:,:)
   real(r8), allocatable :: a_water_wetwat   (:)
   real(r8), allocatable :: a_trc_surface_residue_mass(:,:)
   real(r8), allocatable :: a_trc_subsurface_residue_mass(:,:)
   real(r8), allocatable :: a_trc_layer_dry_mass(:,:)
   real(r8), allocatable :: a_trc_solid_mass(:,:)
   real(r8), allocatable :: a_trc_scv_mass   (:,:)
   real(r8), allocatable :: a_water_scv      (:)

   ! External state contract.  LULCC snapshots and remapping work arrays stay
   ! private; only pools and diagnostics consumed by the land physics, restart,
   ! history, and host facades are exposed.
   PUBLIC :: allocate_Tracer_Vars, deallocate_Tracer_Vars, flush_Tracer_Acc
   PUBLIC :: save_land_tracer_lulcc_state, remap_land_tracer_lulcc_state
   PUBLIC :: zero_provider_owned_land_tracer_state, sync_tracer_patch_ratio
   PUBLIC :: tracer_book_evap_loss

   PUBLIC :: trc_ldew_rain, trc_ldew_snow
   PUBLIC :: trc_wliq_soisno, trc_wice_soisno
   PUBLIC :: trc_wa, trc_wdsrf, trc_wetwat
   PUBLIC :: trc_surface_residue, trc_subsurface_residue, trc_waterstorage
   PUBLIC :: trc_solid_soisno, trc_surface_solid, trc_subsurface_solid
   PUBLIC :: trc_canopy_solid, trc_waterstorage_solid
   PUBLIC :: trc_scv, trc_pg_rain_ground, trc_pg_snow_ground, trc_rnof_step
   PUBLIC :: trc_sm_carry, trc_runtime_forced
   PUBLIC :: trc_leaf_delta_e, trc_leaf_delta_b, trc_leaf_peclet
   PUBLIC :: trc_leaf_water_moles, trc_leaf_iso_storage
   PUBLIC :: trc_storage_beg, trc_balance_err
   PUBLIC :: trc_reactive_source_step, trc_numerical_residual_step

   PUBLIC :: TRC_EVAP_KIND_TRANSP, TRC_EVAP_KIND_SOILEVAP
   PUBLIC :: TRC_EVAP_KIND_CANOPYEVAP, TRC_EVAP_KIND_SUBL, TRC_EVAP_KIND_WETLAND
   PUBLIC :: a_trc_precip, a_water_precip, a_trc_evap, a_water_evap_gross
   PUBLIC :: a_trc_vapor_exchange
   PUBLIC :: a_trc_transp, a_trc_transp_src, a_water_transp
   PUBLIC :: a_trc_soilevap, a_water_soilevap
   PUBLIC :: a_trc_canopyevap, a_water_canopyevap
   PUBLIC :: a_trc_subl, a_water_subl, a_trc_wetland_evap, a_water_wetland_evap
   PUBLIC :: a_trc_rsur, a_trc_rsub, a_trc_rnof, a_water_rnof, a_trc_qinfl, a_trc_qcharge
   PUBLIC :: a_trc_ldew_mass, a_water_ldew
   PUBLIC :: a_trc_soil_mass, a_water_soil, a_trc_snow_mass, a_water_snow
   PUBLIC :: a_trc_wa_mass, a_water_wa, a_trc_wa_debt_mass, a_water_wa_debt
   PUBLIC :: a_trc_wdsrf_mass, a_water_wdsrf
   PUBLIC :: a_trc_wetwat_mass, a_water_wetwat
   PUBLIC :: a_trc_surface_residue_mass, a_trc_subsurface_residue_mass
   PUBLIC :: a_trc_layer_dry_mass, a_trc_solid_mass, a_trc_scv_mass, a_water_scv

CONTAINS

   SUBROUTINE allocate_Tracer_Vars (numpatch, maxsnl, nl_soil)
      IMPLICIT NONE
      integer, intent(in) :: numpatch, maxsnl, nl_soil
      integer :: itrc
      IF (ntracers <= 0 .or. numpatch <= 0) RETURN

      allocate(trc_ldew_rain   (ntracers, numpatch));           trc_ldew_rain   = 0._r8
      allocate(trc_ldew_snow   (ntracers, numpatch));           trc_ldew_snow   = 0._r8
      allocate(trc_wliq_soisno (ntracers, maxsnl+1:nl_soil, numpatch)); trc_wliq_soisno = 0._r8
      allocate(trc_wice_soisno (ntracers, maxsnl+1:nl_soil, numpatch)); trc_wice_soisno = 0._r8
      allocate(trc_wa          (ntracers, numpatch));           trc_wa          = 0._r8
      allocate(trc_wdsrf       (ntracers, numpatch));           trc_wdsrf       = 0._r8
      allocate(trc_wetwat      (ntracers, numpatch));           trc_wetwat      = 0._r8
      allocate(trc_surface_residue(ntracers, numpatch));        trc_surface_residue = 0._r8
      allocate(trc_subsurface_residue(ntracers, numpatch));     trc_subsurface_residue = 0._r8
      allocate(trc_solid_soisno(ntracers, maxsnl+1:nl_soil, numpatch)); trc_solid_soisno = 0._r8
      allocate(trc_canopy_solid(ntracers, numpatch));          trc_canopy_solid = 0._r8
      allocate(trc_surface_solid(ntracers, numpatch));          trc_surface_solid = 0._r8
      allocate(trc_subsurface_solid(ntracers, numpatch));       trc_subsurface_solid = 0._r8
      allocate(trc_waterstorage_solid(ntracers, numpatch));     trc_waterstorage_solid = 0._r8
      allocate(trc_scv         (ntracers, numpatch));           trc_scv         = 0._r8
      allocate(trc_waterstorage(ntracers, numpatch));           trc_waterstorage = 0._r8
      allocate(trc_pg_rain_ground(ntracers, numpatch));      trc_pg_rain_ground = 0._r8
      allocate(trc_pg_snow_ground(ntracers, numpatch));      trc_pg_snow_ground = 0._r8
      allocate(trc_rnof_step(ntracers, numpatch));           trc_rnof_step = 0._r8
      allocate(trc_sm_carry (ntracers, numpatch));           trc_sm_carry  = 0._r8
      allocate(trc_leaf_delta_e(ntracers, numpatch));        trc_leaf_delta_e = 0._r8
      allocate(trc_leaf_delta_b(ntracers, numpatch));        trc_leaf_delta_b = 0._r8
      allocate(trc_leaf_peclet (ntracers, numpatch));        trc_leaf_peclet  = 1._r8
      allocate(trc_leaf_water_moles(ntracers, numpatch));    trc_leaf_water_moles = 0._r8
      allocate(trc_leaf_iso_storage(ntracers, numpatch));    trc_leaf_iso_storage = 0._r8
      allocate(trc_runtime_forced(ntracers));                trc_runtime_forced = .false.
      DO itrc = 1, ntracers
         IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
         trc_leaf_delta_e(itrc, :) = tracers(itrc)%init_delta
         trc_leaf_delta_b(itrc, :) = tracers(itrc)%init_delta
      ENDDO

	      allocate(a_trc_precip    (ntracers, numpatch));           a_trc_precip    = 0._r8
	      allocate(a_trc_vapor_exchange(ntracers, numpatch));       a_trc_vapor_exchange = 0._r8
	      allocate(a_water_precip  (ntracers, numpatch));           a_water_precip  = 0._r8
		      allocate(a_trc_evap      (ntracers, numpatch));           a_trc_evap      = 0._r8
		      allocate(a_water_evap_gross(ntracers, numpatch));         a_water_evap_gross = 0._r8
		      allocate(a_trc_transp    (ntracers, numpatch));           a_trc_transp    = 0._r8
		      allocate(a_trc_transp_src(ntracers, numpatch));           a_trc_transp_src = 0._r8
		      allocate(a_water_transp  (ntracers, numpatch));           a_water_transp  = 0._r8
		      allocate(a_trc_soilevap  (ntracers, numpatch));           a_trc_soilevap  = 0._r8
		      allocate(a_water_soilevap(ntracers, numpatch));           a_water_soilevap = 0._r8
		      allocate(a_trc_canopyevap(ntracers, numpatch));           a_trc_canopyevap = 0._r8
		      allocate(a_water_canopyevap(ntracers, numpatch));         a_water_canopyevap = 0._r8
		      allocate(a_trc_subl      (ntracers, numpatch));           a_trc_subl      = 0._r8
		      allocate(a_water_subl    (ntracers, numpatch));           a_water_subl    = 0._r8
		      allocate(a_trc_wetland_evap(ntracers, numpatch));         a_trc_wetland_evap = 0._r8
		      allocate(a_water_wetland_evap(ntracers, numpatch));       a_water_wetland_evap = 0._r8
      allocate(a_trc_rsur      (ntracers, numpatch));           a_trc_rsur      = 0._r8
      allocate(a_trc_rsub      (ntracers, numpatch));           a_trc_rsub      = 0._r8
      allocate(a_trc_rnof      (ntracers, numpatch));           a_trc_rnof      = 0._r8
      allocate(a_water_rnof    (ntracers, numpatch));           a_water_rnof    = 0._r8
      allocate(a_trc_qinfl     (ntracers, numpatch));           a_trc_qinfl     = 0._r8
      allocate(a_trc_qcharge   (ntracers, numpatch));           a_trc_qcharge   = 0._r8

	      allocate(trc_storage_beg (ntracers, numpatch));           trc_storage_beg = 0._r8
	      allocate(trc_balance_err (ntracers, numpatch));           trc_balance_err = 0._r8
	      allocate(trc_reactive_source_step(ntracers, numpatch));   trc_reactive_source_step = 0._r8
	      allocate(trc_numerical_residual_step(ntracers, numpatch)); trc_numerical_residual_step = 0._r8

	      allocate(a_trc_ldew_mass (ntracers, numpatch));           a_trc_ldew_mass = 0._r8
      allocate(a_water_ldew    (numpatch));                     a_water_ldew    = 0._r8
      allocate(a_trc_soil_mass (ntracers, nl_soil, numpatch));  a_trc_soil_mass = 0._r8
      allocate(a_water_soil    (nl_soil, numpatch));             a_water_soil    = 0._r8
      allocate(a_trc_snow_mass (ntracers, abs(maxsnl), numpatch)); a_trc_snow_mass = 0._r8
      allocate(a_water_snow    (abs(maxsnl), numpatch));            a_water_snow    = 0._r8

	      allocate(a_trc_wa_mass    (ntracers, numpatch));          a_trc_wa_mass     = 0._r8
	      allocate(a_water_wa       (numpatch));                    a_water_wa        = 0._r8
	      allocate(a_trc_wa_debt_mass(ntracers, numpatch));         a_trc_wa_debt_mass = 0._r8
	      allocate(a_water_wa_debt   (numpatch));                   a_water_wa_debt    = 0._r8
      allocate(a_trc_wdsrf_mass (ntracers, numpatch));          a_trc_wdsrf_mass  = 0._r8
      allocate(a_water_wdsrf    (numpatch));                    a_water_wdsrf     = 0._r8
      allocate(a_trc_wetwat_mass(ntracers, numpatch));          a_trc_wetwat_mass = 0._r8
      allocate(a_water_wetwat   (numpatch));                    a_water_wetwat    = 0._r8
      allocate(a_trc_surface_residue_mass(ntracers, numpatch)); a_trc_surface_residue_mass = 0._r8
      allocate(a_trc_subsurface_residue_mass(ntracers, numpatch)); a_trc_subsurface_residue_mass = 0._r8
      allocate(a_trc_layer_dry_mass(ntracers, numpatch));       a_trc_layer_dry_mass = 0._r8
      allocate(a_trc_solid_mass(ntracers, numpatch));           a_trc_solid_mass = 0._r8
      allocate(a_trc_scv_mass   (ntracers, numpatch));          a_trc_scv_mass    = 0._r8
      allocate(a_water_scv      (numpatch));                    a_water_scv       = 0._r8
   END SUBROUTINE allocate_Tracer_Vars

   SUBROUTINE deallocate_Tracer_Vars ()
      IMPLICIT NONE
      IF (allocated(trc_ldew_rain  )) deallocate(trc_ldew_rain  )
      IF (allocated(trc_ldew_snow  )) deallocate(trc_ldew_snow  )
      IF (allocated(trc_wliq_soisno)) deallocate(trc_wliq_soisno)
      IF (allocated(trc_wice_soisno)) deallocate(trc_wice_soisno)
      IF (allocated(trc_wa         )) deallocate(trc_wa         )
      IF (allocated(trc_wdsrf      )) deallocate(trc_wdsrf      )
      IF (allocated(trc_wetwat     )) deallocate(trc_wetwat     )
      IF (allocated(trc_surface_residue)) deallocate(trc_surface_residue)
      IF (allocated(trc_subsurface_residue)) deallocate(trc_subsurface_residue)
      IF (allocated(trc_solid_soisno)) deallocate(trc_solid_soisno)
      IF (allocated(trc_canopy_solid)) deallocate(trc_canopy_solid)
      IF (allocated(trc_surface_solid)) deallocate(trc_surface_solid)
      IF (allocated(trc_subsurface_solid)) deallocate(trc_subsurface_solid)
      IF (allocated(trc_waterstorage_solid)) deallocate(trc_waterstorage_solid)
      IF (allocated(trc_scv        )) deallocate(trc_scv        )
      IF (allocated(trc_waterstorage)) deallocate(trc_waterstorage)
      IF (allocated(trc_pg_rain_ground)) deallocate(trc_pg_rain_ground)
      IF (allocated(trc_pg_snow_ground)) deallocate(trc_pg_snow_ground)
      IF (allocated(trc_rnof_step    )) deallocate(trc_rnof_step    )
      IF (allocated(trc_sm_carry     )) deallocate(trc_sm_carry     )
      IF (allocated(trc_leaf_delta_e )) deallocate(trc_leaf_delta_e )
      IF (allocated(trc_leaf_delta_b )) deallocate(trc_leaf_delta_b )
      IF (allocated(trc_leaf_peclet  )) deallocate(trc_leaf_peclet  )
      IF (allocated(trc_leaf_water_moles)) deallocate(trc_leaf_water_moles)
      IF (allocated(trc_leaf_iso_storage)) deallocate(trc_leaf_iso_storage)
      IF (allocated(trc_runtime_forced)) deallocate(trc_runtime_forced)
		      IF (allocated(a_trc_precip   )) deallocate(a_trc_precip   )
		      IF (allocated(a_trc_vapor_exchange)) deallocate(a_trc_vapor_exchange)
		      IF (allocated(a_water_precip )) deallocate(a_water_precip )
		      IF (allocated(a_trc_evap     )) deallocate(a_trc_evap     )
		      IF (allocated(a_water_evap_gross)) deallocate(a_water_evap_gross)
		      IF (allocated(a_trc_transp   )) deallocate(a_trc_transp   )
		      IF (allocated(a_trc_transp_src)) deallocate(a_trc_transp_src)
		      IF (allocated(a_water_transp )) deallocate(a_water_transp )
		      IF (allocated(a_trc_soilevap )) deallocate(a_trc_soilevap )
		      IF (allocated(a_water_soilevap)) deallocate(a_water_soilevap)
		      IF (allocated(a_trc_canopyevap)) deallocate(a_trc_canopyevap)
		      IF (allocated(a_water_canopyevap)) deallocate(a_water_canopyevap)
		      IF (allocated(a_trc_subl     )) deallocate(a_trc_subl     )
		      IF (allocated(a_water_subl   )) deallocate(a_water_subl   )
		      IF (allocated(a_trc_wetland_evap)) deallocate(a_trc_wetland_evap)
		      IF (allocated(a_water_wetland_evap)) deallocate(a_water_wetland_evap)
      IF (allocated(a_trc_rsur     )) deallocate(a_trc_rsur     )
      IF (allocated(a_trc_rsub     )) deallocate(a_trc_rsub     )
      IF (allocated(a_trc_rnof     )) deallocate(a_trc_rnof     )
      IF (allocated(a_water_rnof   )) deallocate(a_water_rnof   )
      IF (allocated(a_trc_qinfl    )) deallocate(a_trc_qinfl    )
      IF (allocated(a_trc_qcharge  )) deallocate(a_trc_qcharge  )
	      IF (allocated(trc_storage_beg)) deallocate(trc_storage_beg)
	      IF (allocated(trc_balance_err)) deallocate(trc_balance_err)
	      IF (allocated(trc_reactive_source_step)) deallocate(trc_reactive_source_step)
	      IF (allocated(trc_numerical_residual_step)) deallocate(trc_numerical_residual_step)
	      IF (allocated(a_trc_ldew_mass)) deallocate(a_trc_ldew_mass)
      IF (allocated(a_water_ldew   )) deallocate(a_water_ldew   )
      IF (allocated(a_trc_soil_mass)) deallocate(a_trc_soil_mass)
      IF (allocated(a_water_soil   )) deallocate(a_water_soil   )
      IF (allocated(a_trc_snow_mass)) deallocate(a_trc_snow_mass)
      IF (allocated(a_water_snow   )) deallocate(a_water_snow   )
	      IF (allocated(a_trc_wa_mass    )) deallocate(a_trc_wa_mass    )
	      IF (allocated(a_water_wa       )) deallocate(a_water_wa       )
	      IF (allocated(a_trc_wa_debt_mass)) deallocate(a_trc_wa_debt_mass)
	      IF (allocated(a_water_wa_debt   )) deallocate(a_water_wa_debt   )
      IF (allocated(a_trc_wdsrf_mass )) deallocate(a_trc_wdsrf_mass )
      IF (allocated(a_water_wdsrf    )) deallocate(a_water_wdsrf    )
      IF (allocated(a_trc_wetwat_mass)) deallocate(a_trc_wetwat_mass)
      IF (allocated(a_water_wetwat   )) deallocate(a_water_wetwat   )
      IF (allocated(a_trc_surface_residue_mass)) deallocate(a_trc_surface_residue_mass)
      IF (allocated(a_trc_subsurface_residue_mass)) deallocate(a_trc_subsurface_residue_mass)
      IF (allocated(a_trc_layer_dry_mass)) deallocate(a_trc_layer_dry_mass)
      IF (allocated(a_trc_solid_mass)) deallocate(a_trc_solid_mass)
      IF (allocated(a_trc_scv_mass   )) deallocate(a_trc_scv_mass   )
      IF (allocated(a_water_scv      )) deallocate(a_water_scv      )
   END SUBROUTINE deallocate_Tracer_Vars

   SUBROUTINE save_land_tracer_lulcc_state ()
      IMPLICIT NONE

      IF (.not. allocated(trc_ldew_rain) .or. .not. allocated(trc_wliq_soisno)) THEN
         land_tracer_lulcc_snapshot_valid = .false.
         RETURN
      ENDIF

      CALL clear_land_tracer_lulcc_snapshot()

      allocate(lulcc_trc_ldew_rain_old(size(trc_ldew_rain,1), size(trc_ldew_rain,2)))
      lulcc_trc_ldew_rain_old = trc_ldew_rain
      allocate(lulcc_trc_ldew_snow_old(size(trc_ldew_snow,1), size(trc_ldew_snow,2)))
      lulcc_trc_ldew_snow_old = trc_ldew_snow
      allocate(lulcc_trc_wliq_soisno_old(size(trc_wliq_soisno,1), &
         lbound(trc_wliq_soisno,2):ubound(trc_wliq_soisno,2), size(trc_wliq_soisno,3)))
      lulcc_trc_wliq_soisno_old = trc_wliq_soisno
      allocate(lulcc_trc_wice_soisno_old(size(trc_wice_soisno,1), &
         lbound(trc_wice_soisno,2):ubound(trc_wice_soisno,2), size(trc_wice_soisno,3)))
      lulcc_trc_wice_soisno_old = trc_wice_soisno
      allocate(lulcc_trc_wa_old(size(trc_wa,1), size(trc_wa,2)))
      lulcc_trc_wa_old = trc_wa
      allocate(lulcc_trc_wdsrf_old(size(trc_wdsrf,1), size(trc_wdsrf,2)))
      lulcc_trc_wdsrf_old = trc_wdsrf
      allocate(lulcc_trc_wetwat_old(size(trc_wetwat,1), size(trc_wetwat,2)))
      lulcc_trc_wetwat_old = trc_wetwat
      allocate(lulcc_trc_surface_residue_old(size(trc_surface_residue,1), size(trc_surface_residue,2)))
      lulcc_trc_surface_residue_old = trc_surface_residue
      allocate(lulcc_trc_subsurface_residue_old(size(trc_subsurface_residue,1), size(trc_subsurface_residue,2)))
      lulcc_trc_subsurface_residue_old = trc_subsurface_residue
      allocate(lulcc_trc_solid_soisno_old(size(trc_solid_soisno,1), &
         lbound(trc_solid_soisno,2):ubound(trc_solid_soisno,2), size(trc_solid_soisno,3)))
      lulcc_trc_solid_soisno_old = trc_solid_soisno
      allocate(lulcc_trc_canopy_solid_old(size(trc_canopy_solid,1), size(trc_canopy_solid,2)))
      lulcc_trc_canopy_solid_old = trc_canopy_solid
      allocate(lulcc_trc_surface_solid_old(size(trc_surface_solid,1), size(trc_surface_solid,2)))
      lulcc_trc_surface_solid_old = trc_surface_solid
      allocate(lulcc_trc_subsurface_solid_old(size(trc_subsurface_solid,1), size(trc_subsurface_solid,2)))
      lulcc_trc_subsurface_solid_old = trc_subsurface_solid
      allocate(lulcc_trc_waterstorage_solid_old(size(trc_waterstorage_solid,1), size(trc_waterstorage_solid,2)))
      lulcc_trc_waterstorage_solid_old = trc_waterstorage_solid
      allocate(lulcc_trc_waterstorage_old(size(trc_waterstorage,1), size(trc_waterstorage,2)))
      lulcc_trc_waterstorage_old = trc_waterstorage
      allocate(lulcc_trc_scv_old(size(trc_scv,1), size(trc_scv,2)))
      lulcc_trc_scv_old = trc_scv
      allocate(lulcc_trc_leaf_delta_e_old(size(trc_leaf_delta_e,1), size(trc_leaf_delta_e,2)))
      lulcc_trc_leaf_delta_e_old = trc_leaf_delta_e
      allocate(lulcc_trc_leaf_delta_b_old(size(trc_leaf_delta_b,1), size(trc_leaf_delta_b,2)))
      lulcc_trc_leaf_delta_b_old = trc_leaf_delta_b
      allocate(lulcc_trc_leaf_peclet_old(size(trc_leaf_peclet,1), size(trc_leaf_peclet,2)))
      lulcc_trc_leaf_peclet_old = trc_leaf_peclet
      allocate(lulcc_trc_leaf_water_moles_old(size(trc_leaf_water_moles,1), size(trc_leaf_water_moles,2)))
      lulcc_trc_leaf_water_moles_old = trc_leaf_water_moles
      allocate(lulcc_trc_leaf_iso_storage_old(size(trc_leaf_iso_storage,1), size(trc_leaf_iso_storage,2)))
      lulcc_trc_leaf_iso_storage_old = trc_leaf_iso_storage

      land_tracer_lulcc_snapshot_valid = .true.
   END SUBROUTINE save_land_tracer_lulcc_state

   SUBROUTINE remap_land_tracer_lulcc_state (patchclass_new, eindex_new, patchclass_old, eindex_old, &
                                             lccpct_patches, new_patch_area, old_patch_area)
      IMPLICIT NONE
      integer,   intent(in) :: patchclass_new(:), patchclass_old(:)
      integer*8, intent(in) :: eindex_new(:), eindex_old(:)
      real(r8),  intent(in), optional :: lccpct_patches(:,:)
      real(r8),  intent(in), optional :: new_patch_area(:), old_patch_area(:)

      integer :: nnew, maxsnl_saved, nl_soil_saved
      real(r8), allocatable :: mass_before(:), mass_after(:)
      logical :: check_lulcc_mass

      IF (.not. land_tracer_lulcc_snapshot_valid) RETURN
      IF (.not. allocated(lulcc_trc_wliq_soisno_old)) THEN
         CALL clear_land_tracer_lulcc_snapshot()
         RETURN
      ENDIF

      nnew = size(patchclass_new)
      maxsnl_saved = lbound(lulcc_trc_wliq_soisno_old, 2) - 1
      nl_soil_saved = ubound(lulcc_trc_wliq_soisno_old, 2)
      check_lulcc_mass = present(lccpct_patches) .and. present(new_patch_area) .and. &
         present(old_patch_area)
      IF (check_lulcc_mass) THEN
         allocate(mass_before(ntracers), mass_after(ntracers))
         CALL compute_lulcc_snapshot_land_water_mass(old_patch_area, mass_before)
      ENDIF

      IF (allocated(trc_ldew_rain)) CALL deallocate_Tracer_Vars()
      IF (nnew <= 0) THEN
         IF (allocated(mass_before)) deallocate(mass_before, mass_after)
         CALL clear_land_tracer_lulcc_snapshot()
         RETURN
      ENDIF

      CALL allocate_Tracer_Vars(nnew, maxsnl_saved, nl_soil_saved)
      IF (.not. allocated(trc_ldew_rain)) THEN
         IF (allocated(mass_before)) deallocate(mass_before, mass_after)
         CALL clear_land_tracer_lulcc_snapshot()
         RETURN
      ENDIF

      ! Extensive tracer pools are stored per unit patch area.  When both old
      ! and new patch areas are available, remap them by conserving area-mass:
      ! old_pool * old_area is partitioned into the target class fractions and
      ! divided by the new patch area.  Dimensionless/intensive diagnostics
      ! remain area-weighted means.
      CALL remap2d_mass(lulcc_trc_ldew_rain_old,         trc_ldew_rain)
      CALL remap2d_mass(lulcc_trc_ldew_snow_old,         trc_ldew_snow)
      CALL remap3d_mass(lulcc_trc_wliq_soisno_old,       trc_wliq_soisno)
      CALL remap3d_mass(lulcc_trc_wice_soisno_old,       trc_wice_soisno)
      CALL remap2d_mass(lulcc_trc_wa_old,                trc_wa)
      CALL remap2d_mass(lulcc_trc_wdsrf_old,             trc_wdsrf)
      CALL remap2d_mass(lulcc_trc_wetwat_old,            trc_wetwat)
      CALL remap2d_mass(lulcc_trc_surface_residue_old,    trc_surface_residue)
      CALL remap2d_mass(lulcc_trc_subsurface_residue_old, trc_subsurface_residue)
      CALL remap3d_mass(lulcc_trc_solid_soisno_old,       trc_solid_soisno)
      CALL remap2d_mass(lulcc_trc_canopy_solid_old,       trc_canopy_solid)
      CALL remap2d_mass(lulcc_trc_surface_solid_old,      trc_surface_solid)
      CALL remap2d_mass(lulcc_trc_subsurface_solid_old,   trc_subsurface_solid)
      CALL remap2d_mass(lulcc_trc_waterstorage_solid_old, trc_waterstorage_solid)
      CALL remap2d_mass(lulcc_trc_waterstorage_old,      trc_waterstorage)
      CALL remap2d_mass(lulcc_trc_scv_old,               trc_scv)
      CALL remap2d_intensive(lulcc_trc_leaf_delta_e_old,      trc_leaf_delta_e)
      CALL remap2d_intensive(lulcc_trc_leaf_delta_b_old,      trc_leaf_delta_b)
      CALL remap2d_intensive(lulcc_trc_leaf_peclet_old,       trc_leaf_peclet)
      CALL remap2d_mass(lulcc_trc_leaf_water_moles_old,  trc_leaf_water_moles)
      CALL remap2d_mass(lulcc_trc_leaf_iso_storage_old,  trc_leaf_iso_storage)

      IF (check_lulcc_mass) THEN
         CALL compute_current_lulcc_land_water_mass(new_patch_area, mass_after)
         CALL assert_lulcc_land_water_mass_conserved(mass_before, mass_after)
         deallocate(mass_before, mass_after)
      ENDIF

      CALL clear_land_tracer_lulcc_snapshot()

   CONTAINS
      SUBROUTINE compute_lulcc_snapshot_land_water_mass(area, total)
         real(r8), intent(in)  :: area(:)
         real(r8), intent(out) :: total(:)

         total = 0._r8
         IF (allocated(lulcc_trc_ldew_rain_old))    CALL accumulate_lulcc_mass_2d(lulcc_trc_ldew_rain_old, area, total)
         IF (allocated(lulcc_trc_ldew_snow_old))    CALL accumulate_lulcc_mass_2d(lulcc_trc_ldew_snow_old, area, total)
         IF (allocated(lulcc_trc_wliq_soisno_old))  CALL accumulate_lulcc_mass_3d(lulcc_trc_wliq_soisno_old, area, total)
         IF (allocated(lulcc_trc_wice_soisno_old))  CALL accumulate_lulcc_mass_3d(lulcc_trc_wice_soisno_old, area, total)
         IF (allocated(lulcc_trc_wa_old))           CALL accumulate_lulcc_mass_2d(lulcc_trc_wa_old, area, total)
         IF (allocated(lulcc_trc_wdsrf_old))        CALL accumulate_lulcc_mass_2d(lulcc_trc_wdsrf_old, area, total)
         IF (allocated(lulcc_trc_wetwat_old))       CALL accumulate_lulcc_mass_2d(lulcc_trc_wetwat_old, area, total)
         IF (allocated(lulcc_trc_surface_residue_old)) &
            CALL accumulate_lulcc_mass_2d(lulcc_trc_surface_residue_old, area, total)
         IF (allocated(lulcc_trc_subsurface_residue_old)) &
            CALL accumulate_lulcc_mass_2d(lulcc_trc_subsurface_residue_old, area, total)
         IF (allocated(lulcc_trc_solid_soisno_old)) &
            CALL accumulate_lulcc_mass_3d(lulcc_trc_solid_soisno_old, area, total)
         IF (allocated(lulcc_trc_canopy_solid_old)) &
            CALL accumulate_lulcc_mass_2d(lulcc_trc_canopy_solid_old, area, total)
         IF (allocated(lulcc_trc_surface_solid_old)) &
            CALL accumulate_lulcc_mass_2d(lulcc_trc_surface_solid_old, area, total)
         IF (allocated(lulcc_trc_subsurface_solid_old)) &
            CALL accumulate_lulcc_mass_2d(lulcc_trc_subsurface_solid_old, area, total)
         IF (allocated(lulcc_trc_waterstorage_solid_old)) &
            CALL accumulate_lulcc_mass_2d(lulcc_trc_waterstorage_solid_old, area, total)
         IF (allocated(lulcc_trc_waterstorage_old)) CALL accumulate_lulcc_mass_2d(lulcc_trc_waterstorage_old, area, total)
         IF (allocated(lulcc_trc_scv_old))          CALL accumulate_lulcc_mass_2d(lulcc_trc_scv_old, area, total)
      END SUBROUTINE compute_lulcc_snapshot_land_water_mass

      SUBROUTINE compute_current_lulcc_land_water_mass(area, total)
         real(r8), intent(in)  :: area(:)
         real(r8), intent(out) :: total(:)

         total = 0._r8
         IF (allocated(trc_ldew_rain))    CALL accumulate_lulcc_mass_2d(trc_ldew_rain, area, total)
         IF (allocated(trc_ldew_snow))    CALL accumulate_lulcc_mass_2d(trc_ldew_snow, area, total)
         IF (allocated(trc_wliq_soisno))  CALL accumulate_lulcc_mass_3d(trc_wliq_soisno, area, total)
         IF (allocated(trc_wice_soisno))  CALL accumulate_lulcc_mass_3d(trc_wice_soisno, area, total)
         IF (allocated(trc_wa))           CALL accumulate_lulcc_mass_2d(trc_wa, area, total)
         IF (allocated(trc_wdsrf))        CALL accumulate_lulcc_mass_2d(trc_wdsrf, area, total)
         IF (allocated(trc_wetwat))       CALL accumulate_lulcc_mass_2d(trc_wetwat, area, total)
         IF (allocated(trc_surface_residue)) CALL accumulate_lulcc_mass_2d(trc_surface_residue, area, total)
         IF (allocated(trc_subsurface_residue)) CALL accumulate_lulcc_mass_2d(trc_subsurface_residue, area, total)
         IF (allocated(trc_solid_soisno)) CALL accumulate_lulcc_mass_3d(trc_solid_soisno, area, total)
         IF (allocated(trc_canopy_solid)) CALL accumulate_lulcc_mass_2d(trc_canopy_solid, area, total)
         IF (allocated(trc_surface_solid)) CALL accumulate_lulcc_mass_2d(trc_surface_solid, area, total)
         IF (allocated(trc_subsurface_solid)) CALL accumulate_lulcc_mass_2d(trc_subsurface_solid, area, total)
         IF (allocated(trc_waterstorage_solid)) CALL accumulate_lulcc_mass_2d(trc_waterstorage_solid, area, total)
         IF (allocated(trc_waterstorage)) CALL accumulate_lulcc_mass_2d(trc_waterstorage, area, total)
         IF (allocated(trc_scv))          CALL accumulate_lulcc_mass_2d(trc_scv, area, total)
      END SUBROUTINE compute_current_lulcc_land_water_mass

      SUBROUTINE accumulate_lulcc_mass_2d(pool, area, total)
         real(r8), intent(in)    :: pool(:,:)
         real(r8), intent(in)    :: area(:)
         real(r8), intent(inout) :: total(:)
         integer :: ip, itrc, n1
         real(r8) :: patch_area

         n1 = min(size(pool,1), size(total), ntracers)
         DO ip = 1, min(size(pool,2), size(area))
            patch_area = max(0._r8, area(ip))
            IF (patch_area <= tiny(1._r8)) CYCLE
            DO itrc = 1, n1
               IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
               total(itrc) = total(itrc) + pool(itrc, ip) * patch_area
            ENDDO
         ENDDO
      END SUBROUTINE accumulate_lulcc_mass_2d

      SUBROUTINE accumulate_lulcc_mass_3d(pool, area, total)
         real(r8), intent(in)    :: pool(:,:,:)
         real(r8), intent(in)    :: area(:)
         real(r8), intent(inout) :: total(:)
         integer :: ip, itrc, iz, n1
         real(r8) :: patch_area

         n1 = min(size(pool,1), size(total), ntracers)
         DO ip = 1, min(size(pool,3), size(area))
            patch_area = max(0._r8, area(ip))
            IF (patch_area <= tiny(1._r8)) CYCLE
            DO iz = 1, size(pool,2)
               DO itrc = 1, n1
                  IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
                  total(itrc) = total(itrc) + pool(itrc, iz, ip) * patch_area
               ENDDO
            ENDDO
         ENDDO
      END SUBROUTINE accumulate_lulcc_mass_3d

      SUBROUTINE assert_lulcc_land_water_mass_conserved(before, after)
         real(r8), intent(in) :: before(:), after(:)
         integer :: itrc, nbad, worst_itrc
         real(r8) :: diff, scale, tol, abs_err, rel_err, worst_abs

         nbad = 0
         worst_itrc = 0
         worst_abs = 0._r8
         DO itrc = 1, min(size(before), size(after), ntracers)
            IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
            diff = after(itrc) - before(itrc)
            abs_err = abs(diff)
            scale = max(abs(before(itrc)), abs(after(itrc)), 1._r8)
            tol = max(lulcc_mass_abs_tol, lulcc_mass_rel_tol * scale)
            IF (abs_err > worst_abs) THEN
               worst_abs = abs_err
               worst_itrc = itrc
               rel_err = abs_err / scale
            ENDIF
            IF (abs_err > tol) nbad = nbad + 1
         ENDDO

         IF (nbad > DEF_TRACER_LULCC_ABORT_NBAD) THEN
            IF (p_is_io) THEN
               WRITE(*,'(A,I0,A,E12.5,A,E12.5)') &
                  'TRC_LULCC_BAL failed: nbad=', nbad, ' worst_abs=', worst_abs, &
                  ' worst_rel=', rel_err
               IF (worst_itrc > 0) THEN
                  WRITE(*,'(A,I0,3A,2(A,E12.5))') '  tracer=', worst_itrc, &
                     ' name=', trim(tracers(worst_itrc)%name), ' ', &
                     ' before=', before(worst_itrc), ' after=', after(worst_itrc)
               ENDIF
            ENDIF
            CALL CoLM_stop('TRACER LULCC remap land-water mass conservation failed')
         ENDIF
      END SUBROUTINE assert_lulcc_land_water_mass_conserved

      SUBROUTINE remap2d_mass(old, new)
         real(r8), intent(in)    :: old(:,:)
         real(r8), intent(inout) :: new(:,:)
         integer :: np, op, src, n1
         real(r8) :: w, wsum, denom
         logical :: conserve_area_mass
         real(r8) :: default_vals(size(new,1))

         n1 = min(size(new,1), size(old,1))
         DO np = 1, min(size(new,2), nnew)
            wsum = 0._r8
            default_vals(:) = new(:,np)
            IF (present(lccpct_patches)) THEN
               new(:,np) = 0._r8
               conserve_area_mass = area_mass_remap_available(np)
               DO op = 1, min(size(old,2), size(patchclass_old), size(eindex_old))
                  IF (eindex_old(op) /= eindex_new(np)) CYCLE
                  IF (patchclass_old(op) < lbound(lccpct_patches,2) .or. &
                      patchclass_old(op) > ubound(lccpct_patches,2)) CYCLE
                  IF (conserve_area_mass) THEN
                     w = lulcc_mass_transfer_area(np, op)
                  ELSE
                     w = lulcc_source_area_weight(np, op)
                  ENDIF
                  IF (w <= 0._r8) CYCLE
                  new(1:n1,np) = new(1:n1,np) + w * old(1:n1,op)
                  wsum = wsum + w
               ENDDO
            ENDIF
            IF (wsum > 0._r8) THEN
               denom = remap_denominator(np, wsum, conserve_area_mass)
               new(:,np) = new(:,np) / denom
            ELSE
               src = fallback_source(np, size(old,2))
               IF (src > 0) THEN
                  new(:,np) = 0._r8
                  new(1:n1,np) = old(1:n1,src)
               ELSE
                  new(:,np) = default_vals(:)
               ENDIF
            ENDIF
         ENDDO
      END SUBROUTINE remap2d_mass

      SUBROUTINE remap3d_mass(old, new)
         real(r8), intent(in)    :: old(:,:,:)
         real(r8), intent(inout) :: new(:,:,:)
         integer :: np, op, src, n1, n2
         real(r8) :: w, wsum, denom
         logical :: conserve_area_mass
         real(r8) :: default_vals(size(new,1), size(new,2))

         n1 = min(size(new,1), size(old,1))
         n2 = min(size(new,2), size(old,2))
         DO np = 1, min(size(new,3), nnew)
            wsum = 0._r8
            default_vals(:,:) = new(:,:,np)
            IF (present(lccpct_patches)) THEN
               new(:,:,np) = 0._r8
               conserve_area_mass = area_mass_remap_available(np)
               DO op = 1, min(size(old,3), size(patchclass_old), size(eindex_old))
                  IF (eindex_old(op) /= eindex_new(np)) CYCLE
                  IF (patchclass_old(op) < lbound(lccpct_patches,2) .or. &
                      patchclass_old(op) > ubound(lccpct_patches,2)) CYCLE
                  IF (conserve_area_mass) THEN
                     w = lulcc_mass_transfer_area(np, op)
                  ELSE
                     w = lulcc_source_area_weight(np, op)
                  ENDIF
                  IF (w <= 0._r8) CYCLE
                  new(1:n1,1:n2,np) = new(1:n1,1:n2,np) + w * old(1:n1,1:n2,op)
                  wsum = wsum + w
               ENDDO
            ENDIF
            IF (wsum > 0._r8) THEN
               denom = remap_denominator(np, wsum, conserve_area_mass)
               new(:,:,np) = new(:,:,np) / denom
            ELSE
               src = fallback_source(np, size(old,3))
               IF (src > 0) THEN
                  new(:,:,np) = 0._r8
                  new(1:n1,1:n2,np) = old(1:n1,1:n2,src)
               ELSE
                  new(:,:,np) = default_vals(:,:)
               ENDIF
            ENDIF
         ENDDO
      END SUBROUTINE remap3d_mass

      SUBROUTINE remap2d_intensive(old, new)
         real(r8), intent(in)    :: old(:,:)
         real(r8), intent(inout) :: new(:,:)
         integer :: np, op, src, n1
         real(r8) :: w, wsum
         real(r8) :: default_vals(size(new,1))

         n1 = min(size(new,1), size(old,1))
         DO np = 1, min(size(new,2), nnew)
            wsum = 0._r8
            default_vals(:) = new(:,np)
            IF (present(lccpct_patches)) THEN
               new(:,np) = 0._r8
               DO op = 1, min(size(old,2), size(patchclass_old), size(eindex_old))
                  IF (eindex_old(op) /= eindex_new(np)) CYCLE
                  IF (patchclass_old(op) < lbound(lccpct_patches,2) .or. &
                      patchclass_old(op) > ubound(lccpct_patches,2)) CYCLE
                  w = lulcc_source_area_weight(np, op)
                  IF (w <= 0._r8) CYCLE
                  new(1:n1,np) = new(1:n1,np) + w * old(1:n1,op)
                  wsum = wsum + w
               ENDDO
            ENDIF
            IF (wsum > 0._r8) THEN
               new(:,np) = new(:,np) / remap_denominator(np, wsum, .false.)
            ELSE
               src = fallback_source(np, size(old,2))
               IF (src > 0) THEN
                  new(:,np) = 0._r8
                  new(1:n1,np) = old(1:n1,src)
               ELSE
                  new(:,np) = default_vals(:)
               ENDIF
            ENDIF
         ENDDO
      END SUBROUTINE remap2d_intensive

      integer FUNCTION fallback_source(np, old_n) RESULT(src)
         integer, intent(in) :: np, old_n
         integer :: op
         src = 0
         DO op = 1, min(old_n, size(patchclass_old), size(eindex_old))
            IF (eindex_old(op) == eindex_new(np) .and. &
                patchclass_old(op) == patchclass_new(np)) THEN
               src = op
               RETURN
            ENDIF
         ENDDO
         ! Do not fall back across land-cover classes; a missing class match
         ! should cold-start from allocation defaults instead of contaminating
         ! a newly created patch with another surface type's tracer pools.
      END FUNCTION fallback_source

      REAL(r8) FUNCTION lulcc_source_area_weight(np, op) RESULT(w)
         integer, intent(in) :: np, op
         integer :: oq
         real(r8) :: class_area

         w = 0._r8
         IF (.not. present(lccpct_patches)) RETURN
         IF (np > size(lccpct_patches,1)) RETURN
         IF (patchclass_old(op) < lbound(lccpct_patches,2) .or. &
             patchclass_old(op) > ubound(lccpct_patches,2)) RETURN
         w = max(0._r8, lccpct_patches(np, patchclass_old(op)))
         IF (w <= 0._r8) RETURN
         IF (.not. present(old_patch_area)) RETURN
         IF (op > size(old_patch_area)) RETURN

         class_area = 0._r8
         DO oq = 1, min(size(patchclass_old), size(eindex_old), size(old_patch_area))
            IF (eindex_old(oq) == eindex_new(np) .and. &
                patchclass_old(oq) == patchclass_old(op)) THEN
               class_area = class_area + max(0._r8, old_patch_area(oq))
            ENDIF
         ENDDO
         IF (class_area > tiny(1._r8)) THEN
            w = w * max(0._r8, old_patch_area(op)) / class_area
         ENDIF
      END FUNCTION lulcc_source_area_weight

      LOGICAL FUNCTION area_mass_remap_available(np) RESULT(ok)
         integer, intent(in) :: np

         ok = present(lccpct_patches) .and. present(new_patch_area) .and. present(old_patch_area)
         IF (.not. ok) THEN
            CALL warn_lulcc_area_fallback('missing old/new patch area arrays')
            RETURN
         ENDIF
         ok = np <= size(new_patch_area)
         IF (.not. ok) THEN
            CALL warn_lulcc_area_fallback('new patch index exceeds new_patch_area size')
            RETURN
         ENDIF
         ok = new_patch_area(np) > tiny(1._r8)
         IF (.not. ok) CALL warn_lulcc_area_fallback('new patch area is zero')
      END FUNCTION area_mass_remap_available

      SUBROUTINE warn_lulcc_area_fallback(reason)
         character(len=*), intent(in) :: reason

         IF (lulcc_area_fallback_warned) RETURN
         lulcc_area_fallback_warned = .true.
         IF (p_is_io) THEN
            WRITE(*,'(A,A)') 'TRACER LULCC remap falling back to source-area weighting: ', &
               trim(reason)
            WRITE(*,'(A)') '  Land-water tracer area-mass conservation requires old and new patch areas.'
         ENDIF
      END SUBROUTINE warn_lulcc_area_fallback

      REAL(r8) FUNCTION lulcc_mass_transfer_area(np, op) RESULT(w)
         integer, intent(in) :: np, op
         integer :: c, nq
         real(r8) :: target_area, class_target_area

         w = 0._r8
         IF (.not. area_mass_remap_available(np)) RETURN
         IF (op > size(old_patch_area)) RETURN
         IF (op > size(patchclass_old) .or. op > size(eindex_old)) RETURN
         c = patchclass_old(op)
         IF (c < lbound(lccpct_patches,2) .or. c > ubound(lccpct_patches,2)) RETURN

         target_area = lulcc_target_class_area(np, c)
         IF (target_area <= tiny(1._r8)) RETURN

         class_target_area = 0._r8
         DO nq = 1, min(nnew, size(eindex_new), size(new_patch_area))
            IF (eindex_new(nq) == eindex_new(np)) THEN
               class_target_area = class_target_area + lulcc_target_class_area(nq, c)
            ENDIF
         ENDDO
         IF (class_target_area <= tiny(1._r8)) RETURN

         ! Area of old patch op that is transferred into target patch np.
         ! Summing this weight over all target patches of the same element and
         ! source class returns old_patch_area(op), so extensive tracer mass is
         ! conserved when target area is non-zero.
         w = max(0._r8, old_patch_area(op)) * target_area / class_target_area
      END FUNCTION lulcc_mass_transfer_area

      REAL(r8) FUNCTION lulcc_target_class_area(np, c) RESULT(area)
         integer, intent(in) :: np, c
         integer :: cc
         real(r8) :: class_sum

         area = 0._r8
         IF (.not. present(lccpct_patches)) RETURN
         IF (.not. present(new_patch_area)) RETURN
         IF (np > size(new_patch_area)) RETURN
         IF (np > size(lccpct_patches,1)) RETURN
         IF (c < lbound(lccpct_patches,2) .or. c > ubound(lccpct_patches,2)) RETURN

         class_sum = 0._r8
         DO cc = lbound(lccpct_patches,2), ubound(lccpct_patches,2)
            class_sum = class_sum + max(0._r8, lccpct_patches(np, cc))
         ENDDO
         IF (class_sum <= tiny(1._r8)) RETURN

         area = max(0._r8, new_patch_area(np)) * max(0._r8, lccpct_patches(np, c)) / class_sum
      END FUNCTION lulcc_target_class_area

      REAL(r8) FUNCTION remap_denominator(np, wsum, conserve_mass) RESULT(denom)
         integer, intent(in) :: np
         real(r8), intent(in) :: wsum
         logical, intent(in) :: conserve_mass

         denom = max(wsum, tiny(1._r8))
         IF (conserve_mass .and. present(new_patch_area)) THEN
            IF (np <= size(new_patch_area) .and. new_patch_area(np) > tiny(1._r8)) THEN
               denom = new_patch_area(np)
            ENDIF
         ENDIF
      END FUNCTION remap_denominator
   END SUBROUTINE remap_land_tracer_lulcc_state

   SUBROUTINE clear_land_tracer_lulcc_snapshot ()
      IMPLICIT NONE

      IF (allocated(lulcc_trc_ldew_rain_old)) deallocate(lulcc_trc_ldew_rain_old)
      IF (allocated(lulcc_trc_ldew_snow_old)) deallocate(lulcc_trc_ldew_snow_old)
      IF (allocated(lulcc_trc_wliq_soisno_old)) deallocate(lulcc_trc_wliq_soisno_old)
      IF (allocated(lulcc_trc_wice_soisno_old)) deallocate(lulcc_trc_wice_soisno_old)
      IF (allocated(lulcc_trc_wa_old)) deallocate(lulcc_trc_wa_old)
      IF (allocated(lulcc_trc_wdsrf_old)) deallocate(lulcc_trc_wdsrf_old)
      IF (allocated(lulcc_trc_wetwat_old)) deallocate(lulcc_trc_wetwat_old)
      IF (allocated(lulcc_trc_surface_residue_old)) deallocate(lulcc_trc_surface_residue_old)
      IF (allocated(lulcc_trc_subsurface_residue_old)) deallocate(lulcc_trc_subsurface_residue_old)
      IF (allocated(lulcc_trc_solid_soisno_old)) deallocate(lulcc_trc_solid_soisno_old)
      IF (allocated(lulcc_trc_canopy_solid_old)) deallocate(lulcc_trc_canopy_solid_old)
      IF (allocated(lulcc_trc_surface_solid_old)) deallocate(lulcc_trc_surface_solid_old)
      IF (allocated(lulcc_trc_subsurface_solid_old)) deallocate(lulcc_trc_subsurface_solid_old)
      IF (allocated(lulcc_trc_waterstorage_solid_old)) deallocate(lulcc_trc_waterstorage_solid_old)
      IF (allocated(lulcc_trc_waterstorage_old)) deallocate(lulcc_trc_waterstorage_old)
      IF (allocated(lulcc_trc_scv_old)) deallocate(lulcc_trc_scv_old)
      IF (allocated(lulcc_trc_leaf_delta_e_old)) deallocate(lulcc_trc_leaf_delta_e_old)
      IF (allocated(lulcc_trc_leaf_delta_b_old)) deallocate(lulcc_trc_leaf_delta_b_old)
      IF (allocated(lulcc_trc_leaf_peclet_old)) deallocate(lulcc_trc_leaf_peclet_old)
      IF (allocated(lulcc_trc_leaf_water_moles_old)) deallocate(lulcc_trc_leaf_water_moles_old)
      IF (allocated(lulcc_trc_leaf_iso_storage_old)) deallocate(lulcc_trc_leaf_iso_storage_old)
      land_tracer_lulcc_snapshot_valid = .false.
   END SUBROUTINE clear_land_tracer_lulcc_snapshot

   SUBROUTINE flush_Tracer_Acc ()
      IMPLICIT NONE
	      IF (allocated(a_trc_precip )) a_trc_precip  = 0._r8
	      IF (allocated(a_trc_vapor_exchange)) a_trc_vapor_exchange = 0._r8
	      IF (allocated(a_water_precip)) a_water_precip = 0._r8
		      IF (allocated(a_trc_evap   )) a_trc_evap    = 0._r8
		      IF (allocated(a_water_evap_gross)) a_water_evap_gross = 0._r8
		      IF (allocated(a_trc_transp )) a_trc_transp  = 0._r8
		      IF (allocated(a_trc_transp_src)) a_trc_transp_src = 0._r8
		      IF (allocated(a_water_transp)) a_water_transp = 0._r8
		      IF (allocated(a_trc_soilevap)) a_trc_soilevap = 0._r8
		      IF (allocated(a_water_soilevap)) a_water_soilevap = 0._r8
		      IF (allocated(a_trc_canopyevap)) a_trc_canopyevap = 0._r8
		      IF (allocated(a_water_canopyevap)) a_water_canopyevap = 0._r8
		      IF (allocated(a_trc_subl    )) a_trc_subl = 0._r8
		      IF (allocated(a_water_subl  )) a_water_subl = 0._r8
		      IF (allocated(a_trc_wetland_evap)) a_trc_wetland_evap = 0._r8
		      IF (allocated(a_water_wetland_evap)) a_water_wetland_evap = 0._r8
      IF (allocated(a_trc_rsur   )) a_trc_rsur    = 0._r8
      IF (allocated(a_trc_rsub   )) a_trc_rsub    = 0._r8
      IF (allocated(a_trc_rnof   )) a_trc_rnof    = 0._r8
      IF (allocated(a_water_rnof )) a_water_rnof  = 0._r8
      IF (allocated(a_trc_qinfl  )) a_trc_qinfl   = 0._r8
      IF (allocated(a_trc_qcharge)) a_trc_qcharge  = 0._r8
      IF (allocated(a_trc_ldew_mass)) a_trc_ldew_mass = 0._r8
      IF (allocated(a_water_ldew   )) a_water_ldew    = 0._r8
      IF (allocated(a_trc_soil_mass)) a_trc_soil_mass = 0._r8
      IF (allocated(a_water_soil   )) a_water_soil    = 0._r8
      IF (allocated(a_trc_snow_mass)) a_trc_snow_mass = 0._r8
      IF (allocated(a_water_snow   )) a_water_snow    = 0._r8
	      IF (allocated(a_trc_wa_mass    )) a_trc_wa_mass     = 0._r8
	      IF (allocated(a_water_wa       )) a_water_wa        = 0._r8
	      IF (allocated(a_trc_wa_debt_mass)) a_trc_wa_debt_mass = 0._r8
	      IF (allocated(a_water_wa_debt   )) a_water_wa_debt    = 0._r8
      IF (allocated(a_trc_wdsrf_mass )) a_trc_wdsrf_mass  = 0._r8
      IF (allocated(a_water_wdsrf    )) a_water_wdsrf     = 0._r8
      IF (allocated(a_trc_wetwat_mass)) a_trc_wetwat_mass = 0._r8
      IF (allocated(a_trc_surface_residue_mass)) a_trc_surface_residue_mass = 0._r8
      IF (allocated(a_trc_subsurface_residue_mass)) a_trc_subsurface_residue_mass = 0._r8
      IF (allocated(a_trc_layer_dry_mass)) a_trc_layer_dry_mass = 0._r8
      IF (allocated(a_trc_solid_mass)) a_trc_solid_mass = 0._r8
      IF (allocated(a_water_wetwat   )) a_water_wetwat    = 0._r8
      IF (allocated(a_trc_scv_mass   )) a_trc_scv_mass    = 0._r8
      IF (allocated(a_water_scv      )) a_water_scv       = 0._r8
      IF (allocated(trc_pg_rain_ground)) trc_pg_rain_ground = 0._r8
      IF (allocated(trc_pg_snow_ground)) trc_pg_snow_ground = 0._r8
      IF (allocated(trc_rnof_step    )) trc_rnof_step     = 0._r8
	      IF (allocated(trc_sm_carry     )) trc_sm_carry      = 0._r8
	      IF (allocated(trc_reactive_source_step)) trc_reactive_source_step = 0._r8
	      IF (allocated(trc_numerical_residual_step)) trc_numerical_residual_step = 0._r8
	   END SUBROUTINE flush_Tracer_Acc

   SUBROUTINE zero_provider_owned_land_tracer_state ()
      IMPLICIT NONE
      integer :: itrc

      IF (ntracers <= 0) RETURN

      DO itrc = 1, ntracers
         IF (tracer_uses_land_water_transport(itrc)) CYCLE

         IF (allocated(trc_ldew_rain  )) trc_ldew_rain  (itrc, :)    = 0._r8
         IF (allocated(trc_ldew_snow  )) trc_ldew_snow  (itrc, :)    = 0._r8
         IF (allocated(trc_wliq_soisno)) trc_wliq_soisno(itrc, :, :) = 0._r8
         IF (allocated(trc_wice_soisno)) trc_wice_soisno(itrc, :, :) = 0._r8
         IF (allocated(trc_wa         )) trc_wa         (itrc, :)    = 0._r8
         IF (allocated(trc_wdsrf      )) trc_wdsrf      (itrc, :)    = 0._r8
         IF (allocated(trc_wetwat     )) trc_wetwat     (itrc, :)    = 0._r8
         IF (allocated(trc_surface_residue)) trc_surface_residue(itrc, :) = 0._r8
         IF (allocated(trc_subsurface_residue)) trc_subsurface_residue(itrc, :) = 0._r8
         IF (allocated(trc_solid_soisno)) trc_solid_soisno(itrc, :, :) = 0._r8
         IF (allocated(trc_canopy_solid)) trc_canopy_solid(itrc, :) = 0._r8
         IF (allocated(trc_surface_solid)) trc_surface_solid(itrc, :) = 0._r8
         IF (allocated(trc_subsurface_solid)) trc_subsurface_solid(itrc, :) = 0._r8
         IF (allocated(trc_waterstorage_solid)) trc_waterstorage_solid(itrc, :) = 0._r8
         IF (allocated(trc_scv        )) trc_scv        (itrc, :)    = 0._r8
         IF (allocated(trc_waterstorage)) trc_waterstorage(itrc, :)  = 0._r8
         IF (allocated(trc_pg_rain_ground)) trc_pg_rain_ground(itrc, :) = 0._r8
         IF (allocated(trc_pg_snow_ground)) trc_pg_snow_ground(itrc, :) = 0._r8
         IF (allocated(trc_rnof_step    )) trc_rnof_step    (itrc, :) = 0._r8
         IF (allocated(trc_sm_carry     )) trc_sm_carry     (itrc, :) = 0._r8
         IF (allocated(trc_leaf_delta_e )) trc_leaf_delta_e (itrc, :) = 0._r8
         IF (allocated(trc_leaf_delta_b )) trc_leaf_delta_b (itrc, :) = 0._r8
         IF (allocated(trc_leaf_peclet  )) trc_leaf_peclet  (itrc, :) = 1._r8
         IF (allocated(trc_leaf_water_moles)) trc_leaf_water_moles(itrc, :) = 0._r8
         IF (allocated(trc_leaf_iso_storage)) trc_leaf_iso_storage(itrc, :) = 0._r8

         IF (allocated(a_trc_precip   )) a_trc_precip   (itrc, :) = 0._r8
         IF (allocated(a_trc_vapor_exchange)) a_trc_vapor_exchange(itrc, :) = 0._r8
         IF (allocated(a_water_precip )) a_water_precip (itrc, :) = 0._r8
         IF (allocated(a_trc_evap     )) a_trc_evap     (itrc, :) = 0._r8
         IF (allocated(a_water_evap_gross)) a_water_evap_gross(itrc, :) = 0._r8
         IF (allocated(a_trc_transp   )) a_trc_transp   (itrc, :) = 0._r8
         IF (allocated(a_trc_transp_src)) a_trc_transp_src(itrc, :) = 0._r8
         IF (allocated(a_water_transp )) a_water_transp (itrc, :) = 0._r8
         IF (allocated(a_trc_soilevap )) a_trc_soilevap (itrc, :) = 0._r8
         IF (allocated(a_water_soilevap)) a_water_soilevap(itrc, :) = 0._r8
         IF (allocated(a_trc_canopyevap)) a_trc_canopyevap(itrc, :) = 0._r8
         IF (allocated(a_water_canopyevap)) a_water_canopyevap(itrc, :) = 0._r8
         IF (allocated(a_trc_subl     )) a_trc_subl     (itrc, :) = 0._r8
         IF (allocated(a_water_subl   )) a_water_subl   (itrc, :) = 0._r8
         IF (allocated(a_trc_wetland_evap)) a_trc_wetland_evap(itrc, :) = 0._r8
         IF (allocated(a_water_wetland_evap)) a_water_wetland_evap(itrc, :) = 0._r8
         IF (allocated(a_trc_rsur     )) a_trc_rsur     (itrc, :) = 0._r8
         IF (allocated(a_trc_rsub     )) a_trc_rsub     (itrc, :) = 0._r8
         IF (allocated(a_trc_rnof     )) a_trc_rnof     (itrc, :) = 0._r8
         IF (allocated(a_water_rnof   )) a_water_rnof   (itrc, :) = 0._r8
         IF (allocated(a_trc_qinfl    )) a_trc_qinfl    (itrc, :) = 0._r8
         IF (allocated(a_trc_qcharge  )) a_trc_qcharge  (itrc, :) = 0._r8
         IF (allocated(trc_storage_beg)) trc_storage_beg(itrc, :) = 0._r8
         IF (allocated(trc_balance_err)) trc_balance_err(itrc, :) = 0._r8
         IF (allocated(trc_reactive_source_step)) trc_reactive_source_step(itrc, :) = 0._r8
         IF (allocated(trc_numerical_residual_step)) trc_numerical_residual_step(itrc, :) = 0._r8
         IF (allocated(a_trc_ldew_mass)) a_trc_ldew_mass(itrc, :) = 0._r8
         IF (allocated(a_trc_soil_mass)) a_trc_soil_mass(itrc, :, :) = 0._r8
         IF (allocated(a_trc_snow_mass)) a_trc_snow_mass(itrc, :, :) = 0._r8
         IF (allocated(a_trc_wa_mass    )) a_trc_wa_mass    (itrc, :) = 0._r8
         IF (allocated(a_trc_wa_debt_mass)) a_trc_wa_debt_mass(itrc, :) = 0._r8
         IF (allocated(a_trc_wdsrf_mass )) a_trc_wdsrf_mass (itrc, :) = 0._r8
         IF (allocated(a_trc_wetwat_mass)) a_trc_wetwat_mass(itrc, :) = 0._r8
         IF (allocated(a_trc_surface_residue_mass)) a_trc_surface_residue_mass(itrc, :) = 0._r8
         IF (allocated(a_trc_subsurface_residue_mass)) a_trc_subsurface_residue_mass(itrc, :) = 0._r8
         IF (allocated(a_trc_layer_dry_mass)) a_trc_layer_dry_mass(itrc, :) = 0._r8
         IF (allocated(a_trc_solid_mass)) a_trc_solid_mass(itrc, :) = 0._r8
         IF (allocated(a_trc_scv_mass   )) a_trc_scv_mass   (itrc, :) = 0._r8
      ENDDO
   END SUBROUTINE zero_provider_owned_land_tracer_state

	   SUBROUTINE tracer_book_evap_loss (itrc, ipatch, tracer_mass, water_mass, evap_kind)
	      IMPLICIT NONE
	      integer,  intent(in) :: itrc, ipatch
	      real(r8), intent(in) :: tracer_mass, water_mass
	      integer,  intent(in), optional :: evap_kind
	      integer :: kind

	      IF (ntracers <= 0) RETURN
	      IF (.not. allocated(a_trc_evap)) RETURN
	      kind = TRC_EVAP_KIND_TOTAL
	      IF (present(evap_kind)) kind = evap_kind

	      a_trc_evap(itrc, ipatch) = a_trc_evap(itrc, ipatch) + tracer_mass
	      IF (allocated(a_water_evap_gross)) THEN
	         a_water_evap_gross(itrc, ipatch) = a_water_evap_gross(itrc, ipatch) &
	            + max(water_mass, 0._r8)
	      ENDIF

	      SELECT CASE (kind)
	      CASE (TRC_EVAP_KIND_TRANSP)
	         IF (allocated(a_trc_transp)) a_trc_transp(itrc, ipatch) = &
	            a_trc_transp(itrc, ipatch) + tracer_mass
	         IF (allocated(a_water_transp)) a_water_transp(itrc, ipatch) = &
	            a_water_transp(itrc, ipatch) + max(water_mass, 0._r8)
	      CASE (TRC_EVAP_KIND_SOILEVAP)
	         IF (allocated(a_trc_soilevap)) a_trc_soilevap(itrc, ipatch) = &
	            a_trc_soilevap(itrc, ipatch) + tracer_mass
	         IF (allocated(a_water_soilevap)) a_water_soilevap(itrc, ipatch) = &
	            a_water_soilevap(itrc, ipatch) + max(water_mass, 0._r8)
	      CASE (TRC_EVAP_KIND_CANOPYEVAP)
	         IF (allocated(a_trc_canopyevap)) a_trc_canopyevap(itrc, ipatch) = &
	            a_trc_canopyevap(itrc, ipatch) + tracer_mass
	         IF (allocated(a_water_canopyevap)) a_water_canopyevap(itrc, ipatch) = &
	            a_water_canopyevap(itrc, ipatch) + max(water_mass, 0._r8)
	      CASE (TRC_EVAP_KIND_SUBL)
	         IF (allocated(a_trc_subl)) a_trc_subl(itrc, ipatch) = &
	            a_trc_subl(itrc, ipatch) + tracer_mass
	         IF (allocated(a_water_subl)) a_water_subl(itrc, ipatch) = &
	            a_water_subl(itrc, ipatch) + max(water_mass, 0._r8)
	      CASE (TRC_EVAP_KIND_WETLAND)
	         IF (allocated(a_trc_wetland_evap)) a_trc_wetland_evap(itrc, ipatch) = &
	            a_trc_wetland_evap(itrc, ipatch) + tracer_mass
	         IF (allocated(a_water_wetland_evap)) a_water_wetland_evap(itrc, ipatch) = &
	            a_water_wetland_evap(itrc, ipatch) + max(water_mass, 0._r8)
	      END SELECT
	   END SUBROUTINE tracer_book_evap_loss

   !---------------------------------------------------------------
   ! Rebuild one tracer's prognostic storage from a caller-owned
   ! mixed-box ratio. This is used by patch branches that do not run
   ! the full tracer hydrology pipeline (glacier / waterbody) but can
   ! no longer assume a fixed R_init signature when runtime forcing or
   ! fractionation is enabled.
   !
   ! `snl` is the accounting lower bound used by tracer_save_storage:
   ! layers below snl+1 are zeroed so hidden snow-layer tracer cannot
   ! leak outside the caller's conservation account.
   !---------------------------------------------------------------
   SUBROUTINE sync_tracer_patch_ratio (itrc, ipatch, snl, maxsnl, nl_soil, &
      wliq_soisno, wice_soisno, wa, wdsrf, scv, R_mix)
      IMPLICIT NONE
      integer,  intent(in) :: itrc, ipatch, snl, maxsnl, nl_soil
      real(r8), intent(in) :: wliq_soisno(maxsnl+1:nl_soil)
      real(r8), intent(in) :: wice_soisno(maxsnl+1:nl_soil)
      real(r8), intent(in) :: wa, wdsrf, scv, R_mix

      integer :: j

      IF (ntracers <= 0) RETURN
      IF (.not. allocated(trc_wliq_soisno)) RETURN
      IF (itrc < 1 .or. itrc > ntracers) RETURN

      IF (allocated(trc_ldew_rain)) trc_ldew_rain(itrc, ipatch) = 0._r8
      IF (allocated(trc_ldew_snow)) trc_ldew_snow(itrc, ipatch) = 0._r8

      DO j = maxsnl + 1, nl_soil
         IF (j >= snl + 1) THEN
            trc_wliq_soisno(itrc, j, ipatch) = max(wliq_soisno(j), 0._r8) * R_mix
            trc_wice_soisno(itrc, j, ipatch) = max(wice_soisno(j), 0._r8) * R_mix
            CALL tracer_equilibrate_dissolved(itrc, max(wliq_soisno(j), 0._r8), &
               trc_wliq_soisno(itrc, j, ipatch), trc_solid_soisno(itrc, j, ipatch))
         ELSE
            trc_wliq_soisno(itrc, j, ipatch) = 0._r8
            trc_wice_soisno(itrc, j, ipatch) = 0._r8
         ENDIF
      ENDDO

      trc_wa   (itrc, ipatch) = wa * R_mix
      trc_wdsrf(itrc, ipatch) = max(wdsrf, 0._r8) * R_mix
      CALL tracer_equilibrate_dissolved(itrc, wa, trc_wa(itrc, ipatch), &
         trc_subsurface_solid(itrc, ipatch))
      CALL tracer_equilibrate_dissolved(itrc, max(wdsrf, 0._r8), &
         trc_wdsrf(itrc, ipatch), trc_surface_solid(itrc, ipatch))

      IF (snl < 0) THEN
         trc_scv(itrc, ipatch) = 0._r8
      ELSE
         trc_scv(itrc, ipatch) = max(scv, 0._r8) * R_mix
      ENDIF
   END SUBROUTINE sync_tracer_patch_ratio

END MODULE MOD_Tracer_Vars
#endif
