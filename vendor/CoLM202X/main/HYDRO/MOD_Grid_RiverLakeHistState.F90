#include <define.h>

#ifdef GridRiverLakeFlow
MODULE MOD_Grid_RiverLakeHistState
!--------------------------------------------------------------------------------
! DESCRIPTION:
!
!     Restart-persistent state for GridRiverLake history accumulation.
!--------------------------------------------------------------------------------

   USE MOD_Precision
   USE MOD_DataType

   ! -- ACC Fluxes --
   real(r8), allocatable :: acctime_ucat     (:)

   real(r8), allocatable :: a_wdsrf_ucat     (:)
   real(r8), allocatable :: a_veloc_riv      (:)
   real(r8), allocatable :: a_discharge      (:)
   real(r8), allocatable :: a_floodarea      (:)  ! flooded area [m^2]
   real(r8), allocatable :: a_rivsto         (:)  ! river channel storage [m^3]
   real(r8), allocatable :: a_fldsto         (:)  ! visible floodplain storage [m^3]
   real(r8), allocatable :: a_flddph         (:)  ! visible floodplain depth [m]
   real(r8), allocatable :: a_storge         (:)  ! total storage (river+floodplain+levee) [m^3]
   real(r8), allocatable :: a_sfcelv         (:)  ! water surface elevation [m]
   real(r8), allocatable :: a_levsto         (:)  ! accumulated levee storage [m^3 * s]
   real(r8), allocatable :: a_levdph         (:)  ! accumulated levee depth   [m * s]
   real(r8), allocatable :: a_bifout         (:)  ! accumulated bifurcation outflow [m^3/s * s]
   real(r8), allocatable :: a_bifflw_lev     (:,:) ! accumulated pathway-layer bifurcation flow [m^3/s * s]
   real(r8), allocatable :: a_bifflw_acctime  (:)  ! accumulated pathway time [s]

   real(r8), allocatable :: a_wdsrf_ucat_pch (:)
   real(r8), allocatable :: a_veloc_riv_pch  (:)
   real(r8), allocatable :: a_discharge_pch  (:)
   real(r8), allocatable :: a_dis_rmth_pch   (:)
   real(r8), allocatable :: a_floodfrc_pch   (:)  ! flooded area [m^2]

   ! for reservoirs
   real(r8), allocatable :: acctime_resv     (:)

   real(r8), allocatable :: a_volresv        (:)  ! reservoir water volume [m^3]
   real(r8), allocatable :: a_qresv_in       (:)  ! inflow to reservoir    [m^3/s]
   real(r8), allocatable :: a_qresv_out      (:)  ! outflow from reservoir [m^3/s]

   ! grid information
   real(r8), allocatable :: lon_ucat (:)
   real(r8), allocatable :: lat_ucat (:)

   ! auxiliary data
   type(block_data_real8_2d) :: sumarea_ucat        ! 1) area covered by unit catchments
   logical,  allocatable     :: filter_ucat     (:)
   real(r8), allocatable     :: sum_grid_area   (:) !    sum area of patches inside one grid

   logical,  allocatable     :: filter_rivmth   (:) ! 2) area covered by river mouths
   real(r8), allocatable     :: sum_rmth_area   (:)

   logical,  allocatable     :: filter_inpm     (:) ! 3) area covered by input matrix
   type(block_data_real8_2d) :: sumarea_inpm

   type(block_data_real8_2d) :: allups_mask_grid    ! 4) mask of unit catchments with all
   real(r8), allocatable     :: allups_mask_pch (:) !    upstreams in simulation region

   PUBLIC :: read_gridriverlake_hist_restart
   PUBLIC :: write_gridriverlake_hist_restart

