#include <define.h>

#ifdef GridRiverLakeFlow
MODULE MOD_Grid_RiverLakeFlow
!-------------------------------------------------------------------------------------
! DESCRIPTION:
!
!   River Lake flow.
!
!-------------------------------------------------------------------------------------

   USE MOD_Precision
   USE MOD_SPMD_Task
   USE MOD_Namelist
   USE MOD_Grid_RiverLakeNetwork
   USE MOD_Grid_RiverLakeTimeVars
   USE MOD_Grid_Reservoir
   USE MOD_Grid_RiverLakeHist
   USE MOD_Grid_RiverLakeLevee, only: has_levee, levsto, levdph, levee_init, read_levee_restart, &
      levee_visible_volume_from_stage, levee_repartition_storage, levee_apply_protected_flux, levee_final, &
      levee_fldstg
   USE MOD_Grid_RiverLakeBifurcation, only: bifurcation_init, read_bifurcation_restart, &
      bifurcation_final, bifurcation_calc, bifurcation_invalidate_static_dn, &
      bif_hflux_sum, bif_hflux_lev, bif_lev_hflux_sum, bif_path_active
   USE MOD_Tracer_Lifecycle, only: tracer_lifecycle_route_has_active, tracer_lifecycle_route_init, &
      tracer_lifecycle_route_calc, tracer_lifecycle_route_final, tracer_lifecycle_route_diag_accumulate, &
      tracer_lifecycle_route_forcing_put, tracer_lifecycle_route_read_restart, &
      tracer_lifecycle_route_sediment_bif_accumulate, tracer_lifecycle_route_sediment_levee_repartition, &
      tracer_lifecycle_publish_levee_flood_patch, tracer_lifecycle_publish_flood_patch, &
      tracer_lifecycle_has_levee_flood_publisher, tracer_lifecycle_has_flood_publisher
   USE MOD_Tracer_RiverLake, only: river_lake_tracer_init, tracer_init_from_water, &
      tracer_input_from_runoff, tracer_substep, tracer_limiter_stats, read_tracer_restart, &
      river_lake_tracer_final, acc_trc_inp, acc_rnof_ref, trc_mass, trc_inp_buf, trc_flux_out, &
      tracer_diag_accumulate_substep, trc_levsto, trc_solid, trc_levsto_solid, trc_dry_drain, &
      trc_reactive_source, levee_tracer_repartition, equilibrate_river_tracer_cell
   IMPLICIT NONE

   real(r8), parameter :: RIVERMIN  = 1.e-5_r8
   real(r8), parameter :: RIVERLAKE_FLOOD_MISSING_VALUE = -1.e30_r8

   real(r8), save :: acctime_rnof_max
   integer, save :: routing_zero_dt_warn_count = 0

   logical,  allocatable :: filter_rnof (:)
   real(r8), allocatable :: flood_depth_patch(:), flood_fraction_patch(:)
   real(r8), allocatable :: flood_evap_patch(:), flood_infil_patch(:)
   real(r8), allocatable :: flood_visible_uc(:), flood_protected_uc(:)
   logical,  allocatable :: flood_reservoir_uc(:)
   real(r8), allocatable :: flood_credit_patch(:)
   real(r8), allocatable :: flood_grid_area(:)
   real(r8), allocatable :: flood_evap_acc(:), flood_infil_acc(:)
   real(r8), save :: flood_evap_period = 0._r8, flood_infil_period = 0._r8
   real(r8), allocatable :: flood_tracer_credit_patch(:,:)
   real(r8), allocatable :: flood_tracer_evap_patch(:,:)
   real(r8), allocatable :: flood_tracer_land_patch(:,:)
   real(r8), allocatable :: flood_visible_tracer_uc(:,:), flood_protected_tracer_uc(:,:)
   integer, save :: flood_tracer_ledger_reports = 0

