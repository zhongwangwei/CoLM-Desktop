#include <define.h>

#ifdef GridRiverLakeFlow
MODULE MOD_Grid_RiverLakePush
!--------------------------------------------------------------------------------
! DESCRIPTION:
!
!    Batched pushes for the grid-based river-lake routing.
!
!    The routing exchanges several fields over the same push mapping many times
!    per routing substep (bifurcation pathway states, upstream sums).  Pushing
!    them one by one with worker_push_data costs one exchange per field and one
!    pass over a dense (largest upstream count) x (requests) table per field.
!    riverlake_push_batch pushes a set of real8 fields in one exchange and sums
!    multi mappings over compressed rows (CSR) holding only the entries that
!    have a sender.
!
!    It only reads the public components of a worker_pushdata_type built by
!    build_worker_pushdata (MOD_WorkerPushData), so the shared module is not
!    changed.  Build a riverlake_pushcache_type once after the push mapping with
!    riverlake_pushcache_build, and rebuild it whenever the mapping is rebuilt.
!
!    Results of a batch:
!     - single mapping: equal, bit for bit, to worker_push_data for each field;
!     - multi mapping: entries of non-positive area are skipped and the receive
!       vector is always reset to the fill value.  This equals worker_push_data
!       bit for bit when cache%batch_exact is true (all kept areas are positive
!       on all workers), which is the case for push_ups2ucat (weights 1).
!--------------------------------------------------------------------------------

   USE MOD_Precision
   USE MOD_SPMD_Task
   USE MOD_WorkerPushData
   IMPLICIT NONE

   integer, parameter, private :: PUSH_MAPPING_EMPTY  = 0
   integer, parameter, private :: PUSH_MAPPING_SINGLE = 1
   integer, parameter, private :: PUSH_MAPPING_MULTI  = 2

   ! -- one real8 field of a batch (does not own its data) --
   type :: riverlake_push_field_type
      real(r8), pointer :: send(:) => null()
      real(r8), pointer :: recv(:) => null()
      real(r8) :: fillvalue = 0._r8
   END type riverlake_push_field_type

   ! -- derived data and buffers for batched pushes over one push mapping --
   type :: riverlake_pushcache_type

      logical :: ready = .false.
      integer :: mapping_kind = PUSH_MAPPING_EMPTY
      integer :: required_send_size = 0
      integer :: num_recv = 0

      ! multi mapping in compressed rows: request j reads entries
      !    ptr(j) : ptr(j+1)-1   of addr / area
      integer,  allocatable :: ptr  (:)
      integer,  allocatable :: addr (:)
      real(r8), allocatable :: area (:)
      logical :: batch_exact = .false.

      real(r8), allocatable :: recv_uniq (:)
#ifdef USEMPI
      integer :: batch_capacity = 0
      real(r8), allocatable :: sendcache (:), recvcache (:)
      integer,  allocatable :: req_send  (:), req_recv  (:)
      integer,  allocatable :: send_peer (:), recv_peer (:)
      integer,  allocatable :: send_disp (:), recv_disp (:)
#endif
   CONTAINS
      final :: riverlake_pushcache_free_mem
   END type riverlake_pushcache_type

   PUBLIC :: riverlake_pushcache_build
   PUBLIC :: riverlake_push_batch
   PUBLIC :: riverlake_pushcache_free_mem

CONTAINS

   ! ----------
   SUBROUTINE riverlake_pushcache_build (pushdata, cache)

   ! Collective over workers: call on all workers after build_worker_pushdata.

   IMPLICIT NONE

   type(worker_pushdata_type),     intent(in)    :: pushdata
   type(riverlake_pushcache_type), intent(inout) :: cache

   ! Local Variables
   logical, allocatable :: id_found (:)
   integer :: i, j, k, nrow, ndim1
#ifdef USEMPI
   integer :: iworker, ipeer, idsp, nsendpeer, nrecvpeer
#endif

      CALL riverlake_pushcache_free_mem (cache)

      IF (.not. p_is_worker) RETURN

      ! largest send index used by the mapping
      cache%required_send_size = 0
      IF (pushdata%nself > 0) cache%required_send_size = maxval(pushdata%self_from)
