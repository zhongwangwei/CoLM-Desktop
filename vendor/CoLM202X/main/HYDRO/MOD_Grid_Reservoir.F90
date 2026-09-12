#include <define.h>

#ifdef GridRiverLakeFlow
MODULE MOD_Grid_Reservoir
!-----------------------------------------------------------------------
! DESCRIPTION:
!
!    Reservoir module in gridded mesh.
!
! Created by Shupeng Zhang, Oct 2025
!-----------------------------------------------------------------------
   USE MOD_Precision
   USE MOD_DataType

   integer :: totalnumresv
   integer :: numresv
   integer,  allocatable :: ucat2resv   (:)
   ! Local reservoir state is ordered by the active parameter rows. Preserve
   ! both that dense global state ordinal and its owning ucatch so restart files
   ! can prove that volresv has not been rebound after a dam-table change.
   integer,  allocatable :: resv_global_id(:)
   integer,  allocatable :: resv_ucid     (:)
   type(pointer_int32_1d), allocatable :: resv_data_address (:)


   ! parameters
   integer,  allocatable :: dam_GRAND_ID  (:)  ! GRAND dam ID

   integer,  allocatable :: dam_build_year(:)  ! year in which the dam/barrier was built

   real(r8), allocatable :: volresv_total (:)  ! total reservoir volume      [m^3]
   real(r8), allocatable :: volresv_emerg (:)  ! emergency reservoir volume  [m^3]
   real(r8), allocatable :: volresv_adjust(:)  ! adjustment reservoir volume [m^3]
   real(r8), allocatable :: volresv_normal(:)  ! normal reservoir volume     [m^3]

   real(r8), allocatable :: qresv_flood   (:)  ! flood reservoir outflow      [m^3/s]
   real(r8), allocatable :: qresv_adjust  (:)  ! adjustment reservoir outflow [m^3/s]
   real(r8), allocatable :: qresv_normal  (:)  ! normal reservoir outflow     [m^3/s]

   ! fluxes
   real(r8), allocatable :: qresv_in      (:)  ! reservoir inflow  [m^3/s]
   real(r8), allocatable :: qresv_out     (:)  ! reservoir outflow [m^3/s]

   ! -- PUBLIC SUBROUTINEs --
   PUBLIC :: reservoir_init
   PUBLIC :: reservoir_operation
   PUBLIC :: reservoir_final

CONTAINS

   ! -------
   SUBROUTINE reservoir_init ( )

   USE MOD_SPMD_Task
   USE MOD_NetCDFSerial
   USE MOD_Utils
   USE MOD_Namelist,              only: DEF_ReservoirPara_file, DEF_Reservoir_Method
   USE MOD_Grid_RiverLakeNetwork, only: numucat, ucat_ucid, lake_type
   USE, INTRINSIC :: ieee_arithmetic, only: ieee_is_finite

   IMPLICIT NONE

   ! Local variables
   character(len=256) :: parafile

   integer,  allocatable :: dam_seq(:), order(:), loc2all(:), ordinal_count(:)
   integer,  allocatable :: counts(:), disps(:), resv_ids(:)
   integer,  allocatable :: catalogue_to_active(:)
   real(r8), allocatable :: rcache (:)
   integer,  allocatable :: icache (:)

   integer :: i, iloc, irsv, nresv, iworker, nresv_catalogue


      parafile = DEF_ReservoirPara_file

      IF (DEF_Reservoir_Method /= 1) &
         CALL CoLM_stop ('unsupported reservoir operation method')

      CALL ncio_read_bcast_serial (parafile, 'dam_GRAND_ID', dam_GRAND_ID)
      CALL ncio_read_bcast_serial (parafile, 'dam_seq', dam_seq)

      nresv_catalogue = size(dam_seq)
      IF (size(dam_GRAND_ID) /= nresv_catalogue) &
         CALL CoLM_stop ('reservoir dam_GRAND_ID and dam_seq lengths differ')
      numresv = 0  ! Safe default for all ranks; workers overwrite below

      allocate (order (nresv_catalogue))
      order = (/(i, i = 1, nresv_catalogue)/)

      CALL quicksort (nresv_catalogue, dam_seq, order)

      IF (nresv_catalogue > 1) THEN
         IF (any(dam_seq(2:nresv_catalogue) == dam_seq(1:nresv_catalogue-1))) &
            CALL CoLM_stop ('duplicate dam_seq entries in reservoir parameter file')
      ENDIF

      IF (p_is_worker) THEN

         allocate (ucat2resv (numucat))
         allocate (loc2all   (numucat))
         ucat2resv = 0

         numresv = 0
         DO i = 1, numucat
            iloc = find_in_sorted_list1 (ucat_ucid(i), nresv_catalogue, dam_seq)
            IF (iloc > 0) THEN
               numresv = numresv + 1
               lake_type(i) = 2
               ucat2resv(i) = numresv
               loc2all  (numresv) = order(iloc)
            ENDIF
         ENDDO

         allocate (resv_ucid     (numresv))
         IF (numresv > 0) THEN
            DO i = 1, numucat
               IF (ucat2resv(i) > 0) resv_ucid(ucat2resv(i)) = ucat_ucid(i)
            ENDDO
         ENDIF

      ENDIF

      ! The parameter file may be a global catalogue while the active ucatch
      ! network is regional. Rows outside the active domain are valid; only an
      ! active row owned more than once is an invalid state mapping.
      allocate (ordinal_count(nresv_catalogue))
      ordinal_count = 0
      IF (p_is_worker) THEN
         DO irsv = 1, numresv
            IF (loc2all(irsv) < 1 .or. loc2all(irsv) > nresv_catalogue) &
               CALL CoLM_stop ('invalid reservoir parameter-table ordinal')
            ordinal_count(loc2all(irsv)) = ordinal_count(loc2all(irsv)) + 1
         ENDDO
      ENDIF