CONTAINS

   ! ---------
   SUBROUTINE grid_riverlake_flow_init (start_year, spinup)

   USE MOD_LandPatch,           only: numpatch
   USE MOD_Forcing,             only: forcmask_pch
   USE MOD_Vars_TimeInvariants, only: patchtype, patchmask
   USE MOD_Vars_Global,         only: spval
   USE MOD_Tracer_Defs,         only: ntracers, tracer_uses_land_water_transport
   IMPLICIT NONE

      integer, intent(in) :: start_year
      logical, intent(in) :: spinup

      integer :: i, irsv
      logical :: bif_restart_loaded
      real(r8), allocatable :: grid_area_local(:)
      logical :: trc_restart_found, has_flood_tracer
      logical,  allocatable :: trc_missing(:), is_built_resv_init(:)
      real(r8), allocatable :: wdsrf_safe(:), volresv_safe(:)
      integer,  allocatable :: ucat2resv_safe(:)

      acctime_rnof_max = DEF_GRIDBASED_ROUTING_MAX_DT

      ! excluding (patchtype >= 99), virtual patches and those forcing missed
      IF (p_is_worker) THEN
         allocate (filter_rnof (numpatch))
         IF (numpatch > 0) THEN
            filter_rnof = patchtype < 99
            filter_rnof = filter_rnof .and. patchmask
            IF (DEF_forcing%has_missing_value) THEN
               filter_rnof = filter_rnof .and. forcmask_pch
            ENDIF
         ENDIF
      ENDIF

      IF (DEF_USE_TRACER) THEN
      CALL tracer_lifecycle_route_init()
      IF (len_trim(gridriver_restart_file) > 0) THEN
         CALL tracer_lifecycle_route_read_restart(gridriver_restart_file)
      ENDIF
      ENDIF

      CALL levee_init()
      IF (len_trim(gridriver_restart_file) > 0) THEN
         CALL read_levee_restart(gridriver_restart_file, &
            restart_transaction_validated, restart_feature_manifest_present, &
            restart_levee_enabled, fold_protected_to_visible = .not. DEF_USE_LEVEE, &
            volwater_ucat_io = volwater_ucat, volwater_ucat_valid_io = volwater_ucat_valid, &
            wdsrf_ucat_in = wdsrf_ucat)
      ENDIF

      IF (DEF_USE_BIFURCATION) THEN
         CALL bifurcation_init()
         IF (len_trim(gridriver_restart_file) > 0) THEN
            CALL read_bifurcation_restart(gridriver_restart_file, &
               wdsrf_ucat_prev_restart_found, bif_restart_loaded, &
               restart_transaction_validated, restart_feature_manifest_present, &
               restart_bifurcation_enabled, restart_levee_enabled)
            IF (.not. bif_restart_loaded) THEN
               wdsrf_ucat_prev = wdsrf_ucat
               wdsrf_ucat_prev_valid = .true.
            ENDIF
         ENDIF
      ENDIF

      IF (.not. (DEF_USE_BIFURCATION .or. DEF_USE_LEVEE) .and. restart_levee_enabled &
          .and. volwater_ucat_valid .and. p_is_worker) THEN
         DO i = 1, numucat
            wdsrf_ucat(i) = floodplain_curve(i)%depth(volwater_ucat(i))
         ENDDO
      ENDIF

      CALL rebuild_volwater_ucat ()

      IF (DEF_USE_TRACER) THEN
      trc_restart_found = .false.
      CALL river_lake_tracer_init()
      IF (ntracers > 0) THEN
         allocate(trc_missing(ntracers))
         trc_missing = .true.
         IF (len_trim(gridriver_restart_file) > 0) THEN
            CALL read_tracer_restart(gridriver_restart_file, trc_restart_found, trc_missing)
         ENDIF
         IF (any(trc_missing)) THEN
            IF (allocated(wdsrf_ucat)) THEN
               allocate(wdsrf_safe(size(wdsrf_ucat)));   wdsrf_safe = wdsrf_ucat
            ELSE
               allocate(wdsrf_safe(0))
            ENDIF
            IF (allocated(volresv)) THEN
               allocate(volresv_safe(size(volresv)));    volresv_safe = volresv
            ELSE
               allocate(volresv_safe(0))
            ENDIF
            IF (allocated(ucat2resv)) THEN
               allocate(ucat2resv_safe(size(ucat2resv))); ucat2resv_safe = ucat2resv
            ELSE
               allocate(ucat2resv_safe(0))
            ENDIF
            allocate(is_built_resv_init(size(wdsrf_safe)))
            is_built_resv_init = .false.
            IF (allocated(lake_type) .and. allocated(dam_build_year)) THEN
               DO i = 1, min(size(is_built_resv_init), size(lake_type), size(ucat2resv_safe))
                  IF (lake_type(i) /= 2) CYCLE
                  irsv = ucat2resv_safe(i)
                  IF (irsv < 1 .or. irsv > size(dam_build_year)) CYCLE
                  is_built_resv_init(i) = start_year >= dam_build_year(irsv)
               ENDDO
            ENDIF
            CALL tracer_init_from_water(wdsrf_safe, volresv_safe, ucat2resv_safe, trc_missing, is_built_resv_init)
            deallocate(wdsrf_safe, volresv_safe, ucat2resv_safe, is_built_resv_init)
         ENDIF
         deallocate(trc_missing)
      ENDIF
      ENDIF

      gridriver_restart_file = ''

      CALL allocate_flood_patch ()
      IF (p_is_worker) THEN
         IF (DEF_GridRiverLake_FloodFeedback) THEN
      IF (DEF_USE_TRACER) THEN
            has_flood_tracer = .false.
            DO i = 1, ntracers
               IF (tracer_uses_land_water_transport(i)) has_flood_tracer = .true.
            ENDDO
            IF (has_flood_tracer) THEN
               allocate(flood_tracer_credit_patch(ntracers,numpatch), flood_tracer_evap_patch(ntracers,numpatch), &
                  flood_tracer_land_patch(ntracers,numpatch), &
                  flood_visible_tracer_uc(ntracers,numucat), flood_protected_tracer_uc(ntracers,numucat))
               flood_tracer_credit_patch = 0._r8
               flood_tracer_evap_patch = 0._r8
               flood_tracer_land_patch = 0._r8
            ENDIF
      ENDIF
            allocate(flood_visible_uc(numucat), flood_protected_uc(numucat), &
               flood_credit_patch(numpatch), flood_evap_acc(numpatch), flood_infil_acc(numpatch), &
               flood_reservoir_uc(numucat), flood_grid_area(numinpm))
            flood_evap_acc = 0._r8
            flood_infil_acc = 0._r8
            flood_evap_period = 0._r8
            flood_infil_period = 0._r8

            allocate(grid_area_local(numinpm))
            grid_area_local = 0._r8
            flood_credit_patch = 1._r8
            IF (numpatch > 0) CALL worker_remap_data_pset2grid(remap_patch2inpm, &
               flood_credit_patch, grid_area_local, fillvalue=0._r8, filter=flood_credit_patch>0._r8)
            CALL worker_push_data(allreduce_inpm, grid_area_local, flood_grid_area, fillvalue=0._r8)
            flood_grid_area = max(flood_grid_area, push_ucat2inpm%sum_area)
            deallocate(grid_area_local)
         ENDIF
      ENDIF
      IF (DEF_GridRiverLake_FloodFeedback) THEN
         CALL publish_flood_feedback(start_year)
         IF (spinup .and. p_is_worker) THEN
            flood_credit_patch = 0._r8
            flood_depth_patch = 0._r8
            flood_fraction_patch = 0._r8
      IF (DEF_USE_TRACER) THEN
            IF (allocated(flood_tracer_credit_patch)) flood_tracer_credit_patch = 0._r8
      ENDIF
         ENDIF
      ENDIF

   END SUBROUTINE grid_riverlake_flow_init

   ! ---------
   SUBROUTINE allocate_flood_patch ()

   USE MOD_LandPatch, only: numpatch
   IMPLICIT NONE

      IF (p_is_worker) THEN
         IF (allocated(flood_depth_patch)) deallocate(flood_depth_patch, flood_fraction_patch, &
            flood_evap_patch, flood_infil_patch)
         allocate(flood_depth_patch(numpatch), flood_fraction_patch(numpatch), &
            flood_evap_patch(numpatch), flood_infil_patch(numpatch))
         flood_depth_patch = 0._r8
         flood_fraction_patch = 0._r8
         flood_evap_patch = 0._r8
         flood_infil_patch = 0._r8
      ENDIF

   END SUBROUTINE allocate_flood_patch

   ! ---------
   SUBROUTINE rebuild_volwater_ucat ()

   IMPLICIT NONE
   integer :: i

      IF (p_is_worker) THEN
         DO i = 1, numucat
            IF (.not. volwater_ucat_valid .or. &
                (volwater_ucat(i) <= 0._r8 .and. wdsrf_ucat(i) > RIVERMIN)) THEN
               IF (DEF_USE_LEVEE .and. has_levee(i)) THEN
                  volwater_ucat(i) = levee_visible_volume_from_stage(i, wdsrf_ucat(i), levsto(i))
               ELSE
                  volwater_ucat(i) = floodplain_curve(i)%volume(wdsrf_ucat(i))
               ENDIF
            ENDIF
         ENDDO
      ENDIF
      volwater_ucat_valid = .true.

   END SUBROUTINE rebuild_volwater_ucat

   ! ---------
   SUBROUTINE grid_riverlake_flow_lulcc ()

   IMPLICIT NONE

      CALL allocate_flood_patch ()
      CALL rebuild_volwater_ucat ()
      IF (DEF_USE_BIFURCATION) THEN
         wdsrf_ucat_prev = wdsrf_ucat
         wdsrf_ucat_prev_valid = .true.
      ENDIF

   END SUBROUTINE grid_riverlake_flow_lulcc

   ! ---------
   SUBROUTINE grid_riverlake_flow (year, deltime)

   USE MOD_Utils
   USE MOD_Namelist,       only: DEF_Reservoir_Method, &
      DEF_GRIDBASED_ROUTING_MOMENTUM_DT_LIMIT
   USE MOD_Vars_1DFluxes,  only: rnof
   USE MOD_Forcing,        only: forcmask_pch
   USE MOD_Vars_TimeInvariants, only: patchtype, patchmask
   USE MOD_Mesh,           only: numelm
   USE MOD_LandPatch,      only: elm_patch, numpatch
   USE MOD_Const_Physical, only: grav
   USE MOD_Vars_Global,    only: spval
   USE, INTRINSIC :: ieee_arithmetic, only: ieee_is_finite
   USE MOD_Vars_1DForcing, only: forc_prc, forc_prl
   USE MOD_Tracer_Defs,    only: ntracers, tracer_uses_land_water_transport, &
      tracer_has_dissolved_limit, tracer_equilibrate_dissolved
   USE MOD_Tracer_Vars,    only: trc_rnof_step
   IMPLICIT NONE

   integer,  intent(in) :: year
   real(r8), intent(in) :: deltime

   ! Local Variables
   integer  :: i, j, irsv, ntimestep, ipth, i_up
   real(r8) :: dt_this
   integer  :: sed_clock_start, sed_clock_end, sed_clock_rate
   real(r8) :: sed_elapsed

   real(r8), allocatable :: rnof_gd(:)
   real(r8), allocatable :: rnof_uc(:)

   real(r8), allocatable :: prcp_gd(:)
   real(r8), allocatable :: prcp_uc(:)
   real(r8), allocatable :: prcp_pch(:)
   integer  :: itrc, itrc_dep
   integer  :: lim_calls, lim_iter_sum, lim_iter_peak, lim_over_soft
   integer, save :: lim_diag_printed = 0
   real(r8) :: frac_remove, trc_removed
   real(r8), allocatable :: trc_rnof_gd(:,:), trc_rnof_uc(:,:)
   real(r8), allocatable :: prcp_area_gd(:), prcp_area_uc(:)
   logical,  allocatable :: filter_prcp(:)
   real(r8), allocatable :: particle_floodarea(:), particle_protected_area(:)
   real(r8), allocatable :: particle_water_storage_start(:), particle_water_storage(:)
   real(r8), allocatable :: particle_protected_start(:), particle_protected_end(:)

   logical,  allocatable :: is_built_resv(:)

   real(r8), allocatable :: wdsrf_next(:)
   real(r8), allocatable :: veloc_next(:)

   real(r8), allocatable :: hflux_fc(:)
   real(r8), allocatable :: mflux_fc(:)
   real(r8), allocatable :: zgrad_dn(:)

   real(r8), allocatable :: hflux_resv(:)
   real(r8), allocatable :: mflux_resv(:)

   real(r8), allocatable :: hflux_sumups(:)
   real(r8), allocatable :: mflux_sumups(:)
   real(r8), allocatable :: zgrad_sumups(:)

   real(r8), allocatable :: sum_hflux_riv(:)
   real(r8), allocatable :: sum_hflux_base(:)
   real(r8), allocatable :: sum_mflux_riv(:)
   real(r8), allocatable :: sum_zgrad_riv(:)
   real(r8), allocatable :: normal_outgoing_rate(:), ordinary_scale(:), ordinary_scale_next(:)
   real(r8), allocatable :: volresv_safe(:)
   real(r8), allocatable :: levee_floodarea(:)
   integer,  allocatable :: ucat2resv_safe(:)
   real(r8), allocatable :: total_floodarea(:), total_flooddepth(:)

   real(r8) :: veloct_fc, height_fc, momen_fc, zsurf_fc
   real(r8) :: bedelv_fc, height_up, height_dn
   real(r8) :: vwave_up, vwave_dn, hflux_up, hflux_dn, mflux_up, mflux_dn
   real(r8) :: volwater, friction, floodarea, rivsto_hist
   real(r8) :: fldfrc_levee, visible_hflux, protected_hflux, protected_clip
   real(r8) :: vis_vol_bef_lv, levsto_bef_lv, vis_vol_bef_lv2, levsto_bef_lv2
   real(r8) :: totalflood_evap, totalflood_infil, flood_vec(2)
   real(r8),  allocatable :: dt_res(:), dt_all(:)
   logical,   allocatable :: ucatfilter(:)
   logical :: loop_active, next_loop_active
   real(r8) :: totalvol_bef, totalvol_aft, totalrnof, totaldis
   real(r8), allocatable :: trc_mass_bef(:), trc_mass_aft(:)
   real(r8), allocatable :: trc_mass_inp(:), trc_mass_dis(:), trc_mass_reactive(:)


      IF (p_is_worker) THEN

         allocate (rnof_gd (numinpm))
         allocate (rnof_uc (numucat))
         rnof_gd = 0.

         IF (numpatch > 0) THEN
            filter_rnof = patchtype < 99 .and. patchmask
            IF (DEF_forcing%has_missing_value) filter_rnof = filter_rnof .and. forcmask_pch
            CALL worker_remap_data_pset2grid (remap_patch2inpm, rnof, rnof_gd, &
               fillvalue = 0., filter = filter_rnof)
         ENDIF

         IF (numinpm > 0) THEN
            WHERE (push_ucat2inpm%sum_area > 0)
               rnof_gd = rnof_gd / push_ucat2inpm%sum_area
            END WHERE
         ENDIF

         CALL worker_push_data (push_inpm2ucat, rnof_gd, rnof_uc, &
            fillvalue = 0., mode = 'sum')

      IF (DEF_USE_TRACER) THEN
         IF (ntracers > 0) THEN
            allocate (trc_rnof_gd (ntracers, numinpm))
            allocate (trc_rnof_uc (ntracers, numucat))
            trc_rnof_gd = 0._r8
            trc_rnof_uc = 0._r8
            DO itrc = 1, ntracers
               IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
               IF (numpatch > 0) &
                  CALL worker_remap_data_pset2grid(remap_patch2inpm, trc_rnof_step(itrc, :), trc_rnof_gd(itrc, :), &
                     fillvalue = 0._r8, filter = filter_rnof)
               IF (numinpm > 0) THEN
                  WHERE (push_ucat2inpm%sum_area > 0._r8)
                     trc_rnof_gd(itrc, :) = trc_rnof_gd(itrc, :) / push_ucat2inpm%sum_area
                  END WHERE
               ENDIF
               CALL worker_push_data(push_inpm2ucat, trc_rnof_gd(itrc, :), trc_rnof_uc(itrc, :), &
                  fillvalue = 0._r8, mode = 'sum')
            ENDDO
         ENDIF
      ENDIF

         IF (numucat > 0) THEN
            acc_rnof_uc = acc_rnof_uc + rnof_uc*1.e-3*deltime
      IF (DEF_USE_TRACER) THEN
            IF (ntracers > 0) CALL tracer_input_from_runoff(rnof_uc*1.e-3*deltime, numucat, trc_rnof_uc*1.e-3)
      ENDIF
         ENDIF

         deallocate(rnof_gd)
         deallocate(rnof_uc)
      IF (DEF_USE_TRACER) THEN
         IF (allocated(trc_rnof_gd)) deallocate(trc_rnof_gd)
         IF (allocated(trc_rnof_uc)) deallocate(trc_rnof_uc)
      ENDIF

      IF (DEF_USE_TRACER) THEN
         IF (tracer_lifecycle_route_has_active()) THEN
            allocate(prcp_pch(numpatch), filter_prcp(numpatch))
            allocate(prcp_gd(numinpm), prcp_area_gd(numinpm))
            allocate(prcp_uc(numucat), prcp_area_uc(numucat))
            prcp_pch = 0._r8
            prcp_gd = 0._r8
            prcp_area_gd = 0._r8
            IF (numpatch > 0) THEN
               filter_prcp = patchtype < 99 .and. patchmask
               IF (DEF_forcing%has_missing_value) filter_prcp = filter_prcp .and. forcmask_pch
               filter_prcp = filter_prcp .and. ieee_is_finite(forc_prc) .and. ieee_is_finite(forc_prl)
               DO i = 1, numpatch
                  IF (.not. filter_prcp(i)) CYCLE
                  filter_prcp(i) = forc_prc(i) /= spval .and. forc_prl(i) /= spval
                  IF (filter_prcp(i)) filter_prcp(i) = forc_prc(i) >= 0._r8 .and. forc_prl(i) >= 0._r8
               ENDDO
               WHERE (filter_prcp) prcp_pch = forc_prc + forc_prl
               CALL worker_remap_data_pset2grid(remap_patch2inpm, prcp_pch, prcp_gd, &
                  fillvalue = 0._r8, filter = filter_prcp)
            ENDIF
            prcp_pch = 1._r8
            IF (numpatch > 0) &
               CALL worker_remap_data_pset2grid(remap_patch2inpm, prcp_pch, prcp_area_gd, &
                  fillvalue = 0._r8, filter = filter_prcp)
            WHERE (push_ucat2inpm%sum_area > 0._r8)
               prcp_gd = prcp_gd / push_ucat2inpm%sum_area
               prcp_area_gd = prcp_area_gd / push_ucat2inpm%sum_area
            ELSEWHERE
               prcp_gd = 0._r8
               prcp_area_gd = 0._r8
            END WHERE
            CALL worker_push_data(push_inpm2ucat, prcp_gd, prcp_uc, fillvalue = 0._r8, mode = 'sum')
            CALL worker_push_data(push_inpm2ucat, prcp_area_gd, prcp_area_uc, fillvalue = 0._r8, mode = 'sum')
            WHERE (prcp_area_uc > 0._r8)
               prcp_uc = prcp_uc / prcp_area_uc
            ELSEWHERE
               prcp_uc = 0._r8
            END WHERE
            WHERE (topo_area > 0._r8)
               prcp_area_uc = prcp_area_uc / topo_area
            ELSEWHERE
               prcp_area_uc = 0._r8
            END WHERE
            CALL tracer_lifecycle_route_forcing_put(prcp_uc, deltime, prcp_area_uc)
            deallocate(prcp_pch, filter_prcp, prcp_gd, prcp_area_gd, prcp_uc, prcp_area_uc)
         ENDIF
      ENDIF

      ENDIF

      IF (DEF_GridRiverLake_FloodFeedback) THEN
         IF (p_is_worker) THEN
            flood_evap_acc = flood_evap_acc + flood_evap_patch*deltime
            flood_infil_acc = flood_infil_acc + flood_infil_patch*deltime
            flood_evap_patch = 0._r8
            flood_infil_patch = 0._r8
         ENDIF
         CALL debit_flood_feedback(totalflood_evap, totalflood_infil)
         flood_evap_period = flood_evap_period + totalflood_evap
         flood_infil_period = flood_infil_period + totalflood_infil
         CALL publish_flood_feedback(year)
      ENDIF

      acctime_rnof = acctime_rnof + deltime

      IF (acctime_rnof+0.01 < acctime_rnof_max) THEN
         RETURN
      ENDIF

      IF ((DEF_USE_CoLMDEBUG) .and. (DEF_USE_TRACER)) THEN
      allocate (trc_mass_bef(ntracers), trc_mass_aft(ntracers), trc_mass_inp(ntracers), &
         trc_mass_dis(ntracers), trc_mass_reactive(ntracers))
      trc_mass_bef = 0._r8
      trc_mass_aft = 0._r8
      trc_mass_inp = 0._r8
      trc_mass_dis = 0._r8
      trc_mass_reactive = 0._r8
      ENDIF

      IF (p_is_worker) THEN

         allocate (is_built_resv (numucat))
         allocate (wdsrf_next    (numucat))
         allocate (veloc_next    (numucat))
         allocate (hflux_fc      (numucat))
         allocate (mflux_fc      (numucat))
         allocate (zgrad_dn      (numucat))
         allocate (sum_hflux_riv (numucat))
         IF (DEF_USE_BIFURCATION) THEN
            allocate (sum_hflux_base(numucat), normal_outgoing_rate(numucat))
            allocate (ordinary_scale(numucat), ordinary_scale_next(numucat))
         ENDIF
         allocate (sum_mflux_riv (numucat))
         allocate (sum_zgrad_riv (numucat))
         allocate (ucatfilter    (numucat))
         allocate (levee_floodarea(numucat))
         levee_floodarea = 0._r8
      IF (DEF_USE_TRACER) THEN
         allocate (total_floodarea(numucat), total_flooddepth(numucat))
         total_floodarea = 0._r8
         total_flooddepth = 0._r8
         allocate (particle_floodarea(numucat), particle_protected_area(numucat))
         allocate (particle_water_storage_start(numucat), particle_water_storage(numucat))
         allocate (particle_protected_start(numucat), particle_protected_end(numucat))
         particle_floodarea = 0._r8
         particle_protected_area = 0._r8
         particle_water_storage_start = 0._r8
         particle_water_storage = 0._r8
         particle_protected_start = 0._r8
         particle_protected_end = 0._r8
      ENDIF

         allocate (hflux_sumups  (numucat))
         allocate (mflux_sumups  (numucat))
         allocate (zgrad_sumups  (numucat))

         IF (DEF_Reservoir_Method > 0) THEN
            allocate (hflux_resv (numucat))
            allocate (mflux_resv (numucat))
         ENDIF

         allocate (dt_res (numrivsys))
         allocate (dt_all (numrivsys))

         IF (allocated(volresv)) THEN
            allocate (volresv_safe(size(volresv)))
            volresv_safe = volresv
         ELSE
            allocate (volresv_safe(0))
         ENDIF

      IF (DEF_USE_TRACER) THEN
         IF (allocated(ucat2resv)) THEN
            allocate (ucat2resv_safe(size(ucat2resv)))
            ucat2resv_safe = ucat2resv
         ELSE
            allocate (ucat2resv_safe(0))
         ENDIF
         IF (allocated(trc_reactive_source)) trc_reactive_source = 0._r8
      IF (DEF_USE_CoLMDEBUG) THEN
         IF (numucat > 0) THEN
            DO itrc = 1, ntracers
               IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
               trc_mass_bef(itrc) = sum(trc_mass(itrc,:)) + sum(trc_inp_buf(itrc,:))
               IF (allocated(trc_levsto)) trc_mass_bef(itrc) = trc_mass_bef(itrc) + sum(trc_levsto(itrc,:))
               IF (allocated(trc_solid)) trc_mass_bef(itrc) = trc_mass_bef(itrc) &
                  + sum(trc_solid(itrc,:)) + sum(trc_levsto_solid(itrc,:))
               trc_mass_inp(itrc) = sum(acc_trc_inp(itrc,:))
            ENDDO
         ENDIF
      ENDIF
         IF (numucat > 0) THEN
            DO itrc = 1, ntracers
               IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
               trc_inp_buf(itrc, :) = trc_inp_buf(itrc, :) + acc_trc_inp(itrc, :)
               acc_trc_inp(itrc, :) = 0._r8
            ENDDO
         ENDIF
      ENDIF

      IF (DEF_USE_CoLMDEBUG) THEN
         totalrnof = sum(acc_rnof_uc)
         totalvol_bef = 0.
      ENDIF

         DO i = 1, numucat

            is_built_resv(i) = .false.
            IF (lake_type(i) == 2) THEN
               irsv = ucat2resv(i)
               IF (year >= dam_build_year(irsv)) THEN
                  is_built_resv(i) = .true.
                  IF (volresv(irsv) == spval) THEN
                     volresv(irsv) = floodplain_curve(i)%volume (wdsrf_ucat(i))
                  ELSE
                     wdsrf_ucat(i) = floodplain_curve(i)%depth (volresv(irsv))
                  ENDIF
               ENDIF
            ENDIF

            IF (.not. is_built_resv(i)) THEN
               momen_riv(i) = wdsrf_ucat(i) * veloc_riv(i)
               IF (DEF_USE_BIFURCATION .or. DEF_USE_LEVEE) THEN
                  volwater = volwater_ucat(i)
               ELSE
                  volwater = floodplain_curve(i)%volume (wdsrf_ucat(i))
               ENDIF
            ELSE
               ! water in reservoirs is assumued to be stationary.
               momen_riv(i) = 0
               veloc_riv(i) = 0
               volwater = volresv(ucat2resv(i))
            ENDIF

      IF (DEF_USE_CoLMDEBUG) THEN
            totalvol_bef = totalvol_bef + volwater
            IF (DEF_USE_LEVEE .and. has_levee(i) .and. (.not. is_built_resv(i))) &
               totalvol_bef = totalvol_bef + levsto(i)
      ENDIF

            volwater = volwater + acc_rnof_uc(i)

            IF (.not. is_built_resv(i)) THEN
               IF (DEF_USE_LEVEE .and. has_levee(i)) THEN
                  CALL levee_repartition_storage(i, volwater, wdsrf_ucat(i), fldfrc_levee, &
                     vis_vol_bef_lv, levsto_bef_lv)
                  levee_floodarea(i) = fldfrc_levee * topo_area(i)
      IF (DEF_USE_TRACER) THEN
                  IF (ntracers > 0) CALL tracer_lifecycle_route_sediment_levee_repartition(i, &
                     vis_vol_bef_lv, levsto_bef_lv, volwater, levsto(i))
                  IF (ntracers > 0) CALL levee_tracer_repartition(i, vis_vol_bef_lv, levsto_bef_lv, &
                     volwater, levsto(i), pending_trc_pool = trc_inp_buf(:, i))
      ENDIF
               ELSE
                  wdsrf_ucat(i) = floodplain_curve(i)%depth (volwater)
               ENDIF
               volwater_ucat(i) = volwater
               IF (wdsrf_ucat(i) > RIVERMIN) THEN
                   veloc_riv(i) = momen_riv(i) / wdsrf_ucat(i)
                ELSE
                   veloc_riv(i) = 0.
               ENDIF
            ELSE
               volresv(ucat2resv(i)) = volwater
            ENDIF

         ENDDO


         ntimestep = 0
      IF (DEF_USE_CoLMDEBUG) THEN
         totaldis  = 0.
      ENDIF

         dt_res(:) = acctime_rnof

         IF (DEF_USE_BIFURCATION) THEN
            CALL bifurcation_invalidate_static_dn()
            IF (.not. wdsrf_ucat_prev_valid) THEN
               wdsrf_ucat_prev = wdsrf_ucat
               wdsrf_ucat_prev_valid = .true.
            ENDIF
            IF (ieee_is_finite(acctime_rnof)) THEN
               loop_active = acctime_rnof > 0._r8
            ELSE
               loop_active = .true.
            ENDIF
         ELSE
            loop_active = any(dt_res > 0._r8)
         ENDIF

         DO WHILE (loop_active)

            ntimestep = ntimestep + 1

            CALL worker_push_data (push_next2ucat, wdsrf_ucat, wdsrf_next, fillvalue = spval)
            ! velocity in ocean or inland depression is assumed to be 0.
            CALL worker_push_data (push_next2ucat, veloc_riv,  veloc_next, fillvalue = 0.)

            dt_all(:) = min(dt_res(:), 60.)

            DO i = 1, numucat

               ucatfilter(i) = dt_all(irivsys(i)) > 0

               IF (.not. ucatfilter(i)) CYCLE

               sum_hflux_riv(i) = 0.
               sum_mflux_riv(i) = 0.
               sum_zgrad_riv(i) = 0.

               ! reservoir
               IF (is_built_resv(i)) THEN
                  hflux_fc(i) = 0.
                  mflux_fc(i) = 0.
                  zgrad_dn(i) = 0.
                  CYCLE
               ENDIF

               IF ((ucat_next(i) > 0) .or. (ucat_next(i) == -9)) THEN

                  IF (ucat_next(i) > 0) THEN
                     ! both rivers are dry.
                     IF ((wdsrf_ucat(i) < RIVERMIN) .and. (wdsrf_next(i) < RIVERMIN)) THEN
                        hflux_fc(i) = 0
                        mflux_fc(i) = 0
                        zgrad_dn(i) = 0
                        CYCLE
                     ENDIF
                  ENDIF

                  ! reconstruction of height of water near interface
                  IF (ucat_next(i) > 0) THEN
                     bedelv_fc = max(topo_rivelv(i), bedelv_next(i))
                     height_up = max(0., wdsrf_ucat(i)+topo_rivelv(i)-bedelv_fc)
                     height_dn = max(0., wdsrf_next(i)+bedelv_next(i)-bedelv_fc)
                  ELSEIF (ucat_next(i) == -9) THEN ! for river mouth
                     bedelv_fc = topo_rivelv(i)
                     height_up = wdsrf_ucat (i)
                     ! sea level is assumed to be 0. and sea bed is assumed to be negative infinity.
                     height_dn = max(0., - bedelv_fc)
                  ENDIF

                  ! velocity at river downstream face (middle region in Riemann problem)
                  veloct_fc = 0.5 * (veloc_riv(i) + veloc_next(i)) &
                     + sqrt(grav * height_up) - sqrt(grav * height_dn)

                  ! height of water at downstream face (middle region in Riemann problem)
                  height_fc = 1/grav * (0.5*(sqrt(grav*height_up) + sqrt(grav*height_dn)) &
                     + 0.25 * (veloc_riv(i) - veloc_next(i))) ** 2

                  IF (height_up > 0) THEN
                     vwave_up = min(veloc_riv(i)-sqrt(grav*height_up), veloct_fc-sqrt(grav*height_fc))
                  ELSE
                     vwave_up = veloc_next(i) - 2.0 * sqrt(grav*height_dn)
                  ENDIF

                  IF (height_dn > 0) THEN
                     vwave_dn = max(veloc_next(i)+sqrt(grav*height_dn), veloct_fc+sqrt(grav*height_fc))
                  ELSE
                     vwave_dn = veloc_riv(i) + 2.0 * sqrt(grav*height_up)
                  ENDIF

                  hflux_up = veloc_riv(i)  * height_up
                  hflux_dn = veloc_next(i) * height_dn
                  mflux_up = veloc_riv(i)**2  * height_up + 0.5*grav * height_up**2
                  mflux_dn = veloc_next(i)**2 * height_dn + 0.5*grav * height_dn**2

                  IF (vwave_up >= 0.) THEN
                     hflux_fc(i) = outletwth(i) * hflux_up
                     mflux_fc(i) = outletwth(i) * mflux_up
                  ELSEIF (vwave_dn <= 0.) THEN
                     hflux_fc(i) = outletwth(i) * hflux_dn
                     mflux_fc(i) = outletwth(i) * mflux_dn
                  ELSE
                     hflux_fc(i) = outletwth(i) * (vwave_dn*hflux_up - vwave_up*hflux_dn &
                        + vwave_up*vwave_dn*(height_dn-height_up)) / (vwave_dn-vwave_up)
                     mflux_fc(i) = outletwth(i) * (vwave_dn*mflux_up - vwave_up*mflux_dn &
                        + vwave_up*vwave_dn*(hflux_dn-hflux_up)) / (vwave_dn-vwave_up)
                  ENDIF

                  sum_zgrad_riv(i) = sum_zgrad_riv(i) + outletwth(i) * 0.5*grav * height_up**2

                  zgrad_dn(i) = outletwth(i) * 0.5*grav * height_dn**2

               ELSEIF (ucat_next(i) == -99) THEN
                  ! downstream is not in model region.
                  ! assume: 1. downstream river bed is equal to this river bed.
                  !         2. downstream water surface is equal to this river depth.
                  !         3. downstream water velocity is equal to this velocity.

                  veloc_riv(i) = max(veloc_riv(i), 0.)

                  IF (wdsrf_ucat(i) > topo_rivhgt(i)) THEN

                     ! reconstruction of height of water near interface
                     height_up = wdsrf_ucat (i)
                     height_dn = topo_rivhgt(i)

                     veloct_fc = veloc_riv(i) + sqrt(grav * height_up) - sqrt(grav * height_dn)
                     height_fc = 1/grav * (0.5*(sqrt(grav*height_up) + sqrt(grav*height_dn))) ** 2

                     vwave_up = min(veloc_riv(i)-sqrt(grav*height_up), veloct_fc-sqrt(grav*height_fc))
                     vwave_dn = max(veloc_riv(i)+sqrt(grav*height_dn), veloct_fc+sqrt(grav*height_fc))

                     hflux_up = veloc_riv(i) * height_up
                     hflux_dn = veloc_riv(i) * height_dn
                     mflux_up = veloc_riv(i)**2 * height_up + 0.5*grav * height_up**2
                     mflux_dn = veloc_riv(i)**2 * height_dn + 0.5*grav * height_dn**2

                     IF (vwave_up >= 0.) THEN
                        hflux_fc(i) = outletwth(i) * hflux_up
                        mflux_fc(i) = outletwth(i) * mflux_up
                     ELSEIF (vwave_dn <= 0.) THEN
                        hflux_fc(i) = outletwth(i) * hflux_dn
                        mflux_fc(i) = outletwth(i) * mflux_dn
                     ELSE
                        hflux_fc(i) = outletwth(i) * (vwave_dn*hflux_up - vwave_up*hflux_dn &
                           + vwave_up*vwave_dn*(height_dn-height_up)) / (vwave_dn-vwave_up)
                        mflux_fc(i) = outletwth(i) * (vwave_dn*mflux_up - vwave_up*mflux_dn &
                           + vwave_up*vwave_dn*(hflux_dn-hflux_up)) / (vwave_dn-vwave_up)
                     ENDIF

                     sum_zgrad_riv(i) = sum_zgrad_riv(i) + outletwth(i) * 0.5*grav * height_up**2

                  ELSE
                     hflux_fc(i) = 0
                     mflux_fc(i) = 0
                  ENDIF

               ELSEIF (ucat_next(i) == -10) THEN ! inland depression
                  hflux_fc(i) = 0
                  mflux_fc(i) = 0
               ENDIF

               sum_hflux_riv(i) = sum_hflux_riv(i) + hflux_fc(i)
               sum_mflux_riv(i) = sum_mflux_riv(i) + mflux_fc(i)

            ENDDO

            CALL worker_push_data (push_ups2ucat, hflux_fc, hflux_sumups, fillvalue = 0., mode = 'sum')
            CALL worker_push_data (push_ups2ucat, mflux_fc, mflux_sumups, fillvalue = 0., mode = 'sum')
            CALL worker_push_data (push_ups2ucat, zgrad_dn, zgrad_sumups, fillvalue = 0., mode = 'sum')

            IF (numucat > 0) THEN
               WHERE (ucatfilter)
                  sum_hflux_riv = sum_hflux_riv - hflux_sumups
                  sum_mflux_riv = sum_mflux_riv - mflux_sumups
                  sum_zgrad_riv = sum_zgrad_riv - zgrad_sumups
               END WHERE
            ENDIF

            ! reservoir operation.
            IF (DEF_Reservoir_Method > 0) THEN

               hflux_resv = 0.
               mflux_resv = 0.

               DO i = 1, numucat

                  IF (.not. ucatfilter(i)) CYCLE

                  IF (ucat_next(i) == -10) THEN
                     IF (is_built_resv(i)) THEN
                        qresv_in (ucat2resv(i)) = - sum_hflux_riv(i)
                        qresv_out(ucat2resv(i)) = 0.
                     ENDIF
                     CYCLE
                  ENDIF

                  IF (is_built_resv(i)) THEN

                     irsv = ucat2resv(i)
                     qresv_in(irsv) = - sum_hflux_riv(i)

                     IF (volresv(irsv) > 1.e-4 * volresv_total(irsv)) THEN
                        CALL reservoir_operation (DEF_Reservoir_Method, &
                           irsv, qresv_in(irsv), volresv(irsv), qresv_out(irsv))
                     ELSE
                        qresv_out (irsv) = 0.
                     ENDIF

                     hflux_fc(i) = qresv_out(irsv)
                     mflux_fc(i) = qresv_out(irsv) * sqrt(2*grav*wdsrf_ucat(i))

                     sum_hflux_riv(i) = sum_hflux_riv(i) + hflux_fc(i)
                     sum_mflux_riv(i) = sum_mflux_riv(i) + mflux_fc(i)

                     hflux_resv(i) = hflux_fc(i)
                     mflux_resv(i) = mflux_fc(i)
                  ENDIF

               ENDDO

               CALL worker_push_data (push_ups2ucat, hflux_resv, hflux_sumups, fillvalue = 0., mode = 'sum')
               CALL worker_push_data (push_ups2ucat, mflux_resv, mflux_sumups, fillvalue = 0., mode = 'sum')

               IF (numucat > 0) THEN
                  WHERE (ucatfilter)
                     sum_hflux_riv = sum_hflux_riv - hflux_sumups
                     sum_mflux_riv = sum_mflux_riv - mflux_sumups
                  END WHERE
               ENDIF

            ENDIF

            DO i = 1, numucat

               IF (.not. ucatfilter(i)) CYCLE

               dt_this = dt_all(irivsys(i))

               ! constraint 1: CFL condition (only for rivers)
               IF (.not. is_built_resv(i)) THEN
                  IF ((veloc_riv(i) /= 0.) .or. (wdsrf_ucat(i) > 0.)) THEN
                     dt_this = min(dt_this, topo_rivlen(i)/(abs(veloc_riv(i))+sqrt(grav*wdsrf_ucat(i)))*0.8)
                  ENDIF
               ENDIF

               ! constraint 2: Avoid negative values of water
               IF (sum_hflux_riv(i) > 0) THEN
                  IF (.not. is_built_resv(i)) THEN
                     ! for river or lake catchment
                     IF (DEF_USE_BIFURCATION .or. DEF_USE_LEVEE) THEN
                        volwater = volwater_ucat(i)
                     ELSE
                        volwater = floodplain_curve(i)%volume (wdsrf_ucat(i))
                     ENDIF
                  ELSE
                     ! for reservoir
                     volwater = volresv(ucat2resv(i))
                  ENDIF

                  dt_this = min(dt_this, volwater / sum_hflux_riv(i))

               ENDIF

               ! constraint 3: Avoid change of flow direction (only for rivers)
               IF (DEF_GRIDBASED_ROUTING_MOMENTUM_DT_LIMIT .and. (.not. is_built_resv(i))) THEN
                  IF (abs(veloc_riv(i)) > 0.1_r8 .and. &
                      veloc_riv(i) * (sum_mflux_riv(i)-sum_zgrad_riv(i)) > 0._r8) THEN
                     dt_this = min(dt_this, &
                        abs(momen_riv(i) * topo_rivare(i) / (sum_mflux_riv(i)-sum_zgrad_riv(i))))
                  ENDIF
               ENDIF

               dt_all(irivsys(i)) = min(dt_this, dt_all(irivsys(i)))

            ENDDO

            IF (DEF_USE_BIFURCATION) THEN
               normal_outgoing_rate = 0._r8
               hflux_sumups = 0._r8
               DO i = 1, numucat
                  IF (.not. ucatfilter(i)) CYCLE
                  IF (hflux_fc(i) >= 0._r8) THEN
                     normal_outgoing_rate(i) = hflux_fc(i)
                  ELSE
                     hflux_sumups(i) = -hflux_fc(i)
                  ENDIF
               ENDDO
               CALL worker_push_data(push_ups2ucat, hflux_sumups, mflux_sumups, &
                  fillvalue=0._r8, mode='sum')
               WHERE (ucatfilter)
                  normal_outgoing_rate = normal_outgoing_rate + mflux_sumups
               END WHERE
               ordinary_scale = 1._r8
               DO i = 1, numucat
                  IF (.not. ucatfilter(i)) CYCLE
                  IF (normal_outgoing_rate(i) <= 0._r8) CYCLE
                  IF (is_built_resv(i)) THEN
                     volwater = volresv(ucat2resv(i))
                  ELSE
                     volwater = volwater_ucat(i)
                  ENDIF
                  IF (.not. ieee_is_finite(dt_all(irivsys(i)))) THEN
                     ordinary_scale(i) = 0._r8
                  ELSEIF (dt_all(irivsys(i)) <= 0._r8) THEN
                     ordinary_scale(i) = 0._r8
                  ELSE
                     ordinary_scale(i) = min(1._r8, max(volwater, 0._r8) / &
                        (normal_outgoing_rate(i) * dt_all(irivsys(i))))
                  ENDIF
                  normal_outgoing_rate(i) = normal_outgoing_rate(i) * ordinary_scale(i)
               ENDDO
               CALL worker_push_data(push_next2ucat, ordinary_scale, ordinary_scale_next, fillvalue=1._r8)
               DO i = 1, numucat
                  IF (.not. ucatfilter(i)) CYCLE
                  IF (hflux_fc(i) >= 0._r8) THEN
                     dt_this = ordinary_scale(i)
                  ELSE
                     dt_this = ordinary_scale_next(i)
                  ENDIF
                  hflux_fc(i) = hflux_fc(i) * dt_this
                  mflux_fc(i) = mflux_fc(i) * dt_this
                  IF (is_built_resv(i)) qresv_out(ucat2resv(i)) = hflux_fc(i)
               ENDDO
               CALL worker_push_data(push_ups2ucat, hflux_fc, hflux_sumups, fillvalue=0._r8, mode='sum')
               CALL worker_push_data(push_ups2ucat, mflux_fc, mflux_sumups, fillvalue=0._r8, mode='sum')
               WHERE (ucatfilter)
                  sum_hflux_riv = hflux_fc - hflux_sumups
                  sum_mflux_riv = mflux_fc - mflux_sumups
               END WHERE
               DO i = 1, numucat
                  IF (.not. ucatfilter(i)) CYCLE
                  IF (is_built_resv(i)) qresv_in(ucat2resv(i)) = hflux_sumups(i)
               ENDDO
               IF (DEF_GRIDBASED_ROUTING_MOMENTUM_DT_LIMIT) THEN
                  DO i = 1, numucat
                     IF (.not. ucatfilter(i) .or. is_built_resv(i)) CYCLE
                     IF (abs(veloc_riv(i)) > 0.1_r8 .and. &
                         veloc_riv(i) * (sum_mflux_riv(i)-sum_zgrad_riv(i)) > 0._r8) THEN
                        dt_this = abs(momen_riv(i) * topo_rivare(i) / &
                           (sum_mflux_riv(i)-sum_zgrad_riv(i)))
                        IF (.not. ieee_is_finite(dt_this) .or. dt_this <= 0._r8) &
                           CALL CoLM_stop('grid_riverlake_flow: invalid final momentum dt')
                        dt_all(irivsys(i)) = min(dt_all(irivsys(i)), dt_this)
                     ENDIF
                  ENDDO
               ENDIF
               CALL sync_global_routing_dt(dt_res, dt_all, next_loop_active)
               sum_hflux_base = sum_hflux_riv
               IF (allocated(volresv)) volresv_safe = volresv
               CALL bifurcation_calc(wdsrf_ucat, wdsrf_ucat_prev, volwater_ucat, &
                  volwater_ucat_valid, volresv_safe, is_built_resv, dt_all, &
                  irivsys, ucatfilter, normal_outgoing_rate)
               wdsrf_ucat_prev = wdsrf_ucat
               wdsrf_ucat_prev_valid = .true.
               IF (allocated(a_bifflw_lev) .and. allocated(a_bifflw_acctime)) THEN
                  DO ipth = 1, npthout_local
                     i_up = pth_upst_local(ipth)
                     IF (i_up < 1 .or. i_up > numucat) CYCLE
                     IF (.not. ucatfilter(i_up)) CYCLE
                     IF (allocated(bif_path_active)) THEN
                        IF (.not. bif_path_active(ipth)) CYCLE
                     ENDIF
                     a_bifflw_lev(:, ipth) = a_bifflw_lev(:, ipth) &
                        + bif_hflux_lev(:, ipth) * dt_all(irivsys(i_up))
                     a_bifflw_acctime(ipth) = a_bifflw_acctime(ipth) + dt_all(irivsys(i_up))
                  ENDDO
               ENDIF
               WHERE (ucatfilter)
                  sum_hflux_riv = sum_hflux_base + bif_hflux_sum
               END WHERE
            ELSE
