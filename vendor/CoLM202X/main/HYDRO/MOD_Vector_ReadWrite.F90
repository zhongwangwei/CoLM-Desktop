#include <define.h>

MODULE MOD_Vector_ReadWrite
!-----------------------------------------------------------------------
! DESCRIPTION:
!
!    Read/Write data in vector form.
!
! Created by Shupeng Zhang, May 2023
!-----------------------------------------------------------------------

   PUBLIC :: vector_gather_and_write
   PUBLIC :: vector_gather_map2grid_and_write
   PUBLIC :: vector_read_and_scatter

CONTAINS

   ! -------
   SUBROUTINE vector_gather_to_master ( &
         vector, vlen, totalvlen, data_address, wdata)

   USE MOD_Precision
   USE MOD_SPMD_Task
   USE MOD_DataType
   USE MOD_Vars_Global, only: spval
   IMPLICIT NONE

   real(r8), intent(in) :: vector (:)
   integer,  intent(in) :: vlen
   integer,  intent(in) :: totalvlen

   type(pointer_int32_1d), intent(in) :: data_address (0:)

   real(r8), allocatable,  intent(inout) :: wdata (:)

   ! Local variables
   integer :: iwork, mesg(2), isrc, ndata
   integer, allocatable :: counts(:), disps(:)
   real(r8), allocatable :: rcache(:), sendbuf(:)

      IF (totalvlen <= 0) RETURN

      ! Cells no worker writes keep spval, so a truly uncovered grid point is
      ! visible as missing rather than whatever the allocator returned.
      IF (p_is_master) THEN
         allocate (wdata (totalvlen))
         wdata = spval
      ENDIF

#if defined(USEMPI) && defined(FLAT_SPMD)
      allocate (counts(0:p_np_glb-1), disps(0:p_np_glb-1))
      CALL mpi_allgather (vlen, 1, MPI_INTEGER, counts, 1, MPI_INTEGER, p_comm_glb, p_err)
      disps(0) = 0
      DO iwork = 1, p_np_glb-1
         disps(iwork) = disps(iwork-1) + counts(iwork-1)
      ENDDO

      allocate (sendbuf(max(1,vlen)))
      IF (vlen > 0) sendbuf(1:vlen) = vector
      IF (p_is_master) THEN
         allocate (rcache(max(1,totalvlen)))
      ELSE
         allocate (rcache(1))
      ENDIF
      CALL mpi_gatherv (sendbuf, vlen, MPI_REAL8, rcache, counts, disps, &
         MPI_REAL8, p_root, p_comm_glb, p_err)

      IF (p_is_master) THEN
         IF (sum(counts) /= totalvlen) &
            CALL CoLM_stop ('vector_gather_to_master: vector length mismatch')
         DO iwork = 0, p_np_glb-1
            ndata = counts(iwork)
            IF (ndata > 0) THEN
               IF (.not. allocated(data_address(iwork)%val)) &
                  CALL CoLM_stop ('vector_gather_to_master: missing data address')
               IF (size(data_address(iwork)%val) /= ndata) &
                  CALL CoLM_stop ('vector_gather_to_master: data address size mismatch')
               wdata(data_address(iwork)%val) = rcache(disps(iwork)+1:disps(iwork)+ndata)
            ENDIF
         ENDDO
      ENDIF
      deallocate (counts, disps, rcache, sendbuf)
#elif defined(USEMPI)
      CALL mpi_barrier (p_comm_glb, p_err)

      IF (p_is_worker) THEN
         mesg = (/p_iam_glb, vlen/)
         CALL mpi_send (mesg, 2, MPI_INTEGER, p_address_master, mpi_tag_mesg, p_comm_glb, p_err)
         IF (vlen > 0) THEN
            CALL mpi_send (vector, vlen, MPI_REAL8, &
               p_address_master, mpi_tag_data, p_comm_glb, p_err)
         ENDIF
      ENDIF

      IF (p_is_master) THEN
         DO iwork = 0, p_np_worker-1
            CALL mpi_recv (mesg, 2, MPI_INTEGER, MPI_ANY_SOURCE, &
               mpi_tag_mesg, p_comm_glb, p_stat, p_err)

            isrc  = mesg(1)
            ndata = mesg(2)
            IF (ndata > 0) THEN
               allocate(rcache (ndata))

               CALL mpi_recv (rcache, ndata, MPI_REAL8, isrc, &
                  mpi_tag_data, p_comm_glb, p_stat, p_err)

               wdata(data_address(p_itis_worker(isrc))%val) = rcache

               deallocate (rcache)
            ENDIF
         ENDDO
      ENDIF

      CALL mpi_barrier (p_comm_glb, p_err)
#else
      wdata(data_address(0)%val) = vector
