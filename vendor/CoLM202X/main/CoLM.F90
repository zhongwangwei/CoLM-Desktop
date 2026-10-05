#include <define.h>

PROGRAM CoLM
!-----------------------------------------------------------------------------
!  Description:
!    This is the main program for the Common Land Model (CoLM)
!
!    Copyright © Yongjiu Dai Land Modeling Group at the School of Atmospheric Sciences
!    of the Sun Yat-sen University, Guangdong, CHINA.
!    All rights reserved.
!
!  Initial : Yongjiu Dai, 1998-2014
!  Revised : Hua Yuan, Shupeng Zhang, Nan Wei, Xingjie Lu, Zhongwang Wei, Yongjiu Dai
!            2014-2024
!-----------------------------------------------------------------------------

   USE MOD_Precision
   USE MOD_SPMD_Task
   USE MOD_Namelist
   USE MOD_Vars_Global
   USE MOD_Const_LC
   USE MOD_Const_PFT
   USE MOD_Const_Physical
   USE MOD_Vars_TimeInvariants
   USE MOD_Vars_TimeVariables
   USE MOD_Vars_1DForcing
   USE MOD_Vars_2DForcing
   USE MOD_Vars_1DFluxes
   USE MOD_Vars_1DAccFluxes
   USE MOD_Forcing
   USE MOD_Hist
   USE MOD_CheckEquilibrium
   USE MOD_TimeManager
   USE MOD_RangeCheck

   USE MOD_Block
   USE MOD_Pixel
   USE MOD_Mesh
   USE MOD_LandElm
#ifdef CATCHMENT
   USE MOD_LandHRU
#endif
   USE MOD_LandPatch
   USE MOD_LandUrban
   USE MOD_Urban_LAIReadin
   USE MOD_LandPFT
#if (defined UNSTRUCTURED || defined CATCHMENT)
   USE MOD_ElmVector
#endif
#ifdef CATCHMENT
   USE MOD_HRUVector
#endif
#if (defined CaMa_Flood)
   USE MOD_CaMa_colmCaMa, only: colm_CaMa_init, colm_CaMa_drv, colm_cama_write_restart, colm_cama_exit
   USE YOS_CMF_INPUT, only: CSETFILE
#endif
#ifdef SinglePoint
   USE MOD_SingleSrfdata
#endif
#if (defined CatchLateralFlow)
   USE MOD_Catch_BasinNetwork
   USE MOD_Catch_LateralFlow
#endif
#if (defined GridRiverLakeFlow)
   USE MOD_Grid_RiverLakeFlow
#endif

   USE MOD_Ozone, only: init_ozone_data, update_ozone_data

   USE MOD_SrfdataRestart
   USE MOD_LAIReadin

   USE MOD_NitrifData
   USE MOD_NdepData
   USE MOD_FireData
   USE MOD_LightningData

#ifdef CROP
   USE MOD_CropReadin
#endif

   USE MOD_Lulcc_Driver

   USE MOD_Hydro_SoilWater

#ifdef HYPERSPECTRAL
   ! SNICAR model
   USE MOD_SnowSnicar_HiRes, only: SnowAge_init, SnowOptics_init
#else
   USE MOD_SnowSnicar, only: SnowAge_init, SnowOptics_init
#endif
   USE MOD_Aerosol, only: AerosolDepInit, AerosolDepReadin

   USE MOD_ParameterOptimization
   USE MOD_Tracer_LandPhase, only: land_tracer_init, land_tracer_final
   USE MOD_Tracer_Lifecycle, only: tracer_lifecycle_reset
   USE MOD_Tracer_Defs, only: tracer_defs_final
#ifdef CaMa_Flood
   USE MOD_Tracer_Defs, only: ntracers, tracer_uses_land_water_transport
#endif
   USE MOD_Tracer_Forcing, only: tracer_forcing_init, read_tracer_forcing, &
                                 tracer_forcing_reset, tracer_forcing_final, tracer_forcing_read_restart, &
                                 tracer_forcing_lulcc_save, tracer_forcing_lulcc_restore

#ifdef DataAssimilation
   USE MOD_DA_Main
#endif

#ifdef HYPERSPECTRAL
   USE MOD_HighRes_Parameters
#endif

#ifdef USEMPI
   USE MOD_HistWriteBack
