#include <define.h>

#ifdef TRACER
MODULE MOD_Tracer_Forcing

   USE MOD_Precision
   USE MOD_Namelist, only: DEF_forcing, DEF_Forcing_Interp_Method
   USE MOD_SPMD_Task
   USE MOD_Grid
   USE MOD_DataType
   USE MOD_NetCDFBlock, only: ncio_read_block_time
   USE MOD_SpatialMapping
   USE MOD_LandPatch
   USE MOD_TimeManager
   USE MOD_UserSpecifiedForcing, only: metfilename
   USE MOD_Tracer_Defs, only: ntracers, tracers, tracer_precip_default_ratio, &
      tracer_vapor_default_ratio, &
      tracer_is_isotope, delta_to_R, trc_tiny, trc_delta_sanity_max, &
      tracer_uses_land_water_transport, tracer_lower
   USE MOD_Tracer_Frac, only: tracer_fractionation_active
   USE MOD_Tracer_Vars, only: trc_runtime_forced
   USE MOD_Tracer_Isotope_Registrations, only: ensure_isotope_physics_registered
   USE MOD_Tracer_ForcingInput, only: tracer_forcing_spec_type, tracer_forcing_input_load, &
      tracer_forcing_input_get, tracer_forcing_input_find, tracer_forcing_input_final

   IMPLICIT NONE
   SAVE

   integer, parameter :: STREAM_PRECIP = 1
   integer, parameter :: STREAM_VAPOR  = 2
   integer, parameter :: STREAM_TOTAL_PRECIP = 3
   integer, parameter :: STREAM_TOTAL_VAPOR  = 4

   integer, parameter :: DECODE_INVALID  = 0
   integer, parameter :: DECODE_NEAR_DRY = 1
   integer, parameter :: DECODE_VALID    = 2

   integer, parameter :: MODE_VALUE = 1
   integer, parameter :: MODE_DELTA = 2
   integer, parameter :: MODE_HEAVY_OVER_TOTAL = 3
   integer, parameter :: MODE_NORMALIZED_OVER_TOTAL = 4

   ! Normalized isotope-over-total precipitation ratios are numerically
   ! unreliable for near-dry IsoGSM records. 1e-7 kg m-2 s-1 is only
   ! 0.00216 mm per 6-hour forcing interval, so filtering it has negligible
   ! water-mass impact while preventing ratio spikes from becoming forcings.
   real(r8), parameter :: trc_forc_min_prcp = 1.0e-7_r8
   real(r8), parameter :: trc_forc_min_q    = 1.0e-12_r8
   real(r8), parameter :: trc_forc_max_abs  = 1.0e10_r8

   logical :: trc_runtime_forcing_enabled = .false.
   integer :: trc_forcing_log_count = 0
   integer :: n_trc_forc_vars = 0
   integer :: idx_total_precip = 0
   integer :: idx_total_vapor = 0

   integer, allocatable :: trc_var_stream(:)
   integer, allocatable :: trc_var_itrc(:)
   integer, allocatable :: trc_var_mode(:)
   integer, allocatable :: trc_var_total(:)
   integer, allocatable :: trc_var_dtime(:)
   integer, allocatable :: trc_var_offset(:)
   character(len=256), allocatable :: trc_var_fprefix(:)
   character(len=256), allocatable :: trc_var_vname(:)
   character(len=256), allocatable :: trc_var_tintalgo(:)
   character(len=256), allocatable :: trc_var_timelog(:)
   type(timestamp), allocatable :: trc_tstamp_LB(:)
   type(timestamp), allocatable :: trc_tstamp_UB(:)

   type(spatial_mapping_type) :: trc_mg2p_forc
   type(grid_type) :: trc_gforc
   type(block_data_real8_2d) :: trc_metdata
   type(block_data_real8_2d), allocatable :: trc_forcn(:)
   type(block_data_real8_2d), allocatable :: trc_forcn_LB(:)
   type(block_data_real8_2d), allocatable :: trc_forcn_UB(:)
   real(r8), allocatable :: trc_forc_patch(:,:)

   real(r8), allocatable :: trc_forc_precip_value(:,:)
   real(r8), allocatable :: trc_forc_vapor_value(:,:)
   logical,  allocatable :: trc_forc_has_precip(:,:)
   logical,  allocatable :: trc_forc_has_vapor(:,:)
   integer,  allocatable :: trc_forc_precip_status(:,:)
   integer,  allocatable :: trc_forc_vapor_status(:,:)
   logical,  allocatable :: trc_forc_precip_warned_dry(:)
   logical,  allocatable :: trc_forc_precip_warned_invalid(:)
   logical,  allocatable :: trc_forc_vapor_warned_dry(:)
   logical,  allocatable :: trc_forc_vapor_warned_invalid(:)

   PUBLIC :: tracer_forcing_init
   PUBLIC :: read_tracer_forcing
   PUBLIC :: tracer_forcing_reset
   PUBLIC :: tracer_forcing_final
   PUBLIC :: tracer_forcing_precip_value
   PUBLIC :: tracer_forcing_vapor_value
   PUBLIC :: tracer_forcing_precip_ratio
   PUBLIC :: tracer_forcing_vapor_ratio
   PUBLIC :: tracer_forcing_has_precip
   PUBLIC :: tracer_forcing_has_vapor
   PUBLIC :: tracer_forcing_ratio_to_delta

