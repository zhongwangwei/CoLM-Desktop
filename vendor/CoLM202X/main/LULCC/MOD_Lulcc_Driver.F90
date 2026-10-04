#include <define.h>

MODULE MOD_Lulcc_Driver

   USE MOD_Precision
   USE MOD_Mesh, only: mesh
   USE MOD_Pixel, only: pixel
   USE MOD_Pixelset, only: pixelset_type
   USE MOD_Utils, only: areaquad, quicksort
   USE MOD_Const_LC, only: patchtypes
   USE MOD_Vars_Global, only: N_land_classification, CROPLAND
   USE MOD_Namelist, only: DEF_USE_PFT, DEF_SOLO_PFT, DEF_FAST_PC
   USE, INTRINSIC :: ieee_arithmetic, only: ieee_is_finite
   IMPLICIT NONE
   SAVE

! PUBLIC MEMBER FUNCTIONS:
   PUBLIC :: LulccDriver

 CONTAINS


   SUBROUTINE LulccDriver (casename, dir_landdata, dir_restart, &
                           jdate, greenwich)

!-----------------------------------------------------------------------
!
! !DESCRIPTION:
!  the main subroutine for Land use and land cover change simulation
!
!  Created by Hua Yuan, 04/08/2022
!
! !REVISIONS:
!  07/2023, Wenzong Dong: porting to MPI version.
!  08/2023, Wanyi Lin: add interface for Mass&Energy conserved scheme.
!
!-----------------------------------------------------------------------
!
!                  ***** For Development *****
!
!  Extra processes when adding a new variable and #define LULCC:
!
!  1. Save a copy of new variable (if called "var", save it to "var_")
!  with 2 steps:
!
!  1.1 main/LULCC/MOD_Lulcc_Vars_TimeVariables.F90's subroutine
!  "allocate_LulccTimeVariables":
!           allocate (var_(dimension))
!
!  1.2 main/LULCC/MOD_Lulcc_Vars_TimeVariables.F90's subroutine
!  "SAVE_LulccTimeVariables":
!           var_ = var
!
!  2. Reassignment for the next year
!
!  2.1 if used Same Type Assignment (SAT) scheme for variable recovery
!  main/LULCC/MOD_Lulcc_Vars_TimeVariables.F90's subroutine
!  "REST_LulccTimeVariables"
!           var(np) = var_(np_)
!
!  2.2 if using Mass and Energy conservation (MEC) scheme for variable
!  recovery
!
!  2.2.1 main/LULCC/MOD_Lulcc_Vars_TimeVariables.F90's subroutine
!  "REST_LulccTimeVariables":
!           var(np) = var_(np_)
!
!  2.2.2 [No need for PFT/PC scheme] Mass and Energy conserve
!  adjustment, add after line 519 of MOD_Lulcc_MassEnergyConserve.F90.
!
!  o if variable should be mass conserved:
!       var(:,np) = var(:,np) + &
!       var(:,frnp_(k))*lccpct_np(patchclass_(frnp_(k)))/sum_lccpct_np
!
!  o if variable should be energy conserved, take soil temperature
!  "t_soisno" as an example: [May neeed extra calculation]
!       t_soisno (1:nl_soil,np) = t_soisno (1:nl_soil,np) + &
!                t_soisno_(1:nl_soil,frnp_(k))*cvsoil_(1:nl_soil,k)* &
!                lccpct_np(patchclass_(frnp_(k)))/wgt(1:nl_soil)
!  where cvsoil_ is the heat capacity, wgt is the sum of
!  cvsoil_(1:nl_soil,k)*lccpct_np(patchclass_(frnp_(k))), which need to
!  be calculated in advance.
!
!  3. Deallocate the copy of new variable in
!  MOD_Lulcc_Vars_TimeVariables.F90's subroutine
!  "deallocate_LulccTimeVariables":
!           deallocate (var_)
!
!-----------------------------------------------------------------------

   USE MOD_Precision
   USE MOD_SPMD_Task
   USE MOD_Lulcc_Vars_TimeInvariants
   USE MOD_Lulcc_Vars_TimeVariables
   USE MOD_Lulcc_Initialize
   USE MOD_Vars_TimeVariables
   USE MOD_Vars_TimeInvariants, only: patchclass
   USE MOD_LandPatch, only: landpatch, numpatch
   USE MOD_Lulcc_TransferTraceReadin
   USE MOD_Lulcc_MassEnergyConserve
   USE MOD_Namelist
   USE MOD_Opt_Baseflow, only: zwt_init
   USE MOD_Vars_Global, only: spval
   USE MOD_Tracer_Defs, only: ntracers
   USE MOD_Tracer_Lifecycle, only: tracer_lifecycle_land_save_lulcc_state, &
      tracer_lifecycle_land_remap_lulcc_state, tracer_lifecycle_land_reload_lulcc_inputs
   USE MOD_Tracer_Vars, only: save_land_tracer_lulcc_state, &
      remap_land_tracer_lulcc_state
   USE MOD_Tracer_Conservation, only: deallocate_tracer_conservation
   USE MOD_Tracer_Forcing, only: tracer_forcing_lulcc_remap

   IMPLICIT NONE

   character(len=256), intent(in) :: casename      !casename name
   character(len=256), intent(in) :: dir_landdata  !surface data directory
   character(len=256), intent(in) :: dir_restart   !case restart data directory

   logical, intent(in)    :: greenwich   !true: greenwich time, false: local time
   integer, intent(inout) :: jdate(3)    !year, julian day, seconds of the starting time
   real(r8), allocatable :: old_patch_area(:), new_patch_area(:), inventory_trace(:,:)
   logical :: inventory_active
