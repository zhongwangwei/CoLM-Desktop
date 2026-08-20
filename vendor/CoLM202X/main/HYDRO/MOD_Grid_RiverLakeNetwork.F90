#include <define.h>

#ifdef GridRiverLakeFlow
MODULE MOD_Grid_RiverLakeNetwork
!--------------------------------------------------------------------------------
! DESCRIPTION:
!--------------------------------------------------------------------------------

   USE MOD_Grid
   USE MOD_WorkerPushData
   USE, INTRINSIC :: ieee_arithmetic, ONLY: ieee_is_finite
#ifdef USEMPI
   USE MOD_SPMD_Task, ONLY: MPI_COMM_NULL
#endif
   IMPLICIT NONE

   ! ----- River Lake network -----

   type(grid_type) :: griducat

   integer :: totalnumucat
   integer :: numucat
   integer, allocatable :: ucat_ucid (:)   ! index in unit catchment numbering
   integer, allocatable :: x_ucat    (:)   !
   integer, allocatable :: y_ucat    (:)   !
   integer, allocatable :: ucat_gdid (:)   !

   integer, allocatable :: numucat_wrk (:)
   type(pointer_int32_1d), allocatable :: ucat_data_address (:)

   ! ----- Part 1: between runoff input elements and unit catchments -----
   integer :: numinpm
   integer,  allocatable :: inpm_gdid (:)

   integer :: inpn
   integer,  allocatable :: idmap_gd2uc (:,:)
   real(r8), allocatable :: area_gd2uc  (:,:)

   integer :: nucpart
   integer,  allocatable :: idmap_uc2gd (:,:)
   real(r8), allocatable :: area_uc2gd  (:,:)

   type(worker_remapdata_type) :: remap_patch2inpm
   type(worker_pushdata_type)  :: push_inpm2ucat
   type(worker_pushdata_type)  :: push_ucat2inpm
   type(worker_pushdata_type)  :: push_ucat2grid
   type(worker_pushdata_type)  :: allreduce_inpm

   ! ----- Part 2: between upstream and downstream unit catchments -----
   integer,  allocatable :: ucat_next (:)  ! next unit catchment
   integer :: upnmax
   integer,  allocatable :: ucat_ups (:,:) ! upstream unit catchments
   real(r8), allocatable :: wts_ups  (:,:)

   type(worker_pushdata_type) :: push_next2ucat
   type(worker_pushdata_type) :: push_ups2ucat

   ! ----- Part 3: river systems -----
   integer :: numrivsys
   logical :: rivsys_by_multiple_procs
   integer, allocatable :: irivsys (:)
#ifdef USEMPI
   integer :: p_comm_rivsys = MPI_COMM_NULL
#endif


   ! ----- Parameters for River and Lake -----

   integer,  allocatable :: lake_type      (:)   ! 0: river; 2: reservoir.

   real(r8), allocatable :: topo_rivelv    (:)   ! river bed elevation [m]
   real(r8), allocatable :: topo_rivhgt    (:)   ! river channel depth [m]
   real(r8), allocatable :: topo_rivlen    (:)   ! river channel length [m]
   real(r8), allocatable :: topo_rivman    (:)   ! river manning coefficient [m]
   real(r8), allocatable :: topo_rivwth    (:)   ! river channel width [m]
   real(r8), allocatable :: topo_rivare    (:)   ! river channel area [m^2]
   real(r8), allocatable :: topo_rivstomax (:)   ! max river channel storage [m^3]

   real(r8), allocatable :: topo_area      (:)   ! floodplain area [m^2]
   real(r8), allocatable :: topo_fldhgt    (:,:) ! floodplain height profile [m]

   ! ----- Levee parameters (read from file) -----
   real(r8), allocatable :: levee_frc_data (:)   ! levee unprotected fraction [0-1]
   real(r8), allocatable :: levee_hgt_data (:)   ! levee crest height above floodplain datum [m]

   ! ----- Bifurcation pathway parameters (read from file) -----
   integer  :: totalnpthout  = 0         ! total number of pathways globally
   integer  :: npthout_local = 0         ! number of pathways on this worker
   integer  :: npthlev_bif   = 0         ! number of vertical layers

   integer,  allocatable :: pth_upst_local  (:)   ! upstream ucat local index
   integer,  allocatable :: pth_down_local  (:)   ! downstream ucat local index (or -1 if remote)
   integer,  allocatable :: pth_down_ucid   (:)   ! downstream ucat global ID
   integer,  allocatable :: pth_global_id   (:)   ! global pathway ID (1..totalnpthout)
   real(r8), allocatable :: pth_dst         (:)   ! pathway length [m], used for BIF slope (not adaptive-dt CFL)
   real(r8), allocatable :: pth_elv         (:,:) ! elevation profile (npthlev, npthout_local) [m]
   real(r8), allocatable :: pth_wth         (:,:) ! width profile (npthlev, npthout_local) [m]
   real(r8), allocatable :: pth_man         (:)   ! Manning coefficients (npthlev)

   ! Reverse mapping: for each ucat, which global pathway IDs feed into it as downstream
   integer  :: max_bif_incoming
   integer,  allocatable :: bif_incoming_pths (:,:) ! (max_bif_incoming, numucat)
   real(r8), allocatable :: bif_incoming_wts  (:,:) ! weights, all 1.0

   ! Push objects for bifurcation
   type(worker_pushdata_type) :: push_bif_dn2pth   ! ucat state -> pathway downstream end
   type(worker_pushdata_type) :: push_bif_influx   ! pathway flux -> downstream ucats

   real(r8), allocatable :: bedelv_next    (:)   ! downstream river bed elevation [m]
   real(r8), allocatable :: outletwth      (:)   ! river outlet width [m]

   type :: vol_dep_curve_type
      integer  :: nlfp
      real(r8) :: rivhgt
      real(r8) :: rivare
      real(r8) :: rivstomax
      real(r8), allocatable :: flphgt    (:) ! floodplain height profile [m]
      real(r8), allocatable :: flparea   (:) ! flood plain area [m^2]
      real(r8), allocatable :: flpaccare (:) ! flood plain accumulated area [m^2]
      real(r8), allocatable :: flpstomax (:) ! max flood plain storage [m^3]
   CONTAINS
      procedure, PUBLIC :: depth     => retrieve_depth_from_volume
      procedure, PUBLIC :: volume    => retrieve_volume_from_depth
      procedure, PUBLIC :: floodarea => retrieve_area_from_depth
      final :: vol_depth_curve_free_mem
   END type vol_dep_curve_type

   type(vol_dep_curve_type), allocatable :: floodplain_curve (:)


   ! ----- Mask of Grids with all upstream area in the simulation region -----
   real(r8), allocatable :: allups_mask_ucat (:)

CONTAINS

   ! ----------
   SUBROUTINE build_riverlake_network ()

   USE MOD_SPMD_Task
   USE MOD_Namelist
   USE MOD_NetCDFSerial
   USE MOD_Mesh
   USE MOD_Utils
   USE MOD_LandPatch
   USE MOD_Vars_Global, only: spval
   IMPLICIT NONE

   ! Local Variables
   character(len=256)    :: parafile

   integer,  allocatable :: idmap_x(:,:), idmap_y(:,:)

   integer :: numrivmth
   integer,  allocatable :: rivermouth(:)

   integer,  allocatable :: nups_nst  (:), iups_nst  (:), nups_all(:)
   integer,  allocatable :: uc_up2down(:), order_ucat(:)
   integer,  allocatable :: addr_ucat (:)

   integer , allocatable :: nuc_rs(:), iwrk_rs(:), nwrk_rs(:), nave_rs(:)
   real(r8), allocatable :: wt_uc (:), wt_rs  (:), wt_wrk (:), nuc_wrk(:)

   integer,  allocatable :: grdindex(:)


   integer,  allocatable :: idata1d(:), idata2d(:,:)
   real(r8), allocatable :: rdata1d(:), rdata2d(:,:)

   integer,  allocatable :: allgrd_in_inp (:), nucat_g2d(:,:), iucat_g(:)

   integer,  allocatable :: idmap_uc2gd_all(:,:)
   real(r8), allocatable :: area_uc2gd_all (:,:)

   real(r8), allocatable :: ucat_area_all (:)

   integer  :: nlat_ucat, nlon_ucat
   integer  :: nucat, iriv, ngrdall, igrd, ngrd
   integer  :: p_np_rivsys, color
   integer  :: iworker, iwrkdsp
   integer  :: iloc, i, j, ithis
   real(r8) :: sumwt
   logical  :: is_new, invalid_rivsys_partition


#ifdef USEMPI
      CALL mpi_barrier (p_comm_glb, p_err)
#endif

      ! read in parameters from file.
      IF (p_is_master) THEN

         parafile = DEF_UnitCatchment_file

         CALL ncio_read_serial (parafile, 'seq_x', x_ucat)
         CALL ncio_read_serial (parafile, 'seq_y', y_ucat)

         CALL ncio_read_serial (parafile, 'seq_next', ucat_next)

         CALL ncio_inquire_length (parafile, 'lon', nlon_ucat)
         CALL ncio_inquire_length (parafile, 'lat', nlat_ucat)

         CALL ncio_read_serial (parafile, 'inpmat_x', idmap_x)
         CALL ncio_read_serial (parafile, 'inpmat_y', idmap_y)
         CALL ncio_read_serial (parafile, 'inpmat_area', area_gd2uc)

      ENDIF

      IF (p_is_master) THEN

         totalnumucat = size(x_ucat)

         allocate (nups_nst (totalnumucat))
         allocate (iups_nst (totalnumucat))

         nups_nst(:) = 0
         DO i = 1, totalnumucat
            j = ucat_next(i)
            IF (j > 0) THEN
               nups_nst(j) = nups_nst(j) + 1
            ENDIF
         ENDDO

         ! sort unit catchment from upstream to downstream, recorded by "uc_up2down"
         allocate (uc_up2down (totalnumucat))

         ithis = 0
         iups_nst(:) = 0
         DO i = 1, totalnumucat
            IF (iups_nst(i) == nups_nst(i)) THEN

               ithis = ithis + 1
               uc_up2down(ithis) = i
               iups_nst(i) = -1

               j = ucat_next(i)
               DO WHILE (j > 0)

                  iups_nst(j) = iups_nst(j) + 1

                  IF (iups_nst(j) == nups_nst(j)) THEN
                     ithis = ithis + 1
                     uc_up2down(ithis) = j
                     iups_nst(j) = -1

                     j = ucat_next(j)
                  ELSE
                     EXIT
                  ENDIF
               ENDDO
            ENDIF
         ENDDO

      ENDIF