#endif

   END SUBROUTINE vector_gather_to_master

   ! -------
   SUBROUTINE vector_gather_and_write ( vector, vlen, totalvlen, data_address, &
         fileout, varname, dimname, itime_in_file, longname, units)

   USE MOD_Precision
   USE MOD_SPMD_Task
   USE MOD_Namelist
   USE MOD_DataType
   USE MOD_NetCDFSerial
   USE MOD_Vars_Global, only: spval
   IMPLICIT NONE

   real(r8), intent(in) :: vector (:)
   integer,  intent(in) :: vlen
   integer,  intent(in) :: totalvlen

   type(pointer_int32_1d), intent(in) :: data_address (0:)

   character(len=*), intent(in) :: fileout
   character(len=*), intent(in) :: varname
   character(len=*), intent(in) :: dimname

   integer,          intent(in), optional :: itime_in_file
   character(len=*), intent(in), optional :: longname
   character(len=*), intent(in), optional :: units

   ! Local variables
   real(r8), allocatable :: wdata(:)
   logical :: write_attr


      ! [GridRiverLakeFlow] Nothing to write when the vector is empty on all
      ! processes (e.g. no reservoir or no bifurcation pathway in the domain);
      ! writing it would define a zero-length dimension, which netCDF takes as
      ! unlimited.  totalvlen is the same on all processes, so all return together.
      ! Results are unchanged for any non-empty vector.
      IF (totalvlen <= 0) RETURN

      CALL vector_gather_to_master (vector, vlen, totalvlen, data_address, wdata)

      IF (p_is_master) THEN

         IF (present(itime_in_file)) THEN
            CALL ncio_write_serial_time (fileout, varname, itime_in_file, wdata, &
               dimname, 'time', DEF_HIST_CompressLevel)
         ELSE
            CALL ncio_write_serial (fileout, varname, wdata, &
               dimname, DEF_REST_CompressLevel)
         ENDIF

         IF (present(itime_in_file)) THEN
            write_attr = itime_in_file <= 1
         ELSE
            write_attr = .true.
         ENDIF

         IF (write_attr) THEN
            CALL ncio_put_attr (fileout, varname, 'missing_value', spval)
            IF (present(longname)) CALL ncio_put_attr (fileout, varname, 'long_name', longname)
            IF (present(units   )) CALL ncio_put_attr (fileout, varname, 'units',     units   )
         ENDIF

         deallocate (wdata)

      ENDIF

   END SUBROUTINE vector_gather_and_write

   ! -------
   SUBROUTINE vector_gather_map2grid_and_write ( &
         vector,  vlen,    totalvlen, data_address, nlon, x_vec,   nlat, y_vec,   &
         fileout, varname, lon_name,  lat_name,     itime_in_file, longname, units)

   USE MOD_Precision
   USE MOD_SPMD_Task
   USE MOD_Namelist
   USE MOD_DataType
   USE MOD_NetCDFSerial
   USE MOD_Vars_Global, only: spval
   IMPLICIT NONE

   character(len=*), intent(in) :: fileout
   real(r8),         intent(in) :: vector (:)
   integer,          intent(in) :: vlen
   integer,          intent(in) :: totalvlen
   character(len=*), intent(in) :: varname
   character(len=*), intent(in) :: lon_name, lat_name

   type(pointer_int32_1d), intent(in) :: data_address (0:)

   integer, intent(in) :: nlon, x_vec (:)
   integer, intent(in) :: nlat, y_vec (:)

   integer,          intent(in), optional :: itime_in_file
   character(len=*), intent(in), optional :: longname
   character(len=*), intent(in), optional :: units

   ! Local variables
   integer :: i
   real(r8), allocatable :: wdata(:), wdata2d(:,:)
   logical :: write_attr

      ! [GridRiverLakeFlow] Nothing to write when the vector is empty on all
      ! processes (e.g. no reservoir or no bifurcation pathway in the domain);
      ! writing it would define a zero-length dimension, which netCDF takes as
      ! unlimited.  totalvlen is the same on all processes, so all return together.
      ! Results are unchanged for any non-empty vector.
      IF (totalvlen <= 0) RETURN

      CALL vector_gather_to_master (vector, vlen, totalvlen, data_address, wdata)

      IF (p_is_master) THEN

         allocate (wdata2d (nlon,nlat))
         wdata2d(:,:) = spval

         ! The output grid may be a window of the full grid (route history crops the unitcat
         ! file to the model domain); elements outside it are routed but not written.
         DO i = 1, totalvlen
            IF ((x_vec(i) >= 1) .and. (x_vec(i) <= nlon) .and. &
                (y_vec(i) >= 1) .and. (y_vec(i) <= nlat)) THEN
               wdata2d(x_vec(i),y_vec(i)) = wdata(i)
            ENDIF
         ENDDO

         IF (present(itime_in_file)) THEN
            CALL ncio_write_serial_time (fileout, varname, itime_in_file, wdata2d, &
               lon_name, lat_name, 'time', DEF_HIST_CompressLevel)
         ELSE
            CALL ncio_write_serial (fileout, varname, wdata2d, &
               lon_name, lat_name, DEF_REST_CompressLevel)
         ENDIF

         IF (present(itime_in_file)) THEN
            write_attr = itime_in_file == 1
         ELSE
            write_attr = .true.
         ENDIF

         IF (write_attr) THEN
            CALL ncio_put_attr (fileout, varname, 'missing_value', spval)
            IF (present(longname)) CALL ncio_put_attr (fileout, varname, 'long_name', longname)
            IF (present(units   )) CALL ncio_put_attr (fileout, varname, 'units',     units   )
         ENDIF

         deallocate (wdata  )
         deallocate (wdata2d)

      ENDIF

   END SUBROUTINE vector_gather_map2grid_and_write

   ! -----
   SUBROUTINE vector_read_and_scatter ( &
         filein, vector, vlen, varname, data_address)

   USE MOD_Precision
   USE MOD_SPMD_Task
   USE MOD_DataType
   USE MOD_NetCDFSerial
   IMPLICIT NONE

   character(len=*),       intent(in)    :: filein
   real(r8),  allocatable, intent(inout) :: vector (:)
   integer,                intent(in)    :: vlen
   character(len=*),       intent(in)    :: varname
   type(pointer_int32_1d), intent(in)    :: data_address (0:)

   ! Local variables
   integer :: iwork, ndata, expected_length
   integer, allocatable :: counts(:), disps(:)
   real(r8), allocatable :: rdata(:), rcache(:), recvbuf(:)

      IF (p_is_master) THEN
         CALL ncio_read_serial (filein, varname, rdata)

         ! The restart vector must describe exactly the global vectors that
         ! this run is about to scatter into: a length or index mismatch means
         ! the restart belongs to another network and would be read into the
         ! wrong unit catchments, so stop instead of scattering it.
         expected_length = 0
         DO iwork = lbound(data_address, 1), ubound(data_address, 1)
            IF (allocated(data_address(iwork)%val)) &
               expected_length = expected_length + size(data_address(iwork)%val)
         ENDDO
         IF (size(rdata) /= expected_length) THEN
            CALL CoLM_stop ('vector_read_and_scatter: restart vector length mismatch')
         ENDIF

         DO iwork = lbound(data_address, 1), ubound(data_address, 1)
            IF (allocated(data_address(iwork)%val)) THEN
               IF (any(data_address(iwork)%val < 1 .or. &
                       data_address(iwork)%val > size(rdata))) THEN
                  CALL CoLM_stop ('vector_read_and_scatter: restart vector address out of range')
               ENDIF
            ENDIF
         ENDDO
      ENDIF