#ifdef USEMPI
      DO iworker = 0, p_np_worker-1
         IF (pushdata%n_to_other(iworker) > 0) THEN
            cache%required_send_size = max(cache%required_send_size, &
               maxval(pushdata%to_other(iworker)%val))
         ENDIF
      ENDDO
#endif

      cache%batch_exact = .true.

      IF (allocated(pushdata%addr_multi)) THEN

         cache%mapping_kind = PUSH_MAPPING_MULTI
         ndim1 = size(pushdata%addr_multi,1)
         nrow  = size(pushdata%addr_multi,2)
         cache%num_recv = nrow

         ! Sources that have a sender, as in build_worker_pushdata_multi.
         ! A source without sender stays at the fill value after the exchange,
         ! so worker_push_data never adds it; it is left out of the rows.
         ! Kept entries stay in their original order inside each request, so
         ! every sum is formed in the same order as in worker_push_data.
         allocate (id_found (pushdata%num_req_uniq))
         id_found(:) = .false.
         IF (pushdata%nself > 0) id_found(pushdata%self_to) = .true.
#ifdef USEMPI
         DO iworker = 0, p_np_worker-1
            IF (pushdata%n_from_other(iworker) > 0) THEN
               id_found(pushdata%other_to(iworker)%val) = .true.
            ENDIF
         ENDDO
#endif

         k = 0
         DO j = 1, nrow
            DO i = 1, ndim1
               IF (id_found(pushdata%addr_multi(i,j))) k = k + 1
            ENDDO
         ENDDO
         allocate (cache%ptr  (nrow+1))
         allocate (cache%addr (k))
         allocate (cache%area (k))
         k = 0
         DO j = 1, nrow
            cache%ptr(j) = k + 1
            DO i = 1, ndim1
               IF (id_found(pushdata%addr_multi(i,j))) THEN
                  k = k + 1
                  cache%addr(k) = pushdata%addr_multi(i,j)
                  cache%area(k) = pushdata%area_multi(i,j)
               ENDIF
            ENDDO
         ENDDO
         cache%ptr(nrow+1) = k + 1

         cache%batch_exact = all(cache%area > 0._r8)

         deallocate (id_found)

      ELSEIF (allocated(pushdata%addr_single)) THEN
         cache%mapping_kind = PUSH_MAPPING_SINGLE
         cache%num_recv = size(pushdata%addr_single)
      ELSE
         ! no request on this worker: it only sends
         cache%mapping_kind = PUSH_MAPPING_EMPTY
         cache%num_recv = 0
      ENDIF

#ifdef USEMPI
      ! Every worker must take the same path (batched or not) in an exchange.
      CALL mpi_allreduce (MPI_IN_PLACE, cache%batch_exact, 1, MPI_LOGICAL, MPI_LAND, &
         p_comm_worker, p_err)
#endif

      allocate (cache%recv_uniq (pushdata%num_req_uniq))

#ifdef USEMPI
      nsendpeer = count(pushdata%n_to_other   > 0)
      nrecvpeer = count(pushdata%n_from_other > 0)

      allocate (cache%req_send  (nsendpeer))
      allocate (cache%req_recv  (nrecvpeer))
      allocate (cache%send_peer (nsendpeer), cache%send_disp (nsendpeer))
      allocate (cache%recv_peer (nrecvpeer), cache%recv_disp (nrecvpeer))

      ipeer = 0
      idsp  = 0
      DO iworker = 0, p_np_worker-1
         IF (pushdata%n_to_other(iworker) > 0) THEN
            ipeer = ipeer + 1
            cache%send_peer(ipeer) = iworker
            cache%send_disp(ipeer) = idsp + 1
            idsp = idsp + pushdata%n_to_other(iworker)
         ENDIF
      ENDDO

      ipeer = 0
      idsp  = 0
      DO iworker = 0, p_np_worker-1
         IF (pushdata%n_from_other(iworker) > 0) THEN
            ipeer = ipeer + 1
            cache%recv_peer(ipeer) = iworker
            cache%recv_disp(ipeer) = idsp + 1
            idsp = idsp + pushdata%n_from_other(iworker)
         ENDIF
      ENDDO