#ifdef USEMPI
               IF (rivsys_by_multiple_procs) THEN
                  CALL mpi_allreduce (MPI_IN_PLACE, dt_all, 1, MPI_REAL8, MPI_MIN, p_comm_rivsys, p_err)
               ENDIF
#endif
            ENDIF

      IF (DEF_USE_TRACER) THEN
            IF (ntracers > 0 .and. DEF_USE_BIFURCATION) &
               CALL tracer_lifecycle_route_sediment_bif_accumulate(dt_all, irivsys, ucatfilter, bif_hflux_lev)
            IF (allocated(volresv)) volresv_safe = volresv
            IF (DEF_USE_BIFURCATION) THEN
               CALL tracer_substep (acctime_rnof, dt_all, irivsys, hflux_fc, sum_hflux_riv, &
                  wdsrf_ucat, ucatfilter, volresv_safe, ucat2resv_safe, is_built_resv, &
                  do_bif = .true., bif_hflux_lev_in = bif_hflux_lev, npthout_local_in = npthout_local)
            ELSE
               CALL tracer_substep (acctime_rnof, dt_all, irivsys, hflux_fc, sum_hflux_riv, &
                  wdsrf_ucat, ucatfilter, volresv_safe, ucat2resv_safe, is_built_resv)
            ENDIF
      ENDIF

            DO i = 1, numucat

               IF (.not. ucatfilter(i)) CYCLE

               IF (.not. is_built_resv(i)) THEN
                  IF (DEF_USE_BIFURCATION .or. DEF_USE_LEVEE) THEN
                     volwater = volwater_ucat(i)
                  ELSE
                     volwater = floodplain_curve(i)%volume (wdsrf_ucat(i))
                  ENDIF
               ELSE
                  volwater = volresv(ucat2resv(i))
               ENDIF

      IF (DEF_USE_TRACER) THEN
               particle_water_storage_start(i) = max(volwater, 0._r8)
               particle_protected_start(i) = 0._r8
               IF (DEF_USE_LEVEE .and. has_levee(i) .and. (.not. is_built_resv(i))) THEN
                  particle_water_storage_start(i) = particle_water_storage_start(i) + max(levsto(i), 0._r8)
                  particle_protected_start(i) = max(levsto(i), 0._r8)
               ENDIF
      ENDIF

               visible_hflux = sum_hflux_riv(i)
               protected_hflux = 0._r8
               IF (DEF_USE_LEVEE .and. DEF_USE_BIFURCATION) THEN
                  IF (has_levee(i) .and. (.not. is_built_resv(i)) .and. allocated(bif_lev_hflux_sum)) THEN
                     protected_hflux = bif_lev_hflux_sum(i)
                     visible_hflux = visible_hflux - protected_hflux
                  ENDIF
               ENDIF
               volwater = volwater - visible_hflux * dt_all(irivsys(i))
               IF (DEF_USE_LEVEE .and. has_levee(i) .and. (.not. is_built_resv(i))) THEN
                  CALL levee_apply_protected_flux(i, protected_hflux, dt_all(irivsys(i)), protected_clip)
                  IF (protected_clip > 0._r8) CALL CoLM_stop('BIF protected-side limiter failed')
               ENDIF
               volwater = max(volwater, 0.)

               ! for inland depression, remove excess water (to be optimized)
               IF (ucat_next(i) == -10) THEN
                  IF (volwater > topo_rivstomax(i)) THEN
                     hflux_fc(i) = (volwater - topo_rivstomax(i)) / dt_all(irivsys(i))
                     IF (is_built_resv(i)) qresv_out(ucat2resv(i)) = hflux_fc(i)
      IF (DEF_USE_TRACER) THEN
                     IF (volwater > 1.e-6_r8) THEN
                        frac_remove = (volwater - topo_rivstomax(i)) / volwater
                        DO itrc_dep = 1, ntracers
                           IF (.not. tracer_uses_land_water_transport(itrc_dep)) CYCLE
                           IF (allocated(trc_solid)) THEN
                              IF (tracer_has_dissolved_limit(itrc_dep)) &
                                 CALL tracer_equilibrate_dissolved(itrc_dep, volwater, &
                                    trc_mass(itrc_dep, i), trc_solid(itrc_dep, i))
                           ENDIF
                           trc_removed = trc_mass(itrc_dep, i) * frac_remove
                           trc_mass(itrc_dep, i) = trc_mass(itrc_dep, i) - trc_removed
                           trc_flux_out(itrc_dep, i) = trc_removed / dt_all(irivsys(i))
                        ENDDO
                     ENDIF
      ENDIF
                     volwater = topo_rivstomax(i)
                  ENDIF
               ENDIF

               IF (DEF_USE_LEVEE .and. has_levee(i) .and. (.not. is_built_resv(i))) THEN
                  CALL levee_repartition_storage(i, volwater, wdsrf_ucat(i), fldfrc_levee, &
                     vis_vol_bef_lv2, levsto_bef_lv2)
                  levee_floodarea(i) = fldfrc_levee * topo_area(i)
      IF (DEF_USE_TRACER) THEN
                  IF (ntracers > 0) CALL tracer_lifecycle_route_sediment_levee_repartition(i, &
                     vis_vol_bef_lv2, levsto_bef_lv2, volwater, levsto(i))
                  CALL levee_tracer_repartition(i, vis_vol_bef_lv2, levsto_bef_lv2, volwater, levsto(i))
      ENDIF
               ELSE
                  wdsrf_ucat(i) = floodplain_curve(i)%depth (volwater)
               ENDIF

               IF (is_built_resv(i)) THEN
                  volresv(ucat2resv(i)) = volwater
               ELSE
                  volwater_ucat(i) = volwater
               ENDIF

               IF ((.not. is_built_resv(i)) .and. (wdsrf_ucat(i) >= RIVERMIN)) THEN
                  friction = grav * topo_rivman(i)**2 / wdsrf_ucat(i)**(7.0/3.0) * abs(momen_riv(i))
                  momen_riv(i) = (momen_riv(i) &
                     - (sum_mflux_riv(i) - sum_zgrad_riv(i)) / topo_rivare(i) * dt_all(irivsys(i))) &
                     / (1 + friction * dt_all(irivsys(i)))
                  veloc_riv(i) = momen_riv(i) / wdsrf_ucat(i)
               ELSE
                  momen_riv(i) = 0
                  veloc_riv(i) = 0
               ENDIF

               ! inland depression river
               IF ((.not. is_built_resv(i)) .and. (ucat_next(i) == -10)) THEN
                  momen_riv(i) = min(0., momen_riv(i))
                  veloc_riv(i) = min(0., veloc_riv(i))
               ENDIF

               veloc_riv(i) = min(veloc_riv(i),  20.)
               veloc_riv(i) = max(veloc_riv(i), -20.)
               IF ((.not. is_built_resv(i)) .and. (wdsrf_ucat(i) >= RIVERMIN)) &
                  momen_riv(i) = veloc_riv(i) * wdsrf_ucat(i)

            ENDDO

      IF (DEF_USE_TRACER) THEN
            IF (numucat > 0) THEN
               IF (allocated(volresv)) volresv_safe = volresv
               CALL tracer_diag_accumulate_substep (dt_all, irivsys, ucatfilter, wdsrf_ucat, &
                  volresv_safe, ucat2resv_safe, is_built_resv)
            ENDIF
      ENDIF

            DO i = 1, numucat
               IF (ucatfilter(i)) THEN

      IF (DEF_USE_CoLMDEBUG) THEN
                  IF (ucat_next(i) <= 0) THEN
                     totaldis = totaldis + hflux_fc(i)*dt_all(irivsys(i))
      IF (DEF_USE_TRACER) THEN
                     DO itrc = 1, ntracers
                        IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
                        trc_mass_dis(itrc) = trc_mass_dis(itrc) + trc_flux_out(itrc,i)*dt_all(irivsys(i))
                     ENDDO
      ENDIF
                  ENDIF
      ENDIF

                  acctime_ucat(i) = acctime_ucat(i) + dt_all(irivsys(i))

                  a_wdsrf_ucat(i) = a_wdsrf_ucat(i) + wdsrf_ucat(i) * dt_all(irivsys(i))
                  a_veloc_riv (i) = a_veloc_riv (i) + veloc_riv (i) * dt_all(irivsys(i))
                  a_discharge (i) = a_discharge (i) + hflux_fc  (i) * dt_all(irivsys(i))

                  IF (DEF_USE_LEVEE) THEN
                     IF (levee_floodarea(i) > 0._r8) THEN
                        floodarea = levee_floodarea(i)
                     ELSE
                        floodarea = floodplain_curve(i)%floodarea (wdsrf_ucat(i))
                     ENDIF
                  ELSE
                     floodarea = floodplain_curve(i)%floodarea (wdsrf_ucat(i))
                  ENDIF
                  a_floodarea (i) = a_floodarea (i) + floodarea * dt_all(irivsys(i))
      IF (DEF_USE_TRACER) THEN
                  total_floodarea(i) = floodarea
                  IF (DEF_USE_LEVEE .and. levee_floodarea(i) > 0._r8) THEN
                     total_flooddepth(i) = max(levdph(i), max(wdsrf_ucat(i) - floodplain_curve(i)%rivhgt, 0._r8))
                  ELSE
                     total_flooddepth(i) = max(wdsrf_ucat(i) - floodplain_curve(i)%rivhgt, 0._r8)
                  ENDIF
      ENDIF

                  IF (is_built_resv(i)) THEN
                     volwater = volresv(ucat2resv(i))
                  ELSE
                     IF (DEF_USE_BIFURCATION .or. DEF_USE_LEVEE) THEN
                        volwater = volwater_ucat(i)
                     ELSE
                        volwater = floodplain_curve(i)%volume(wdsrf_ucat(i))
                     ENDIF
                  ENDIF
                  rivsto_hist = min(volwater, floodplain_curve(i)%rivstomax)
                  a_rivsto(i) = a_rivsto(i) + rivsto_hist * dt_all(irivsys(i))
                  a_fldsto(i) = a_fldsto(i) + (volwater - rivsto_hist) * dt_all(irivsys(i))
                  a_flddph(i) = a_flddph(i) + &
                     max(wdsrf_ucat(i) - floodplain_curve(i)%rivhgt, 0._r8) * dt_all(irivsys(i))
                  IF (DEF_USE_LEVEE .and. has_levee(i) .and. (.not. is_built_resv(i))) THEN
                     a_storge(i) = a_storge(i) + (volwater + levsto(i)) * dt_all(irivsys(i))
                     a_levsto(i) = a_levsto(i) + levsto(i) * dt_all(irivsys(i))
                     a_levdph(i) = a_levdph(i) + levdph(i) * dt_all(irivsys(i))
                  ELSE
                     a_storge(i) = a_storge(i) + volwater * dt_all(irivsys(i))
                  ENDIF
                  a_sfcelv(i) = a_sfcelv(i) + &
                     (topo_rivelv(i) + wdsrf_ucat(i)) * dt_all(irivsys(i))

                  IF (DEF_USE_BIFURCATION) &
                     a_bifout(i) = a_bifout(i) + bif_hflux_sum(i) * dt_all(irivsys(i))

                  IF (is_built_resv(i)) THEN
                     irsv = ucat2resv(i)
                     acctime_resv(irsv) = acctime_resv(irsv) + dt_all(irivsys(i))
                     a_volresv   (irsv) = a_volresv  (irsv) + volresv  (irsv) * dt_all(irivsys(i))
                     a_qresv_in  (irsv) = a_qresv_in (irsv) + qresv_in (irsv) * dt_all(irivsys(i))
                     a_qresv_out (irsv) = a_qresv_out(irsv) + qresv_out(irsv) * dt_all(irivsys(i))
                  ENDIF

               ENDIF
            ENDDO

            dt_res = dt_res - dt_all

            IF (DEF_USE_BIFURCATION) THEN
               loop_active = next_loop_active
            ELSE
               loop_active = any(dt_res > 0._r8)
            ENDIF

      IF (DEF_USE_TRACER) THEN
            IF (tracer_lifecycle_route_has_active()) THEN
               DO i = 1, numucat
                  IF (ucatfilter(i)) THEN
                     particle_protected_end(i) = 0._r8
                     particle_floodarea(i) = total_floodarea(i)
                     particle_protected_area(i) = 0._r8
                     IF (DEF_USE_LEVEE .and. has_levee(i) .and. (.not. is_built_resv(i))) &
                        particle_protected_area(i) = min(max(levee_floodarea(i) - &
                           levee_frc_data(i) * topo_area(i), 0._r8), &
                           (1._r8 - levee_frc_data(i)) * topo_area(i))
                     IF (is_built_resv(i)) THEN
                        irsv = ucat2resv(i)
                        IF (volresv(irsv) /= spval) THEN
                           particle_water_storage(i) = max(volresv(irsv), 0._r8)
                        ELSE
                           particle_water_storage(i) = max(volwater_ucat(i), 0._r8)
                        ENDIF
                     ELSE
                        particle_water_storage(i) = max(volwater_ucat(i), 0._r8)
                        IF (DEF_USE_LEVEE .and. has_levee(i)) THEN
                           particle_water_storage(i) = particle_water_storage(i) + max(levsto(i), 0._r8)
                           particle_protected_end(i) = max(levsto(i), 0._r8)
                        ENDIF
                     ENDIF
                  ELSE
                     particle_floodarea(i) = 0._r8
                     particle_protected_area(i) = 0._r8
                     particle_water_storage(i) = 0._r8
                  ENDIF
               ENDDO
               CALL tracer_lifecycle_route_diag_accumulate(dt_all, irivsys, ucatfilter, &
                  veloc_riv, wdsrf_ucat, particle_water_storage_start, particle_water_storage, &
                  hflux_fc, particle_floodarea, particle_protected_start, particle_protected_end, &
                  particle_protected_area)
            ENDIF
      ENDIF

         ENDDO

      IF (DEF_USE_CoLMDEBUG) THEN
         totalvol_aft = 0.
         DO i = 1, numucat
            IF (.not. is_built_resv(i)) THEN
               IF (DEF_USE_BIFURCATION .or. DEF_USE_LEVEE) THEN
                  totalvol_aft = totalvol_aft + volwater_ucat(i)
               ELSE
                  totalvol_aft = totalvol_aft + floodplain_curve(i)%volume (wdsrf_ucat(i))
               ENDIF
               IF (DEF_USE_LEVEE .and. has_levee(i)) totalvol_aft = totalvol_aft + levsto(i)
            ELSE
               totalvol_aft = totalvol_aft + volresv(ucat2resv(i))
            ENDIF
         ENDDO
      IF (DEF_USE_TRACER) THEN
         IF (numucat > 0) THEN
            DO itrc = 1, ntracers
               IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
               trc_mass_aft(itrc) = sum(trc_mass(itrc,:)) + sum(trc_inp_buf(itrc,:))
               IF (allocated(trc_levsto)) trc_mass_aft(itrc) = trc_mass_aft(itrc) + sum(trc_levsto(itrc,:))
               IF (allocated(trc_solid)) trc_mass_aft(itrc) = trc_mass_aft(itrc) &
                  + sum(trc_solid(itrc,:)) + sum(trc_levsto_solid(itrc,:))
               trc_mass_dis(itrc) = trc_mass_dis(itrc) + sum(trc_dry_drain(itrc,:))
               IF (allocated(trc_reactive_source)) trc_mass_reactive(itrc) = sum(trc_reactive_source(itrc,:))
            ENDDO
         ENDIF
      ENDIF
      ENDIF
      ENDIF

      IF (DEF_USE_CoLMDEBUG) THEN
