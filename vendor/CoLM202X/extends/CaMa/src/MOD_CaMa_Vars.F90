#include <define.h>

MODULE MOD_CaMa_Vars
!DESCRIPTION
!===========
   !---This MODULE is the coupler for the colm and CaMa-Flood model.

!ANCILLARY FUNCTIONS AND SUBROUTINES
!-------------------
   !* :SUBROUTINE:"allocate_acc_cama_fluxes"   :  Initilization Accumulation of cama-flood variables
   !* :SUBROUTINE:"deallocate_acc_cama_fluxes" :  deallocate Accumulation of cama-flood variables
   !* :SUBROUTINE:"FLUSH_acc_cama_fluxes"      :  Reset Accumulation of cama-flood variables
   !* :SUBROUTINE:"accumulate_cama_fluxes"     :  Get accumulated cama-flood variables
   !* :SUBROUTINE:"allocate_2D_cama_Fluxes"    :  Get floodplain evaporation
   !* :SUBROUTINE:"hist_out_cama"              :  Average camaflood history variables and write out
   !* :SUBROUTINE:"hist_write_cama_time"       :  write out cama-flood history variables
   !* :SUBROUTINE:"flux_map_and_write_2d_cama" :  map cama output and write 1 or 2D variables, e.g. lon, lat
   !* :SUBROUTINE:"colm2cama_real8"            :  Send variables from worker processors to master processors
   !* :SUBROUTINE:"cama2colm_real8"            :  Send variables from master processors (cama) to worker processors (colm)

!REVISION HISTORY
!----------------
   !---2023.02.23  Zhongwang Wei @ SYSU
   !---2021.12.12  Zhongwang Wei @ SYSU
   !---2020.10.21  Zhongwang Wei @ SYSU

#if(defined CaMa_Flood)
   USE MOD_Precision
   USE MOD_Grid
   USE MOD_DataType
   USE PARKIND1,                 only: JPRB
   USE MOD_SpatialMapping
   USE YOS_CMF_INPUT,            only: RMIS, DMIS,LSEDIMENT,CSETFILE
   USE MOD_Vars_Global,          only: spval
   USE YOS_CMF_INPUT,            ONLY: LOGNAM
   USE CMF_CTRL_SED_MOD,         ONLY: nsed, sDiam, d2sedout_avg, d2sedcon, d2sedinp_avg, d2bedout_avg, d2netflw_avg, d2layer

   real(r8) :: nacc
   real(r8), allocatable         :: a_rnof_cama (:)        ! on worker : total runoff [mm/s]
   type(block_data_real8_2d)     :: f_rnof_cama            ! on IO     : total runoff [mm/s]
   real(r8), allocatable         :: runoff_2d (:,:)        ! on Master : total runoff [mm/s]
   ! Precipitation for sediment forcing (coupled mode)
   real(r8), allocatable         :: a_prcp_cama (:)        ! on worker : precipitation (rain+snow) [mm/s]
   real(r8), allocatable         :: a_prcp_time (:)
   type(block_data_real8_2d)     :: f_prcp_cama            ! on IO     : precipitation [mm/s]
   real(r8), allocatable         :: prcp_2d (:,:)          ! on Master : precipitation [mm/s]
   
   !!!!!!!!!!!! added by shulei
   real(r8), allocatable         :: a_dirrig_cama (:)      ! on worker : total runoff [mm/s]
   real(r8), allocatable         :: dirrig_tmp(:)
   real(r8), allocatable         :: dirrig_day(:)
   type(block_data_real8_2d)     :: f_dirrig_cama          ! on IO     : total runoff [mm/s]
   real(r8), allocatable         :: dirrig_2d (:,:)        ! on Master : total runoff [mm/s]
   real(r8), allocatable         :: dirrig_2d_orig (:,:)
   !!!!!!!!!!!! added by shulei

   !!!!!!!!!!!! added by shulei
   real(r8), allocatable         :: withdrawal_cama (:)    ! on worker : total runoff [mm/s]
   real(r8), allocatable         :: withdrawal_riv_cama (:)! on worker : total runoff [mm/s]
   real(r8), allocatable         :: withdrawal_dam_cama (:)! on worker : total runoff [mm/s]
   real(r8), allocatable         :: withdrawal_rof_cama (:)! on worker : total runoff [mm/s]
   type(block_data_real8_2d)     :: f_withdrawal_cama      ! on IO     : total runoff [mm/s]
   type(block_data_real8_2d)     :: f_withdrawal_riv_cama  ! on IO     : total runoff [mm/s]
   type(block_data_real8_2d)     :: f_withdrawal_dam_cama  ! on IO     : total runoff [mm/s]
   type(block_data_real8_2d)     :: f_withdrawal_rof_cama  ! on IO     : total runoff [mm/s]
   real(r8), allocatable         :: withdrawal_tmp (:,:)       ! on Master : total runoff [mm/s]
   real(r8), allocatable         :: withdrawal_riv_tmp (:,:)   ! on Master : total runoff [mm/s]
   real(r8), allocatable         :: withdrawal_dam_tmp (:,:)   ! on Master : total runoff [mm/s]
   real(r8), allocatable         :: withdrawal_rof_tmp (:,:)   ! on Master : total runoff [mm/s]
   !!!!!!!!!!!! added by shulei

   real(r8), allocatable         :: flddepth_cama (:)      ! on worker : flddepth [m]
   type(block_data_real8_2d)     :: f_flddepth_cama        ! on IO     : flddepth [m]
   real(r8), allocatable         :: flddepth_tmp(:,:)

   real(r8), allocatable         :: fldfrc_cama (:)        ! on worker : flddepth [m]
   type(block_data_real8_2d)     :: f_fldfrc_cama          ! on IO     : flddepth [m]
   real(r8), allocatable         :: fldfrc_tmp (:,:)       ! on Master : total runoff [mm/s]

   real(r8), allocatable         :: fevpg_fld(:)           ! m/s
   real(r8), allocatable         :: finfg_fld(:)           ! m/s

   real(r8), allocatable         :: a_fevpg_fld (:)        ! on worker : flddepth [m]
   type(block_data_real8_2d)     :: f_fevpg_fld            ! on IO : total runoff [mm/s]
   real(r8), allocatable         :: fevpg_2d (:,:)         ! on Master : total runoff [mm/s]

   real(r8), allocatable         :: a_finfg_fld (:)        ! on worker : flddepth [m]
   type(block_data_real8_2d)     :: f_finfg_fld            ! on IO : total runoff [mm/s]
   real(r8), allocatable         :: finfg_2d (:,:)         ! on Master : total runoff [mm/s]
   type(grid_type) :: gcama

   type (spatial_mapping_type) :: mp2g_cama               ! mapping pset to grid
   type (spatial_mapping_type) :: mg2p_cama               ! mapping grid to pset
   real(r8), allocatable :: flood_credit(:)
   type(pointer_real8_1d), allocatable :: flood_credit_part(:), flood_sink_part(:)

   type (grid_concat_type)       :: cama_gather            ! gather grid

   type(block_data_real8_2d)     :: IO_Effdepth            ! inundation to water depth [m]
   type(block_data_real8_2d)     :: IO_Effarea             ! inundation to water area [m2]

   type history_var_cama_type
      logical :: rivout       = .false.
      logical :: rivsto       = .false.
      logical :: rivdph       = .false.
      logical :: rivvel       = .false.
      logical :: fldout       = .false.
      logical :: fldsto       = .false.
      logical :: flddph       = .false.
      logical :: fldfrc       = .false.
      logical :: fldare       = .false.
      logical :: sfcelv       = .false.
      logical :: totout       = .false.
      logical :: outflw       = .false.
      logical :: totsto       = .false.
      logical :: storge       = .false.
      logical :: pthflw       = .false.
      logical :: pthout       = .false.
      logical :: maxflw       = .false.
      logical :: maxdph       = .false.
      logical :: maxsto       = .false.
      logical :: gwsto        = .false.
      logical :: gdwsto       = .false.
      logical :: gwout        = .false.
      logical :: gdwrtn       = .false.
      logical :: runoff       = .false.
      logical :: runoffsub    = .false.
      logical :: rofsfc       = .false.
      logical :: rofsub       = .false.
      logical :: damsto       = .false.
      logical :: daminf       = .false.
      logical :: wevap        = .false.
      logical :: winfilt      = .false.
      logical :: levsto       = .false.
      logical :: levdph       = .false.
      logical :: outflw_ocean = .false.
      logical :: outins       = .false.
      logical :: sedout       = .false.
      logical :: sedcon       = .false.
      logical :: sedinp       = .false.
      logical :: bedout       = .false.
      logical :: netflw       = .false.
      logical :: layer        = .false.
   END type history_var_cama_type

   type (history_var_cama_type) :: DEF_hist_cama_vars

   ! --- subroutines ---
   PUBLIC :: allocate_acc_cama_fluxes
   PUBLIC :: deallocate_acc_cama_fluxes
   PUBLIC :: flush_acc_cama_fluxes
   PUBLIC :: accumulate_cama_fluxes
   PUBLIC :: allocate_2D_cama_Fluxes
   PUBLIC :: colm2cama_real8
   PUBLIC :: colmvar2cama_real8
   PUBLIC :: cama2colm_real8
   PUBLIC :: camavar2colm_real8
   PUBLIC :: hist_out_cama
   PUBLIC :: flux_map_and_write_3d_cama