#endif

      cache%ready = .true.

   END SUBROUTINE riverlake_pushcache_build

   ! ----------
   SUBROUTINE riverlake_push_check (cache, fields, mode)

   IMPLICIT NONE

   type(riverlake_pushcache_type),  intent(in) :: cache
   type(riverlake_push_field_type), intent(in) :: fields(:)
   character(len=*), intent(in), optional :: mode

   integer :: ifield

      IF (.not. cache%ready) &
         CALL CoLM_stop ('riverlake_push_batch: push cache is not built')

      SELECT CASE (cache%mapping_kind)
      CASE (PUSH_MAPPING_SINGLE)
         IF (present(mode)) &
            CALL CoLM_stop ('riverlake_push_batch: mode is invalid for a single mapping')
      CASE (PUSH_MAPPING_MULTI)
         IF (.not. present(mode)) &
            CALL CoLM_stop ('riverlake_push_batch: mode is required for a multi mapping')
      END SELECT

      DO ifield = 1, size(fields)
         IF (.not. associated(fields(ifield)%send) .or. &
             .not. associated(fields(ifield)%recv)) &
            CALL CoLM_stop ('riverlake_push_batch: null field')
         IF (size(fields(ifield)%send) < cache%required_send_size) &
            CALL CoLM_stop ('riverlake_push_batch: short send field')
         IF (size(fields(ifield)%recv) /= cache%num_recv) &
            CALL CoLM_stop ('riverlake_push_batch: wrong receive field size')
      ENDDO

   END SUBROUTINE riverlake_push_check

   ! ----------
   SUBROUTINE riverlake_push_batch (pushdata, cache, fields, mode)

   IMPLICIT NONE

   type(worker_pushdata_type),      intent(in)    :: pushdata
   type(riverlake_pushcache_type),  intent(inout) :: cache
   type(riverlake_push_field_type), intent(in)    :: fields(:)
   character(len=*), intent(in), optional :: mode

   ! Local Variables
   integer  :: nfield, ifield, i, j, k, i_to
   real(r8) :: val, sumarea
   logical  :: do_average
#ifdef USEMPI
   integer  :: iworker, iproc, npeer, ibase, istt, iend
#endif

      IF (.not. p_is_worker) RETURN

      nfield = size(fields)
      IF (nfield <= 0) RETURN

      CALL riverlake_push_check (cache, fields, mode)

      do_average = .false.
      IF (present(mode)) do_average = (trim(mode) == 'average')

#ifdef USEMPI
      IF (nfield > cache%batch_capacity) THEN
         IF (allocated(cache%sendcache)) deallocate (cache%sendcache)
         IF (allocated(cache%recvcache)) deallocate (cache%recvcache)
         allocate (cache%sendcache (sum(pushdata%n_to_other  ) * nfield))
         allocate (cache%recvcache (sum(pushdata%n_from_other) * nfield))
         cache%batch_capacity = nfield
      ENDIF
#endif

      DO ifield = 1, nfield
         fields(ifield)%recv(:) = fields(ifield)%fillvalue
      ENDDO

#ifdef USEMPI
      ! one message per peer holding all fields, field after field
      DO iproc = 1, size(cache%send_peer)
         iworker = cache%send_peer(iproc)
         npeer   = pushdata%n_to_other(iworker)
         ibase   = 1 + nfield * (cache%send_disp(iproc) - 1)

         DO ifield = 1, nfield
            istt = ibase + (ifield - 1) * npeer
            iend = istt + npeer - 1
            cache%sendcache(istt:iend) = fields(ifield)%send(pushdata%to_other(iworker)%val)
         ENDDO

         CALL mpi_isend (cache%sendcache(ibase:ibase+nfield*npeer-1), nfield*npeer, MPI_REAL8, &
            iworker, 101, p_comm_worker, cache%req_send(iproc), p_err)
      ENDDO

      DO iproc = 1, size(cache%recv_peer)
         iworker = cache%recv_peer(iproc)
         npeer   = pushdata%n_from_other(iworker)
         ibase   = 1 + nfield * (cache%recv_disp(iproc) - 1)

         CALL mpi_irecv (cache%recvcache(ibase:ibase+nfield*npeer-1), nfield*npeer, MPI_REAL8, &
            iworker, 101, p_comm_worker, cache%req_recv(iproc), p_err)
      ENDDO

      IF (size(cache%req_recv) > 0) THEN
         CALL mpi_waitall (size(cache%req_recv), cache%req_recv, MPI_STATUSES_IGNORE, p_err)
      ENDIF