#ifdef USEMPI
      IF (.not. p_is_worker) ntimestep = 0
      CALL mpi_allreduce (MPI_IN_PLACE, ntimestep, 1, MPI_INTEGER, MPI_MAX, p_comm_glb, p_err)

      IF (.not. p_is_worker) totalvol_bef = 0.
      IF (.not. p_is_worker) totalvol_aft = 0.
      IF (.not. p_is_worker) totalrnof    = 0.
      IF (.not. p_is_worker) totaldis     = 0.

      CALL mpi_allreduce (MPI_IN_PLACE, totalvol_bef, 1, MPI_REAL8, MPI_SUM, p_comm_glb, p_err)
      CALL mpi_allreduce (MPI_IN_PLACE, totalvol_aft, 1, MPI_REAL8, MPI_SUM, p_comm_glb, p_err)
      CALL mpi_allreduce (MPI_IN_PLACE, totalrnof,    1, MPI_REAL8, MPI_SUM, p_comm_glb, p_err)
      CALL mpi_allreduce (MPI_IN_PLACE, totaldis,     1, MPI_REAL8, MPI_SUM, p_comm_glb, p_err)
      IF (DEF_USE_TRACER) THEN
      IF (ntracers > 0) THEN
         CALL mpi_allreduce (MPI_IN_PLACE, trc_mass_bef, ntracers, MPI_REAL8, MPI_SUM, p_comm_glb, p_err)
         CALL mpi_allreduce (MPI_IN_PLACE, trc_mass_aft, ntracers, MPI_REAL8, MPI_SUM, p_comm_glb, p_err)
         CALL mpi_allreduce (MPI_IN_PLACE, trc_mass_inp, ntracers, MPI_REAL8, MPI_SUM, p_comm_glb, p_err)
         CALL mpi_allreduce (MPI_IN_PLACE, trc_mass_dis, ntracers, MPI_REAL8, MPI_SUM, p_comm_glb, p_err)
         CALL mpi_allreduce (MPI_IN_PLACE, trc_mass_reactive, ntracers, MPI_REAL8, MPI_SUM, p_comm_glb, p_err)
      ENDIF
      ENDIF