#ifdef USEMPI
      IF (nresv_catalogue > 0) CALL mpi_allreduce (MPI_IN_PLACE, ordinal_count, nresv_catalogue, &
         MPI_INTEGER, MPI_SUM, p_comm_glb, p_err)
#endif
      IF (nresv_catalogue > 0) THEN
         IF (any(ordinal_count > 1)) THEN
            IF (p_is_master) write(*,'(A,I0)') &
               'ERROR: reservoir rows mapped to multiple active ucatch owners=', count(ordinal_count > 1)
            CALL CoLM_stop ('active reservoir ucatch IDs must map one-to-one to parameter rows')
         ENDIF
         IF (p_is_master .and. any(ordinal_count == 0)) write(*,'(A,I0,A,I0)') &
            'Reservoir catalogue rows outside active domain: ', count(ordinal_count == 0), &
            ' of ', nresv_catalogue
      ENDIF

      ! State/history/restart vectors use a dense active-reservoir axis. The
      ! original catalogue row remains in loc2all solely for parameter lookup.
      allocate (catalogue_to_active(nresv_catalogue))
      catalogue_to_active = 0
      totalnumresv = 0
      DO i = 1, nresv_catalogue
         IF (ordinal_count(i) > 0) THEN
            totalnumresv = totalnumresv + 1
            catalogue_to_active(i) = totalnumresv
         ENDIF
      ENDDO
      dam_GRAND_ID = pack(dam_GRAND_ID, ordinal_count > 0)
      IF (p_is_worker) THEN
         allocate (resv_global_id(numresv))
         IF (numresv > 0) resv_global_id = catalogue_to_active(loc2all(1:numresv))
      ENDIF
      deallocate (ordinal_count)
      deallocate (catalogue_to_active)

#ifdef USEMPI
#ifdef FLAT_SPMD
      allocate (counts(0:p_np_glb-1), disps(0:p_np_glb-1))
      CALL mpi_allgather (numresv, 1, MPI_INTEGER, counts, 1, MPI_INTEGER, p_comm_glb, p_err)
      disps(0) = 0
      DO iworker = 1, p_np_glb-1
         disps(iworker) = disps(iworker-1) + counts(iworker-1)
      ENDDO

      allocate (icache(max(1,numresv)))
      IF (numresv > 0) icache(1:numresv) = resv_global_id
      IF (p_is_master) THEN
         allocate (resv_ids(max(1,sum(counts))))
      ELSE
         allocate (resv_ids(1))
      ENDIF
      CALL mpi_gatherv (icache, numresv, MPI_INTEGER, resv_ids, counts, disps, &
         MPI_INTEGER, p_root, p_comm_glb, p_err)

      allocate (resv_data_address(0:p_np_glb-1))
      IF (p_is_master) THEN
         DO iworker = 0, p_np_glb-1
            nresv = counts(iworker)
            IF (nresv > 0) THEN
               allocate (resv_data_address(iworker)%val(nresv))
               resv_data_address(iworker)%val = &
                  resv_ids(disps(iworker)+1:disps(iworker)+nresv)
            ENDIF
         ENDDO
      ENDIF
      deallocate (counts, disps, resv_ids, icache)
