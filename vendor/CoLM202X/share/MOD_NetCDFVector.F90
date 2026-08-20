#include <define.h>

!----------------------------------------------------------------------------------
! !DESCRIPTION:
!
!    High-level Subroutines to read and write variables in files with netCDF format.
!
!    CoLM read and write netCDF files mainly in three ways:
!    1. Serial: read and write data by a single process;
!    2. Vector: 1) read vector data by IO and scatter from IO to workers
!               2) gather from workers to IO and write vectors by IO
!    3. Block : read blocked data by IO
!               Notice: input file is a single file.
!
!    This MODULE CONTAINS subroutines of "2. Vector".
!
!    Two implementations can be used,
!    1) "MOD_NetCDFVectorBlk.F90":
!       A vector is saved in separated files, each associated with a block.
!       READ/WRITE are fast in this way and compression can be used.
!       However, there may be too many files, especially when blocks are small.
!       CHOOSE this implementation by "#undef VectorInOneFile" in include/define.h
!    2) "MOD_NetCDFVectorOne.F90":
!       A vector is saved in one file.
!       READ/WRITE may be slow in this way.
!       CHOOSE this implementation by "#define VectorInOneFile" in include/define.h
!
!  Created by Shupeng Zhang, May 2023
!----------------------------------------------------------------------------------

MODULE MOD_NetCDFVector


   USE MOD_DataType
   IMPLICIT NONE

   ! PUBLIC subroutines

   INTERFACE ncio_read_vector
      MODULE procedure ncio_read_vector_logical_1d
      MODULE procedure ncio_read_vector_int32_1d
      MODULE procedure ncio_read_vector_int64_1d
      MODULE procedure ncio_read_vector_real8_1d
      MODULE procedure ncio_read_vector_real8_2d
      MODULE procedure ncio_read_vector_real8_3d
      MODULE procedure ncio_read_vector_real8_4d
      MODULE procedure ncio_read_vector_real8_5d
   END INTERFACE ncio_read_vector

   ! Restart fields that are optional for backward compatibility still need
   ! to be transactionally complete when present.  Legacy/defaultable reads
   ! accept a variable absent from every block; a committed-schema scope makes
   ! the same field required.  Both modes reject a mixed hot/cold field.
   INTERFACE ncio_read_vector_complete
      MODULE procedure ncio_read_vector_complete_real8_1d
      MODULE procedure ncio_read_vector_complete_real8_2d
   END INTERFACE ncio_read_vector_complete

   PUBLIC :: ncio_read_vector_complete
   PUBLIC :: ncio_set_complete_require_present
   PUBLIC :: ncio_vector_group_presence
   PUBLIC :: ncio_vector_var_present

   ! A committed schema restart is not a legacy/defaultable read: every field
   ! requested through ncio_read_vector_complete must exist on every block.
   ! The methane restart callback scopes this flag around its transaction.
   logical, private, save :: complete_require_present = .false.

   PUBLIC :: ncio_create_file_vector
   PUBLIC :: ncio_define_dimension_vector

   INTERFACE ncio_write_vector
      MODULE procedure ncio_write_vector_logical_1d
      MODULE procedure ncio_write_vector_int32_1d
      MODULE procedure ncio_write_vector_int32_3d
      MODULE procedure ncio_write_vector_int64_1d
      MODULE procedure ncio_write_vector_real8_1d
      MODULE procedure ncio_write_vector_real8_2d
      MODULE procedure ncio_write_vector_real8_3d
      MODULE procedure ncio_write_vector_real8_4d
      MODULE procedure ncio_write_vector_real8_5d
   END INTERFACE ncio_write_vector

CONTAINS

   SUBROUTINE ncio_set_complete_require_present (require_present)

   IMPLICIT NONE
   logical, intent(in) :: require_present

      complete_require_present = require_present

   END SUBROUTINE ncio_set_complete_require_present

   !---------------------------------------------------------
   SUBROUTINE ncio_require_complete_vector_var (filename, dataname, pixelset, &
         expected_rank, allow_missing, all_present, expected_dim1)

   USE MOD_NetCDFSerial, only: ncio_var_exist, ncio_inquire_varsize
   USE MOD_SPMD_Task
   USE MOD_Block, only: get_filename_block
   USE MOD_Pixelset, only: pixelset_type
   IMPLICIT NONE

   character(len=*), intent(in) :: filename
   character(len=*), intent(in) :: dataname
   type(pixelset_type), intent(in) :: pixelset
   integer, intent(in) :: expected_rank
   logical, intent(in) :: allow_missing
   logical, intent(out) :: all_present
   integer, intent(in), optional :: expected_dim1

   integer :: iblkgrp, iblk, jblk
   integer :: counts(3)
   integer, allocatable :: varsize(:)
   character(len=256) :: fileblock
   logical :: block_shape_ok

      counts(:) = 0
      all_present = .false.

      ! The control master has no vector blocks and is intentionally outside
      ! the IO/worker group.  It still enters high-level restart callbacks;
      ! treating its empty singleton group as a missing required variable
      ! would stop a healthy MPI restart before the real IO ranks inspect it.
      IF (.not. (p_is_io .or. p_is_worker)) RETURN

      ! Only IO ranks inspect their assigned files.  The IO-wide reduction
      ! makes the result global across all block groups; the following group
      ! broadcast gives each group's workers the same decision before any
      ! scatter collective can be entered.
      IF (p_is_io) THEN
         counts(1) = pixelset%nblkgrp
         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)
            CALL get_filename_block(filename, iblk, jblk, fileblock)
            IF (ncio_var_exist(fileblock, dataname, readflag=.false.)) THEN
               counts(2) = counts(2) + 1
               CALL ncio_inquire_varsize(fileblock, dataname, varsize)

               block_shape_ok = .false.
               IF (allocated(varsize)) THEN
                  IF (size(varsize) == expected_rank) THEN
                     block_shape_ok = &
                        varsize(expected_rank) == pixelset%vecgs%vlen(iblk,jblk)
                     IF (block_shape_ok .and. present(expected_dim1)) &
                        block_shape_ok = varsize(1) == expected_dim1
                  ENDIF
                  deallocate(varsize)
               ENDIF
               IF (.not. block_shape_ok) counts(3) = counts(3) + 1
            ENDIF
         ENDDO
      ENDIF

#ifdef USEMPI
      IF (p_is_io) THEN
         CALL mpi_allreduce(MPI_IN_PLACE, counts, 3, MPI_INTEGER, MPI_SUM, p_comm_io, p_err)
      ENDIF
      CALL mpi_bcast(counts, 3, MPI_INTEGER, p_root, p_comm_group, p_err)
#endif

      IF (counts(2) > 0 .and. counts(2) < counts(1)) THEN
         IF (p_is_io .and. p_iam_io == p_root) WRITE(*,'(A)') &
            'ERROR: restart variable '//trim(dataname)//' is missing from some block files in '//trim(filename)//'.'
         CALL CoLM_stop()
      ENDIF
      IF (counts(2) == 0 .and. .not. allow_missing) THEN
         IF (p_is_io .and. p_iam_io == p_root) WRITE(*,'(A)') &
            'ERROR: required restart variable '//trim(dataname)//' is absent from all block files in '//trim(filename)//'.'
         CALL CoLM_stop()
      ENDIF
      IF (counts(3) > 0) THEN
         IF (p_is_io .and. p_iam_io == p_root) WRITE(*,'(A)') &
            'ERROR: restart variable '//trim(dataname)//' has an incompatible shape in '//trim(filename)//'.'
         CALL CoLM_stop()
      ENDIF
      all_present = counts(1) > 0 .and. counts(2) == counts(1)

   END SUBROUTINE ncio_require_complete_vector_var

   !---------------------------------------------------------
   ! Collective block-aware existence probe for vector files.  The unsuffixed
   ! base filename is not itself a NetCDF file when vector data are split by
   ! block, so restart compatibility checks must use this helper.  Mixed
   ! presence is corruption, not an optional legacy field.
   logical FUNCTION ncio_vector_var_present (filename, dataname, pixelset)

   USE MOD_NetCDFSerial, only: ncio_var_exist
   USE MOD_SPMD_Task
   USE MOD_Block, only: get_filename_block
   USE MOD_Pixelset, only: pixelset_type
   IMPLICIT NONE

   character(len=*), intent(in) :: filename
   character(len=*), intent(in) :: dataname
   type(pixelset_type), intent(in) :: pixelset

   integer :: iblkgrp, iblk, jblk
   integer :: counts(2), global_counts(2)
   character(len=256) :: fileblock

      counts(:) = 0
      IF (p_is_io) THEN
         counts(1) = pixelset%nblkgrp
         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)
            CALL get_filename_block(filename, iblk, jblk, fileblock)
            IF (ncio_var_exist(fileblock, dataname, readflag=.false.)) &
               counts(2) = counts(2) + 1
         ENDDO
      ENDIF

#ifdef USEMPI
      IF (p_is_io) THEN
         CALL mpi_allreduce(MPI_IN_PLACE, counts, 2, MPI_INTEGER, MPI_SUM, p_comm_io, p_err)
      ENDIF
      global_counts(:) = 0
      IF (p_is_io) THEN
         IF (p_iam_io == p_root) global_counts(:) = counts(:)
      ENDIF
      CALL mpi_allreduce(MPI_IN_PLACE, global_counts, 2, MPI_INTEGER, MPI_SUM, p_comm_glb, p_err)
#else
      global_counts(:) = counts(:)