#ifdef USEMPI
      ! divide unit catchments into groups and assign to workers
      IF (p_is_master) THEN

         allocate (wt_uc (totalnumucat));  wt_uc(:) = 1.

         allocate (rivermouth (totalnumucat))
         numrivmth = 0
         DO i = totalnumucat, 1, -1
            j = ucat_next(uc_up2down(i))
            IF (j <= 0) THEN
               numrivmth = numrivmth + 1
               rivermouth(uc_up2down(i)) = numrivmth
            ELSE
               rivermouth(uc_up2down(i)) = rivermouth(j)
            ENDIF
         ENDDO

         allocate (nuc_rs (numrivmth)); nuc_rs(:) = 0
         allocate (wt_rs  (numrivmth)); wt_rs (:) = 0.
         DO i = 1, totalnumucat
            nuc_rs(rivermouth(i)) = nuc_rs(rivermouth(i)) + 1
            wt_rs (rivermouth(i)) = wt_rs (rivermouth(i)) + wt_uc(i)
         ENDDO

         sumwt = sum(wt_rs)

         allocate (iwrk_rs (numrivmth))
         allocate (nwrk_rs (numrivmth))
         allocate (nave_rs (numrivmth))

         iwrkdsp = -1
         DO i = 1, numrivmth
            nwrk_rs(i) = floor(wt_rs(i)/sumwt * p_np_worker)
            IF (nwrk_rs(i) > 1) THEN

               nave_rs(i) = nuc_rs(i) / nwrk_rs(i)
               IF (mod(nuc_rs(i), nwrk_rs(i)) /= 0) THEN
                  nave_rs(i) = nave_rs(i) + 1
               ENDIF

               iwrk_rs(i) = iwrkdsp + 1
               iwrkdsp = iwrkdsp + nwrk_rs(i)
            ENDIF
         ENDDO

         allocate (nups_all (totalnumucat));  nups_all(:) = 1

         DO i = 1, totalnumucat
            j = ucat_next(uc_up2down(i))
            IF (j > 0) THEN
               nups_all(j) = nups_all(j) + nups_all(uc_up2down(i))
            ENDIF
         ENDDO

         allocate (addr_ucat (totalnumucat));  addr_ucat(:) = -1

         allocate (wt_wrk (0:p_np_worker-1));  wt_wrk (:) = 0
         allocate (nuc_wrk(0:p_np_worker-1));  nuc_wrk(:) = 0

         allocate (order_ucat (totalnumucat))
         order_ucat(uc_up2down) = (/(i, i = 1, totalnumucat)/)

         ithis = totalnumucat
         DO WHILE (ithis > 0)

            i = uc_up2down(ithis)

            IF (addr_ucat(i) >= 0) THEN
               ithis = ithis - 1
               CYCLE
            ENDIF

            j = ucat_next(i)
            IF (j > 0) THEN
               IF (addr_ucat(j) >= 0) THEN
                  addr_ucat(i) = addr_ucat(j)
                  ithis = ithis - 1
                  CYCLE
               ENDIF
            ENDIF

            iriv = rivermouth(i)
            IF (nwrk_rs(iriv) > 1) THEN
               iworker = iwrk_rs(iriv)
               IF (nups_all(i) <= nave_rs(iriv)-nuc_wrk(iworker)) THEN

                  addr_ucat(i) = p_address_worker(iworker)

                  nuc_wrk(iworker) = nuc_wrk(iworker) + nups_all(i)
                  IF (nuc_wrk(iworker) == nave_rs(iriv)) THEN
                     iwrk_rs(iriv) = iwrk_rs(iriv) + 1
                  ENDIF

                  j = ucat_next(i)
                  IF (j > 0) THEN
                     DO WHILE (j > 0)
                        nups_all(j) = nups_all(j) - nups_all(i)
                        ithis = order_ucat(j)
                        j = ucat_next(j)
                     ENDDO
                  ELSE
                     ithis = ithis - 1
                  ENDIF
               ELSE
                  ithis = ithis - 1
               ENDIF
            ELSE
               iworker = minloc(wt_wrk(iwrkdsp+1:p_np_worker-1), dim=1) + iwrkdsp

               addr_ucat(i) = p_address_worker(iworker)

               wt_wrk(iworker) = wt_wrk(iworker) + wt_rs(iriv)
               ithis = ithis - 1
            ENDIF

         ENDDO

         deallocate (order_ucat)
         deallocate (nups_all  )
         deallocate (nuc_rs    )
         deallocate (iwrk_rs   )
         deallocate (nwrk_rs   )
         deallocate (nave_rs   )
         deallocate (wt_uc     )
         deallocate (wt_rs     )
         deallocate (wt_wrk    )
         deallocate (nuc_wrk   )

      ENDIF

      IF (p_is_master) THEN

         allocate(ucat_ucid (totalnumucat))
         ucat_ucid = (/(i, i = 1, totalnumucat)/)

         allocate (numucat_wrk       (0:p_np_worker-1))
         allocate (ucat_data_address (0:p_np_worker-1))

         DO iworker = 0, p_np_worker-1
            nucat = count(addr_ucat == p_address_worker(iworker))
            numucat_wrk(iworker) = nucat
            IF (nucat > 0) THEN
               allocate (ucat_data_address(iworker)%val (nucat))
               ucat_data_address(iworker)%val = &
                  pack(ucat_ucid, mask = (addr_ucat == p_address_worker(iworker)))
            ENDIF
         ENDDO

         deallocate (ucat_ucid)
      ENDIF

      CALL mpi_bcast (totalnumucat, 1, MPI_INTEGER, p_address_master, p_comm_glb, p_err)

      ! send unit catchment index to workers
      IF (p_is_master) THEN

         DO iworker = 0, p_np_worker-1

            CALL mpi_send (numucat_wrk(iworker), 1, MPI_INTEGER, p_address_worker(iworker), &
               mpi_tag_mesg, p_comm_glb, p_err)

            nucat = numucat_wrk(iworker)
            IF (nucat > 0) THEN

               CALL mpi_send (ucat_data_address(iworker)%val, nucat, MPI_INTEGER, &
                  p_address_worker(iworker), mpi_tag_data, p_comm_glb, p_err)

               allocate (idata1d (nucat))

               idata1d = x_ucat (ucat_data_address(iworker)%val)
               CALL mpi_send (idata1d, nucat, MPI_INTEGER, &
                  p_address_worker(iworker), mpi_tag_data, p_comm_glb, p_err)

               idata1d = y_ucat (ucat_data_address(iworker)%val)
               CALL mpi_send (idata1d, nucat, MPI_INTEGER, &
                  p_address_worker(iworker), mpi_tag_data, p_comm_glb, p_err)

               deallocate (idata1d)
            ENDIF
         ENDDO

      ELSEIF (p_is_worker) THEN

         CALL mpi_recv (numucat, 1, MPI_INTEGER, p_address_master, mpi_tag_mesg, p_comm_glb, p_stat, p_err)

         IF (numucat > 0) THEN
            allocate (ucat_ucid (numucat))
            allocate (x_ucat    (numucat))
            allocate (y_ucat    (numucat))
            CALL mpi_recv (ucat_ucid, numucat, MPI_INTEGER, p_address_master, &
               mpi_tag_data, p_comm_glb, p_stat, p_err)
            CALL mpi_recv (x_ucat, numucat, MPI_INTEGER, p_address_master, &
               mpi_tag_data, p_comm_glb, p_stat, p_err)
            CALL mpi_recv (y_ucat, numucat, MPI_INTEGER, p_address_master, &
               mpi_tag_data, p_comm_glb, p_stat, p_err)
         ENDIF

      ENDIF

      ! Non-worker ranks (master, IO) need numucat=0 and zero-length arrays
      ! so that hist gather calls and build_worker_pushdata receive valid arguments.
      IF (.not. p_is_worker) THEN
         numucat = 0
         IF (.not. allocated(ucat_ucid)) allocate (ucat_ucid (0))
         IF (.not. allocated(x_ucat   )) allocate (x_ucat    (0))
         IF (.not. allocated(y_ucat   )) allocate (y_ucat    (0))
      ENDIF

      CALL mpi_barrier (p_comm_glb, p_err)
#else
      numucat = totalnumucat

      allocate(ucat_ucid (totalnumucat))
      ucat_ucid = (/(i, i = 1, totalnumucat)/)

      allocate (numucat_wrk (0:0))
      numucat_wrk(0) = numucat

      allocate (ucat_data_address (0:0))
      allocate (ucat_data_address(0)%val (numucat))
      ucat_data_address(0)%val = ucat_ucid
#endif

      IF (allocated(addr_ucat)) deallocate(addr_ucat)

      ! ----- Part 1: between runoff input elements and unit catchments -----

#ifdef USEMPI
      CALL mpi_bcast (nlon_ucat, 1, MPI_INTEGER, p_address_master, p_comm_glb, p_err)
      CALL mpi_bcast (nlat_ucat, 1, MPI_INTEGER, p_address_master, p_comm_glb, p_err)