CONTAINS

   SUBROUTINE allocate_acc_cama_fluxes
!DESCRIPTION
!===========
   ! This subrountine is used for initilization Accumulation of cama-flood variables

!ANCILLARY FUNCTIONS AND SUBROUTINES
!-------------------

!REVISION HISTORY
!----------------
   ! 2021.12.12  Zhongwang Wei @ SYSU


   USE MOD_SPMD_Task !spmd_task
   USE MOD_LandPatch, only: numpatch
   USE MOD_Vars_Global

   IMPLICIT NONE

      !allocate cama-flood variables on worker
      IF (p_is_worker) THEN
         IF (numpatch > 0 .OR. CSETFILE=='NONE') THEN
            allocate (a_rnof_cama(numpatch))
            allocate (a_fevpg_fld(numpatch))
            allocate (a_finfg_fld(numpatch))
            allocate (a_dirrig_cama(numpatch))
            allocate (a_prcp_cama(numpatch))
            IF(CSETFILE=='NONE') allocate(a_prcp_time(numpatch))
         ENDIF
      ENDIF
      IF(CSETFILE=='NONE'.AND..NOT.p_is_worker) allocate(a_prcp_time(0))

   END SUBROUTINE allocate_acc_cama_fluxes

   SUBROUTINE deallocate_acc_cama_fluxes()
!DESCRIPTION
!===========
! This subrountine is used for deallocate Accumulation of cama-flood variables

!ANCILLARY FUNCTIONS AND SUBROUTINES
   !-------------------

!REVISION HISTORY
   !----------------
   ! 2020.10.21  Zhongwang Wei @ SYSU

   USE MOD_SPMD_Task
   USE MOD_LandPatch, only: numpatch

   IMPLICIT NONE

      IF (p_is_worker) THEN
         IF (numpatch > 0 .OR. CSETFILE=='NONE') THEN
            deallocate (a_rnof_cama)
            deallocate (a_prcp_cama)
            deallocate (a_fevpg_fld)
            deallocate (a_finfg_fld)
            deallocate (a_dirrig_cama)
         ENDIF
      ENDIF
      IF(allocated(a_prcp_time)) deallocate(a_prcp_time)

   END SUBROUTINE deallocate_acc_cama_fluxes

   SUBROUTINE FLUSH_acc_cama_fluxes ()
!DESCRIPTION
!===========
! This subrountine is used for reset Accumulation of cama-flood variables

!ANCILLARY FUNCTIONS AND SUBROUTINES
   !-------------------

!REVISION HISTORY
   !----------------
   ! 2020.10.21  Zhongwang Wei @ SYSU
   USE MOD_SPMD_Task
   USE MOD_LandPatch,      only: numpatch
   USE MOD_Vars_Global,    only: spval

   IMPLICIT NONE

      IF (p_is_worker) THEN

         nacc = 0

         IF (numpatch > 0) THEN
            ! flush the Fluxes for accumulation
            a_rnof_cama (:) = spval
            a_prcp_cama (:) = spval
            IF(allocated(a_prcp_time)) a_prcp_time(:)=0._r8
            a_fevpg_fld (:) = spval
            a_finfg_fld (:) = spval
            a_dirrig_cama(:)= spval
         ENDIF
      ENDIF

   END SUBROUTINE FLUSH_acc_cama_fluxes

   SUBROUTINE accumulate_cama_fluxes(deltim)
!DESCRIPTION
!===========
   ! This subrountine is used for accumulating  cama-flood variables

!ANCILLARY FUNCTIONS AND SUBROUTINES
   !-------------------
   !* :SUBROUTINE:"acc1d_cama"            :  accumulating 1D cama-flood variables

!REVISION HISTORY
   !----------------
   ! 2020.10.21  Zhongwang Wei @ SYSU

   USE MOD_Precision
   USE MOD_SPMD_Task
   USE MOD_Vars_1DFluxes, only: rnof
   USE MOD_Vars_1DForcing, only: forc_rain
   USE MOD_Vars_TimeVariables, only: reservoirriver_demand
   USE MOD_LandPatch, only: numpatch
   USE MOD_Namelist, only: DEF_forcing
   USE MOD_Forcing, only: forcmask_pch
   USE YOS_CMF_INPUT, only: LWEVAP, LWINFILT, LDAMIRR

   IMPLICIT NONE

      real(r8), optional, intent(in) :: deltim
      logical :: valid_patch(numpatch)
      integer :: i

      IF (p_is_worker) THEN
         IF (numpatch > 0) THEN
            IF(CSETFILE=='NONE')THEN
               IF(.NOT.PRESENT(deltim)) ERROR STOP 'CaMa: missing coupling time step'
               valid_patch=.true.
               IF(DEF_forcing%has_missing_value) valid_patch=forcmask_pch
               nacc=nacc+deltim
               CALL acc1d_cama(rnof,a_rnof_cama,deltim,valid_patch)
               IF(LSEDIMENT)THEN
                  CALL acc1d_cama(forc_rain,a_prcp_cama,deltim,valid_patch)
                  WHERE(valid_patch.AND.forc_rain/=spval) a_prcp_time=a_prcp_time+deltim
               ENDIF
               IF(LWEVAP) CALL acc1d_cama(fevpg_fld,a_fevpg_fld,deltim,valid_patch)
               IF(LWINFILT) CALL acc1d_cama(finfg_fld,a_finfg_fld,deltim,valid_patch)
               IF(LDAMIRR) CALL acc1d_cama(reservoirriver_demand,a_dirrig_cama,msk=valid_patch)
            ELSE
               nacc = nacc + 1
               CALL acc1d_cama (rnof, a_rnof_cama)
               CALL acc1d_cama (forc_rain, a_prcp_cama)
               CALL acc1d_cama (fevpg_fld, a_fevpg_fld)
               CALL acc1d_cama (finfg_fld, a_finfg_fld)
               call acc1d_cama (reservoirriver_demand, a_dirrig_cama)
            ENDIF
         ENDIF
      ENDIF

   END SUBROUTINE accumulate_cama_fluxes

   SUBROUTINE acc1d_cama (var, s, weight, msk)
!DESCRIPTION
!===========
   ! This subrountine is used for accumulating 1D cama-flood variables

!ANCILLARY FUNCTIONS AND SUBROUTINES
   !-------------------

!REVISION HISTORY
   !----------------
   ! 2020.10.21  Zhongwang Wei @ SYSU

   USE MOD_Precision
   USE MOD_Vars_Global, only: spval

   IMPLICIT NONE

      real(r8), intent(in)    :: var(:) ! variable to be accumulated
      real(r8), intent(inout) :: s  (:) ! new added value
      real(r8), optional, intent(in) :: weight
      logical, optional, intent(in) :: msk(:)
!----------------------- Dummy argument --------------------------------
      logical, dimension(size(var)) :: valid_var, valid_s
      real(r8) :: wt

      ! Use vectorized operations for better performance
      wt=1._r8
      IF(PRESENT(weight)) wt=weight
      valid_var = (var /= spval)
      IF(PRESENT(msk)) valid_var=valid_var.AND.msk
      valid_s   = (s /= spval)
      
      ! Initialize where s is invalid but var is valid
      WHERE (valid_var .and. .not. valid_s) s = var*wt
      
      ! Accumulate where both are valid
      WHERE (valid_var .and. valid_s) s = s + var*wt

   END SUBROUTINE acc1d_cama

   SUBROUTINE allocate_2D_cama_Fluxes (grid)
!DESCRIPTION
!===========
   ! This subrountine is used for accumulating 2D cama-flood variables

!ANCILLARY FUNCTIONS AND SUBROUTINES
!-------------------
   !* :SUBROUTINE:"allocate_block_data"            :  allocate 2D cama-flood variables to colm block

!REVISION HISTORY
   !----------------
   ! 2020.10.21  Zhongwang Wei @ SYSU

   USE MOD_SPMD_Task
   USE MOD_Grid
   USE MOD_DataType

   IMPLICIT NONE

      type(grid_type), intent(in) :: grid

      IF (p_is_io) THEN
         CALL allocate_block_data (grid, f_rnof_cama)      ! total runoff         [m/s]
         CALL allocate_block_data (grid, f_prcp_cama)      ! precipitation         [m/s]
         CALL allocate_block_data (grid, f_flddepth_cama)  ! inundation depth     [m/s]
         CALL allocate_block_data (grid, f_fldfrc_cama)    ! inundation fraction  [m/s]
         !TODO: check the following variables
         CALL allocate_block_data (grid, f_fevpg_fld)      ! inundation evaporation [m/s]
         CALL allocate_block_data (grid, f_finfg_fld)      ! inundation re-infiltration [m/s]
         CALL allocate_block_data (grid, f_dirrig_cama)
         CALL allocate_block_data (grid, f_withdrawal_cama)
         CALL allocate_block_data (grid, f_withdrawal_riv_cama)
         CALL allocate_block_data (grid, f_withdrawal_dam_cama)
         CALL allocate_block_data (grid, f_withdrawal_rof_cama)
      ENDIF

   END SUBROUTINE allocate_2D_cama_Fluxes

   SUBROUTINE hist_out_cama (file_hist, itime_in_file)