#endif

#ifdef EXTERNAL_LAKE
   USE MOD_Lake_Namelist
#endif

   IMPLICIT NONE

   character(len=256) :: nlfile
   character(len=256) :: casename
   character(len=256) :: dir_landdata
   character(len=256) :: dir_forcing
   character(len=256) :: dir_hist
   character(len=256) :: dir_restart
   character(len=256) :: fsrfdata

   real(r8) :: deltim       ! time step (seconds)
   integer  :: sdate(3)     ! calendar (year, julian day, seconds)
   integer  :: idate(3)     ! calendar (year, julian day, seconds)
   integer  :: edate(3)     ! calendar (year, julian day, seconds)
   integer  :: pdate(3)     ! calendar (year, julian day, seconds)
   integer  :: jdate(3)     ! calendar (year, julian day, seconds), year beginning style
   logical  :: greenwich    ! greenwich time

   logical :: doalb         ! true => start up the surface albedo calculation
   logical :: dolai         ! true => start up the time-varying vegetation parameter
   logical :: dosst         ! true => update sst/ice/snow

   integer :: Julian_1day_p, Julian_1day
   integer :: Julian_8day_p, Julian_8day
   integer :: s_year, s_month, s_day, s_seconds, s_julian
   integer :: e_year, e_month, e_day, e_seconds, e_julian
   integer :: p_year, p_month, p_day, p_seconds, p_julian
   integer :: lc_year, lai_year, restart_lc_year
   integer :: month, mday, year_p, month_p, mday_p, month_prev, mday_prev
   integer :: n_spinupcycle, i_spinupcycle, istep
#ifdef CaMa_Flood
   integer :: itrc_cama
#endif
   logical :: is_spinup
   logical :: lulcc_in_spinup = .false. ! LulccDriver ran during the current spinup cycle
   logical :: history_saved_raw
   logical :: tracer_loaded_restart
   character(len=256) :: tracer_restart_file

   type(timestamp) :: ststamp, itstamp, etstamp, ptstamp, time_prev

   integer*8 :: start_time, end_time, c_per_sec, time_used
!-----------------------------------------------------------------------

#ifdef USEMPI
#ifdef USESplitAI
      integer :: num_procs, my_rank, ierr, color, new_comm = MPI_COMM_NULL
      logical :: split_mpi_inited, split_mpi_owned = .false.
      CALL MPI_Initialized(split_mpi_inited, ierr)
      split_mpi_owned = .not. split_mpi_inited
      IF (split_mpi_owned) CALL MPI_Init(ierr) ! Initialize MPI
      CALL MPI_Comm_size(MPI_COMM_WORLD, num_procs, ierr) ! Get the total number of processes
      CALL MPI_Comm_rank(MPI_COMM_WORLD, my_rank, ierr) ! Get the rank of the current process
      color = 1 ! The pyroot process will be in its own communicator
      print*, 'before split I am process', my_rank, 'of', num_procs
      CALL MPI_Comm_split(MPI_COMM_WORLD, color, my_rank, new_comm, ierr) ! Split the communicator
      print*, 'after split I am process', my_rank, 'of', num_procs
      CALL MPI_Comm_size(new_comm, num_procs, ierr) ! Get the total number of processes
      CALL MPI_Comm_rank(new_comm, my_rank, ierr) ! Get the rank of the current process
      print*,num_procs,"for CoLM"
      CALL spmd_init (new_comm)
#else
      CALL spmd_init ()
#endif
#endif

      CALL getarg (1, nlfile)

      CALL read_namelist (nlfile)

#ifndef SinglePoint
      IF (trim(DEF_DS_precipitation_adjust_scheme) == 'III') THEN
         CALL CoLM_stop ('Precipitation scheme III is unavailable: this build has no Python MPI server ranks.')
      ENDIF
#endif

#ifdef EXTERNAL_LAKE
      CALL read_lake_namelist (nlfile)
#endif

#ifdef USEMPI
#ifndef FLAT_SPMD
      IF (DEF_HIST_WriteBack) THEN
         CALL spmd_assign_writeback ()
      ENDIF

      IF (p_is_writeback) THEN
         CALL hist_writeback_daemon ()
      ELSE