#endif

      CALL griducat%define_by_ndims (nlon_ucat, nlat_ucat)

      CALL build_worker_remapdata (landpatch, griducat, remap_patch2inpm)

      IF (p_is_worker) THEN
         numinpm = remap_patch2inpm%num_grid
         IF (numinpm > 0) THEN
            allocate (inpm_gdid (numinpm))
            inpm_gdid = remap_patch2inpm%ids_me
         ENDIF
      ENDIF


      IF (p_is_master) THEN

         inpn = size(idmap_x,1)

         allocate(idmap_gd2uc (inpn,totalnumucat))

         idmap_gd2uc = (idmap_y-1)*nlon_ucat + idmap_x

         WHERE ((area_gd2uc <= 0) .or. (idmap_gd2uc <= 0))
            idmap_gd2uc = 0
            area_gd2uc  = 0.
         END WHERE

         allocate (nucat_g2d (nlon_ucat,nlat_ucat))
         nucat_g2d(:,:) = 0

         DO i = 1, totalnumucat
            DO j = 1, inpn
               IF (idmap_gd2uc(j,i) > 0) THEN
                  nucat_g2d(idmap_x(j,i),idmap_y(j,i)) = nucat_g2d(idmap_x(j,i),idmap_y(j,i)) + 1
               ENDIF
            ENDDO
         ENDDO

         nucpart = maxval(nucat_g2d)
         ngrdall = count(nucat_g2d > 0)

         allocate (allgrd_in_inp (ngrdall))

         igrd = 0
         DO i = 1, nlat_ucat
            DO j = 1, nlon_ucat
               IF (nucat_g2d(j,i) > 0) THEN
                  igrd = igrd + 1
                  allgrd_in_inp(igrd) = (i-1)*nlon_ucat + j
               ENDIF
            ENDDO
         ENDDO

         allocate (idmap_uc2gd_all (nucpart, ngrdall));  idmap_uc2gd_all(:,:) = 0
         allocate (area_uc2gd_all  (nucpart, ngrdall));  area_uc2gd_all (:,:) = 0.

         allocate (iucat_g (ngrdall)); iucat_g(:) = 0

         DO i = 1, totalnumucat
            DO j = 1, inpn
               IF (idmap_gd2uc(j,i) > 0) THEN
                  iloc = find_in_sorted_list1 (idmap_gd2uc(j,i), ngrdall, allgrd_in_inp(1:ngrdall))
                  iucat_g(iloc) = iucat_g(iloc) + 1
                  idmap_uc2gd_all(iucat_g(iloc),iloc) = i
                  area_uc2gd_all (iucat_g(iloc),iloc) = area_gd2uc(j,i)
               ENDIF
            ENDDO
         ENDDO

      ENDIF

#ifdef USEMPI
      CALL mpi_bcast (inpn, 1, MPI_INTEGER, p_address_master, p_comm_glb, p_err)

      IF (p_is_master) THEN

         DO iworker = 0, p_np_worker-1

            nucat = numucat_wrk(iworker)

            IF (nucat > 0) THEN
               allocate (idata2d (inpn, nucat))
               DO i = 1, nucat
                  idata2d(:,i) = idmap_gd2uc(:,ucat_data_address(iworker)%val(i))
               ENDDO

               CALL mpi_send (idata2d, inpn*nucat, MPI_INTEGER, p_address_worker(iworker), &
                  mpi_tag_data, p_comm_glb, p_err)

               allocate (rdata2d (inpn, nucat))
               DO i = 1, nucat
                  rdata2d(:,i) = area_gd2uc(:,ucat_data_address(iworker)%val(i))
               ENDDO

               CALL mpi_send (rdata2d, inpn*nucat, MPI_REAL8, p_address_worker(iworker), &
                  mpi_tag_data, p_comm_glb, p_err)

               deallocate (idata2d)
               deallocate (rdata2d)
            ENDIF
         ENDDO

         deallocate (idmap_gd2uc)
         deallocate (area_gd2uc )

      ELSEIF (p_is_worker) THEN

         IF (numucat > 0) THEN

            allocate (idmap_gd2uc (inpn, numucat))
            CALL mpi_recv (idmap_gd2uc, inpn*numucat, MPI_INTEGER, p_address_master, &
               mpi_tag_data, p_comm_glb, p_stat, p_err)

            allocate (area_gd2uc (inpn, numucat))
            CALL mpi_recv (area_gd2uc, inpn*numucat, MPI_REAL8, p_address_master, &
               mpi_tag_data, p_comm_glb, p_stat, p_err)

         ENDIF

      ENDIF

      CALL mpi_bcast (nucpart, 1, mpi_integer, p_address_master, p_comm_glb, p_err)

      IF (p_is_master) THEN

         DO iworker = 0, p_np_worker-1

            CALL mpi_recv (ngrd, 1, MPI_INTEGER, &
               p_address_worker(iworker), mpi_tag_mesg, p_comm_glb, p_stat, p_err)

            IF (ngrd > 0) THEN

               allocate (grdindex (ngrd))
               allocate (idata2d  (nucpart, ngrd));  idata2d(:,:) = 0
               allocate (rdata2d  (nucpart, ngrd));  rdata2d(:,:) = 0.

               CALL mpi_recv (grdindex, ngrd, MPI_INTEGER, &
                  p_address_worker(iworker), mpi_tag_data, p_comm_glb, p_stat, p_err)

               DO i = 1, ngrd
                  iloc = find_in_sorted_list1 (grdindex(i), ngrdall, allgrd_in_inp(1:ngrdall))
                  IF (iloc > 0) THEN
                     idata2d(:,i) = idmap_uc2gd_all(:,iloc)
                     rdata2d(:,i) = area_uc2gd_all (:,iloc)
                  ENDIF
               ENDDO

               CALL mpi_send (idata2d, nucpart*ngrd, MPI_INTEGER, p_address_worker(iworker), &
                  mpi_tag_data, p_comm_glb, p_err)
               CALL mpi_send (rdata2d, nucpart*ngrd, MPI_REAL8,   p_address_worker(iworker), &
                  mpi_tag_data, p_comm_glb, p_err)

               deallocate (grdindex)
               deallocate (idata2d )
               deallocate (rdata2d )
            ENDIF
         ENDDO

      ELSEIF (p_is_worker) THEN

         CALL mpi_send (numinpm, 1, MPI_INTEGER, p_address_master, mpi_tag_mesg, p_comm_glb, p_err)

         IF (numinpm > 0) THEN

            CALL mpi_send (inpm_gdid, numinpm, MPI_INTEGER, p_address_master, &
               mpi_tag_data, p_comm_glb, p_err)

            allocate (idmap_uc2gd (nucpart,numinpm))
            CALL mpi_recv (idmap_uc2gd, nucpart*numinpm, MPI_INTEGER, p_address_master, &
               mpi_tag_data, p_comm_glb, p_stat, p_err)

            allocate (area_uc2gd (nucpart,numinpm))
            CALL mpi_recv (area_uc2gd, nucpart*numinpm, MPI_REAL8, p_address_master, &
               mpi_tag_data, p_comm_glb, p_stat, p_err)

         ENDIF

      ENDIF

      CALL mpi_barrier (p_comm_glb, p_err)
#else
      allocate (idmap_uc2gd (nucpart,numinpm))
      allocate (area_uc2gd  (nucpart,numinpm))
      idmap_uc2gd = 0
      area_uc2gd  = 0.

      DO i = 1, numinpm
         iloc = find_in_sorted_list1 (inpm_gdid(i), ngrdall, allgrd_in_inp(1:ngrdall))
         IF (iloc > 0) THEN
            idmap_uc2gd(:,i) = idmap_uc2gd_all(:,iloc)
            area_uc2gd (:,i) = area_uc2gd_all (:,iloc)
         ENDIF
      ENDDO
#endif

      IF (p_is_worker) THEN
         IF (numucat > 0) THEN
            allocate (ucat_gdid (numucat))
            ucat_gdid = (y_ucat-1)*nlon_ucat + x_ucat
         ENDIF
      ENDIF

      CALL build_worker_pushdata (numinpm, inpm_gdid, numucat, idmap_gd2uc, area_gd2uc, push_inpm2ucat)
      CALL build_worker_pushdata (numucat, ucat_ucid, numinpm, idmap_uc2gd, area_uc2gd, push_ucat2inpm)
      CALL build_worker_pushdata (numucat, ucat_gdid, numinpm, inpm_gdid, push_ucat2grid)
      CALL build_worker_pushdata (numinpm, inpm_gdid, numinpm, inpm_gdid, allreduce_inpm)

      IF (p_is_master) THEN
         deallocate (idmap_x        )
         deallocate (idmap_y        )
         deallocate (allgrd_in_inp  )
         deallocate (nucat_g2d      )
         deallocate (iucat_g        )
         deallocate (idmap_uc2gd_all)
         deallocate (area_uc2gd_all )
      ENDIF

      ! ----- Part 2: between upstream and downstream unit catchments -----

      IF (p_is_master) THEN

         upnmax = maxval(nups_nst)
         allocate (ucat_ups (upnmax,totalnumucat))
         ucat_ups(:,:) = 0

         iups_nst(:) = 0
         DO i = 1, totalnumucat
            j = ucat_next(i)
            IF (j > 0) THEN
               iups_nst(j) = iups_nst(j) + 1
               ucat_ups(iups_nst(j),j) = i
            ENDIF
         ENDDO

      ENDIF


#ifdef USEMPI
      CALL mpi_bcast (upnmax, 1, MPI_INTEGER, p_address_master, p_comm_glb, p_err)

      IF (p_is_master) THEN

         DO iworker = 0, p_np_worker-1

            nucat = numucat_wrk(iworker)

            IF (nucat > 0) THEN
               allocate (idata1d (nucat))
               idata1d = ucat_next(ucat_data_address(iworker)%val)
               CALL mpi_send (idata1d, nucat, MPI_INTEGER, p_address_worker(iworker), &
                  mpi_tag_data, p_comm_glb, p_err)

               allocate (idata2d (upnmax,nucat))
               DO i = 1, nucat
                  idata2d(:,i) = ucat_ups(:,ucat_data_address(iworker)%val(i))
               ENDDO
               CALL mpi_send (idata2d, upnmax*nucat, MPI_INTEGER, p_address_worker(iworker), &
                  mpi_tag_data, p_comm_glb, p_err)

               deallocate (idata1d)
               deallocate (idata2d)
            ENDIF
         ENDDO

         deallocate (ucat_ups )

      ELSEIF (p_is_worker) THEN

         IF (numucat > 0) THEN

            allocate (ucat_next (numucat))
            CALL mpi_recv (ucat_next, numucat, MPI_INTEGER, p_address_master, &
               mpi_tag_data, p_comm_glb, p_stat, p_err)

            allocate (ucat_ups (upnmax, numucat))
            CALL mpi_recv (ucat_ups, upnmax*numucat, MPI_INTEGER, p_address_master, &
               mpi_tag_data, p_comm_glb, p_stat, p_err)

         ENDIF

      ENDIF

      CALL mpi_barrier (p_comm_glb, p_err)

      ! Non-worker ranks need zero-length arrays for build_worker_pushdata
      IF (.not. p_is_worker) THEN
         IF (.not. allocated(ucat_next)) allocate (ucat_next (0))
         IF (.not. allocated(ucat_ups )) allocate (ucat_ups  (upnmax, 0))
         IF (.not. allocated(wts_ups  )) allocate (wts_ups   (upnmax, 0))
      ENDIF