#endif
      IF (p_is_master) THEN
         write(*,'(/,A)') 'Checking River Routing Flow ...'
         write(*,'(A,F12.5,A)') 'River Lake Flow minimum average timestep: ', acctime_rnof/ntimestep, ' seconds'
         write(*,'(A,ES8.1,A)') 'Total water before :  ', totalvol_bef,  ' m^3'
         write(*,'(A,ES8.1,A)') 'Total runoff :        ', totalrnof, ' m^3'
         write(*,'(A,ES8.1,A)') 'Total discharge :     ', totaldis,  ' m^3'
         write(*,'(A,ES8.1,A)') 'Total water change :  ', totalvol_aft-totalvol_bef,  ' m^3'
         write(*,'(A,ES8.1,A)') 'Total water balance : ', totalvol_aft-totalvol_bef-totalrnof+totaldis,  ' m^3'
      IF (DEF_USE_TRACER) THEN
         DO itrc = 1, ntracers
            IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
            write(*,'(A,I0,A,ES12.4,A)') 'Tracer(', itrc, ') mass before  : ', trc_mass_bef(itrc), ' R*m3'
            write(*,'(A,I0,A,ES12.4,A)') 'Tracer(', itrc, ') mass input   : ', trc_mass_inp(itrc), ' R*m3'
            write(*,'(A,I0,A,ES12.4,A)') 'Tracer(', itrc, ') mass discharge: ', trc_mass_dis(itrc), ' R*m3'
            write(*,'(A,I0,A,ES12.4,A)') 'Tracer(', itrc, ') mass reactive : ', trc_mass_reactive(itrc), ' R*m3'
            write(*,'(A,I0,A,ES12.4,A)') 'Tracer(', itrc, ') mass after   : ', trc_mass_aft(itrc), ' R*m3'
            write(*,'(A,I0,A,ES12.4,A)') 'Tracer(', itrc, ') mass balance : ', &
               trc_mass_aft(itrc) - trc_mass_bef(itrc) - trc_mass_inp(itrc) + trc_mass_dis(itrc) &
               - trc_mass_reactive(itrc), ' R*m3'
         ENDDO
      ENDIF
      ENDIF
      IF (DEF_USE_TRACER) THEN
      deallocate (trc_mass_bef, trc_mass_aft, trc_mass_inp, trc_mass_dis, trc_mass_reactive)
      ENDIF
      ENDIF

      IF (DEF_GridRiverLake_FloodFeedback) THEN
         flood_vec = (/flood_evap_period, flood_infil_period/)
