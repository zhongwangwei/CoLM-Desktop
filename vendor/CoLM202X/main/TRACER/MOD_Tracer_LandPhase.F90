#include <define.h>

#ifdef TRACER
MODULE MOD_Tracer_LandPhase

   USE MOD_Precision
   USE MOD_Tracer_Defs
   USE MOD_Tracer_Vars
   USE MOD_Tracer_Precip
   USE MOD_Tracer_Evapo
   USE MOD_Tracer_SoilWater
   USE MOD_Tracer_Snow
   USE MOD_Tracer_Conservation
   USE MOD_Tracer_Rest
   USE MOD_Tracer_SoilInit
   USE MOD_SPMD_Task
   USE MOD_Namelist, only: DEF_USE_IRRIGATION
   USE MOD_Tracer_Lifecycle, only: tracer_lifecycle_init, &
      tracer_lifecycle_land_init, tracer_lifecycle_land_final, &
      tracer_lifecycle_lake_step, &
      tracer_lifecycle_wetland_decomp, tracer_lifecycle_soil_step, &
      tracer_lifecycle_land_report, tracer_lifecycle_land_flush_acc_fluxes, &
      tracer_lifecycle_land_accumulate_fluxes, tracer_lifecycle_land_read_restart

   IMPLICIT NONE
   PRIVATE

   ! Land-side multi-phase TRACER facade. Keeps the implementation split
   ! across physics modules but exposes one import surface per host, so
   ! drivers do not USE every low-level tracer module directly. NOTE: this
   ! is a facade, not the engine -- per-step driving lives in the called
   ! modules. River/lake ROUTING tracers are driven separately in HYDRO
   ! (MOD_Grid_RiverLakeFlow -> MOD_Tracer_Lifecycle) and are NOT re-exported
   ! here. PUBLIC declarations below are grouped by consuming host phase.

   ! -- Lifecycle: one-shot init/final, called from CoLM.F90 --------------
   PUBLIC :: land_tracer_init, land_tracer_final

   ! -- Column phase: per-patch soil-water/precip/evap/snow transport,
   !    called inside the CoLMMAIN patch loop (per ipatch, snl, nl_soil) --
   PUBLIC :: ntracers, trc_tiny, tracer_uses_land_water_transport
   PUBLIC :: tracer_precip, tracer_evapo, tracer_soil_water, tracer_wetland
   PUBLIC :: tracer_newsnow, tracer_save_storage, tracer_balance_check
   PUBLIC :: tracer_apply_reactive_processes
   PUBLIC :: trc_wliq_soisno, trc_wice_soisno, trc_solid_soisno, trc_scv
   PUBLIC :: trc_ldew_rain, trc_ldew_snow, trc_sm_carry

   ! -- Element phase: per-step orchestration, called from CoLMDRIVER;
   !    these delegate to the reactive (methane) family ------------------
   PUBLIC :: tracer_resolve_step, tracer_lake_step, tracer_wetland_decomp
   PUBLIC :: tracer_soil_step, tracer_report

   ! -- Acc-flux: called from MOD_Vars_1DAccFluxes -----------------------
   PUBLIC :: tracer_flush_acc_fluxes, tracer_accumulate_fluxes

