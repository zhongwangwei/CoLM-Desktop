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
   PUBLIC :: vector_gather_matrix_to_master
   PUBLIC :: vector_gather_map2grid_and_write
   PUBLIC :: vector_read_and_scatter
   PUBLIC :: vector_read_matrix_and_scatter

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
   real(r8), allocatable :: rcache(:)

      IF (totalvlen <= 0) RETURN

      IF (p_is_master) THEN
         allocate (wdata (totalvlen))
         wdata = spval
      ENDIF

#ifdef USEMPI
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
   SUBROUTINE vector_gather_matrix_to_master ( &
         matrix, nrow, ncol_local, ncol_global, global_id, wdata)

   USE MOD_Precision
   USE MOD_SPMD_Task
   IMPLICIT NONE

   real(r8), intent(in) :: matrix (:,:)
   integer,  intent(in) :: nrow
   integer,  intent(in) :: ncol_local
   integer,  intent(in) :: ncol_global
   integer,  intent(in) :: global_id (:)

   real(r8), allocatable, intent(inout) :: wdata (:,:)

   integer :: iwork, isrc, ndata, ipth, mesg(2)
   integer, allocatable :: icache(:)
   real(r8), allocatable :: rcache(:)

      IF (nrow <= 0 .or. ncol_global <= 0) RETURN

      ! Current use assumes dense global IDs in 1..ncol_global, with one
      ! column per global pathway ID across all workers.
      IF (size(global_id) /= ncol_local) THEN
         CALL CoLM_stop ('vector_gather_matrix_to_master: global_id size mismatch')
      ENDIF

      IF (p_is_master) THEN
         allocate (wdata (nrow, ncol_global))
         wdata(:,:) = 0._r8
      ENDIF

