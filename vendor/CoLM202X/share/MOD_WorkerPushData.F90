#include <define.h>

MODULE MOD_WorkerPushData
!--------------------------------------------------------------------------------
! DESCRIPTION:
!--------------------------------------------------------------------------------

   USE MOD_Precision
   USE MOD_DataType
   USE MOD_SPMD_Task
   USE MOD_Utils
   IMPLICIT NONE

   integer, parameter, private :: WORKER_PUSH_MAPPING_UNSET  = 0
   integer, parameter, private :: WORKER_PUSH_MAPPING_SINGLE = 1
   integer, parameter, private :: WORKER_PUSH_MAPPING_MULTI  = 2

   ! -- Non-owning descriptor for one real8 field in a batched push --
   type :: worker_push_real8_field_type
      real(r8), pointer :: send(:) => null()
      real(r8), pointer :: recv(:) => null()
      real(r8) :: fillvalue = 0._r8
   END type worker_push_real8_field_type

   ! -- Data Type : push data between workers --
   type :: worker_pushdata_type

      integer :: mapping_kind = WORKER_PUSH_MAPPING_UNSET
      integer :: required_send_size = 0
      integer :: num_req_uniq

      integer,  allocatable :: addr_single (:)

      integer,  allocatable :: addr_multi  (:,:)
      real(r8), allocatable :: area_multi  (:,:)
      real(r8), allocatable :: sum_area    (:)

      ! data is on the same processor
      integer :: nself
      integer,  allocatable :: self_from (:)
      integer,  allocatable :: self_to   (:)

      ! Reused real8 communication scratch.  A push mapping is immutable after
      ! construction, so its unique receive size, peer counts, and packed
      ! message sizes are also immutable.  Keeping these buffers with the
      ! mapping avoids several heap allocations in every routing/tracer push.
      real(r8), allocatable :: recv_uniq_real8 (:)
      logical :: real8_scratch_ready = .false.
#ifdef USEMPI
      ! data is on other processors
      integer, allocatable :: n_to_other   (:)
      integer, allocatable :: n_from_other (:)
      type(pointer_int32_1d), allocatable :: to_other (:)
      type(pointer_int32_1d), allocatable :: other_to (:)
      real(r8), allocatable :: sendcache_real8 (:)
      real(r8), allocatable :: recvcache_real8 (:)
      integer :: real8_batch_capacity = 0
      real(r8), allocatable :: sendcache_real8_batch (:)
      real(r8), allocatable :: recvcache_real8_batch (:)
      integer,  allocatable :: req_send_real8  (:)
      integer,  allocatable :: req_recv_real8  (:)
      integer,  allocatable :: send_peer_real8 (:)
      integer,  allocatable :: recv_peer_real8 (:)
      integer,  allocatable :: send_disp_real8 (:)
      integer,  allocatable :: recv_disp_real8 (:)
#endif
   CONTAINS
      final :: worker_pushdata_free_mem
   END type worker_pushdata_type


   ! -- Data Type : remap data on workers --
   type :: worker_remapdata_type

      integer :: num_grid
      integer, allocatable :: ilon_me (:)
      integer, allocatable :: ilat_me (:)
      integer, allocatable :: ids_me  (:)

      integer :: npset
      real(r8), allocatable :: sum_area (:)
      integer,  allocatable :: npart    (:)
      type(pointer_int32_1d), allocatable :: part_to  (:) !
      type(pointer_real8_1d), allocatable :: areapart (:) ! intersection area

   CONTAINS
      final :: worker_remapdata_free_mem
   END type worker_remapdata_type

   ! -- public subroutines --
   INTERFACE build_worker_pushdata
      MODULE procedure build_worker_pushdata_single
      MODULE procedure build_worker_pushdata_multi
   END INTERFACE build_worker_pushdata

   PUBLIC :: build_worker_pushdata_subset

   PUBLIC :: build_worker_remapdata

   INTERFACE worker_push_data
      MODULE procedure worker_push_data_single_real8
      MODULE procedure worker_push_data_single_int32
      MODULE procedure worker_push_data_multi_real8
      MODULE procedure worker_push_data_multi_real8_batch
   END INTERFACE worker_push_data

   INTERFACE worker_remap_data_pset2grid
      MODULE procedure worker_remap_data_pset2grid_real8
   END INTERFACE worker_remap_data_pset2grid

   INTERFACE worker_remap_data_grid2pset
      MODULE procedure worker_remap_data_grid2pset_real8
   END INTERFACE worker_remap_data_grid2pset

CONTAINS

   ! ----------
   SUBROUTINE build_worker_pushdata_uniq (num_me, ids_me, n_req_uniq, ids_req_uniq, pushdata)

   IMPLICIT NONE

   integer, intent(in) :: num_me,     ids_me       (:)
   integer, intent(in) :: n_req_uniq, ids_req_uniq (:)
   type(worker_pushdata_type), intent(inout) :: pushdata

   ! Local Variables
   integer, allocatable :: ids_me_sorted(:), order_ids(:), self_from(:)
#ifdef USEMPI
   integer, allocatable :: ids(:), loc_from_me(:), loc_from_other(:)
   integer :: request(3)
#endif
   integer :: i, iloc, iworker, jworker, n_req_other

      CALL reset_worker_push_real8_scratch (pushdata)

      IF (p_is_worker) THEN

         pushdata%required_send_size = 0

         IF (num_me > 0) THEN
            allocate (ids_me_sorted (num_me));  ids_me_sorted = ids_me
            allocate (order_ids     (num_me));  order_ids = (/(i, i = 1, num_me)/)

            CALL quicksort (num_me, ids_me_sorted, order_ids)
         ENDIF

         pushdata%nself = 0

         IF (n_req_uniq > 0) THEN
            allocate (self_from (n_req_uniq))
            self_from(:) = -1

            IF (num_me > 0) THEN
               DO i = 1, n_req_uniq
                  iloc = find_in_sorted_list1 (ids_req_uniq(i), num_me, ids_me_sorted)
                  IF (iloc > 0) THEN
                     self_from(i) = order_ids(iloc)
                  ENDIF
               ENDDO
            ENDIF

            pushdata%nself = count(self_from > 0)
            IF (pushdata%nself > 0) THEN
               allocate (pushdata%self_from (pushdata%nself))
               allocate (pushdata%self_to   (pushdata%nself))
               pushdata%self_from = pack(self_from, self_from > 0)
               pushdata%self_to   = pack((/(i,i=1,n_req_uniq)/), self_from > 0)
            ENDIF
         ENDIF