CONTAINS

   SUBROUTINE land_tracer_init (numpatch, maxsnl, nl_soil, init_month, lc_year, jdate, &
      casename, dir_restart, dir_landdata, ldew_rain, ldew_snow, wliq_soisno, &
      wice_soisno, wa, wdsrf, wetwat, scv, waterstorage)

      IMPLICIT NONE
      integer, intent(in) :: numpatch, maxsnl, nl_soil
      integer, intent(in) :: init_month
      integer, intent(in) :: lc_year
      integer, intent(in) :: jdate(3)
      character(len=*), intent(in) :: casename
      character(len=*), intent(in) :: dir_restart
      character(len=*), intent(in) :: dir_landdata
      real(r8), allocatable, intent(in), optional :: ldew_rain(:)
      real(r8), allocatable, intent(in), optional :: ldew_snow(:)
      real(r8), allocatable, intent(in), optional :: wliq_soisno(:,:)
      real(r8), allocatable, intent(in), optional :: wice_soisno(:,:)
      real(r8), allocatable, intent(in), optional :: wa(:)
      real(r8), allocatable, intent(in), optional :: wdsrf(:)
      real(r8), allocatable, intent(in), optional :: wetwat(:)
      real(r8), allocatable, intent(in), optional :: scv(:)
      real(r8), allocatable, intent(in), optional :: waterstorage(:)

      character(len=256) :: file_restart_trc
      character(len=14)  :: cdate_restart
      character(len=256) :: cyear_restart
      real(r8), allocatable :: tracer_dummy_patch(:)
      real(r8), allocatable :: tracer_dummy_soisno(:,:)
      logical :: have_patch_state
      logical :: have_waterstorage

      write(cyear_restart,'(i4.4)') lc_year
      write(cdate_restart,'(i4.4,"-",i3.3,"-",i5.5)') jdate(1), jdate(2), jdate(3)
      file_restart_trc = trim(dir_restart)//'/'//trim(cdate_restart)//'/'//trim(casename)// &
                         '_restart_'//trim(cdate_restart)//'_lc'//trim(cyear_restart)//'.nc'

      have_patch_state = present(ldew_rain) .and. present(ldew_snow) .and. &
         present(wliq_soisno) .and. present(wice_soisno) .and. present(wa) .and. &
         present(wdsrf) .and. present(wetwat) .and. present(scv)
      IF (have_patch_state) THEN
         have_patch_state = allocated(ldew_rain) .and. allocated(ldew_snow) .and. &
            allocated(wliq_soisno) .and. allocated(wice_soisno) .and. allocated(wa) .and. &
            allocated(wdsrf) .and. allocated(wetwat) .and. allocated(scv)
      ENDIF

      IF (have_patch_state) THEN
         have_waterstorage = DEF_USE_IRRIGATION .and. present(waterstorage)
         IF (have_waterstorage) have_waterstorage = allocated(waterstorage)
         IF (have_waterstorage) THEN
            CALL tracer_init_from_arrays (numpatch, maxsnl, nl_soil, &
               ldew_rain, ldew_snow, wliq_soisno, wice_soisno, &
               wa, wdsrf, wetwat, scv, file_restart_trc, waterstorage, &
               init_month=init_month, lc_year=lc_year, jdate=jdate, &
               casename=casename, dir_restart=dir_restart, dir_landdata=dir_landdata)
         ELSE
            CALL tracer_init_from_arrays (numpatch, maxsnl, nl_soil, &
               ldew_rain, ldew_snow, wliq_soisno, wice_soisno, &
               wa, wdsrf, wetwat, scv, file_restart_trc, init_month=init_month, &
               lc_year=lc_year, jdate=jdate, casename=casename, &
               dir_restart=dir_restart, dir_landdata=dir_landdata)
         ENDIF
      ELSE
         ! Vector restart I/O is collective over IO/worker groups. Non-worker
         ! ranks do not own patch water arrays, but they must still enter the
         ! TRACER init path so vector restart reads can complete.
         allocate(tracer_dummy_patch(0))
         allocate(tracer_dummy_soisno(maxsnl+1:nl_soil, 0))
         CALL tracer_init_from_arrays (0, maxsnl, nl_soil, &
            tracer_dummy_patch, tracer_dummy_patch, &
            tracer_dummy_soisno, tracer_dummy_soisno, &
            tracer_dummy_patch, tracer_dummy_patch, tracer_dummy_patch, &
            tracer_dummy_patch, file_restart_trc, init_month=init_month, &
            lc_year=lc_year, jdate=jdate, casename=casename, &
            dir_restart=dir_restart, dir_landdata=dir_landdata)
         deallocate(tracer_dummy_patch, tracer_dummy_soisno)
      ENDIF

   END SUBROUTINE land_tracer_init

   SUBROUTINE tracer_init_from_arrays (numpatch, maxsnl, nl_soil, &
      ldew_rain, ldew_snow, wliq_soisno, wice_soisno, &
      wa, wdsrf, wetwat, scv, file_restart, waterstorage, init_month, &
      lc_year, jdate, casename, dir_restart, dir_landdata)

      IMPLICIT NONE
      integer,  intent(in) :: numpatch, maxsnl, nl_soil
      real(r8), intent(in) :: ldew_rain(numpatch)
      real(r8), intent(in) :: ldew_snow(numpatch)
      real(r8), intent(in) :: wliq_soisno(maxsnl+1:nl_soil, numpatch)
      real(r8), intent(in) :: wice_soisno(maxsnl+1:nl_soil, numpatch)
      real(r8), intent(in) :: wa(numpatch)
      real(r8), intent(in) :: wdsrf(numpatch)
      real(r8), intent(in) :: wetwat(numpatch)
      real(r8), intent(in) :: scv(numpatch)
      character(len=*), intent(in), optional :: file_restart
      ! Optional: irrigation reservoir from MOD_Vars_TimeVariables.
      ! Passed by CoLM.F90 only under CROP+DEF_USE_IRRIGATION. Used for
      ! cold-start initialisation of trc_waterstorage; the restart path
      ! already handles persistence via read_land_tracer_restart.
      real(r8), intent(in), optional :: waterstorage(numpatch)
      integer,  intent(in), optional :: init_month
      integer, intent(in), optional :: lc_year
      integer, intent(in), optional :: jdate(3)
      character(len=*), intent(in), optional :: casename
      character(len=*), intent(in), optional :: dir_restart
      character(len=*), intent(in), optional :: dir_landdata
      logical :: found_restart, scv_missing, waterstorage_missing
      integer :: soil_init_month

      ! Defensive: if a previous land_tracer_init left state behind (e.g. a
      ! LULCC transition that rebuilt numpatch without an explicit
      ! land_tracer_final call), release it so the re-allocation below uses
      ! the new numpatch. Without this the allocate() call would trip
      ! Fortran's "already allocated" runtime check, OR the save-level
      ! snap_* arrays in MOD_Tracer_Conservation would keep the old
      ! numpatch and then misalign with the new tracer arrays.
      CALL tracer_lifecycle_land_final ()
      CALL deallocate_Tracer_Vars()
      CALL deallocate_tracer_conservation()
      CALL tracer_defs_init()
      CALL tracer_lifecycle_init()
      IF (present(lc_year) .and. present(jdate) .and. present(casename) .and. &
          present(dir_restart) .and. present(dir_landdata)) THEN
         CALL tracer_lifecycle_land_init (numpatch, lc_year, jdate, casename, dir_restart, dir_landdata)
      ENDIF
      IF (ntracers <= 0) RETURN
      CALL allocate_Tracer_Vars(numpatch, maxsnl, nl_soil)
      found_restart = .false.
      scv_missing   = .false.
      waterstorage_missing = .false.
      IF (present(file_restart)) THEN
         CALL read_land_tracer_restart(file_restart, maxsnl, nl_soil, &
            found_restart, scv_missing, waterstorage_missing)
      ENDIF
