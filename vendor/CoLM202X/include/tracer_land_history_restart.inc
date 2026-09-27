! Generic land-tracer history shares the land-history sidecar commit marker.
! Keep this inventory in lockstep with the a_trc_*/a_water_* declarations.
   logical FUNCTION tracer_history_required()
   IMPLICIT NONE
   integer :: itrc
      tracer_history_required=.false.
      DO itrc=1,ntracers
         IF (tracer_uses_land_water_transport(itrc)) tracer_history_required=.true.
      ENDDO
   END FUNCTION tracer_history_required

   SUBROUTINE tracer_history_manifest(names, ranks, dims)
   IMPLICIT NONE
   character(len=64), intent(out) :: names(TRC_HISTORY_FIELDS)
   integer, intent(out) :: ranks(TRC_HISTORY_FIELDS), dims(2,TRC_HISTORY_FIELDS)
      names=[character(len=64) :: &
         'a_trc_precip', &
         'a_trc_vapor_exchange', &
         'a_water_precip', &
         'a_trc_evap', &
         'a_water_evap_gross', &
         'a_trc_transp', &
         'a_trc_transp_src', &
         'a_water_transp', &
         'a_trc_soilevap', &
         'a_water_soilevap', &
         'a_trc_canopyevap', &
         'a_water_canopyevap', &
         'a_trc_subl', &
         'a_water_subl', &
         'a_trc_wetland_evap', &
         'a_water_wetland_evap', &
         'a_trc_rsur', &
         'a_trc_rsub', &
         'a_trc_rnof', &
         'a_water_rnof', &
         'a_trc_qinfl', &
         'a_trc_qcharge', &
         'a_trc_ldew_mass', &
         'a_water_ldew', &
         'a_trc_soil_mass', &
         'a_water_soil', &
         'a_trc_snow_mass', &
         'a_water_snow', &
         'a_trc_wa_mass', &
         'a_water_wa', &
         'a_trc_wa_debt_mass', &
         'a_water_wa_debt', &
         'a_water_aquifer_actual', &
         'a_trc_aquifer_actual_mass', &
         'a_trc_wdsrf_mass', &
         'a_water_wdsrf', &
         'a_trc_wetwat_mass', &
         'a_water_wetwat', &
         'a_trc_surface_residue_mass', &
         'a_trc_subsurface_residue_mass', &
         'a_trc_layer_dry_mass', &
         'a_trc_solid_mass', &
         'a_trc_scv_mass', &
         'a_water_scv']
      ranks=2; dims=0
      dims(1,:)=ntracers
      ranks(24)=1; dims(1,24)=0
      ranks(25)=3; dims(2,25)=size(a_trc_soil_mass,2)
      dims(1,26)=size(a_water_soil,1)
      ranks(27)=3; dims(2,27)=size(a_trc_snow_mass,2)
      dims(1,28)=size(a_water_snow,1)
      ranks(30)=1; dims(1,30)=0
      ranks(32)=1; dims(1,32)=0
      ranks(33)=1; dims(1,33)=0
      ranks(36)=1; dims(1,36)=0
      ranks(38)=1; dims(1,38)=0
      ranks(44)=1; dims(1,44)=0
   END SUBROUTINE tracer_history_manifest

   SUBROUTINE tracer_history_descriptor(file, writing)
   USE MOD_LandPatch, only: landpatch
   USE MOD_Block, only: get_filename_block
   USE MOD_NetCDFSerial, only: ncio_var_exist, ncio_inquire_varsize, &
      ncio_read_serial, ncio_write_serial, ncio_define_dimension
#ifdef USEMPI
   USE MOD_SPMD_Task, only: p_comm_glb, p_err, MPI_IN_PLACE, MPI_INTEGER, MPI_MAX
