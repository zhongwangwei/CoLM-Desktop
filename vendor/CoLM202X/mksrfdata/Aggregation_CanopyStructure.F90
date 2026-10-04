#include <define.h>

SUBROUTINE Aggregation_CanopyStructure (gland, dir_rawdata, dir_model_landdata, lc_year)

   USE MOD_Precision
   USE MOD_Namelist
   USE MOD_SPMD_Task
   USE MOD_Grid
   USE MOD_LandPatch
   USE MOD_Land2mWMO
   USE MOD_NetCDFVector
   USE MOD_NetCDFBlock
   USE MOD_AggregationRequestData
   USE MOD_Const_LC
   USE MOD_5x5DataReadin
   USE MOD_LandPFT
   USE MOD_Filesystem, ONLY: make_directory

   IMPLICIT NONE

   integer, intent(in) :: lc_year
   type(grid_type), intent(in) :: gland
   character(len=*), intent(in) :: dir_rawdata, dir_model_landdata

   character(len=256) :: landdir, lndname, dir_5x5, suffix, cyear
   type(block_data_real8_2d) :: ncd_grid, ncw_grid, bcw_grid
   real(r8), allocatable :: ncd_patch(:), ncw_patch(:), bcw_patch(:)
   real(r8), allocatable :: ncd_one(:), ncw_one(:), bcw_one(:), area_one(:)
   integer :: ipatch
   real(r8) :: sumarea
   logical :: raw_exists

   type(block_data_real8_3d) :: pftPCT
   real(r8), allocatable :: ncd_pft(:), ncw_pft(:), bcw_pft(:), pct_one(:,:)
   integer :: ip, p, wmo_src

   write(cyear,'(i4.4)') lc_year
   landdir = trim(dir_model_landdata) // '/cstructure/' // trim(cyear)
   dir_5x5 = trim(dir_rawdata) // '/canopy_data'
   suffix = 'CanopyStructure_500m_CH90_aggregated'

   inquire(file=trim(dir_5x5), exist=raw_exists)
   IF (.not. raw_exists) THEN
      IF (p_is_master) write(*,'(A)') 'Warning: canopy_data not found; CoLM2024 will use legacy canopy capacity.'
      RETURN
   ENDIF

#ifdef USEMPI
   CALL mpi_barrier (p_comm_glb, p_err)
#endif
   IF (p_is_master) THEN
      write(*,'(/,A)') 'Aggregate canopy structure for CoLM2024 interception ...'
      CALL make_directory(trim(adjustl(landdir)))
   ENDIF
#ifdef USEMPI
   CALL mpi_barrier (p_comm_glb, p_err)
#endif

! LULC_IGBP_PFT/PC are runtime subgrids (DEF_USE_PFT/PC) on an IGBP/USGS kernel.
#if (defined LULC_USGS || defined LULC_IGBP)
   IF (p_is_io) THEN
      CALL allocate_block_data (gland, ncd_grid)
      CALL allocate_block_data (gland, ncw_grid)
      CALL allocate_block_data (gland, bcw_grid)
      IF ((DEF_USE_PFT .or. DEF_USE_PC)) THEN
      CALL allocate_block_data (gland, pftPCT, N_PFT_modis, lb1=0)
      ENDIF

      CALL read_5x5_data (dir_5x5, suffix, gland, 'NEEDLELEAF_CROWN_DEPTH', ncd_grid)
      CALL read_5x5_data (dir_5x5, suffix, gland, 'NEEDLELEAF_CROWN_WIDTH', ncw_grid)
      CALL read_5x5_data (dir_5x5, suffix, gland, 'BROADLEAF_CROWN_WIDTH', bcw_grid)
      IF ((DEF_USE_PFT .or. DEF_USE_PC)) THEN
      CALL read_5x5_data_pft (trim(dir_rawdata)//'/plant_15s', 'MOD'//trim(cyear), &
                              gland, 'PCT_PFT', pftPCT)
      ENDIF
#ifdef USEMPI
      ! pftPCT is an optional input: pass it only when the PFT/PC subgrid is active.
      IF (DEF_USE_PFT .or. DEF_USE_PC) THEN
         CALL aggregation_data_daemon (gland, data_r8_2d_in1=ncd_grid, &
            data_r8_2d_in2=ncw_grid, data_r8_2d_in3=bcw_grid &
            , data_r8_3d_in1=pftPCT, n1_r8_3d_in1=16 &
            )
      ELSE
         CALL aggregation_data_daemon (gland, data_r8_2d_in1=ncd_grid, &
            data_r8_2d_in2=ncw_grid, data_r8_2d_in3=bcw_grid)
      ENDIF