!DESCRIPTION
!===========
   ! This subrountine is used for averaging and writing 2D cama-flood variables out

!ANCILLARY FUNCTIONS AND SUBROUTINES
   !-------------------
   !* :SUBROUTINE:"CMF_DIAG_GETAVE_OUTPUT"                      :  averaging the diagnostic variables of cama-flood
   !* :SUBROUTINE:"flux_map_and_write_2d_cama"            :  map camaflood variables to colm block and write out
   !* :SUBROUTINE:"CMF_DIAG_RESET_OUTPUT"                        :  reset diagnostic variables of cama-flood
!REVISION HISTORY
   !----------------
   ! 2020.10.21  Zhongwang Wei @ SYSU

   USE MOD_SPMD_Task
   USE CMF_CALC_DIAG_MOD,  only: CMF_DIAG_GETAVE_OUTPUT, CMF_DIAG_RESET_OUTPUT
   USE YOS_CMF_TIME,       only: KMIN, KMINEND, JDD, JMM, JHHMM
   USE MOD_Namelist,       only: DEF_HIST_FREQ
   USE YOS_CMF_DIAG,       only: NADD_out, RESTART_DIAG_FIELD
   USE YOS_CMF_PROG,       only: P2RIVSTO,     P2FLDSTO,     P2GDWSTO, &
         P2damsto,P2LEVSTO !!! added
   USE YOS_CMF_DIAG,       only: D2RIVDPH,     D2FLDDPH,     D2FLDFRC,     D2FLDARE,     &
         D2SFCELV,     D2STORGE,                                                         &
         D2OUTFLW_oAVG, D2RIVOUT_oAVG, D2FLDOUT_oAVG, D2PTHOUT_oAVG, D1PTHFLW_oAVG,           &
         D2RIVVEL_oAVG, D2GDWRTN_oAVG, D2RUNOFF_oAVG, D2ROFSUB_oAVG,                         &
         D2OUTFLW_oMAX, D2STORGE_oMAX, D2RIVDPH_oMAX,                                       &
         d2daminf_oavg, D2WEVAPEX_oAVG,D2WINFILTEX_oAVG, D2LEVDPH, D2OUTINS !!! added
   USE YOS_CMF_MAP, only: NSEQMAX, NPTHOUT, PTH_UPST
 !      USE MOD_Vars_2DFluxes

   IMPLICIT NONE

   character(LEN=*), intent(in) :: file_hist
   integer, intent(in)          :: itime_in_file
   real(r8), allocatable :: pthflw_cell(:,:)
   integer :: ipth
   real(JPRB), allocatable :: raw_history(:,:,:), raw_pthflw(:,:)
   real(JPRB), pointer :: history_field(:,:)
   real(JPRB) :: raw_elapsed
   character(len=32) :: history_name
   logical :: preserve_partial_history
   integer :: ihist


      IF (CSETFILE=='NONE' .AND. NADD_out<=0._r8) RETURN
      preserve_partial_history=.false.
      IF (CSETFILE=='NONE' .AND. KMIN>=KMINEND) THEN
         SELECT CASE (TRIM(ADJUSTL(DEF_HIST_FREQ)))
         CASE ('HOURLY')
            preserve_partial_history=MOD(KMIN,60)/=0
         CASE ('DAILY')
            preserve_partial_history=MOD(KMIN,1440)/=0
         CASE ('MONTHLY')
            preserve_partial_history=JDD/=1 .OR. JHHMM/=0
         CASE ('YEARLY')
            preserve_partial_history=JMM/=1 .OR. JDD/=1 .OR. JHHMM/=0
         END SELECT
      ENDIF
      IF (preserve_partial_history) THEN
         raw_elapsed=NADD_out
         allocate(raw_history(NSEQMAX,1,14))
         raw_history=0._JPRB
         DO ihist=1,14
            CALL RESTART_DIAG_FIELD(ihist,history_name,history_field)
            IF (associated(history_field)) raw_history(:,:,ihist)=history_field
         ENDDO
         raw_pthflw=D1PTHFLW_oAVG
      ENDIF

      !*** average variable
      CALL CMF_DIAG_GETAVE_OUTPUT

      !*** write output data
      CALL flux_map_and_write_2d_cama (DEF_hist_cama_vars%rivout, &
      real(D2RIVOUT_oAVG), file_hist, 'rivout', itime_in_file,'river discharge','m3/s')

      CALL flux_map_and_write_2d_cama (DEF_hist_cama_vars%rivsto, &
      real(P2RIVSTO), file_hist, 'rivsto', itime_in_file,'river storage','m3')

      CALL flux_map_and_write_2d_cama (DEF_hist_cama_vars%rivdph, &
      real(D2RIVDPH), file_hist, 'rivdph', itime_in_file,'river depth','m')

      CALL flux_map_and_write_2d_cama (DEF_hist_cama_vars%rivvel, &
      real(D2RIVVEL_oAVG), file_hist, 'rivvel', itime_in_file,'river velocity','m/s')

      CALL flux_map_and_write_2d_cama(DEF_hist_cama_vars%fldout, &
      real(D2FLDOUT_oAVG), file_hist, 'fldout', itime_in_file,'floodplain discharge','m3/s')

      CALL flux_map_and_write_2d_cama(DEF_hist_cama_vars%fldsto, &
      real(P2FLDSTO), file_hist, 'fldsto', itime_in_file,'floodplain storage','m3')

      CALL flux_map_and_write_2d_cama(DEF_hist_cama_vars%flddph, &
      real(D2FLDDPH), file_hist, 'flddph', itime_in_file,'floodplain depth','m')

      CALL flux_map_and_write_2d_cama(DEF_hist_cama_vars%fldfrc, &
      real(D2FLDFRC), file_hist, 'fldfrc', itime_in_file,'flooded fraction','0-1')

      CALL flux_map_and_write_2d_cama(DEF_hist_cama_vars%fldare, &
      real(D2FLDARE), file_hist, 'fldare', itime_in_file,'flooded area','m2')

      CALL flux_map_and_write_2d_cama(DEF_hist_cama_vars%sfcelv, &
      real(D2SFCELV), file_hist, 'sfcelv', itime_in_file,'water surface elevation','m')

      CALL flux_map_and_write_2d_cama(DEF_hist_cama_vars%totout, &
      real(D2OUTFLW_oAVG), file_hist, 'totout', itime_in_file,'discharge (river+floodplain)','m3/s')

      CALL flux_map_and_write_2d_cama(DEF_hist_cama_vars%outflw, &
      real(D2OUTFLW_oAVG), file_hist, 'outflw', itime_in_file,'discharge (river+floodplain)','m3/s')

      CALL flux_map_and_write_2d_cama(DEF_hist_cama_vars%totsto, &
      real(D2STORGE), file_hist, 'totsto', itime_in_file,'total storage (river+floodplain)','m3')

      CALL flux_map_and_write_2d_cama(DEF_hist_cama_vars%storge, &
      real(D2STORGE), file_hist, 'storge', itime_in_file,'total storage (river+floodplain)','m3')

      IF(CSETFILE=='NONE'.AND.DEF_hist_cama_vars%pthflw)THEN
         allocate(pthflw_cell(NSEQMAX,1))
         pthflw_cell=0._r8
         IF(allocated(PTH_UPST).AND.allocated(D1PTHFLW_oAVG))THEN
            DO ipth=1,NPTHOUT
               IF(PTH_UPST(ipth)<1.OR.PTH_UPST(ipth)>NSEQMAX) &
                  ERROR STOP 'CaMa: invalid bifurcation upstream cell'
               pthflw_cell(PTH_UPST(ipth),1)=pthflw_cell(PTH_UPST(ipth),1)+ &
                  REAL(SUM(D1PTHFLW_oAVG(ipth,:)),KIND=r8)
            ENDDO
         ENDIF
         CALL flux_map_and_write_2d_cama(.true.,pthflw_cell,file_hist,'pthflw',itime_in_file, &
            'bifurcation channel discharge (sum over levels, attributed to the upstream cell)','m3/s')
         deallocate(pthflw_cell)
      ELSE IF(CSETFILE/='NONE')THEN
         CALL flux_map_and_write_2d_cama(DEF_hist_cama_vars%pthflw, &
            real(D1PTHFLW_oAVG),file_hist,'pthflw',itime_in_file, &
            'bifurcation channel discharge ','m3/s')
      ENDIF

      CALL flux_map_and_write_2d_cama(DEF_hist_cama_vars%pthout, &
      real(D2PTHOUT_oAVG), file_hist, 'pthout', itime_in_file,'net bifurcation discharge','m3/s')

      CALL flux_map_and_write_2d_cama(DEF_hist_cama_vars%gdwsto, &
      real(P2GDWSTO), file_hist, 'gdwsto', itime_in_file,'ground water storage','m3')

      CALL flux_map_and_write_2d_cama(DEF_hist_cama_vars%gwsto, &
      real(P2GDWSTO), file_hist, 'gwsto', itime_in_file,'ground water storage','m3')

      CALL flux_map_and_write_2d_cama(DEF_hist_cama_vars%gwout, &
      real(D2GDWRTN_oAVG), file_hist, 'gwout', itime_in_file,'ground water discharge','m3/s')

      CALL flux_map_and_write_2d_cama(DEF_hist_cama_vars%runoff, &
      real(D2RUNOFF_oAVG), file_hist, 'runoff', itime_in_file,'Surface runoff','m3/s')

      CALL flux_map_and_write_2d_cama(DEF_hist_cama_vars%runoffsub, &
      real(D2ROFSUB_oAVG)  , file_hist, 'runoffsub', itime_in_file,'sub-surface runoff','m3/s')

      CALL flux_map_and_write_2d_cama(DEF_hist_cama_vars%maxsto, &
      real(D2STORGE_oMAX), file_hist, 'maxsto', itime_in_file,'daily maximum storage','m3')

      CALL flux_map_and_write_2d_cama(DEF_hist_cama_vars%maxflw, &
      real(D2OUTFLW_oMAX), file_hist, 'maxflw', itime_in_file,'daily maximum discharge','m3/s')

      CALL flux_map_and_write_2d_cama(DEF_hist_cama_vars%maxdph, &
      real(D2RIVDPH_oMAX), file_hist, 'maxdph', itime_in_file,'daily maximum river depth','m')

      IF (DEF_hist_cama_vars%damsto) THEN
      CALL flux_map_and_write_2d_cama(DEF_hist_cama_vars%damsto, &
      real(p2damsto), file_hist, 'damsto', itime_in_file,'reservoir storage','m3')
      ENDIF
      IF (DEF_hist_cama_vars%outins) THEN
         CALL flux_map_and_write_2d_cama(DEF_hist_cama_vars%outins, &
         real(D2OUTINS), file_hist, 'outins', itime_in_file,'instantaneous discharge','m3/s')
      ENDIF

      IF (DEF_hist_cama_vars%daminf) THEN
      CALL flux_map_and_write_2d_cama(DEF_hist_cama_vars%daminf, &
      real(d2daminf_oavg), file_hist, 'daminf', itime_in_file,'reservoir inflow','m3/s')
      ENDIF

      IF (DEF_hist_cama_vars%wevap) THEN
      CALL flux_map_and_write_2d_cama(DEF_hist_cama_vars%wevap, &
      real(D2WEVAPEX_oAVG), file_hist, 'wevap', itime_in_file,'inundation water evaporation','m3/s')
      ENDIF

      IF (DEF_hist_cama_vars%winfilt) THEN
      CALL flux_map_and_write_2d_cama(DEF_hist_cama_vars%winfilt, &
      real(D2WINFILTEX_oAVG), file_hist, 'winfilt', itime_in_file,'inundation water infiltration','m3/s')
      ENDIF

      IF (DEF_hist_cama_vars%levsto) THEN
      CALL flux_map_and_write_2d_cama(DEF_hist_cama_vars%levsto, &
      real(P2LEVSTO), file_hist, 'levsto', itime_in_file,'protected area storage','m3')
      ENDIF

      IF (DEF_hist_cama_vars%levdph) THEN
      CALL flux_map_and_write_2d_cama(DEF_hist_cama_vars%levdph, &
      real(D2LEVDPH), file_hist, 'levdph', itime_in_file,'protected area depth','m')
      ENDIF

      IF (DEF_hist_cama_vars%outflw_ocean) THEN
      CALL flux_map_and_write_2d_cama_ocean(DEF_hist_cama_vars%outflw_ocean, &
      real(D2OUTFLW_oAVG), file_hist, 'outflw_ocean', itime_in_file,'discharge to ocean','m3/s')
      ENDIF

      ! Sediment output
      IF (DEF_hist_cama_vars%sedout) THEN
      CALL flux_map_and_write_3d_cama(DEF_hist_cama_vars%sedout, &
      real(d2sedout_avg), file_hist, 'sedout', itime_in_file,'suspended sediment flow','m3/s')
      ENDIF

      IF (DEF_hist_cama_vars%sedcon) THEN
      CALL flux_map_and_write_3d_cama(DEF_hist_cama_vars%sedcon, &
      real(d2sedcon), file_hist, 'sedcon', itime_in_file,'suspended sediment concentration','m3/m3')
      ENDIF

      IF (DEF_hist_cama_vars%sedinp) THEN
      CALL flux_map_and_write_3d_cama(DEF_hist_cama_vars%sedinp, &
      real(d2sedinp_avg), file_hist, 'sedinp', itime_in_file,'sediment inflow from land','m3/s')
      ENDIF

      IF (DEF_hist_cama_vars%bedout) THEN
      CALL flux_map_and_write_3d_cama(DEF_hist_cama_vars%bedout, &
      real(d2bedout_avg), file_hist, 'bedout', itime_in_file,'bedload','m3/s')
      ENDIF

      IF (DEF_hist_cama_vars%netflw) THEN
      CALL flux_map_and_write_3d_cama(DEF_hist_cama_vars%netflw, &
      real(d2netflw_avg), file_hist, 'netflw', itime_in_file,'net entrainment flow','m3/s')
      ENDIF

      IF (DEF_hist_cama_vars%layer) THEN
      CALL flux_map_and_write_3d_cama(DEF_hist_cama_vars%layer, &
      real(d2layer), file_hist, 'layer', itime_in_file,'exchange layer volume','m3')
      ENDIF

      ! Handle deplyr variables (sediment deposition layers)
      ! Note: deplyr parsing would require additional logic to extract layer number
      ! For now, we use the same layer variable as a placeholder

      !*** reset variable
      CALL CMF_DIAG_RESET_OUTPUT

      IF (preserve_partial_history) THEN
         NADD_out=raw_elapsed
         DO ihist=1,14
            CALL RESTART_DIAG_FIELD(ihist,history_name,history_field)
            IF (associated(history_field)) history_field=raw_history(:,:,ihist)
         ENDDO
         D1PTHFLW_oAVG=raw_pthflw
      ENDIF

   END SUBROUTINE hist_out_cama


   SUBROUTINE hist_write_cama_time (filename, dataname, time, itime)