#ifdef USEMPI
         CALL mpi_barrier (p_comm_worker, p_err)

         allocate (pushdata%n_to_other   (0:p_np_worker-1))
         allocate (pushdata%to_other     (0:p_np_worker-1))

         allocate (pushdata%n_from_other (0:p_np_worker-1))
         allocate (pushdata%other_to     (0:p_np_worker-1))

         pushdata%n_to_other  (:) = 0
         pushdata%n_from_other(:) = 0

         IF (n_req_uniq > 0) allocate (loc_from_other (n_req_uniq))

         iworker = modulo(p_iam_worker+1, p_np_worker)
         jworker = modulo(p_iam_worker-1, p_np_worker)
         DO WHILE (iworker /= p_iam_worker)

            CALL mpi_isend (n_req_uniq, 1, MPI_INTEGER, jworker, 10, &
               p_comm_worker, request(1), p_err)

            IF (n_req_uniq > 0) THEN
               CALL mpi_isend(ids_req_uniq, n_req_uniq, MPI_INTEGER, jworker, 11, &
                  p_comm_worker, request(2), p_err)
            ENDIF

            CALL mpi_recv (n_req_other, 1, MPI_INTEGER, iworker, 10, &
               p_comm_worker, p_stat, p_err)

            IF (n_req_other > 0) THEN

               allocate (ids (n_req_other))
               CALL mpi_recv (ids, n_req_other, MPI_INTEGER, iworker, 11, &
                  p_comm_worker, p_stat, p_err)

               allocate (loc_from_me (n_req_other))
               loc_from_me(:) = -1

               IF (num_me > 0) THEN
                  DO i = 1, n_req_other
                     iloc = find_in_sorted_list1 (ids(i), num_me, ids_me_sorted)
                     IF (iloc > 0) THEN
                        loc_from_me(i) = order_ids(iloc)
                     ENDIF
                  ENDDO
               ENDIF

               pushdata%n_to_other(iworker) = count(loc_from_me > 0)
               IF (pushdata%n_to_other(iworker) > 0) THEN
                  allocate (pushdata%to_other(iworker)%val (pushdata%n_to_other(iworker)))
                  pushdata%to_other(iworker)%val = pack(loc_from_me, loc_from_me > 0)
               ENDIF

               CALL mpi_isend (loc_from_me, n_req_other, MPI_INTEGER, iworker, 12, &
                  p_comm_worker, request(3), p_err)

            ENDIF

            IF (n_req_uniq > 0) THEN

               CALL mpi_recv (loc_from_other, n_req_uniq, MPI_INTEGER, &
                  jworker, 12, p_comm_worker, p_stat, p_err)

               pushdata%n_from_other(jworker) = count(loc_from_other > 0)
               IF (pushdata%n_from_other(jworker) > 0) THEN
                  allocate (pushdata%other_to(jworker)%val (pushdata%n_from_other(jworker)))
                  pushdata%other_to(jworker)%val = pack((/(i,i=1,n_req_uniq)/), loc_from_other > 0)
               ENDIF

            ENDIF

            CALL mpi_wait(request(1), MPI_STATUS_IGNORE, p_err)
            IF (n_req_uniq  > 0) CALL mpi_wait(request(2), MPI_STATUSES_IGNORE, p_err)
            IF (n_req_other > 0) CALL mpi_wait(request(3), MPI_STATUSES_IGNORE, p_err)

            IF (allocated(ids        )) deallocate(ids        )
            IF (allocated(loc_from_me)) deallocate(loc_from_me)

            iworker = modulo(iworker+1, p_np_worker)
            jworker = modulo(jworker-1, p_np_worker)
         ENDDO

         IF (allocated (loc_from_other)) deallocate (loc_from_other)

         CALL mpi_barrier (p_comm_worker, p_err)
#endif

         CALL update_worker_push_required_send_size (pushdata)

         IF (allocated(ids_me_sorted)) deallocate(ids_me_sorted)
         IF (allocated(order_ids    )) deallocate(order_ids    )
         IF (allocated(self_from    )) deallocate(self_from    )

      ENDIF

   END SUBROUTINE build_worker_pushdata_uniq

   ! ----------
   SUBROUTINE update_worker_push_required_send_size (pushdata)

   IMPLICIT NONE

   type(worker_pushdata_type), intent(inout) :: pushdata

#ifdef USEMPI
   integer :: iworker
#endif

      pushdata%required_send_size = 0
      IF (allocated(pushdata%self_from)) THEN
         IF (size(pushdata%self_from) > 0) &
            pushdata%required_send_size = maxval(pushdata%self_from)
      ENDIF

#ifdef USEMPI
      IF (allocated(pushdata%n_to_other) .and. allocated(pushdata%to_other)) THEN
         DO iworker = lbound(pushdata%n_to_other, 1), ubound(pushdata%n_to_other, 1)
            IF (pushdata%n_to_other(iworker) > 0) THEN
               pushdata%required_send_size = max(pushdata%required_send_size, &
                  maxval(pushdata%to_other(iworker)%val))
            ENDIF
         ENDDO
      ENDIF
#endif

   END SUBROUTINE update_worker_push_required_send_size

   ! ----------
   SUBROUTINE build_worker_pushdata_single (num_me, ids_me, num_req, ids_req, pushdata)

   IMPLICIT NONE

   integer, intent(in) :: num_me,  ids_me  (:)
   integer, intent(in) :: num_req, ids_req (:)
   type(worker_pushdata_type), intent(inout) :: pushdata

   ! Local Variables
   integer :: n_req_uniq, iloc, i
   integer, allocatable :: ids_req_uniq (:)

      IF (p_is_worker) THEN

         n_req_uniq = 0
         ! A zero-sized allocated actual is valid; an array section of an
         ! unallocated allocatable is not.  Empty workers still build every
         ! mapping collectively, including mappings with no local requests.
         allocate (ids_req_uniq (num_req))

         IF (num_req > 0) THEN
            DO i = 1, size(ids_req)
               CALL insert_into_sorted_list1 (ids_req(i), n_req_uniq, ids_req_uniq, iloc)
            ENDDO

            allocate (pushdata%addr_single (num_req))
            DO i = 1, size(ids_req)
               pushdata%addr_single(i) = &
                  find_in_sorted_list1 (ids_req(i), n_req_uniq, ids_req_uniq(1:n_req_uniq))
            ENDDO
         ENDIF

         pushdata%num_req_uniq = n_req_uniq

         CALL build_worker_pushdata_uniq ( &
            num_me, ids_me, n_req_uniq, ids_req_uniq(1:n_req_uniq), pushdata)
         pushdata%mapping_kind = WORKER_PUSH_MAPPING_SINGLE

         IF (allocated (ids_req_uniq)) deallocate(ids_req_uniq)

      ENDIF

   END SUBROUTINE build_worker_pushdata_single

   ! ----------
   SUBROUTINE build_worker_pushdata_multi ( &
         num_me, ids_me, num_req, ids_req, area_req, pushdata)

   IMPLICIT NONE

   integer,  intent(in) :: num_me,  ids_me  (:)
   integer,  intent(in) :: num_req, ids_req (:,:)
   real(r8), intent(in) :: area_req(:,:)
   type(worker_pushdata_type), intent(inout) :: pushdata

   ! Local Variables
   integer :: ndim1, n_req_uniq, iloc, i, j, iworker
   integer, allocatable :: ids_req_uniq (:)
   logical, allocatable :: id_found     (:)

      IF (p_is_worker) THEN

         n_req_uniq = 0
         ndim1 = size(ids_req,1)
         ! See the single-map builder above: keep the zero-request actual
         ! allocated so build_worker_pushdata_uniq receives a legal 1:0 slice.
         allocate (ids_req_uniq (ndim1*num_req))

         IF (num_req > 0) THEN
            DO j = 1, num_req
               DO i = 1, ndim1
                  CALL insert_into_sorted_list1 (ids_req(i,j), n_req_uniq, ids_req_uniq, iloc)
               ENDDO
            ENDDO

            allocate (pushdata%addr_multi (ndim1,num_req))

            DO j = 1, num_req
               DO i = 1, ndim1
                  pushdata%addr_multi(i,j) = &
                     find_in_sorted_list1 (ids_req(i,j), n_req_uniq, ids_req_uniq(1:n_req_uniq))
               ENDDO
            ENDDO
         ENDIF

         pushdata%num_req_uniq = n_req_uniq

         CALL build_worker_pushdata_uniq ( &
            num_me, ids_me, n_req_uniq, ids_req_uniq(1:n_req_uniq), pushdata)
         pushdata%mapping_kind = WORKER_PUSH_MAPPING_MULTI

         IF (num_req > 0) THEN
            allocate (pushdata%area_multi (ndim1,num_req))
            allocate (pushdata%sum_area   (num_req))

            pushdata%area_multi = area_req

            WHERE ((area_req <= 0.) .or. (ids_req <= 0))
               pushdata%area_multi = 0.
            END WHERE

            allocate (id_found (n_req_uniq))
            id_found(:) = .false.

            IF (pushdata%nself > 0) id_found(pushdata%self_to) = .true.