#endif
   ENDIF

   IF (p_is_worker) THEN
      allocate(ncd_patch(numpatch), ncw_patch(numpatch), bcw_patch(numpatch))
      ncd_patch = -1.0e36_r8
      ncw_patch = -1.0e36_r8
      bcw_patch = -1.0e36_r8
      IF ((DEF_USE_PFT .or. DEF_USE_PC)) THEN
      allocate(ncd_pft(numpft), ncw_pft(numpft), bcw_pft(numpft))
      ncd_pft = -1.0e36_r8
      ncw_pft = -1.0e36_r8
      bcw_pft = -1.0e36_r8
      ENDIF

      DO ipatch = 1, numpatch
      IF ((DEF_USE_PFT .or. DEF_USE_PC)) THEN
         IF (ipatch == wmo_patch(landpatch%ielm(ipatch))) THEN
            wmo_src = wmo_source(landpatch%ielm(ipatch))
            ncd_patch(ipatch) = ncd_patch(wmo_src)
            ncw_patch(ipatch) = ncw_patch(wmo_src)
            bcw_patch(ipatch) = bcw_patch(wmo_src)
            ip = patch_pft_s(ipatch)
            ncd_pft(ip) = ncd_patch(ipatch)
            ncw_pft(ip) = ncw_patch(ipatch)
            bcw_pft(ip) = bcw_patch(ipatch)
            CYCLE
         ENDIF
      ENDIF

         IF (DEF_USE_PFT .or. DEF_USE_PC) THEN
            CALL aggregation_request_data (landpatch, ipatch, gland, zip=USE_zip_for_aggregation, &
               area=area_one, data_r8_2d_in1=ncd_grid, data_r8_2d_out1=ncd_one, &
               data_r8_2d_in2=ncw_grid, data_r8_2d_out2=ncw_one, &
               data_r8_2d_in3=bcw_grid, data_r8_2d_out3=bcw_one &
               , data_r8_3d_in1=pftPCT, data_r8_3d_out1=pct_one, &
               n1_r8_3d_in1=16, lb1_r8_3d_in1=0 &
               )
         ELSE
            CALL aggregation_request_data (landpatch, ipatch, gland, zip=USE_zip_for_aggregation, &
               area=area_one, data_r8_2d_in1=ncd_grid, data_r8_2d_out1=ncd_one, &
               data_r8_2d_in2=ncw_grid, data_r8_2d_out2=ncw_one, &
               data_r8_2d_in3=bcw_grid, data_r8_2d_out3=bcw_one)
         ENDIF

         sumarea = sum(area_one, mask=ncd_one > 0._r8 .and. ncd_one < 1000._r8)
         IF (sumarea > 0._r8) ncd_patch(ipatch) = &
            sum(ncd_one*area_one, mask=ncd_one > 0._r8 .and. ncd_one < 1000._r8) / sumarea
         sumarea = sum(area_one, mask=ncw_one > 0._r8 .and. ncw_one < 1000._r8)
         IF (sumarea > 0._r8) ncw_patch(ipatch) = &
            sum(ncw_one*area_one, mask=ncw_one > 0._r8 .and. ncw_one < 1000._r8) / sumarea
         sumarea = sum(area_one, mask=bcw_one > 0._r8 .and. bcw_one < 1000._r8)
         IF (sumarea > 0._r8) bcw_patch(ipatch) = &
            sum(bcw_one*area_one, mask=bcw_one > 0._r8 .and. bcw_one < 1000._r8) / sumarea

      IF (DEF_USE_PFT .or. DEF_USE_PC) THEN
         pct_one = max(pct_one, 0._r8)
#ifndef CROP
         IF (patchtypes(landpatch%settyp(ipatch)) == 0) THEN
#else
         IF (patchtypes(landpatch%settyp(ipatch)) == 0 .and. landpatch%settyp(ipatch) /= CROPLAND) THEN