!-----------------------------------------------------------------------

      ! allocate Lulcc memory
      CALL allocate_LulccTimeInvariants
      CALL allocate_LulccTimeVariables

      ! SAVE variables
      CALL SAVE_LulccTimeInvariants
      CALL SAVE_LulccTimeVariables
      IF (DEF_USE_TRACER) THEN
      CALL save_land_tracer_lulcc_state ()
      CALL tracer_lifecycle_land_save_lulcc_state ()
      inventory_active = p_is_worker .and. ntracers > 0 .and. DEF_LULCC_SCHEME == 2
      IF (inventory_active) THEN
         IF (numpatch > 0) THEN
            IF (.not. allocated(landpatch%eindex)) &
               CALL CoLM_stop('TRACER LULCC old patch map unavailable')
            IF (landpatch%nset /= numpatch) &
               CALL CoLM_stop('TRACER LULCC old patch count mismatch')
            CALL lulcc_patch_areas(landpatch, old_patch_area)
         ELSE
            allocate(old_patch_area(0))
         ENDIF
      ENDIF
      ENDIF

      ! =============================================================
      ! cold start for Lulcc
      ! =============================================================

      IF (p_is_master) THEN
         print *, ">>> LULCC: initializing..."
      ENDIF

      CALL LulccInitialize (casename, dir_landdata, dir_restart, &
                            jdate, greenwich)
      IF (DEF_USE_TRACER) THEN
      IF (inventory_active) THEN
         IF ((size(old_patch_area) > 0) .neqv. (numpatch > 0)) &
            CALL CoLM_stop('TRACER LULCC worker element footprint changed')
         IF (size(old_patch_area) > 0) THEN
            IF (.not. allocated(patchclass_) .or. .not. allocated(landpatch_%eindex)) &
               CALL CoLM_stop('TRACER LULCC old patch map unavailable')
         ENDIF
      ENDIF
      ENDIF


      ! =============================================================
      ! 1. Same Type Assignment (SAT) scheme for variable recovery
      ! =============================================================

      IF (DEF_LULCC_SCHEME == 1) THEN
         IF (p_is_master) THEN
            print *, ">>> LULCC: Same Type Assignment (SAT) scheme for variable recovery..."
         ENDIF
         CALL REST_LulccTimeVariables
      ENDIF


      ! =============================================================
      ! 2. Mass and Energy conservation (MEC) scheme for variable recovery
      ! =============================================================

      IF (DEF_LULCC_SCHEME == 2) THEN
         IF (p_is_master) THEN
            print *, ">>> LULCC: Mass&Energy conserve (MEC) for variable recovery..."
         ENDIF
         CALL allocate_LulccTransferTrace()
         CALL REST_LulccTimeVariables
         CALL LulccTransferTraceReadin(jdate(1))
      IF (DEF_USE_TRACER) THEN
         IF (inventory_active) THEN
            IF (.not. allocated(lccpct_patches)) &
               CALL CoLM_stop('TRACER LULCC MEC requires the transfer trace')
            IF (numpatch > 0) THEN
               IF (.not. allocated(landpatch%eindex)) &
                  CALL CoLM_stop('TRACER LULCC new patch map unavailable')
               IF (landpatch%nset /= numpatch) &
                  CALL CoLM_stop('TRACER LULCC new patch count mismatch')
               CALL lulcc_patch_areas(landpatch, new_patch_area)
            ELSE
               allocate(new_patch_area(0))
            ENDIF
            IF (allocated(patchclass_)) THEN
               IF (.not. allocated(patchclass)) &
                  CALL CoLM_stop('TRACER LULCC MEC patch class map unavailable')
            ELSE
               IF (size(old_patch_area) > 0 .or. size(new_patch_area) > 0) &
                  CALL CoLM_stop('TRACER LULCC MEC patch class map unavailable')
            ENDIF
            CALL lulcc_inventory_trace(lccpct_patches, inventory_trace)
            IF (size(old_patch_area) > 0 .or. size(new_patch_area) > 0) &
               CALL lulcc_check_inventory_transfer(patchclass_, landpatch_%eindex, old_patch_area, &
                  patchclass, landpatch%eindex, new_patch_area, inventory_trace)
         ENDIF
      ENDIF
         CALL LulccMassEnergyConserve()
      ENDIF

      ! new patches of the baseflow optimization start from the new year's water table
      IF (p_is_worker .and. allocated(zwt_init)) THEN
         WHERE (zwt_init == spval) zwt_init = zwt
      ENDIF

      IF (DEF_USE_TRACER) THEN
      IF (p_is_worker .and. allocated(patchclass)) THEN
         IF (size(patchclass) > 0 .and. .not. allocated(patchclass_)) &
            CALL CoLM_stop('TRACER LULCC cannot remap a worker from zero to nonzero patches')
      ENDIF
      IF (p_is_worker .and. allocated(patchclass) .and. allocated(patchclass_) .and. &
          allocated(landpatch%eindex) .and. allocated(landpatch_%eindex)) THEN
         CALL deallocate_tracer_conservation ()
         IF (inventory_active) THEN
            CALL remap_land_tracer_lulcc_state (patchclass, landpatch%eindex, &
               patchclass_, landpatch_%eindex, inventory_trace, new_patch_area, old_patch_area)
            CALL tracer_lifecycle_land_remap_lulcc_state (patchclass, landpatch%eindex, &
               patchclass_, landpatch_%eindex, inventory_trace, new_patch_area, old_patch_area)
            CALL tracer_forcing_lulcc_remap (patchclass, landpatch%eindex, &
               patchclass_, landpatch_%eindex, lccpct_patches)
         ELSE
            CALL remap_land_tracer_lulcc_state (patchclass, landpatch%eindex, &
               patchclass_, landpatch_%eindex)
            CALL tracer_lifecycle_land_remap_lulcc_state (patchclass, landpatch%eindex, &
               patchclass_, landpatch_%eindex)
            CALL tracer_forcing_lulcc_remap (patchclass, landpatch%eindex, &
               patchclass_, landpatch_%eindex)
         ENDIF
      ENDIF
      IF (allocated(old_patch_area)) deallocate(old_patch_area, new_patch_area, inventory_trace)
      CALL tracer_lifecycle_land_reload_lulcc_inputs (jdate(1), dir_landdata)
      ENDIF


      ! deallocate Lulcc memory
      CALL deallocate_LulccTimeInvariants()
      CALL deallocate_LulccTimeVariables()
      IF (DEF_LULCC_SCHEME == 2) THEN
         CALL deallocate_LulccTransferTrace()
      ENDIF

   END SUBROUTINE LulccDriver

   SUBROUTINE lulcc_patch_areas(patches, areas)
      USE MOD_SPMD_Task, only: CoLM_stop
      TYPE(pixelset_type), intent(in) :: patches
      real(r8), allocatable, intent(out) :: areas(:)
      integer :: ip, ie, ix, first, last

      allocate(areas(patches%nset))
      areas = 0._r8
      IF (patches%nset <= 0) RETURN
      IF (.not. allocated(mesh) .or. .not. allocated(patches%ielm) .or. &
          .not. allocated(patches%ipxstt) .or. .not. allocated(patches%ipxend) .or. &
          .not. allocated(pixel%lat_s) .or. .not. allocated(pixel%lat_n) .or. &
          .not. allocated(pixel%lon_w) .or. .not. allocated(pixel%lon_e)) &
         CALL CoLM_stop('TRACER LULCC patch geometry unavailable')
      IF (size(patches%ielm) /= patches%nset .or. size(patches%ipxstt) /= patches%nset .or. &
          size(patches%ipxend) /= patches%nset) &
         CALL CoLM_stop('TRACER LULCC patch geometry shape mismatch')
      IF (patches%has_shared) THEN
         IF (.not. allocated(patches%pctshared)) &
            CALL CoLM_stop('TRACER LULCC missing shared patch fractions')
         IF (size(patches%pctshared) /= patches%nset) &
            CALL CoLM_stop('TRACER LULCC shared patch area shape mismatch')
      ENDIF
      DO ip = 1, patches%nset
         ie = patches%ielm(ip)
         IF (ie < 1 .or. ie > size(mesh)) CALL CoLM_stop('TRACER LULCC invalid patch element')
         IF (.not. allocated(mesh(ie)%ilat) .or. .not. allocated(mesh(ie)%ilon)) &
            CALL CoLM_stop('TRACER LULCC element pixels unavailable')
         IF (size(mesh(ie)%ilat) < mesh(ie)%npxl .or. size(mesh(ie)%ilon) < mesh(ie)%npxl) &
            CALL CoLM_stop('TRACER LULCC element pixel shape mismatch')
         first = patches%ipxstt(ip)
         last = patches%ipxend(ip)
         IF (first == -1 .and. last == -1) THEN
            CYCLE
         ENDIF
         IF (first < 1 .or. last > mesh(ie)%npxl .or. last < first) &
            CALL CoLM_stop('TRACER LULCC invalid patch pixel range')
         DO ix = first, last
            IF (mesh(ie)%ilat(ix) < 1 .or. mesh(ie)%ilat(ix) > size(pixel%lat_s) .or. &
                mesh(ie)%ilat(ix) > size(pixel%lat_n) .or. mesh(ie)%ilon(ix) < 1 .or. &
                mesh(ie)%ilon(ix) > size(pixel%lon_w) .or. &
                mesh(ie)%ilon(ix) > size(pixel%lon_e)) &
               CALL CoLM_stop('TRACER LULCC invalid pixel coordinate index')
            areas(ip) = areas(ip) + 1.e6_r8 * areaquad( &
               pixel%lat_s(mesh(ie)%ilat(ix)), pixel%lat_n(mesh(ie)%ilat(ix)), &
               pixel%lon_w(mesh(ie)%ilon(ix)), pixel%lon_e(mesh(ie)%ilon(ix)))
         ENDDO
         IF (patches%has_shared) THEN
            IF (.not. ieee_is_finite(patches%pctshared(ip))) &
               CALL CoLM_stop('TRACER LULCC invalid shared patch fraction')
            IF (patches%pctshared(ip) < 0._r8 .or. patches%pctshared(ip) > 1._r8) &
               CALL CoLM_stop('TRACER LULCC invalid shared patch fraction')
            areas(ip) = areas(ip) * patches%pctshared(ip)
         ENDIF
         IF (.not. ieee_is_finite(areas(ip))) &
            CALL CoLM_stop('TRACER LULCC invalid physical patch area')
         IF (areas(ip) < 0._r8) &
            CALL CoLM_stop('TRACER LULCC invalid physical patch area')
      ENDDO
   END SUBROUTINE lulcc_patch_areas

   SUBROUTINE lulcc_inventory_trace(raw, mapped)
      USE MOD_SPMD_Task, only: CoLM_stop
      real(r8), intent(in) :: raw(:,0:)
      real(r8), allocatable, intent(out) :: mapped(:,:)
      integer :: c, dest

      IF (ubound(raw,2) /= N_land_classification) &
         CALL CoLM_stop('TRACER LULCC transfer trace class count mismatch')
      IF (any(.not. ieee_is_finite(raw))) &
         CALL CoLM_stop('TRACER LULCC invalid transfer trace')
      IF (any(raw < 0._r8)) &
         CALL CoLM_stop('TRACER LULCC invalid transfer trace')
      allocate(mapped(size(raw,1),0:N_land_classification))
      mapped = raw
      IF (.not. ((DEF_USE_PFT .and. .not. DEF_SOLO_PFT) .or. DEF_FAST_PC)) RETURN
      mapped = 0._r8
      mapped(:,0) = raw(:,0)
      DO c = 1, N_land_classification
         dest = c
         IF (patchtypes(c) == 0) dest = 1
         IF (DEF_FAST_PC .and. (c == CROPLAND .or. c == 14)) dest = CROPLAND
         mapped(:,dest) = mapped(:,dest) + raw(:,c)
      ENDDO
   END SUBROUTINE lulcc_inventory_trace

   SUBROUTINE lulcc_check_inventory_transfer(old_class, old_element, old_area, &
      new_class, new_element, new_area, trace)
      USE MOD_SPMD_Task, only: CoLM_stop
      integer, intent(in) :: old_class(:), new_class(:)
      integer*8, intent(in) :: old_element(:), new_element(:)
      real(r8), intent(in) :: old_area(:), new_area(:), trace(:,0:)
      real(r8) :: source(0:N_land_classification), target(0:N_land_classification)
      real(r8) :: old_total, new_total, tol, rowsum
      integer*8, allocatable :: old_sorted(:), new_sorted(:)
      integer, allocatable :: old_order(:), new_order(:)
      integer :: os, oe, ns, ne, op, np, c, k

      IF (size(old_class) /= size(old_area) .or. size(old_class) /= size(old_element) .or. &
          size(new_class) /= size(new_area) .or. size(new_class) /= size(new_element) .or. &
          size(trace,1) /= size(new_class) .or. ubound(trace,2) /= N_land_classification) &
         CALL CoLM_stop('TRACER LULCC inventory map shape mismatch')
      IF (any(.not. ieee_is_finite(old_area)) .or. any(.not. ieee_is_finite(new_area)) .or. &
          any(.not. ieee_is_finite(trace))) &
         CALL CoLM_stop('TRACER LULCC invalid inventory area or transfer trace')
      IF (any(old_area < 0._r8) .or. any(new_area < 0._r8) .or. any(trace < 0._r8)) &
         CALL CoLM_stop('TRACER LULCC invalid inventory area or transfer trace')
      old_sorted = old_element
      new_sorted = new_element
      old_order = [(k, k=1,size(old_class))]
      new_order = [(k, k=1,size(new_class))]
      CALL quicksort(size(old_class), old_sorted, old_order)
      CALL quicksort(size(new_class), new_sorted, new_order)
      os = 1
      ns = 1
      DO WHILE (os <= size(old_class) .or. ns <= size(new_class))
         IF (os > size(old_class) .or. ns > size(new_class)) &
            CALL CoLM_stop('TRACER LULCC element footprint changed')
         IF (old_sorted(os) /= new_sorted(ns)) &
            CALL CoLM_stop('TRACER LULCC element footprint changed')
         oe = os
         DO WHILE (oe < size(old_class))
            IF (old_sorted(oe+1) /= old_sorted(os)) EXIT
            oe = oe + 1
         ENDDO
         ne = ns
         DO WHILE (ne < size(new_class))
            IF (new_sorted(ne+1) /= new_sorted(ns)) EXIT
            ne = ne + 1
         ENDDO
         source = 0._r8
         target = 0._r8
         DO k = os, oe
            op = old_order(k)
            IF (old_area(op) <= tiny(1._r8)) CYCLE
            c = old_class(op)
            IF (c < 0 .or. c > N_land_classification) &
               CALL CoLM_stop('TRACER LULCC invalid old patch class')
            source(c) = source(c) + old_area(op)
         ENDDO
         DO k = ns, ne
            np = new_order(k)
            IF (new_area(np) <= tiny(1._r8)) CYCLE
            c = new_class(np)
            IF (c < 0 .or. c > N_land_classification) &
               CALL CoLM_stop('TRACER LULCC invalid new patch class')
            rowsum = sum(trace(np,:))
            IF (abs(rowsum - 1._r8) > 1.e-10_r8) &
               CALL CoLM_stop('TRACER LULCC incomplete source trace')
            target = target + new_area(np) * trace(np,:) / rowsum
         ENDDO
         old_total = sum(source)
         new_total = sum(new_area(new_order(ns:ne)))
         tol = 1.e-10_r8 * max(old_total, new_total, 1._r8)
         IF (abs(old_total - new_total) > tol) THEN
            WRITE(*,'(A,I0,2(A,ES16.8))') 'TRACER LULCC element=', old_sorted(os), &
               ' old_area_m2=', old_total, ' new_area_m2=', new_total
            CALL CoLM_stop('TRACER LULCC element physical footprint changed')
         ENDIF
         DO c = 0, N_land_classification
            IF (((source(c) > 0._r8) .neqv. (target(c) > 0._r8)) .or. &
                abs(source(c) - target(c)) > 1.e-10_r8 * max(source(c), target(c))) THEN
               WRITE(*,'(A,I0,A,I0,2(A,ES16.8))') 'TRACER LULCC element=', old_sorted(os), &
                  ' class=', c, ' source_area_m2=', source(c), ' target_area_m2=', target(c)
               CALL CoLM_stop('TRACER LULCC source-class physical area mismatch')
            ENDIF
         ENDDO
         os = oe + 1
         ns = ne + 1
      ENDDO
   END SUBROUTINE lulcc_check_inventory_transfer

END MODULE MOD_Lulcc_Driver
! ---------- EOP ------------