#ifdef USEMPI
            DO iworker = 0, p_np_worker-1
               IF (pushdata%n_from_other(iworker) > 0) THEN
                  id_found(pushdata%other_to(iworker)%val) = .true.
               ENDIF
            ENDDO
#endif

            DO j = 1, num_req
               DO i = 1, ndim1
                  IF (.not. id_found(pushdata%addr_multi(i,j))) then
                     pushdata%area_multi(i,j) = 0.
                  ENDIF
               ENDDO
            ENDDO

            pushdata%sum_area = sum(pushdata%area_multi, dim = 1)

            deallocate (id_found)
         ENDIF

         IF (allocated (ids_req_uniq)) deallocate(ids_req_uniq)

      ENDIF

   END SUBROUTINE build_worker_pushdata_multi

   ! ----------
   SUBROUTINE build_worker_pushdata_subset ( &
         num_me, num_req, pushdata_in, subset_in, pushdata_out, subset_out, numsubset)

   USE MOD_Pixelset
   IMPLICIT NONE

   integer, intent(in) :: num_me
   integer, intent(in) :: num_req

   type(worker_pushdata_type),  intent(in)    :: pushdata_in
   type(subset_type),           intent(in)    :: subset_in

   type(worker_pushdata_type),  intent(inout) :: pushdata_out
   type(subset_type), optional, intent(inout) :: subset_out
   integer, optional,           intent(inout) :: numsubset

   ! Local Variables
   integer :: i, j, ii, jj, idsp, jdsp, kdsp, nsub, iworker
   integer, allocatable :: nsub_me  (:), nsub_req(:), nsub_req_uniq(:)
   integer, allocatable :: subdsp_me(:), subdsp_req_uniq(:)

      IF (p_is_worker) THEN

         pushdata_out%mapping_kind = WORKER_PUSH_MAPPING_SINGLE

         IF (num_me > 0) THEN
            allocate (nsub_me (num_me))
            nsub_me = subset_in%subend - subset_in%substt + 1
         ENDIF

         IF (num_req > 0) THEN
            allocate (nsub_req (num_req))
         ENDIF

         CALL worker_push_data (pushdata_in, nsub_me, nsub_req, 0)

         IF (present(subset_out) .and. present(numsubset)) THEN
            IF (num_req > 0) THEN
               allocate (subset_out%substt (num_req))
               allocate (subset_out%subend (num_req))

               idsp = 0
               DO i = 1, num_req
                  IF (nsub_req(i) == 0) THEN
                     subset_out%substt(i) = 0
                     subset_out%subend(i) = -1
                  ELSE
                     subset_out%substt(i) = idsp + 1
                     subset_out%subend(i) = idsp + nsub_req(i)
                  ENDIF
                  idsp = idsp + nsub_req(i)
               ENDDO

               numsubset = idsp
            ELSE
               numsubset = 0
            ENDIF
         ENDIF

         IF (num_me > 0) THEN
            allocate (subdsp_me (num_me))
            idsp = 0
            DO i = 1, num_me
               IF (nsub_me(i) > 0) THEN
                  subdsp_me(i) = idsp
                  idsp = idsp + nsub_me(i)
               ENDIF
            ENDDO
         ENDIF

         pushdata_out%num_req_uniq = 0
         IF (pushdata_in%num_req_uniq > 0) THEN

            allocate (nsub_req_uniq   (pushdata_in%num_req_uniq))
            allocate (subdsp_req_uniq (pushdata_in%num_req_uniq))

            nsub_req_uniq(:) = 0
            DO i = 1, size(pushdata_in%addr_single)
               nsub_req_uniq(pushdata_in%addr_single(i)) = nsub_req(i)
            ENDDO

            idsp = 0
            DO i = 1, pushdata_in%num_req_uniq
               IF (nsub_req_uniq(i) > 0) THEN
                  subdsp_req_uniq(i) = idsp
                  idsp = idsp + nsub_req_uniq(i)
               ENDIF
            ENDDO

            pushdata_out%num_req_uniq = sum(nsub_req_uniq)
         ENDIF


         IF (pushdata_out%num_req_uniq > 0) THEN
            allocate (pushdata_out%addr_single (sum(nsub_req)))

            idsp = 0
            DO i = 1, num_req
               IF (nsub_req(i) > 0) THEN
                  jdsp = subdsp_req_uniq(pushdata_in%addr_single(i))
                  pushdata_out%addr_single(idsp+1:idsp+nsub_req(i)) = &
                     (/(jj, jj=jdsp+1,jdsp+nsub_req(i))/)
                  idsp = idsp + nsub_req(i)
               ENDIF
            ENDDO
         ENDIF

         pushdata_out%nself = 0
         DO i = 1, pushdata_in%nself
            pushdata_out%nself = pushdata_out%nself + nsub_me(pushdata_in%self_from(i))
         ENDDO

         IF (pushdata_out%nself > 0) THEN
            allocate (pushdata_out%self_from (pushdata_out%nself))
            allocate (pushdata_out%self_to   (pushdata_out%nself))

            kdsp = 0
            DO i = 1, pushdata_in%nself
               IF (nsub_me(pushdata_in%self_from(i)) > 0) THEN
                  nsub = nsub_me(pushdata_in%self_from(i))
                  idsp = subdsp_me      (pushdata_in%self_from(i))
                  jdsp = subdsp_req_uniq(pushdata_in%self_to  (i))
                  pushdata_out%self_from(kdsp+1:kdsp+nsub) = (/(ii, ii=idsp+1,idsp+nsub)/)
                  pushdata_out%self_to  (kdsp+1:kdsp+nsub) = (/(jj, jj=jdsp+1,jdsp+nsub)/)
                  kdsp = kdsp + nsub
               ENDIF
            ENDDO
         ENDIF