#ifdef USEMPI
         CALL mpi_allreduce (MPI_IN_PLACE, flood_vec, 2, MPI_REAL8, MPI_SUM, p_comm_glb, p_err)
#endif
         IF (p_is_master .and. any(flood_vec > 0._r8)) &
            write(*,'(A,2(1X,ES24.16))') 'Grid flood feedback evap/infil [m3]:', flood_vec
         flood_evap_period = 0._r8
         flood_infil_period = 0._r8
      ENDIF

      IF (DEF_USE_TRACER) THEN
      IF (p_is_worker) THEN
         CALL tracer_limiter_stats (lim_calls, lim_iter_sum, lim_iter_peak, lim_over_soft, reset = .true.)
#ifdef USEMPI
         CALL mpi_allreduce (MPI_IN_PLACE, lim_calls, 1, MPI_INTEGER, MPI_SUM, p_comm_worker, p_err)
         CALL mpi_allreduce (MPI_IN_PLACE, lim_iter_sum, 1, MPI_INTEGER, MPI_SUM, p_comm_worker, p_err)
         CALL mpi_allreduce (MPI_IN_PLACE, lim_over_soft, 1, MPI_INTEGER, MPI_SUM, p_comm_worker, p_err)
         CALL mpi_allreduce (MPI_IN_PLACE, lim_iter_peak, 1, MPI_INTEGER, MPI_MAX, p_comm_worker, p_err)
#endif
         IF (p_iam_worker == p_root) THEN
            IF (lim_calls > 0 .and. (lim_diag_printed < 5 .or. lim_over_soft > 0)) THEN
               IF (lim_diag_printed < 5) lim_diag_printed = lim_diag_printed + 1
               write(*,'(A,I0,A,F9.3,A,I0,A,I0)') 'River tracer donor limiter: calls=', lim_calls, &
                  ' mean iter=', real(lim_iter_sum, r8) / real(lim_calls, r8), &
                  ' peak=', lim_iter_peak, ' over_soft=', lim_over_soft
            ENDIF
         ENDIF
      ENDIF

      IF (tracer_lifecycle_route_has_active() .and. p_is_worker) THEN
         CALL tracer_lifecycle_route_calc(acctime_rnof)
      ENDIF

      IF (p_is_worker .and. numucat > 0) THEN
         IF (allocated(acc_trc_inp)) acc_trc_inp = 0._r8
         IF (allocated(acc_rnof_ref)) acc_rnof_ref = 0._r8
         IF (allocated(trc_dry_drain)) trc_dry_drain = 0._r8
      ENDIF
      ENDIF

      acctime_rnof = 0.

      IF (p_is_worker) THEN
         IF (numucat > 0) THEN
            acc_rnof_uc = 0.
         ENDIF
      ENDIF

      IF (DEF_USE_TRACER) THEN
      IF (allocated(levee_floodarea)) CALL publish_levee_fldfrc_to_patches (levee_floodarea)
      IF (allocated(total_floodarea)) CALL publish_fldfrc_to_patches (total_floodarea, total_flooddepth)
      ENDIF
      IF (DEF_GridRiverLake_FloodFeedback) CALL publish_flood_feedback(year)

      IF (allocated(is_built_resv)) deallocate(is_built_resv)
      IF (allocated(wdsrf_next   )) deallocate(wdsrf_next   )
      IF (allocated(veloc_next   )) deallocate(veloc_next   )
      IF (allocated(hflux_fc     )) deallocate(hflux_fc     )
      IF (allocated(mflux_fc     )) deallocate(mflux_fc     )
      IF (allocated(zgrad_dn     )) deallocate(zgrad_dn     )
      IF (allocated(hflux_resv   )) deallocate(hflux_resv   )
      IF (allocated(mflux_resv   )) deallocate(mflux_resv   )
      IF (allocated(hflux_sumups )) deallocate(hflux_sumups )
      IF (allocated(mflux_sumups )) deallocate(mflux_sumups )
      IF (allocated(zgrad_sumups )) deallocate(zgrad_sumups )
      IF (allocated(sum_hflux_riv)) deallocate(sum_hflux_riv)
      IF (allocated(sum_hflux_base)) deallocate(sum_hflux_base)
      IF (allocated(sum_mflux_riv)) deallocate(sum_mflux_riv)
      IF (allocated(sum_zgrad_riv)) deallocate(sum_zgrad_riv)
      IF (allocated(normal_outgoing_rate)) deallocate(normal_outgoing_rate)
      IF (allocated(ordinary_scale)) deallocate(ordinary_scale)
      IF (allocated(ordinary_scale_next)) deallocate(ordinary_scale_next)
      IF (allocated(volresv_safe)) deallocate(volresv_safe)
      IF (allocated(levee_floodarea)) deallocate(levee_floodarea)
      IF (DEF_USE_TRACER) THEN
      IF (allocated(ucat2resv_safe)) deallocate(ucat2resv_safe)
      IF (allocated(total_floodarea)) deallocate(total_floodarea)
      IF (allocated(total_flooddepth)) deallocate(total_flooddepth)
      IF (allocated(particle_floodarea)) deallocate(particle_floodarea, particle_protected_area)
      IF (allocated(particle_water_storage)) deallocate(particle_water_storage_start, particle_water_storage)
      IF (allocated(particle_protected_start)) deallocate(particle_protected_start, particle_protected_end)
      ENDIF
      IF (allocated(ucatfilter   )) deallocate(ucatfilter   )
      IF (allocated(dt_res       )) deallocate(dt_res       )
      IF (allocated(dt_all       )) deallocate(dt_all       )

   END SUBROUTINE grid_riverlake_flow

   SUBROUTINE publish_levee_fldfrc_to_patches (levee_floodarea_in)

   USE MOD_LandPatch, only: numpatch
   IMPLICIT NONE

   real(r8), intent(in) :: levee_floodarea_in(:)
   real(r8), allocatable :: fldfrc_uc(:), fldfrc_gd(:), fldfrc_patch(:)
   integer :: i

      IF (.not. p_is_worker) RETURN
      IF (.not. tracer_lifecycle_has_levee_flood_publisher()) RETURN

      allocate (fldfrc_uc(numucat), fldfrc_gd(numinpm), fldfrc_patch(numpatch))
      DO i = 1, numucat
         IF (topo_area(i) > 0._r8 .and. i <= size(levee_floodarea_in)) THEN
            fldfrc_uc(i) = min(1._r8, max(0._r8, levee_floodarea_in(i) / topo_area(i)))
         ELSE
            fldfrc_uc(i) = 0._r8
         ENDIF
      ENDDO

      CALL worker_push_data (push_ucat2inpm, fldfrc_uc, fldfrc_gd, &
         fillvalue = RIVERLAKE_FLOOD_MISSING_VALUE, mode = 'average')
      CALL worker_remap_data_grid2pset (remap_patch2inpm, fldfrc_gd, &
         fldfrc_patch, fillvalue = RIVERLAKE_FLOOD_MISSING_VALUE, mode = 'average')
      WHERE (fldfrc_patch == RIVERLAKE_FLOOD_MISSING_VALUE) fldfrc_patch = 0._r8
      CALL tracer_lifecycle_publish_levee_flood_patch (fldfrc_patch)

      deallocate (fldfrc_uc, fldfrc_gd, fldfrc_patch)

   END SUBROUTINE publish_levee_fldfrc_to_patches

   SUBROUTINE publish_fldfrc_to_patches (total_floodarea_in, total_flooddepth_in)

   USE MOD_LandPatch, only: numpatch
   IMPLICIT NONE

   real(r8), intent(in) :: total_floodarea_in(:)
   real(r8), intent(in) :: total_flooddepth_in(:)
   real(r8), allocatable :: fldfrc_uc(:), fldfrc_gd(:), fldwat_uc(:), fldwat_gd(:)
   real(r8), allocatable :: fldfrc_patch(:), fldwat_patch(:), flddph_patch(:)
   integer :: i

      IF (.not. p_is_worker) RETURN
      IF (.not. tracer_lifecycle_has_flood_publisher()) RETURN

      allocate (fldfrc_uc(numucat), fldwat_uc(numucat), fldfrc_gd(numinpm), fldwat_gd(numinpm))
      allocate (fldfrc_patch(numpatch), fldwat_patch(numpatch), flddph_patch(numpatch))
      fldwat_uc = 0._r8
      DO i = 1, numucat
         IF (topo_area(i) > 0._r8 .and. i <= size(total_floodarea_in)) THEN
            fldfrc_uc(i) = min(1._r8, max(0._r8, total_floodarea_in(i) / topo_area(i)))
            IF (i <= size(total_flooddepth_in)) &
               fldwat_uc(i) = fldfrc_uc(i) * max(0._r8, total_flooddepth_in(i))
         ELSE
            fldfrc_uc(i) = 0._r8
         ENDIF
      ENDDO

      CALL worker_push_data (push_ucat2inpm, fldfrc_uc, fldfrc_gd, &
         fillvalue = RIVERLAKE_FLOOD_MISSING_VALUE, mode = 'average')
      CALL worker_remap_data_grid2pset (remap_patch2inpm, fldfrc_gd, &
         fldfrc_patch, fillvalue = RIVERLAKE_FLOOD_MISSING_VALUE, mode = 'average')
      WHERE (fldfrc_patch == RIVERLAKE_FLOOD_MISSING_VALUE) fldfrc_patch = 0._r8

      CALL worker_push_data (push_ucat2inpm, fldwat_uc, fldwat_gd, &
         fillvalue = RIVERLAKE_FLOOD_MISSING_VALUE, mode = 'average')
      CALL worker_remap_data_grid2pset (remap_patch2inpm, fldwat_gd, &
         fldwat_patch, fillvalue = RIVERLAKE_FLOOD_MISSING_VALUE, mode = 'average')
      WHERE (fldwat_patch == RIVERLAKE_FLOOD_MISSING_VALUE) fldwat_patch = 0._r8

      flddph_patch = 0._r8
      WHERE (fldfrc_patch > 0._r8) flddph_patch = max(0._r8, fldwat_patch) / fldfrc_patch
      CALL tracer_lifecycle_publish_flood_patch (fldfrc_patch, flddph_patch)

      deallocate (fldfrc_uc, fldfrc_gd, fldwat_uc, fldwat_gd, fldfrc_patch, fldwat_patch, flddph_patch)

   END SUBROUTINE publish_fldfrc_to_patches

   SUBROUTINE sync_global_routing_dt(dt_res, dt_all, next_loop_active)

   USE, INTRINSIC :: ieee_arithmetic, only: ieee_is_finite
   IMPLICIT NONE

   real(r8), intent(in) :: dt_res(:)
   real(r8), intent(inout) :: dt_all(:)
   logical, intent(out) :: next_loop_active
   real(r8) :: dt_reduce(2), dt_global, max_residual
   logical :: pathological_mask(size(dt_res))
   integer :: i

      pathological_mask = .false.
      DO i = 1, size(dt_res)
         IF (.not. ieee_is_finite(dt_res(i))) CYCLE
         IF (dt_res(i) > 0._r8) THEN
            IF (.not. ieee_is_finite(dt_all(i))) THEN
               pathological_mask(i) = .true.
            ELSEIF (dt_all(i) <= 0._r8) THEN
               pathological_mask(i) = .true.
            ENDIF
         ENDIF
      ENDDO

      IF (any(pathological_mask)) THEN
         WHERE (pathological_mask)
            dt_all = min(10._r8, dt_res)
         END WHERE
         IF (p_is_worker .and. p_iam_worker == p_root .and. routing_zero_dt_warn_count < 5) THEN
            routing_zero_dt_warn_count = routing_zero_dt_warn_count + 1
            write(*,'(A)') 'WARNING grid_riverlake_flow: invalid adaptive dt; using bounded fallback.'
         ENDIF
      ENDIF

      dt_reduce = [huge(1._r8), 0._r8]
      IF (any(.not. ieee_is_finite(dt_res))) THEN
         dt_reduce(1) = -huge(1._r8)
      ELSEIF (any(dt_res > 0._r8)) THEN
         dt_reduce(1) = minval(dt_all, mask=dt_res > 0._r8)
         dt_reduce(2) = -maxval(dt_res, mask=dt_res > 0._r8)
      ENDIF