!DESCRIPTION
!===========
   ! This subrountine is used for writing time,longitude and latitude of cama-flood output using netcdf format.

!ANCILLARY FUNCTIONS AND SUBROUTINES
!-------------------
   !* :SUBROUTINE:"ncio_create_file"                      :  create netcdf file, see ncio_serial.F90
   !* :SUBROUTINE:"ncio_define_dimension"                 :  define dimension of netcdf file, see ncio_serial.F90
   !* :SUBROUTINE:"ncio_write_serial"                     :  write serial data into netcdf file (lon, lat), see ncio_serial.F90
   !* :SUBROUTINE:"ncio_write_time"                       :  write time serial into netcdf file (lon, lat), see ncio_serial.F90

!REVISION HISTORY
!----------------
   ! 2023.02.23  Zhongwang Wei @ SYSU

   USE MOD_SPMD_Task
   USE MOD_NetCDFSerial
   USE YOS_CMF_INPUT, only: NX, NY
   USE YOS_CMF_MAP,   only: D1LON, D1LAT
   USE CMF_CTRL_SED_MOD, only: nsed, sDiam

   IMPLICIT NONE

   character (len=*), intent(in) :: filename ! file name
   character (len=*), intent(in) :: dataname ! data name
   integer, intent(in)  :: time(3)           ! time (year, month, day)
   integer, intent(out) :: itime             ! number of time step

   ! Local variables
   logical :: fexists
   logical :: needSedDef

      IF (p_is_master) THEN
         inquire (file=filename, exist=fexists)
         IF (.not. fexists) THEN
            CALL ncio_create_file (trim(filename))
            CALL ncio_define_dimension(filename, 'time', 0)
            CALL ncio_define_dimension(filename,'lat_cama', NY)
            CALL ncio_define_dimension(filename,'lon_cama', NX)
            ! Define sediment dimension only if any sediment outputs are enabled
            needSedDef = LSEDIMENT .and. ( &
     &         DEF_hist_cama_vars%sedout .or. DEF_hist_cama_vars%sedcon .or. DEF_hist_cama_vars%sedinp .or. &
     &         DEF_hist_cama_vars%bedout .or. DEF_hist_cama_vars%netflw .or. DEF_hist_cama_vars%layer)
            IF (needSedDef) CALL ncio_define_dimension(filename, 'sedD', nsed)
            CALL ncio_write_serial (filename, 'lat_cama', D1LAT,'lat_cama')
            CALL ncio_write_serial (filename, 'lon_cama', D1LON,'lon_cama')
            ! Write sediment diameter data only if needed
            IF (needSedDef) THEN
               CALL ncio_write_serial_real8_1d (filename, 'sedD', real(sDiam, kind=r8), 'sedD')
               CALL ncio_put_attr (filename, 'sedD', 'long_name', 'sediment grain size')
               CALL ncio_put_attr (filename, 'sedD', 'units', 'meters')
            ENDIF
         ENDIF

         CALL ncio_write_time (filename, dataname, time, itime)
      ENDIF

   END SUBROUTINE hist_write_cama_time

   SUBROUTINE flux_map_and_write_2d_cama_ocean (is_hist, &
         var_in, file_hist, varname, itime_in_file,longname,units)
         !DESCRIPTION
         !===========
            ! This subrountine is used for mapping cama-flood output using netcdf format.

         !ANCILLARY FUNCTIONS AND SUBROUTINES
         !-------------------
            !* :SUBROUTINE:"ncio_put_attr"                      :  write netcdf attribute, see ncio_serial.F90
            !* :SUBROUTINE:"vecP2mapR"                          :  convert 1D vector data -> 2D map data (real*4), CAMA/cmf_utils_mod.F90
            !* :SUBROUTINE:"ncio_write_serial_time"             :  define dimension of netcdf file, see ncio_serial.F90

         !REVISION HISTORY
         !----------------
            ! 2023.02.23  Zhongwang Wei @ SYSU

         USE MOD_Namelist
         USE MOD_Vars_Global, only: spval
         USE YOS_CMF_INPUT,  only: NX, NY
         USE YOS_CMF_MAP,    only: NSEQMAX
         USE PARKIND1,       only: JPRM
         USE CMF_UTILS_MOD,  only: vecP2mapR
         USE MOD_NetCDFSerial,    only: ncio_write_serial_time, ncio_put_attr, ncio_var_exist
         USE YOS_CMF_MAP,        only: I2NEXTX, I2NEXTY
         IMPLICIT NONE
         logical, intent(in)          :: is_hist
         real(r8), intent(in)         ::  var_in (NSEQMAX, 1)
         character(len=*), intent(in) :: file_hist
         character(len=*), intent(in) :: varname
         integer, intent(in)          :: itime_in_file
      
         character (len=*), intent(in),optional :: longname
         character (len=*), intent(in),optional :: units
      
         real(KIND=JPRM)             :: R2OUT(NX,NY)
         integer  :: i,j
         integer  :: compress
         logical  :: var_is_new
      
            IF (.not. is_hist) RETURN
            CALL vecP2mapR(var_in(1:NSEQMAX,1),R2OUT)
            do j = 1,NY
               do i = 1,NX
                  if (I2NEXTX(i,j) .NE. -9) then
                     R2OUT(i,j) = real(real(spval,kind=JPRM),kind=8)
                  endif
               enddo
            enddo
 
            compress = DEF_HIST_CompressLevel
            var_is_new = .not. ncio_var_exist (file_hist, varname, readflag = .false.)
            CALL ncio_write_serial_time (file_hist, varname,  &
               itime_in_file, real(R2OUT,kind=8), 'lon_cama', 'lat_cama', 'time',compress)
            IF (itime_in_file == 1 .or. var_is_new) THEN
               CALL ncio_put_attr (file_hist, varname, 'long_name', longname)
               CALL ncio_put_attr (file_hist, varname, 'units', units)
               CALL ncio_put_attr (file_hist, varname, 'missing_value',real(real(spval,kind=JPRM),kind=8))
            ENDIF
   end subroutine flux_map_and_write_2d_cama_ocean


   SUBROUTINE flux_map_and_write_2d_cama (is_hist, &
         var_in, file_hist, varname, itime_in_file,longname,units)