#ifdef USEMPI
         allocate (pushdata_out%n_to_other (0:p_np_worker-1))
         allocate (pushdata_out%to_other   (0:p_np_worker-1))

         DO iworker = 0, p_np_worker-1

            pushdata_out%n_to_other(iworker) = 0
            DO i = 1, pushdata_in%n_to_other(iworker)
               pushdata_out%n_to_other(iworker) = pushdata_out%n_to_other(iworker) &
                  + nsub_me(pushdata_in%to_other(iworker)%val(i))
            ENDDO

            IF (pushdata_out%n_to_other(iworker) > 0) THEN
               allocate (pushdata_out%to_other(iworker)%val (pushdata_out%n_to_other(iworker)))
               kdsp = 0
               DO i = 1, pushdata_in%n_to_other(iworker)
                  IF (nsub_me(pushdata_in%to_other(iworker)%val(i)) > 0) THEN
                     nsub = nsub_me  (pushdata_in%to_other(iworker)%val(i))
                     idsp = subdsp_me(pushdata_in%to_other(iworker)%val(i))
                     pushdata_out%to_other(iworker)%val(kdsp+1:kdsp+nsub) = (/(ii, ii=idsp+1,idsp+nsub)/)
                     kdsp = kdsp + nsub
                  ENDIF
               ENDDO
            ENDIF

         ENDDO

         allocate (pushdata_out%n_from_other (0:p_np_worker-1))
         allocate (pushdata_out%other_to     (0:p_np_worker-1))

         DO iworker = 0, p_np_worker-1
            pushdata_out%n_from_other(iworker) = 0
            DO i = 1, pushdata_in%n_from_other(iworker)
               pushdata_out%n_from_other(iworker) = pushdata_out%n_from_other(iworker) &
                  + nsub_req_uniq(pushdata_in%other_to(iworker)%val(i))
            ENDDO

            IF (pushdata_out%n_from_other(iworker) > 0) THEN
               allocate (pushdata_out%other_to(iworker)%val (pushdata_out%n_from_other(iworker)))
               kdsp = 0
               DO i = 1, pushdata_in%n_from_other(iworker)
                  IF (nsub_req_uniq(pushdata_in%other_to(iworker)%val(i)) > 0) THEN
                     nsub = nsub_req_uniq  (pushdata_in%other_to(iworker)%val(i))
                     idsp = subdsp_req_uniq(pushdata_in%other_to(iworker)%val(i))
                     pushdata_out%other_to(iworker)%val(kdsp+1:kdsp+nsub) = (/(ii, ii=idsp+1,idsp+nsub)/)
                     kdsp = kdsp + nsub
                  ENDIF
               ENDDO
            ENDIF
         ENDDO
#endif

         CALL update_worker_push_required_send_size (pushdata_out)

         IF (allocated (nsub_me        )) deallocate (nsub_me        )
         IF (allocated (nsub_req       )) deallocate (nsub_req       )
         IF (allocated (nsub_req_uniq  )) deallocate (nsub_req_uniq  )
         IF (allocated (subdsp_me      )) deallocate (subdsp_me      )
         IF (allocated (subdsp_req_uniq)) deallocate (subdsp_req_uniq)

      ENDIF

   END SUBROUTINE build_worker_pushdata_subset

   ! ----------
   SUBROUTINE build_worker_remapdata (pixelset, grid, remapdata)

   USE MOD_Grid
   USE MOD_Pixelset
   USE MOD_SpatialMapping
   IMPLICIT NONE

   type(pixelset_type), intent(in) :: pixelset
   type(grid_type),     intent(in) :: grid

   type(worker_remapdata_type), intent(inout) :: remapdata

   ! Local Variables
   type(spatial_mapping_type) :: mapping
   integer, allocatable :: ilon_me(:), ilat_me(:)
   integer :: ngrid, iproc, ig, iloc, iset, ipart


      CALL mapping%build_arealweighted (grid, pixelset)

      IF (p_is_worker) THEN

         ngrid = 0
         DO iproc = 0, p_np_io-1
            ngrid = ngrid + mapping%glist(iproc)%ng
         ENDDO

         IF (ngrid > 0) THEN
            allocate (ilon_me (ngrid))
            allocate (ilat_me (ngrid))
         ENDIF

         ngrid = 0
         DO iproc = 0, p_np_io-1
            DO ig = 1, mapping%glist(iproc)%ng
               CALL insert_into_sorted_list2 ( &
                  mapping%glist(iproc)%ilon(ig), mapping%glist(iproc)%ilat(ig), &
                  ngrid, ilon_me, ilat_me, iloc)
            ENDDO
         ENDDO

         remapdata%num_grid = ngrid

         IF (ngrid > 0) THEN
            allocate (remapdata%ilon_me (ngrid))
            allocate (remapdata%ilat_me (ngrid))
            allocate (remapdata%ids_me  (ngrid))
            remapdata%ilon_me = ilon_me(1:ngrid)
            remapdata%ilat_me = ilat_me(1:ngrid)
            remapdata%ids_me  = (ilat_me(1:ngrid)-1) * grid%nlon + ilon_me(1:ngrid)
         ENDIF

         remapdata%npset = mapping%npset

         IF (remapdata%npset > 0) THEN

            allocate (remapdata%sum_area (remapdata%npset))
            allocate (remapdata%npart    (remapdata%npset))
            allocate (remapdata%part_to  (remapdata%npset))
            allocate (remapdata%areapart (remapdata%npset))

            remapdata%npart = mapping%npart

            DO iset = 1, remapdata%npset
               IF (remapdata%npart(iset) > 0) THEN
                  allocate (remapdata%part_to(iset)%val  (remapdata%npart(iset)))
                  allocate (remapdata%areapart(iset)%val (remapdata%npart(iset)))
               ENDIF

               DO ipart = 1, remapdata%npart(iset)
                  iproc = mapping%address(iset)%val(1,ipart)
                  iloc  = mapping%address(iset)%val(2,ipart)
                  remapdata%part_to(iset)%val(ipart)  = find_in_sorted_list2 ( &
                     mapping%glist(iproc)%ilon(iloc), mapping%glist(iproc)%ilat(iloc), ngrid, ilon_me, ilat_me)
                  ! from km^2 to m^2
                  remapdata%areapart(iset)%val(ipart) = mapping%areapart(iset)%val(ipart) * 1.e6
               ENDDO

               IF (remapdata%npart(iset) > 0) THEN
                  remapdata%sum_area(iset) = sum(remapdata%areapart(iset)%val)
               ENDIF
            ENDDO
         ENDIF

         IF (allocated(ilon_me)) deallocate(ilon_me)
         IF (allocated(ilat_me)) deallocate(ilat_me)

      ENDIF

   END SUBROUTINE build_worker_remapdata

   ! ----------
   SUBROUTINE reset_worker_push_real8_scratch (pushdata)

   IMPLICIT NONE

   type(worker_pushdata_type), intent(inout) :: pushdata

      pushdata%real8_scratch_ready = .false.
      IF (allocated(pushdata%recv_uniq_real8)) deallocate(pushdata%recv_uniq_real8)
#ifdef USEMPI
      IF (allocated(pushdata%sendcache_real8)) deallocate(pushdata%sendcache_real8)
      IF (allocated(pushdata%recvcache_real8)) deallocate(pushdata%recvcache_real8)
      pushdata%real8_batch_capacity = 0
      IF (allocated(pushdata%sendcache_real8_batch)) deallocate(pushdata%sendcache_real8_batch)
      IF (allocated(pushdata%recvcache_real8_batch)) deallocate(pushdata%recvcache_real8_batch)
      IF (allocated(pushdata%req_send_real8 )) deallocate(pushdata%req_send_real8 )
      IF (allocated(pushdata%req_recv_real8 )) deallocate(pushdata%req_recv_real8 )
      IF (allocated(pushdata%send_peer_real8)) deallocate(pushdata%send_peer_real8)
      IF (allocated(pushdata%recv_peer_real8)) deallocate(pushdata%recv_peer_real8)
      IF (allocated(pushdata%send_disp_real8)) deallocate(pushdata%send_disp_real8)
      IF (allocated(pushdata%recv_disp_real8)) deallocate(pushdata%recv_disp_real8)
#endif

   END SUBROUTINE reset_worker_push_real8_scratch

   ! ----------
   SUBROUTINE ensure_worker_push_real8_scratch (pushdata)

   IMPLICIT NONE

   type(worker_pushdata_type), intent(inout) :: pushdata

   integer :: ndatasend, ndatarecv, nsendpeer, nrecvpeer
#ifdef USEMPI
   integer :: iworker, ipeer, idsp