#endif
            DO ip = patch_pft_s(ipatch), patch_pft_e(ipatch)
               p = landpft%settyp(ip)
               sumarea = sum(pct_one(p,:)*area_one, &
                  mask=pct_one(p,:) > 0._r8 .and. ncd_one > 0._r8 .and. ncd_one < 1000._r8)
               IF (sumarea > 0._r8) THEN
                  ncd_pft(ip) = sum(ncd_one*pct_one(p,:)*area_one, &
                     mask=ncd_one > 0._r8 .and. ncd_one < 1000._r8) / sumarea
               ELSE
                  ncd_pft(ip) = ncd_patch(ipatch)
               ENDIF
               sumarea = sum(pct_one(p,:)*area_one, &
                  mask=pct_one(p,:) > 0._r8 .and. ncw_one > 0._r8 .and. ncw_one < 1000._r8)
               IF (sumarea > 0._r8) THEN
                  ncw_pft(ip) = sum(ncw_one*pct_one(p,:)*area_one, &
                     mask=ncw_one > 0._r8 .and. ncw_one < 1000._r8) / sumarea
               ELSE
                  ncw_pft(ip) = ncw_patch(ipatch)
               ENDIF
               sumarea = sum(pct_one(p,:)*area_one, &
                  mask=pct_one(p,:) > 0._r8 .and. bcw_one > 0._r8 .and. bcw_one < 1000._r8)
               IF (sumarea > 0._r8) THEN
                  bcw_pft(ip) = sum(bcw_one*pct_one(p,:)*area_one, &
                     mask=bcw_one > 0._r8 .and. bcw_one < 1000._r8) / sumarea
               ELSE
                  bcw_pft(ip) = bcw_patch(ipatch)
               ENDIF
            ENDDO
#ifdef CROP
         ELSEIF (landpatch%settyp(ipatch) == CROPLAND) THEN
            ip = patch_pft_s(ipatch)
            ncd_pft(ip) = ncd_patch(ipatch)
            ncw_pft(ip) = ncw_patch(ipatch)
            bcw_pft(ip) = bcw_patch(ipatch)
#endif
         ENDIF
      ENDIF
      ENDDO

#ifdef USEMPI
      CALL aggregation_worker_done ()
#endif
   ENDIF

#ifdef USEMPI
   CALL mpi_barrier (p_comm_glb, p_err)
#endif

   lndname = trim(landdir)//'/ncd_patches.nc'
   CALL ncio_create_file_vector (lndname, landpatch)
   CALL ncio_define_dimension_vector (lndname, landpatch, 'patch')
   CALL ncio_write_vector (lndname, 'ncd_patches', 'patch', landpatch, ncd_patch, DEF_Srfdata_CompressLevel)
   lndname = trim(landdir)//'/ncw_patches.nc'
   CALL ncio_create_file_vector (lndname, landpatch)
   CALL ncio_define_dimension_vector (lndname, landpatch, 'patch')
   CALL ncio_write_vector (lndname, 'ncw_patches', 'patch', landpatch, ncw_patch, DEF_Srfdata_CompressLevel)
   lndname = trim(landdir)//'/bcw_patches.nc'
   CALL ncio_create_file_vector (lndname, landpatch)
   CALL ncio_define_dimension_vector (lndname, landpatch, 'patch')
   CALL ncio_write_vector (lndname, 'bcw_patches', 'patch', landpatch, bcw_patch, DEF_Srfdata_CompressLevel)

      IF ((DEF_USE_PFT .or. DEF_USE_PC)) THEN
   lndname = trim(landdir)//'/ncd_pfts.nc'
   CALL ncio_create_file_vector (lndname, landpft)
   CALL ncio_define_dimension_vector (lndname, landpft, 'pft')
   CALL ncio_write_vector (lndname, 'ncd_pfts', 'pft', landpft, ncd_pft, DEF_Srfdata_CompressLevel)
   lndname = trim(landdir)//'/ncw_pfts.nc'
   CALL ncio_create_file_vector (lndname, landpft)
   CALL ncio_define_dimension_vector (lndname, landpft, 'pft')
   CALL ncio_write_vector (lndname, 'ncw_pfts', 'pft', landpft, ncw_pft, DEF_Srfdata_CompressLevel)
   lndname = trim(landdir)//'/bcw_pfts.nc'
   CALL ncio_create_file_vector (lndname, landpft)
   CALL ncio_define_dimension_vector (lndname, landpft, 'pft')
   CALL ncio_write_vector (lndname, 'bcw_pfts', 'pft', landpft, bcw_pft, DEF_Srfdata_CompressLevel)
      ENDIF

   IF (p_is_worker) THEN
      deallocate(ncd_patch, ncw_patch, bcw_patch)
      IF (allocated(ncd_one)) deallocate(ncd_one)
      IF (allocated(ncw_one)) deallocate(ncw_one)
      IF (allocated(bcw_one)) deallocate(bcw_one)
      IF (allocated(area_one)) deallocate(area_one)
      IF ((DEF_USE_PFT .or. DEF_USE_PC)) THEN
      deallocate(ncd_pft, ncw_pft, bcw_pft)
      IF (allocated(pct_one)) deallocate(pct_one)
      ENDIF
   ENDIF
#endif

END SUBROUTINE Aggregation_CanopyStructure