#endif

      IF (global_counts(2) > 0 .and. global_counts(2) < global_counts(1)) THEN
         IF (p_is_master) WRITE(*,'(A)') &
            'ERROR: restart variable '//trim(dataname)//' is missing from some block files in '//trim(filename)//'.'
         CALL CoLM_stop()
      ENDIF
      ncio_vector_var_present = global_counts(1) > 0 .and. global_counts(2) == global_counts(1)

   END FUNCTION ncio_vector_var_present

   !---------------------------------------------------------
   ! Probe an optional feature group in one block scan and one pair of MPI
   ! reductions.  This is the group equivalent of ncio_vector_var_present:
   ! each returned flag is true only when the field exists on every block,
   ! and mixed per-block presence is corruption.  Batching avoids one global
   ! collective per feature field during restart initialization.
   SUBROUTINE ncio_vector_group_presence (filename, datanames, pixelset, present_flags)

   USE MOD_NetCDFSerial, only: ncio_var_exist
   USE MOD_SPMD_Task
   USE MOD_Block, only: get_filename_block
   USE MOD_Pixelset, only: pixelset_type
   IMPLICIT NONE

   character(len=*), intent(in) :: filename
   character(len=*), intent(in) :: datanames(:)
   type(pixelset_type), intent(in) :: pixelset
   logical, intent(out) :: present_flags(size(datanames))

   integer :: iblkgrp, iblk, jblk, ivar
   integer, allocatable :: counts(:), global_counts(:)
   character(len=256) :: fileblock

      allocate(counts(0:size(datanames)), global_counts(0:size(datanames)))
      counts(:) = 0
      IF (p_is_io) THEN
         counts(0) = pixelset%nblkgrp
         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)
            CALL get_filename_block(filename, iblk, jblk, fileblock)
            DO ivar = 1, size(datanames)
               IF (ncio_var_exist(fileblock, trim(datanames(ivar)), readflag=.false.)) &
                  counts(ivar) = counts(ivar) + 1
            ENDDO
         ENDDO
      ENDIF

#ifdef USEMPI
      IF (p_is_io) THEN
         CALL mpi_allreduce(MPI_IN_PLACE, counts, size(counts), MPI_INTEGER, MPI_SUM, p_comm_io, p_err)
      ENDIF
      global_counts(:) = 0
      IF (p_is_io) THEN
         IF (p_iam_io == p_root) global_counts(:) = counts(:)
      ENDIF
      CALL mpi_allreduce(MPI_IN_PLACE, global_counts, size(global_counts), &
         MPI_INTEGER, MPI_SUM, p_comm_glb, p_err)
#else
      global_counts(:) = counts(:)
#endif

      DO ivar = 1, size(datanames)
         IF (global_counts(ivar) > 0 .and. global_counts(ivar) < global_counts(0)) THEN
            IF (p_is_master) WRITE(*,'(A,A,A)') 'ERROR: restart feature-group field ', &
               trim(datanames(ivar)), ' is missing from some vector blocks.'
            CALL CoLM_stop()
         ENDIF
         present_flags(ivar) = global_counts(0) > 0 .and. &
            global_counts(ivar) == global_counts(0)
      ENDDO

      deallocate(counts, global_counts)
   END SUBROUTINE ncio_vector_group_presence

   !---------------------------------------------------------
   SUBROUTINE ncio_read_vector_complete_real8_1d (filename, dataname, pixelset, rdata, defval)

   USE MOD_Precision
   USE MOD_Pixelset, only: pixelset_type
   IMPLICIT NONE

   character(len=*), intent(in) :: filename
   character(len=*), intent(in) :: dataname
   type(pixelset_type), intent(in) :: pixelset
   real(r8), allocatable, intent(inout) :: rdata(:)
   real(r8), intent(in), optional :: defval
   logical :: field_present, allow_field_missing

      allow_field_missing = present(defval) .and. (.not. complete_require_present)
      CALL ncio_require_complete_vector_var(filename, dataname, pixelset, &
         expected_rank=1, allow_missing=allow_field_missing, all_present=field_present)
      IF (present(defval)) THEN
         CALL ncio_read_vector_real8_1d(filename, dataname, pixelset, rdata, &
            defval, known_present=field_present)
      ELSE
         CALL ncio_read_vector_real8_1d(filename, dataname, pixelset, rdata, &
            known_present=field_present)
      ENDIF

   END SUBROUTINE ncio_read_vector_complete_real8_1d

   !---------------------------------------------------------
   SUBROUTINE ncio_read_vector_complete_real8_2d (filename, dataname, ndim1, pixelset, rdata, defval)

   USE MOD_Precision
   USE MOD_Pixelset, only: pixelset_type
   IMPLICIT NONE

   character(len=*), intent(in) :: filename
   character(len=*), intent(in) :: dataname
   integer, intent(in) :: ndim1
   type(pixelset_type), intent(in) :: pixelset
   real(r8), allocatable, intent(inout) :: rdata(:,:)
   real(r8), intent(in), optional :: defval
   logical :: field_present, allow_field_missing

      allow_field_missing = present(defval) .and. (.not. complete_require_present)
      CALL ncio_require_complete_vector_var(filename, dataname, pixelset, &
         expected_rank=2, allow_missing=allow_field_missing, all_present=field_present, &
         expected_dim1=ndim1)
      IF (present(defval)) THEN
         CALL ncio_read_vector_real8_2d(filename, dataname, ndim1, pixelset, rdata, &
            defval, known_present=field_present)
      ELSE
         CALL ncio_read_vector_real8_2d(filename, dataname, ndim1, pixelset, rdata, &
            known_present=field_present)
      ENDIF

   END SUBROUTINE ncio_read_vector_complete_real8_2d

   !---------------------------------------------------------
   SUBROUTINE ncio_read_vector_int32_1d ( &
         filename, dataname, pixelset, rdata, defval)

   USE MOD_NetCDFSerial
   USE MOD_SPMD_Task
   USE MOD_Block
   USE MOD_Pixelset
   IMPLICIT NONE

   character(len=*), intent(in) :: filename
   character(len=*), intent(in) :: dataname
   type(pixelset_type), intent(in) :: pixelset

   integer, allocatable, intent(inout) :: rdata (:)
   integer, intent(in), optional :: defval

   ! Local variables
   integer :: iblkgrp, iblk, jblk, istt, iend
   character(len=256) :: fileblock
   integer, allocatable :: sbuff(:), rbuff(:)
   logical :: any_data_exists

      IF (p_is_worker) THEN
         IF ((pixelset%nset > 0) .and. (.not. allocated(rdata))) THEN
            allocate (rdata (pixelset%nset))
         ENDIF
      ENDIF

      any_data_exists = .false.

      IF (p_is_io) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            allocate (sbuff (pixelset%vecgs%vlen(iblk,jblk)))
            CALL get_filename_block (filename, iblk, jblk, fileblock)

            IF (ncio_var_exist(fileblock,dataname,readflag=.false.)) THEN
               CALL ncio_read_serial (fileblock, dataname, sbuff)
               any_data_exists = .true.
            ELSEIF (present(defval)) THEN
               sbuff(:) = defval
            ENDIF

#ifdef USEMPI
            CALL mpi_scatterv ( &
               sbuff, pixelset%vecgs%vcnt(:,iblk,jblk), &
               pixelset%vecgs%vdsp(:,iblk,jblk), MPI_INTEGER, &
               MPI_IN_PLACE, 0, MPI_INTEGER, &
               p_root, p_comm_group, p_err)
#else
            istt = pixelset%vecgs%vstt(iblk,jblk)
            iend = pixelset%vecgs%vend(iblk,jblk)
            rdata(istt:iend) = sbuff
#endif

            deallocate (sbuff)

         ENDDO

#ifdef USEMPI
         CALL mpi_allreduce (MPI_IN_PLACE, any_data_exists, 1, MPI_LOGICAL, MPI_LOR, p_comm_io, p_err)
#endif
         IF (.not. any_data_exists) THEN
            IF (p_iam_io == p_root) THEN
               IF (.not. present(defval)) THEN
                  write(*,*) 'Warning : restart data '//trim(dataname) &
                     //' in '//trim(filename)//' not found.'
                  CALL CoLM_stop ()
               ELSE
                  write(*,*) 'Warning : restart data '//trim(dataname) &
                     //' in '//trim(filename)//' not found, default value is used.'
               ENDIF
            ENDIF
         ENDIF
      ENDIF

#ifdef USEMPI
      IF (p_is_worker) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            IF (pixelset%vecgs%vlen(iblk,jblk) > 0) THEN
               allocate (rbuff (pixelset%vecgs%vlen(iblk,jblk)))
            ELSE
               allocate (rbuff(1))
            ENDIF

            CALL mpi_scatterv ( &
               MPI_INULL_P, MPI_INULL_P, MPI_INULL_P, MPI_INTEGER, & ! insignificant on workers
               rbuff, pixelset%vecgs%vlen(iblk,jblk), MPI_INTEGER, &
               p_root, p_comm_group, p_err)

            IF (pixelset%vecgs%vlen(iblk,jblk) > 0) THEN
               istt = pixelset%vecgs%vstt(iblk,jblk)
               iend = pixelset%vecgs%vend(iblk,jblk)
               rdata(istt:iend) = rbuff
            ENDIF

            IF (allocated(rbuff)) deallocate (rbuff)

         ENDDO

      ENDIF