#endif

      IF (pushdata%real8_scratch_ready) RETURN

      IF (allocated(pushdata%recv_uniq_real8)) THEN
         IF (size(pushdata%recv_uniq_real8) /= pushdata%num_req_uniq) &
            deallocate(pushdata%recv_uniq_real8)
      ENDIF
      IF (.not. allocated(pushdata%recv_uniq_real8)) &
         allocate(pushdata%recv_uniq_real8(pushdata%num_req_uniq))

#ifdef USEMPI
      ndatasend = sum(pushdata%n_to_other)
      ndatarecv = sum(pushdata%n_from_other)
      nsendpeer = count(pushdata%n_to_other > 0)
      nrecvpeer = count(pushdata%n_from_other > 0)

      IF (allocated(pushdata%sendcache_real8)) THEN
         IF (size(pushdata%sendcache_real8) /= ndatasend) deallocate(pushdata%sendcache_real8)
      ENDIF
      IF (.not. allocated(pushdata%sendcache_real8)) allocate(pushdata%sendcache_real8(ndatasend))

      IF (allocated(pushdata%recvcache_real8)) THEN
         IF (size(pushdata%recvcache_real8) /= ndatarecv) deallocate(pushdata%recvcache_real8)
      ENDIF
      IF (.not. allocated(pushdata%recvcache_real8)) allocate(pushdata%recvcache_real8(ndatarecv))

      IF (allocated(pushdata%req_send_real8)) THEN
         IF (size(pushdata%req_send_real8) /= nsendpeer) deallocate(pushdata%req_send_real8)
      ENDIF
      IF (.not. allocated(pushdata%req_send_real8)) allocate(pushdata%req_send_real8(nsendpeer))

      IF (allocated(pushdata%req_recv_real8)) THEN
         IF (size(pushdata%req_recv_real8) /= nrecvpeer) deallocate(pushdata%req_recv_real8)
      ENDIF
      IF (.not. allocated(pushdata%req_recv_real8)) allocate(pushdata%req_recv_real8(nrecvpeer))

      allocate(pushdata%send_peer_real8(nsendpeer), pushdata%send_disp_real8(nsendpeer))
      ipeer = 0
      idsp = 0
      DO iworker = 0, p_np_worker-1
         IF (pushdata%n_to_other(iworker) > 0) THEN
            ipeer = ipeer + 1
            pushdata%send_peer_real8(ipeer) = iworker
            pushdata%send_disp_real8(ipeer) = idsp + 1
            idsp = idsp + pushdata%n_to_other(iworker)
         ENDIF
      ENDDO

      allocate(pushdata%recv_peer_real8(nrecvpeer), pushdata%recv_disp_real8(nrecvpeer))
      ipeer = 0
      idsp = 0
      DO iworker = 0, p_np_worker-1
         IF (pushdata%n_from_other(iworker) > 0) THEN
            ipeer = ipeer + 1
            pushdata%recv_peer_real8(ipeer) = iworker
            pushdata%recv_disp_real8(ipeer) = idsp + 1
            idsp = idsp + pushdata%n_from_other(iworker)
         ENDIF
      ENDDO
#endif

      pushdata%real8_scratch_ready = .true.

   END SUBROUTINE ensure_worker_push_real8_scratch

   ! ----------
   SUBROUTINE ensure_worker_push_real8_batch_scratch (pushdata, nfield)

   IMPLICIT NONE

   type(worker_pushdata_type), intent(inout) :: pushdata
   integer, intent(in) :: nfield

#ifdef USEMPI
   integer :: ndatasend, ndatarecv
#endif

      CALL ensure_worker_push_real8_scratch (pushdata)

#ifdef USEMPI
      IF (nfield <= pushdata%real8_batch_capacity) RETURN

      ndatasend = sum(pushdata%n_to_other)
      ndatarecv = sum(pushdata%n_from_other)

      IF (allocated(pushdata%sendcache_real8_batch)) deallocate(pushdata%sendcache_real8_batch)
      IF (allocated(pushdata%recvcache_real8_batch)) deallocate(pushdata%recvcache_real8_batch)
      allocate(pushdata%sendcache_real8_batch(ndatasend * nfield))
      allocate(pushdata%recvcache_real8_batch(ndatarecv * nfield))
      pushdata%real8_batch_capacity = nfield
#endif

   END SUBROUTINE ensure_worker_push_real8_batch_scratch

   ! ----------
   SUBROUTINE worker_push_data_uniq_real8 ( &
         pushdata, vec_send, fillvalue)

   IMPLICIT NONE

   type(worker_pushdata_type), intent(inout) :: pushdata

   real(r8), intent(in)   , optional :: vec_send (:)
   real(r8), intent(in)   , optional :: fillvalue

   ! Local Variables
   integer :: iworker, iproc, idsp, istt, iend, i, i_to


      IF (p_is_worker) THEN

         IF (pushdata%num_req_uniq > 0) THEN
            pushdata%recv_uniq_real8 = fillvalue
         ENDIF

         IF (pushdata%nself > 0) THEN
            pushdata%recv_uniq_real8(pushdata%self_to) = vec_send(pushdata%self_from)
         ENDIF

#ifdef USEMPI
         IF (size(pushdata%send_peer_real8) > 0) THEN
            DO iproc = 1, size(pushdata%send_peer_real8)
                  iworker = pushdata%send_peer_real8(iproc)
                  istt  = pushdata%send_disp_real8(iproc)
                  iend  = istt + pushdata%n_to_other(iworker) - 1

                  pushdata%sendcache_real8(istt:iend) = vec_send(pushdata%to_other(iworker)%val)
                  CALL mpi_isend(pushdata%sendcache_real8(istt:iend), pushdata%n_to_other(iworker), MPI_REAL8, &
                     iworker, 101, p_comm_worker, pushdata%req_send_real8(iproc), p_err)
            ENDDO
         ENDIF

         IF (size(pushdata%recv_peer_real8) > 0) THEN
            DO iproc = 1, size(pushdata%recv_peer_real8)
                  iworker = pushdata%recv_peer_real8(iproc)
                  istt  = pushdata%recv_disp_real8(iproc)
                  iend  = istt + pushdata%n_from_other(iworker) - 1

                  CALL mpi_irecv(pushdata%recvcache_real8(istt:iend), pushdata%n_from_other(iworker), MPI_REAL8, &
                     iworker, 101, p_comm_worker, pushdata%req_recv_real8(iproc), p_err)
            ENDDO
         ENDIF

         IF (size(pushdata%recv_peer_real8) > 0) THEN

            CALL mpi_waitall(size(pushdata%req_recv_real8), pushdata%req_recv_real8, MPI_STATUSES_IGNORE, p_err)

            DO iproc = 1, size(pushdata%recv_peer_real8)
               iworker = pushdata%recv_peer_real8(iproc)
               idsp = pushdata%recv_disp_real8(iproc) - 1
               DO i = 1, pushdata%n_from_other(iworker)

                  IF (pushdata%recvcache_real8(idsp+i) /= fillvalue) THEN
                     i_to = pushdata%other_to(iworker)%val(i)
                     IF (pushdata%recv_uniq_real8(i_to) == fillvalue) THEN
                        pushdata%recv_uniq_real8(i_to) = pushdata%recvcache_real8(idsp+i)
                     ELSE
                        pushdata%recv_uniq_real8(i_to) = pushdata%recv_uniq_real8(i_to) &
                           + pushdata%recvcache_real8(idsp+i)
                     ENDIF
                  ENDIF

               ENDDO
            ENDDO

         ENDIF

         IF (size(pushdata%send_peer_real8) > 0) THEN
            CALL mpi_waitall(size(pushdata%req_send_real8), pushdata%req_send_real8, MPI_STATUSES_IGNORE, p_err)
         ENDIF