#endif

      IF (p_is_worker) THEN
         IF (numucat > 0) THEN
            allocate (wts_ups (upnmax,numucat))
            wts_ups(:,:) = 1.
         ENDIF
      ENDIF

      CALL build_worker_pushdata (numucat, ucat_ucid, numucat, ucat_next, push_next2ucat)
      CALL build_worker_pushdata (numucat, ucat_ucid, numucat, ucat_ups,  wts_ups, push_ups2ucat )

#ifdef CoLMDEBUG
      ! IF (p_is_worker) THEN
      !    write(*,'(A,I0,A,I0,A,I0,A)') 'worker ', p_iam_worker, ' has ', numucat, &
      !       ' unit catchment with ', sum(push_next2ucat%n_from_other), ' downstream to other workers'
      ! ENDIF
#endif

      ! ----- Part 3: river systems -----

#ifdef USEMPI
      IF (p_is_master) THEN
         DO iworker = 0, p_np_worker-1
            nucat = numucat_wrk(iworker)
            IF (nucat > 0) THEN
               allocate (idata1d (nucat))
               idata1d = rivermouth(ucat_data_address(iworker)%val)
               CALL mpi_send (idata1d, nucat, MPI_INTEGER, p_address_worker(iworker), &
                  mpi_tag_data, p_comm_glb, p_err)
               deallocate (idata1d)
            ENDIF
         ENDDO
      ELSEIF (p_is_worker) THEN
         IF (numucat > 0) THEN
            allocate (rivermouth (numucat))
            CALL mpi_recv (rivermouth, numucat, MPI_INTEGER, p_address_master, &
               mpi_tag_data, p_comm_glb, p_stat, p_err)
         ENDIF
      ENDIF

      rivsys_by_multiple_procs = .false.
      IF (p_is_worker) THEN
         IF (numucat > 0) THEN
            color = maxval(rivermouth)
            CALL mpi_comm_split (p_comm_worker, color, p_iam_worker, p_comm_rivsys, p_err)
         ELSE
            CALL mpi_comm_split (p_comm_worker, MPI_UNDEFINED, p_iam_worker, p_comm_rivsys, p_err)
         ENDIF

         IF (p_comm_rivsys /= MPI_COMM_NULL) THEN
            CALL mpi_comm_size (p_comm_rivsys, p_np_rivsys, p_err)
            IF (p_np_rivsys > 1) THEN
               rivsys_by_multiple_procs = .true.
            ENDIF
         ENDIF
      ENDIF

      ! The scalar p_comm_rivsys reduction below is valid only because the
      ! partitioner reserves each multi-rank river system an exclusive worker
      ! range.  Make that implicit invariant executable: a future partitioner
      ! change must not silently put two river systems on one member of a
      ! multi-rank communicator and then reduce only one dt value.
      invalid_rivsys_partition = .false.
      IF (p_is_worker .and. rivsys_by_multiple_procs .and. numucat > 0) THEN
         invalid_rivsys_partition = minval(rivermouth) /= maxval(rivermouth)
      ENDIF
      CALL mpi_allreduce (MPI_IN_PLACE, invalid_rivsys_partition, 1, MPI_LOGICAL, &
         MPI_LOR, p_comm_glb, p_err)
      IF (invalid_rivsys_partition) THEN
         IF (p_is_master) write(*,'(A)') &
            'ERROR: a multi-rank river communicator contains more than one river system.'
         CALL CoLM_stop ('invalid river-system MPI partition')
      ENDIF
#else
      rivsys_by_multiple_procs = .false.
#endif

      IF (p_is_worker) THEN

         numrivsys = 0
         allocate (irivsys (numucat))

         IF (.not. rivsys_by_multiple_procs) THEN
            IF (numucat > 0) THEN

               allocate (order_ucat (numucat))
               order_ucat = (/(i, i = 1, numucat)/)

               CALL quicksort (numucat, rivermouth, order_ucat)

               numrivsys = 1
               irivsys(order_ucat(1)) = numrivsys
               DO i = 2, numucat
                  IF (rivermouth(i) /= rivermouth(i-1)) THEN
                     numrivsys = numrivsys + 1
                  ENDIF
                  irivsys(order_ucat(i)) = numrivsys
               ENDDO

            ENDIF
         ELSE
            numrivsys  = 1
            irivsys(:) = 1
         ENDIF

      ENDIF

      IF (allocated(rivermouth)) deallocate(rivermouth)
      IF (allocated(order_ucat)) deallocate(order_ucat)

      ! ----- Parameters for River and Lake -----

      CALL readin_riverlake_parameter (parafile, 'topo_rivelv',    rdata1d = topo_rivelv   )
      CALL readin_riverlake_parameter (parafile, 'topo_rivhgt',    rdata1d = topo_rivhgt   )
      CALL readin_riverlake_parameter (parafile, 'topo_rivlen',    rdata1d = topo_rivlen   )
      CALL readin_riverlake_parameter (parafile, 'topo_rivman',    rdata1d = topo_rivman   )
      CALL readin_riverlake_parameter (parafile, 'topo_rivwth',    rdata1d = topo_rivwth   )
      CALL readin_riverlake_parameter (parafile, 'topo_rivstomax', rdata1d = topo_rivstomax)
      CALL readin_riverlake_parameter (parafile, 'topo_area',      rdata1d = topo_area     )
      CALL readin_riverlake_parameter (parafile, 'topo_fldhgt',    rdata2d = topo_fldhgt   )

      IF (DEF_USE_LEVEE) THEN
         CALL readin_riverlake_parameter (parafile, 'levee_frc', rdata1d = levee_frc_data)
         CALL readin_riverlake_parameter (parafile, 'levee_hgt', rdata1d = levee_hgt_data)
      ENDIF

      IF (DEF_USE_BIFURCATION) THEN
         CALL read_and_distribute_bifurcation (parafile)
      ENDIF

      IF (p_is_worker) THEN
         IF (numucat > 0) THEN

            allocate (lake_type (numucat))
            lake_type(:) = 0

            allocate (topo_rivare (numucat))
            topo_rivare = topo_rivstomax / topo_rivhgt

            allocate (floodplain_curve (numucat))

            DO i = 1, numucat
               floodplain_curve(i)%nlfp      = size(topo_fldhgt,1)
               floodplain_curve(i)%rivhgt    = topo_rivhgt(i)
               floodplain_curve(i)%rivstomax = topo_rivstomax(i)
               floodplain_curve(i)%rivare    = topo_rivare(i)

               allocate (floodplain_curve(i)%flphgt    (0:floodplain_curve(i)%nlfp))
               allocate (floodplain_curve(i)%flparea   (0:floodplain_curve(i)%nlfp))
               allocate (floodplain_curve(i)%flpaccare (0:floodplain_curve(i)%nlfp))
               allocate (floodplain_curve(i)%flpstomax (0:floodplain_curve(i)%nlfp))

               floodplain_curve(i)%flphgt(0)  = 0.
               floodplain_curve(i)%flphgt(1:) = topo_fldhgt(:,i)

               floodplain_curve(i)%flparea(0)  = 0.
               floodplain_curve(i)%flparea(1:) = topo_area(i) / floodplain_curve(i)%nlfp

               floodplain_curve(i)%flpaccare(0) = 0.
               DO j = 1, floodplain_curve(i)%nlfp
                  floodplain_curve(i)%flpaccare(j) = &
                     floodplain_curve(i)%flpaccare(j-1) + floodplain_curve(i)%flparea(j)
               ENDDO

               floodplain_curve(i)%flpstomax(0) = 0.
               DO j = 1, floodplain_curve(i)%nlfp
                  floodplain_curve(i)%flpstomax(j) = floodplain_curve(i)%flpstomax(j-1)        &
                     + 0.5 * (floodplain_curve(i)%flpaccare(j) + floodplain_curve(i)%flpaccare(j-1)) &
                           * (floodplain_curve(i)%flphgt(j)  - floodplain_curve(i)%flphgt(j-1))
               ENDDO
            ENDDO

            allocate (bedelv_next (numucat))
            allocate (outletwth   (numucat))

         ENDIF
      ENDIF

      CALL worker_push_data (push_next2ucat, topo_rivelv, bedelv_next, fillvalue = spval)
      CALL worker_push_data (push_next2ucat, topo_rivwth, outletwth  , fillvalue = spval)

      IF (p_is_worker) THEN
         IF (numucat > 0) THEN
            WHERE (ucat_next > 0)
               outletwth = (outletwth + topo_rivwth) * 0.5
            ELSEWHERE
               outletwth = topo_rivwth
            END WHERE
         ENDIF
      ENDIF

      ! ----- Mask of Grids with all upstream area in the simulation region -----

      IF (p_is_master) allocate (ucat_area_all (totalnumucat))

#ifdef USEMPI
      IF (p_is_worker) THEN

         IF (numucat > 0) THEN
            CALL mpi_send (push_inpm2ucat%sum_area, numucat, MPI_REAL8, p_address_master, &
               mpi_tag_data, p_comm_glb, p_err)
         ENDIF

      ELSEIF (p_is_master) THEN

         DO iworker = 0, p_np_worker-1
            IF (numucat_wrk(iworker) > 0) THEN

               allocate (rdata1d (numucat_wrk(iworker)))
               CALL mpi_recv (rdata1d, numucat_wrk(iworker), MPI_REAL8, p_address_worker(iworker), &
                  mpi_tag_data, p_comm_glb, p_stat, p_err)

               ucat_area_all(ucat_data_address(iworker)%val) = rdata1d

               deallocate (rdata1d)
            ENDIF

         ENDDO
      ENDIF
#else
      ucat_area_all = push_inpm2ucat%sum_area
#endif

      IF (p_is_master) THEN

         allocate (allups_mask_ucat (totalnumucat))
         allups_mask_ucat (:) = 0

         iups_nst(:) = 0
         DO i = 1, totalnumucat
            j = uc_up2down(i)
            IF (ucat_area_all(j) > 0.) THEN
               IF (iups_nst(j) == nups_nst(j)) THEN

                  allups_mask_ucat(j) = 1

                  IF (ucat_next(j) > 0) THEN
                     iups_nst(ucat_next(j)) = iups_nst(ucat_next(j)) + 1
                  ENDIF
               ENDIF
            ENDIF
         ENDDO

      ENDIF