#endif

   END SUBROUTINE ncio_read_vector_int32_1d

   !---------------------------------------------------------
   SUBROUTINE ncio_read_vector_int64_1d ( &
         filename, dataname, pixelset, rdata, defval)

   USE MOD_NetCDFSerial
   USE MOD_SPMD_Task
   USE MOD_Block
   USE MOD_Pixelset
   IMPLICIT NONE

   character(len=*), intent(in) :: filename
   character(len=*), intent(in) :: dataname
   type(pixelset_type), intent(in) :: pixelset

   integer*8, allocatable, intent(inout) :: rdata (:)
   integer, intent(in), optional :: defval

   ! Local variables
   integer :: iblkgrp, iblk, jblk, istt, iend
   character(len=256) :: fileblock
   integer*8, allocatable :: sbuff(:), rbuff(:)
   logical :: any_data_exists

      IF (p_is_worker) THEN
         IF ((pixelset%nset > 0) .and. (.not. allocated(rdata))) THEN
            allocate (rdata (pixelset%nset))
         ENDIF
      ENDIF

      any_data_exists = .false.

      IF (p_is_io) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            allocate (sbuff (pixelset%vecgs%vlen(iblk,jblk)))
            CALL get_filename_block (filename, iblk, jblk, fileblock)

            IF (ncio_var_exist(fileblock,dataname,readflag=.false.)) THEN
               CALL ncio_read_serial (fileblock, dataname, sbuff)
               any_data_exists = .true.
            ELSEIF (present(defval)) THEN
               sbuff(:) = defval
            ENDIF

#ifdef USEMPI
            CALL mpi_scatterv ( &
               sbuff, pixelset%vecgs%vcnt(:,iblk,jblk), &
               pixelset%vecgs%vdsp(:,iblk,jblk), MPI_INTEGER8, &
               MPI_IN_PLACE, 0, MPI_INTEGER8, &
               p_root, p_comm_group, p_err)
#else
            istt = pixelset%vecgs%vstt(iblk,jblk)
            iend = pixelset%vecgs%vend(iblk,jblk)
            rdata(istt:iend) = sbuff
#endif

            deallocate (sbuff)

         ENDDO

#ifdef USEMPI
         CALL mpi_allreduce (MPI_IN_PLACE, any_data_exists, 1, MPI_LOGICAL, MPI_LOR, p_comm_io, p_err)
#endif
         IF (.not. any_data_exists) THEN
            IF (p_iam_io == p_root) THEN
               IF (.not. present(defval)) THEN
                  write(*,*) 'Warning : restart data '//trim(dataname) &
                     //' in '//trim(filename)//' not found.'
                  CALL CoLM_stop ()
               ELSE
                  write(*,*) 'Warning : restart data '//trim(dataname) &
                     //' in '//trim(filename)//' not found, default value is used.'
               ENDIF
            ENDIF
         ENDIF
      ENDIF

#ifdef USEMPI
      IF (p_is_worker) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            IF (pixelset%vecgs%vlen(iblk,jblk) > 0) THEN
               allocate (rbuff (pixelset%vecgs%vlen(iblk,jblk)))
            ELSE
               allocate (rbuff(1))
            ENDIF

            CALL mpi_scatterv ( &
               MPI_INULL_P, MPI_INULL_P, MPI_INULL_P, MPI_INTEGER8, & ! insignificant on workers
               rbuff, pixelset%vecgs%vlen(iblk,jblk), MPI_INTEGER8, &
               p_root, p_comm_group, p_err)

            IF (pixelset%vecgs%vlen(iblk,jblk) > 0) THEN
               istt = pixelset%vecgs%vstt(iblk,jblk)
               iend = pixelset%vecgs%vend(iblk,jblk)
               rdata(istt:iend) = rbuff
            ENDIF

            IF (allocated(rbuff)) deallocate (rbuff)

         ENDDO

      ENDIF
#endif

   END SUBROUTINE ncio_read_vector_int64_1d

   !---------------------------------------------------------
   SUBROUTINE ncio_read_vector_logical_1d (filename, dataname, pixelset, rdata, &
         defval)

   USE MOD_NetCDFSerial
   USE MOD_SPMD_Task
   USE MOD_Block
   USE MOD_Pixelset
   IMPLICIT NONE

   character(len=*), intent(in) :: filename
   character(len=*), intent(in) :: dataname
   type(pixelset_type), intent(in) :: pixelset

   logical, allocatable, intent(inout) :: rdata (:)
   logical, intent(in), optional :: defval

   ! Local variables
   integer :: iblkgrp, iblk, jblk, istt, iend
   character(len=256) :: fileblock
   integer(1), allocatable :: sbuff(:), rbuff(:)
   logical :: any_data_exists

      IF (p_is_worker) THEN
         IF ((pixelset%nset > 0) .and. (.not. allocated(rdata))) THEN
            allocate (rdata (pixelset%nset))
         ENDIF
      ENDIF

      any_data_exists = .false.

      IF (p_is_io) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            allocate (sbuff (pixelset%vecgs%vlen(iblk,jblk)))
            CALL get_filename_block (filename, iblk, jblk, fileblock)

            IF (ncio_var_exist(fileblock,dataname,readflag=.false.)) THEN
               CALL ncio_read_serial (fileblock, dataname, sbuff)
               any_data_exists = .true.
            ELSEIF (present(defval)) THEN
               IF (defval) THEN
                  sbuff(:) = 1
               ELSE
                  sbuff(:) = 0
               ENDIF
            ENDIF

#ifdef USEMPI
            CALL mpi_scatterv ( &
               sbuff, pixelset%vecgs%vcnt(:,iblk,jblk), &
               pixelset%vecgs%vdsp(:,iblk,jblk), MPI_INTEGER1, &
               MPI_IN_PLACE, 0, MPI_INTEGER1, &
               p_root, p_comm_group, p_err)
#else
            istt = pixelset%vecgs%vstt(iblk,jblk)
            iend = pixelset%vecgs%vend(iblk,jblk)
            rdata(istt:iend) = (sbuff == 1)
#endif

            deallocate (sbuff)

         ENDDO

#ifdef USEMPI
         CALL mpi_allreduce (MPI_IN_PLACE, any_data_exists, 1, MPI_LOGICAL, MPI_LOR, p_comm_io, p_err)
#endif
         IF (.not. any_data_exists) THEN
            IF (p_iam_io == p_root) THEN
               IF (.not. present(defval)) THEN
                  write(*,*) 'Warning : restart data '//trim(dataname) &
                     //' in '//trim(filename)//' not found.'
                  CALL CoLM_stop ()
               ELSE
                  write(*,*) 'Warning : restart data '//trim(dataname) &
                     //' in '//trim(filename)//' not found, default value is used.'
               ENDIF
            ENDIF
         ENDIF
      ENDIF

#ifdef USEMPI
      IF (p_is_worker) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            IF (pixelset%vecgs%vlen(iblk,jblk) > 0) THEN
               allocate (rbuff (pixelset%vecgs%vlen(iblk,jblk)))
            ELSE
               allocate (rbuff(1))
            ENDIF

            CALL mpi_scatterv ( &
               MPI_INULL_P, MPI_INULL_P, MPI_INULL_P, MPI_INTEGER1, & ! insignificant on workers
               rbuff, pixelset%vecgs%vlen(iblk,jblk), MPI_INTEGER1, &
               p_root, p_comm_group, p_err)

            IF (pixelset%vecgs%vlen(iblk,jblk) > 0) THEN
               istt = pixelset%vecgs%vstt(iblk,jblk)
               iend = pixelset%vecgs%vend(iblk,jblk)
               rdata(istt:iend) = (rbuff == 1)
            ENDIF

            IF (allocated(rbuff)) deallocate (rbuff)

         ENDDO

      ENDIF
#endif

   END SUBROUTINE ncio_read_vector_logical_1d

   !---------------------------------------------------------
   SUBROUTINE ncio_read_vector_real8_1d (filename, dataname, pixelset, rdata, &
         defval, known_present)

   USE MOD_Precision
   USE MOD_NetCDFSerial
   USE MOD_SPMD_Task
   USE MOD_Block
   USE MOD_Pixelset
   IMPLICIT NONE

   character(len=*), intent(in) :: filename
   character(len=*), intent(in) :: dataname
   type(pixelset_type), intent(in) :: pixelset

   real(r8), allocatable, intent(inout) :: rdata (:)
   real(r8), intent(in), optional :: defval
   logical, intent(in), optional :: known_present

   ! Local variables
   integer :: iblkgrp, iblk, jblk, istt, iend
   character(len=256) :: fileblock
   real(r8), allocatable :: sbuff(:), rbuff(:)
   logical :: any_data_exists

      IF (p_is_worker) THEN
         IF ((pixelset%nset > 0) .and. (.not. allocated(rdata))) THEN
            allocate (rdata (pixelset%nset))
         ENDIF
      ENDIF

      any_data_exists = .false.
      IF (present(known_present)) any_data_exists = known_present

      IF (p_is_io) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            allocate (sbuff (pixelset%vecgs%vlen(iblk,jblk)))
            CALL get_filename_block (filename, iblk, jblk, fileblock)

            IF (present(known_present)) THEN
               IF (known_present) THEN
                  CALL ncio_read_serial (fileblock, dataname, sbuff)
               ELSEIF (present(defval)) THEN
                  sbuff(:) = defval
               ENDIF
            ELSEIF (ncio_var_exist(fileblock,dataname,readflag=.false.)) THEN
               CALL ncio_read_serial (fileblock, dataname, sbuff)
               any_data_exists = .true.
            ELSEIF (present(defval)) THEN
               sbuff(:) = defval
            ENDIF

#ifdef USEMPI
            CALL mpi_scatterv ( &
               sbuff, pixelset%vecgs%vcnt(:,iblk,jblk), &
               pixelset%vecgs%vdsp(:,iblk,jblk), MPI_REAL8, &
               MPI_IN_PLACE, 0, MPI_REAL8, &
               p_root, p_comm_group, p_err)
#else
            istt = pixelset%vecgs%vstt(iblk,jblk)
            iend = pixelset%vecgs%vend(iblk,jblk)
            rdata(istt:iend) = sbuff
#endif

            deallocate (sbuff)

         ENDDO

#ifdef USEMPI
         IF (.not. present(known_present)) THEN
            CALL mpi_allreduce (MPI_IN_PLACE, any_data_exists, 1, MPI_LOGICAL, MPI_LOR, p_comm_io, p_err)
         ENDIF