#endif

      ENDIF

   END SUBROUTINE worker_push_data_uniq_real8

   ! ----------
   SUBROUTINE worker_push_data_uniq_int32 ( &
         pushdata, vec_send, vec_recv, fillvalue)

   IMPLICIT NONE

   type(worker_pushdata_type), intent(in) :: pushdata

   integer, intent(in)   , optional :: vec_send (:)
   integer, intent(inout), optional :: vec_recv (:)
   integer, intent(in)   , optional :: fillvalue

   ! Local Variables
   integer :: ndatasend
   integer, allocatable :: req_send  (:)
   integer, allocatable :: sendcache (:)

   integer :: ndatarecv
   integer, allocatable :: req_recv  (:)
   integer, allocatable :: recvcache (:)

   integer :: iworker, iproc, idsp, istt, iend, i, i_to


      IF (p_is_worker) THEN

         IF (pushdata%num_req_uniq > 0) THEN
            vec_recv = fillvalue
         ENDIF

         IF (pushdata%nself > 0) THEN
            vec_recv(pushdata%self_to) = vec_send(pushdata%self_from)
         ENDIF

#ifdef USEMPI
         ndatasend = sum(pushdata%n_to_other)
         IF (ndatasend > 0) THEN

            allocate (sendcache(ndatasend))
            allocate (req_send (count(pushdata%n_to_other > 0)))

            iproc = 0
            idsp  = 0
            DO iworker = 0, p_np_worker-1
               IF (pushdata%n_to_other(iworker) > 0) THEN
                  iproc = iproc + 1
                  istt  = idsp + 1
                  iend  = idsp + pushdata%n_to_other(iworker)

                  sendcache(istt:iend) = vec_send(pushdata%to_other(iworker)%val)
                  CALL mpi_isend(sendcache(istt:iend), pushdata%n_to_other(iworker), MPI_INTEGER, &
                     iworker, 101, p_comm_worker, req_send(iproc), p_err)

                  idsp = iend
               ENDIF
            ENDDO
         ENDIF

         ndatarecv = sum(pushdata%n_from_other)
         IF (ndatarecv > 0) THEN

            allocate (recvcache(ndatarecv))
            allocate (req_recv (count(pushdata%n_from_other > 0)))

            iproc = 0
            idsp  = 0
            DO iworker = 0, p_np_worker-1
               IF (pushdata%n_from_other(iworker) > 0) THEN
                  iproc = iproc + 1
                  istt  = idsp + 1
                  iend  = idsp + pushdata%n_from_other(iworker)

                  CALL mpi_irecv(recvcache(istt:iend), pushdata%n_from_other(iworker), MPI_INTEGER, &
                     iworker, 101, p_comm_worker, req_recv(iproc), p_err)

                  idsp = iend
               ENDIF
            ENDDO
         ENDIF

         IF (ndatarecv > 0) THEN

            CALL mpi_waitall(size(req_recv), req_recv, MPI_STATUSES_IGNORE, p_err)

            idsp = 0
            DO iworker = 0, p_np_worker-1
               DO i = 1, pushdata%n_from_other(iworker)

                  IF (recvcache(idsp+i) /= fillvalue) THEN
                     i_to = pushdata%other_to(iworker)%val(i)
                     IF (vec_recv(i_to) == fillvalue) THEN
                        vec_recv(i_to) = recvcache(idsp+i)
                     ELSE
                        vec_recv(i_to) = vec_recv(i_to) + recvcache(idsp+i)
                     ENDIF
                  ENDIF

               ENDDO
               idsp = idsp + pushdata%n_from_other(iworker)
            ENDDO

         ENDIF

         IF (ndatasend > 0) THEN
            CALL mpi_waitall(size(req_send), req_send, MPI_STATUSES_IGNORE, p_err)
         ENDIF

         IF (allocated(req_send )) deallocate(req_send )
         IF (allocated(sendcache)) deallocate(sendcache)
         IF (allocated(req_recv )) deallocate(req_recv )
         IF (allocated(recvcache)) deallocate(recvcache)
#endif

      ENDIF

   END SUBROUTINE worker_push_data_uniq_int32

   ! ----------
   SUBROUTINE worker_push_data_single_real8 (pushdata, vec_send, vec_recv, fillvalue)

   IMPLICIT NONE

   type(worker_pushdata_type), intent(inout) :: pushdata

   real(r8), intent(in)    :: vec_send (:)
   real(r8), intent(inout) :: vec_recv (:)
   real(r8), intent(in)    :: fillvalue

      IF (p_is_worker) THEN

         CALL ensure_worker_push_real8_scratch (pushdata)
         IF (pushdata%num_req_uniq > 0) THEN
            pushdata%recv_uniq_real8(:) = fillvalue
         ENDIF

         CALL worker_push_data_uniq_real8 (pushdata, vec_send, fillvalue)

         IF (pushdata%num_req_uniq > 0) THEN
            vec_recv = pushdata%recv_uniq_real8(pushdata%addr_single)
         ENDIF

      ENDIF

   END SUBROUTINE worker_push_data_single_real8

   ! ----------
   SUBROUTINE worker_push_data_multi_real8 (pushdata, vec_send, vec_recv, fillvalue, mode)

   IMPLICIT NONE

   type(worker_pushdata_type), intent(inout) :: pushdata

   real(r8), intent(in)    :: vec_send (:)
   real(r8), intent(inout) :: vec_recv (:)
   real(r8), intent(in)    :: fillvalue

   character(len=*), intent(in) :: mode

   ! Local Variables
   integer  :: i, j
   real(r8) :: val, sumarea

      IF (p_is_worker) THEN

         CALL ensure_worker_push_real8_scratch (pushdata)
         vec_recv(:) = fillvalue
         IF (pushdata%num_req_uniq > 0) THEN
            pushdata%recv_uniq_real8(:) = fillvalue
         ENDIF

         CALL worker_push_data_uniq_real8 (pushdata, vec_send, fillvalue)

         IF (pushdata%num_req_uniq > 0) THEN
            DO j = 1, size(pushdata%addr_multi,2)

               sumarea = 0.

               DO i = 1, size(pushdata%addr_multi,1)
                  IF (pushdata%area_multi(i,j) <= 0._r8) CYCLE
                  val = pushdata%recv_uniq_real8(pushdata%addr_multi(i,j))
                  IF (val /= fillvalue) THEN
                     IF (vec_recv(j) == fillvalue) THEN
                        vec_recv(j) = val * pushdata%area_multi(i,j)
                     ELSE
                        vec_recv(j) = vec_recv(j) + val * pushdata%area_multi(i,j)
                     ENDIF
                     sumarea = sumarea + pushdata%area_multi(i,j)
                  ENDIF
               ENDDO

               IF (trim(mode) == 'average') THEN
                  IF (vec_recv(j) /= fillvalue .and. sumarea > 0._r8) THEN
                     vec_recv(j) = vec_recv(j) / sumarea
                  ENDIF
               ENDIF
            ENDDO

         ENDIF
      ENDIF

   END SUBROUTINE worker_push_data_multi_real8

   ! ----------
   SUBROUTINE validate_worker_push_real8_batch (pushdata, fields, mode)

   IMPLICIT NONE

   type(worker_pushdata_type), intent(in) :: pushdata
   type(worker_push_real8_field_type), intent(in) :: fields(:)
   character(len=*), intent(in), optional :: mode

   integer :: expected_recv_size, ifield

      SELECT CASE (pushdata%mapping_kind)
      CASE (WORKER_PUSH_MAPPING_SINGLE)
         IF (present(mode)) &
            CALL CoLM_stop('worker_push_data: mode is invalid for a batched single mapping')
         IF (pushdata%num_req_uniq > 0 .and. .not. allocated(pushdata%addr_single)) &
            CALL CoLM_stop('worker_push_data: incomplete batched single mapping')
         expected_recv_size = 0
         IF (allocated(pushdata%addr_single)) expected_recv_size = size(pushdata%addr_single)

      CASE (WORKER_PUSH_MAPPING_MULTI)
         IF (.not. present(mode)) &
            CALL CoLM_stop('worker_push_data: mode is required for a batched multi mapping')
         IF (pushdata%num_req_uniq > 0 .and. &
             (.not. allocated(pushdata%addr_multi) .or. .not. allocated(pushdata%area_multi))) &
            CALL CoLM_stop('worker_push_data: incomplete batched multi mapping')
         expected_recv_size = 0
         IF (allocated(pushdata%addr_multi)) expected_recv_size = size(pushdata%addr_multi, 2)

      CASE DEFAULT
         CALL CoLM_stop('worker_push_data: batch used with an untyped push mapping')
      END SELECT

      DO ifield = 1, size(fields)
         IF (.not. associated(fields(ifield)%send) .or. &
             .not. associated(fields(ifield)%recv)) &
            CALL CoLM_stop('worker_push_data: null field in real8 batch')
         IF (size(fields(ifield)%send) < pushdata%required_send_size) &
            CALL CoLM_stop('worker_push_data: short send field in real8 batch')
         IF (size(fields(ifield)%recv) /= expected_recv_size) &
            CALL CoLM_stop('worker_push_data: wrong receive field size in real8 batch')
      ENDDO

   END SUBROUTINE validate_worker_push_real8_batch

   ! ----------
   SUBROUTINE worker_push_data_multi_real8_batch (pushdata, fields, mode)

   IMPLICIT NONE

   type(worker_pushdata_type), intent(inout) :: pushdata
   type(worker_push_real8_field_type), intent(in) :: fields(:)
   character(len=*), intent(in), optional :: mode

   integer :: ifield, i, j, i_to
   real(r8) :: val, sumarea
   character(len=16) :: batch_mode