#ifdef USEMPI
      ! The control/master rank does not own vector restart blocks, so
      ! read_land_tracer_restart can only make a local decision there.
      ! Make restart usability global before any cold-start path that
      ! contains MPI collectives, including tracer soil initialisation.
      CALL mpi_allreduce(MPI_IN_PLACE, found_restart, 1, MPI_LOGICAL, &
         MPI_LAND, p_comm_glb, p_err)
      CALL mpi_allreduce(MPI_IN_PLACE, scv_missing, 1, MPI_LOGICAL, &
         MPI_LOR, p_comm_glb, p_err)
      CALL mpi_allreduce(MPI_IN_PLACE, waterstorage_missing, 1, MPI_LOGICAL, &
         MPI_LOR, p_comm_glb, p_err)
      scv_missing = scv_missing .and. found_restart
      waterstorage_missing = waterstorage_missing .and. found_restart
#endif
      IF (.not. found_restart) THEN
         soil_init_month = 1
         IF (present(init_month)) soil_init_month = init_month
         IF (present(waterstorage)) THEN
            CALL tracer_init_from_water(numpatch, maxsnl, nl_soil, &
               ldew_rain, ldew_snow, wliq_soisno, wice_soisno, &
               wa, wdsrf, wetwat, scv, waterstorage)
         ELSE
            CALL tracer_init_from_water(numpatch, maxsnl, nl_soil, &
               ldew_rain, ldew_snow, wliq_soisno, wice_soisno, &
               wa, wdsrf, wetwat, scv)
         ENDIF
         CALL tracer_soil_init_from_file(soil_init_month, numpatch, maxsnl, nl_soil, &
            wliq_soisno, wice_soisno)
      ELSE
         IF (waterstorage_missing .and. present(waterstorage)) THEN
            CALL tracer_init_waterstorage_from_ratio(numpatch, maxsnl, nl_soil, &
               ldew_rain, ldew_snow, wliq_soisno, wice_soisno, &
               wa, wdsrf, wetwat, scv, waterstorage)
         ENDIF
         IF (scv_missing) THEN
            ! Old-format restart: every other pool loaded from file, only
            ! trc_scv needs to be rebuilt from the hydrology scv.
            CALL tracer_init_scv_from_water(numpatch, maxsnl, nl_soil, &
               wliq_soisno, wice_soisno, scv)
         ENDIF
      ENDIF
      ! Provider-owned state has its own restart schema and must not be gated
      ! by the generic land-water descriptor.  Each provider probes its own
      ! mandatory fields and no-ops when the checkpoint has no provider state.
      IF (present(file_restart)) CALL tracer_lifecycle_land_read_restart(file_restart)
      IF (present(waterstorage)) THEN
         CALL tracer_enforce_solubility_from_water(numpatch, maxsnl, nl_soil, &
            ldew_rain, wliq_soisno, wa, wdsrf, wetwat, waterstorage)
      ELSE
         CALL tracer_enforce_solubility_from_water(numpatch, maxsnl, nl_soil, &
            ldew_rain, wliq_soisno, wa, wdsrf, wetwat)
      ENDIF
   END SUBROUTINE tracer_init_from_arrays

   SUBROUTINE tracer_resolve_step (istep_in, istep_local)

      IMPLICIT NONE
      integer, intent(in),  optional :: istep_in
      integer, intent(out)           :: istep_local

      istep_local = 1
      IF (present(istep_in)) istep_local = istep_in

   END SUBROUTINE tracer_resolve_step

   SUBROUTINE tracer_lake_step (istep_local, ipatch, idate, deltim_phy, isub, nsub)

      IMPLICIT NONE
      integer,  intent(in) :: istep_local
      integer,  intent(in) :: ipatch
      integer,  intent(in) :: idate(3)
      real(r8), intent(in) :: deltim_phy
      integer,  intent(in) :: isub
      integer,  intent(in) :: nsub

      CALL tracer_lifecycle_lake_step (istep_local, ipatch, idate, deltim_phy, isub, nsub)

   END SUBROUTINE tracer_lake_step

   SUBROUTINE tracer_wetland_decomp (ipatch, deltim)

      IMPLICIT NONE
      integer, intent(in) :: ipatch
      real(r8), intent(in) :: deltim

      CALL tracer_lifecycle_wetland_decomp (ipatch, deltim)

   END SUBROUTINE tracer_wetland_decomp

   SUBROUTINE tracer_soil_step (istep_local, ipatch, idate, deltim)

      IMPLICIT NONE
      integer,  intent(in) :: istep_local
      integer,  intent(in) :: ipatch
      integer,  intent(in) :: idate(3)
      real(r8), intent(in) :: deltim

      CALL tracer_lifecycle_soil_step (istep_local, ipatch, idate, deltim)

   END SUBROUTINE tracer_soil_step

   SUBROUTINE tracer_report ()

      IMPLICIT NONE

      CALL tracer_balance_report ()
      CALL tracer_lifecycle_land_report ()

   END SUBROUTINE tracer_report

   SUBROUTINE tracer_flush_acc_fluxes ()

      IMPLICIT NONE

      CALL tracer_lifecycle_land_flush_acc_fluxes ()

   END SUBROUTINE tracer_flush_acc_fluxes

   SUBROUTINE tracer_accumulate_fluxes ()

      IMPLICIT NONE

      CALL tracer_lifecycle_land_accumulate_fluxes ()

   END SUBROUTINE tracer_accumulate_fluxes

   SUBROUTINE land_tracer_final ()
      CALL tracer_lifecycle_land_final ()
      CALL deallocate_Tracer_Vars()
      CALL deallocate_tracer_conservation()
   END SUBROUTINE land_tracer_final

END MODULE MOD_Tracer_LandPhase
#endif