#else
      IF (DEF_HIST_WriteBack .and. p_is_master) THEN
         write(*,*) 'FLAT_SPMD ignores DEF_HIST_WriteBack; rank 0 participates in computation.'
      ENDIF
#endif
#endif

      IF (p_is_master) THEN
         CALL system_clock (start_time)
      ENDIF

      casename     = DEF_CASE_NAME
      dir_landdata = DEF_dir_landdata
      dir_forcing  = DEF_dir_forcing
      dir_hist     = DEF_dir_history
      dir_restart  = DEF_dir_restart

#ifdef SinglePoint
      fsrfdata = trim(dir_landdata) // '/srfdata.nc'
      IF (.not. DEF_URBAN_RUN) THEN
         CALL read_surface_data_single (fsrfdata, mksrfdata=.false.)
      ELSE
         CALL read_urban_surface_data_single (fsrfdata, mksrfdata=.false., mkrun=.true.)
      ENDIF
#endif

      deltim    = DEF_simulation_time%timestep
      greenwich = DEF_simulation_time%greenwich
      s_year    = DEF_simulation_time%start_year
      s_month   = DEF_simulation_time%start_month
      s_day     = DEF_simulation_time%start_day
      s_seconds = DEF_simulation_time%start_sec
      e_year    = DEF_simulation_time%end_year
      e_month   = DEF_simulation_time%end_month
      e_day     = DEF_simulation_time%end_day
      e_seconds = DEF_simulation_time%end_sec
      p_year    = DEF_simulation_time%spinup_year
      p_month   = DEF_simulation_time%spinup_month
      p_day     = DEF_simulation_time%spinup_day
      p_seconds = DEF_simulation_time%spinup_sec

      n_spinupcycle = DEF_simulation_time%spinup_repeat

      CALL initimetype(greenwich)
      CALL monthday2julian(s_year,s_month,s_day,s_julian)
      CALL monthday2julian(e_year,e_month,e_day,e_julian)
      CALL monthday2julian(p_year,p_month,p_day,p_julian)

      sdate(1) = s_year; sdate(2) = s_julian; sdate(3) = s_seconds
      edate(1) = e_year; edate(2) = e_julian; edate(3) = e_seconds
      pdate(1) = p_year; pdate(2) = p_julian; pdate(3) = p_seconds

      CALL Init_GlobalVars
      CALL Init_LC_Const
      CALL Init_PFT_Const

      IF (DEF_USE_LULCC) THEN
         lc_year = s_year
         DEF_LC_YEAR = lc_year
      ELSE
         lc_year = DEF_LC_YEAR
      ENDIF

#ifndef SinglePoint
      CALL pixel%load_from_file    (dir_landdata)
      CALL gblock%load_from_file   (dir_landdata)

      CALL mesh_load_from_file (dir_landdata, lc_year)

      CALL pixelset_load_from_file (dir_landdata, 'landelm'  , landelm  , numelm  , lc_year)

#ifdef CATCHMENT
      CALL pixelset_load_from_file (dir_landdata, 'landhru'  , landhru  , numhru  , lc_year)
#endif

      CALL pixelset_load_from_file (dir_landdata, 'landpatch', landpatch, numpatch, lc_year)

      IF (DEF_USE_PFT .or. DEF_USE_PC) THEN
         CALL pixelset_load_from_file (dir_landdata, 'landpft'  , landpft  , numpft  , lc_year)
         CALL map_patch_to_pft
      ENDIF

      IF (DEF_URBAN_RUN) THEN
         CALL pixelset_load_from_file (dir_landdata, 'landurban', landurban, numurban, lc_year)
         CALL map_patch_to_urban
      ENDIF

#if (defined UNSTRUCTURED || defined CATCHMENT)
      CALL elm_vector_init ()
#ifdef CATCHMENT
      CALL hru_vector_init ()
#endif
#endif

#ifdef CatchLateralFlow
      CALL build_basin_network ()
#endif

#ifdef GridRiverLakeFlow
      IF (DEF_USE_GridRiverLakeFlow) THEN
         CALL build_riverlake_network ()
         IF (DEF_Reservoir_Method > 0) CALL reservoir_init ()
      ENDIF