#ifdef USEMPI
   integer :: iworker, iproc, npeer, ibase, istt, iend
#endif

      IF (p_is_worker) THEN

         IF (size(fields) <= 0) RETURN
         CALL validate_worker_push_real8_batch (pushdata, fields, mode)
         CALL ensure_worker_push_real8_batch_scratch (pushdata, size(fields))
         batch_mode = ''
         IF (present(mode)) batch_mode = mode

         DO ifield = 1, size(fields)
            fields(ifield)%recv(:) = fields(ifield)%fillvalue
         ENDDO

#ifdef USEMPI
         IF (size(pushdata%send_peer_real8) > 0) THEN
            DO iproc = 1, size(pushdata%send_peer_real8)
               iworker = pushdata%send_peer_real8(iproc)
               npeer = pushdata%n_to_other(iworker)
               ibase = 1 + size(fields) * (pushdata%send_disp_real8(iproc) - 1)

               DO ifield = 1, size(fields)
                  istt = ibase + (ifield - 1) * npeer
                  iend = istt + npeer - 1
                  pushdata%sendcache_real8_batch(istt:iend) = &
                     fields(ifield)%send(pushdata%to_other(iworker)%val)
               ENDDO

               CALL mpi_isend(pushdata%sendcache_real8_batch(ibase:ibase+size(fields)*npeer-1), &
                  size(fields)*npeer, MPI_REAL8, iworker, 101, p_comm_worker, &
                  pushdata%req_send_real8(iproc), p_err)
            ENDDO
         ENDIF

         IF (size(pushdata%recv_peer_real8) > 0) THEN
            DO iproc = 1, size(pushdata%recv_peer_real8)
               iworker = pushdata%recv_peer_real8(iproc)
               npeer = pushdata%n_from_other(iworker)
               ibase = 1 + size(fields) * (pushdata%recv_disp_real8(iproc) - 1)

               CALL mpi_irecv(pushdata%recvcache_real8_batch(ibase:ibase+size(fields)*npeer-1), &
                  size(fields)*npeer, MPI_REAL8, iworker, 101, p_comm_worker, &
                  pushdata%req_recv_real8(iproc), p_err)
            ENDDO

            CALL mpi_waitall(size(pushdata%req_recv_real8), pushdata%req_recv_real8, &
               MPI_STATUSES_IGNORE, p_err)
         ENDIF
#endif

         IF (pushdata%num_req_uniq > 0) THEN
            DO ifield = 1, size(fields)
               pushdata%recv_uniq_real8(:) = fields(ifield)%fillvalue

               IF (pushdata%nself > 0) THEN
                  pushdata%recv_uniq_real8(pushdata%self_to) = &
                     fields(ifield)%send(pushdata%self_from)
               ENDIF

#ifdef USEMPI
               DO iproc = 1, size(pushdata%recv_peer_real8)
                  iworker = pushdata%recv_peer_real8(iproc)
                  npeer = pushdata%n_from_other(iworker)
                  ibase = 1 + size(fields) * (pushdata%recv_disp_real8(iproc) - 1)
                  istt = ibase + (ifield - 1) * npeer

                  DO i = 1, npeer
                     val = pushdata%recvcache_real8_batch(istt+i-1)
                     IF (val /= fields(ifield)%fillvalue) THEN
                        i_to = pushdata%other_to(iworker)%val(i)
                        IF (pushdata%recv_uniq_real8(i_to) == fields(ifield)%fillvalue) THEN
                           pushdata%recv_uniq_real8(i_to) = val
                        ELSE
                           pushdata%recv_uniq_real8(i_to) = pushdata%recv_uniq_real8(i_to) + val
                        ENDIF
                     ENDIF
                  ENDDO
               ENDDO
#endif

               SELECT CASE (pushdata%mapping_kind)
               CASE (WORKER_PUSH_MAPPING_SINGLE)
                  fields(ifield)%recv = pushdata%recv_uniq_real8(pushdata%addr_single)

               CASE (WORKER_PUSH_MAPPING_MULTI)
                  DO j = 1, size(pushdata%addr_multi,2)
                     sumarea = 0._r8
                     DO i = 1, size(pushdata%addr_multi,1)
                        IF (pushdata%area_multi(i,j) <= 0._r8) CYCLE
                        val = pushdata%recv_uniq_real8(pushdata%addr_multi(i,j))
                        IF (val /= fields(ifield)%fillvalue) THEN
                           IF (fields(ifield)%recv(j) == fields(ifield)%fillvalue) THEN
                              fields(ifield)%recv(j) = val * pushdata%area_multi(i,j)
                           ELSE
                              fields(ifield)%recv(j) = fields(ifield)%recv(j) + &
                                 val * pushdata%area_multi(i,j)
                           ENDIF
                           sumarea = sumarea + pushdata%area_multi(i,j)
                        ENDIF
                     ENDDO

                     IF (trim(batch_mode) == 'average') THEN
                        IF (fields(ifield)%recv(j) /= fields(ifield)%fillvalue .and. &
                            sumarea > 0._r8) THEN
                           fields(ifield)%recv(j) = fields(ifield)%recv(j) / sumarea
                        ENDIF
                     ENDIF
                  ENDDO
               END SELECT
            ENDDO
         ENDIF