#ifdef USEMPI
      CALL mpi_allreduce (MPI_IN_PLACE, dt_reduce, 2, MPI_REAL8, MPI_MIN, p_comm_worker, p_err)
#endif

      IF (.not. ieee_is_finite(dt_reduce(1))) &
         CALL CoLM_stop('grid_riverlake_flow: invalid synchronized routing dt')
      IF (dt_reduce(1) <= -0.5_r8 * huge(1._r8)) &
         CALL CoLM_stop('grid_riverlake_flow: non-finite routing residual')

      dt_global = dt_reduce(1)
      max_residual = -dt_reduce(2)
      next_loop_active = .false.

      IF (dt_global < 0.5_r8 * huge(1._r8)) THEN
         IF (.not. ieee_is_finite(max_residual)) &
            CALL CoLM_stop('grid_riverlake_flow: invalid synchronized routing dt')
         IF (dt_global <= 0._r8) &
            CALL CoLM_stop('grid_riverlake_flow: non-positive synchronized routing dt')
         IF (max_residual > 0._r8 .and. max_residual - dt_global >= max_residual) &
            CALL CoLM_stop('grid_riverlake_flow: routing dt makes no numerical progress')
         next_loop_active = max_residual - dt_global > 0._r8
         WHERE (dt_res > 0._r8)
            dt_all = dt_global
         ELSEWHERE
            dt_all = 0._r8
         END WHERE
      ELSE
         dt_all = 0._r8
      ENDIF

   END SUBROUTINE sync_global_routing_dt

   SUBROUTINE publish_flood_feedback(year)

   USE MOD_LandPatch, only: numpatch
   USE MOD_Vars_Global, only: spval
   USE MOD_Tracer_Defs, only: ntracers, tracer_uses_land_water_transport
   IMPLICIT NONE
   integer, intent(in) :: year
   integer :: i
   real(r8) :: visible, protected, stage, protected_stage, protected_new, fraction
   real(r8), allocatable :: density(:), grid_visible(:), grid_protected(:)
   real(r8), allocatable :: frac_uc(:), frac_grid(:)
   integer :: itrc
   real(r8), allocatable :: grid_tracer(:), patch_tracer(:)

      IF (.not. p_is_worker) RETURN
      allocate(density(numucat), grid_visible(numinpm), grid_protected(numinpm), &
         frac_uc(numucat), frac_grid(numinpm))
      flood_visible_uc = 0._r8
      flood_protected_uc = 0._r8
      flood_reservoir_uc = .false.
      IF (DEF_USE_TRACER) THEN
      IF (allocated(flood_visible_tracer_uc)) THEN
         flood_visible_tracer_uc = 0._r8
         flood_protected_tracer_uc = 0._r8
         flood_tracer_credit_patch = 0._r8
         flood_tracer_evap_patch = 0._r8
         flood_tracer_land_patch = 0._r8
      ENDIF
      ENDIF
      frac_uc = 0._r8
      DO i = 1, numucat
         IF (lake_type(i) == 2 .and. allocated(ucat2resv)) THEN
            IF (ucat2resv(i) > 0 .and. allocated(dam_build_year)) THEN
               flood_reservoir_uc(i) = year >= dam_build_year(ucat2resv(i))
            ENDIF
         ENDIF
         visible = max(0._r8, volwater_ucat(i))
         IF (.not. volwater_ucat_valid) visible = max(0._r8, floodplain_curve(i)%volume(wdsrf_ucat(i)))
         IF (flood_reservoir_uc(i)) THEN
            IF (volresv(ucat2resv(i)) == spval) &
               volresv(ucat2resv(i)) = floodplain_curve(i)%volume(wdsrf_ucat(i))
            visible = max(0._r8, volresv(ucat2resv(i)))
         ENDIF
         protected = 0._r8
         IF (DEF_USE_LEVEE .and. has_levee(i)) protected = max(0._r8, levsto(i))
      IF (DEF_USE_TRACER) THEN
         IF (allocated(trc_solid)) CALL equilibrate_river_tracer_cell(i, visible, protected)
      ENDIF
         IF (DEF_USE_LEVEE .and. has_levee(i)) THEN
            CALL levee_fldstg(i, visible+protected, stage, protected_new, protected_stage, fraction)
         ELSE
            stage = floodplain_curve(i)%depth(visible)
            fraction = floodplain_curve(i)%floodarea(stage) / max(topo_area(i), 1._r8)
         ENDIF
         IF (fraction <= 0._r8) CYCLE
         flood_visible_uc(i) = max(0._r8, visible - topo_rivstomax(i))
         flood_protected_uc(i) = protected
         frac_uc(i) = min(1._r8, fraction)
      IF (DEF_USE_TRACER) THEN
         IF (allocated(flood_visible_tracer_uc)) THEN
            IF (visible > 0._r8) flood_visible_tracer_uc(:,i) = &
               max(trc_mass(:,i), 0._r8) * (flood_visible_uc(i) / visible)
            IF (protected > 0._r8) flood_protected_tracer_uc(:,i) = max(trc_levsto(:,i), 0._r8)
         ENDIF
      ENDIF
      ENDDO

      density = 0._r8
      WHERE (push_inpm2ucat%sum_area > 0._r8) &
         density = flood_visible_uc / max(push_inpm2ucat%sum_area, tiny(1._r8))
      CALL worker_push_data(push_ucat2inpm, density, grid_visible, fillvalue=0._r8, mode='sum')
      density = 0._r8
      WHERE (push_inpm2ucat%sum_area > 0._r8) &
         density = flood_protected_uc / max(push_inpm2ucat%sum_area, tiny(1._r8))
      CALL worker_push_data(push_ucat2inpm, density, grid_protected, fillvalue=0._r8, mode='sum')
      CALL worker_push_data(push_ucat2inpm, frac_uc, frac_grid, fillvalue=0._r8, mode='average')
      WHERE (flood_grid_area > 0._r8)
         frac_grid = min(1._r8, max(0._r8, frac_grid))
         grid_visible = grid_visible / max(flood_grid_area, tiny(1._r8))
         grid_protected = grid_protected / max(flood_grid_area, tiny(1._r8))
      ELSEWHERE
         frac_grid = 0._r8
         grid_visible = 0._r8
         grid_protected = 0._r8
      END WHERE
      IF (numpatch > 0) THEN
         CALL worker_remap_data_grid2pset(remap_patch2inpm, grid_visible, &
            flood_credit_patch, fillvalue=RIVERLAKE_FLOOD_MISSING_VALUE, mode='average')
         WHERE (flood_credit_patch == RIVERLAKE_FLOOD_MISSING_VALUE) flood_credit_patch = 0._r8
         CALL worker_remap_data_grid2pset(remap_patch2inpm, grid_protected, &
            flood_depth_patch, fillvalue=RIVERLAKE_FLOOD_MISSING_VALUE, mode='average')
         WHERE (flood_depth_patch == RIVERLAKE_FLOOD_MISSING_VALUE) flood_depth_patch = 0._r8
         flood_credit_patch = max(0._r8, flood_credit_patch) + max(0._r8, flood_depth_patch)
         CALL worker_remap_data_grid2pset(remap_patch2inpm, frac_grid, &
            flood_fraction_patch, fillvalue=RIVERLAKE_FLOOD_MISSING_VALUE, mode='average')
         WHERE (flood_fraction_patch == RIVERLAKE_FLOOD_MISSING_VALUE) flood_fraction_patch = 0._r8
         flood_fraction_patch = min(1._r8, max(0._r8, flood_fraction_patch))
         flood_depth_patch = 0._r8
         WHERE (flood_fraction_patch > epsilon(1._r8)) &
            flood_depth_patch = 1000._r8 * flood_credit_patch / max(flood_fraction_patch, tiny(1._r8))
      ENDIF
      IF (DEF_USE_TRACER) THEN
      IF (allocated(flood_visible_tracer_uc)) THEN
         allocate(grid_tracer(numinpm), patch_tracer(numpatch))
         DO itrc = 1, ntracers
            IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
            density = 0._r8
            WHERE (push_inpm2ucat%sum_area > 0._r8) &
               density = flood_visible_tracer_uc(itrc,:) / &
                  max(push_inpm2ucat%sum_area, tiny(1._r8))
            CALL worker_push_data(push_ucat2inpm, density, grid_tracer, fillvalue=0._r8, mode='sum')
            WHERE (flood_grid_area > 0._r8)
               grid_tracer = grid_tracer / max(flood_grid_area, tiny(1._r8))
            ELSEWHERE
               grid_tracer = 0._r8
            END WHERE
            IF (numpatch > 0) THEN
               CALL worker_remap_data_grid2pset(remap_patch2inpm, grid_tracer, &
                  flood_tracer_credit_patch(itrc,:), fillvalue=RIVERLAKE_FLOOD_MISSING_VALUE, mode='average')
               WHERE (flood_tracer_credit_patch(itrc,:) == RIVERLAKE_FLOOD_MISSING_VALUE) &
                  flood_tracer_credit_patch(itrc,:) = 0._r8
            ENDIF
            density = 0._r8
            WHERE (push_inpm2ucat%sum_area > 0._r8) &
               density = flood_protected_tracer_uc(itrc,:) / &
                  max(push_inpm2ucat%sum_area, tiny(1._r8))
            CALL worker_push_data(push_ucat2inpm, density, grid_tracer, fillvalue=0._r8, mode='sum')
            WHERE (flood_grid_area > 0._r8)
               grid_tracer = grid_tracer / max(flood_grid_area, tiny(1._r8))
            ELSEWHERE
               grid_tracer = 0._r8
            END WHERE
            IF (numpatch > 0) THEN
               CALL worker_remap_data_grid2pset(remap_patch2inpm, grid_tracer, &
                  patch_tracer, fillvalue=RIVERLAKE_FLOOD_MISSING_VALUE, mode='average')
               WHERE (patch_tracer == RIVERLAKE_FLOOD_MISSING_VALUE) patch_tracer = 0._r8
               flood_tracer_credit_patch(itrc,:) = 1000._r8 * &
                  (flood_tracer_credit_patch(itrc,:) + patch_tracer)
            ENDIF
         ENDDO
      ENDIF
      ENDIF
   END SUBROUTINE publish_flood_feedback


   SUBROUTINE debit_flood_feedback(evap_volume, infil_volume)

   USE MOD_LandPatch, only: numpatch
   USE MOD_Tracer_Defs, only: ntracers, tracer_uses_land_water_transport, &
      tracer_is_nonvolatile_solute, tracer_has_dissolved_limit
   USE, INTRINSIC :: ieee_arithmetic, only: ieee_is_finite
   IMPLICIT NONE
   real(r8), intent(out) :: evap_volume, infil_volume
   real(r8), allocatable :: ratio_patch(:), ratio_grid(:), ratio_uc(:), fraction_sum(:)
   real(r8), allocatable :: gain_patch(:), coefficient_uc(:), gain_uc(:)
   real(r8), allocatable :: tracer_exchange_ledger(:,:)
   real(r8) :: patch_area, tracer_cell_debit, ledger_tol
   real(r8) :: water_credit, tracer_credit, vapor_loss, positive_loss, vapor_gain
   real(r8) :: infiltrated_fraction, evaporated_fraction, tracer_after
   integer :: itrc
   real(r8) :: debit_fraction, sink, credit, tol, fraction
   real(r8) :: visible_before, protected_before
   integer :: i, j

      evap_volume = 0._r8
      infil_volume = 0._r8
      IF (.not. p_is_worker) RETURN
      allocate(ratio_patch(numpatch), ratio_grid(numinpm), ratio_uc(numucat), fraction_sum(numucat))
      fraction_sum = 0._r8
      IF (DEF_USE_TRACER) THEN
      IF (allocated(flood_visible_tracer_uc)) THEN
         allocate(gain_patch(numpatch), coefficient_uc(numucat), gain_uc(numucat), &
            tracer_exchange_ledger(3,ntracers))
         tracer_exchange_ledger = 0._r8
      ENDIF
      ENDIF
      DO i = 1, numpatch
         IF (.not. ieee_is_finite(flood_evap_acc(i)) .or. &
             .not. ieee_is_finite(flood_infil_acc(i))) &
            CALL CoLM_stop('grid flood feedback: nonfinite land sink')
         sink = (flood_evap_acc(i) + flood_infil_acc(i)) * 1.e-3_r8
         credit = flood_credit_patch(i)
         tol = 1.e-10_r8 * max(credit, 1.e-6_r8)
         IF (sink < -tol .or. sink > credit + tol) &
            CALL CoLM_stop('grid flood feedback: land exceeded published patch credit')
      ENDDO
      DO i = 1, 2
         ratio_patch = 0._r8
         IF (i == 1) THEN
            WHERE (flood_credit_patch > 0._r8) &
               ratio_patch = max(0._r8, flood_evap_acc*1.e-3_r8) / max(flood_credit_patch, tiny(1._r8))
         ELSE
            WHERE (flood_credit_patch > 0._r8) &
               ratio_patch = max(0._r8, flood_infil_acc*1.e-3_r8) / max(flood_credit_patch, tiny(1._r8))
         ENDIF
         CALL worker_remap_data_pset2grid(remap_patch2inpm, ratio_patch, ratio_grid, &
            fillvalue=0._r8, filter=flood_credit_patch>0._r8)
         WHERE (flood_grid_area > 0._r8)
            ratio_grid = ratio_grid / max(flood_grid_area, tiny(1._r8))
         ELSEWHERE
            ratio_grid = 0._r8
         END WHERE
         IF (any(ratio_grid < -1.e-10_r8) .or. any(ratio_grid > 1._r8+1.e-10_r8)) &
            CALL CoLM_stop('grid flood feedback: grid sink exceeded donor credit')
         CALL worker_push_data(push_inpm2ucat, ratio_grid, ratio_uc, fillvalue=0._r8, mode='sum')
         DO j = 1, numucat
            IF (push_inpm2ucat%sum_area(j) > 0._r8) THEN
               debit_fraction = min(1._r8, max(0._r8, ratio_uc(j) / push_inpm2ucat%sum_area(j)))
            ELSE
               debit_fraction = 0._r8
            ENDIF
            IF (i == 1) THEN
               evap_volume = evap_volume + debit_fraction * &
                  (flood_visible_uc(j) + flood_protected_uc(j))
            ELSE
               infil_volume = infil_volume + debit_fraction * &
                  (flood_visible_uc(j) + flood_protected_uc(j))
            ENDIF
            fraction_sum(j) = fraction_sum(j) + debit_fraction
         ENDDO
      ENDDO
      IF (any(fraction_sum > 1._r8+1.e-10_r8)) &
         CALL CoLM_stop('grid flood feedback: donor overdraft')
      IF (DEF_USE_TRACER) THEN
      IF (allocated(flood_visible_tracer_uc)) THEN
         DO itrc = 1, ntracers
            IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
            ratio_patch = 0._r8
            gain_patch = 0._r8
            DO i = 1, numpatch
               water_credit = flood_credit_patch(i)*1000._r8
               IF (water_credit <= 0._r8) CYCLE
               tracer_credit = flood_tracer_credit_patch(itrc,i)
               vapor_loss = flood_tracer_evap_patch(itrc,i)
               IF (.not. ieee_is_finite(vapor_loss)) &
                  CALL CoLM_stop('grid flood feedback: nonfinite isotope vapor exchange')
               positive_loss = max(vapor_loss,0._r8)
               vapor_gain = max(-vapor_loss,0._r8)
               ledger_tol = max(1.e-12_r8, 1.e-10_r8*abs(tracer_credit))
               IF (positive_loss > tracer_credit+ledger_tol) &
                  CALL CoLM_stop('grid flood feedback: isotope evaporation overdrew tracer')
               evaporated_fraction = 0._r8
               IF (tracer_credit > 0._r8) evaporated_fraction = &
                  min(1._r8,positive_loss/tracer_credit)
               infiltrated_fraction = 0._r8
               IF (flood_infil_acc(i) > 0._r8) THEN
                  IF (water_credit <= flood_evap_acc(i)) THEN
                     IF (flood_infil_acc(i) > 1.e-12_r8*max(water_credit, 1._r8)) &
                        CALL CoLM_stop('grid flood feedback: infiltration after full evaporation')
                  ELSE
                     infiltrated_fraction = flood_infil_acc(i)/(water_credit-flood_evap_acc(i))
                  ENDIF
               ENDIF
               IF (infiltrated_fraction > 1._r8+1.e-10_r8) &
                  CALL CoLM_stop('grid flood feedback: tracer infiltration overdraft')
               infiltrated_fraction = min(1._r8,max(0._r8,infiltrated_fraction))
               ratio_patch(i) = evaporated_fraction + &
                  (1._r8-evaporated_fraction)*infiltrated_fraction
               IF (tracer_has_dissolved_limit(itrc) .and. tracer_credit > 0._r8) &
                  ratio_patch(i) = min(1._r8,max(0._r8,flood_tracer_land_patch(itrc,i)/tracer_credit))
               gain_patch(i) = vapor_gain*(1._r8-infiltrated_fraction)/water_credit
               patch_area = sum(remap_patch2inpm%areapart(i)%val)*1.e-3_r8
               IF (tracer_has_dissolved_limit(itrc)) THEN
                  IF (flood_tracer_land_patch(itrc,i) < -ledger_tol .or. &
                      flood_tracer_land_patch(itrc,i) > &
                      (tracer_credit-vapor_loss)*infiltrated_fraction + ledger_tol) &
                     CALL CoLM_stop('grid flood feedback: finite solute input exceeds donor exchange')
               ELSE
                  IF (abs(flood_tracer_land_patch(itrc,i) - &
                      (tracer_credit-vapor_loss)*infiltrated_fraction) > &
                      max(1.e-12_r8,1.e-10_r8*max(abs(tracer_credit),abs(flood_tracer_land_patch(itrc,i))))) &
                     CALL CoLM_stop('grid flood feedback: land tracer input differs from donor exchange')
               ENDIF
               tracer_exchange_ledger(1,itrc) = tracer_exchange_ledger(1,itrc) + &
                  flood_tracer_land_patch(itrc,i)*patch_area
               tracer_exchange_ledger(2,itrc) = tracer_exchange_ledger(2,itrc) + &
                  vapor_loss*patch_area
            ENDDO
            CALL worker_remap_data_pset2grid(remap_patch2inpm, ratio_patch, ratio_grid, &
               fillvalue=0._r8, filter=flood_credit_patch>0._r8)
            WHERE (flood_grid_area > 0._r8)
               ratio_grid = ratio_grid/max(flood_grid_area,tiny(1._r8))
            ELSEWHERE
               ratio_grid = 0._r8
            END WHERE
            CALL worker_push_data(push_inpm2ucat, ratio_grid, ratio_uc, fillvalue=0._r8, mode='sum')
            coefficient_uc = 0._r8
            WHERE (push_inpm2ucat%sum_area > 0._r8) &
               coefficient_uc = min(1._r8,max(0._r8,ratio_uc/push_inpm2ucat%sum_area))
            gain_uc = 0._r8
            IF (.not. tracer_is_nonvolatile_solute(itrc)) THEN
               CALL worker_remap_data_pset2grid(remap_patch2inpm, gain_patch, ratio_grid, &
                  fillvalue=0._r8, filter=flood_credit_patch>0._r8)
               WHERE (flood_grid_area > 0._r8)
                  ratio_grid = ratio_grid/max(flood_grid_area,tiny(1._r8))
               ELSEWHERE
                  ratio_grid = 0._r8
               END WHERE
               CALL worker_push_data(push_inpm2ucat, ratio_grid, ratio_uc, fillvalue=0._r8, mode='sum')
               WHERE (push_inpm2ucat%sum_area > 0._r8) &
                  gain_uc = ratio_uc/push_inpm2ucat%sum_area
            ENDIF
            DO j = 1, numucat
               tracer_after = trc_mass(itrc,j) - coefficient_uc(j)*flood_visible_tracer_uc(itrc,j) &
                  + gain_uc(j)*flood_visible_uc(j)
               IF (tracer_after < -1.e-10_r8*max(1._r8,trc_mass(itrc,j))) &
                  CALL CoLM_stop('grid flood feedback: negative visible tracer')
               tracer_cell_debit = coefficient_uc(j)*flood_visible_tracer_uc(itrc,j) &
                  - gain_uc(j)*flood_visible_uc(j) + min(tracer_after,0._r8)
               trc_mass(itrc,j) = max(0._r8,tracer_after)
               tracer_after = trc_levsto(itrc,j) - coefficient_uc(j)*flood_protected_tracer_uc(itrc,j) &
                  + gain_uc(j)*flood_protected_uc(j)
               IF (tracer_after < -1.e-10_r8*max(1._r8,trc_levsto(itrc,j))) &
                  CALL CoLM_stop('grid flood feedback: negative protected tracer')
               tracer_cell_debit = tracer_cell_debit + coefficient_uc(j)*flood_protected_tracer_uc(itrc,j) &
                  - gain_uc(j)*flood_protected_uc(j) + min(tracer_after,0._r8)
               trc_levsto(itrc,j) = max(0._r8,tracer_after)
               tracer_exchange_ledger(3,itrc) = tracer_exchange_ledger(3,itrc) + &
                  tracer_cell_debit
            ENDDO
         ENDDO