#endif
#endif

      CALL adj2end(sdate)
      CALL adj2end(edate)
      CALL adj2end(pdate)

      ststamp = sdate
      etstamp = edate
      ptstamp = pdate

      ! date in beginning style
      jdate = sdate
      CALL adj2begin(jdate)

      i_spinupcycle = 1
      n_spinupcycle = max(n_spinupcycle,1)
      is_spinup = (ststamp < ptstamp)

      ! ----------------------------------------------------------------------
      ! Read in the model time invariant constant data
      CALL allocate_TimeInvariants ()
      CALL READ_TimeInvariants (lc_year, casename, dir_restart)

      ! Read in the model time varying data (model state variables)
      CALL allocate_TimeVariables  ()
      CALL READ_TimeVariables (jdate, lc_year, casename, dir_restart)

      IF (DEF_USE_TRACER) THEN
      CALL land_tracer_init (numpatch, maxsnl, nl_soil, s_month, lc_year, jdate, &
         casename, dir_restart, dir_landdata, ldew_rain, ldew_snow, wliq_soisno, &
         wice_soisno, wa, wdsrf, wetwat, scv, waterstorage, &
         loaded_restart=tracer_loaded_restart, restart_file=tracer_restart_file)
      ENDIF

      ! Read in SNICAR optical and aging parameters
      IF (DEF_USE_SNICAR) THEN
         CALL SnowOptics_init( DEF_file_snowoptics ) ! SNICAR optical parameters
         CALL SnowAge_init( DEF_file_snowaging )     ! SNICAR aging   parameters
      ENDIF

#ifdef HYPERSPECTRAL
      ! ----------------------------------------------------------------------
      ! Read in FSDS fraction, leaf properties and surface albedo (single point)
      CALL flux_frac_init( )
      CALL leaf_property_init( rho_p, tau_p )
      CALL get_water_optical_properties( )
      CALL readin_urban_albedo( )
#endif

   !-----------------------
      doalb = .true.
      dolai = .true.
      dosst = .false.

      ! Initialize meteorological forcing data module
      CALL allocate_1D_Forcing ()
      CALL forcing_init (dir_forcing, deltim, ststamp, lc_year, etstamp)
      IF (DEF_USE_TRACER) THEN
      CALL tracer_forcing_init (gforc, numpatch)
      CALL tracer_forcing_read_restart (tracer_restart_file, tracer_loaded_restart)
      ENDIF
      CALL allocate_2D_Forcing (gforc)

      ! Initialize history data module
      CALL hist_init (dir_hist)
      CALL read_history_acc_restart (jdate, lc_year, casename, dir_restart)
      CALL allocate_1D_Fluxes ()

      CALL CheckEqb_init (n_spinupcycle, lc_year)

#if (defined CaMa_Flood)
#ifdef USEMPI
      CALL mpi_barrier (p_comm_glb, p_err)
#endif
      CALL colm_CaMa_init(jdate)
#ifdef USEMPI
      CALL mpi_barrier (p_comm_glb, p_err)
#endif
      IF (DEF_USE_TRACER) THEN
      IF (p_is_master .and. ntracers > 0) THEN
         IF (any([(tracer_uses_land_water_transport(itrc_cama), itrc_cama = 1, ntracers)])) &
            write(*,'(A)') 'WARNING: CaMa_Flood does not route land runoff tracers; they leave at the land boundary.'
      ENDIF
      ENDIF
#endif

      IF(DEF_USE_OZONEDATA)THEN
         CALL init_Ozone_data (sdate)
      ENDIF

      ! Initialize aerosol deposition forcing data
      IF (DEF_Aerosol_Readin) THEN
         CALL AerosolDepInit ()
      ENDIF

      IF (DEF_USE_BGC) THEN
      IF (DEF_USE_NITRIF) THEN
         CALL init_nitrif_data (ststamp)
      ENDIF

      ! Read the start year: sdate(1) is moved to the previous year by adj2end for a
      ! 00:00 Jan-1 start (formerly only TRACER builds read s_year).
      IF (DEF_NDEP_FREQUENCY==1)THEN ! Initial annual ndep data readin
            CALL init_ndep_data_annually (s_year)
      ELSEIF(DEF_NDEP_FREQUENCY==2)THEN ! Initial monthly ndep data readin
            CALL init_ndep_data_monthly (s_year,s_month)
      ELSE
         write(6,*) 'ERROR: DEF_NDEP_FREQUENCY should be only 1-2, Current is:', &
                     DEF_NDEP_FREQUENCY
         CALL CoLM_stop ()
      ENDIF

      IF (DEF_USE_FIRE) THEN
         CALL init_fire_data (s_year)
         CALL init_lightning_data (sdate)
      ENDIF
      ENDIF