#endif
         IF (.not. any_data_exists) THEN
            IF (p_iam_io == p_root) THEN
               IF (.not. present(defval)) THEN
                  write(*,*) 'Warning : restart data '//trim(dataname) &
                     //' in '//trim(filename)//' not found.'
                  CALL CoLM_stop ()
               ELSE
                  write(*,*) 'Warning : restart data '//trim(dataname) &
                     //' in '//trim(filename)//' not found, default value is used.'
               ENDIF
            ENDIF
         ENDIF
      ENDIF

#ifdef USEMPI
      IF (p_is_worker) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            IF (pixelset%vecgs%vlen(iblk,jblk) > 0) THEN
               allocate (rbuff (pixelset%vecgs%vlen(iblk,jblk)))
            ELSE
               allocate (rbuff(1))
            ENDIF

            CALL mpi_scatterv ( &
               MPI_RNULL_P, MPI_INULL_P, MPI_INULL_P, MPI_REAL8, & ! insignificant on workers
               rbuff, pixelset%vecgs%vlen(iblk,jblk), MPI_REAL8, &
               p_root, p_comm_group, p_err)

            IF (pixelset%vecgs%vlen(iblk,jblk) > 0) THEN
               istt = pixelset%vecgs%vstt(iblk,jblk)
               iend = pixelset%vecgs%vend(iblk,jblk)
               rdata(istt:iend) = rbuff
            ENDIF

            IF (allocated(rbuff)) deallocate (rbuff)

         ENDDO

      ENDIF
#endif

   END SUBROUTINE ncio_read_vector_real8_1d

   !---------------------------------------------------------
   SUBROUTINE ncio_read_vector_real8_2d ( &
         filename, dataname, ndim1, pixelset, rdata, defval, known_present)

   USE MOD_Precision
   USE MOD_NetCDFSerial
   USE MOD_SPMD_Task
   USE MOD_Block
   USE MOD_Pixelset
   IMPLICIT NONE

   character(len=*), intent(in) :: filename
   character(len=*), intent(in) :: dataname
   integer, intent(in) :: ndim1
   type(pixelset_type), intent(in) :: pixelset

   real(r8), allocatable, intent(inout) :: rdata (:,:)
   real(r8), intent(in), optional :: defval
   logical, intent(in), optional :: known_present

   ! Local variables
   integer :: iblkgrp, iblk, jblk, istt, iend
   character(len=256) :: fileblock
   real(r8), allocatable :: sbuff(:,:), rbuff(:,:)
   logical :: any_data_exists

      IF (p_is_worker) THEN
         IF ((pixelset%nset > 0) .and. (.not. allocated(rdata))) THEN
            allocate (rdata (ndim1, pixelset%nset))
         ENDIF
      ENDIF

      any_data_exists = .false.
      IF (present(known_present)) any_data_exists = known_present

      IF (p_is_io) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            allocate (sbuff (ndim1, pixelset%vecgs%vlen(iblk,jblk)))
            CALL get_filename_block (filename, iblk, jblk, fileblock)

            IF (present(known_present)) THEN
               IF (known_present) THEN
                  CALL ncio_read_serial (fileblock, dataname, sbuff)
               ELSEIF (present(defval)) THEN
                  sbuff(:,:) = defval
               ENDIF
            ELSEIF (ncio_var_exist(fileblock,dataname,readflag=.false.)) THEN
               CALL ncio_read_serial (fileblock, dataname, sbuff)
               any_data_exists = .true.
            ELSEIF (present(defval)) THEN
               sbuff(:,:) = defval
            ENDIF

#ifdef USEMPI
            CALL mpi_scatterv ( &
               sbuff, ndim1 * pixelset%vecgs%vcnt(:,iblk,jblk), &
               ndim1 * pixelset%vecgs%vdsp(:,iblk,jblk), MPI_REAL8, &
               MPI_IN_PLACE, 0, MPI_REAL8, &
               p_root, p_comm_group, p_err)
#else
            istt = pixelset%vecgs%vstt(iblk,jblk)
            iend = pixelset%vecgs%vend(iblk,jblk)
            rdata(:,istt:iend) = sbuff
#endif

            deallocate (sbuff)

         ENDDO

#ifdef USEMPI
         IF (.not. present(known_present)) THEN
            CALL mpi_allreduce (MPI_IN_PLACE, any_data_exists, 1, MPI_LOGICAL, MPI_LOR, p_comm_io, p_err)
         ENDIF
#endif
         IF (.not. any_data_exists) THEN
            IF (p_iam_io == p_root) THEN
               IF (.not. present(defval)) THEN
                  write(*,*) 'Warning : restart data '//trim(dataname) &
                     //' in '//trim(filename)//' not found.'
                  CALL CoLM_stop ()
               ELSE
                  write(*,*) 'Warning : restart data '//trim(dataname) &
                     //' in '//trim(filename)//' not found, default value is used.'
               ENDIF
            ENDIF
         ENDIF
      ENDIF

#ifdef USEMPI
      IF (p_is_worker) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            IF (pixelset%vecgs%vlen(iblk,jblk) > 0) THEN
               allocate (rbuff (ndim1, pixelset%vecgs%vlen(iblk,jblk)))
            ELSE
               allocate (rbuff(1,1))
            ENDIF

            CALL mpi_scatterv ( &
               MPI_RNULL_P, MPI_INULL_P, MPI_INULL_P, MPI_REAL8, & ! insignificant on workers
               rbuff, ndim1 * pixelset%vecgs%vlen(iblk,jblk), MPI_REAL8, &
               p_root, p_comm_group, p_err)

            IF (pixelset%vecgs%vlen(iblk,jblk) > 0) THEN
               istt = pixelset%vecgs%vstt(iblk,jblk)
               iend = pixelset%vecgs%vend(iblk,jblk)
               rdata(:,istt:iend) = rbuff
            ENDIF

            IF (allocated(rbuff)) deallocate (rbuff)

         ENDDO

      ENDIF
#endif

   END SUBROUTINE ncio_read_vector_real8_2d

   !---------------------------------------------------------
   SUBROUTINE ncio_read_vector_real8_3d ( &
         filename, dataname, ndim1, ndim2, pixelset, rdata, defval)

   USE MOD_Precision
   USE MOD_NetCDFSerial
   USE MOD_SPMD_Task
   USE MOD_Block
   USE MOD_Pixelset
   IMPLICIT NONE

   character(len=*), intent(in) :: filename
   character(len=*), intent(in) :: dataname
   integer, intent(in) :: ndim1, ndim2
   type(pixelset_type), intent(in) :: pixelset

   real(r8), allocatable, intent(inout) :: rdata (:,:,:)
   real(r8), intent(in), optional :: defval

   ! Local variables
   integer :: iblkgrp, iblk, jblk, istt, iend
   character(len=256) :: fileblock
   real(r8), allocatable :: sbuff(:,:,:), rbuff(:,:,:)
   logical :: any_data_exists

      IF (p_is_worker) THEN
         IF ((pixelset%nset > 0) .and. (.not. allocated(rdata))) THEN
            allocate (rdata (ndim1,ndim2, pixelset%nset))
         ENDIF
      ENDIF

      any_data_exists = .false.

      IF (p_is_io) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            allocate (sbuff (ndim1,ndim2, pixelset%vecgs%vlen(iblk,jblk)))
            CALL get_filename_block (filename, iblk, jblk, fileblock)

            IF (ncio_var_exist(fileblock,dataname,readflag=.false.)) THEN
               CALL ncio_read_serial (fileblock, dataname, sbuff)
               any_data_exists = .true.
            ELSEIF (present(defval)) THEN
               sbuff(:,:,:) = defval
            ENDIF

#ifdef USEMPI
            CALL mpi_scatterv ( &
               sbuff, ndim1 * ndim2 * pixelset%vecgs%vcnt(:,iblk,jblk), &
               ndim1 * ndim2 * pixelset%vecgs%vdsp(:,iblk,jblk), MPI_REAL8, &
               MPI_IN_PLACE, 0, MPI_REAL8, &
               p_root, p_comm_group, p_err)
#else
            istt = pixelset%vecgs%vstt(iblk,jblk)
            iend = pixelset%vecgs%vend(iblk,jblk)
            rdata(:,:,istt:iend) = sbuff
#endif

            deallocate (sbuff)

         ENDDO

#ifdef USEMPI
         CALL mpi_allreduce (MPI_IN_PLACE, any_data_exists, 1, MPI_LOGICAL, MPI_LOR, p_comm_io, p_err)
#endif
         IF (.not. any_data_exists) THEN
            IF (p_iam_io == p_root) THEN
               IF (.not. present(defval)) THEN
                  write(*,*) 'Warning : restart data '//trim(dataname) &
                     //' in '//trim(filename)//' not found.'
                  CALL CoLM_stop ()
               ELSE
                  write(*,*) 'Warning : restart data '//trim(dataname) &
                     //' in '//trim(filename)//' not found, default value is used.'
               ENDIF
            ENDIF
         ENDIF
      ENDIF

#ifdef USEMPI
      IF (p_is_worker) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            IF (pixelset%vecgs%vlen(iblk,jblk) > 0) THEN
               allocate (rbuff (ndim1,ndim2, pixelset%vecgs%vlen(iblk,jblk)))
            ELSE
               allocate (rbuff(1,1,1))
            ENDIF

            CALL mpi_scatterv ( &
               MPI_RNULL_P, MPI_INULL_P, MPI_INULL_P, MPI_REAL8, & ! insignificant on workers
               rbuff, ndim1 * ndim2 * pixelset%vecgs%vlen(iblk,jblk), MPI_REAL8, &
               p_root, p_comm_group, p_err)

            IF (pixelset%vecgs%vlen(iblk,jblk) > 0) THEN
               istt = pixelset%vecgs%vstt(iblk,jblk)
               iend = pixelset%vecgs%vend(iblk,jblk)
               rdata(:,:,istt:iend) = rbuff
            ENDIF

            IF (allocated(rbuff)) deallocate (rbuff)

         ENDDO

      ENDIF
