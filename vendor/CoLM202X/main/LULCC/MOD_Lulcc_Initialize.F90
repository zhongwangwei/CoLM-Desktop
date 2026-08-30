#include <define.h>
MODULE MOD_Lulcc_Initialize

   USE MOD_Precision
   IMPLICIT NONE
   SAVE

! PUBLIC MEMBER FUNCTIONS:
   PUBLIC :: LulccInitialize

CONTAINS

   SUBROUTINE LulccInitialize (casename, dir_landdata, dir_restart, &
                               jdate, greenwich)

!-----------------------------------------------------------------------
!
! !DESCRIPTION:
!  Initialization routine for Land-use-Land-cover-change (Lulcc) case
!
!  Created by Hua Yuan, 04/08/2022
!
! !REVISIONS:
!  08/2023, Wenzong Dong: Porting to MPI version and share the same code
!           with MOD_Initialize:initialize()
!
!-----------------------------------------------------------------------

   USE MOD_Precision
   USE MOD_Namelist
   USE MOD_SPMD_Task
   USE MOD_Block
   USE MOD_Mesh
   USE MOD_LandElm
#if defined (UNSTRUCTURED) || defined (CATCHMENT)
   USE MOD_ElmVector, only: elm_vector_init
#endif
#ifdef CATCHMENT
   USE MOD_LandHRU
   USE MOD_HRUVector, only: hru_vector_init
#endif
   USE MOD_LandPatch
   USE MOD_LandPFT
   USE MOD_LandUrban
   USE MOD_Const_LC
   USE MOD_Const_PFT
   USE MOD_TimeManager
   USE MOD_Lulcc_Vars_TimeInvariants
   USE MOD_Lulcc_Vars_TimeVariables
   USE MOD_SrfdataRestart
   USE MOD_Vars_TimeInvariants
   USE MOD_Vars_TimeVariables
   USE MOD_Initialize

   IMPLICIT NONE

!-------------------------- Dummy Arguments ----------------------------
   character(len=*), intent(in) :: casename      ! case name
   character(len=*), intent(in) :: dir_landdata
   character(len=*), intent(in) :: dir_restart

   integer, intent(inout) :: jdate(3)   ! year, julian day, seconds of the starting time
   logical, intent(in)    :: greenwich  ! true: greenwich time, false: local time

!-------------------------- Local Variables ----------------------------
   integer :: year, jday

!-----------------------------------------------------------------------

      ! initial time of model run and consts
      year = jdate(1)
      jday = jdate(2)

      CALL Init_GlobalVars
      CALL Init_LC_Const
      CALL Init_PFT_Const

      ! deallocate pixelset and mesh data of previous year
      CALL mesh_free_mem
      CALL landelm%forc_free_mem
      CALL landpatch%forc_free_mem
      IF (DEF_USE_PFT .or. DEF_USE_PC) THEN
      CALL landpft%forc_free_mem
      ENDIF
      IF (DEF_URBAN_RUN) THEN
      CALL landurban%forc_free_mem
      ENDIF

      ! load pixelset and mesh data of next year
      ! CALL pixel%load_from_file  (dir_landdata)
      ! CALL gblock%load_from_file (dir_landdata)
      CALL mesh_load_from_file     (dir_landdata, year)
      CALL pixelset_load_from_file (dir_landdata, 'landelm'  , landelm  , numelm  , year)

      ! load CATCHMENT of next year
#ifdef CATCHMENT
      CALL pixelset_load_from_file (dir_landdata, 'landhru'  , landhru  , numhru  , year)
#endif

      ! load landpatch data of next year
      CALL pixelset_load_from_file (dir_landdata, 'landpatch', landpatch, numpatch, year)

      ! load pft data of PFT/PC of next year
      IF (DEF_USE_PFT .or. DEF_USE_PC) THEN
      CALL pixelset_load_from_file (dir_landdata, 'landpft'  , landpft  , numpft  , year)
      CALL map_patch_to_pft
      ENDIF

      ! load urban data of next year
      IF (DEF_URBAN_RUN) THEN
      CALL pixelset_load_from_file (dir_landdata, 'landurban', landurban, numurban, year)
      CALL map_patch_to_urban
      ENDIF

      ! initialize for data associated with land element
#if (defined UNSTRUCTURED || defined CATCHMENT)
      CALL elm_vector_init ()
#ifdef CATCHMENT
      CALL hru_vector_init ()
#endif
#endif

      ! build element subfraction of next year which it's needed in the MOD_Lulcc_TransferTrace
      IF (p_is_worker) THEN
         CALL elm_patch%build (landelm, landpatch, use_frac = .true.)
      ENDIF


      ! --------------------------------------------------------------------
      ! Deallocates memory for CoLM 1d [numpatch] variables
      ! --------------------------------------------------------------------
      CALL deallocate_TimeInvariants
      CALL deallocate_TimeVariables

      ! initialize all state variables of next year
      CALL initialize (casename, dir_landdata, dir_restart,&
                       jdate, year, greenwich, lulcc_call=.true.)

   END SUBROUTINE LulccInitialize

END MODULE MOD_Lulcc_Initialize
! ---------- EOP ------------