#ifdef CROP
      CALL CROP_readin ()
#endif

#if (defined CatchLateralFlow)
      CALL lateral_flow_init (lc_year)
#endif
#ifdef GridRiverLakeFlow
      IF (DEF_USE_GridRiverLakeFlow) THEN
         CALL grid_riverlake_flow_init (s_year, is_spinup)
         CALL restore_river_history_acc_restart (jdate, casename, dir_restart)
      ELSE
         ! CoLMDRIVER still passes the (zero) flood fields to the land model.
         CALL allocate_flood_patch ()
      ENDIF
#endif

      CALL ParaOpt_init (jdate, lc_year)

#ifdef DataAssimilation
      ! initialize data assimilation
      CALL init_DA ()
#endif

      ! ======================================================================
      ! begin time stepping loop
      ! ======================================================================

      istep   = 1
      idate   = sdate
      itstamp = ststamp

      TIMELOOP : DO WHILE (itstamp < etstamp)

         CALL julian2monthday (jdate(1), jdate(2), month_p, mday_p)

         year_p = jdate(1)

         IF (p_is_master) THEN
            IF (itstamp < ptstamp) THEN
               write(*, 99) istep, jdate(1), month_p, mday_p, jdate(3), &
                  i_spinupcycle, n_spinupcycle
            ELSE
               write(*,100) istep, jdate(1), month_p, mday_p, jdate(3)
            ENDIF
         ENDIF

         Julian_1day_p = int(calendarday(jdate)-1)/1*1 + 1
         Julian_8day_p = int(calendarday(jdate)-1)/8*8 + 1

         ! Read in the meteorological forcing
         ! ----------------------------------------------------------------------
         CALL read_forcing (jdate, dir_forcing, is_spinup)
         IF (DEF_USE_TRACER) CALL read_tracer_forcing (jdate, dir_forcing)

         IF(DEF_USE_OZONEDATA)THEN
            CALL update_Ozone_data(itstamp, deltim)
         ENDIF

         IF (DEF_USE_BGC) THEN
         IF(DEF_USE_NITRIF) THEN
            time_prev = itstamp + int(-deltim)
            CALL julian2monthday(time_prev%year,time_prev%day,month_prev,mday_prev)
            if(month_p /= month_prev)then
               CALL update_nitrif_data (month_p)
            end if
         ENDIF
         IF(DEF_USE_FIRE)THEN
            CALL update_lightning_data (itstamp, deltim)
         ENDIF
         ENDIF

         ! Read in aerosol deposition forcing data
         IF (DEF_Aerosol_Readin) THEN
            CALL AerosolDepReadin (jdate)
         ENDIF

         ! Calendar for NEXT time step
         ! ----------------------------------------------------------------------
         CALL TICKTIME (deltim,idate)
         itstamp = itstamp + int(deltim)
         jdate = idate
         CALL adj2begin(jdate)

         CALL julian2monthday (jdate(1), jdate(2), month, mday)

         IF (DEF_USE_BGC) THEN

         IF (DEF_NDEP_FREQUENCY==1)THEN ! Read Annual Ndep data
            IF (jdate(1) /= year_p) THEN
               CALL update_ndep_data_annually (idate(1), iswrite = .true.)
            ENDIF
         ELSEIF(DEF_NDEP_FREQUENCY==2)THEN! Read Monthly Ndep data
            IF (jdate(1) /= year_p .or. month /= month_p) THEN
               CALL update_ndep_data_monthly (jdate(1), month, iswrite = .true.)
            ENDIF
         ELSE
            write(6,*) 'ERROR: DEF_NDEP_FREQUENCY should be only 1-2, Current is:',&
                        DEF_NDEP_FREQUENCY
            CALL CoLM_stop ()
         ENDIF

         IF(DEF_USE_FIRE)THEN
            IF (jdate(1) /= year_p) THEN
               CALL update_hdm_data (idate(1))
            ENDIF
         ENDIF
         ENDIF

         ! Call CoLM driver
         ! ----------------------------------------------------------------------
         IF (p_is_worker) THEN
      IF (DEF_USE_TRACER) THEN
            CALL CoLMDRIVER (idate,deltim,dolai,doalb,dosst,oroflag,istep)
      ELSE
            CALL CoLMDRIVER (idate,deltim,dolai,doalb,dosst,oroflag)
      ENDIF
         ENDIF