#endif
   IMPLICIT NONE
   character(len=*), intent(in) :: file
   logical, intent(in) :: writing
   integer, allocatable :: expected(:,:), actual(:,:), varsize(:)
   integer :: iblkgrp, iblk, jblk, schema, nstored, bad
   character(len=256) :: block_file
      CALL tracer_build_descriptor_identity(expected)
      bad=0
      IF (p_is_io) THEN
         DO iblkgrp=1,landpatch%nblkgrp
            iblk=landpatch%xblkgrp(iblkgrp)
            jblk=landpatch%yblkgrp(iblkgrp)
            CALL get_filename_block(file, iblk, jblk, block_file)
            IF (writing) THEN
               CALL ncio_write_serial(block_file, 'trc_hist_schema', 1)
               CALL ncio_write_serial(block_file, 'trc_hist_count', ntracers)
               CALL ncio_define_dimension(block_file, 'trc_hist_descriptor_field', size(expected,1))
               CALL ncio_define_dimension(block_file, 'trc_hist_descriptor_row', ntracers)
               CALL ncio_write_serial(block_file, 'trc_hist_descriptor', expected, &
                  'trc_hist_descriptor_field', 'trc_hist_descriptor_row')
            ELSE
               IF (.not.ncio_var_exist(block_file, 'trc_hist_schema', readflag=.false.) .or. &
                   .not.ncio_var_exist(block_file, 'trc_hist_count', readflag=.false.) .or. &
                   .not.ncio_var_exist(block_file, 'trc_hist_descriptor', readflag=.false.)) THEN
                  bad=1
                  CYCLE
               ENDIF
               CALL ncio_inquire_varsize(block_file, 'trc_hist_schema', varsize)
               IF (.not.allocated(varsize)) THEN
                  bad=1
                  CYCLE
               ENDIF
               IF (size(varsize)/=0) bad=1
               deallocate(varsize)
               CALL ncio_inquire_varsize(block_file, 'trc_hist_count', varsize)
               IF (.not.allocated(varsize)) THEN
                  bad=1
                  CYCLE
               ENDIF
               IF (size(varsize)/=0) bad=1
               deallocate(varsize)
               IF (bad/=0) CYCLE
               CALL ncio_inquire_varsize(block_file, 'trc_hist_descriptor', varsize)
               IF (.not.allocated(varsize)) THEN
                  bad=1
                  CYCLE
               ENDIF
               IF (size(varsize)/=2) THEN
                  bad=1
               ELSEIF (any(varsize/=shape(expected))) THEN
                  bad=1
               ENDIF
               deallocate(varsize)
               IF (bad/=0) CYCLE
               CALL ncio_read_serial(block_file, 'trc_hist_schema', schema)
               CALL ncio_read_serial(block_file, 'trc_hist_count', nstored)
               IF (schema/=1 .or. nstored/=ntracers) THEN
                  bad=1
                  CYCLE
               ENDIF
               CALL ncio_read_serial(block_file, 'trc_hist_descriptor', actual)
               IF (.not.allocated(actual)) THEN
                  bad=1
               ELSE
                  IF (any(actual/=expected)) bad=1
                  deallocate(actual)
               ENDIF
            ENDIF
         ENDDO
      ENDIF
#ifdef USEMPI
      CALL mpi_allreduce(MPI_IN_PLACE, bad, 1, MPI_INTEGER, MPI_MAX, p_comm_glb, p_err)