CONTAINS

   SUBROUTINE tracer_forcing_init (gforc, numpatch)
      IMPLICIT NONE
      type(grid_type), intent(in) :: gforc
      integer, intent(in) :: numpatch
      integer :: iv

      CALL tracer_forcing_final()
      IF (ntracers <= 0) RETURN

      CALL tracer_forcing_configure()
      CALL tracer_forcing_allocate_state(numpatch)
      IF (.not. trc_runtime_forcing_enabled) RETURN
      IF (trim(DEF_forcing%dataset) == 'POINT') THEN
         IF (p_is_master) THEN
            WRITE(*,'(A)') 'ERROR tracer runtime forcing does not support DEF_forcing%dataset=POINT.'
            WRITE(*,'(A)') '      Disable tracer forcing variables or use gridded forcing for tracer fields.'
         ENDIF
         CALL CoLM_stop()
      ENDIF

      CALL trc_gforc%define_by_copy(gforc)
      IF (trim(DEF_Forcing_Interp_Method) == 'arealweight') THEN
         CALL trc_mg2p_forc%build_arealweighted(trc_gforc, landpatch)
      ELSEIF (trim(DEF_Forcing_Interp_Method) == 'bilinear') THEN
         CALL trc_mg2p_forc%build_bilinear(trc_gforc, landpatch)
      ELSE
         IF (p_is_master) WRITE(*,'(2A)') 'ERROR tracer forcing unsupported interp method: ', &
            trim(DEF_Forcing_Interp_Method)
         CALL CoLM_stop()
      ENDIF

      allocate(trc_forcn(n_trc_forc_vars))
      IF (p_is_io) THEN
         allocate(trc_forcn_LB(n_trc_forc_vars))
         allocate(trc_forcn_UB(n_trc_forc_vars))
         DO iv = 1, n_trc_forc_vars
            CALL allocate_block_data(trc_gforc, trc_forcn(iv))
            CALL allocate_block_data(trc_gforc, trc_forcn_LB(iv))
            CALL allocate_block_data(trc_gforc, trc_forcn_UB(iv))
         ENDDO
         CALL allocate_block_data(trc_gforc, trc_metdata)
      ENDIF

      allocate(trc_forc_patch(numpatch, n_trc_forc_vars))
      trc_forc_patch(:,:) = 0._r8

      IF (p_is_master) THEN
         WRITE(*,'(A,I0,A)') 'Tracer runtime forcing enabled: ', n_trc_forc_vars, &
            ' variable(s) from main forcing namelist.'
      ENDIF
   END SUBROUTINE tracer_forcing_init

   SUBROUTINE read_tracer_forcing (idate, dir_forcing)
      IMPLICIT NONE
      integer, intent(in) :: idate(3)
      character(len=*), intent(in) :: dir_forcing
      integer :: iv

      CALL tracer_forcing_prepare_step()
      IF (.not. trc_runtime_forcing_enabled) RETURN

      CALL tracer_forcing_read_LBUB(idate, dir_forcing)

      IF (p_is_io) THEN
         DO iv = 1, n_trc_forc_vars
            CALL tracer_forcing_interpolate_var(idate, iv)
         ENDDO
      ENDIF

      DO iv = 1, n_trc_forc_vars
         CALL trc_mg2p_forc%grid2pset(trc_forcn(iv), trc_forc_patch(:, iv))
      ENDDO

      CALL tracer_forcing_update_values()
      CALL tracer_forcing_log_ranges(idate)
   END SUBROUTINE read_tracer_forcing

   SUBROUTINE tracer_forcing_reset ()
      IMPLICIT NONE

      IF (allocated(trc_tstamp_LB)) trc_tstamp_LB(:) = timestamp(-1, -1, -1)
      IF (allocated(trc_tstamp_UB)) trc_tstamp_UB(:) = timestamp(-1, -1, -1)
      trc_forcing_log_count = 0
   END SUBROUTINE tracer_forcing_reset

   SUBROUTINE tracer_forcing_final ()
      IMPLICIT NONE

      CALL trc_mg2p_forc%forc_free_mem()
      IF (allocated(trc_forcn)) deallocate(trc_forcn)
      IF (allocated(trc_forcn_LB)) deallocate(trc_forcn_LB)
      IF (allocated(trc_forcn_UB)) deallocate(trc_forcn_UB)
      IF (allocated(trc_forc_patch)) deallocate(trc_forc_patch)
      CALL tracer_forcing_deallocate_config()
      CALL tracer_forcing_deallocate_state()
      CALL tracer_forcing_input_final()

      IF (allocated(trc_runtime_forced)) trc_runtime_forced(:) = .false.
      trc_runtime_forcing_enabled = .false.
      trc_forcing_log_count = 0
      n_trc_forc_vars = 0
      idx_total_precip = 0
      idx_total_vapor = 0
   END SUBROUTINE tracer_forcing_final

   SUBROUTINE tracer_forcing_configure ()
      IMPLICIT NONE
      integer :: itrc, mode, dtime, offset, nold, k
      character(len=256) :: fprefix, vname, tintalgo, token
      logical, allocatable :: has_precip(:), has_vapor(:)
      type(tracer_forcing_spec_type) :: spec

      ! Load each tracer's OWN forcing specs from its parameter file
      ! (&nl_colm_tracer_forcing). This replaces the former global
      ! DEF_forcing%tracer_* / legacy DEF_forcing%precipitation_O18_* path.
      CALL tracer_forcing_input_load()
      CALL tracer_forcing_deallocate_config()
      allocate(trc_var_stream(2*ntracers + 6))
      allocate(trc_var_itrc(2*ntracers + 6))
      allocate(trc_var_mode(2*ntracers + 6))
      allocate(trc_var_total(2*ntracers + 6))
      allocate(trc_var_dtime(2*ntracers + 6))
      allocate(trc_var_offset(2*ntracers + 6))
      allocate(trc_var_fprefix(2*ntracers + 6))
      allocate(trc_var_vname(2*ntracers + 6))
      allocate(trc_var_tintalgo(2*ntracers + 6))
      allocate(trc_var_timelog(2*ntracers + 6))
      allocate(trc_tstamp_LB(2*ntracers + 6))
      allocate(trc_tstamp_UB(2*ntracers + 6))
      allocate(has_precip(ntracers), has_vapor(ntracers))

      n_trc_forc_vars = 0
      idx_total_precip = 0
      idx_total_vapor = 0
      has_precip(:) = .false.
      has_vapor(:) = .false.
      trc_tstamp_LB(:) = timestamp(-1, -1, -1)
      trc_tstamp_UB(:) = timestamp(-1, -1, -1)
      trc_var_timelog(:) = 'instant'

      ! Per-species forcing: each tracer declares its own precip/vapor inputs
      ! in &nl_colm_tracer_forcing (loaded above by MOD_Tracer_ForcingInput).
      ! These two find() calls are the ONLY consumers of a forcing role in the
      ! whole model, and this loop skips provider-owned species. Anything else
      ! is refused by tracer_forcing_role_valid rather than stored and ignored.
      DO itrc = 1, ntracers
         IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
         k = tracer_forcing_input_find(itrc, 'precip')
         IF (k > 0) THEN
            spec = tracer_forcing_input_get(itrc, k)
            mode = tracer_forcing_parse_mode(spec%input_mode)
            nold = n_trc_forc_vars
            CALL tracer_forcing_add_var(STREAM_PRECIP, itrc, mode, &
               spec%fprefix, spec%vname, spec%tintalgo, spec%dtime, spec%offset)
            has_precip(itrc) = n_trc_forc_vars > nold
         ENDIF

         k = tracer_forcing_input_find(itrc, 'vapor')
         IF (k > 0) THEN
            spec = tracer_forcing_input_get(itrc, k)
            mode = tracer_forcing_parse_mode(spec%input_mode)
            nold = n_trc_forc_vars
            CALL tracer_forcing_add_var(STREAM_VAPOR, itrc, mode, &
               spec%fprefix, spec%vname, spec%tintalgo, spec%dtime, spec%offset)
            has_vapor(itrc) = n_trc_forc_vars > nold
         ENDIF

         IF (tracer_fractionation_active(itrc) .and. .not. has_vapor(itrc)) THEN
            IF (p_is_master) THEN
               WRITE(*,'(A,I0,3A)') &
                  'ERROR tracer_forcing_configure: fractionating tracer ', itrc, &
                  ' (', trim(tracers(itrc)%name), &
                  ') must define vapor forcing with role=''vapor'' in &nl_colm_tracer_forcing.'
            ENDIF
            CALL CoLM_stop()
         ENDIF
      ENDDO

      trc_runtime_forcing_enabled = n_trc_forc_vars > 0
      IF (allocated(trc_runtime_forced)) THEN
         trc_runtime_forced(:) = has_precip(:) .or. has_vapor(:)
      ENDIF
      deallocate(has_precip, has_vapor)
   END SUBROUTINE tracer_forcing_configure

   SUBROUTINE tracer_forcing_deallocate_config ()
      IMPLICIT NONE

      IF (allocated(trc_var_stream)) deallocate(trc_var_stream)
      IF (allocated(trc_var_itrc)) deallocate(trc_var_itrc)
      IF (allocated(trc_var_mode)) deallocate(trc_var_mode)
      IF (allocated(trc_var_total)) deallocate(trc_var_total)
      IF (allocated(trc_var_dtime)) deallocate(trc_var_dtime)
      IF (allocated(trc_var_offset)) deallocate(trc_var_offset)
      IF (allocated(trc_var_fprefix)) deallocate(trc_var_fprefix)
      IF (allocated(trc_var_vname)) deallocate(trc_var_vname)
      IF (allocated(trc_var_tintalgo)) deallocate(trc_var_tintalgo)
      IF (allocated(trc_var_timelog)) deallocate(trc_var_timelog)
      IF (allocated(trc_tstamp_LB)) deallocate(trc_tstamp_LB)
      IF (allocated(trc_tstamp_UB)) deallocate(trc_tstamp_UB)
   END SUBROUTINE tracer_forcing_deallocate_config

   SUBROUTINE tracer_forcing_add_var (stream, itrc, mode, fprefix, vname, tintalgo, dtime, offset)
      IMPLICIT NONE
      integer, intent(in) :: stream, itrc, mode, dtime, offset
      character(len=*), intent(in) :: fprefix, vname, tintalgo
      integer :: total_idx

      IF (dtime <= 0) THEN
         IF (p_is_master) WRITE(*,'(A,I0,A,I0,A)') &
            'ERROR tracer_forcing_add_var: forcing dtime for tracer ', itrc, ' must be > 0, got ', dtime, '.'
         CALL CoLM_stop()
      ENDIF
      IF (.not. tracer_forcing_token_present(fprefix)) RETURN
      IF (.not. tracer_forcing_token_present(vname)) RETURN

      total_idx = 0
      IF (mode == MODE_HEAVY_OVER_TOTAL .or. mode == MODE_NORMALIZED_OVER_TOTAL) THEN
         total_idx = tracer_forcing_ensure_total(stream)
         IF (dtime /= trc_var_dtime(total_idx) .or. offset /= trc_var_offset(total_idx) .or. &
             trim(tracer_lower(tintalgo)) /= trim(trc_var_tintalgo(total_idx))) THEN
            IF (p_is_master) WRITE(*,'(A,I0,A)') &
               'ERROR tracer_forcing_add_var: raw/total forcing temporal config mismatch for tracer ', &
               itrc, '.'
            CALL CoLM_stop()
         ENDIF
      ENDIF

      n_trc_forc_vars = n_trc_forc_vars + 1
      trc_var_stream(n_trc_forc_vars) = stream
      trc_var_itrc(n_trc_forc_vars) = itrc
      trc_var_mode(n_trc_forc_vars) = mode
      trc_var_total(n_trc_forc_vars) = total_idx
      trc_var_fprefix(n_trc_forc_vars) = trim(fprefix)
      trc_var_vname(n_trc_forc_vars) = trim(vname)
      trc_var_tintalgo(n_trc_forc_vars) = trim(tintalgo)
      trc_var_timelog(n_trc_forc_vars) = 'instant'
      IF (total_idx > 0) trc_var_timelog(n_trc_forc_vars) = trc_var_timelog(total_idx)
      trc_var_dtime(n_trc_forc_vars) = dtime
      trc_var_offset(n_trc_forc_vars) = offset
   END SUBROUTINE tracer_forcing_add_var

   integer FUNCTION tracer_forcing_ensure_total (stream)
      IMPLICIT NONE
      integer, intent(in) :: stream

      IF (stream == STREAM_PRECIP) THEN
         IF (idx_total_precip == 0) THEN
            IF (DEF_forcing%dtime(4) <= 0) THEN
               IF (p_is_master) WRITE(*,'(A,I0,A)') &
                  'ERROR tracer_forcing_ensure_total: total precipitation dtime must be > 0, got ', &
                  DEF_forcing%dtime(4), '.'
               CALL CoLM_stop()
            ENDIF
            n_trc_forc_vars = n_trc_forc_vars + 1
            idx_total_precip = n_trc_forc_vars
            trc_var_stream(idx_total_precip) = STREAM_TOTAL_PRECIP
            trc_var_itrc(idx_total_precip) = 0
            trc_var_mode(idx_total_precip) = MODE_VALUE
            trc_var_total(idx_total_precip) = 0
            trc_var_fprefix(idx_total_precip) = trim(DEF_forcing%fprefix(4))
            trc_var_vname(idx_total_precip) = trim(DEF_forcing%vname(4))
            trc_var_tintalgo(idx_total_precip) = trim(tracer_lower(DEF_forcing%tintalgo(4)))
            trc_var_timelog(idx_total_precip) = trim(tracer_lower(DEF_forcing%timelog(4)))
            trc_var_dtime(idx_total_precip) = DEF_forcing%dtime(4)
            trc_var_offset(idx_total_precip) = DEF_forcing%offset(4)
         ENDIF
         tracer_forcing_ensure_total = idx_total_precip
      ELSE
         IF (idx_total_vapor == 0) THEN
            IF (DEF_forcing%dtime(2) <= 0) THEN
               IF (p_is_master) WRITE(*,'(A,I0,A)') &
                  'ERROR tracer_forcing_ensure_total: total vapor dtime must be > 0, got ', &
                  DEF_forcing%dtime(2), '.'
               CALL CoLM_stop()
            ENDIF
            n_trc_forc_vars = n_trc_forc_vars + 1
            idx_total_vapor = n_trc_forc_vars
            trc_var_stream(idx_total_vapor) = STREAM_TOTAL_VAPOR
            trc_var_itrc(idx_total_vapor) = 0
            trc_var_mode(idx_total_vapor) = MODE_VALUE
            trc_var_total(idx_total_vapor) = 0
            trc_var_fprefix(idx_total_vapor) = trim(DEF_forcing%fprefix(2))
            trc_var_vname(idx_total_vapor) = trim(DEF_forcing%vname(2))
            trc_var_tintalgo(idx_total_vapor) = trim(tracer_lower(DEF_forcing%tintalgo(2)))
            trc_var_timelog(idx_total_vapor) = trim(tracer_lower(DEF_forcing%timelog(2)))
            trc_var_dtime(idx_total_vapor) = DEF_forcing%dtime(2)
            trc_var_offset(idx_total_vapor) = DEF_forcing%offset(2)
         ENDIF
         tracer_forcing_ensure_total = idx_total_vapor
      ENDIF
   END FUNCTION tracer_forcing_ensure_total

   SUBROUTINE tracer_forcing_allocate_state (numpatch)
      IMPLICIT NONE
      integer, intent(in) :: numpatch
      integer :: itrc

      CALL tracer_forcing_deallocate_state()
      allocate(trc_forc_precip_value(ntracers, numpatch))
      allocate(trc_forc_vapor_value(ntracers, numpatch))
      allocate(trc_forc_has_precip(ntracers, numpatch))
      allocate(trc_forc_has_vapor(ntracers, numpatch))
      allocate(trc_forc_precip_status(ntracers, numpatch))
      allocate(trc_forc_vapor_status(ntracers, numpatch))
      allocate(trc_forc_precip_warned_dry(ntracers))
      allocate(trc_forc_precip_warned_invalid(ntracers))
      allocate(trc_forc_vapor_warned_dry(ntracers))
      allocate(trc_forc_vapor_warned_invalid(ntracers))
      trc_forc_precip_warned_dry(:) = .false.
      trc_forc_precip_warned_invalid(:) = .false.
      trc_forc_vapor_warned_dry(:) = .false.
      trc_forc_vapor_warned_invalid(:) = .false.
      DO itrc = 1, ntracers
         trc_forc_precip_value(itrc, :) = tracer_precip_default_ratio(itrc)
         trc_forc_vapor_value(itrc, :) = tracer_vapor_default_ratio(itrc)
      ENDDO
      CALL tracer_forcing_prepare_step()
   END SUBROUTINE tracer_forcing_allocate_state

   SUBROUTINE tracer_forcing_deallocate_state ()
      IMPLICIT NONE

      IF (allocated(trc_forc_precip_value)) deallocate(trc_forc_precip_value)
      IF (allocated(trc_forc_vapor_value)) deallocate(trc_forc_vapor_value)
      IF (allocated(trc_forc_has_precip)) deallocate(trc_forc_has_precip)
      IF (allocated(trc_forc_has_vapor)) deallocate(trc_forc_has_vapor)
      IF (allocated(trc_forc_precip_status)) deallocate(trc_forc_precip_status)
      IF (allocated(trc_forc_vapor_status)) deallocate(trc_forc_vapor_status)
      IF (allocated(trc_forc_precip_warned_dry)) deallocate(trc_forc_precip_warned_dry)
      IF (allocated(trc_forc_precip_warned_invalid)) deallocate(trc_forc_precip_warned_invalid)
      IF (allocated(trc_forc_vapor_warned_dry)) deallocate(trc_forc_vapor_warned_dry)
      IF (allocated(trc_forc_vapor_warned_invalid)) deallocate(trc_forc_vapor_warned_invalid)
   END SUBROUTINE tracer_forcing_deallocate_state

   SUBROUTINE tracer_forcing_prepare_step ()
      IMPLICIT NONE

      IF (.not. allocated(trc_forc_precip_value)) RETURN

      trc_forc_has_precip(:,:) = .false.
      trc_forc_has_vapor(:,:) = .false.
      trc_forc_precip_status(:,:) = DECODE_INVALID
      trc_forc_vapor_status(:,:) = DECODE_INVALID
   END SUBROUTINE tracer_forcing_prepare_step

   SUBROUTINE tracer_forcing_read_LBUB (idate, dir_forcing)
      IMPLICIT NONE
      integer, intent(in) :: idate(3)
      character(len=*), intent(in) :: dir_forcing
      integer :: iv, year, month, day, time_i
      type(timestamp) :: mtstamp
      character(len=256) :: filename

      mtstamp = idate

      DO iv = 1, n_trc_forc_vars
         IF (.not.(trc_tstamp_LB(iv) == 'NULL') .and. .not.(trc_tstamp_UB(iv) == 'NULL') .and. &
             trc_tstamp_LB(iv) <= mtstamp .and. mtstamp < trc_tstamp_UB(iv)) CYCLE

         IF (trc_tstamp_LB(iv) == 'NULL') THEN
            CALL tracer_forcing_setstamp_LB(mtstamp, iv, year, month, day, time_i)
            filename = trim(dir_forcing)//trim(tracer_forcing_filename(year, month, day, iv))
            IF (p_is_io) THEN
               CALL ncio_read_block_time(filename, trim(trc_var_vname(iv)), trc_gforc, time_i, trc_metdata)
               CALL block_data_copy(trc_metdata, trc_forcn_LB(iv))
            ENDIF
         ENDIF

         DO WHILE (trc_tstamp_UB(iv) == 'NULL' .or. trc_tstamp_UB(iv) <= mtstamp)
            IF (.not. (trc_tstamp_UB(iv) == 'NULL')) THEN
               IF (p_is_io) CALL block_data_copy(trc_forcn_UB(iv), trc_forcn_LB(iv))
            ENDIF

            CALL tracer_forcing_setstamp_UB(iv, year, month, day, time_i)
            filename = trim(dir_forcing)//trim(tracer_forcing_filename(year, month, day, iv))
            IF (p_is_io) THEN
               CALL ncio_read_block_time(filename, trim(trc_var_vname(iv)), trc_gforc, time_i, trc_metdata)
               CALL block_data_copy(trc_metdata, trc_forcn_UB(iv))
            ENDIF
         ENDDO
      ENDDO
   END SUBROUTINE tracer_forcing_read_LBUB

   SUBROUTINE tracer_forcing_interpolate_var (idate, iv)
      IMPLICIT NONE
      integer, intent(in) :: idate(3), iv
      type(timestamp) :: mtstamp
      integer :: dtLB, dtUB

      mtstamp = idate
      IF ((mtstamp < trc_tstamp_LB(iv)) .or. (trc_tstamp_UB(iv) < mtstamp)) THEN
         WRITE(6, *) 'tracer forcing data required is out of range! STOP!'
         CALL CoLM_stop()
      ENDIF

      dtLB = mtstamp - trc_tstamp_LB(iv)
      dtUB = trc_tstamp_UB(iv) - mtstamp

      IF (trim(trc_var_tintalgo(iv)) == 'linear') THEN
         IF ((dtLB + dtUB) > 0) THEN
            CALL block_data_linear_interp( &
               trc_forcn_LB(iv), real(dtUB, r8) / real(dtLB + dtUB, r8), &
               trc_forcn_UB(iv), real(dtLB, r8) / real(dtLB + dtUB, r8), &
               trc_forcn(iv))
         ELSE
            CALL block_data_copy(trc_forcn_LB(iv), trc_forcn(iv))
         ENDIF
      ELSEIF (trim(trc_var_tintalgo(iv)) == 'nearest') THEN
         IF (dtLB <= dtUB) THEN
            CALL block_data_copy(trc_forcn_LB(iv), trc_forcn(iv))
         ELSE
            CALL block_data_copy(trc_forcn_UB(iv), trc_forcn(iv))
         ENDIF
      ELSEIF (trim(trc_var_tintalgo(iv)) == 'uniform') THEN
         ! Match MOD_Forcing exactly: interval means logged forward belong
         ! to the lower timestamp; backward (and other non-forward logs)
         ! belong to the upper timestamp.
         IF (trim(trc_var_timelog(iv)) == 'forward') THEN
            CALL block_data_copy(trc_forcn_LB(iv), trc_forcn(iv))
         ELSE
            CALL block_data_copy(trc_forcn_UB(iv), trc_forcn(iv))
         ENDIF
      ELSE
         IF (p_is_master) WRITE(*,'(A,I0,3A)') &
            'ERROR tracer_forcing_interpolate_var: variable ', iv, &
            ' has invalid interpolation mode "', trim(trc_var_tintalgo(iv)), '".'
         CALL CoLM_stop()
      ENDIF
   END SUBROUTINE tracer_forcing_interpolate_var

   SUBROUTINE tracer_forcing_update_values ()
      IMPLICIT NONE
      integer :: iv, ip, itrc, decode_status
      real(r8) :: value
      logical :: value_valid

      IF (.not. p_is_worker) RETURN
      IF (.not. allocated(trc_forc_patch)) RETURN
      IF (.not. allocated(trc_forc_precip_value)) RETURN

      DO iv = 1, n_trc_forc_vars
         itrc = trc_var_itrc(iv)
         IF (itrc < 1 .or. itrc > ntracers) CYCLE
         DO ip = 1, size(trc_forc_patch, 1)
            CALL tracer_forcing_decode_value(iv, ip, value, value_valid, decode_status)
            IF (trc_var_stream(iv) == STREAM_PRECIP) trc_forc_precip_status(itrc, ip) = decode_status
            IF (trc_var_stream(iv) == STREAM_VAPOR) trc_forc_vapor_status(itrc, ip) = decode_status
            IF (.not. value_valid) CYCLE
            IF (trc_var_stream(iv) == STREAM_PRECIP) THEN
               trc_forc_precip_value(itrc, ip) = value
               trc_forc_has_precip(itrc, ip) = .true.
            ELSEIF (trc_var_stream(iv) == STREAM_VAPOR) THEN
               trc_forc_vapor_value(itrc, ip) = value
               trc_forc_has_vapor(itrc, ip) = .true.
            ENDIF
         ENDDO
      ENDDO
   END SUBROUTINE tracer_forcing_update_values

   SUBROUTINE tracer_forcing_decode_value (iv, ip, value, valid, status)
      IMPLICIT NONE
      integer, intent(in) :: iv, ip
      real(r8), intent(out) :: value
      logical, intent(out) :: valid
      integer, intent(out), optional :: status
      integer :: itrc, total_idx, decode_status
      real(r8) :: raw, total, min_total, delta

      value = 0._r8
      valid = .false.
      decode_status = DECODE_INVALID
      raw = trc_forc_patch(ip, iv)

      itrc = trc_var_itrc(iv)
      SELECT CASE (trc_var_mode(iv))
      CASE (MODE_HEAVY_OVER_TOTAL, MODE_NORMALIZED_OVER_TOTAL)
         total_idx = trc_var_total(iv)
         IF (total_idx <= 0) THEN
            IF (present(status)) status = decode_status
            RETURN
         ENDIF
         total = trc_forc_patch(ip, total_idx)
         IF (.not. tracer_forcing_valid_value(total)) THEN
            IF (present(status)) status = decode_status
            RETURN
         ENDIF
         min_total = trc_forc_min_q
         IF (trc_var_stream(iv) == STREAM_PRECIP) min_total = trc_forc_min_prcp
         IF (total <= min_total) THEN
            decode_status = DECODE_NEAR_DRY
            IF (present(status)) status = decode_status
            RETURN
         ENDIF
         IF (.not. tracer_forcing_valid_value(raw)) THEN
            IF (present(status)) status = decode_status
            RETURN
         ENDIF
         IF (raw < 0._r8) THEN
            IF (present(status)) status = decode_status
            RETURN
         ENDIF
         value = raw / total
         IF (trc_var_mode(iv) == MODE_NORMALIZED_OVER_TOTAL) THEN
            ! `normalized_over_total` means the heavy/total stream is
            ! normalized by the tracer reference value. This is not limited
            ! to category=isotope: a no-fractionation HDO conservative test
            ! still needs the IsoGSM normalized ratio converted back to the
            ! stored concentration per unit water. For ordinary conservative
            ! tracers the default ref_ratio=1 keeps this identical to
            ! heavy_over_total.
            value = value * tracers(itrc)%ref_ratio
         ENDIF
      CASE (MODE_DELTA)
         IF (.not. tracer_forcing_valid_value(raw)) THEN
            IF (present(status)) status = decode_status
            RETURN
         ENDIF
         IF (tracer_is_isotope(itrc)) THEN
            value = delta_to_R(raw, tracers(itrc)%ref_ratio)
         ELSE
            value = raw
         ENDIF
      CASE DEFAULT
         IF (.not. tracer_forcing_valid_value(raw)) THEN
            IF (present(status)) status = decode_status
            RETURN
         ENDIF
         value = raw
      END SELECT

      IF (tracer_is_isotope(itrc)) THEN
         IF (value <= trc_tiny) THEN
            IF (present(status)) status = decode_status
            RETURN
         ENDIF
         delta = tracer_forcing_ratio_to_delta(value, tracers(itrc)%ref_ratio)
         IF (abs(delta) > trc_delta_sanity_max) THEN
            IF (present(status)) status = decode_status
            RETURN
         ENDIF
      ELSE
         ! Conservative/reactive forcings are concentrations per unit water;
         ! zero is a valid concentration, negative values are not.
         IF (value < 0._r8) THEN
            IF (present(status)) status = decode_status
            RETURN
         ENDIF
      ENDIF
      valid = .true.
      decode_status = DECODE_VALID
      IF (present(status)) status = decode_status
   END SUBROUTINE tracer_forcing_decode_value

   logical FUNCTION tracer_forcing_valid_value (x)
      IMPLICIT NONE
      real(r8), intent(in) :: x

      tracer_forcing_valid_value = abs(x) < trc_forc_max_abs
   END FUNCTION tracer_forcing_valid_value

   SUBROUTINE tracer_forcing_setstamp_LB (mtstamp, iv, year, month, mday, time_i)
      IMPLICIT NONE
      type(timestamp), intent(in) :: mtstamp
      integer, intent(in) :: iv
      integer, intent(out) :: year, month, mday, time_i
      integer :: day, sec, sec_file
      integer :: months(0:12)

      CALL tracer_forcing_validate_var_dtime(iv)

      year = mtstamp%year
      day = mtstamp%day
      sec = mtstamp%sec

      trc_tstamp_LB(iv)%year = year
      trc_tstamp_LB(iv)%day = day

      IF (trim(DEF_forcing%groupby) == 'year') THEN
         sec_file = 86400 * (day - 1) + sec
         time_i = floor((sec_file - trc_var_offset(iv)) * 1.0_r8 / trc_var_dtime(iv)) + 1
         sec = (time_i - 1) * trc_var_dtime(iv) + trc_var_offset(iv) - 86400 * (day - 1)
         trc_tstamp_LB(iv)%sec = sec

         IF (sec < 0) THEN
            trc_tstamp_LB(iv)%sec = 86400 + sec
            trc_tstamp_LB(iv)%day = day - 1
            IF (trc_tstamp_LB(iv)%day == 0) THEN
               trc_tstamp_LB(iv)%year = year - 1
               IF (isleapyear(trc_tstamp_LB(iv)%year)) THEN
                  trc_tstamp_LB(iv)%day = 366
               ELSE
                  trc_tstamp_LB(iv)%day = 365
               ENDIF
            ENDIF
         ENDIF

         IF (sec < 0 .or. (sec == 0 .and. trc_var_offset(iv) /= 0)) THEN
            IF (year == DEF_forcing%startyr .and. DEF_forcing%startmo == 1 .and. day == 1) THEN
               sec = trc_var_offset(iv)
            ELSE
               sec = 86400 + sec
               day = day - 1
               IF (day == 0) THEN
                  year = year - 1
                  IF (isleapyear(year)) THEN
                     day = 366
                  ELSE
                     day = 365
                  ENDIF
               ENDIF
            ENDIF
         ENDIF

         IF (.not. DEF_forcing%leapyear .and. isleapyear(year) .and. day > 59) day = day - 1

         sec_file = 86400 * (day - 1) + sec
         time_i = floor((sec_file - trc_var_offset(iv)) * 1.0_r8 / trc_var_dtime(iv)) + 1
         CALL julian2monthday(year, day, month, mday)

      ELSEIF (trim(DEF_forcing%groupby) == 'month') THEN
         IF (isleapyear(year)) THEN
            months = (/0,31,60,91,121,152,182,213,244,274,305,335,366/)
         ELSE
            months = (/0,31,59,90,120,151,181,212,243,273,304,334,365/)
         ENDIF

         CALL julian2monthday(year, day, month, mday)
         sec_file = 86400 * (mday - 1) + sec
         time_i = floor((sec_file - trc_var_offset(iv)) * 1.0_r8 / trc_var_dtime(iv)) + 1
         sec = (time_i - 1) * trc_var_dtime(iv) + trc_var_offset(iv) - 86400 * (mday - 1)
         trc_tstamp_LB(iv)%sec = sec

         IF (sec < 0) THEN
            trc_tstamp_LB(iv)%sec = 86400 + sec
            trc_tstamp_LB(iv)%day = day - 1
            IF (trc_tstamp_LB(iv)%day == 0) THEN
               trc_tstamp_LB(iv)%year = year - 1
               IF (isleapyear(trc_tstamp_LB(iv)%year)) THEN
                  trc_tstamp_LB(iv)%day = 366
               ELSE
                  trc_tstamp_LB(iv)%day = 365
               ENDIF
            ENDIF
         ENDIF

         IF (sec < 0 .or. (sec == 0 .and. trc_var_offset(iv) /= 0)) THEN
            IF (year == DEF_forcing%startyr .and. month == DEF_forcing%startmo .and. mday == 1) THEN
               sec = trc_var_offset(iv)
            ELSE
               sec = 86400 + sec
               mday = mday - 1
               IF (mday == 0) THEN
                  month = month - 1
                  IF (month == 0) THEN
                     month = 12
                     year = year - 1
                     mday = 31
                  ELSE
                     mday = months(month) - months(month - 1)
                  ENDIF
               ENDIF
            ENDIF
         ENDIF

         IF (.not. DEF_forcing%leapyear .and. isleapyear(year) .and. month == 2 .and. mday == 29) mday = 28

         sec_file = 86400 * (mday - 1) + sec
         time_i = floor((sec_file - trc_var_offset(iv)) * 1.0_r8 / trc_var_dtime(iv)) + 1

      ELSEIF (trim(DEF_forcing%groupby) == 'day') THEN
         CALL julian2monthday(year, day, month, mday)
         time_i = floor((sec - trc_var_offset(iv)) * 1.0_r8 / trc_var_dtime(iv)) + 1
         sec = (time_i - 1) * trc_var_dtime(iv) + trc_var_offset(iv)
         trc_tstamp_LB(iv)%sec = sec

         IF (sec < 0) THEN
            trc_tstamp_LB(iv)%sec = 86400 + sec
            trc_tstamp_LB(iv)%day = day - 1
            IF (trc_tstamp_LB(iv)%day == 0) THEN
               trc_tstamp_LB(iv)%year = year - 1
               IF (isleapyear(trc_tstamp_LB(iv)%year)) THEN
                  trc_tstamp_LB(iv)%day = 366
               ELSE
                  trc_tstamp_LB(iv)%day = 365
               ENDIF
            ENDIF

            IF (year == DEF_forcing%startyr .and. month == DEF_forcing%startmo .and. mday == 1) THEN
               sec = trc_var_offset(iv)
            ELSE
               sec = 86400 + sec
               year = trc_tstamp_LB(iv)%year
               CALL julian2monthday(trc_tstamp_LB(iv)%year, trc_tstamp_LB(iv)%day, month, mday)
            ENDIF
         ENDIF

         IF (.not. DEF_forcing%leapyear .and. isleapyear(year) .and. month == 2 .and. mday == 29) mday = 28
         time_i = floor((sec - trc_var_offset(iv)) * 1.0_r8 / trc_var_dtime(iv)) + 1

      ELSE
         IF (p_is_master) WRITE(*,'(2A)') 'ERROR tracer forcing unsupported groupby: ', trim(DEF_forcing%groupby)
         CALL CoLM_stop()
      ENDIF

      IF (time_i <= 0) THEN
         WRITE(6, *) 'got the wrong time record of tracer forcing! STOP!'
         CALL CoLM_stop()
      ENDIF
   END SUBROUTINE tracer_forcing_setstamp_LB

   SUBROUTINE tracer_forcing_setstamp_UB (iv, year, month, mday, time_i)
      IMPLICIT NONE
      integer, intent(in) :: iv
      integer, intent(out) :: year, month, mday, time_i
      integer :: day, sec, sec_file
      integer :: months(0:12)

      CALL tracer_forcing_validate_var_dtime(iv)

      IF (trc_tstamp_UB(iv) == 'NULL') THEN
         trc_tstamp_UB(iv) = trc_tstamp_LB(iv) + trc_var_dtime(iv)
      ELSE
         trc_tstamp_LB(iv) = trc_tstamp_UB(iv)
         trc_tstamp_UB(iv) = trc_tstamp_UB(iv) + trc_var_dtime(iv)
      ENDIF

      year = trc_tstamp_UB(iv)%year
      day = trc_tstamp_UB(iv)%day
      sec = trc_tstamp_UB(iv)%sec

      IF (trim(DEF_forcing%groupby) == 'year') THEN
         IF (sec == 86400 .and. trc_var_offset(iv) == 0) THEN
            sec = 0
            day = day + 1
            IF (isleapyear(year) .and. day == 367) THEN
               year = year + 1
               day = 1
            ENDIF
            IF ((.not. isleapyear(year)) .and. day == 366) THEN
               year = year + 1
               day = 1
            ENDIF
         ENDIF

         IF (.not. DEF_forcing%leapyear .and. isleapyear(year) .and. day > 59) day = day - 1

         sec_file = 86400 * (day - 1) + sec
         time_i = floor((sec_file - trc_var_offset(iv)) * 1.0_r8 / trc_var_dtime(iv)) + 1
         CALL julian2monthday(year, day, month, mday)

      ELSEIF (trim(DEF_forcing%groupby) == 'month') THEN
         IF (isleapyear(year)) THEN
            months = (/0,31,60,91,121,152,182,213,244,274,305,335,366/)
         ELSE
            months = (/0,31,59,90,120,151,181,212,243,273,304,334,365/)
         ENDIF

         CALL julian2monthday(year, day, month, mday)

         IF (sec == 86400 .and. trc_var_offset(iv) == 0) THEN
            sec = 0
            mday = mday + 1
            IF (mday > (months(month) - months(month - 1))) THEN
               mday = 1
               IF (month == 12) THEN
                  month = 1
                  year = year + 1
               ELSE
                  month = month + 1
               ENDIF
            ENDIF
         ENDIF

         IF (.not. DEF_forcing%leapyear .and. isleapyear(year) .and. month == 2 .and. mday == 29) mday = 28

         sec_file = 86400 * (mday - 1) + sec
         time_i = floor((sec_file - trc_var_offset(iv)) * 1.0_r8 / trc_var_dtime(iv)) + 1

      ELSEIF (trim(DEF_forcing%groupby) == 'day') THEN
         IF (isleapyear(year)) THEN
            months = (/0,31,60,91,121,152,182,213,244,274,305,335,366/)
         ELSE
            months = (/0,31,59,90,120,151,181,212,243,273,304,334,365/)
         ENDIF

         CALL julian2monthday(year, day, month, mday)

         IF (sec == 86400 .and. trc_var_offset(iv) == 0) THEN
            sec = 0
            mday = mday + 1
            IF (mday > (months(month) - months(month - 1))) THEN
               mday = 1
               IF (month == 12) THEN
                  month = 1
                  year = year + 1
               ELSE
                  month = month + 1
               ENDIF
            ENDIF
         ENDIF

         IF (.not. DEF_forcing%leapyear .and. isleapyear(year) .and. month == 2 .and. mday == 29) mday = 28
         time_i = floor((sec - trc_var_offset(iv)) * 1.0_r8 / trc_var_dtime(iv)) + 1

      ELSE
         IF (p_is_master) WRITE(*,'(2A)') 'ERROR tracer forcing unsupported groupby: ', trim(DEF_forcing%groupby)
         CALL CoLM_stop()
      ENDIF

      IF (time_i <= 0) THEN
         WRITE(6, *) 'got the wrong time record of tracer forcing! STOP!'
         CALL CoLM_stop()
      ENDIF
   END SUBROUTINE tracer_forcing_setstamp_UB

   SUBROUTINE tracer_forcing_validate_var_dtime (iv)
      IMPLICIT NONE
      integer, intent(in) :: iv

      IF (.not. allocated(trc_var_dtime)) THEN
         IF (p_is_master) WRITE(*,'(A)') 'ERROR tracer forcing dtime is not configured.'
         CALL CoLM_stop()
      ENDIF
      IF (iv < 1 .or. iv > n_trc_forc_vars) THEN
         IF (p_is_master) WRITE(*,'(A,I0,A)') 'ERROR tracer forcing variable index ', iv, ' is out of range.'
         CALL CoLM_stop()
      ENDIF
      IF (trc_var_dtime(iv) <= 0) THEN
         IF (p_is_master) WRITE(*,'(A,I0,A,I0,A)') 'ERROR tracer forcing dtime for variable ', iv, &
            ' must be > 0, got ', trc_var_dtime(iv), '.'
         CALL CoLM_stop()
      ENDIF
   END SUBROUTINE tracer_forcing_validate_var_dtime

   character(len=256) FUNCTION tracer_forcing_filename (year, month, day, iv)
      IMPLICIT NONE
      integer, intent(in) :: year, month, day, iv
      integer :: main_var
      character(len=16) :: yearstr, monthstr, daystr

      main_var = tracer_forcing_main_var_index(iv)
      IF (main_var > 0 .and. trim(trc_var_fprefix(iv)) == trim(DEF_forcing%fprefix(main_var))) THEN
         tracer_forcing_filename = metfilename(year, month, day, main_var)
         RETURN
      ENDIF

      write(yearstr, '(I4.4)') year
      write(monthstr, '(I2.2)') month
      write(daystr, '(I2.2)') day

      IF (trim(DEF_forcing%groupby) == 'year') THEN
         tracer_forcing_filename = '/'//trim(trc_var_fprefix(iv))//'_'//trim(yearstr)//'.nc'
      ELSEIF (trim(DEF_forcing%groupby) == 'month') THEN
         tracer_forcing_filename = '/'//trim(trc_var_fprefix(iv))//'_'//trim(yearstr)//'_'//trim(monthstr)//'.nc'
      ELSEIF (trim(DEF_forcing%groupby) == 'day') THEN
         tracer_forcing_filename = '/'//trim(trc_var_fprefix(iv))//'_'//trim(yearstr)//'_'//trim(monthstr)// &
            '_'//trim(daystr)//'.nc'
      ELSE
         tracer_forcing_filename = '/'//trim(trc_var_fprefix(iv))//'_'//trim(yearstr)//'.nc'
      ENDIF
   END FUNCTION tracer_forcing_filename

   integer FUNCTION tracer_forcing_main_var_index (iv)
      IMPLICIT NONE
      integer, intent(in) :: iv

      tracer_forcing_main_var_index = 0
      SELECT CASE (trc_var_stream(iv))
      CASE (STREAM_PRECIP, STREAM_TOTAL_PRECIP)
         tracer_forcing_main_var_index = 4
      CASE (STREAM_VAPOR, STREAM_TOTAL_VAPOR)
         tracer_forcing_main_var_index = 2
      END SELECT
   END FUNCTION tracer_forcing_main_var_index

   real(r8) FUNCTION tracer_forcing_precip_value (itrc, ipatch)
      IMPLICIT NONE
      integer, intent(in) :: itrc, ipatch

      tracer_forcing_precip_value = tracer_precip_default_ratio(itrc)
      IF (.not. allocated(trc_forc_precip_value)) RETURN
      IF (itrc < 1 .or. itrc > ntracers) RETURN
      IF (ipatch < 1 .or. ipatch > size(trc_forc_precip_value, 2)) RETURN
      tracer_forcing_precip_value = trc_forc_precip_value(itrc, ipatch)
   END FUNCTION tracer_forcing_precip_value

   real(r8) FUNCTION tracer_forcing_vapor_value (itrc, ipatch)
      IMPLICIT NONE
      integer, intent(in) :: itrc, ipatch

      tracer_forcing_vapor_value = tracer_vapor_default_ratio(itrc)
      IF (.not. allocated(trc_forc_vapor_value)) RETURN
      IF (itrc < 1 .or. itrc > ntracers) RETURN
      IF (ipatch < 1 .or. ipatch > size(trc_forc_vapor_value, 2)) RETURN
      tracer_forcing_vapor_value = trc_forc_vapor_value(itrc, ipatch)
   END FUNCTION tracer_forcing_vapor_value

   real(r8) FUNCTION tracer_forcing_precip_ratio (itrc, ipatch)
      IMPLICIT NONE
      integer, intent(in) :: itrc, ipatch
      tracer_forcing_precip_ratio = tracer_forcing_precip_value(itrc, ipatch)
   END FUNCTION tracer_forcing_precip_ratio

   real(r8) FUNCTION tracer_forcing_vapor_ratio (itrc, ipatch)
      IMPLICIT NONE
      integer, intent(in) :: itrc, ipatch
      tracer_forcing_vapor_ratio = tracer_forcing_vapor_value(itrc, ipatch)
   END FUNCTION tracer_forcing_vapor_ratio

   logical FUNCTION tracer_forcing_has_precip (itrc, ipatch)
      IMPLICIT NONE
      integer, intent(in) :: itrc, ipatch

      tracer_forcing_has_precip = .false.
      IF (.not. allocated(trc_forc_has_precip)) RETURN
      IF (itrc < 1 .or. itrc > ntracers) RETURN
      IF (ipatch < 1 .or. ipatch > size(trc_forc_has_precip, 2)) RETURN
      tracer_forcing_has_precip = trc_forc_has_precip(itrc, ipatch) .or. &
         tracer_forcing_precip_configured(itrc)
   END FUNCTION tracer_forcing_has_precip

   logical FUNCTION tracer_forcing_has_vapor (itrc, ipatch)
      IMPLICIT NONE
      integer, intent(in) :: itrc, ipatch

      tracer_forcing_has_vapor = .false.
      IF (.not. allocated(trc_forc_has_vapor)) RETURN
      IF (itrc < 1 .or. itrc > ntracers) RETURN
      IF (ipatch < 1 .or. ipatch > size(trc_forc_has_vapor, 2)) RETURN
      tracer_forcing_has_vapor = trc_forc_has_vapor(itrc, ipatch) .or. &
         tracer_forcing_vapor_configured(itrc)
   END FUNCTION tracer_forcing_has_vapor


   logical FUNCTION tracer_forcing_precip_configured (itrc)
      IMPLICIT NONE
      integer, intent(in) :: itrc
      integer :: iv

      tracer_forcing_precip_configured = .false.
      DO iv = 1, n_trc_forc_vars
         IF (trc_var_stream(iv) == STREAM_PRECIP .and. trc_var_itrc(iv) == itrc) THEN
            tracer_forcing_precip_configured = .true.
            RETURN
         ENDIF
      ENDDO
   END FUNCTION tracer_forcing_precip_configured

   logical FUNCTION tracer_forcing_vapor_configured (itrc)
      IMPLICIT NONE
      integer, intent(in) :: itrc
      integer :: iv

      tracer_forcing_vapor_configured = .false.
      DO iv = 1, n_trc_forc_vars
         IF (trc_var_stream(iv) == STREAM_VAPOR .and. trc_var_itrc(iv) == itrc) THEN
            tracer_forcing_vapor_configured = .true.
            RETURN
         ENDIF
      ENDDO
   END FUNCTION tracer_forcing_vapor_configured

   real(r8) FUNCTION tracer_forcing_ratio_to_delta (ratio, ref_ratio)
      IMPLICIT NONE
      real(r8), intent(in) :: ratio, ref_ratio

      IF (ratio > trc_tiny .and. ref_ratio > trc_tiny) THEN
         tracer_forcing_ratio_to_delta = (ratio / ref_ratio - 1._r8) * 1000._r8
      ELSE
         tracer_forcing_ratio_to_delta = 0._r8
      ENDIF
   END FUNCTION tracer_forcing_ratio_to_delta

   SUBROUTINE tracer_forcing_log_ranges (idate)
      IMPLICIT NONE
      integer, intent(in) :: idate(3)
      integer :: itrc, ip
      real(r8), allocatable :: pmin(:), pmax(:), vmin(:), vmax(:)
      integer, allocatable :: pcnt(:), vcnt(:), pmiss(:), pdry(:), vmiss(:), vdry(:)
      real(r8) :: outval
      logical :: emit_header

      IF (.not. allocated(trc_forc_precip_value)) RETURN

      allocate(pmin(ntracers), pmax(ntracers), vmin(ntracers), vmax(ntracers))
      allocate(pcnt(ntracers), vcnt(ntracers), pmiss(ntracers), pdry(ntracers), vmiss(ntracers), vdry(ntracers))
      pmin(:) = huge(1._r8)
      vmin(:) = huge(1._r8)
      pmax(:) = -huge(1._r8)
      vmax(:) = -huge(1._r8)
      pcnt(:) = 0
      vcnt(:) = 0
      pmiss(:) = 0
      pdry(:) = 0
      vmiss(:) = 0
      vdry(:) = 0

      IF (p_is_worker) THEN
         DO itrc = 1, ntracers
            IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
            DO ip = 1, size(trc_forc_precip_value, 2)
               IF (trc_forc_has_precip(itrc, ip)) THEN
                  outval = tracer_forcing_diag_value(itrc, trc_forc_precip_value(itrc, ip))
                  pmin(itrc) = min(pmin(itrc), outval)
                  pmax(itrc) = max(pmax(itrc), outval)
                  pcnt(itrc) = pcnt(itrc) + 1
               ELSEIF (tracer_is_isotope(itrc) .and. tracer_forcing_precip_configured(itrc)) THEN
                  IF (allocated(trc_forc_precip_status) .and. &
                      trc_forc_precip_status(itrc, ip) == DECODE_NEAR_DRY) THEN
                     pdry(itrc) = pdry(itrc) + 1
                  ELSE
                     pmiss(itrc) = pmiss(itrc) + 1
                  ENDIF
               ENDIF
               IF (trc_forc_has_vapor(itrc, ip)) THEN
                  outval = tracer_forcing_diag_value(itrc, trc_forc_vapor_value(itrc, ip))
                  vmin(itrc) = min(vmin(itrc), outval)
                  vmax(itrc) = max(vmax(itrc), outval)
                  vcnt(itrc) = vcnt(itrc) + 1
               ELSEIF (tracer_fractionation_active(itrc)) THEN
                  IF (allocated(trc_forc_vapor_status) .and. &
                      trc_forc_vapor_status(itrc, ip) == DECODE_NEAR_DRY) THEN
                     vdry(itrc) = vdry(itrc) + 1
                  ELSE
                     vmiss(itrc) = vmiss(itrc) + 1
                  ENDIF
               ENDIF
            ENDDO
         ENDDO
      ENDIF