#endif

      IF (pushdata%num_req_uniq > 0) THEN
         DO ifield = 1, nfield

            ! the same steps as worker_push_data_uniq_real8
            cache%recv_uniq(:) = fields(ifield)%fillvalue

            IF (pushdata%nself > 0) THEN
               cache%recv_uniq(pushdata%self_to) = fields(ifield)%send(pushdata%self_from)
            ENDIF

#ifdef USEMPI
            DO iproc = 1, size(cache%recv_peer)
               iworker = cache%recv_peer(iproc)
               npeer   = pushdata%n_from_other(iworker)
               istt    = 1 + nfield * (cache%recv_disp(iproc) - 1) + (ifield - 1) * npeer

               DO i = 1, npeer
                  val = cache%recvcache(istt+i-1)
                  IF (val /= fields(ifield)%fillvalue) THEN
                     i_to = pushdata%other_to(iworker)%val(i)
                     IF (cache%recv_uniq(i_to) == fields(ifield)%fillvalue) THEN
                        cache%recv_uniq(i_to) = val
                     ELSE
                        cache%recv_uniq(i_to) = cache%recv_uniq(i_to) + val
                     ENDIF
                  ENDIF
               ENDDO
            ENDDO
#endif

            SELECT CASE (cache%mapping_kind)
            CASE (PUSH_MAPPING_SINGLE)

               fields(ifield)%recv = cache%recv_uniq(pushdata%addr_single)

            CASE (PUSH_MAPPING_MULTI)

               DO j = 1, cache%num_recv
                  sumarea = 0._r8
                  DO k = cache%ptr(j), cache%ptr(j+1)-1
                     IF (cache%area(k) <= 0._r8) CYCLE
                     val = cache%recv_uniq(cache%addr(k))
                     IF (val /= fields(ifield)%fillvalue) THEN
                        IF (fields(ifield)%recv(j) == fields(ifield)%fillvalue) THEN
                           fields(ifield)%recv(j) = val * cache%area(k)
                        ELSE
                           fields(ifield)%recv(j) = fields(ifield)%recv(j) + val * cache%area(k)
                        ENDIF
                        sumarea = sumarea + cache%area(k)
                     ENDIF
                  ENDDO

                  IF (do_average) THEN
                     IF (fields(ifield)%recv(j) /= fields(ifield)%fillvalue .and. sumarea > 0._r8) THEN
                        fields(ifield)%recv(j) = fields(ifield)%recv(j) / sumarea
                     ENDIF
                  ENDIF
               ENDDO

            END SELECT
         ENDDO
      ENDIF

#ifdef USEMPI
      IF (size(cache%req_send) > 0) THEN
         CALL mpi_waitall (size(cache%req_send), cache%req_send, MPI_STATUSES_IGNORE, p_err)
      ENDIF
#endif

   END SUBROUTINE riverlake_push_batch

   ! ----------
   SUBROUTINE riverlake_pushcache_free_mem (this)

   IMPLICIT NONE
   type(riverlake_pushcache_type) :: this

      this%ready = .false.
      this%mapping_kind = PUSH_MAPPING_EMPTY
      this%required_send_size = 0
      this%num_recv = 0
      this%batch_exact = .false.

      IF (allocated(this%ptr      )) deallocate (this%ptr      )
      IF (allocated(this%addr     )) deallocate (this%addr     )
      IF (allocated(this%area     )) deallocate (this%area     )
      IF (allocated(this%recv_uniq)) deallocate (this%recv_uniq)
#ifdef USEMPI
      this%batch_capacity = 0
      IF (allocated(this%sendcache)) deallocate (this%sendcache)
      IF (allocated(this%recvcache)) deallocate (this%recvcache)
      IF (allocated(this%req_send )) deallocate (this%req_send )
      IF (allocated(this%req_recv )) deallocate (this%req_recv )
      IF (allocated(this%send_peer)) deallocate (this%send_peer)
      IF (allocated(this%recv_peer)) deallocate (this%recv_peer)
      IF (allocated(this%send_disp)) deallocate (this%send_disp)
      IF (allocated(this%recv_disp)) deallocate (this%recv_disp)
#endif

   END SUBROUTINE riverlake_pushcache_free_mem

END MODULE MOD_Grid_RiverLakePush
#endif