#if (defined CatchLateralFlow)
         CALL lateral_flow (idate(1), deltim)
#endif

#if (defined GridRiverLakeFlow)
         IF (DEF_USE_GridRiverLakeFlow .and. .not. is_spinup) THEN
            CALL grid_riverlake_flow (idate(1), deltim)
         ENDIF
#endif
#if (defined CaMa_Flood)
#ifdef USEMPI
         CALL mpi_barrier (p_comm_glb, p_err)
#endif
         IF(CSETFILE=='NONE')THEN
            CALL colm_CaMa_drv(idate(3),deltim,save_to_restart(idate,deltim,itstamp,ptstamp,etstamp))
         ELSE
            CALL colm_CaMa_drv(idate(3))
         ENDIF
#ifdef USEMPI
         CALL mpi_barrier (p_comm_glb, p_err)
#endif
#endif

         CALL CheckEquilibrium (idate, deltim, i_spinupcycle, is_spinup, dir_hist, casename)

#ifdef DataAssimilation
         CALL run_DA (idate, deltim, dolai, doalb, dosst, oroflag)
#endif

         ! Write out the model histroy file
         ! ----------------------------------------------------------------------
         CALL hist_out (idate, deltim, itstamp, etstamp, ptstamp, &
            dir_hist, casename, jdate, dir_restart, history_saved_raw)

         ! Close the parameter optimization year before LULCC: LULCC reallocates the
         ! fluxes (spval) and changes the patch layout, so calling it afterwards dropped the
         ! year's last step and closed the year on the new layout. Nothing between here and
         ! the old call site (LAI readin, restart output) touches its inputs.
         CALL ParameterOptimization (idate, deltim, is_spinup)

         ! DO land use and land cover change simulation
         ! ----------------------------------------------------------------------
         IF (DEF_USE_LULCC) THEN
         IF ( isendofyear(idate, deltim) .and. &
            ( jdate(1)>=2000 .or. (jdate(1)>1985 .and. MOD(jdate(1),5)==0) ) ) THEN

            ! Deallocate all Forcing and Fluxes variable of last year
            CALL deallocate_1D_Forcing
            CALL deallocate_1D_Fluxes

      IF (DEF_USE_TRACER) THEN
            CALL tracer_forcing_lulcc_save ()
            CALL tracer_forcing_final ()
      ENDIF
            CALL forcing_final ()
            CALL hist_final    ()

            ! Call LULCC driver
            CALL LulccDriver (casename, dir_landdata, dir_restart, jdate, greenwich)
            IF (is_spinup) lulcc_in_spinup = .true.
#ifdef GridRiverLakeFlow
            IF (DEF_USE_GridRiverLakeFlow) THEN
               CALL grid_riverlake_flow_lulcc ()
            ELSE
               CALL allocate_flood_patch ()
            ENDIF
#endif

            ! Allocate Forcing and Fluxes variable of next year
            CALL allocate_1D_Forcing
            CALL forcing_init (dir_forcing, deltim, itstamp, jdate(1), lulcc_call=.true.)
      IF (DEF_USE_TRACER) THEN
            CALL tracer_forcing_init (gforc, numpatch)
            CALL tracer_forcing_lulcc_restore ()
      ENDIF

            CALL hist_init (dir_hist, lulcc_call=.true.)
            CALL allocate_1D_Fluxes
         ENDIF
         ENDIF

         ! Get leaf area index
         ! ----------------------------------------------------------------------
#if (defined DYN_PHENOLOGY)
         ! Update once a day
         dolai = .false.
         Julian_1day = int(calendarday(jdate)-1)/1*1 + 1
         IF(Julian_1day /= Julian_1day_p)THEN
            dolai = .true.
         ENDIF
