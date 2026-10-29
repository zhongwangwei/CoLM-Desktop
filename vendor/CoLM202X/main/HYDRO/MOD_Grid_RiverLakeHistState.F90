#include <define.h>

#ifdef GridRiverLakeFlow
MODULE MOD_Grid_RiverLakeHistState
!--------------------------------------------------------------------------------
! DESCRIPTION:
!
!     State for GridRiverLake history accumulation.
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

END MODULE MOD_Grid_RiverLakeHistState
#endif