!DESCRIPTION
!===========
   ! This subrountine is used for mapping cama-flood output using netcdf format.

!ANCILLARY FUNCTIONS AND SUBROUTINES
!-------------------
   !* :SUBROUTINE:"ncio_put_attr"                      :  write netcdf attribute, see ncio_serial.F90
   !* :SUBROUTINE:"vecP2mapR"                          :  convert 1D vector data -> 2D map data (real*4), CAMA/cmf_utils_mod.F90
   !* :SUBROUTINE:"ncio_write_serial_time"             :  define dimension of netcdf file, see ncio_serial.F90

!REVISION HISTORY
!----------------
   ! 2023.02.23  Zhongwang Wei @ SYSU

   USE MOD_Namelist
   USE YOS_CMF_INPUT,  only: NX, NY, LSEDIMENT
   USE YOS_CMF_MAP,    only: NSEQMAX
   USE PARKIND1,       only: JPRM
   USE CMF_UTILS_MOD,  only: vecP2mapR
   USE MOD_NetCDFSerial,    only: ncio_write_serial_time, ncio_put_attr, ncio_var_exist

   IMPLICIT NONE
   logical, intent(in)          :: is_hist
   real(r8), intent(in)         ::  var_in (NSEQMAX, 1)
   character(len=*), intent(in) :: file_hist
   character(len=*), intent(in) :: varname
   integer, intent(in)          :: itime_in_file
   
   character (len=*), intent(in),optional :: longname
   character (len=*), intent(in),optional :: units

   real(KIND=JPRM)             :: R2OUT(NX,NY)

   integer  :: compress
   logical  :: var_is_new

      IF (.not. is_hist) RETURN

      CALL vecP2mapR(var_in(1:NSEQMAX,1),R2OUT)
      compress = DEF_HIST_CompressLevel
      var_is_new = .not. ncio_var_exist (file_hist, varname, readflag = .false.)
      CALL ncio_write_serial_time (file_hist, varname,  &
         itime_in_file, real(R2OUT,kind=8), 'lon_cama', 'lat_cama', 'time',compress)
      IF (itime_in_file == 1 .or. var_is_new) THEN
         CALL ncio_put_attr (file_hist, varname, 'long_name', longname)
         CALL ncio_put_attr (file_hist, varname, 'units', units)
         CALL ncio_put_attr (file_hist, varname, 'missing_value',real(real(spval,kind=JPRM),kind=8))
      ENDIF

   END SUBROUTINE flux_map_and_write_2d_cama

   SUBROUTINE flux_map_and_write_3d_cama (is_hist, &
         var_in, file_hist, varname, itime_in_file, longname, units)
!DESCRIPTION
!===========
   ! This subrountine is used for mapping 3D cama-flood sediment output using netcdf format.
   ! 3D data has dimensions (NSEQALL, nsed) where nsed is the number of sediment classes.

!ANCILLARY FUNCTIONS AND SUBROUTINES
!-------------------
   !* :SUBROUTINE:"ncio_put_attr"                      :  write netcdf attribute, see ncio_serial.F90
   !* :SUBROUTINE:"vecP2mapR"                          :  convert 1D vector data -> 2D map data (real*4), CAMA/cmf_utils_mod.F90
   !* :SUBROUTINE:"ncio_write_serial_time"             :  define dimension of netcdf file, see ncio_serial.F90

!REVISION HISTORY
!----------------
   ! 2025.09.23  Zhongwang Wei @ SYSU

   USE MOD_Namelist
   USE YOS_CMF_INPUT,  only: NX, NY
   USE YOS_CMF_MAP,    only: NSEQMAX
   USE PARKIND1,       only: JPRM
   USE CMF_UTILS_MOD,  only: vecP2mapR
   USE MOD_NetCDFSerial,    only: ncio_write_serial_time, ncio_put_attr, ncio_var_exist, ncio_write_serial_real8_1d, ncio_define_dimension

   IMPLICIT NONE
   logical, intent(in)          :: is_hist
   real(r8), intent(in)         :: var_in (NSEQMAX, nsed)
   character(len=*), intent(in) :: file_hist
   character(len=*), intent(in) :: varname
   integer, intent(in)          :: itime_in_file

   character (len=*), intent(in),optional :: longname
   character (len=*), intent(in),optional :: units

   real(KIND=JPRM)             :: R3OUT(NX,NY,nsed)
   integer                     :: compress
   integer                     :: ised
   logical                     :: var_is_new

      IF (.not. is_hist) RETURN

      R3OUT(:,:,:) = real(real(spval,kind=JPRM),kind=8)

      DO ised = 1, nsed
         CALL vecP2mapR(var_in(1:NSEQMAX,ised), R3OUT(:,:,ised))
      END DO

      compress = DEF_HIST_CompressLevel

      ! Ensure sedD dimension exists even if file was created earlier without sediment
      IF (LSEDIMENT) THEN
         CALL ncio_define_dimension (file_hist, 'sedD', nsed)
      ENDIF
      var_is_new = .not. ncio_var_exist (file_hist, varname, readflag = .false.)
      CALL ncio_write_serial_time (file_hist, varname,  &
         itime_in_file, real(R3OUT,kind=8), 'lon_cama', 'lat_cama', 'sedD', 'time', compress)

      IF (itime_in_file == 1 .or. var_is_new) THEN
         CALL ncio_put_attr (file_hist, varname, 'long_name', longname)
         CALL ncio_put_attr (file_hist, varname, 'units', units)
         CALL ncio_put_attr (file_hist, varname, 'missing_value', real(real(spval,kind=JPRM),kind=8))

         ! Ensure sediment diameter variable exists and add attributes
         CALL ncio_write_serial_real8_1d (file_hist, 'sedD', real(sDiam, kind=r8), 'sedD')
         CALL ncio_put_attr (file_hist, 'sedD', 'long_name', 'sediment grain size')
         CALL ncio_put_attr (file_hist, 'sedD', 'units', 'meters')
         
      ENDIF

   END SUBROUTINE flux_map_and_write_3d_cama

   FUNCTION cama_master_cols (dsp, cnt, n) RESULT (cols)
   IMPLICIT NONE
   integer, intent(in) :: dsp, cnt, n
   integer :: cols(cnt), k
      cols = (/ (mod(dsp + cama_gather%ilon0 + k - 2, n) + 1, k = 1, cnt) /)
   END FUNCTION cama_master_cols

   SUBROUTINE colm2cama_real8 (WorkerVar, IOVar, MasterVar, integral, flood_sink, valid_time)