#else
         ! READ in Leaf area index and stem area index
         ! ----------------------------------------------------------------------
         ! NOTES: Should be caution for setting DEF_LAI_CHANGE_YEARLY to true in non-LULCC
         ! case, that means the LAI changes without consideration of land cover change.

         IF (DEF_LAI_CHANGE_YEARLY) THEN
            lai_year = jdate(1)
         ELSE
            lai_year = DEF_LC_YEAR
         ENDIF

         IF (DEF_LAI_MONTHLY) THEN
            IF (month /= month_p) THEN
               CALL LAI_readin (lai_year, month, dir_landdata)
               IF (DEF_URBAN_RUN) THEN
                  CALL UrbanLAI_readin(lai_year, month, dir_landdata)
               ENDIF
            ENDIF
         ELSE
            ! Update every 8 days (time interval of the MODIS LAI data)
            Julian_8day = int(calendarday(jdate)-1)/8*8 + 1
            IF (Julian_8day /= Julian_8day_p) THEN
               CALL LAI_readin (jdate(1), Julian_8day, dir_landdata)
            ENDIF
         ENDIF
#endif

         ! Write out the model state variables for restart run
         ! ----------------------------------------------------------------------
         IF (save_to_restart (idate, deltim, itstamp, ptstamp, etstamp)) THEN
            IF (.not.history_saved_raw) &
               CALL write_history_acc_restart (jdate, casename, dir_restart)
            IF (DEF_USE_LULCC) THEN
               IF (jdate(1) >= 2000) THEN
                  restart_lc_year = jdate(1)
                  CALL WRITE_TimeVariables (jdate, jdate(1), casename, dir_restart)
               ELSE
                  restart_lc_year = (jdate(1)/5)*5
                  CALL WRITE_TimeVariables (jdate, (jdate(1)/5)*5, casename, dir_restart)
               ENDIF
            ELSE
               restart_lc_year = lc_year
               CALL WRITE_TimeVariables (jdate, lc_year,  casename, dir_restart)
            ENDIF
            CALL mark_history_acc_restart (jdate, restart_lc_year, casename, dir_restart)
            CALL complete_history_acc_restart (jdate, casename, dir_restart)

#if (defined CaMa_Flood)
#ifdef USEMPI
            CALL mpi_barrier (p_comm_glb, p_err)
#endif
            IF (p_is_master) THEN
               CALL colm_cama_write_restart (jdate, lc_year,  casename, dir_restart)
            ENDIF
#ifdef USEMPI
            CALL mpi_barrier (p_comm_glb, p_err)
#endif
#endif
         ENDIF

         IF (DEF_USE_RangeCheck) THEN
         CALL check_TimeVariables ()
         ENDIF

#ifdef USEMPI
         CALL mpi_barrier (p_comm_glb, p_err)
#endif

         IF (DEF_USE_CoLMDEBUG) THEN
         IF (DEF_USE_VariablySaturatedFlow) THEN
            CALL print_VSF_iteration_stat_info ()
         ENDIF
         ENDIF

         IF (p_is_master) THEN
            CALL system_clock (end_time, count_rate = c_per_sec)
            time_used = (end_time - start_time) / c_per_sec
            IF (time_used >= 3600) THEN
               write(*,101) time_used/3600, mod(time_used,3600)/60, mod(time_used,60)
            ELSEIF (time_used >= 60) THEN
               write(*,102) time_used/60, mod(time_used,60)
            ELSE
               write(*,103) time_used
            ENDIF
         ENDIF

         IF (is_spinup) THEN
            IF (ptstamp <= itstamp) THEN
               IF (i_spinupcycle < n_spinupcycle) THEN
                  i_spinupcycle = i_spinupcycle + 1
                  idate   = sdate
                  jdate   = sdate
                  itstamp = ststamp
                  CALL adj2begin(jdate)
                  ! FIX upstream-bugs #50: the rewind used to reset clock and forcing only, so
                  ! after a LULCC year end the next cycle ran the start year on the new year's
                  ! land cover and patch layout. Change the land cover back to the start year
                  ! the way the year-end LULCC does; there is no transfer trace from a later
                  ! year back to the start year, so this change always uses SAT.
                  IF (lulcc_in_spinup) THEN
                     CALL deallocate_1D_Forcing
                     CALL deallocate_1D_Fluxes
      IF (DEF_USE_TRACER) THEN
                     CALL tracer_forcing_lulcc_save ()
                     CALL tracer_forcing_final ()
      ENDIF
                     CALL forcing_final ()
                     CALL hist_final    ()

                     CALL LulccDriver (casename, dir_landdata, dir_restart, jdate, greenwich, &
                        rewind = .true.)