#endif

   END SUBROUTINE ncio_read_vector_real8_3d

   !---------------------------------------------------------
   SUBROUTINE ncio_read_vector_real8_4d ( &
         filename, dataname, ndim1, ndim2, ndim3, pixelset, rdata, defval)

   USE MOD_Precision
   USE MOD_NetCDFSerial
   USE MOD_SPMD_Task
   USE MOD_Block
   USE MOD_Pixelset
   IMPLICIT NONE

   character(len=*), intent(in) :: filename
   character(len=*), intent(in) :: dataname
   integer, intent(in) :: ndim1, ndim2, ndim3
   type(pixelset_type), intent(in) :: pixelset

   real(r8), allocatable, intent(inout) :: rdata (:,:,:,:)
   real(r8), intent(in), optional :: defval

   ! Local variables
   integer :: iblkgrp, iblk, jblk, istt, iend
   character(len=256) :: fileblock
   real(r8), allocatable :: sbuff(:,:,:,:), rbuff(:,:,:,:)
   logical :: any_data_exists

      IF (p_is_worker) THEN
         IF ((pixelset%nset > 0) .and. (.not. allocated(rdata))) THEN
            allocate (rdata (ndim1,ndim2,ndim3, pixelset%nset))
         ENDIF
      ENDIF

      any_data_exists = .false.

      IF (p_is_io) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            allocate (sbuff (ndim1,ndim2,ndim3, pixelset%vecgs%vlen(iblk,jblk)))
            CALL get_filename_block (filename, iblk, jblk, fileblock)

            IF (ncio_var_exist(fileblock,dataname,readflag=.false.)) THEN
               CALL ncio_read_serial (fileblock, dataname, sbuff)
               any_data_exists = .true.
            ELSEIF (present(defval)) THEN
               sbuff(:,:,:,:) = defval
            ENDIF

#ifdef USEMPI
            CALL mpi_scatterv ( &
               sbuff, ndim1 * ndim2 * ndim3 * pixelset%vecgs%vcnt(:,iblk,jblk), &
               ndim1 * ndim2 * ndim3 * pixelset%vecgs%vdsp(:,iblk,jblk), MPI_REAL8, &
               MPI_IN_PLACE, 0, MPI_REAL8, &
               p_root, p_comm_group, p_err)
#else
            istt = pixelset%vecgs%vstt(iblk,jblk)
            iend = pixelset%vecgs%vend(iblk,jblk)
            rdata(:,:,:,istt:iend) = sbuff
#endif

            deallocate (sbuff)

         ENDDO

#ifdef USEMPI
         CALL mpi_allreduce (MPI_IN_PLACE, any_data_exists, 1, MPI_LOGICAL, MPI_LOR, p_comm_io, p_err)
#endif
         IF (.not. any_data_exists) THEN
            IF (p_iam_io == p_root) THEN
               IF (.not. present(defval)) THEN
                  write(*,*) 'Warning : restart data '//trim(dataname) &
                     //' in '//trim(filename)//' not found.'
                  CALL CoLM_stop ()
               ELSE
                  write(*,*) 'Warning : restart data '//trim(dataname) &
                     //' in '//trim(filename)//' not found, default value is used.'
               ENDIF
            ENDIF
         ENDIF
      ENDIF

#ifdef USEMPI
      IF (p_is_worker) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            IF (pixelset%vecgs%vlen(iblk,jblk) > 0) THEN
               allocate (rbuff (ndim1,ndim2,ndim3, pixelset%vecgs%vlen(iblk,jblk)))
            ELSE
               allocate (rbuff(1,1,1,1))
            ENDIF

            CALL mpi_scatterv ( &
               MPI_RNULL_P, MPI_INULL_P, MPI_INULL_P, MPI_REAL8, & ! insignificant on workers
               rbuff, ndim1 * ndim2 * ndim3 * pixelset%vecgs%vlen(iblk,jblk), MPI_REAL8, &
               p_root, p_comm_group, p_err)

            IF (pixelset%vecgs%vlen(iblk,jblk) > 0) THEN
               istt = pixelset%vecgs%vstt(iblk,jblk)
               iend = pixelset%vecgs%vend(iblk,jblk)
               rdata(:,:,:,istt:iend) = rbuff
            ENDIF

            IF (allocated(rbuff)) deallocate (rbuff)

         ENDDO

      ENDIF
#endif

   END SUBROUTINE ncio_read_vector_real8_4d


   !---------------------------------------------------------
   SUBROUTINE ncio_read_vector_real8_5d ( &
         filename, dataname, ndim1, ndim2, ndim3, ndim4, pixelset, rdata, defval)

   USE MOD_Precision
   USE MOD_NetCDFSerial
   USE MOD_SPMD_Task
   USE MOD_Block
   USE MOD_Pixelset
   IMPLICIT NONE

   character(len=*), intent(in) :: filename
   character(len=*), intent(in) :: dataname
   integer, intent(in) :: ndim1, ndim2, ndim3, ndim4
   type(pixelset_type), intent(in) :: pixelset

   real(r8), allocatable, intent(inout) :: rdata (:,:,:,:,:)
   real(r8), intent(in), optional :: defval

   ! Local variables
   integer :: iblkgrp, iblk, jblk, istt, iend
   character(len=256) :: fileblock
   real(r8), allocatable :: sbuff(:,:,:,:,:), rbuff(:,:,:,:,:)
   logical :: any_data_exists

      IF (p_is_worker) THEN
         IF ((pixelset%nset > 0) .and. (.not. allocated(rdata))) THEN
            allocate (rdata (ndim1,ndim2,ndim3,ndim4, pixelset%nset))
         ENDIF
      ENDIF

      any_data_exists = .false.

      IF (p_is_io) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            allocate (sbuff (ndim1,ndim2,ndim3,ndim4, pixelset%vecgs%vlen(iblk,jblk)))
            CALL get_filename_block (filename, iblk, jblk, fileblock)

            IF (ncio_var_exist(fileblock,dataname,readflag=.false.)) THEN
               CALL ncio_read_serial (fileblock, dataname, sbuff)
               any_data_exists = .true.
            ELSEIF (present(defval)) THEN
               sbuff(:,:,:,:,:) = defval
            ENDIF

#ifdef USEMPI
            CALL mpi_scatterv ( &
               sbuff, ndim1 * ndim2 * ndim3 * ndim4 * pixelset%vecgs%vcnt(:,iblk,jblk), &
               ndim1 * ndim2 * ndim3 * ndim4 * pixelset%vecgs%vdsp(:,iblk,jblk), MPI_REAL8, &
               MPI_IN_PLACE, 0, MPI_REAL8, &
               p_root, p_comm_group, p_err)
#else
            istt = pixelset%vecgs%vstt(iblk,jblk)
            iend = pixelset%vecgs%vend(iblk,jblk)
            rdata(:,:,:,:,istt:iend) = sbuff
#endif

            deallocate (sbuff)

         ENDDO

#ifdef USEMPI
         CALL mpi_allreduce (MPI_IN_PLACE, any_data_exists, 1, MPI_LOGICAL, MPI_LOR, p_comm_io, p_err)
#endif
         IF (.not. any_data_exists) THEN
            IF (p_iam_io == p_root) THEN
               IF (.not. present(defval)) THEN
                  write(*,*) 'Warning : restart data '//trim(dataname) &
                     //' in '//trim(filename)//' not found.'
                  CALL CoLM_stop ()
               ELSE
                  write(*,*) 'Warning : restart data '//trim(dataname) &
                     //' in '//trim(filename)//' not found, default value is used.'
               ENDIF
            ENDIF
         ENDIF
      ENDIF

#ifdef USEMPI
      IF (p_is_worker) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            IF (pixelset%vecgs%vlen(iblk,jblk) > 0) THEN
               allocate (rbuff (ndim1,ndim2,ndim3,ndim4, pixelset%vecgs%vlen(iblk,jblk)))
            ELSE
               allocate (rbuff(1,1,1,1,1))
            ENDIF

            CALL mpi_scatterv ( &
               MPI_RNULL_P, MPI_INULL_P, MPI_INULL_P, MPI_REAL8, & ! insignificant on workers
               rbuff, ndim1 * ndim2 * ndim3 * ndim4 * pixelset%vecgs%vlen(iblk,jblk), MPI_REAL8, &
               p_root, p_comm_group, p_err)

            IF (pixelset%vecgs%vlen(iblk,jblk) > 0) THEN
               istt = pixelset%vecgs%vstt(iblk,jblk)
               iend = pixelset%vecgs%vend(iblk,jblk)
               rdata(:,:,:,:,istt:iend) = rbuff
            ENDIF

            IF (allocated(rbuff)) deallocate (rbuff)

         ENDDO

      ENDIF