#ifdef USEMPI
      CALL mpi_allreduce(MPI_IN_PLACE, pmin, ntracers, MPI_REAL8, MPI_MIN, p_comm_glb, p_err)
      CALL mpi_allreduce(MPI_IN_PLACE, pmax, ntracers, MPI_REAL8, MPI_MAX, p_comm_glb, p_err)
      CALL mpi_allreduce(MPI_IN_PLACE, vmin, ntracers, MPI_REAL8, MPI_MIN, p_comm_glb, p_err)
      CALL mpi_allreduce(MPI_IN_PLACE, vmax, ntracers, MPI_REAL8, MPI_MAX, p_comm_glb, p_err)
      CALL mpi_allreduce(MPI_IN_PLACE, pcnt, ntracers, MPI_INTEGER, MPI_SUM, p_comm_glb, p_err)
      CALL mpi_allreduce(MPI_IN_PLACE, vcnt, ntracers, MPI_INTEGER, MPI_SUM, p_comm_glb, p_err)
      CALL mpi_allreduce(MPI_IN_PLACE, pmiss, ntracers, MPI_INTEGER, MPI_SUM, p_comm_glb, p_err)
      CALL mpi_allreduce(MPI_IN_PLACE, pdry, ntracers, MPI_INTEGER, MPI_SUM, p_comm_glb, p_err)
      CALL mpi_allreduce(MPI_IN_PLACE, vmiss, ntracers, MPI_INTEGER, MPI_SUM, p_comm_glb, p_err)
      CALL mpi_allreduce(MPI_IN_PLACE, vdry, ntracers, MPI_INTEGER, MPI_SUM, p_comm_glb, p_err)