#ifdef USEMPI
         CALL mpi_allreduce(MPI_IN_PLACE, tracer_exchange_ledger, 3*ntracers, MPI_REAL8, &
            MPI_SUM, p_comm_worker, p_err)
#endif
         DO itrc = 1, ntracers
            IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
            ledger_tol = max(1.e-8_r8, 1.e-10_r8*maxval(abs(tracer_exchange_ledger(:,itrc))))
            IF (abs(tracer_exchange_ledger(1,itrc)+tracer_exchange_ledger(2,itrc) - &
                   tracer_exchange_ledger(3,itrc)) > ledger_tol) &
               CALL CoLM_stop('grid flood feedback: land/river tracer exchange ledger mismatch')
            IF (p_iam_worker == p_root .and. flood_tracer_ledger_reports < 5 .and. &
                tracer_exchange_ledger(1,itrc) > 0._r8) THEN
               write(*,'(A,I0,4(1X,ES16.8))') 'Grid flood tracer land/vapor/donor/residual:', itrc, &
                  tracer_exchange_ledger(1,itrc), tracer_exchange_ledger(2,itrc), &
                  tracer_exchange_ledger(3,itrc), &
                  tracer_exchange_ledger(1,itrc)+tracer_exchange_ledger(2,itrc)-tracer_exchange_ledger(3,itrc)
               flood_tracer_ledger_reports = flood_tracer_ledger_reports + 1
            ENDIF
         ENDDO
      ENDIF
      ENDIF
      DO j = 1, numucat
         IF (flood_reservoir_uc(j)) THEN
            volresv(ucat2resv(j)) = max(0._r8, volresv(ucat2resv(j)) - fraction_sum(j)*flood_visible_uc(j))
            volwater_ucat(j) = volresv(ucat2resv(j))
         ELSE
            volwater_ucat(j) = max(0._r8, volwater_ucat(j) - fraction_sum(j)*flood_visible_uc(j))
         ENDIF
         IF (DEF_USE_LEVEE .and. has_levee(j)) &
            levsto(j) = max(0._r8, levsto(j) - fraction_sum(j)*flood_protected_uc(j))
      IF (DEF_USE_TRACER) THEN
         IF (allocated(trc_solid)) THEN
            protected_before = 0._r8
            IF (DEF_USE_LEVEE .and. has_levee(j)) protected_before = levsto(j)
            CALL equilibrate_river_tracer_cell(j, volwater_ucat(j), protected_before)
         ENDIF
      ENDIF
         IF (fraction_sum(j) > 0._r8) THEN
            IF (DEF_USE_LEVEE .and. has_levee(j) .and. .not. flood_reservoir_uc(j)) THEN
               CALL levee_repartition_storage(j, volwater_ucat(j), wdsrf_ucat(j), &
                  fraction, visible_before, protected_before)
      IF (DEF_USE_TRACER) THEN
               IF (ntracers > 0) CALL tracer_lifecycle_route_sediment_levee_repartition(j, &
                  visible_before, protected_before, volwater_ucat(j), levsto(j))
               IF (ntracers > 0) CALL levee_tracer_repartition(j, &
                  visible_before, protected_before, volwater_ucat(j), levsto(j))
      ENDIF
            ELSEIF (flood_reservoir_uc(j)) THEN
               wdsrf_ucat(j) = floodplain_curve(j)%depth(volresv(ucat2resv(j)))
            ELSE
               wdsrf_ucat(j) = floodplain_curve(j)%depth(volwater_ucat(j))
            ENDIF
         ENDIF
      ENDDO
      flood_evap_acc = 0._r8
      flood_infil_acc = 0._r8
   END SUBROUTINE debit_flood_feedback

   ! ---------
   SUBROUTINE grid_riverlake_flow_final ()

      CALL riverlake_network_final ()

      IF (DEF_Reservoir_Method > 0) THEN
         CALL reservoir_final ()
      ENDIF

      IF (DEF_USE_TRACER) THEN
      CALL tracer_lifecycle_route_final()
      ENDIF

      CALL levee_final()
      CALL bifurcation_final()

      IF (DEF_USE_TRACER) THEN
      CALL river_lake_tracer_final()
      ENDIF

      IF (allocated(filter_rnof)) deallocate(filter_rnof)
      IF (allocated(flood_depth_patch)) deallocate(flood_depth_patch, flood_fraction_patch, &
         flood_evap_patch, flood_infil_patch)
      IF (allocated(flood_visible_uc)) deallocate(flood_visible_uc, flood_protected_uc, &
         flood_credit_patch, flood_evap_acc, flood_infil_acc, flood_reservoir_uc, flood_grid_area)
      IF (DEF_USE_TRACER) THEN
      IF (allocated(flood_tracer_credit_patch)) deallocate(flood_tracer_credit_patch, &
         flood_tracer_evap_patch, flood_tracer_land_patch, &
         flood_visible_tracer_uc, flood_protected_tracer_uc)
      ENDIF

   END SUBROUTINE grid_riverlake_flow_final

END MODULE MOD_Grid_RiverLakeFlow
#endif