#endif

   END SUBROUTINE ncio_read_vector_real8_5d


   !---------------------------------------------------------
   SUBROUTINE ncio_create_file_vector (filename, pixelset)

   USE MOD_NetCDFSerial
   USE MOD_SPMD_Task
   USE MOD_Block
   USE MOD_Pixelset
   IMPLICIT NONE

   character(len=*), intent(in) :: filename
   type(pixelset_type), intent(in) :: pixelset

   ! Local variables
   integer :: iblkgrp, iblk, jblk
   character(len=256) :: fileblock

      IF (p_is_io) THEN
         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            CALL get_filename_block (filename, iblk, jblk, fileblock)
            CALL ncio_create_file (fileblock)

         ENDDO
      ENDIF

   END SUBROUTINE ncio_create_file_vector

   !---------------------------------------------------------
   SUBROUTINE ncio_define_dimension_vector (filename, pixelset, dimname, dimlen)

   USE MOD_NetCDFSerial
   USE MOD_SPMD_Task
   USE MOD_Block
   USE MOD_Pixelset
   IMPLICIT NONE

   character(len=*),    intent(in) :: filename
   type(pixelset_type), intent(in) :: pixelset
   character(len=*), intent(in)  :: dimname
   integer, intent(in), optional :: dimlen

   ! Local variables
   integer :: iblkgrp, iblk, jblk
   character(len=256) :: fileblock
   logical :: fexists

      IF (p_is_io) THEN
         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            CALL get_filename_block (filename, iblk, jblk, fileblock)
            inquire (file=trim(fileblock), exist=fexists)
            IF (.not. fexists) THEN
               CALL ncio_create_file (fileblock)
            ENDIF

            IF (present(dimlen)) THEN
               CALL ncio_define_dimension (fileblock, trim(dimname), dimlen)
            ELSE
               CALL ncio_define_dimension (fileblock, trim(dimname), &
                  pixelset%vecgs%vlen(iblk,jblk))
            ENDIF

         ENDDO
      ENDIF

   END SUBROUTINE ncio_define_dimension_vector

   !---------------------------------------------------------
   SUBROUTINE ncio_write_vector_int32_1d ( &
         filename, dataname, dimname, pixelset, wdata, compress_level)

   USE MOD_NetCDFSerial
   USE MOD_SPMD_Task
   USE MOD_Block
   USE MOD_Pixelset
   IMPLICIT NONE

   character(len=*), intent(in) :: filename
   character(len=*), intent(in) :: dataname
   character(len=*), intent(in) :: dimname
   type(pixelset_type), intent(in) :: pixelset
   integer, intent(in) :: wdata (:)

   integer, intent(in), optional :: compress_level

   ! Local variables
   integer :: iblkgrp, iblk, jblk, istt, iend
   character(len=256) :: fileblock
   integer, allocatable :: sbuff(:), rbuff(:)

      IF (p_is_io) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            allocate (rbuff (pixelset%vecgs%vlen(iblk,jblk)))
#ifdef USEMPI
            CALL mpi_gatherv (MPI_IN_PLACE, 0, MPI_INTEGER, &
               rbuff, pixelset%vecgs%vcnt(:,iblk,jblk), &
               pixelset%vecgs%vdsp(:,iblk,jblk), MPI_INTEGER, &
               p_root, p_comm_group, p_err)
#else
            istt = pixelset%vecgs%vstt(iblk,jblk)
            iend = pixelset%vecgs%vend(iblk,jblk)
            rbuff = wdata(istt:iend)
#endif

            CALL get_filename_block (filename, iblk, jblk, fileblock)

            IF (present(compress_level)) THEN
               CALL ncio_write_serial (fileblock, dataname, rbuff, dimname, &
                  compress = compress_level)
            ELSE
               CALL ncio_write_serial (fileblock, dataname, rbuff, dimname)
            ENDIF

            deallocate (rbuff)

         ENDDO

      ENDIF

#ifdef USEMPI
      IF (p_is_worker) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            IF (pixelset%vecgs%vlen(iblk,jblk) > 0) THEN
               allocate (sbuff (pixelset%vecgs%vlen(iblk,jblk)))
               istt = pixelset%vecgs%vstt(iblk,jblk)
               iend = pixelset%vecgs%vend(iblk,jblk)
               sbuff = wdata(istt:iend)
            ELSE
               allocate (sbuff (1))
            ENDIF

            CALL mpi_gatherv ( &
               sbuff, pixelset%vecgs%vlen(iblk,jblk), MPI_INTEGER, &
               MPI_INULL_P, MPI_INULL_P, MPI_INULL_P, MPI_INTEGER, & ! insignificant on workers
               p_root, p_comm_group, p_err)

            IF (allocated(sbuff)) deallocate (sbuff)

         ENDDO

      ENDIF
#endif

   END SUBROUTINE ncio_write_vector_int32_1d

   !---------------------------------------------------------
   SUBROUTINE ncio_write_vector_logical_1d ( &
         filename, dataname, dimname, pixelset, wdata, compress_level)

   USE MOD_NetCDFSerial
   USE MOD_SPMD_Task
   USE MOD_Block
   USE MOD_Pixelset
   IMPLICIT NONE

   character(len=*), intent(in) :: filename
   character(len=*), intent(in) :: dataname
   character(len=*), intent(in) :: dimname
   type(pixelset_type), intent(in) :: pixelset
   logical, intent(in) :: wdata (:)

   integer, intent(in), optional :: compress_level

   ! Local variables
   integer :: iblkgrp, iblk, jblk, istt, iend, i
   character(len=256) :: fileblock
   integer(1), allocatable :: sbuff(:), rbuff(:)

      IF (p_is_io) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            allocate (rbuff (pixelset%vecgs%vlen(iblk,jblk)))
#ifdef USEMPI
            CALL mpi_gatherv (MPI_IN_PLACE, 0, MPI_INTEGER1, &
               rbuff, pixelset%vecgs%vcnt(:,iblk,jblk), &
               pixelset%vecgs%vdsp(:,iblk,jblk), MPI_INTEGER1, &
               p_root, p_comm_group, p_err)
#else
            istt = pixelset%vecgs%vstt(iblk,jblk)
            iend = pixelset%vecgs%vend(iblk,jblk)
            DO i = istt, iend
               IF(wdata(i))THEN
                  rbuff(i-istt+1) = 1
               ELSE
                  rbuff(i-istt+1) = 0
               ENDIF
            ENDDO
#endif

            CALL get_filename_block (filename, iblk, jblk, fileblock)

            IF (present(compress_level)) THEN
               CALL ncio_write_serial (fileblock, dataname, rbuff, dimname, &
                  compress = compress_level)
            ELSE
               CALL ncio_write_serial (fileblock, dataname, rbuff, dimname)
            ENDIF

            deallocate (rbuff)

         ENDDO

      ENDIF

#ifdef USEMPI
      IF (p_is_worker) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            IF (pixelset%vecgs%vlen(iblk,jblk) > 0) THEN
               allocate (sbuff (pixelset%vecgs%vlen(iblk,jblk)))
               istt = pixelset%vecgs%vstt(iblk,jblk)
               iend = pixelset%vecgs%vend(iblk,jblk)
               DO i = istt, iend
                  IF(wdata(i))THEN
                     sbuff(i-istt+1) = 1
                  ELSE
                     sbuff(i-istt+1) = 0
                  ENDIF
               ENDDO
            ELSE
               allocate (sbuff (1))
            ENDIF

            CALL mpi_gatherv ( &
               sbuff, pixelset%vecgs%vlen(iblk,jblk), MPI_INTEGER1, &
               MPI_INULL_P, MPI_INULL_P, MPI_INULL_P, MPI_INTEGER1, & ! insignificant on workers
               p_root, p_comm_group, p_err)

            IF (allocated(sbuff)) deallocate (sbuff)

         ENDDO

      ENDIF
#endif

   END SUBROUTINE ncio_write_vector_logical_1d

   !---------------------------------------------------------
   SUBROUTINE ncio_write_vector_int32_3d ( &
         filename, dataname, dim1name, ndim1, dim2name, ndim2, &
         dim3name, pixelset, wdata, compress_level)

   USE MOD_NetCDFSerial
   USE MOD_SPMD_Task
   USE MOD_Block
   USE MOD_Pixelset
   IMPLICIT NONE

   character(len=*), intent(in) :: filename
   character(len=*), intent(in) :: dataname
   character(len=*), intent(in) :: dim1name, dim2name, dim3name
   type(pixelset_type), intent(in) :: pixelset
   integer, intent(in) :: ndim1, ndim2
   integer, intent(in) :: wdata (:,:,:)

   integer, intent(in), optional :: compress_level

   ! Local variables
   integer :: iblkgrp, iblk, jblk, istt, iend
   character(len=256) :: fileblock
   integer, allocatable :: sbuff(:,:,:), rbuff(:,:,:)

      IF (p_is_io) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            allocate (rbuff (ndim1,ndim2,pixelset%vecgs%vlen(iblk,jblk)))
#ifdef USEMPI
            CALL mpi_gatherv (MPI_IN_PLACE, 0, MPI_INTEGER, &
               rbuff, ndim1*ndim2*pixelset%vecgs%vcnt(:,iblk,jblk), &
               ndim1*ndim2*pixelset%vecgs%vdsp(:,iblk,jblk), MPI_INTEGER, &
               p_root, p_comm_group, p_err)
#else
            istt = pixelset%vecgs%vstt(iblk,jblk)
            iend = pixelset%vecgs%vend(iblk,jblk)
            rbuff = wdata(:,:,istt:iend)
#endif

            CALL get_filename_block (filename, iblk, jblk, fileblock)

            IF (present(compress_level)) THEN
               CALL ncio_write_serial (fileblock, dataname, rbuff, &
                  dim1name, dim2name, dim3name, compress = compress_level)
            ELSE
               CALL ncio_write_serial (fileblock, dataname, rbuff, &
                  dim1name, dim2name, dim3name)
            ENDIF

            deallocate (rbuff)

         ENDDO

      ENDIF