#endif

      IF (p_is_master) THEN
         emit_header = trc_forcing_log_count < 3
         DO itrc = 1, ntracers
            IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
            IF (pdry(itrc) > 0 .and. &
                (trc_forcing_log_count < 3 .or. .not. trc_forc_precip_warned_dry(itrc))) &
               emit_header = .true.
            IF (pmiss(itrc) > 0 .and. &
                (trc_forcing_log_count < 3 .or. .not. trc_forc_precip_warned_invalid(itrc))) &
               emit_header = .true.
            IF (vdry(itrc) > 0 .and. &
                (trc_forcing_log_count < 3 .or. .not. trc_forc_vapor_warned_dry(itrc))) &
               emit_header = .true.
            IF (vmiss(itrc) > 0 .and. &
                (trc_forcing_log_count < 3 .or. .not. trc_forc_vapor_warned_invalid(itrc))) &
               emit_header = .true.
         ENDDO
         IF (emit_header) WRITE(*,'(/,A,I4.4,A,I3.3,A,I5.5)') &
            'Checking tracer forcing at ', idate(1), '-', idate(2), '-', idate(3)
         DO itrc = 1, ntracers
            IF (.not. tracer_uses_land_water_transport(itrc)) CYCLE
            IF (trc_forcing_log_count < 3 .and. pcnt(itrc) > 0) THEN
               WRITE(*,'(3A,I0,A,F12.5,A,F12.5)') '  precip ', trim(tracers(itrc)%name), &
                  ' n=', pcnt(itrc), ' min=', pmin(itrc), ' max=', pmax(itrc)
            ENDIF
            IF (trc_forcing_log_count < 3 .and. vcnt(itrc) > 0) THEN
               WRITE(*,'(3A,I0,A,F12.5,A,F12.5)') '  vapor   ', trim(tracers(itrc)%name), &
                  ' n=', vcnt(itrc), ' min=', vmin(itrc), ' max=', vmax(itrc)
            ENDIF
            IF (pdry(itrc) > 0 .and. &
                (trc_forcing_log_count < 3 .or. .not. trc_forc_precip_warned_dry(itrc))) THEN
               WRITE(*,'(3A,I0,A)') '  WARNING precip ', trim(tracers(itrc)%name), &
                  ' near-dry fallback patches=', pdry(itrc), &
                  ' (retaining the previous valid/default precip ratio)'
               trc_forc_precip_warned_dry(itrc) = .true.
            ENDIF
            IF (pmiss(itrc) > 0 .and. &
                (trc_forcing_log_count < 3 .or. .not. trc_forc_precip_warned_invalid(itrc))) THEN
               WRITE(*,'(3A,I0,A)') '  WARNING precip ', trim(tracers(itrc)%name), &
                  ' invalid/missing wet fallback patches=', pmiss(itrc), &
                  ' (retaining the previous valid/default precip ratio)'
               trc_forc_precip_warned_invalid(itrc) = .true.
            ENDIF
            IF (vdry(itrc) > 0 .and. &
                (trc_forcing_log_count < 3 .or. .not. trc_forc_vapor_warned_dry(itrc))) THEN
               WRITE(*,'(3A,I0,A)') '  WARNING vapor ', trim(tracers(itrc)%name), &
                  ' near-dry fallback patches=', vdry(itrc), &
                  ' (retaining the previous valid/default vapor ratio)'
               trc_forc_vapor_warned_dry(itrc) = .true.
            ENDIF
            IF (vmiss(itrc) > 0 .and. &
                (trc_forcing_log_count < 3 .or. .not. trc_forc_vapor_warned_invalid(itrc))) THEN
               WRITE(*,'(3A,I0,A)') '  WARNING vapor ', trim(tracers(itrc)%name), &
                  ' invalid/missing wet fallback patches=', vmiss(itrc), &
                  ' (retaining the previous valid/default vapor ratio)'
               trc_forc_vapor_warned_invalid(itrc) = .true.
            ENDIF
         ENDDO
      ENDIF

      trc_forcing_log_count = trc_forcing_log_count + 1
      deallocate(pmin, pmax, vmin, vmax, pcnt, vcnt, pmiss, pdry, vmiss, vdry)
   END SUBROUTINE tracer_forcing_log_ranges

   real(r8) FUNCTION tracer_forcing_diag_value (itrc, value)
      IMPLICIT NONE
      integer, intent(in) :: itrc
      real(r8), intent(in) :: value

      IF (tracer_is_isotope(itrc)) THEN
         tracer_forcing_diag_value = tracer_forcing_ratio_to_delta(value, tracers(itrc)%ref_ratio)
      ELSE
         tracer_forcing_diag_value = value
      ENDIF
   END FUNCTION tracer_forcing_diag_value

   logical FUNCTION tracer_forcing_pair_present (fprefix, vname)
      IMPLICIT NONE
      character(len=*), intent(in) :: fprefix, vname
      tracer_forcing_pair_present = tracer_forcing_token_present(fprefix) .and. tracer_forcing_token_present(vname)
   END FUNCTION tracer_forcing_pair_present

   logical FUNCTION tracer_forcing_token_present (token)
      IMPLICIT NONE
      character(len=*), intent(in) :: token
      character(len=256) :: low

      low = tracer_lower(trim(token))
      tracer_forcing_token_present = len_trim(low) > 0 .and. trim(low) /= 'null' .and. trim(low) /= 'none'
   END FUNCTION tracer_forcing_token_present

   integer FUNCTION tracer_forcing_parse_mode (token)
      IMPLICIT NONE
      character(len=*), intent(in) :: token
      character(len=256) :: low

      low = tracer_lower(trim(token))
      IF (trim(low) == 'delta') THEN
         tracer_forcing_parse_mode = MODE_DELTA
      ELSEIF (trim(low) == 'heavy_over_total' .or. trim(low) == 'ratio_to_total' .or. &
              trim(low) == 'over_total') THEN
         tracer_forcing_parse_mode = MODE_HEAVY_OVER_TOTAL
      ELSEIF (trim(low) == 'normalized_over_total' .or. trim(low) == 'normalized_ratio' .or. &
              trim(low) == 'isogsm' .or. trim(low) == 'standard_over_total') THEN
         tracer_forcing_parse_mode = MODE_NORMALIZED_OVER_TOTAL
      ELSEIF (trim(low) == 'direct' .or. trim(low) == 'value' .or. trim(low) == 'raw') THEN
         tracer_forcing_parse_mode = MODE_VALUE
      ELSE
         IF (p_is_master) WRITE(*,'(3A)') &
            'ERROR tracer_forcing_parse_mode: unknown forcing_input_mode "', trim(token), '".'
         CALL CoLM_stop()
         tracer_forcing_parse_mode = MODE_VALUE
      ENDIF
   END FUNCTION tracer_forcing_parse_mode

   SUBROUTINE tracer_forcing_csv_token (csv, idx, token)
      IMPLICIT NONE
      character(len=*), intent(in) :: csv
      integer, intent(in) :: idx
      character(len=*), intent(out) :: token
      integer :: i, n, start, finish

      token = ''
      IF (idx <= 0) RETURN
      n = 1
      start = 1
      DO i = 1, len_trim(csv) + 1
         IF (i > len_trim(csv) .or. csv(i:i) == ',') THEN
            finish = i - 1
            IF (n == idx) THEN
               IF (finish >= start) token = adjustl(csv(start:finish))
               RETURN
            ENDIF
            n = n + 1
            start = i + 1
         ENDIF
      ENDDO
   END SUBROUTINE tracer_forcing_csv_token

   integer FUNCTION tracer_forcing_csv_int (csv, idx, default_value)
      IMPLICIT NONE
      character(len=*), intent(in) :: csv
      integer, intent(in) :: idx, default_value
      character(len=256) :: token
      integer :: ierr

      tracer_forcing_csv_int = default_value
      CALL tracer_forcing_csv_token(csv, idx, token)
      IF (.not. tracer_forcing_token_present(token)) RETURN
      read(token, *, iostat=ierr) tracer_forcing_csv_int
      IF (ierr /= 0) tracer_forcing_csv_int = default_value
   END FUNCTION tracer_forcing_csv_int

END MODULE MOD_Tracer_Forcing
#endif