#ifdef USEMPI
      IF (p_is_master) THEN
         DO iworker = 0, p_np_worker-1
            IF (numucat_wrk(iworker) > 0) THEN
               allocate (rdata1d (numucat_wrk(iworker)))
               rdata1d = allups_mask_ucat(ucat_data_address(iworker)%val)

               CALL mpi_send (rdata1d, numucat_wrk(iworker), MPI_REAL8, p_address_worker(iworker), &
                  mpi_tag_data, p_comm_glb, p_err)

               deallocate (rdata1d)
            ENDIF
         ENDDO
      ELSEIF (p_is_worker) THEN
         IF (numucat > 0) THEN
            allocate (allups_mask_ucat (numucat))
            CALL mpi_recv (allups_mask_ucat, numucat, MPI_REAL8, p_address_master, &
               mpi_tag_data, p_comm_glb, p_stat, p_err)
         ENDIF
      ENDIF
#endif


      IF (allocated (uc_up2down   )) deallocate (uc_up2down   )
      IF (allocated (nups_nst     )) deallocate (nups_nst     )
      IF (allocated (iups_nst     )) deallocate (iups_nst     )
      IF (allocated (ucat_area_all)) deallocate (ucat_area_all)

   END SUBROUTINE build_riverlake_network

   ! ---------
   SUBROUTINE readin_riverlake_parameter (parafile, varname, rdata1d, rdata2d, idata1d)

   USE MOD_SPMD_Task
   USE MOD_NetCDFSerial
   IMPLICIT NONE

   character(len=*), intent(in) :: parafile
   character(len=*), intent(in) :: varname

   real(r8), allocatable, intent(inout), optional :: rdata1d (:)
   real(r8), allocatable, intent(inout), optional :: rdata2d (:,:)
   integer,  allocatable, intent(inout), optional :: idata1d (:)

   ! Local Variables
   integer :: iworker, nucat, ndim1, i
   real(r8), allocatable :: rsend1d (:)
   real(r8), allocatable :: rsend2d (:,:)
   integer,  allocatable :: isend1d (:)

      IF (p_is_master) THEN
         IF (present(rdata1d))  CALL ncio_read_serial (parafile, varname, rdata1d)
         IF (present(rdata2d))  CALL ncio_read_serial (parafile, varname, rdata2d)
         IF (present(idata1d))  CALL ncio_read_serial (parafile, varname, idata1d)
      ENDIF

#ifdef USEMPI
      CALL mpi_barrier (p_comm_glb, p_err)

      IF (present(rdata2d)) THEN
         IF (p_is_master) ndim1 = size(rdata2d,1)
         CALL mpi_bcast (ndim1, 1, MPI_INTEGER, p_address_master, p_comm_glb, p_err)
      ENDIF

      ! send unit catchment index to workers
      IF (p_is_master) THEN

         DO iworker = 0, p_np_worker-1

            nucat = numucat_wrk(iworker)

            IF (nucat > 0) THEN
               IF (present(rdata1d)) THEN
                  allocate (rsend1d (nucat))

                  rsend1d = rdata1d(ucat_data_address(iworker)%val)
                  CALL mpi_send (rsend1d, nucat, MPI_REAL8, p_address_worker(iworker), &
                     mpi_tag_data, p_comm_glb, p_err)

                  deallocate (rsend1d)
               ENDIF

               IF (present(rdata2d)) THEN
                  allocate (rsend2d (ndim1,nucat))

                  DO i = 1, nucat
                     rsend2d(:,i) = rdata2d(:,ucat_data_address(iworker)%val(i))
                  ENDDO
                  CALL mpi_send (rsend2d, ndim1*nucat, MPI_REAL8, p_address_worker(iworker), &
                     mpi_tag_data, p_comm_glb, p_err)

                  deallocate (rsend2d)
               ENDIF

               IF (present(idata1d)) THEN
                  allocate (isend1d (nucat))

                  isend1d = idata1d(ucat_data_address(iworker)%val)
                  CALL mpi_send (isend1d, nucat, MPI_INTEGER, p_address_worker(iworker), &
                     mpi_tag_data, p_comm_glb, p_err)

                  deallocate (isend1d)
               ENDIF
            ENDIF
         ENDDO

         IF (present(rdata1d))  deallocate (rdata1d)
         IF (present(rdata2d))  deallocate (rdata2d)
         IF (present(idata1d))  deallocate (idata1d)

      ELSEIF (p_is_worker) THEN

         IF (numucat > 0) THEN
            IF (present(rdata1d)) THEN
               allocate (rdata1d (numucat))
               CALL mpi_recv (rdata1d, numucat, MPI_REAL8, p_address_master, &
                  mpi_tag_data, p_comm_glb, p_stat, p_err)
            ENDIF

            IF (present(rdata2d)) THEN
               allocate (rdata2d (ndim1,numucat))
               CALL mpi_recv (rdata2d, ndim1*numucat, MPI_REAL8, p_address_master, &
                  mpi_tag_data, p_comm_glb, p_stat, p_err)
            ENDIF

            IF (present(idata1d)) THEN
               allocate (idata1d (numucat))
               CALL mpi_recv (idata1d, numucat, MPI_INTEGER, p_address_master, &
                  mpi_tag_data, p_comm_glb, p_stat, p_err)
            ENDIF
         ENDIF

      ENDIF

      CALL mpi_barrier (p_comm_glb, p_err)
#endif

   END SUBROUTINE readin_riverlake_parameter

   !
   FUNCTION retrieve_depth_from_volume (this, volume) result(depth)

   IMPLICIT NONE

   class(vol_dep_curve_type) :: this
   real(r8), intent(in)      :: volume
   real(r8) :: depth

   ! Local Variables
   real(r8) :: v0, g
   integer  :: i

      v0 = volume - this%rivstomax
      IF (v0 <= 0) THEN
         depth = volume / this%rivare
      ELSE
         i = 1
         DO WHILE (i <= this%nlfp)
            IF (v0 > this%flpstomax(i)) THEN
               i = i + 1
            ELSE
               EXIT
            ENDIF
         ENDDO

         IF (i == this%nlfp+1) THEN
            depth = this%rivhgt + this%flphgt(this%nlfp) &
               + (v0-this%flpstomax(this%nlfp)) / this%flpaccare(this%nlfp)
         ELSE
            g = (this%flphgt(i)-this%flphgt(i-1))/this%flparea(i)
            depth = this%rivhgt + this%flphgt(i-1) &
               + g * (-this%flpaccare(i-1)+sqrt((this%flpaccare(i-1))**2+2*(v0-this%flpstomax(i-1))/g))
         ENDIF
      ENDIF

   END FUNCTION retrieve_depth_from_volume

   !
   FUNCTION retrieve_volume_from_depth (this, depth) result(volume)

   IMPLICIT NONE

   class(vol_dep_curve_type) :: this
   real(r8), intent(in)  :: depth
   real(r8) :: volume

   ! Local Variables
   real(r8) :: h, d
   integer  :: i

      IF (depth <= this%rivhgt) THEN
         volume = this%rivare * depth
      ELSE
         i = 1
         DO WHILE (i <= this%nlfp)
            IF (depth > this%rivhgt+this%flphgt(i)) THEN
               i = i + 1
            ELSE
               EXIT
            ENDIF
         ENDDO

         d = depth - this%rivhgt - this%flphgt(i-1)
         IF (i == this%nlfp+1) THEN
            volume = this%rivstomax + this%flpstomax(this%nlfp) + d * this%flpaccare(this%nlfp)
         ELSE
            h = this%flphgt(i)-this%flphgt(i-1)
            volume = this%rivstomax + this%flpstomax(i-1) &
               + (d/h*this%flparea(i)+2*this%flpaccare(i-1))*d*0.5
         ENDIF
      ENDIF

   END FUNCTION retrieve_volume_from_depth

   !
   FUNCTION retrieve_area_from_depth (this, depth) result(area)

   IMPLICIT NONE

   class(vol_dep_curve_type) :: this
   real(r8), intent(in)  :: depth
   real(r8) :: area

   ! Local Variables
   real(r8) :: h, d
   integer  :: i

      IF (depth <= this%rivhgt) THEN
         area = 0.
      ELSE
         i = 1
         DO WHILE (i <= this%nlfp)
            IF (depth > this%rivhgt+this%flphgt(i)) THEN
               i = i + 1
            ELSE
               EXIT
            ENDIF
         ENDDO

         IF (i == this%nlfp+1) THEN
            area = this%flpaccare(this%nlfp)
         ELSE
            h = this%flphgt(i)-this%flphgt(i-1)
            d = depth - this%rivhgt - this%flphgt(i-1)
            area = this%flpaccare(i-1) + d/h * this%flparea(i)
         ENDIF
      ENDIF

   END FUNCTION retrieve_area_from_depth

   ! ---
   SUBROUTINE vol_depth_curve_free_mem (this)

   IMPLICIT NONE
   type(vol_dep_curve_type) :: this

      IF (allocated(this%flphgt   )) deallocate (this%flphgt   )
      IF (allocated(this%flparea  )) deallocate (this%flparea  )
      IF (allocated(this%flpaccare)) deallocate (this%flpaccare)
      IF (allocated(this%flpstomax)) deallocate (this%flpstomax)

   END SUBROUTINE vol_depth_curve_free_mem

   ! ---------
   SUBROUTINE riverlake_network_final ()

#ifdef USEMPI
   USE MOD_SPMD_Task, ONLY: p_err