CONTAINS

   !-----------------------
   SUBROUTINE read_gridriverlake_hist_restart (file_restart)

   USE MOD_SPMD_Task
   USE MOD_Namelist, only: DEF_USE_LEVEE, DEF_USE_BIFURCATION, DEF_Reservoir_Method
   USE MOD_Vector_ReadWrite, only: vector_read_and_scatter, vector_read_matrix_and_scatter
   USE MOD_Grid_RiverLakeNetwork, only: numucat, totalnumucat, ucat_data_address, &
      npthlev_bif, npthout_local, totalnpthout, pth_global_id
   USE MOD_Grid_Reservoir, only: numresv, totalnumresv, resv_data_address
   IMPLICIT NONE

   character(len=*), intent(in) :: file_restart
   integer :: ncol_local_bif
   logical :: has_bif_signature
   integer, allocatable :: pth_global_id_bif(:)
   real(r8), allocatable :: bif_acctime_tmp(:,:)

      IF (.not. allocated(acctime_ucat)) RETURN

      IF (totalnumucat > 0) THEN
         IF (restart_var_exists(file_restart, 'hist_acctime_ucat')) &
            CALL vector_read_and_scatter(file_restart, acctime_ucat, numucat, 'hist_acctime_ucat', ucat_data_address)
         IF (restart_var_exists(file_restart, 'hist_wdsrf_ucat')) &
            CALL vector_read_and_scatter(file_restart, a_wdsrf_ucat, numucat, 'hist_wdsrf_ucat', ucat_data_address)
         IF (restart_var_exists(file_restart, 'hist_veloc_riv')) &
            CALL vector_read_and_scatter(file_restart, a_veloc_riv, numucat, 'hist_veloc_riv', ucat_data_address)
         IF (restart_var_exists(file_restart, 'hist_discharge')) &
            CALL vector_read_and_scatter(file_restart, a_discharge, numucat, 'hist_discharge', ucat_data_address)
         IF (restart_var_exists(file_restart, 'hist_floodarea')) &
            CALL vector_read_and_scatter(file_restart, a_floodarea, numucat, 'hist_floodarea', ucat_data_address)
         IF (restart_var_exists(file_restart, 'hist_rivsto')) &
            CALL vector_read_and_scatter(file_restart, a_rivsto, numucat, 'hist_rivsto', ucat_data_address)
         IF (restart_var_exists(file_restart, 'hist_fldsto')) &
            CALL vector_read_and_scatter(file_restart, a_fldsto, numucat, 'hist_fldsto', ucat_data_address)
         IF (restart_var_exists(file_restart, 'hist_flddph')) &
            CALL vector_read_and_scatter(file_restart, a_flddph, numucat, 'hist_flddph', ucat_data_address)
         IF (restart_var_exists(file_restart, 'hist_storge')) &
            CALL vector_read_and_scatter(file_restart, a_storge, numucat, 'hist_storge', ucat_data_address)
         IF (restart_var_exists(file_restart, 'hist_sfcelv')) &
            CALL vector_read_and_scatter(file_restart, a_sfcelv, numucat, 'hist_sfcelv', ucat_data_address)
         IF (DEF_USE_LEVEE .and. allocated(a_levsto)) THEN
            IF (restart_var_exists(file_restart, 'hist_levsto')) &
               CALL vector_read_and_scatter(file_restart, a_levsto, numucat, 'hist_levsto', ucat_data_address)
            IF (restart_var_exists(file_restart, 'hist_levdph')) &
               CALL vector_read_and_scatter(file_restart, a_levdph, numucat, 'hist_levdph', ucat_data_address)
         ENDIF
         IF (DEF_USE_BIFURCATION .and. allocated(a_bifout)) THEN
            IF (restart_var_exists(file_restart, 'hist_bifout')) &
               CALL vector_read_and_scatter(file_restart, a_bifout, numucat, 'hist_bifout', ucat_data_address)
         ENDIF
      ENDIF

      IF (DEF_Reservoir_Method > 0 .and. totalnumresv > 0 .and. allocated(acctime_resv)) THEN
         IF (restart_var_exists(file_restart, 'hist_acctime_resv')) &
            CALL vector_read_and_scatter(file_restart, acctime_resv, numresv, 'hist_acctime_resv', resv_data_address)
         IF (restart_var_exists(file_restart, 'hist_volresv')) &
            CALL vector_read_and_scatter(file_restart, a_volresv, numresv, 'hist_volresv', resv_data_address)
         IF (restart_var_exists(file_restart, 'hist_qresv_in')) &
            CALL vector_read_and_scatter(file_restart, a_qresv_in, numresv, 'hist_qresv_in', resv_data_address)
         IF (restart_var_exists(file_restart, 'hist_qresv_out')) &
            CALL vector_read_and_scatter(file_restart, a_qresv_out, numresv, 'hist_qresv_out', resv_data_address)
      ENDIF

      IF (DEF_USE_BIFURCATION .and. totalnpthout > 0 .and. npthlev_bif > 0 .and. &
          allocated(a_bifflw_lev) .and. allocated(a_bifflw_acctime)) THEN
         ! Path-history columns use the same ordinal IDs as BIF momentum.
         ! A legacy restart without identity metadata cannot prove that those
         ! ordinals still describe the current pathways, so keep the freshly
         ! zeroed accumulators rather than silently attaching history to a
         ! reordered network. The BIF state reader performs the full signature
         ! comparison later during flow initialization.
         has_bif_signature = restart_var_exists(file_restart, 'bif_path_signature')
         IF (has_bif_signature) THEN
            IF (p_is_worker) THEN
               ncol_local_bif = npthout_local
               allocate (pth_global_id_bif(ncol_local_bif))
               pth_global_id_bif(:) = pth_global_id(:)
            ELSE
               ncol_local_bif = 0
               allocate (pth_global_id_bif(0))
            ENDIF

            IF (restart_var_exists(file_restart, 'hist_bifflw_lev')) &
               CALL vector_read_matrix_and_scatter(file_restart, a_bifflw_lev, npthlev_bif, &
                  ncol_local_bif, 'hist_bifflw_lev', pth_global_id_bif, totalnpthout)
            IF (restart_var_exists(file_restart, 'hist_bifflw_acctime')) THEN
               allocate (bif_acctime_tmp(1, ncol_local_bif))
               bif_acctime_tmp = 0._r8
               CALL vector_read_matrix_and_scatter(file_restart, bif_acctime_tmp, 1, &
                  ncol_local_bif, 'hist_bifflw_acctime', pth_global_id_bif, totalnpthout)
               a_bifflw_acctime(:) = bif_acctime_tmp(1, :)
               deallocate (bif_acctime_tmp)
            ENDIF
            deallocate (pth_global_id_bif)
         ELSEIF (p_is_master) THEN
            write(*,'(A)') 'WARNING: legacy BIF restart has no path signature; cold-starting pathway history accumulators.'
         ENDIF
      ENDIF

   END SUBROUTINE read_gridriverlake_hist_restart

   !-----------------------
   SUBROUTINE write_gridriverlake_hist_restart (file_restart)

   USE MOD_SPMD_Task
   USE MOD_Namelist, only: DEF_USE_LEVEE, DEF_USE_BIFURCATION, DEF_Reservoir_Method, DEF_REST_CompressLevel
   USE MOD_NetCDFSerial, only: ncio_define_dimension, ncio_write_serial
   USE MOD_Vector_ReadWrite, only: vector_gather_and_write, vector_gather_matrix_to_master
   USE MOD_Grid_RiverLakeNetwork, only: numucat, totalnumucat, ucat_data_address, &
      npthlev_bif, npthout_local, totalnpthout, pth_global_id
   USE MOD_Grid_Reservoir, only: numresv, totalnumresv, resv_data_address
   IMPLICIT NONE

   character(len=*), intent(in) :: file_restart
   integer :: ncol_local_bif
   integer, allocatable :: pth_global_id_bif(:)
   real(r8), allocatable :: bif_acctime_tmp(:,:), wdata(:,:)

      IF (.not. allocated(acctime_ucat)) RETURN

      IF (totalnumucat > 0) THEN
         CALL vector_gather_and_write(acctime_ucat, numucat, totalnumucat, ucat_data_address, &
            file_restart, 'hist_acctime_ucat', 'ucatch')
         CALL vector_gather_and_write(a_wdsrf_ucat, numucat, totalnumucat, ucat_data_address, &
            file_restart, 'hist_wdsrf_ucat', 'ucatch')
         CALL vector_gather_and_write(a_veloc_riv, numucat, totalnumucat, ucat_data_address, &
            file_restart, 'hist_veloc_riv', 'ucatch')
         CALL vector_gather_and_write(a_discharge, numucat, totalnumucat, ucat_data_address, &
            file_restart, 'hist_discharge', 'ucatch')
         CALL vector_gather_and_write(a_floodarea, numucat, totalnumucat, ucat_data_address, &
            file_restart, 'hist_floodarea', 'ucatch')
         CALL vector_gather_and_write(a_rivsto, numucat, totalnumucat, ucat_data_address, &
            file_restart, 'hist_rivsto', 'ucatch')
         CALL vector_gather_and_write(a_fldsto, numucat, totalnumucat, ucat_data_address, &
            file_restart, 'hist_fldsto', 'ucatch')
         CALL vector_gather_and_write(a_flddph, numucat, totalnumucat, ucat_data_address, &
            file_restart, 'hist_flddph', 'ucatch')
         CALL vector_gather_and_write(a_storge, numucat, totalnumucat, ucat_data_address, &
            file_restart, 'hist_storge', 'ucatch')
         CALL vector_gather_and_write(a_sfcelv, numucat, totalnumucat, ucat_data_address, &
            file_restart, 'hist_sfcelv', 'ucatch')
         IF (DEF_USE_LEVEE .and. allocated(a_levsto)) THEN
            CALL vector_gather_and_write(a_levsto, numucat, totalnumucat, ucat_data_address, &
               file_restart, 'hist_levsto', 'ucatch')
            CALL vector_gather_and_write(a_levdph, numucat, totalnumucat, ucat_data_address, &
               file_restart, 'hist_levdph', 'ucatch')
         ENDIF
         IF (DEF_USE_BIFURCATION .and. allocated(a_bifout)) THEN
            CALL vector_gather_and_write(a_bifout, numucat, totalnumucat, ucat_data_address, &
               file_restart, 'hist_bifout', 'ucatch')
         ENDIF
      ENDIF

      IF (DEF_Reservoir_Method > 0 .and. totalnumresv > 0 .and. allocated(acctime_resv)) THEN
         CALL vector_gather_and_write(acctime_resv, numresv, totalnumresv, resv_data_address, &
            file_restart, 'hist_acctime_resv', 'reservoir')
         CALL vector_gather_and_write(a_volresv, numresv, totalnumresv, resv_data_address, &
            file_restart, 'hist_volresv', 'reservoir')
         CALL vector_gather_and_write(a_qresv_in, numresv, totalnumresv, resv_data_address, &
            file_restart, 'hist_qresv_in', 'reservoir')
         CALL vector_gather_and_write(a_qresv_out, numresv, totalnumresv, resv_data_address, &
            file_restart, 'hist_qresv_out', 'reservoir')
      ENDIF

      IF (DEF_USE_BIFURCATION .and. totalnpthout > 0 .and. npthlev_bif > 0 .and. &
          allocated(a_bifflw_lev) .and. allocated(a_bifflw_acctime)) THEN
         IF (p_is_worker) THEN
            ncol_local_bif = npthout_local
            allocate (pth_global_id_bif(ncol_local_bif))
            pth_global_id_bif(:) = pth_global_id(:)
         ELSE
            ncol_local_bif = 0
            allocate (pth_global_id_bif(0))
         ENDIF

         IF (p_is_master) THEN
            CALL ncio_define_dimension(file_restart, 'bifurcation_level', npthlev_bif)
            CALL ncio_define_dimension(file_restart, 'bifurcation_pathway', totalnpthout)
            CALL ncio_define_dimension(file_restart, 'bifurcation_history_scalar', 1)
         ENDIF

         CALL vector_gather_matrix_to_master(a_bifflw_lev, npthlev_bif, ncol_local_bif, &
            totalnpthout, pth_global_id_bif, wdata)
         IF (p_is_master) THEN
            CALL ncio_write_serial(file_restart, 'hist_bifflw_lev', wdata, &
               'bifurcation_level', 'bifurcation_pathway', DEF_REST_CompressLevel)
            deallocate (wdata)
         ENDIF

         allocate (bif_acctime_tmp(1, ncol_local_bif))
         bif_acctime_tmp(1, :) = a_bifflw_acctime(:)
         CALL vector_gather_matrix_to_master(bif_acctime_tmp, 1, ncol_local_bif, &
            totalnpthout, pth_global_id_bif, wdata)
         IF (p_is_master) THEN
            CALL ncio_write_serial(file_restart, 'hist_bifflw_acctime', wdata, &
               'bifurcation_history_scalar', 'bifurcation_pathway', DEF_REST_CompressLevel)
            deallocate (wdata)
         ENDIF
         deallocate (bif_acctime_tmp)
         deallocate (pth_global_id_bif)
      ENDIF

   END SUBROUTINE write_gridriverlake_hist_restart

   !-----------------------
   LOGICAL FUNCTION restart_var_exists (file_restart, varname)

   USE MOD_SPMD_Task
   USE MOD_NetCDFSerial, only: ncio_var_exist
   IMPLICIT NONE

   character(len=*), intent(in) :: file_restart
   character(len=*), intent(in) :: varname

      restart_var_exists = .false.
      IF (p_is_master) restart_var_exists = ncio_var_exist(file_restart, varname, readflag = .false.)
#ifdef USEMPI
      CALL mpi_bcast(restart_var_exists, 1, MPI_LOGICAL, p_address_master, p_comm_glb, p_err)
#endif

   END FUNCTION restart_var_exists

END MODULE MOD_Grid_RiverLakeHistState
#endif
