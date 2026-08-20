#include <define.h>

#if (defined TRACER) && (defined BGC)
SUBROUTINE Aggregation_LakeSoilC ( &
      gland, dir_rawdata, dir_model_landdata, lc_year)

!-----------------------------------------------------------------------
! !DESCRIPTION:
!  Aggregate lake-sediment organic carbon for Methane lake production.
!
!  Required when the target domain contains lake patches:
!    1) <dir_rawdata>/lake_soilc.nc, or
!    2) <dir_rawdata>/soil/lake_soilc.nc
!  variable: lake_soilc(soil, lon, lat) [gC/m3]
!-----------------------------------------------------------------------

   USE MOD_Precision
   USE, INTRINSIC :: ieee_arithmetic, only: ieee_is_finite
   USE MOD_Vars_Global, only: nl_soil, spval
   USE MOD_Namelist
   USE MOD_SPMD_Task
   USE MOD_Grid
   USE MOD_LandPatch
   USE MOD_NetCDFVector
   USE MOD_NetCDFBlock
#ifdef RangeCheck
   USE MOD_RangeCheck
#endif
   USE MOD_AggregationRequestData

   IMPLICIT NONE

   integer, intent(in) :: lc_year
   type(grid_type),  intent(in) :: gland
   character(len=*), intent(in) :: dir_rawdata
   character(len=*), intent(in) :: dir_model_landdata

   character(len=256) :: landdir, lndname, cyear
   integer :: ipatch, L, j, lake_local, lake_global
   logical :: raw_exists
   real(r8) :: valid_area

   type(block_data_real8_3d) :: lake_soilc_grid
   real(r8), allocatable :: lake_soilc_patches(:,:), lake_soilc_one(:,:), area_one(:)
#ifdef SrfdataDiag
   ! No gridded diagnostic is written here because srfdata_map_and_write is
   ! scalar-patch oriented; history/restart expose the 3-D soil-by-patch field.
#endif

      write(cyear,'(i4.4)') lc_year
      landdir = trim(dir_model_landdata) // '/soil/' // trim(cyear)

#ifdef USEMPI
      CALL mpi_barrier (p_comm_glb, p_err)
#endif
      IF (p_is_master) THEN
         write(*,'(/, A)') 'Aggregate lake sediment organic carbon ...'
         CALL system('mkdir -p ' // trim(adjustl(landdir)))
      ENDIF
#ifdef USEMPI
      CALL mpi_barrier (p_comm_glb, p_err)
#endif

      lake_local = 0
      IF (p_is_worker) THEN
         DO ipatch = 1, numpatch
            IF (landpatch%settyp(ipatch) == WATERBODY) lake_local = lake_local + 1
         ENDDO
         allocate(lake_soilc_patches(nl_soil,numpatch))
         lake_soilc_patches(:,:) = 0._r8
      ENDIF
      lake_global = lake_local
#ifdef USEMPI
      CALL mpi_allreduce (lake_local, lake_global, 1, MPI_INTEGER, MPI_SUM, p_comm_glb, p_err)
#endif

      IF (lake_global > 0) THEN
         lndname = trim(dir_rawdata)//'/lake_soilc.nc'
         raw_exists = .false.
         IF (p_is_master) inquire(file=trim(lndname), exist=raw_exists)
#ifdef USEMPI
         CALL mpi_bcast (raw_exists, 1, MPI_LOGICAL, p_address_master, p_comm_glb, p_err)
#endif
         IF (.not. raw_exists) THEN
            lndname = trim(dir_rawdata)//'/soil/lake_soilc.nc'
            IF (p_is_master) inquire(file=trim(lndname), exist=raw_exists)
#ifdef USEMPI
            CALL mpi_bcast (raw_exists, 1, MPI_LOGICAL, p_address_master, p_comm_glb, p_err)
#endif
         ENDIF

         IF (.not. raw_exists) THEN
            CALL CoLM_stop (' ***** ERROR: lake CH4 production requires raw lake_soilc.nc surface data.')
         ENDIF

         IF (p_is_master) write(*,*) '  using raw lake_soilc dataset: ', trim(lndname)
         IF (p_is_io) THEN
            CALL allocate_block_data (gland, lake_soilc_grid, nl_soil)
            CALL ncio_read_block (lndname, 'lake_soilc', gland, nl_soil, lake_soilc_grid)
#ifdef USEMPI
            CALL aggregation_data_daemon (gland, data_r8_3d_in1 = lake_soilc_grid, n1_r8_3d_in1 = nl_soil)
#endif
         ENDIF

         IF (p_is_worker) THEN
            DO ipatch = 1, numpatch
               L = landpatch%settyp(ipatch)
               IF (L == WATERBODY) THEN
                  CALL aggregation_request_data (landpatch, ipatch, gland, &
                     zip = USE_zip_for_aggregation, area = area_one, &
                     data_r8_3d_in1 = lake_soilc_grid, data_r8_3d_out1 = lake_soilc_one, &
                     n1_r8_3d_in1 = nl_soil, lb1_r8_3d_in1 = 1)

                  DO j = 1, nl_soil
                     valid_area = sum(area_one, mask = area_one > 0._r8 .and. &
                        ieee_is_finite(lake_soilc_one(j,:)) .and. lake_soilc_one(j,:) >= 0._r8 .and. &
                        abs(lake_soilc_one(j,:)) < 0.5_r8*abs(spval))
                     IF (valid_area > tiny(1._r8)) THEN
                        lake_soilc_patches(j,ipatch) = sum(lake_soilc_one(j,:)*area_one, &
                           mask = area_one > 0._r8 .and. ieee_is_finite(lake_soilc_one(j,:)) .and. &
                           lake_soilc_one(j,:) >= 0._r8 .and. &
                           abs(lake_soilc_one(j,:)) < 0.5_r8*abs(spval)) / valid_area
                     ENDIF
                  ENDDO
               ENDIF
            ENDDO

#ifdef USEMPI
            CALL aggregation_worker_done ()
#endif
         ENDIF
      ELSEIF (p_is_master) THEN
         write(*,'(A)') '  no lake patches in domain; writing zero lake sediment carbon.'
      ENDIF

#ifdef USEMPI
      CALL mpi_barrier (p_comm_glb, p_err)
#endif

#ifdef RangeCheck
      CALL check_vector_data ('lake_soilc_patches [gC/m3]', lake_soilc_patches)
#endif

      lndname = trim(landdir)//'/lake_soilc_patches.nc'
      CALL ncio_create_file_vector (lndname, landpatch)
      CALL ncio_define_dimension_vector (lndname, landpatch, 'soil', nl_soil)
      CALL ncio_define_dimension_vector (lndname, landpatch, 'patch')
      CALL ncio_write_vector (lndname, 'lake_soilc_patches', 'soil', nl_soil, 'patch', &
         landpatch, lake_soilc_patches, DEF_Srfdata_CompressLevel)

      IF (p_is_worker) THEN
         deallocate(lake_soilc_patches)
      ENDIF

END SUBROUTINE Aggregation_LakeSoilC
#endif