#ifdef USEMPI
         IF (size(pushdata%send_peer_real8) > 0) THEN
            CALL mpi_waitall(size(pushdata%req_send_real8), pushdata%req_send_real8, &
               MPI_STATUSES_IGNORE, p_err)
         ENDIF
#endif

      ENDIF

   END SUBROUTINE worker_push_data_multi_real8_batch

   ! ----------
   SUBROUTINE worker_push_data_single_int32 (pushdata, vec_send, vec_recv, fillvalue)

   IMPLICIT NONE

   type(worker_pushdata_type) :: pushdata

   integer, intent(in)    :: vec_send (:)
   integer, intent(inout) :: vec_recv (:)
   integer, intent(in)    :: fillvalue

   ! Local Variables
   integer, allocatable   :: vec_recv_uniq (:)

      IF (p_is_worker) THEN

         allocate (vec_recv_uniq (pushdata%num_req_uniq))
         IF (pushdata%num_req_uniq > 0) THEN
            vec_recv_uniq(:) = fillvalue
         ENDIF

         CALL worker_push_data_uniq_int32 (pushdata, vec_send, vec_recv_uniq, fillvalue)

         IF (pushdata%num_req_uniq > 0) THEN
            vec_recv = vec_recv_uniq(pushdata%addr_single)
         ENDIF
         deallocate (vec_recv_uniq)

      ENDIF

   END SUBROUTINE worker_push_data_single_int32

   ! ---------
   SUBROUTINE worker_remap_data_pset2grid_real8 (remapdata, vec_in, vec_out, fillvalue, filter)

   IMPLICIT NONE

   type(worker_remapdata_type), intent(in) :: remapdata

   real(r8), intent(in)    :: vec_in (:)
   real(r8), intent(inout) :: vec_out(:)
   real(r8), intent(in)    :: fillvalue
   logical,  intent(in)    :: filter (:)

   ! Local Variables
   integer  :: iset, ipart, iloc
   real(r8) :: area


      IF (p_is_worker) THEN
         IF (remapdata%num_grid > 0) THEN

            vec_out(:) = fillvalue

            DO iset = 1, remapdata%npset
               IF (filter(iset) .and. (vec_in(iset) /= fillvalue)) THEN
                  DO ipart = 1, remapdata%npart(iset)

                     iloc = remapdata%part_to(iset)%val(ipart)
                     area = remapdata%areapart(iset)%val(ipart)

                     IF (vec_out(iloc) == fillvalue) THEN
                        vec_out(iloc) = vec_in(iset) * area
                     ELSE
                        vec_out(iloc) = vec_out(iloc) + vec_in(iset) * area
                     ENDIF

                  ENDDO
               ENDIF
            ENDDO

         ENDIF
      ENDIF

   END SUBROUTINE worker_remap_data_pset2grid_real8

   ! ---------
   SUBROUTINE worker_remap_data_grid2pset_real8 (remapdata, vec_in, vec_out, fillvalue, mode)

   IMPLICIT NONE

   type(worker_remapdata_type), intent(in) :: remapdata

   real(r8), intent(in)    :: vec_in (:)
   real(r8), intent(inout) :: vec_out(:)
   real(r8), intent(in)    :: fillvalue

   character(len=*), intent(in) :: mode

   ! Local Variables
   integer  :: iset, ipart, iloc
   real(r8) :: area, sumarea

      IF (p_is_worker) THEN
         IF (remapdata%npset > 0) THEN

            vec_out(:) = fillvalue

            DO iset = 1, remapdata%npset

               sumarea = 0.

               DO ipart = 1, remapdata%npart(iset)
                  iloc = remapdata%part_to(iset)%val(ipart)
                  area = remapdata%areapart(iset)%val(ipart)
                  IF (area <= 0._r8) CYCLE

                  IF (vec_in(iloc) /= fillvalue) THEN
                     IF (vec_out(iset) == fillvalue) THEN
                        vec_out(iset) = vec_in(iloc) * area
                     ELSE
                        vec_out(iset) = vec_out(iset) + vec_in(iloc) * area
                     ENDIF
                     sumarea = sumarea + area
                  ENDIF
               ENDDO

               IF (trim(mode) == 'average') THEN
                  IF (vec_out(iset) /= fillvalue .and. sumarea > 0._r8) THEN
                     vec_out(iset) = vec_out(iset) / sumarea
                  ENDIF
               ENDIF
            ENDDO

         ENDIF
      ENDIF

   END SUBROUTINE worker_remap_data_grid2pset_real8

   ! ---------
   SUBROUTINE worker_pushdata_free_mem (this)

   IMPLICIT NONE
   type(worker_pushdata_type) :: this

      IF (allocated(this%addr_single )) deallocate(this%addr_single )
      IF (allocated(this%addr_multi  )) deallocate(this%addr_multi  )
      IF (allocated(this%area_multi  )) deallocate(this%area_multi  )
      IF (allocated(this%sum_area    )) deallocate(this%sum_area    )
      IF (allocated(this%self_from   )) deallocate(this%self_from   )
      IF (allocated(this%self_to     )) deallocate(this%self_to     )
      IF (allocated(this%recv_uniq_real8)) deallocate(this%recv_uniq_real8)
#ifdef USEMPI
      IF (allocated(this%n_to_other  )) deallocate(this%n_to_other  )
      IF (allocated(this%n_from_other)) deallocate(this%n_from_other)
      IF (allocated(this%to_other    )) deallocate(this%to_other    )
      IF (allocated(this%other_to    )) deallocate(this%other_to    )
      IF (allocated(this%sendcache_real8)) deallocate(this%sendcache_real8)
      IF (allocated(this%recvcache_real8)) deallocate(this%recvcache_real8)
      IF (allocated(this%sendcache_real8_batch)) deallocate(this%sendcache_real8_batch)
      IF (allocated(this%recvcache_real8_batch)) deallocate(this%recvcache_real8_batch)
      IF (allocated(this%req_send_real8 )) deallocate(this%req_send_real8 )
      IF (allocated(this%req_recv_real8 )) deallocate(this%req_recv_real8 )
      IF (allocated(this%send_peer_real8)) deallocate(this%send_peer_real8)
      IF (allocated(this%recv_peer_real8)) deallocate(this%recv_peer_real8)
      IF (allocated(this%send_disp_real8)) deallocate(this%send_disp_real8)
      IF (allocated(this%recv_disp_real8)) deallocate(this%recv_disp_real8)
#endif
      this%mapping_kind = WORKER_PUSH_MAPPING_UNSET
      this%required_send_size = 0

   END SUBROUTINE worker_pushdata_free_mem

   ! ---------
   SUBROUTINE worker_remapdata_free_mem (this)

   IMPLICIT NONE
   type(worker_remapdata_type) :: this

      IF (allocated(this%ilon_me )) deallocate(this%ilon_me )
      IF (allocated(this%ilat_me )) deallocate(this%ilat_me )
      IF (allocated(this%ids_me  )) deallocate(this%ids_me  )
      IF (allocated(this%npart   )) deallocate(this%npart   )
      IF (allocated(this%sum_area)) deallocate(this%sum_area)
      IF (allocated(this%part_to )) deallocate(this%part_to )
      IF (allocated(this%areapart)) deallocate(this%areapart)

   END SUBROUTINE worker_remapdata_free_mem

END MODULE MOD_WorkerPushData