#ifdef USEMPI
      IF (p_is_worker) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            IF (pixelset%vecgs%vlen(iblk,jblk) > 0) THEN
               allocate (sbuff (ndim1,ndim2, pixelset%vecgs%vlen(iblk,jblk)))
               istt = pixelset%vecgs%vstt(iblk,jblk)
               iend = pixelset%vecgs%vend(iblk,jblk)
               sbuff = wdata(:,:,istt:iend)
            ELSE
               allocate (sbuff (1,1,1))
            ENDIF

            CALL mpi_gatherv ( &
               sbuff, ndim1*ndim2*pixelset%vecgs%vlen(iblk,jblk), MPI_INTEGER, &
               MPI_INULL_P, MPI_INULL_P, MPI_INULL_P, MPI_INTEGER, & ! insignificant on workers
               p_root, p_comm_group, p_err)

            IF (allocated(sbuff)) deallocate (sbuff)

         ENDDO

      ENDIF
#endif

   END SUBROUTINE ncio_write_vector_int32_3d

   !---------------------------------------------------------
   SUBROUTINE ncio_write_vector_int64_1d ( &
         filename, dataname, dimname, pixelset, wdata, compress_level)

   USE MOD_NetCDFSerial
   USE MOD_SPMD_Task
   USE MOD_Block
   USE MOD_Pixelset
   IMPLICIT NONE

   character(len=*), intent(in) :: filename
   character(len=*), intent(in) :: dataname
   character(len=*), intent(in) :: dimname
   type(pixelset_type), intent(in) :: pixelset
   integer*8, intent(in) :: wdata (:)

   integer, intent(in), optional :: compress_level

   ! Local variables
   integer :: iblkgrp, iblk, jblk, istt, iend
   character(len=256) :: fileblock
   integer*8, allocatable :: sbuff(:), rbuff(:)

      IF (p_is_io) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            allocate (rbuff (pixelset%vecgs%vlen(iblk,jblk)))
#ifdef USEMPI
            CALL mpi_gatherv (MPI_IN_PLACE, 0, MPI_INTEGER8, &
               rbuff, pixelset%vecgs%vcnt(:,iblk,jblk), &
               pixelset%vecgs%vdsp(:,iblk,jblk), MPI_INTEGER8, &
               p_root, p_comm_group, p_err)
#else
            istt = pixelset%vecgs%vstt(iblk,jblk)
            iend = pixelset%vecgs%vend(iblk,jblk)
            rbuff = wdata(istt:iend)
#endif

            CALL get_filename_block (filename, iblk, jblk, fileblock)

            IF (present(compress_level)) THEN
               CALL ncio_write_serial (fileblock, dataname, rbuff, dimname, &
                  compress = compress_level)
            ELSE
               CALL ncio_write_serial (fileblock, dataname, rbuff, dimname)
            ENDIF

            deallocate (rbuff)

         ENDDO

      ENDIF

#ifdef USEMPI
      IF (p_is_worker) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            IF (pixelset%vecgs%vlen(iblk,jblk) > 0) THEN
               allocate (sbuff (pixelset%vecgs%vlen(iblk,jblk)))
               istt = pixelset%vecgs%vstt(iblk,jblk)
               iend = pixelset%vecgs%vend(iblk,jblk)
               sbuff = wdata(istt:iend)
            ELSE
               allocate (sbuff (1))
            ENDIF

            CALL mpi_gatherv ( &
               sbuff, pixelset%vecgs%vlen(iblk,jblk), MPI_INTEGER8, &
               MPI_INULL_P, MPI_INULL_P, MPI_INULL_P, MPI_INTEGER8, & ! insignificant on workers
               p_root, p_comm_group, p_err)

            IF (allocated(sbuff)) deallocate (sbuff)

         ENDDO

      ENDIF
#endif

   END SUBROUTINE ncio_write_vector_int64_1d

   !---------------------------------------------------------
   SUBROUTINE ncio_write_vector_real8_1d ( &
         filename, dataname, dimname, pixelset, wdata, compress_level)

   USE MOD_Precision
   USE MOD_NetCDFSerial
   USE MOD_SPMD_Task
   USE MOD_Block
   USE MOD_Pixelset
   IMPLICIT NONE

   character(len=*), intent(in) :: filename
   character(len=*), intent(in) :: dataname
   character(len=*), intent(in) :: dimname
   type(pixelset_type), intent(in) :: pixelset
   real(r8), intent(in) :: wdata (:)

   integer, intent(in), optional :: compress_level

   ! Local variables
   integer :: iblkgrp, iblk, jblk, istt, iend
   character(len=256) :: fileblock
   real(r8), allocatable :: sbuff(:), rbuff(:)

      IF (p_is_io) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            allocate (rbuff (pixelset%vecgs%vlen(iblk,jblk)))
#ifdef USEMPI
            CALL mpi_gatherv ( MPI_IN_PLACE, 0, MPI_REAL8, &
               rbuff, pixelset%vecgs%vcnt(:,iblk,jblk), &
               pixelset%vecgs%vdsp(:,iblk,jblk), MPI_REAL8, &
               p_root, p_comm_group, p_err)
#else
            istt = pixelset%vecgs%vstt(iblk,jblk)
            iend = pixelset%vecgs%vend(iblk,jblk)
            rbuff = wdata(istt:iend)
#endif

            CALL get_filename_block (filename, iblk, jblk, fileblock)
            IF (present(compress_level)) THEN
               CALL ncio_write_serial (fileblock, dataname, rbuff, &
                  dimname, compress = compress_level)
            ELSE
               CALL ncio_write_serial (fileblock, dataname, rbuff, dimname)
            ENDIF

            deallocate (rbuff)

         ENDDO

      ENDIF

#ifdef USEMPI
      IF (p_is_worker) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            IF (pixelset%vecgs%vlen(iblk,jblk) > 0) THEN
               allocate (sbuff (pixelset%vecgs%vlen(iblk,jblk)))
               istt = pixelset%vecgs%vstt(iblk,jblk)
               iend = pixelset%vecgs%vend(iblk,jblk)
               sbuff = wdata(istt:iend)
            ELSE
               allocate (sbuff (1))
            ENDIF

            CALL mpi_gatherv ( &
               sbuff, pixelset%vecgs%vlen(iblk,jblk), MPI_REAL8, &
               MPI_RNULL_P, MPI_INULL_P, MPI_INULL_P, MPI_REAL8, & ! insignificant on workers
               p_root, p_comm_group, p_err)

            IF (allocated(sbuff)) deallocate (sbuff)

         ENDDO

      ENDIF
#endif

   END SUBROUTINE ncio_write_vector_real8_1d

   !---------------------------------------------------------
   SUBROUTINE ncio_write_vector_real8_2d ( &
         filename, dataname, dim1name, ndim1, &
         dim2name, pixelset, wdata, compress_level)

   USE MOD_Precision
   USE MOD_NetCDFSerial
   USE MOD_SPMD_Task
   USE MOD_Block
   USE MOD_Pixelset
   IMPLICIT NONE

   character(len=*), intent(in) :: filename
   character(len=*), intent(in) :: dataname
   character(len=*), intent(in) :: dim1name, dim2name
   integer,  intent(in) :: ndim1
   type(pixelset_type), intent(in) :: pixelset
   real(r8), intent(in) :: wdata (:,:)

   integer,  intent(in), optional :: compress_level

   ! Local variables
   integer :: iblkgrp, iblk, jblk, istt, iend
   character(len=256) :: fileblock
   real(r8), allocatable :: sbuff(:,:), rbuff(:,:)

      IF (p_is_io) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            allocate (rbuff (ndim1, pixelset%vecgs%vlen(iblk,jblk)))
#ifdef USEMPI
            CALL mpi_gatherv (MPI_IN_PLACE, 0, MPI_REAL8, &
               rbuff, ndim1 * pixelset%vecgs%vcnt(:,iblk,jblk), &
               ndim1 * pixelset%vecgs%vdsp(:,iblk,jblk), MPI_REAL8, &
               p_root, p_comm_group, p_err)
#else
            istt = pixelset%vecgs%vstt(iblk,jblk)
            iend = pixelset%vecgs%vend(iblk,jblk)
            rbuff = wdata(:,istt:iend)
#endif

            CALL get_filename_block (filename, iblk, jblk, fileblock)

            IF (present(compress_level)) THEN
               CALL ncio_write_serial (fileblock, dataname, rbuff, &
                  dim1name, dim2name, compress = compress_level)
            ELSE
               CALL ncio_write_serial (fileblock, dataname, rbuff, &
                  dim1name, dim2name)
            ENDIF

            deallocate (rbuff)

         ENDDO

      ENDIF

#ifdef USEMPI
      IF (p_is_worker) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            IF (pixelset%vecgs%vlen(iblk,jblk) > 0) THEN
               allocate (sbuff (ndim1,pixelset%vecgs%vlen(iblk,jblk)))
               istt = pixelset%vecgs%vstt(iblk,jblk)
               iend = pixelset%vecgs%vend(iblk,jblk)
               sbuff = wdata(:,istt:iend)
            ELSE
               allocate (sbuff (1,1))
            ENDIF

            CALL mpi_gatherv ( &
               sbuff, ndim1 * pixelset%vecgs%vlen(iblk,jblk), MPI_REAL8, &
               MPI_RNULL_P, MPI_INULL_P, MPI_INULL_P, MPI_REAL8, & ! insignificant on workers
               p_root, p_comm_group, p_err)

            IF (allocated(sbuff)) deallocate (sbuff)

         ENDDO

      ENDIF
#endif

   END SUBROUTINE ncio_write_vector_real8_2d

   !---------------------------------------------------------
   SUBROUTINE ncio_write_vector_real8_3d ( &
         filename, dataname, dim1name, ndim1, dim2name, ndim2, &
         dim3name, pixelset, wdata, compress_level)

   USE MOD_Precision
   USE MOD_NetCDFSerial
   USE MOD_SPMD_Task
   USE MOD_Block
   USE MOD_Pixelset
   IMPLICIT NONE

   character(len=*), intent(in) :: filename
   character(len=*), intent(in) :: dataname
   character(len=*), intent(in) :: dim1name, dim2name, dim3name
   type(pixelset_type), intent(in) :: pixelset
   integer,  intent(in) :: ndim1, ndim2
   real(r8), intent(in) :: wdata (:,:,:)

   integer,  intent(in), optional :: compress_level

   ! Local variables
   integer :: iblkgrp, iblk, jblk, istt, iend
   character(len=256) :: fileblock
   real(r8), allocatable :: sbuff(:,:,:), rbuff(:,:,:)

      IF (p_is_io) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            allocate (rbuff (ndim1, ndim2, pixelset%vecgs%vlen(iblk,jblk)))