!DESCRIPTION
!===========
   ! This subrountine is used for mapping colm output to cama input.

!ANCILLARY FUNCTIONS AND SUBROUTINES
!-------------------
   !* :SUBROUTINE:"allocate_block_data"                      :  allocate data into block

!REVISION HISTORY
!----------------
   ! 2023.02.23  Zhongwang Wei @ SYSU

   USE MOD_Precision
   USE MOD_Namelist
   USE MOD_TimeManager
   USE MOD_SPMD_Task
   USE MOD_Block
   USE MOD_DataType
   USE MOD_LandPatch
   USE MOD_Vars_TimeInvariants, only: patchtype
   USE MOD_Forcing, only: forcmask_pch

   IMPLICIT NONE

   real(r8),                  intent(inout) :: WorkerVar(:)    !varialbe on worker processer
   type(block_data_real8_2d), intent(inout) :: IOVar           !varialbe on IO processer
   real(r8),                  intent(inout) :: MasterVar(:,:)  !varialbe on master processer
   logical, optional, intent(in) :: integral, flood_sink
   real(r8), optional, intent(in) :: valid_time(:)

   type(block_data_real8_2d) :: sumwt                          !sum of weight
   logical,  allocatable     :: filter(:)                      !filter for patchtype
   logical :: volumes, newmode, credits
   integer :: ip
   !----------------------- Dummy argument --------------------------------
   integer :: xblk, yblk, xloc, yloc
   integer :: iblk, jblk, idata, ixseg, iyseg
   integer :: rmesg(3), smesg(3), isrc
   integer :: xdsp, ydsp, xcnt, ycnt
   real(r8), allocatable :: rbuf(:,:), sbuf(:,:), vdata(:,:)

      volumes = .false.
      IF(PRESENT(integral)) volumes = integral
      credits = .false.
      IF(PRESENT(flood_sink)) credits = flood_sink
      newmode=CSETFILE=='NONE'
      IF(credits.AND..NOT.newmode) ERROR STOP 'CaMa: flood sink mapping requires NC mode'
      IF(PRESENT(valid_time).AND.(volumes.OR.credits)) ERROR STOP 'CaMa: valid time requires an intensive field'
      IF(p_is_master)THEN
         MasterVar(:,:) = spval
         IF(volumes.OR.newmode) MasterVar(:,:) = 0._r8
      ENDIF

      IF (p_is_worker) THEN
         IF(newmode.AND.numpatch>0.AND.nacc<=0._r8) ERROR STOP 'CaMa: empty flux window'
         IF(.NOT.PRESENT(valid_time))THEN
            WHERE (WorkerVar /= spval)
               WorkerVar = WorkerVar / nacc
            ENDWHERE
         ENDIF

         IF (numpatch > 0) THEN
            allocate (filter (numpatch))

            filter(:) = patchtype < 99
            IF(newmode) filter=filter.AND.WorkerVar/=spval
            IF(newmode.AND.PRESENT(valid_time)) filter=filter.AND.valid_time>0._r8
            IF (.NOT.newmode.AND.DEF_forcing%has_missing_value) THEN
               filter = filter .and. forcmask_pch
            ENDIF
         ENDIF
      ENDIF

      IF(credits)THEN
         IF(p_is_worker)THEN
            DO ip=1,numpatch
               IF(mg2p_cama%npart(ip)==0) CYCLE
               flood_sink_part(ip)%val=0._r8
               IF(WorkerVar(ip)==spval) CYCLE
               IF(WorkerVar(ip)<0._r8) ERROR STOP 'CaMa: negative flood sink'
               IF(flood_credit(ip)>0._r8)THEN
                  flood_sink_part(ip)%val=flood_credit_part(ip)%val * &
                     (WorkerVar(ip)*1.e-3_r8/flood_credit(ip))
               ELSEIF(WorkerVar(ip)>1.e-12_r8)THEN
                  ERROR STOP 'CaMa: patch consumed unpublished flood water'
               ENDIF
            ENDDO
         ENDIF
         CALL mg2p_cama%part2grid(flood_sink_part,IOVar)
      ELSE
         CALL mp2g_cama%pset2grid (WorkerVar, IOVar, spv = spval, msk = filter)
      ENDIF

      IF (.NOT.volumes) THEN
         IF (p_is_io) CALL allocate_block_data (gcama, sumwt)
         IF(PRESENT(valid_time))THEN
            CALL mp2g_cama%pset2grid(valid_time,sumwt,spv=spval,msk=filter)
         ELSE
            CALL mp2g_cama%get_sumarea (sumwt, filter)
         ENDIF
      ENDIF


      IF (p_is_io) THEN
         DO yblk = 1, gblock%nyblk
            DO xblk = 1, gblock%nxblk
               IF (gblock%pio(xblk,yblk) == p_iam_glb) THEN
                  DO yloc = 1, gcama%ycnt(yblk)
                     DO xloc = 1, gcama%xcnt(xblk)

                        IF (credits) THEN
                           IF(IOVar%blk(xblk,yblk)%val(xloc,yloc)==spval)THEN
                              IOVar%blk(xblk,yblk)%val(xloc,yloc)=0._r8
                           ELSE
                              IOVar%blk(xblk,yblk)%val(xloc,yloc)= &
                                 IOVar%blk(xblk,yblk)%val(xloc,yloc) * &
                                 mg2p_cama%areagrid%blk(xblk,yblk)%val(xloc,yloc)*1.e6_r8
                           ENDIF
                        ELSEIF (volumes) THEN
                           IF (IOVar%blk(xblk,yblk)%val(xloc,yloc) == spval) THEN
                              IOVar%blk(xblk,yblk)%val(xloc,yloc) = 0._r8
                           ELSE
                              IOVar%blk(xblk,yblk)%val(xloc,yloc) = &
                                 IOVar%blk(xblk,yblk)%val(xloc,yloc) * 1.e3_r8
                           ENDIF
                        ELSEIF (sumwt%blk(xblk,yblk)%val(xloc,yloc) > MERGE(0._r8,0.00001_r8,newmode)) THEN
                           IF (IOVar%blk(xblk,yblk)%val(xloc,yloc) /= spval) THEN
                              IOVar%blk(xblk,yblk)%val(xloc,yloc) &
                                 = IOVar%blk(xblk,yblk)%val(xloc,yloc) &
                                 / sumwt%blk(xblk,yblk)%val(xloc,yloc)
                           ENDIF
                        ELSE
                           IOVar%blk(xblk,yblk)%val(xloc,yloc) = MERGE(0._r8,spval,newmode)
                        ENDIF
                     ENDDO
                  ENDDO

               ENDIF
            ENDDO
         ENDDO
      ENDIF

      IF (p_is_master) THEN
         DO idata = 1, cama_gather%ndatablk
            CALL mpi_recv (rmesg, 3, MPI_INTEGER, MPI_ANY_SOURCE, 10011, p_comm_glb, p_stat, p_err)
            isrc  = rmesg(1)
            ixseg = rmesg(2)
            iyseg = rmesg(3)

            xdsp = cama_gather%xsegs(ixseg)%gdsp
            ydsp = cama_gather%ysegs(iyseg)%gdsp
            xcnt = cama_gather%xsegs(ixseg)%cnt
            ycnt = cama_gather%ysegs(iyseg)%cnt

            allocate (rbuf(xcnt,ycnt))
            CALL mpi_recv (rbuf, xcnt * ycnt, MPI_DOUBLE, &
               isrc, 10011, p_comm_glb, p_stat, p_err)
            IF(newmode)THEN
               MasterVar(cama_master_cols(xdsp,xcnt,size(MasterVar,1)), &
                  ydsp+cama_gather%ilat0:ydsp+cama_gather%ilat0+ycnt-1) = rbuf
            ELSE
               MasterVar(xdsp+1:xdsp+xcnt,ydsp+1:ydsp+ycnt) = rbuf
            ENDIF

            deallocate (rbuf)
         ENDDO

      ELSEIF (p_is_io) THEN
         DO iyseg = 1, cama_gather%nyseg
            DO ixseg = 1, cama_gather%nxseg

               iblk = cama_gather%xsegs(ixseg)%blk
               jblk = cama_gather%ysegs(iyseg)%blk

               IF (gblock%pio(iblk,jblk) == p_iam_glb) THEN
                  xdsp = cama_gather%xsegs(ixseg)%bdsp
                  ydsp = cama_gather%ysegs(iyseg)%bdsp
                  xcnt = cama_gather%xsegs(ixseg)%cnt
                  ycnt = cama_gather%ysegs(iyseg)%cnt

                  allocate (sbuf (xcnt,ycnt))
                  sbuf = IOVar%blk(iblk,jblk)%val(xdsp+1:xdsp+xcnt,ydsp+1:ydsp+ycnt)

                  smesg = (/p_iam_glb, ixseg, iyseg/)
                  CALL mpi_send (smesg, 3, MPI_INTEGER, &
                     p_address_master, 10011, p_comm_glb, p_err)
                  CALL mpi_send (sbuf, xcnt*ycnt, MPI_DOUBLE, &
                     p_address_master, 10011, p_comm_glb, p_err)

                  deallocate (sbuf)

               ENDIF
            ENDDO
         ENDDO
      ENDIF

      IF (allocated(filter)) deallocate(filter)

   END SUBROUTINE colm2cama_real8

   SUBROUTINE colmvar2cama_real8 (WorkerVar, IOVar, MasterVar)
      !DESCRIPTION
      !===========
      ! This subrountine is used for mapping colm output to cama input. 
      ! colm output is total irrigation demands at each grid cell

      !ANCILLARY FUNCTIONS AND SUBROUTINES
      !-------------------
      !* :SUBROUTINE:"allocate_block_data"                      :  allocate data into block

      !REVISION HISTORY
      !----------------
      ! 2023.02.23  Zhongwang Wei @ SYSU

      USE MOD_Precision
      USE MOD_Namelist
      USE MOD_TimeManager
      USE MOD_SPMD_Task
      USE MOD_Block
      USE MOD_DataType
      USE MOD_LandPatch
      USE MOD_Vars_TimeInvariants, only: patchtype
      USE MOD_Forcing, only: forcmask_pch

      IMPLICIT NONE

      real(r8),                  intent(inout) :: WorkerVar(:)    !varialbe on worker processer
      TYPE(block_data_real8_2d), intent(inout) :: IOVar           !varialbe on IO processer
      real(r8),                  INTENT(inout) :: MasterVar(:,:)  !varialbe on master processer

      type(block_data_real8_2d) :: sumwt                          !sum of weight
      logical,  allocatable     :: filter(:)                      !filter for patchtype
      !----------------------- Dummy argument --------------------------------
      integer :: xblk, yblk, xloc, yloc
      integer :: iblk, jblk, idata, ixseg, iyseg
      integer :: rmesg(3), smesg(3), isrc
      real(r8), allocatable :: rbuf(:,:), sbuf(:,:), vdata(:,:)
      integer :: xdsp, ydsp, xcnt, ycnt

      IF (p_is_master) THEN
         MasterVar(:,:) = spval
      ENDIF


      IF (p_is_worker) THEN
         WHERE (WorkerVar /= spval)
            WorkerVar = WorkerVar
         endwhere
 

         IF (numpatch > 0) THEN
            allocate (filter (numpatch))

            filter(:) = patchtype < 99
            IF (DEF_forcing%has_missing_value) THEN
               filter = filter .and. forcmask_pch
            ENDIF
         ENDIF
      ENDIF
 
      CALL mp2g_cama%pset2grid (WorkerVar, IOVar, spv = spval, msk = filter)

      IF (p_is_io) CALL allocate_block_data (gcama, sumwt)
      call mp2g_cama%get_sumarea (sumwt, filter)

      IF (p_is_io) then
         do yblk = 1, gblock%nyblk
            do xblk = 1, gblock%nxblk
               IF (gblock%pio(xblk,yblk) == p_iam_glb) then
                  do yloc = 1, gcama%ycnt(yblk)
                     do xloc = 1, gcama%xcnt(xblk)

                        IF (sumwt%blk(xblk,yblk)%val(xloc,yloc) > 0.00001) then
                           IF (IOVar%blk(xblk,yblk)%val(xloc,yloc) /= spval) THEN
                              ! if(IOVar%blk(xblk,yblk)%val(xloc,yloc).gt.0)THEN
                              !    write(*,*),"LHB debug line848 withdraw error : IOVar, sumwt -----> ",&
                              !       IOVar%blk(xblk,yblk)%val(xloc,yloc), sumwt%blk(xblk,yblk)%val(xloc,yloc), xblk, yblk, xloc, yloc
                              ! endif
                              IOVar%blk(xblk,yblk)%val(xloc,yloc) &
                                 = IOVar%blk(xblk,yblk)%val(xloc,yloc) * 1000000.0D0
                           ENDIF
                        else
                           IOVar%blk(xblk,yblk)%val(xloc,yloc) = spval
                        endif
                     enddo
                  enddo
               endif
            enddo
         enddo
      endif
     
      IF (p_is_master) THEN
         DO idata = 1, cama_gather%ndatablk      
            CALL mpi_recv (rmesg, 3, MPI_INTEGER, MPI_ANY_SOURCE, 10011, p_comm_glb, p_stat, p_err)

            isrc  = rmesg(1)
            ixseg = rmesg(2)
            iyseg = rmesg(3)

            xdsp = cama_gather%xsegs(ixseg)%gdsp
            ydsp = cama_gather%ysegs(iyseg)%gdsp
            xcnt = cama_gather%xsegs(ixseg)%cnt
            ycnt = cama_gather%ysegs(iyseg)%cnt

            allocate (rbuf(xcnt,ycnt))
            CALL mpi_recv (rbuf, xcnt * ycnt, MPI_DOUBLE, &
               isrc, 10011, p_comm_glb, p_stat, p_err)
            IF(CSETFILE=='NONE')THEN
               MasterVar(cama_master_cols(xdsp,xcnt,size(MasterVar,1)), &
                  ydsp+cama_gather%ilat0:ydsp+cama_gather%ilat0+ycnt-1) = rbuf
            ELSE
               MasterVar(xdsp+1:xdsp+xcnt,ydsp+1:ydsp+ycnt) = rbuf
            ENDIF
            deallocate (rbuf)
         ENDDO

      ELSEIF (p_is_io) THEN
         DO iyseg = 1, cama_gather%nyseg
            DO ixseg = 1, cama_gather%nxseg

               iblk = cama_gather%xsegs(ixseg)%blk
               jblk = cama_gather%ysegs(iyseg)%blk

               IF (gblock%pio(iblk,jblk) == p_iam_glb) THEN
                  xdsp = cama_gather%xsegs(ixseg)%bdsp
                  ydsp = cama_gather%ysegs(iyseg)%bdsp
                  xcnt = cama_gather%xsegs(ixseg)%cnt
                  ycnt = cama_gather%ysegs(iyseg)%cnt

                  allocate (sbuf (xcnt,ycnt))
                  sbuf = IOVar%blk(iblk,jblk)%val(xdsp+1:xdsp+xcnt,ydsp+1:ydsp+ycnt)

                  smesg = (/p_iam_glb, ixseg, iyseg/)
                  CALL mpi_send (smesg, 3, MPI_INTEGER, &
                     p_address_master, 10011, p_comm_glb, p_err)
                  CALL mpi_send (sbuf, xcnt*ycnt, MPI_DOUBLE, &
                     p_address_master, 10011, p_comm_glb, p_err)

                  deallocate (sbuf)

               ENDIF
            ENDDO
         ENDDO
      ENDIF
      
      IF (allocated(filter)) deallocate(filter)

   END SUBROUTINE colmvar2cama_real8

   SUBROUTINE cama2colm_real8 (MasterVar, IOVar, WorkerVar)

