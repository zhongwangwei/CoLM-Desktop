#include <define.h>

MODULE MOD_UnitCatchmentRegional

!-------------------------------------------------------------------------------------
! DESCRIPTION:
!
!    Cut the unit-catchment routing network down to the river systems that receive
!    runoff from the land domain (DEF_UnitCatchment_regional = .true.).
!
!    GridRiverLakeFlow routes every unit catchment of the network file, wet or dry.
!    For a regional domain most of a global network never receives water, so the
!    run pays for it without using it.  This step finds the runoff-input grid cells
!    that the land patches overlap, in the same way GridRiverLakeFlow does at run
!    time (build_worker_remapdata on landpatch and the input grid), and lets
!    MOD_UnitCatchmentSubset write the regional network to
!    <landdata>/riverlake/unitcatchment_regional.nc.  mkinidata and the model read
!    that file instead of DEF_UnitCatchment_file (see get_unitcatchment_file).
!
!    Called by mksrfdata once the land patches are built.
!-------------------------------------------------------------------------------------

   IMPLICIT NONE
   PRIVATE

   PUBLIC :: unitcatchment_regional_build

CONTAINS

   SUBROUTINE unitcatchment_regional_build ()

   USE MOD_Precision
   USE MOD_SPMD_Task
   USE MOD_Namelist
   USE MOD_Grid
   USE MOD_LandPatch, only: landpatch
   USE MOD_WorkerPushData
   USE MOD_UnitCatchmentSubset
   USE MOD_Filesystem, ONLY: make_directory
   IMPLICIT NONE

   type(grid_type)             :: gridro
   type(worker_remapdata_type) :: remap
   logical, allocatable :: touched(:,:)
   integer, allocatable :: ids(:)
   integer :: nlon, nlat, ng, iworker, i, ix, iy, nkeep, nsystem
   character(len=256) :: file_regional

      file_regional = regional_unitcatchment_file ()

      nlon = 0
      nlat = 0
      IF (p_is_master) THEN
         CALL unitcatchment_grid_size (trim(DEF_UnitCatchment_file), nlon, nlat)
      ENDIF
#ifdef USEMPI
      CALL mpi_bcast (nlon, 1, MPI_INTEGER, p_address_master, p_comm_glb, p_err)
      CALL mpi_bcast (nlat, 1, MPI_INTEGER, p_address_master, p_comm_glb, p_err)
#endif

      ! Runoff-input cells overlapped by the land patches: the cells GridRiverLakeFlow
      ! will pull runoff from.
      CALL gridro%define_by_ndims (nlon, nlat)
      CALL build_worker_remapdata (landpatch, gridro, remap)

      allocate (touched (nlon, nlat))
      touched = .false.

#ifdef USEMPI
! ==== FIX: in flat SPMD (one rank that is both master and worker) the master waited in
! ==== mpi_recv for a message from itself that the ELSEIF branch never sent: deadlock
! ==== (upstream-bugs #48). The master now reads its own remap directly.
      IF (p_is_master) THEN
         DO iworker = 0, p_np_worker-1
            IF (p_address_worker(iworker) == p_iam_glb) THEN
               DO i = 1, remap%num_grid
                  ix = mod(remap%ids_me(i)-1, nlon) + 1
                  iy = (remap%ids_me(i)-1) / nlon + 1
                  touched(ix,iy) = .true.
               ENDDO
               CYCLE
            ENDIF
            CALL mpi_recv (ng, 1, MPI_INTEGER, p_address_worker(iworker), mpi_tag_mesg, p_comm_glb, p_stat, p_err)
            IF (ng > 0) THEN
               allocate (ids (ng))
               CALL mpi_recv (ids, ng, MPI_INTEGER, p_address_worker(iworker), mpi_tag_data, p_comm_glb, p_stat, p_err)
               DO i = 1, ng
                  ix = mod(ids(i)-1, nlon) + 1
                  iy = (ids(i)-1) / nlon + 1
                  touched(ix,iy) = .true.
               ENDDO
               deallocate (ids)
            ENDIF
         ENDDO
      ELSEIF (p_is_worker) THEN
         ng = remap%num_grid
         CALL mpi_send (ng, 1, MPI_INTEGER, p_address_master, mpi_tag_mesg, p_comm_glb, p_err)
         IF (ng > 0) THEN
            CALL mpi_send (remap%ids_me, ng, MPI_INTEGER, p_address_master, mpi_tag_data, p_comm_glb, p_err)
         ENDIF
      ENDIF
#else
      ng = remap%num_grid
      DO i = 1, ng
         ix = mod(remap%ids_me(i)-1, nlon) + 1
         iy = (remap%ids_me(i)-1) / nlon + 1
         touched(ix,iy) = .true.
      ENDDO
#endif

      IF (p_is_master) THEN
         CALL make_directory(trim(DEF_dir_landdata) // '/riverlake')
         ! Close over bifurcation pathways only when bifurcation routing is on: otherwise the
         ! pathways are unused and the closure drags in unrelated river systems.
         CALL unitcatchment_subset_write (trim(DEF_UnitCatchment_file), trim(file_regional), nlon, nlat, &
            touched, DEF_USE_BIFURCATION, nkeep, nsystem)
         write(*,'(/,A,I0,A,I0,A)') ' Regional unit-catchment network: ', nkeep, ' unit catchments in ', &
            nsystem, ' river systems'
         write(*,'(2A)') '   written to ', trim(file_regional)
      ENDIF

      CALL worker_remapdata_free_mem (remap)
      CALL grid_free_mem (gridro)
      deallocate (touched)

#ifdef USEMPI
      CALL mpi_barrier (p_comm_glb, p_err)
#endif

   END SUBROUTINE unitcatchment_regional_build

END MODULE MOD_UnitCatchmentRegional