#ifdef USEMPI
            CALL mpi_gatherv (MPI_IN_PLACE, 0, MPI_REAL8, &
               rbuff, ndim1 * ndim2 * pixelset%vecgs%vcnt(:,iblk,jblk), &
               ndim1 * ndim2 * pixelset%vecgs%vdsp(:,iblk,jblk), MPI_REAL8, &
               p_root, p_comm_group, p_err)
#else
            istt = pixelset%vecgs%vstt(iblk,jblk)
            iend = pixelset%vecgs%vend(iblk,jblk)
            rbuff = wdata(:,:,istt:iend)
#endif

            CALL get_filename_block (filename, iblk, jblk, fileblock)
            IF (present(compress_level)) THEN
               CALL ncio_write_serial (fileblock, dataname, rbuff, &
                  dim1name, dim2name, dim3name, compress = compress_level)
            ELSE
               CALL ncio_write_serial (fileblock, dataname, rbuff, &
                  dim1name, dim2name, dim3name)
            ENDIF

            deallocate (rbuff)

         ENDDO

      ENDIF

#ifdef USEMPI
      IF (p_is_worker) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            IF (pixelset%vecgs%vlen(iblk,jblk) > 0) THEN
               allocate (sbuff (ndim1,ndim2,pixelset%vecgs%vlen(iblk,jblk)))
               istt = pixelset%vecgs%vstt(iblk,jblk)
               iend = pixelset%vecgs%vend(iblk,jblk)
               sbuff = wdata(:,:,istt:iend)
            ELSE
               allocate (sbuff (1,1,1))
            ENDIF

            CALL mpi_gatherv ( sbuff, &
               ndim1 * ndim2 * pixelset%vecgs%vlen(iblk,jblk), MPI_REAL8, &
               MPI_RNULL_P, MPI_INULL_P, MPI_INULL_P, MPI_REAL8, & ! insignificant on workers
               p_root, p_comm_group, p_err)

            IF (allocated(sbuff)) deallocate (sbuff)

         ENDDO

      ENDIF
#endif

   END SUBROUTINE ncio_write_vector_real8_3d

   !---------------------------------------------------------
   SUBROUTINE ncio_write_vector_real8_4d ( &
         filename, dataname, dim1name, ndim1, dim2name, ndim2, dim3name, ndim3, &
         dim4name, pixelset, wdata, compress_level)

   USE MOD_Precision
   USE MOD_NetCDFSerial
   USE MOD_SPMD_Task
   USE MOD_Block
   USE MOD_Pixelset
   IMPLICIT NONE

   character(len=*), intent(in) :: filename
   character(len=*), intent(in) :: dataname
   character(len=*), intent(in) :: dim1name, dim2name, dim3name, dim4name
   integer,  intent(in) :: ndim1, ndim2, ndim3
   type(pixelset_type), intent(in) :: pixelset
   real(r8), intent(in) :: wdata (:,:,:,:)

   integer,  intent(in), optional :: compress_level

   ! Local variables
   integer :: iblkgrp, iblk, jblk, istt, iend
   character(len=256) :: fileblock
   real(r8), allocatable :: sbuff(:,:,:,:), rbuff(:,:,:,:)

      IF (p_is_io) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            allocate (rbuff (ndim1, ndim2, ndim3, pixelset%vecgs%vlen(iblk,jblk)))
#ifdef USEMPI
            CALL mpi_gatherv (MPI_IN_PLACE, 0, MPI_REAL8, &
               rbuff, ndim1 * ndim2 * ndim3 * pixelset%vecgs%vcnt(:,iblk,jblk), &
               ndim1 * ndim2 * ndim3 * pixelset%vecgs%vdsp(:,iblk,jblk), MPI_REAL8, &
               p_root, p_comm_group, p_err)
#else
            istt = pixelset%vecgs%vstt(iblk,jblk)
            iend = pixelset%vecgs%vend(iblk,jblk)
            rbuff = wdata(:,:,:,istt:iend)
#endif

            CALL get_filename_block (filename, iblk, jblk, fileblock)
            IF (present(compress_level)) THEN
               CALL ncio_write_serial (fileblock, dataname, rbuff, &
                  dim1name, dim2name, dim3name, dim4name, compress = compress_level)
            ELSE
               CALL ncio_write_serial (fileblock, dataname, rbuff, &
                  dim1name, dim2name, dim3name, dim4name)
            ENDIF

            deallocate (rbuff)

         ENDDO

      ENDIF

#ifdef USEMPI
      IF (p_is_worker) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            IF (pixelset%vecgs%vlen(iblk,jblk) > 0) THEN
               allocate (sbuff (ndim1,ndim2,ndim3,pixelset%vecgs%vlen(iblk,jblk)))
               istt = pixelset%vecgs%vstt(iblk,jblk)
               iend = pixelset%vecgs%vend(iblk,jblk)
               sbuff = wdata(:,:,:,istt:iend)
            ELSE
               allocate (sbuff (1,1,1,1))
            ENDIF

            CALL mpi_gatherv ( sbuff, &
               ndim1 * ndim2 * ndim3 * pixelset%vecgs%vlen(iblk,jblk), MPI_REAL8, &
               MPI_RNULL_P, MPI_INULL_P, MPI_INULL_P, MPI_REAL8, & ! insignificant on workers
               p_root, p_comm_group, p_err)

            IF (allocated(sbuff)) deallocate (sbuff)

         ENDDO

      ENDIF
#endif

   END SUBROUTINE ncio_write_vector_real8_4d


   !------------------------------------------------
   SUBROUTINE ncio_write_vector_real8_5d ( &
         filename, dataname, dim1name, ndim1, dim2name, ndim2, &
         dim3name, ndim3, dim4name, ndim4, dim5name, pixelset, wdata, compress_level)

   USE MOD_Precision
   USE MOD_NetCDFSerial
   USE MOD_SPMD_Task
   USE MOD_Block
   USE MOD_Pixelset
   IMPLICIT NONE

   character(len=*), intent(in) :: filename
   character(len=*), intent(in) :: dataname
   character(len=*), intent(in) :: dim1name, dim2name, dim3name, dim4name, dim5name
   type(pixelset_type), intent(in) :: pixelset
   integer,  intent(in) :: ndim1, ndim2, ndim3, ndim4
   real(r8), intent(in) :: wdata (:,:,:,:,:)

   integer,  intent(in), optional :: compress_level

   ! Local variables
   integer :: iblkgrp, iblk, jblk, istt, iend
   character(len=256) :: fileblock
   real(r8), allocatable :: sbuff(:,:,:,:,:), rbuff(:,:,:,:,:)

      IF (p_is_io) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            allocate (rbuff (ndim1, ndim2, ndim3, ndim4, pixelset%vecgs%vlen(iblk,jblk)))
#ifdef USEMPI
            CALL mpi_gatherv (MPI_IN_PLACE, 0, MPI_REAL8, &
               rbuff, ndim1 * ndim2 * ndim3 * ndim4 * pixelset%vecgs%vcnt(:,iblk,jblk), &
               ndim1 * ndim2 * ndim3 * ndim4 * pixelset%vecgs%vdsp(:,iblk,jblk), MPI_REAL8, &
               p_root, p_comm_group, p_err)
#else
            istt = pixelset%vecgs%vstt(iblk,jblk)
            iend = pixelset%vecgs%vend(iblk,jblk)
            rbuff = wdata(:,:,:,:,istt:iend)
#endif

            CALL get_filename_block (filename, iblk, jblk, fileblock)
            IF (present(compress_level)) THEN
               CALL ncio_write_serial (fileblock, dataname, rbuff, &
                  dim1name, dim2name, dim3name, dim4name, dim5name, compress = compress_level)
            ELSE
               CALL ncio_write_serial (fileblock, dataname, rbuff, &
                  dim1name, dim2name, dim3name, dim4name, dim5name)
            ENDIF

            deallocate (rbuff)

         ENDDO

      ENDIF

#ifdef USEMPI
      IF (p_is_worker) THEN

         DO iblkgrp = 1, pixelset%nblkgrp
            iblk = pixelset%xblkgrp(iblkgrp)
            jblk = pixelset%yblkgrp(iblkgrp)

            IF (pixelset%vecgs%vlen(iblk,jblk) > 0) THEN
               allocate (sbuff (ndim1,ndim2,ndim3,ndim4,pixelset%vecgs%vlen(iblk,jblk)))
               istt = pixelset%vecgs%vstt(iblk,jblk)
               iend = pixelset%vecgs%vend(iblk,jblk)
               sbuff = wdata(:,:,:,:,istt:iend)
            ELSE
               allocate (sbuff (1,1,1,1,1))
            ENDIF

            CALL mpi_gatherv ( sbuff, &
               ndim1 * ndim2 * ndim3 * ndim4 * pixelset%vecgs%vlen(iblk,jblk), MPI_REAL8, &
               MPI_RNULL_P, MPI_INULL_P, MPI_INULL_P, MPI_REAL8, & ! insignificant on workers
               p_root, p_comm_group, p_err)

            IF (allocated(sbuff)) deallocate (sbuff)

         ENDDO

      ENDIF
#endif

   END SUBROUTINE ncio_write_vector_real8_5d
   !------------------------------------------------


END MODULE MOD_NetCDFVector