#ifdef GridRiverLakeFlow
                     IF (DEF_USE_GridRiverLakeFlow) THEN
                        CALL grid_riverlake_flow_lulcc ()
                     ELSE
                        CALL allocate_flood_patch ()
                     ENDIF
#endif

                     CALL allocate_1D_Forcing
                     CALL forcing_init (dir_forcing, deltim, itstamp, jdate(1), lulcc_call=.true.)
      IF (DEF_USE_TRACER) THEN
                     CALL tracer_forcing_init (gforc, numpatch)
                     CALL tracer_forcing_lulcc_restore ()
      ENDIF

                     CALL hist_init (dir_hist, lulcc_call=.true.)
                     CALL allocate_1D_Fluxes
                     lulcc_in_spinup = .false.
                  ENDIF
                  CALL forcing_reset ()
                  IF (DEF_USE_TRACER) CALL tracer_forcing_reset ()
               ELSE
                  is_spinup = .false.
               ENDIF
            ENDIF
         ENDIF

         IF (p_is_master) THEN
            IF (DEF_CheckEquilibrium .and. (len_trim(mesg_equilibrium) > 0)) THEN
               write(*,'(/,A)') trim(mesg_equilibrium)
            ENDIF
         ENDIF

         istep = istep + 1

      ENDDO TIMELOOP

      IF (DEF_USE_TRACER) THEN
         CALL tracer_forcing_final ()
         CALL land_tracer_final ()
      ENDIF

      CALL deallocate_TimeInvariants ()
      CALL deallocate_TimeVariables  ()
      CALL deallocate_1D_Forcing     ()
      CALL deallocate_1D_Fluxes      ()
      CALL mesh_free_mem             ()

#if (defined CatchLateralFlow)
      CALL lateral_flow_final ()
#endif
      CALL ParaOpt_final ()
#ifdef DataAssimilation
      CALL end_DA()
#endif

#if (defined GridRiverLakeFlow)
      IF (DEF_USE_GridRiverLakeFlow) CALL grid_riverlake_flow_final ()
#endif

      IF (DEF_USE_TRACER) THEN
         CALL tracer_lifecycle_reset ()
         CALL tracer_defs_final ()
      ENDIF

      CALL forcing_final ()
      CALL hist_final    ()
      CALL CheckEqb_final()

#ifdef SinglePoint
      CALL single_srfdata_final ()
#endif

#ifdef USEMPI
      CALL mpi_barrier (p_comm_glb, p_err)
#endif

#if (defined CaMa_Flood)
      CALL colm_cama_exit ! finalize CaMa-Flood
#endif

      IF (p_is_master) THEN
         write(*,'(/,A25)') 'CoLM Execution Completed.'
      ENDIF

      99  format(/, 'TIMESTEP = ', I0, ' | DATE = ', I4.4, '-', I2.2, '-', I2.2, '-', I5.5, &
          ' Spinup (cycle ', I0, ' of ', I0, ')')
      100 format(/, 'TIMESTEP = ', I0, ' | DATE = ', I4.4, '-', I2.2, '-', I2.2, '-', I5.5)
      101 format(/, 'Time elapsed : ', I4, ' hours', I3, ' minutes', I3, ' seconds.')
      102 format(/, 'Time elapsed : ', I3, ' minutes', I3, ' seconds.')
      103 format(/, 'Time elapsed : ', I3, ' seconds.')

#ifdef USEMPI
#ifndef FLAT_SPMD
      ENDIF

      IF (DEF_HIST_WriteBack) THEN
         CALL hist_writeback_exit ()
      ENDIF
#endif

      CALL spmd_exit
#ifdef USESplitAI
      IF (new_comm /= MPI_COMM_NULL) CALL MPI_Comm_free(new_comm, ierr)
      IF (split_mpi_owned) CALL MPI_Finalize(ierr)
#endif
#endif

END PROGRAM CoLM
! ---------- EOP ------------