#else
      IF (p_is_master) THEN

         allocate (resv_data_address (0:p_np_worker-1))

         DO iworker = 0, p_np_worker-1

            CALL mpi_recv (nresv, 1, MPI_INTEGER, &
               p_address_worker(iworker), mpi_tag_mesg, p_comm_glb, p_stat, p_err)

            IF (nresv > 0) THEN
               allocate (resv_data_address(iworker)%val (nresv))
               CALL mpi_recv (resv_data_address(iworker)%val, nresv, MPI_INTEGER, &
                  p_address_worker(iworker), mpi_tag_data, p_comm_glb, p_stat, p_err)
            ENDIF
         ENDDO

      ELSEIF (p_is_worker) THEN

         CALL mpi_send (numresv, 1, MPI_INTEGER, p_address_master, mpi_tag_mesg, p_comm_glb, p_err)

         IF (numresv > 0) THEN
            CALL mpi_send (resv_global_id, numresv, MPI_INTEGER, p_address_master, &
               mpi_tag_data, p_comm_glb, p_err)
         ENDIF

      ENDIF

      ! Every non-master rank enters gather/scatter calls; an allocated
      ! zero-size address book is a valid unused actual argument there.
      IF (.not. p_is_master .and. .not. allocated(resv_data_address)) THEN
         allocate (resv_data_address (0:-1))  ! zero-size
      ENDIF
#endif
#else
      IF (numresv > 0) THEN
         allocate (resv_data_address (0:0))
         allocate (resv_data_address(0)%val (numresv))
         resv_data_address(0)%val = resv_global_id
      ENDIF
