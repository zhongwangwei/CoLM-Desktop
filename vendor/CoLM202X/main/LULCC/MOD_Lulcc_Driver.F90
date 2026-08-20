#include <define.h>

#ifdef LULCC
MODULE MOD_Lulcc_Driver

   USE MOD_Precision
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
   USE MOD_LandPatch, only: landpatch
   USE MOD_Lulcc_TransferTraceReadin
   USE MOD_Lulcc_MassEnergyConserve
   USE MOD_Namelist
#ifdef TRACER
   USE MOD_Tracer_Lifecycle, only: tracer_lifecycle_land_save_lulcc_state, &
      tracer_lifecycle_land_remap_lulcc_state, tracer_lifecycle_land_reload_lulcc_inputs
   USE MOD_Tracer_Vars, only: save_land_tracer_lulcc_state, &
      remap_land_tracer_lulcc_state
   USE MOD_Tracer_Conservation, only: deallocate_tracer_conservation
#endif

   IMPLICIT NONE

   character(len=256), intent(in) :: casename      !casename name
   character(len=256), intent(in) :: dir_landdata  !surface data directory
   character(len=256), intent(in) :: dir_restart   !case restart data directory

   logical, intent(in)    :: greenwich   !true: greenwich time, false: local time
   integer, intent(inout) :: jdate(3)    !year, julian day, seconds of the starting time
#ifdef TRACER
   logical :: have_patch_area
#endif
!-----------------------------------------------------------------------

      ! allocate Lulcc memory
      CALL allocate_LulccTimeInvariants
      CALL allocate_LulccTimeVariables

      ! SAVE variables
      CALL SAVE_LulccTimeInvariants
      CALL SAVE_LulccTimeVariables
#ifdef TRACER
      CALL save_land_tracer_lulcc_state ()
      CALL tracer_lifecycle_land_save_lulcc_state ()
#endif

      ! =============================================================
      ! cold start for Lulcc
      ! =============================================================

      IF (p_is_master) THEN
         print *, ">>> LULCC: initializing..."
      ENDIF

      CALL LulccInitialize (casename, dir_landdata, dir_restart, &
                            jdate, greenwich)


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
         CALL LulccMassEnergyConserve()
      ENDIF

#ifdef TRACER
      IF (p_is_worker .and. allocated(patchclass) .and. size(patchclass) > 0 .and. &
          .not. allocated(patchclass_)) THEN
         CALL CoLM_stop('TRACER LULCC cannot remap a worker from zero to nonzero patches')
      ENDIF
      IF (p_is_worker .and. allocated(patchclass) .and. allocated(patchclass_) .and. &
          allocated(landpatch%eindex) .and. allocated(landpatch_%eindex)) THEN
         CALL deallocate_tracer_conservation ()
         have_patch_area = allocated(landpatch%pctshared) .and. allocated(landpatch_%pctshared)
         IF (allocated(lccpct_patches)) THEN
            IF (have_patch_area) THEN
               CALL remap_land_tracer_lulcc_state (patchclass, landpatch%eindex, &
                  patchclass_, landpatch_%eindex, lccpct_patches, &
                  landpatch%pctshared, landpatch_%pctshared)
               CALL tracer_lifecycle_land_remap_lulcc_state (patchclass, landpatch%eindex, &
                  patchclass_, landpatch_%eindex, lccpct_patches, &
                  landpatch%pctshared, landpatch_%pctshared)
            ELSE
               CALL remap_land_tracer_lulcc_state (patchclass, landpatch%eindex, &
                  patchclass_, landpatch_%eindex, lccpct_patches)
               CALL tracer_lifecycle_land_remap_lulcc_state (patchclass, landpatch%eindex, &
                  patchclass_, landpatch_%eindex, lccpct_patches)
            ENDIF
         ELSE
            CALL remap_land_tracer_lulcc_state (patchclass, landpatch%eindex, &
               patchclass_, landpatch_%eindex)
            CALL tracer_lifecycle_land_remap_lulcc_state (patchclass, landpatch%eindex, &
               patchclass_, landpatch_%eindex)
         ENDIF
      ENDIF
      ! GIEMS broadcasts and spatial-pH vector I/O use global collectives;
      ! reload them after the worker-local state remap with every rank present.
      ! Pass the current LULCC year and landdata root so all spatial inputs are
      ! rebuilt from the same year-specific patch map.
      CALL tracer_lifecycle_land_reload_lulcc_inputs (jdate(1), dir_landdata)
#endif


      ! deallocate Lulcc memory
      CALL deallocate_LulccTimeInvariants()
      CALL deallocate_LulccTimeVariables()
      IF (DEF_LULCC_SCHEME == 2) THEN
         CALL deallocate_LulccTransferTrace()
      ENDIF

   END SUBROUTINE LulccDriver

END MODULE MOD_Lulcc_Driver
#endif
! ---------- EOP ------------