#endif
   IMPLICIT NONE

      ! Module-scope derived objects are not finalized when this routine
      ! returns. Release their owned allocatables explicitly so a later
      ! riverlake_network_init can rebuild them without allocate-on-allocated
      ! failures. These cleanup routines are local deallocations only.
      CALL worker_pushdata_free_mem (push_inpm2ucat)
      CALL worker_pushdata_free_mem (push_ucat2inpm)
      CALL worker_pushdata_free_mem (push_ucat2grid)
      CALL worker_pushdata_free_mem (allreduce_inpm)
      CALL worker_pushdata_free_mem (push_next2ucat)
      CALL worker_pushdata_free_mem (push_ups2ucat)
      CALL worker_pushdata_free_mem (push_bif_dn2pth)
      CALL worker_pushdata_free_mem (push_bif_influx)
      CALL worker_remapdata_free_mem (remap_patch2inpm)
      CALL grid_free_mem (griducat)

      IF (allocated(x_ucat           )) deallocate(x_ucat           )
      IF (allocated(y_ucat           )) deallocate(y_ucat           )

      IF (allocated(ucat_ucid        )) deallocate(ucat_ucid        )
      IF (allocated(ucat_gdid        )) deallocate(ucat_gdid        )

      IF (allocated(numucat_wrk      )) deallocate(numucat_wrk      )
      IF (allocated(ucat_data_address)) deallocate(ucat_data_address)

      IF (allocated(inpm_gdid        )) deallocate(inpm_gdid        )
      IF (allocated(idmap_gd2uc      )) deallocate(idmap_gd2uc      )
      IF (allocated(area_gd2uc       )) deallocate(area_gd2uc       )
      IF (allocated(idmap_uc2gd      )) deallocate(idmap_uc2gd      )
      IF (allocated(area_uc2gd       )) deallocate(area_uc2gd       )
      IF (allocated(ucat_next        )) deallocate(ucat_next        )
      IF (allocated(ucat_ups         )) deallocate(ucat_ups         )
      IF (allocated(wts_ups          )) deallocate(wts_ups          )
      IF (allocated(irivsys          )) deallocate(irivsys          )
      IF (allocated(lake_type        )) deallocate(lake_type        )

      IF (allocated(topo_rivelv      )) deallocate(topo_rivelv      )
      IF (allocated(topo_rivhgt      )) deallocate(topo_rivhgt      )
      IF (allocated(topo_rivlen      )) deallocate(topo_rivlen      )
      IF (allocated(topo_rivman      )) deallocate(topo_rivman      )
      IF (allocated(topo_rivwth      )) deallocate(topo_rivwth      )
      IF (allocated(topo_rivare      )) deallocate(topo_rivare      )
      IF (allocated(topo_rivstomax   )) deallocate(topo_rivstomax   )
      IF (allocated(topo_area        )) deallocate(topo_area        )
      IF (allocated(topo_fldhgt      )) deallocate(topo_fldhgt      )
      IF (allocated(levee_frc_data   )) deallocate(levee_frc_data   )
      IF (allocated(levee_hgt_data   )) deallocate(levee_hgt_data   )
      IF (allocated(bedelv_next      )) deallocate(bedelv_next      )
      IF (allocated(outletwth        )) deallocate(outletwth        )

      IF (allocated(floodplain_curve )) deallocate(floodplain_curve )

      IF (allocated(allups_mask_ucat )) deallocate(allups_mask_ucat )

      ! ----- Bifurcation arrays -----
      IF (allocated(pth_upst_local    )) deallocate(pth_upst_local    )
      IF (allocated(pth_down_local    )) deallocate(pth_down_local    )
      IF (allocated(pth_down_ucid     )) deallocate(pth_down_ucid     )
      IF (allocated(pth_global_id     )) deallocate(pth_global_id     )
      IF (allocated(pth_dst           )) deallocate(pth_dst           )
      IF (allocated(pth_elv           )) deallocate(pth_elv           )
      IF (allocated(pth_wth           )) deallocate(pth_wth           )
      IF (allocated(pth_man           )) deallocate(pth_man           )
      IF (allocated(bif_incoming_pths )) deallocate(bif_incoming_pths )
      IF (allocated(bif_incoming_wts  )) deallocate(bif_incoming_wts  )

#ifdef USEMPI
      IF (p_comm_rivsys /= MPI_COMM_NULL) THEN
         CALL mpi_comm_free (p_comm_rivsys, p_err)
         p_comm_rivsys = MPI_COMM_NULL
      ENDIF
#endif

   END SUBROUTINE riverlake_network_final

   ! ---------
   SUBROUTINE read_and_distribute_bifurcation (parafile)

   USE MOD_SPMD_Task
   USE MOD_NetCDFSerial
   USE MOD_Utils
   IMPLICIT NONE

   character(len=*), intent(in) :: parafile

   ! Local Variables
   integer,  allocatable :: bif_upst_all  (:)   ! upstream seq index (global)
   integer,  allocatable :: bif_down_all  (:)   ! downstream seq index (global)
   real(r8), allocatable :: bif_dist_all  (:)   ! channel distance
   real(r8), allocatable :: bif_elev_all  (:,:) ! elevation profile (npthlev, npthout)
   real(r8), allocatable :: bif_wdth_all  (:,:) ! width profile (npthlev, npthout)
   real(r8), allocatable :: bif_mann_all  (:)   ! Manning coefficients (npthlev)

   integer,  allocatable :: iworker_of_ucat (:) ! which worker owns each global ucat
   integer,  allocatable :: pth_owner       (:) ! which worker owns each pathway
   integer,  allocatable :: npth_wrk        (:) ! number of pathways per worker

   ! Per-worker send buffers
   integer,  allocatable :: pth_upst_send (:)
   integer,  allocatable :: pth_down_send (:)
   integer,  allocatable :: pth_glid_send (:)
   real(r8), allocatable :: pth_dist_send (:)
   real(r8), allocatable :: pth_elev_send (:,:)
   real(r8), allocatable :: pth_wdth_send (:,:)

   ! Reverse mapping temporaries
   integer,  allocatable :: bif_inc_cnt   (:)   ! count of incoming pathways per ucat (global)
   integer,  allocatable :: bif_inc_all   (:,:) ! global pathway IDs incoming to each ucat
   integer,  allocatable :: bif_inc_send  (:,:)
   real(r8), allocatable :: bif_wt_send   (:,:)

   integer :: iworker, nucat, npth, ip, i, j, iloc
   integer :: max_bif_inc_global
#ifdef CoLMDEBUG
   ! Debug-only bifurcation-connected-component diagnostic.
   integer,  allocatable :: uf_parent (:)   ! union-find over global ucats 1..totalnumucat
   integer,  allocatable :: sys_root  (:)   ! main-channel (river-system) root per ucat
   integer,  allocatable :: comp_nsys (:)   ! river systems per bif-connected component
   integer :: ib, ra, rb, k, nsys, ncomp_bif, max_sys_in_comp
#endif

#ifdef USEMPI

      ! ================================================================
      ! Master: read NetCDF data and prepare for distribution
      ! ================================================================
      IF (p_is_master) THEN

         CALL read_bifurcation_global_arrays (parafile, bif_upst_all, bif_down_all, &
            bif_dist_all, bif_elev_all, bif_wdth_all, bif_mann_all)

#ifdef CoLMDEBUG
         ! Debug-only connectivity summary; do not print or allocate in production.
         IF (totalnumucat > 0) THEN
            allocate (uf_parent (totalnumucat))
            allocate (sys_root  (totalnumucat))
            allocate (comp_nsys (totalnumucat))
            DO k = 1, totalnumucat
               uf_parent(k) = k
            ENDDO
            ! Phase 1: main-channel edges (each drainage tree = one river system)
            DO k = 1, totalnumucat
               ib = ucat_next(k)
               IF (ib < 1 .or. ib > totalnumucat) CYCLE
               ra = k
               DO WHILE (uf_parent(ra) /= ra)
                  uf_parent(ra) = uf_parent(uf_parent(ra));  ra = uf_parent(ra)
               ENDDO
               rb = ib
               DO WHILE (uf_parent(rb) /= rb)
                  uf_parent(rb) = uf_parent(uf_parent(rb));  rb = uf_parent(rb)
               ENDDO
               IF (ra /= rb) uf_parent(max(ra,rb)) = min(ra,rb)
            ENDDO
            ! Snapshot river-system roots before adding bifurcation edges
            DO k = 1, totalnumucat
               ra = k
               DO WHILE (uf_parent(ra) /= ra)
                  ra = uf_parent(ra)
               ENDDO
               sys_root(k) = ra
            ENDDO
            ! Phase 2: bifurcation edges (merge bif-linked systems)
            DO ip = 1, totalnpthout
               IF (bif_down_all(ip) < 1 .or. bif_down_all(ip) > totalnumucat) CYCLE
               ra = bif_upst_all(ip)
               DO WHILE (uf_parent(ra) /= ra)
                  uf_parent(ra) = uf_parent(uf_parent(ra));  ra = uf_parent(ra)
               ENDDO
               rb = bif_down_all(ip)
               DO WHILE (uf_parent(rb) /= rb)
                  uf_parent(rb) = uf_parent(uf_parent(rb));  rb = uf_parent(rb)
               ENDDO
               IF (ra /= rb) uf_parent(max(ra,rb)) = min(ra,rb)
            ENDDO
            ! Count river systems per final component (system reps: sys_root(k)==k)
            comp_nsys(:) = 0
            nsys = 0
            DO k = 1, totalnumucat
               IF (sys_root(k) /= k) CYCLE
               nsys = nsys + 1
               ra = k
               DO WHILE (uf_parent(ra) /= ra)
                  ra = uf_parent(ra)
               ENDDO
               comp_nsys(ra) = comp_nsys(ra) + 1
            ENDDO
            ncomp_bif       = count(comp_nsys > 0)
            max_sys_in_comp = maxval(comp_nsys)
            write(*,'(A)')    '===== Bifurcation connectivity diagnostic (STEP-0 go/no-go) ====='
            write(*,'(A,I0)') '  river systems                    : ', nsys
            write(*,'(A,I0)') '  bifurcation paths (totalnpthout) : ', totalnpthout
            write(*,'(A,I0)') '  bif-connected components         : ', ncomp_bif
            write(*,'(A,I0,A,F6.2,A)') '  largest component (systems)      : ', max_sys_in_comp, &
               ' (', 100._r8*real(max_sys_in_comp,r8)/real(max(nsys,1),r8), '% of systems)'
            write(*,'(A)')    '  GUIDE: largest >~50% of systems => per-component dt gives ~no'
            write(*,'(A)')    '         speedup (one giant component); small/many => worth it.'
            write(*,'(A)')    '================================================================'
            deallocate (uf_parent, sys_root, comp_nsys)
         ENDIF
