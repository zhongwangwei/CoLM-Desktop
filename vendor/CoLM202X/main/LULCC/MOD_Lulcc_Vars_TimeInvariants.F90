#include <define.h>

MODULE MOD_Lulcc_Vars_TimeInvariants

!-----------------------------------------------------------------------
!  Created by Hua Yuan, 04/2022
!
! !REVISIONS:
!
!  07/2023, Wenzong Dong: porting to MPI version
!  08/2023, Hua Yuan: unified PFT and PC process
!
!-----------------------------------------------------------------------

   USE MOD_Precision
   USE MOD_Vars_Global
   USE MOD_PixelSet

   IMPLICIT NONE
   SAVE
!-----------------------------------------------------------------------
   ! for patch time invariant information
   type(pixelset_type)  :: landpatch_
   type(pixelset_type)  :: landelm_
   integer              :: numpatch_
   integer              :: numelm_
   integer              :: numpft_
   integer              :: numpc_
   integer              :: numurban_
   integer, allocatable :: patchclass_    (:)  !index of land cover type
   integer, allocatable :: patchtype_     (:)  !land patch type
   real(r8), allocatable:: csol_        (:,:)  !heat capacity of soil solids [J/(m3 K)]

   ! for LULC_IGBP_PFT and LULC_IGBP_PC
   integer, allocatable :: pftclass_      (:)  !PFT type
   integer, allocatable :: patch_pft_s_   (:)  !start PFT index of a patch
   integer, allocatable :: patch_pft_e_   (:)  !end PFT index of a patch

   ! for Urban model
   integer, allocatable :: urbclass_      (:)  !urban type
   integer, allocatable :: patch2urban_   (:)  !projection from patch to Urban

! PUBLIC MEMBER FUNCTIONS:
   PUBLIC :: allocate_LulccTimeInvariants
   PUBLIC :: deallocate_LulccTimeInvariants
   PUBLIC :: SAVE_LulccTimeInvariants

! PRIVATE MEMBER FUNCTIONS:

!-----------------------------------------------------------------------

CONTAINS

!-----------------------------------------------------------------------

   SUBROUTINE allocate_LulccTimeInvariants
   ! --------------------------------------------------------------------
   ! Allocates memory for Lulcc time invariant variables
   ! --------------------------------------------------------------------

   USE MOD_Namelist
   USE MOD_SPMD_Task
   USE MOD_Precision
   USE MOD_Vars_Global
   USE MOD_LandPatch
   USE MOD_Mesh
   USE MOD_LandPFT
   USE MOD_LandUrban

   IMPLICIT NONE

      IF (p_is_worker) THEN
         IF (numpatch > 0) THEN
            allocate (landpatch_%eindex          (numpatch))
            allocate (landpatch_%ipxstt          (numpatch))
            allocate (landpatch_%ipxend          (numpatch))
            allocate (landpatch_%settyp          (numpatch))
            allocate (landpatch_%ielm            (numpatch))
            allocate (landpatch_%xblkgrp         (numpatch))
            allocate (landpatch_%yblkgrp         (numpatch))

            allocate (landelm_%eindex              (numelm))
            allocate (landelm_%ipxstt              (numelm))
            allocate (landelm_%ipxend              (numelm))
            allocate (landelm_%settyp              (numelm))

            allocate (patchclass_                (numpatch))
            allocate (patchtype_                 (numpatch))
            allocate (csol_              (nl_soil,numpatch))

            IF (numpft > 0) THEN
               allocate (pftclass_                 (numpft))
               allocate (patch_pft_s_            (numpatch))
               allocate (patch_pft_e_            (numpatch))
            ENDIF

            IF (numurban > 0) THEN
                allocate (urbclass_              (numurban))
                allocate (patch2urban_           (numpatch))
            ENDIF
         ENDIF
      ENDIF
   END SUBROUTINE allocate_LulccTimeInvariants


   SUBROUTINE SAVE_LulccTimeInvariants

   USE MOD_Namelist
   USE MOD_Precision
   USE MOD_Vars_Global
   USE MOD_SPMD_Task
   USE MOD_Pixelset
   USE MOD_Vars_TimeInvariants
   USE MOD_Landpatch
   USE MOD_Landelm
   USE MOD_Mesh
   USE MOD_Vars_PFTimeInvariants
   USE MOD_LandPFT
   USE MOD_LandUrban

   IMPLICIT NONE

      IF (p_is_worker) THEN
         IF (numpatch > 0) THEN
            CALL copy_pixelset (landpatch, landpatch_ )
            CALL copy_pixelset (landelm  , landelm_   )
            numpatch_             = numpatch
            numelm_               = numelm
            patchclass_       (:) = patchclass       (:)
            patchtype_        (:) = patchtype        (:)
            csol_           (:,:) = csol           (:,:)

            IF (numpft > 0) THEN
               numpft_            = numpft
               pftclass_      (:) = pftclass         (:)
               patch_pft_s_   (:) = patch_pft_s      (:)
               patch_pft_e_   (:) = patch_pft_e      (:)
            ENDIF

            IF (numurban > 0) THEN
               numurban_          = numurban
               urbclass_      (:) = landurban%settyp (:)
               patch2urban_   (:) = patch2urban      (:)
            ENDIF
         ENDIF
      ENDIF

      CALL landpatch_%set_vecgs

   END SUBROUTINE SAVE_LulccTimeInvariants


   SUBROUTINE deallocate_LulccTimeInvariants
   USE MOD_Namelist
   USE MOD_SPMD_Task
   USE MOD_PixelSet
! --------------------------------------------------
! Deallocates memory for Lulcc time invariant variables
! --------------------------------------------------
      IF (p_is_worker) THEN
         IF (numpatch_ > 0) THEN
            CALL landpatch_%forc_free_mem
            CALL landelm_%forc_free_mem
            deallocate    (patchclass_   )
            deallocate    (patchtype_    )
            deallocate    (csol_         )

IF (DEF_USE_PFT .or. DEF_USE_PC) THEN
            IF (numpft_ > 0) THEN
               deallocate (pftclass_     )
               deallocate (patch_pft_s_  )
               deallocate (patch_pft_e_  )
            ENDIF
ENDIF

IF (DEF_URBAN_RUN) THEN
            IF (numurban_ > 0) THEN
               deallocate (urbclass_     )
               deallocate (patch2urban_  )
            ENDIF
ENDIF
         ENDIF
      ENDIF

   END SUBROUTINE deallocate_LulccTimeInvariants

END MODULE MOD_Lulcc_Vars_TimeInvariants
! ---------- EOP ------------