#if defined(USEMPI) && defined(FLAT_SPMD)
      allocate (counts(0:p_np_glb-1), disps(0:p_np_glb-1))
      CALL mpi_allgather (vlen, 1, MPI_INTEGER, counts, 1, MPI_INTEGER, p_comm_glb, p_err)
      disps(0) = 0
      DO iwork = 1, p_np_glb-1
         disps(iwork) = disps(iwork-1) + counts(iwork-1)
      ENDDO

      IF (p_is_master) THEN
         allocate (rcache(max(1,size(rdata))))
         DO iwork = 0, p_np_glb-1
            ndata = counts(iwork)
            IF (ndata > 0) THEN
               IF (.not. allocated(data_address(iwork)%val)) &
                  CALL CoLM_stop ('vector_read_and_scatter: missing data address')
               IF (size(data_address(iwork)%val) /= ndata) &
                  CALL CoLM_stop ('vector_read_and_scatter: data address size mismatch')
               rcache(disps(iwork)+1:disps(iwork)+ndata) = rdata(data_address(iwork)%val)
            ENDIF
         ENDDO
      ELSE
         allocate (rcache(1))
      ENDIF
      allocate (recvbuf(max(1,vlen)))
      CALL mpi_scatterv (rcache, counts, disps, MPI_REAL8, recvbuf, vlen, &
         MPI_REAL8, p_root, p_comm_glb, p_err)
      IF (vlen > 0) THEN
         IF (.not. allocated(vector)) allocate(vector(vlen))
         vector = recvbuf(1:vlen)
      ENDIF
      deallocate (counts, disps, rcache, recvbuf)
#elif defined(USEMPI)
      CALL mpi_barrier (p_comm_glb, p_err)

      IF (p_is_master) THEN
         DO iwork = 0, p_np_worker-1
            IF (allocated(data_address(iwork)%val)) THEN

               ndata = size(data_address(iwork)%val)
               allocate(rcache (ndata))
               rcache = rdata(data_address(iwork)%val)

               CALL mpi_send (rcache, ndata, MPI_REAL8, &
                  p_address_worker(iwork), mpi_tag_data, p_comm_glb, p_err)

               deallocate (rcache)
            ENDIF
         ENDDO
      ENDIF

      IF (p_is_worker) THEN
         IF (vlen > 0) THEN
            IF (.not. allocated(vector)) allocate(vector(vlen))
            CALL mpi_recv (vector, vlen, MPI_REAL8, p_address_master, &
               mpi_tag_data, p_comm_glb, p_stat, p_err)
         ENDIF
      ENDIF

      CALL mpi_barrier (p_comm_glb, p_err)
#else
      IF (.not. allocated(vector)) allocate(vector(vlen))
      vector = rdata(data_address(0)%val)
#endif

      IF (p_is_master) deallocate(rdata)

   END SUBROUTINE vector_read_and_scatter

END MODULE MOD_Vector_ReadWrite