#endif

         ! Build iworker_of_ucat: maps global seq index -> worker index
         allocate (iworker_of_ucat (totalnumucat))
         iworker_of_ucat(:) = -1
         DO iworker = 0, p_np_worker-1
            DO i = 1, numucat_wrk(iworker)
               iworker_of_ucat(ucat_data_address(iworker)%val(i)) = iworker
            ENDDO
         ENDDO

         ! Assign each pathway to the worker that owns its upstream cell
         allocate (pth_owner (totalnpthout))
         allocate (npth_wrk  (0:p_np_worker-1))
         npth_wrk(:) = 0
         DO ip = 1, totalnpthout
            pth_owner(ip) = iworker_of_ucat(bif_upst_all(ip))
            IF (pth_owner(ip) < 0) CALL CoLM_stop ('bifurcation upstream owner not found')
            npth_wrk(pth_owner(ip)) = npth_wrk(pth_owner(ip)) + 1
         ENDDO

         ! Build reverse mapping: for each ucat, which pathways have it as downstream
         allocate (bif_inc_cnt (totalnumucat))
         bif_inc_cnt(:) = 0
         DO ip = 1, totalnpthout
            j = bif_down_all(ip)
            IF (j > 0 .and. j <= totalnumucat) THEN
               bif_inc_cnt(j) = bif_inc_cnt(j) + 1
            ENDIF
         ENDDO
         max_bif_inc_global = maxval(bif_inc_cnt)
         IF (max_bif_inc_global < 1) max_bif_inc_global = 1

         allocate (bif_inc_all (max_bif_inc_global, totalnumucat))
         bif_inc_all(:,:) = 0
         bif_inc_cnt(:) = 0
         DO ip = 1, totalnpthout
            j = bif_down_all(ip)
            IF (j > 0 .and. j <= totalnumucat) THEN
               bif_inc_cnt(j) = bif_inc_cnt(j) + 1
               bif_inc_all(bif_inc_cnt(j), j) = ip
            ENDIF
         ENDDO

      ENDIF

      ! Broadcast scalar dimensions
      CALL mpi_bcast (totalnpthout, 1, MPI_INTEGER, p_address_master, p_comm_glb, p_err)
      CALL mpi_bcast (npthlev_bif,  1, MPI_INTEGER, p_address_master, p_comm_glb, p_err)

      ! Broadcast Manning coefficients (shared by all ranks participating in p_comm_glb).
      ! IO ranks also enter this broadcast, so they need a valid receive buffer.
      IF (.not. p_is_master) allocate (pth_man (npthlev_bif))
      IF (p_is_master) THEN
         ! Copy from bif_mann_all before broadcast
         IF (.not. allocated(pth_man)) allocate (pth_man (npthlev_bif))
         pth_man(:) = bif_mann_all(:)
         deallocate (bif_mann_all)
      ENDIF
      CALL mpi_bcast (pth_man, npthlev_bif, MPI_REAL8, p_address_master, p_comm_glb, p_err)

      ! Broadcast max_bif_incoming
      IF (p_is_master) max_bif_incoming = max_bif_inc_global
      CALL mpi_bcast (max_bif_incoming, 1, MPI_INTEGER, p_address_master, p_comm_glb, p_err)

      ! ================================================================
      ! Distribute pathway data and reverse mapping to workers
      ! ================================================================
      IF (p_is_master) THEN

         DO iworker = 0, p_np_worker-1

            npth  = npth_wrk(iworker)
            nucat = numucat_wrk(iworker)

            ! Send pathway count
            CALL mpi_send (npth, 1, MPI_INTEGER, p_address_worker(iworker), &
               mpi_tag_mesg, p_comm_glb, p_err)

            IF (npth > 0) THEN
               ! Pack pathway data for this worker
               allocate (pth_upst_send (npth))
               allocate (pth_down_send (npth))
               allocate (pth_glid_send (npth))
               allocate (pth_dist_send (npth))
               allocate (pth_elev_send (npthlev_bif, npth))
               allocate (pth_wdth_send (npthlev_bif, npth))

               j = 0
               DO ip = 1, totalnpthout
                  IF (pth_owner(ip) == iworker) THEN
                     j = j + 1
                     pth_upst_send(j) = bif_upst_all(ip)
                     pth_down_send(j) = bif_down_all(ip)
                     pth_glid_send(j) = ip
                     pth_dist_send(j) = bif_dist_all(ip)
                     pth_elev_send(:,j) = bif_elev_all(:,ip)
                     pth_wdth_send(:,j) = bif_wdth_all(:,ip)
                  ENDIF
               ENDDO

               CALL mpi_send (pth_upst_send, npth, MPI_INTEGER, &
                  p_address_worker(iworker), mpi_tag_data, p_comm_glb, p_err)
               CALL mpi_send (pth_down_send, npth, MPI_INTEGER, &
                  p_address_worker(iworker), mpi_tag_data, p_comm_glb, p_err)
               CALL mpi_send (pth_glid_send, npth, MPI_INTEGER, &
                  p_address_worker(iworker), mpi_tag_data, p_comm_glb, p_err)
               CALL mpi_send (pth_dist_send, npth, MPI_REAL8, &
                  p_address_worker(iworker), mpi_tag_data, p_comm_glb, p_err)
               CALL mpi_send (pth_elev_send, npthlev_bif*npth, MPI_REAL8, &
                  p_address_worker(iworker), mpi_tag_data, p_comm_glb, p_err)
               CALL mpi_send (pth_wdth_send, npthlev_bif*npth, MPI_REAL8, &
                  p_address_worker(iworker), mpi_tag_data, p_comm_glb, p_err)

               deallocate (pth_upst_send)
               deallocate (pth_down_send)
               deallocate (pth_glid_send)
               deallocate (pth_dist_send)
               deallocate (pth_elev_send)
               deallocate (pth_wdth_send)
            ENDIF

            ! Send reverse mapping for this worker's ucats
            IF (nucat > 0) THEN
               allocate (bif_inc_send (max_bif_inc_global, nucat))
               allocate (bif_wt_send  (max_bif_inc_global, nucat))
               DO i = 1, nucat
                  bif_inc_send(:,i) = bif_inc_all(:,ucat_data_address(iworker)%val(i))
               ENDDO
               bif_wt_send(:,:) = 0.
               WHERE (bif_inc_send > 0) bif_wt_send = 1.

               CALL mpi_send (bif_inc_send, max_bif_inc_global*nucat, MPI_INTEGER, &
                  p_address_worker(iworker), mpi_tag_data, p_comm_glb, p_err)
               CALL mpi_send (bif_wt_send,  max_bif_inc_global*nucat, MPI_REAL8, &
                  p_address_worker(iworker), mpi_tag_data, p_comm_glb, p_err)

               deallocate (bif_inc_send)
               deallocate (bif_wt_send)
            ENDIF

         ENDDO

         ! Clean up master arrays
         deallocate (bif_upst_all)
         deallocate (bif_down_all)
         deallocate (bif_dist_all)
         deallocate (bif_elev_all)
         deallocate (bif_wdth_all)
         deallocate (iworker_of_ucat)
         deallocate (pth_owner)
         deallocate (npth_wrk)
         deallocate (bif_inc_cnt)
         deallocate (bif_inc_all)

      ELSEIF (p_is_worker) THEN

         ! Receive pathway count
         CALL mpi_recv (npthout_local, 1, MPI_INTEGER, p_address_master, &
            mpi_tag_mesg, p_comm_glb, p_stat, p_err)

         IF (npthout_local > 0) THEN
            allocate (pth_upst_local (npthout_local))
            allocate (pth_down_ucid  (npthout_local))
            allocate (pth_global_id  (npthout_local))
            allocate (pth_dst        (npthout_local))
            allocate (pth_elv        (npthlev_bif, npthout_local))
            allocate (pth_wth        (npthlev_bif, npthout_local))

            ! Receive raw global indices first
            CALL mpi_recv (pth_upst_local, npthout_local, MPI_INTEGER, p_address_master, &
               mpi_tag_data, p_comm_glb, p_stat, p_err)
            CALL mpi_recv (pth_down_ucid,  npthout_local, MPI_INTEGER, p_address_master, &
               mpi_tag_data, p_comm_glb, p_stat, p_err)
            CALL mpi_recv (pth_global_id,  npthout_local, MPI_INTEGER, p_address_master, &
               mpi_tag_data, p_comm_glb, p_stat, p_err)
            CALL mpi_recv (pth_dst,        npthout_local, MPI_REAL8,   p_address_master, &
               mpi_tag_data, p_comm_glb, p_stat, p_err)
            CALL mpi_recv (pth_elv,        npthlev_bif*npthout_local, MPI_REAL8, p_address_master, &
               mpi_tag_data, p_comm_glb, p_stat, p_err)
            CALL mpi_recv (pth_wth,        npthlev_bif*npthout_local, MPI_REAL8, p_address_master, &
               mpi_tag_data, p_comm_glb, p_stat, p_err)

            allocate (pth_down_local (npthout_local))
            CALL localize_bifurcation_path_indices (pth_upst_local, pth_down_ucid, pth_down_local)
         ELSE
            npthout_local = 0
            ! Allocate zero-size arrays so they are safely passable to
            ! assumed-shape dummy arguments (e.g. build_worker_pushdata).
            allocate (pth_upst_local (0))
            allocate (pth_down_ucid  (0))
            allocate (pth_down_local (0))
            allocate (pth_global_id  (0))
            allocate (pth_dst        (0))
            allocate (pth_elv        (npthlev_bif, 0))
            allocate (pth_wth        (npthlev_bif, 0))
         ENDIF

         ! Receive reverse mapping
         IF (numucat > 0) THEN
            allocate (bif_incoming_pths (max_bif_incoming, numucat))
            allocate (bif_incoming_wts  (max_bif_incoming, numucat))
            CALL mpi_recv (bif_incoming_pths, max_bif_incoming*numucat, MPI_INTEGER, &
               p_address_master, mpi_tag_data, p_comm_glb, p_stat, p_err)
            CALL mpi_recv (bif_incoming_wts,  max_bif_incoming*numucat, MPI_REAL8, &
               p_address_master, mpi_tag_data, p_comm_glb, p_stat, p_err)
         ELSE
            allocate (bif_incoming_pths (max_bif_incoming, 0))
            allocate (bif_incoming_wts  (max_bif_incoming, 0))
         ENDIF

      ENDIF

      CALL mpi_barrier (p_comm_glb, p_err)

      ! IO processes did not enter master or worker branches above.
      ! Allocate zero-length arrays so build_worker_pushdata (MPI collective) is safe.
      IF (p_is_io) THEN
         npthout_local = 0
         allocate (pth_upst_local (0))
         allocate (pth_down_ucid  (0))
         allocate (pth_down_local (0))
         allocate (pth_global_id  (0))
         allocate (pth_dst        (0))
         allocate (pth_elv        (npthlev_bif, 0))
         allocate (pth_wth        (npthlev_bif, 0))
         allocate (bif_incoming_pths (max_bif_incoming, 0))
         allocate (bif_incoming_wts  (max_bif_incoming, 0))
      ENDIF