#endif

      IF (p_is_worker) THEN

         IF (numresv > 0) THEN

            allocate (dam_build_year (numresv))

            allocate (volresv_total  (numresv))
            allocate (volresv_emerg  (numresv))
            allocate (volresv_adjust (numresv))
            allocate (volresv_normal (numresv))

            allocate (qresv_flood    (numresv))
            allocate (qresv_adjust   (numresv))
            allocate (qresv_normal   (numresv))

            allocate (qresv_in       (numresv))
            allocate (qresv_out      (numresv))

         ENDIF

      ENDIF

      CALL ncio_read_bcast_serial (parafile, 'dam_year', icache)
      IF (size(icache) /= nresv_catalogue) &
         CALL CoLM_stop ('reservoir dam_year and dam_seq lengths differ')
      IF (p_is_worker .and. (numresv > 0)) THEN
         dam_build_year = icache(loc2all(1:numresv))
      ENDIF

      CALL ncio_read_bcast_serial (parafile, 'dam_TotalVol_mcm', rcache)
      IF (size(rcache) /= nresv_catalogue) &
         CALL CoLM_stop ('reservoir dam_TotalVol_mcm and dam_seq lengths differ')
      IF (p_is_worker .and. (numresv > 0)) THEN
         volresv_total = rcache(loc2all(1:numresv))*1.e6
      ENDIF

      CALL ncio_read_bcast_serial (parafile, 'dam_ConVol_mcm', rcache)
      IF (size(rcache) /= nresv_catalogue) &
         CALL CoLM_stop ('reservoir dam_ConVol_mcm and dam_seq lengths differ')
      IF (p_is_worker .and. (numresv > 0)) THEN
         volresv_normal = rcache(loc2all(1:numresv))*1.e6
      ENDIF

      CALL ncio_read_bcast_serial (parafile, 'dam_Qn', rcache)
      IF (size(rcache) /= nresv_catalogue) &
         CALL CoLM_stop ('reservoir dam_Qn and dam_seq lengths differ')
      IF (p_is_worker .and. (numresv > 0)) THEN
         qresv_normal = rcache(loc2all(1:numresv))
      ENDIF

      CALL ncio_read_bcast_serial (parafile, 'dam_Qf', rcache)
      IF (size(rcache) /= nresv_catalogue) &
         CALL CoLM_stop ('reservoir dam_Qf and dam_seq lengths differ')
      IF (p_is_worker .and. (numresv > 0)) THEN
         qresv_flood = rcache(loc2all(1:numresv))
      ENDIF


      IF (p_is_worker) THEN
         DO irsv = 1, numresv
            IF (.not. ieee_is_finite(volresv_total(irsv)) .or. volresv_total(irsv) <= 0._r8 .or. &
                .not. ieee_is_finite(volresv_normal(irsv)) .or. volresv_normal(irsv) <= 0._r8 .or. &
                .not. ieee_is_finite(qresv_normal(irsv)) .or. qresv_normal(irsv) < 0._r8 .or. &
                .not. ieee_is_finite(qresv_flood(irsv)) .or. qresv_flood(irsv) < 0._r8) &
               CALL CoLM_stop ('invalid active reservoir volume or outflow parameter')
            volresv_emerg (irsv) = volresv_total(irsv) * 0.94
            volresv_adjust(irsv) = volresv_total(irsv) * 0.77
            volresv_normal(irsv) = min(volresv_total(irsv)*0.7, volresv_normal(irsv))
            qresv_adjust  (irsv) = (qresv_normal(irsv) + qresv_flood(irsv)) * 0.5
         ENDDO
      ENDIF

      IF (allocated(dam_seq)) deallocate(dam_seq)
      IF (allocated(order  )) deallocate(order  )
      IF (allocated(loc2all)) deallocate(loc2all)
      IF (allocated(rcache )) deallocate(rcache )
      IF (allocated(icache )) deallocate(icache )

   END SUBROUTINE reservoir_init


   SUBROUTINE reservoir_operation (method, irsv, qin, vol, qout)

   USE MOD_SPMD_Task, only: CoLM_stop
   IMPLICIT NONE
   integer,  intent(in)  :: method
   integer,  intent(in)  :: irsv
   real(r8), intent(in)  :: qin, vol
   real(r8), intent(out) :: qout

   ! local variables
   real(r8) :: q1

      SELECT CASE (method)
      CASE (1)
         ! *** Reference ***
         ! [1] Mizuki Funato, Dai Yamazaki, Dung Trung Vu.
         ! Development of an Improved Reservoir Operation Scheme for Global Flood Modeling (CaMa-Flood v4.20).
         ! ESS Open Archive . October 24, 2024.

         IF (vol > volresv_emerg(irsv)) THEN
            qout = max(qin, qresv_flood(irsv))
         ELSEIF (vol > volresv_adjust(irsv)) THEN
            qout = qresv_adjust(irsv) + (qresv_flood(irsv)-qresv_adjust(irsv)) &
               * ((vol-volresv_adjust(irsv))/(volresv_emerg(irsv)-volresv_adjust(irsv)))**0.1
            IF (qin > qresv_flood(irsv)) THEN
               q1 = qresv_normal(irsv) + (qin-qresv_normal(irsv)) &
                  * (vol-volresv_normal(irsv))/(volresv_emerg(irsv)-volresv_normal(irsv))
               qout = max(q1, qout)
            ENDIF
         ELSEIF (vol > volresv_normal(irsv)) THEN
            qout = qresv_normal(irsv) + (qresv_adjust(irsv)-qresv_normal(irsv)) &
               * ((vol-volresv_normal(irsv))/(volresv_adjust(irsv)-volresv_normal(irsv)))**3.
         ELSE
            qout = (vol/volresv_normal(irsv))**0.5 * qresv_normal(irsv)
         ENDIF

      CASE DEFAULT
         CALL CoLM_stop ('unsupported reservoir operation method')
      END SELECT

   END SUBROUTINE reservoir_operation


   SUBROUTINE reservoir_final ()

   IMPLICIT NONE

      IF (allocated(ucat2resv        )) deallocate (ucat2resv        )
      IF (allocated(resv_global_id   )) deallocate (resv_global_id   )
      IF (allocated(resv_ucid        )) deallocate (resv_ucid        )
      IF (allocated(resv_data_address)) deallocate (resv_data_address)

      IF (allocated(dam_GRAND_ID     )) deallocate (dam_GRAND_ID     )
      IF (allocated(dam_build_year   )) deallocate (dam_build_year   )

      IF (allocated(volresv_total    )) deallocate (volresv_total    )
      IF (allocated(volresv_emerg    )) deallocate (volresv_emerg    )
      IF (allocated(volresv_adjust   )) deallocate (volresv_adjust   )
      IF (allocated(volresv_normal   )) deallocate (volresv_normal   )

      IF (allocated(qresv_flood      )) deallocate (qresv_flood      )
      IF (allocated(qresv_adjust     )) deallocate (qresv_adjust     )
      IF (allocated(qresv_normal     )) deallocate (qresv_normal     )

      IF (allocated(qresv_in         )) deallocate (qresv_in         )
      IF (allocated(qresv_out        )) deallocate (qresv_out        )

   END SUBROUTINE reservoir_final

END MODULE MOD_Grid_Reservoir
#endif