#ifdef USEMPI
      IF (p_is_worker) THEN
         mesg = (/p_iam_glb, ncol_local/)
         CALL mpi_send (mesg, 2, MPI_INTEGER, p_address_master, mpi_tag_mesg, p_comm_glb, p_err)
         IF (ncol_local > 0) THEN
            CALL mpi_send (global_id, ncol_local, MPI_INTEGER, &
               p_address_master, mpi_tag_mesg + 1, p_comm_glb, p_err)
            CALL mpi_send (matrix, nrow*ncol_local, MPI_REAL8, &
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
               allocate (icache (ndata))
               allocate (rcache (nrow*ndata))

               CALL mpi_recv (icache, ndata, MPI_INTEGER, isrc, &
                  mpi_tag_mesg + 1, p_comm_glb, p_stat, p_err)
               CALL mpi_recv (rcache, nrow*ndata, MPI_REAL8, isrc, &
                  mpi_tag_data, p_comm_glb, p_stat, p_err)

               DO ipth = 1, ndata
                  IF (icache(ipth) < 1 .or. icache(ipth) > ncol_global) THEN
                     CALL CoLM_stop ('vector_gather_matrix_to_master: global_id out of range')
                  ENDIF
                  wdata(:, icache(ipth)) = rcache((ipth-1)*nrow+1:ipth*nrow)
               ENDDO

               deallocate (icache)
               deallocate (rcache)
            ENDIF
         ENDDO
      ENDIF

      CALL mpi_barrier (p_comm_glb, p_err)
#else
      DO ipth = 1, ncol_local
         IF (global_id(ipth) < 1 .or. global_id(ipth) > ncol_global) THEN
            CALL CoLM_stop ('vector_gather_matrix_to_master: global_id out of range')
         ENDIF
         wdata(:, global_id(ipth)) = matrix(:, ipth)
      ENDDO
#endif

   END SUBROUTINE vector_gather_matrix_to_master

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

      IF (totalvlen <= 0) RETURN

      CALL vector_gather_to_master (vector, vlen, totalvlen, data_address, wdata)

      IF (p_is_master) THEN

         allocate (wdata2d (nlon,nlat))
         wdata2d(:,:) = spval

         DO i = 1, totalvlen
            wdata2d(x_vec(i),y_vec(i)) = wdata(i)
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
   real(r8), allocatable :: rdata(:), rcache(:)

      IF (p_is_master) THEN
         CALL ncio_read_serial (filein, varname, rdata)

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

#ifdef USEMPI
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

   ! -----
   SUBROUTINE vector_read_matrix_and_scatter ( &
         filein, matrix, nrow, ncol_local, varname, global_id, ncol_global)

   USE MOD_Precision
   USE MOD_SPMD_Task
   USE MOD_NetCDFSerial
   IMPLICIT NONE

   character(len=*),       intent(in)    :: filein
   real(r8),  allocatable, intent(inout) :: matrix (:,:)
   integer,                intent(in)    :: nrow
   integer,                intent(in)    :: ncol_local
   character(len=*),       intent(in)    :: varname
   integer,                intent(in)    :: global_id (:)
   integer,                intent(in)    :: ncol_global

   integer :: iwork, isrc, ndata, ipth, mesg(2)
   integer, allocatable :: icache(:)
   real(r8), allocatable :: rdata(:,:), rcache(:)

      IF (nrow <= 0 .or. ncol_global <= 0) RETURN

      IF (size(global_id) /= ncol_local) THEN
         CALL CoLM_stop ('vector_read_matrix_and_scatter: global_id size mismatch')
      ENDIF

      IF (p_is_master) THEN
         CALL ncio_read_serial (filein, varname, rdata)

         IF (size(rdata, 1) /= nrow .or. size(rdata, 2) /= ncol_global) THEN
            CALL CoLM_stop ('vector_read_matrix_and_scatter: restart matrix shape mismatch')
         ENDIF
      ENDIF

#ifdef USEMPI
      CALL mpi_barrier (p_comm_glb, p_err)

      IF (p_is_worker) THEN
         mesg = (/p_iam_glb, ncol_local/)
         CALL mpi_send (mesg, 2, MPI_INTEGER, p_address_master, mpi_tag_mesg, p_comm_glb, p_err)

         IF (ncol_local > 0) THEN
            CALL mpi_send (global_id, ncol_local, MPI_INTEGER, &
               p_address_master, mpi_tag_mesg + 1, p_comm_glb, p_err)

            IF (.not. allocated(matrix)) allocate (matrix(nrow, ncol_local))
            CALL mpi_recv (matrix, nrow*ncol_local, MPI_REAL8, p_address_master, &
               mpi_tag_data, p_comm_glb, p_stat, p_err)
         ENDIF
      ENDIF

      IF (p_is_master) THEN
         DO iwork = 0, p_np_worker-1
            CALL mpi_recv (mesg, 2, MPI_INTEGER, MPI_ANY_SOURCE, &
               mpi_tag_mesg, p_comm_glb, p_stat, p_err)

            isrc  = mesg(1)
            ndata = mesg(2)
            IF (ndata > 0) THEN
               allocate (icache (ndata))
               allocate (rcache (nrow*ndata))

               CALL mpi_recv (icache, ndata, MPI_INTEGER, isrc, &
                  mpi_tag_mesg + 1, p_comm_glb, p_stat, p_err)

               DO ipth = 1, ndata
                  IF (icache(ipth) < 1 .or. icache(ipth) > ncol_global) THEN
                     CALL CoLM_stop ('vector_read_matrix_and_scatter: global_id out of range')
                  ENDIF
                  rcache((ipth-1)*nrow+1:ipth*nrow) = rdata(:, icache(ipth))
               ENDDO

               CALL mpi_send (rcache, nrow*ndata, MPI_REAL8, isrc, &
                  mpi_tag_data, p_comm_glb, p_err)

               deallocate (icache)
               deallocate (rcache)
            ENDIF
         ENDDO
      ENDIF

      CALL mpi_barrier (p_comm_glb, p_err)
#else
      IF (.not. allocated(matrix)) allocate (matrix(nrow, ncol_local))
      DO ipth = 1, ncol_local
         IF (global_id(ipth) < 1 .or. global_id(ipth) > ncol_global) THEN
            CALL CoLM_stop ('vector_read_matrix_and_scatter: global_id out of range')
         ENDIF
         matrix(:, ipth) = rdata(:, global_id(ipth))
      ENDDO
#endif

      IF (p_is_master) deallocate (rdata)

   END SUBROUTINE vector_read_matrix_and_scatter

END MODULE MOD_Vector_ReadWrite