#endif
      IF (bad/=0) CALL CoLM_stop("incompatible or incomplete generic land-tracer history sidecar")
      deallocate(expected)
   END SUBROUTINE tracer_history_descriptor

   SUBROUTINE tracer_history_write(file)
   USE MOD_LandPatch, only: landpatch
   USE MOD_NetCDFVector, only: ncio_define_dimension_vector, ncio_write_vector
   IMPLICIT NONE
   character(len=*), intent(in) :: file
   character(len=64) :: names(TRC_HISTORY_FIELDS)
   integer :: ranks(TRC_HISTORY_FIELDS), dims(2,TRC_HISTORY_FIELDS), i
      CALL tracer_history_descriptor(file, .true.)
      CALL tracer_history_manifest(names, ranks, dims)
      DO i=1,TRC_HISTORY_FIELDS
         IF (ranks(i)>1) CALL ncio_define_dimension_vector(file, landpatch, &
            'd1_'//trim(names(i)), dims(1,i))
         IF (ranks(i)>2) CALL ncio_define_dimension_vector(file, landpatch, &
            'd2_'//trim(names(i)), dims(2,i))
      ENDDO
      CALL ncio_write_vector(file, 'a_trc_precip', 'd1_a_trc_precip', size(a_trc_precip,1), &
         'patch', landpatch, a_trc_precip)
      CALL ncio_write_vector(file, 'a_trc_vapor_exchange', 'd1_a_trc_vapor_exchange', size(a_trc_vapor_exchange,1), &
         'patch', landpatch, a_trc_vapor_exchange)
      CALL ncio_write_vector(file, 'a_water_precip', 'd1_a_water_precip', size(a_water_precip,1), &
         'patch', landpatch, a_water_precip)
      CALL ncio_write_vector(file, 'a_trc_evap', 'd1_a_trc_evap', size(a_trc_evap,1), &
         'patch', landpatch, a_trc_evap)
      CALL ncio_write_vector(file, 'a_water_evap_gross', 'd1_a_water_evap_gross', size(a_water_evap_gross,1), &
         'patch', landpatch, a_water_evap_gross)
      CALL ncio_write_vector(file, 'a_trc_transp', 'd1_a_trc_transp', size(a_trc_transp,1), &
         'patch', landpatch, a_trc_transp)
      CALL ncio_write_vector(file, 'a_trc_transp_src', 'd1_a_trc_transp_src', size(a_trc_transp_src,1), &
         'patch', landpatch, a_trc_transp_src)
      CALL ncio_write_vector(file, 'a_water_transp', 'd1_a_water_transp', size(a_water_transp,1), &
         'patch', landpatch, a_water_transp)
      CALL ncio_write_vector(file, 'a_trc_soilevap', 'd1_a_trc_soilevap', size(a_trc_soilevap,1), &
         'patch', landpatch, a_trc_soilevap)
      CALL ncio_write_vector(file, 'a_water_soilevap', 'd1_a_water_soilevap', size(a_water_soilevap,1), &
         'patch', landpatch, a_water_soilevap)
      CALL ncio_write_vector(file, 'a_trc_canopyevap', 'd1_a_trc_canopyevap', size(a_trc_canopyevap,1), &
         'patch', landpatch, a_trc_canopyevap)
      CALL ncio_write_vector(file, 'a_water_canopyevap', 'd1_a_water_canopyevap', size(a_water_canopyevap,1), &
         'patch', landpatch, a_water_canopyevap)
      CALL ncio_write_vector(file, 'a_trc_subl', 'd1_a_trc_subl', size(a_trc_subl,1), &
         'patch', landpatch, a_trc_subl)
      CALL ncio_write_vector(file, 'a_water_subl', 'd1_a_water_subl', size(a_water_subl,1), &
         'patch', landpatch, a_water_subl)
      CALL ncio_write_vector(file, 'a_trc_wetland_evap', 'd1_a_trc_wetland_evap', size(a_trc_wetland_evap,1), &
         'patch', landpatch, a_trc_wetland_evap)
      CALL ncio_write_vector(file, 'a_water_wetland_evap', 'd1_a_water_wetland_evap', size(a_water_wetland_evap,1), &
         'patch', landpatch, a_water_wetland_evap)
      CALL ncio_write_vector(file, 'a_trc_rsur', 'd1_a_trc_rsur', size(a_trc_rsur,1), &
         'patch', landpatch, a_trc_rsur)
      CALL ncio_write_vector(file, 'a_trc_rsub', 'd1_a_trc_rsub', size(a_trc_rsub,1), &
         'patch', landpatch, a_trc_rsub)
      CALL ncio_write_vector(file, 'a_trc_rnof', 'd1_a_trc_rnof', size(a_trc_rnof,1), &
         'patch', landpatch, a_trc_rnof)
      CALL ncio_write_vector(file, 'a_water_rnof', 'd1_a_water_rnof', size(a_water_rnof,1), &
         'patch', landpatch, a_water_rnof)
      CALL ncio_write_vector(file, 'a_trc_qinfl', 'd1_a_trc_qinfl', size(a_trc_qinfl,1), &
         'patch', landpatch, a_trc_qinfl)
      CALL ncio_write_vector(file, 'a_trc_qcharge', 'd1_a_trc_qcharge', size(a_trc_qcharge,1), &
         'patch', landpatch, a_trc_qcharge)
      CALL ncio_write_vector(file, 'a_trc_ldew_mass', 'd1_a_trc_ldew_mass', size(a_trc_ldew_mass,1), &
         'patch', landpatch, a_trc_ldew_mass)
      CALL ncio_write_vector(file, 'a_water_ldew', 'patch', landpatch, a_water_ldew)
      CALL ncio_write_vector(file, 'a_trc_soil_mass', 'd1_a_trc_soil_mass', size(a_trc_soil_mass,1), &
         'd2_a_trc_soil_mass', size(a_trc_soil_mass,2), 'patch', landpatch, a_trc_soil_mass)
      CALL ncio_write_vector(file, 'a_water_soil', 'd1_a_water_soil', size(a_water_soil,1), &
         'patch', landpatch, a_water_soil)
      CALL ncio_write_vector(file, 'a_trc_snow_mass', 'd1_a_trc_snow_mass', size(a_trc_snow_mass,1), &
         'd2_a_trc_snow_mass', size(a_trc_snow_mass,2), 'patch', landpatch, a_trc_snow_mass)
      CALL ncio_write_vector(file, 'a_water_snow', 'd1_a_water_snow', size(a_water_snow,1), &
         'patch', landpatch, a_water_snow)
      CALL ncio_write_vector(file, 'a_trc_wa_mass', 'd1_a_trc_wa_mass', size(a_trc_wa_mass,1), &
         'patch', landpatch, a_trc_wa_mass)
      CALL ncio_write_vector(file, 'a_water_wa', 'patch', landpatch, a_water_wa)
      CALL ncio_write_vector(file, 'a_trc_wa_debt_mass', 'd1_a_trc_wa_debt_mass', size(a_trc_wa_debt_mass,1), &
         'patch', landpatch, a_trc_wa_debt_mass)
      CALL ncio_write_vector(file, 'a_water_wa_debt', 'patch', landpatch, a_water_wa_debt)
      CALL ncio_write_vector(file, 'a_water_aquifer_actual', 'patch', landpatch, a_water_aquifer_actual)
      CALL ncio_write_vector(file, 'a_trc_aquifer_actual_mass', 'd1_a_trc_aquifer_actual_mass', size(a_trc_aquifer_actual_mass,1), &
         'patch', landpatch, a_trc_aquifer_actual_mass)
      CALL ncio_write_vector(file, 'a_trc_wdsrf_mass', 'd1_a_trc_wdsrf_mass', size(a_trc_wdsrf_mass,1), &
         'patch', landpatch, a_trc_wdsrf_mass)
      CALL ncio_write_vector(file, 'a_water_wdsrf', 'patch', landpatch, a_water_wdsrf)
      CALL ncio_write_vector(file, 'a_trc_wetwat_mass', 'd1_a_trc_wetwat_mass', size(a_trc_wetwat_mass,1), &
         'patch', landpatch, a_trc_wetwat_mass)
      CALL ncio_write_vector(file, 'a_water_wetwat', 'patch', landpatch, a_water_wetwat)
      CALL ncio_write_vector(file, 'a_trc_surface_residue_mass', 'd1_a_trc_surface_residue_mass', size(a_trc_surface_residue_mass,1), &
         'patch', landpatch, a_trc_surface_residue_mass)
      CALL ncio_write_vector(file, 'a_trc_subsurface_residue_mass', 'd1_a_trc_subsurface_residue_mass', size(a_trc_subsurface_residue_mass,1), &
         'patch', landpatch, a_trc_subsurface_residue_mass)
      CALL ncio_write_vector(file, 'a_trc_layer_dry_mass', 'd1_a_trc_layer_dry_mass', size(a_trc_layer_dry_mass,1), &
         'patch', landpatch, a_trc_layer_dry_mass)
      CALL ncio_write_vector(file, 'a_trc_solid_mass', 'd1_a_trc_solid_mass', size(a_trc_solid_mass,1), &
         'patch', landpatch, a_trc_solid_mass)
      CALL ncio_write_vector(file, 'a_trc_scv_mass', 'd1_a_trc_scv_mass', size(a_trc_scv_mass,1), &
         'patch', landpatch, a_trc_scv_mass)
      CALL ncio_write_vector(file, 'a_water_scv', 'patch', landpatch, a_water_scv)
   END SUBROUTINE tracer_history_write

   SUBROUTINE tracer_history_restore(file)
   USE, INTRINSIC :: ieee_arithmetic, only: ieee_is_finite
   USE MOD_LandPatch, only: landpatch
   USE MOD_SPMD_Task, only: p_is_worker
#ifdef USEMPI
   USE MOD_SPMD_Task, only: p_comm_glb, p_err, MPI_IN_PLACE, MPI_INTEGER, MPI_MAX
#endif
   USE MOD_NetCDFVector, only: ncio_read_vector
   IMPLICIT NONE
   character(len=*), intent(in) :: file
   integer :: bad
      CALL tracer_history_descriptor(file, .false.)
      CALL ncio_read_vector(file, 'a_trc_precip', size(a_trc_precip,1), landpatch, a_trc_precip, known_present=.true.)
      CALL ncio_read_vector(file, 'a_trc_vapor_exchange', size(a_trc_vapor_exchange,1), landpatch, a_trc_vapor_exchange, known_present=.true.)
      CALL ncio_read_vector(file, 'a_water_precip', size(a_water_precip,1), landpatch, a_water_precip, known_present=.true.)
      CALL ncio_read_vector(file, 'a_trc_evap', size(a_trc_evap,1), landpatch, a_trc_evap, known_present=.true.)
      CALL ncio_read_vector(file, 'a_water_evap_gross', size(a_water_evap_gross,1), landpatch, a_water_evap_gross, known_present=.true.)
      CALL ncio_read_vector(file, 'a_trc_transp', size(a_trc_transp,1), landpatch, a_trc_transp, known_present=.true.)
      CALL ncio_read_vector(file, 'a_trc_transp_src', size(a_trc_transp_src,1), landpatch, a_trc_transp_src, known_present=.true.)
      CALL ncio_read_vector(file, 'a_water_transp', size(a_water_transp,1), landpatch, a_water_transp, known_present=.true.)
      CALL ncio_read_vector(file, 'a_trc_soilevap', size(a_trc_soilevap,1), landpatch, a_trc_soilevap, known_present=.true.)
      CALL ncio_read_vector(file, 'a_water_soilevap', size(a_water_soilevap,1), landpatch, a_water_soilevap, known_present=.true.)
      CALL ncio_read_vector(file, 'a_trc_canopyevap', size(a_trc_canopyevap,1), landpatch, a_trc_canopyevap, known_present=.true.)
      CALL ncio_read_vector(file, 'a_water_canopyevap', size(a_water_canopyevap,1), landpatch, a_water_canopyevap, known_present=.true.)
      CALL ncio_read_vector(file, 'a_trc_subl', size(a_trc_subl,1), landpatch, a_trc_subl, known_present=.true.)
      CALL ncio_read_vector(file, 'a_water_subl', size(a_water_subl,1), landpatch, a_water_subl, known_present=.true.)
      CALL ncio_read_vector(file, 'a_trc_wetland_evap', size(a_trc_wetland_evap,1), landpatch, a_trc_wetland_evap, known_present=.true.)
      CALL ncio_read_vector(file, 'a_water_wetland_evap', size(a_water_wetland_evap,1), landpatch, a_water_wetland_evap, known_present=.true.)
      CALL ncio_read_vector(file, 'a_trc_rsur', size(a_trc_rsur,1), landpatch, a_trc_rsur, known_present=.true.)
      CALL ncio_read_vector(file, 'a_trc_rsub', size(a_trc_rsub,1), landpatch, a_trc_rsub, known_present=.true.)
      CALL ncio_read_vector(file, 'a_trc_rnof', size(a_trc_rnof,1), landpatch, a_trc_rnof, known_present=.true.)
      CALL ncio_read_vector(file, 'a_water_rnof', size(a_water_rnof,1), landpatch, a_water_rnof, known_present=.true.)
      CALL ncio_read_vector(file, 'a_trc_qinfl', size(a_trc_qinfl,1), landpatch, a_trc_qinfl, known_present=.true.)
      CALL ncio_read_vector(file, 'a_trc_qcharge', size(a_trc_qcharge,1), landpatch, a_trc_qcharge, known_present=.true.)
      CALL ncio_read_vector(file, 'a_trc_ldew_mass', size(a_trc_ldew_mass,1), landpatch, a_trc_ldew_mass, known_present=.true.)
      CALL ncio_read_vector(file, 'a_water_ldew', landpatch, a_water_ldew, known_present=.true.)
      CALL ncio_read_vector(file, 'a_trc_soil_mass', size(a_trc_soil_mass,1), size(a_trc_soil_mass,2), landpatch, a_trc_soil_mass)
      CALL ncio_read_vector(file, 'a_water_soil', size(a_water_soil,1), landpatch, a_water_soil, known_present=.true.)
      CALL ncio_read_vector(file, 'a_trc_snow_mass', size(a_trc_snow_mass,1), size(a_trc_snow_mass,2), landpatch, a_trc_snow_mass)
      CALL ncio_read_vector(file, 'a_water_snow', size(a_water_snow,1), landpatch, a_water_snow, known_present=.true.)
      CALL ncio_read_vector(file, 'a_trc_wa_mass', size(a_trc_wa_mass,1), landpatch, a_trc_wa_mass, known_present=.true.)
      CALL ncio_read_vector(file, 'a_water_wa', landpatch, a_water_wa, known_present=.true.)
      CALL ncio_read_vector(file, 'a_trc_wa_debt_mass', size(a_trc_wa_debt_mass,1), landpatch, a_trc_wa_debt_mass, known_present=.true.)
      CALL ncio_read_vector(file, 'a_water_wa_debt', landpatch, a_water_wa_debt, known_present=.true.)
      CALL ncio_read_vector(file, 'a_water_aquifer_actual', landpatch, a_water_aquifer_actual, known_present=.true.)
      CALL ncio_read_vector(file, 'a_trc_aquifer_actual_mass', size(a_trc_aquifer_actual_mass,1), landpatch, a_trc_aquifer_actual_mass, known_present=.true.)
      CALL ncio_read_vector(file, 'a_trc_wdsrf_mass', size(a_trc_wdsrf_mass,1), landpatch, a_trc_wdsrf_mass, known_present=.true.)
      CALL ncio_read_vector(file, 'a_water_wdsrf', landpatch, a_water_wdsrf, known_present=.true.)
      CALL ncio_read_vector(file, 'a_trc_wetwat_mass', size(a_trc_wetwat_mass,1), landpatch, a_trc_wetwat_mass, known_present=.true.)
      CALL ncio_read_vector(file, 'a_water_wetwat', landpatch, a_water_wetwat, known_present=.true.)
      CALL ncio_read_vector(file, 'a_trc_surface_residue_mass', size(a_trc_surface_residue_mass,1), landpatch, a_trc_surface_residue_mass, known_present=.true.)
      CALL ncio_read_vector(file, 'a_trc_subsurface_residue_mass', size(a_trc_subsurface_residue_mass,1), landpatch, a_trc_subsurface_residue_mass, known_present=.true.)
      CALL ncio_read_vector(file, 'a_trc_layer_dry_mass', size(a_trc_layer_dry_mass,1), landpatch, a_trc_layer_dry_mass, known_present=.true.)
      CALL ncio_read_vector(file, 'a_trc_solid_mass', size(a_trc_solid_mass,1), landpatch, a_trc_solid_mass, known_present=.true.)
      CALL ncio_read_vector(file, 'a_trc_scv_mass', size(a_trc_scv_mass,1), landpatch, a_trc_scv_mass, known_present=.true.)
      CALL ncio_read_vector(file, 'a_water_scv', landpatch, a_water_scv, known_present=.true.)
      bad=0
      IF (p_is_worker) THEN
         IF (any(.not.ieee_is_finite(a_trc_precip))) bad=1
         IF (any(.not.ieee_is_finite(a_trc_vapor_exchange))) bad=1
         IF (any(.not.ieee_is_finite(a_water_precip))) bad=1
         IF (any(.not.ieee_is_finite(a_trc_evap))) bad=1
         IF (any(.not.ieee_is_finite(a_water_evap_gross))) bad=1
         IF (any(.not.ieee_is_finite(a_trc_transp))) bad=1
         IF (any(.not.ieee_is_finite(a_trc_transp_src))) bad=1
         IF (any(.not.ieee_is_finite(a_water_transp))) bad=1
         IF (any(.not.ieee_is_finite(a_trc_soilevap))) bad=1
         IF (any(.not.ieee_is_finite(a_water_soilevap))) bad=1
         IF (any(.not.ieee_is_finite(a_trc_canopyevap))) bad=1
         IF (any(.not.ieee_is_finite(a_water_canopyevap))) bad=1
         IF (any(.not.ieee_is_finite(a_trc_subl))) bad=1
         IF (any(.not.ieee_is_finite(a_water_subl))) bad=1
         IF (any(.not.ieee_is_finite(a_trc_wetland_evap))) bad=1
         IF (any(.not.ieee_is_finite(a_water_wetland_evap))) bad=1
         IF (any(.not.ieee_is_finite(a_trc_rsur))) bad=1
         IF (any(.not.ieee_is_finite(a_trc_rsub))) bad=1
         IF (any(.not.ieee_is_finite(a_trc_rnof))) bad=1
         IF (any(.not.ieee_is_finite(a_water_rnof))) bad=1
         IF (any(.not.ieee_is_finite(a_trc_qinfl))) bad=1
         IF (any(.not.ieee_is_finite(a_trc_qcharge))) bad=1
         IF (any(.not.ieee_is_finite(a_trc_ldew_mass))) bad=1
         IF (any(.not.ieee_is_finite(a_water_ldew))) bad=1
         IF (any(.not.ieee_is_finite(a_trc_soil_mass))) bad=1
         IF (any(.not.ieee_is_finite(a_water_soil))) bad=1
         IF (any(.not.ieee_is_finite(a_trc_snow_mass))) bad=1
         IF (any(.not.ieee_is_finite(a_water_snow))) bad=1
         IF (any(.not.ieee_is_finite(a_trc_wa_mass))) bad=1
         IF (any(.not.ieee_is_finite(a_water_wa))) bad=1
         IF (any(.not.ieee_is_finite(a_trc_wa_debt_mass))) bad=1
         IF (any(.not.ieee_is_finite(a_water_wa_debt))) bad=1
         IF (any(.not.ieee_is_finite(a_water_aquifer_actual))) bad=1
         IF (any(.not.ieee_is_finite(a_trc_aquifer_actual_mass))) bad=1
         IF (any(.not.ieee_is_finite(a_trc_wdsrf_mass))) bad=1
         IF (any(.not.ieee_is_finite(a_water_wdsrf))) bad=1
         IF (any(.not.ieee_is_finite(a_trc_wetwat_mass))) bad=1
         IF (any(.not.ieee_is_finite(a_water_wetwat))) bad=1
         IF (any(.not.ieee_is_finite(a_trc_surface_residue_mass))) bad=1
         IF (any(.not.ieee_is_finite(a_trc_subsurface_residue_mass))) bad=1
         IF (any(.not.ieee_is_finite(a_trc_layer_dry_mass))) bad=1
         IF (any(.not.ieee_is_finite(a_trc_solid_mass))) bad=1
         IF (any(.not.ieee_is_finite(a_trc_scv_mass))) bad=1
         IF (any(.not.ieee_is_finite(a_water_scv))) bad=1
      ENDIF
#ifdef USEMPI
      CALL mpi_allreduce(MPI_IN_PLACE, bad, 1, MPI_INTEGER, MPI_MAX, p_comm_glb, p_err)
#endif
      IF (bad/=0) CALL CoLM_stop("non-finite generic land-tracer history accumulator")
   END SUBROUTINE tracer_history_restore