!DESCRIPTION
!===========
   ! This subrountine is used for mapping cama-flood output to colm input

!ANCILLARY FUNCTIONS AND SUBROUTINES
!-------------------
   !* :SUBROUTINE:"mg2p_cama%map_aweighted"                 :  mapping grid to pset_type

!REVISION HISTORY
   !----------------
   ! 2023.02.23  Zhongwang Wei @ SYSU
   ! 2022.?      Zhongwang Wei and ShuPeng Zhang @ SYSU

   USE MOD_Precision
   USE MOD_Namelist
   USE MOD_TimeManager
   USE MOD_SPMD_Task
   USE MOD_Block
   USE MOD_DataType
   USE MOD_LandPatch
   USE MOD_Vars_TimeInvariants, only: patchtype
   USE MOD_Grid

   IMPLICIT NONE

   real(r8),                  intent(in)    :: MasterVar (:,:) ! Variable at master processor
   type(block_data_real8_2d), intent(inout) :: IOVar           ! Variable at io processor
   real(r8),                  intent(inout) :: WorkerVar (:)   ! Variable at worker processor

   integer :: xblk    , yblk    , xloc ,  yloc
   integer :: iblk    , jblk    , idata,  ixseg,  iyseg
   integer :: rmesg(2), smesg(2), isrc ,  iproc
   integer :: xdsp    , ydsp    , xcnt ,  ycnt
   real(r8), allocatable :: rbuf(:,:), sbuf(:,:), vdata(:,:)


      IF (p_is_master) THEN
         DO iyseg = 1, cama_gather%nyseg
            DO ixseg = 1, cama_gather%nxseg
               iblk = cama_gather%xsegs(ixseg)%blk
               jblk = cama_gather%ysegs(iyseg)%blk

               IF (gblock%pio(iblk,jblk) >= 0) THEN
                  xdsp = cama_gather%xsegs(ixseg)%gdsp
                  ydsp = cama_gather%ysegs(iyseg)%gdsp
                  xcnt = cama_gather%xsegs(ixseg)%cnt
                  ycnt = cama_gather%ysegs(iyseg)%cnt

                  allocate (sbuf (xcnt,ycnt))
                  IF(CSETFILE=='NONE')THEN
                     sbuf = MasterVar(cama_master_cols(xdsp,xcnt,size(MasterVar,1)), &
                        ydsp+cama_gather%ilat0:ydsp+cama_gather%ilat0+ycnt-1)
                  ELSE
                     sbuf = MasterVar(xdsp+1:xdsp+xcnt,ydsp+1:ydsp+ycnt)
                  ENDIF
                  smesg = (/ixseg, iyseg/)
                  CALL mpi_send (smesg, 2, MPI_INTEGER, &
                     gblock%pio(iblk,jblk), 10000, p_comm_glb, p_err)
                  CALL mpi_send (sbuf, xcnt*ycnt, MPI_DOUBLE, &
                     gblock%pio(iblk,jblk), 10000, p_comm_glb, p_err)
                  deallocate (sbuf)
               ENDIF
            ENDDO
         ENDDO

         DO iproc = 0, p_np_io-1
            smesg = (/0, 0/)
            CALL mpi_send(smesg, 2, MPI_INTEGER, p_address_io(iproc), 10000, p_comm_glb, p_err)
         ENDDO
      ELSEIF  (p_is_io) THEN
         DO WHILE (.true.)
            CALL mpi_recv (rmesg, 2, MPI_INTEGER, p_address_master, 10000, p_comm_glb, p_stat, p_err)
            ixseg = rmesg(1)
            iyseg = rmesg(2)

            IF ((ixseg > 0) .and. (iyseg > 0)) THEN
               iblk = cama_gather%xsegs(ixseg)%blk
               jblk = cama_gather%ysegs(iyseg)%blk
               xdsp = cama_gather%xsegs(ixseg)%bdsp
               ydsp = cama_gather%ysegs(iyseg)%bdsp
               xcnt = cama_gather%xsegs(ixseg)%cnt
               ycnt = cama_gather%ysegs(iyseg)%cnt

               allocate (rbuf(xcnt,ycnt))
               CALL mpi_recv (rbuf, xcnt*ycnt, MPI_DOUBLE, &
                  p_address_master, 10000, p_comm_glb, p_stat, p_err)
               IOVar%blk(iblk,jblk)%val(xdsp+1:xdsp+xcnt,ydsp+1:ydsp+ycnt)= rbuf
               deallocate (rbuf)
            ELSE
               EXIT
            ENDIF
         ENDDO
      ENDIF

      CALL mg2p_cama%grid2pset (IOVar, WorkerVar) !mapping grid to pset_type

   END SUBROUTINE cama2colm_real8

   SUBROUTINE camavar2colm_real8 (MasterVar, IOVar, WorkerVar)

      !DESCRIPTION
      !===========
      ! This subrountine is used for mapping cama-flood output to colm input
      ! cama-flood output is total irrigation demands at each grid cell

      !ANCILLARY FUNCTIONS AND SUBROUTINES
      !-------------------
      !* :SUBROUTINE:"mg2p_cama%map_aweighted"                 :  mapping grid to pset_type

      !REVISION HISTORY
      !----------------
      ! 2023.02.23  Zhongwang Wei @ SYSU
      ! 2022.?      Zhongwang Wei and ShuPeng Zhang @ SYSU

      USE MOD_Precision
      USE MOD_Namelist
      USE MOD_TimeManager
      USE MOD_SPMD_Task
      USE MOD_Block
      USE MOD_DataType
      USE MOD_LandPatch
      USE MOD_Vars_TimeInvariants, only: patchtype
      USE MOD_Grid

      IMPLICIT NONE

      real(r8),                  INTENT(in)    :: MasterVar (:,:) ! Variable at master processor
      type(block_data_real8_2d), INTENT(inout) :: IOVar           ! Variable at io processor
      real(r8),                  intent(inout) :: WorkerVar (:)   ! Variable at worker processor

      integer :: xblk    , yblk    , xloc ,  yloc
      integer :: iblk    , jblk    , idata,  ixseg,  iyseg
      integer :: rmesg(2), smesg(2), isrc ,  iproc
      integer :: xdsp    , ydsp    , xcnt ,  ycnt
      real(r8), allocatable :: rbuf(:,:), sbuf(:,:), vdata(:,:)

      IF (p_is_master) THEN
         DO iyseg = 1, cama_gather%nyseg
            DO ixseg = 1, cama_gather%nxseg
               iblk = cama_gather%xsegs(ixseg)%blk
               jblk = cama_gather%ysegs(iyseg)%blk

               IF (gblock%pio(iblk,jblk) >= 0) THEN
                  xdsp = cama_gather%xsegs(ixseg)%gdsp
                  ydsp = cama_gather%ysegs(iyseg)%gdsp
                  xcnt = cama_gather%xsegs(ixseg)%cnt
                  ycnt = cama_gather%ysegs(iyseg)%cnt

                  allocate (sbuf (xcnt,ycnt))
                  IF(CSETFILE=='NONE')THEN
                     sbuf = MasterVar(cama_master_cols(xdsp,xcnt,size(MasterVar,1)), &
                        ydsp+cama_gather%ilat0:ydsp+cama_gather%ilat0+ycnt-1)
                  ELSE
                     sbuf = MasterVar(xdsp+1:xdsp+xcnt,ydsp+1:ydsp+ycnt)
                  ENDIF
                  smesg = (/ixseg, iyseg/)
                  CALL mpi_send (smesg, 2, MPI_INTEGER, &
                     gblock%pio(iblk,jblk), 10000, p_comm_glb, p_err)
                  CALL mpi_send (sbuf, xcnt*ycnt, MPI_DOUBLE, &
                     gblock%pio(iblk,jblk), 10000, p_comm_glb, p_err)
                  deallocate (sbuf)
               ENDIF
            ENDDO
         ENDDO

         DO iproc = 0, p_np_io-1
            smesg = (/0, 0/)
            CALL mpi_send(smesg, 2, MPI_INTEGER, p_address_io(iproc), 10000, p_comm_glb, p_err)
         ENDDO
      ELSEIF  (p_is_io) THEN
         DO WHILE (.true.)
            call mpi_recv (rmesg, 2, MPI_INTEGER, p_address_master, 10000, p_comm_glb, p_stat, p_err)
            ixseg = rmesg(1)
            iyseg = rmesg(2)

            IF ((ixseg > 0) .and. (iyseg > 0)) THEN
               iblk = cama_gather%xsegs(ixseg)%blk
               jblk = cama_gather%ysegs(iyseg)%blk
               xdsp = cama_gather%xsegs(ixseg)%bdsp
               ydsp = cama_gather%ysegs(iyseg)%bdsp
               xcnt = cama_gather%xsegs(ixseg)%cnt
               ycnt = cama_gather%ysegs(iyseg)%cnt

               allocate (rbuf(xcnt,ycnt))
               call mpi_recv (rbuf, xcnt*ycnt, MPI_DOUBLE, &
                  p_address_master, 10000, p_comm_glb, p_stat, p_err)
               IOVar%blk(iblk,jblk)%val(xdsp+1:xdsp+xcnt,ydsp+1:ydsp+ycnt)= rbuf
               !write(*,*) "LHB debug line1037 withdraw error : ixseg, iyseg, IOVar -----> ", ixseg, iyseg, rbuf
               deallocate (rbuf)
            ELSE
               exit
            ENDIF
         ENDDO
      ENDIF

      CALL mg2p_cama%grid2pset_varvalue (IOVar, WorkerVar) !mapping grid to pset_type
   END SUBROUTINE camavar2colm_real8

#endif

END MODULE MOD_CaMa_Vars
! ----- EOP ---------