#else
      ! ================================================================
      ! Serial (non-MPI) path
      ! ================================================================

      CALL read_bifurcation_global_arrays (parafile, bif_upst_all, bif_down_all, &
         bif_dist_all, bif_elev_all, bif_wdth_all, bif_mann_all)

      npthout_local = totalnpthout

      allocate (pth_man (npthlev_bif))
      pth_man(:) = bif_mann_all(:)
      deallocate (bif_mann_all)

      IF (npthout_local > 0) THEN
         allocate (pth_upst_local (npthout_local))
         allocate (pth_down_ucid  (npthout_local))
         allocate (pth_down_local (npthout_local))
         allocate (pth_global_id  (npthout_local))
         allocate (pth_dst        (npthout_local))
         allocate (pth_elv        (npthlev_bif, npthout_local))
         allocate (pth_wth        (npthlev_bif, npthout_local))

         pth_down_ucid(:) = bif_down_all(:)
         pth_dst(:)       = bif_dist_all(:)
         pth_elv(:,:)     = bif_elev_all(:,:)
         pth_wth(:,:)     = bif_wdth_all(:,:)

         DO ip = 1, npthout_local
            pth_global_id(ip) = ip
         ENDDO

         pth_upst_local(:) = bif_upst_all(:)
         CALL localize_bifurcation_path_indices (pth_upst_local, pth_down_ucid, pth_down_local)
      ELSE
         npthout_local = 0
         allocate (pth_upst_local (0))
         allocate (pth_down_ucid  (0))
         allocate (pth_down_local (0))
         allocate (pth_global_id  (0))
         allocate (pth_dst        (0))
         allocate (pth_elv        (npthlev_bif, 0))
         allocate (pth_wth        (npthlev_bif, 0))
      ENDIF

      deallocate (bif_upst_all)
      deallocate (bif_down_all)
      deallocate (bif_dist_all)
      deallocate (bif_elev_all)
      deallocate (bif_wdth_all)

      ! Build reverse mapping
      allocate (bif_inc_cnt (totalnumucat))
      bif_inc_cnt(:) = 0
      DO ip = 1, totalnpthout
         j = pth_down_ucid(ip)
         IF (j > 0 .and. j <= totalnumucat) THEN
            bif_inc_cnt(j) = bif_inc_cnt(j) + 1
         ENDIF
      ENDDO
      max_bif_incoming = maxval(bif_inc_cnt)
      IF (max_bif_incoming < 1) max_bif_incoming = 1

      allocate (bif_incoming_pths (max_bif_incoming, numucat))
      allocate (bif_incoming_wts  (max_bif_incoming, numucat))
      bif_incoming_pths(:,:) = 0
      bif_incoming_wts(:,:)  = 0.

      bif_inc_cnt(:) = 0
      DO ip = 1, totalnpthout
         j = pth_down_ucid(ip)
         IF (j > 0 .and. j <= totalnumucat) THEN
            bif_inc_cnt(j) = bif_inc_cnt(j) + 1
            ! Find local index of ucat j
            DO i = 1, numucat
               IF (ucat_ucid(i) == j) THEN
                  bif_incoming_pths(bif_inc_cnt(j), i) = ip
                  bif_incoming_wts (bif_inc_cnt(j), i) = 1.
                  EXIT
               ENDIF
            ENDDO
         ENDIF
      ENDDO

      deallocate (bif_inc_cnt)

#endif

      ! ================================================================
      ! Build push objects for bifurcation (both MPI and serial)
      ! ================================================================

      ! push_bif_dn2pth: single-source push from ucats to pathways
      !   Each pathway needs the state of its downstream ucat
      CALL build_worker_pushdata (numucat, ucat_ucid, npthout_local, pth_down_ucid, push_bif_dn2pth)

      ! push_bif_influx: multi-source push from pathways to ucats
      !   Each ucat may receive flux from multiple pathways
      !   Uses global pathway IDs as source IDs
      CALL build_worker_pushdata (npthout_local, pth_global_id, numucat, &
         bif_incoming_pths, bif_incoming_wts, push_bif_influx)

   END SUBROUTINE read_and_distribute_bifurcation

   ! ---------
   SUBROUTINE read_bifurcation_global_arrays (parafile, bif_upst_all, bif_down_all, &
      bif_dist_all, bif_elev_all, bif_wdth_all, bif_mann_all)

   USE MOD_NetCDFSerial
   USE MOD_Utils
   IMPLICIT NONE

   character(len=*), intent(in) :: parafile
   integer,  allocatable, intent(out) :: bif_upst_all  (:)
   integer,  allocatable, intent(out) :: bif_down_all  (:)
   real(r8), allocatable, intent(out) :: bif_dist_all  (:)
   real(r8), allocatable, intent(out) :: bif_elev_all  (:,:)
   real(r8), allocatable, intent(out) :: bif_wdth_all  (:,:)
   real(r8), allocatable, intent(out) :: bif_mann_all  (:)
   integer :: ip, ilev

      CALL ncio_inquire_length (parafile, 'bifurcation_upst',    totalnpthout)
      CALL ncio_inquire_length (parafile, 'bifurcation_manning', npthlev_bif)

      CALL ncio_read_serial (parafile, 'bifurcation_upst',      bif_upst_all)
      CALL ncio_read_serial (parafile, 'bifurcation_down',      bif_down_all)
      CALL ncio_read_serial (parafile, 'bifurcation_distance',  bif_dist_all)
      CALL ncio_read_serial (parafile, 'bifurcation_elevation', bif_elev_all)
      CALL ncio_read_serial (parafile, 'bifurcation_width',     bif_wdth_all)
      CALL ncio_read_serial (parafile, 'bifurcation_manning',   bif_mann_all)

      IF (size(bif_down_all) /= totalnpthout .or. &
          size(bif_dist_all) /= totalnpthout .or. &
          size(bif_elev_all, 1) /= npthlev_bif .or. &
          size(bif_elev_all, 2) /= totalnpthout .or. &
          size(bif_wdth_all, 1) /= npthlev_bif .or. &
          size(bif_wdth_all, 2) /= totalnpthout .or. &
          size(bif_mann_all) /= npthlev_bif) THEN
         CALL CoLM_stop ('bifurcation parameter dimensions are inconsistent')
      ENDIF

      IF (any(.not. ieee_is_finite(bif_dist_all))) THEN
         CALL CoLM_stop ('bifurcation distance must be finite and positive')
      ELSEIF (any(bif_dist_all <= 0._r8)) THEN
         CALL CoLM_stop ('bifurcation distance must be finite and positive')
      ENDIF
      IF (any(.not. ieee_is_finite(bif_elev_all))) THEN
         CALL CoLM_stop ('bifurcation elevation contains a non-finite value')
      ENDIF
      IF (any(.not. ieee_is_finite(bif_wdth_all))) THEN
         CALL CoLM_stop ('bifurcation width contains a non-finite value')
      ELSEIF (any(bif_wdth_all < 0._r8)) THEN
         CALL CoLM_stop ('bifurcation width must be non-negative')
      ENDIF
      IF (any(.not. ieee_is_finite(bif_mann_all))) THEN
         CALL CoLM_stop ('bifurcation Manning coefficient contains a non-finite value')
      ENDIF

      DO ilev = 1, npthlev_bif
         IF (any(bif_wdth_all(ilev, :) > 0._r8) .and. bif_mann_all(ilev) <= 0._r8) THEN
            CALL CoLM_stop ('active bifurcation layer requires a positive Manning coefficient')
         ENDIF
      ENDDO

      DO ip = 1, totalnpthout
         IF (bif_upst_all(ip) < 1 .or. bif_upst_all(ip) > totalnumucat) CALL CoLM_stop ( &
            'bifurcation upstream index out of range')
         IF (bif_down_all(ip) > totalnumucat) CALL CoLM_stop ('bifurcation downstream index out of range')
         IF (bif_down_all(ip) == bif_upst_all(ip)) CALL CoLM_stop ('bifurcation self-loop pathway is invalid')
         IF (.not. any(bif_wdth_all(:, ip) > 0._r8)) THEN
            CALL CoLM_stop ('bifurcation pathway has no active positive-width layer')
         ENDIF
         DO ilev = 2, npthlev_bif
            IF (bif_wdth_all(ilev, ip) > 0._r8) THEN
               IF (bif_wdth_all(ilev-1, ip) <= 0._r8) THEN
                  CALL CoLM_stop ('active bifurcation layers must be contiguous from layer 1')
               ENDIF
               IF (bif_elev_all(ilev, ip) < bif_elev_all(ilev-1, ip)) THEN
                  CALL CoLM_stop ( &
                     'active bifurcation layer elevation must be non-decreasing from layer 1')
               ENDIF
            ENDIF
         ENDDO
      ENDDO

   END SUBROUTINE read_bifurcation_global_arrays

   ! ---------
   SUBROUTINE localize_bifurcation_path_indices (path_upst, path_down_ucid, path_down_local)

   IMPLICIT NONE

   integer, intent(inout) :: path_upst(:)
   integer, intent(in)    :: path_down_ucid(:)
   integer, intent(out)   :: path_down_local(:)
   integer :: ip, i

      DO ip = 1, size(path_upst)
         DO i = 1, numucat
            IF (ucat_ucid(i) == path_upst(ip)) THEN
               path_upst(ip) = i
               EXIT
            ENDIF
         ENDDO
      ENDDO

      path_down_local = -1
      DO ip = 1, size(path_down_ucid)
         DO i = 1, numucat
            IF (ucat_ucid(i) == path_down_ucid(ip)) THEN
               path_down_local(ip) = i
               EXIT
            ENDIF
         ENDDO
      ENDDO

   END SUBROUTINE localize_bifurcation_path_indices

END MODULE MOD_Grid_RiverLakeNetwork
#endif
