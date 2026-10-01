#include <define.h>

MODULE MOD_NewSnow

!-----------------------------------------------------------------------
   USE MOD_Precision
   IMPLICIT NONE
   SAVE

! PUBLIC MEMBER FUNCTIONS:
   PUBLIC :: newsnow
   PUBLIC :: relocate_soil_frost_ice


!-----------------------------------------------------------------------

CONTAINS

!-----------------------------------------------------------------------


   SUBROUTINE newsnow (patchtype,maxsnl,deltim,t_grnd,pg_rain,pg_snow,bifall,&
                       t_precip,zi_soisno,z_soisno,dz_soisno,t_soisno,&
                       wliq_soisno,wice_soisno,fiold,snl,sag,scv,snowdp,fsno,wetwat)

!=======================================================================
!  add new snow nodes.
!  Original author: Yongjiu Dai, 09/15/1999; 08/31/2002, 07/2013, 04/2014
!=======================================================================

   USE MOD_Precision
   USE MOD_Namelist, only: DEF_USE_VariablySaturatedFlow
   USE MOD_Const_Physical, only: tfrz, cpliq, cpice

   IMPLICIT NONE

!-------------------------- Dummy Arguments ----------------------------

   integer, intent(in) :: maxsnl     ! maximum number of snow layers
   integer, intent(in) :: patchtype  ! land patch type (0=soil, 1=urban and built-up,
                                     ! 2=wetland, 3=land ice, 4=land water bodies, 99=ocean)
   real(r8), intent(in) :: deltim    ! model time step [second]
   real(r8), intent(in) :: t_grnd    ! ground surface temperature [k]
   real(r8), intent(in) :: pg_rain   ! rainfall onto ground including canopy runoff [kg/(m2 s)]
   real(r8), intent(in) :: pg_snow   ! snowfall onto ground including canopy runoff [kg/(m2 s)]
   real(r8), intent(in) :: bifall    ! bulk density of newly fallen dry snow [kg/m3]
   real(r8), intent(in) :: t_precip  ! snowfall/rainfall temperature [kelvin]

   real(r8), intent(inout) ::   zi_soisno(maxsnl:0)   ! interface level below a "z" level (m)
   real(r8), intent(inout) ::    z_soisno(maxsnl+1:0) ! layer depth (m)
   real(r8), intent(inout) ::   dz_soisno(maxsnl+1:0) ! layer thickness (m)
   real(r8), intent(inout) ::    t_soisno(maxsnl+1:0) ! soil + snow layer temperature [K]
   real(r8), intent(inout) :: wliq_soisno(maxsnl+1:0) ! liquid water (kg/m2)
   real(r8), intent(inout) :: wice_soisno(maxsnl+1:0) ! ice lens (kg/m2)
   real(r8), intent(inout) :: fiold(maxsnl+1:0)       ! fraction of ice relative to the total water
   integer , intent(inout) :: snl                     ! number of snow layers
   real(r8), intent(inout) :: sag                     ! non dimensional snow age [-]
   real(r8), intent(inout) :: scv                     ! snow mass (kg/m2)
   real(r8), intent(inout) :: snowdp                  ! snow depth (m)
   real(r8), intent(inout) :: fsno                    ! fraction of soil covered by snow [-]

   real(r8), intent(inout), optional :: wetwat        ! wetland water [mm]

!-------------------------- Local Variables ----------------------------

   real(r8) dz_snowf  ! layer thickness rate change due to precipitation [mm/s]
   integer newnode    ! signification when new snow node is set, (1=yes, 0=no)
   integer lb

!-----------------------------------------------------------------------

      newnode = 0

      dz_snowf = pg_snow/bifall
      snowdp = snowdp + dz_snowf*deltim
      scv = scv + pg_snow*deltim              ! snow water equivalent (mm)

      ! snowfall on warmer wetland melts into the wetland water, but only while there is
      ! no snow layer: with layers, scv is the layers' sum and clearing it alone would
      ! break the snow mass (formerly enforced only in TRACER builds).
      IF(patchtype==2 .and. t_grnd>tfrz .and. snl==0)THEN
         IF (present(wetwat) .and. DEF_USE_VariablySaturatedFlow) THEN
            wetwat = wetwat + scv
         ENDIF
         scv=0.; snowdp=0.; sag=0.; fsno = 0.
      ENDIF

      zi_soisno(0) = 0.

! when the snow accumulation exceeds 10 mm, initialize a snow layer

      IF(snl==0 .and. pg_snow>0.0 .and. snowdp>=0.01)THEN
         snl = -1
         newnode = 1
         dz_soisno(0)  = snowdp             ! meter
         z_soisno (0)  = -0.5*dz_soisno(0)
         zi_soisno(-1) = -dz_soisno(0)

         sag = 0.                           ! snow age
         t_soisno (0) = min(tfrz, t_precip) ! K
         wice_soisno(0) = scv               ! kg/m2
         wliq_soisno(0) = 0.                ! kg/m2
         fiold(0) = 1.
         fsno = min(1.,tanh(0.1*pg_snow*deltim))
      ENDIF

      ! --------------------------------------------------
      ! snowfall on snow pack
      ! --------------------------------------------------
      ! the change of ice partial density of surface node due to precipitation
      ! only ice part of snowfall is added here, the liquid part will be added latter

      IF(snl<0 .and. newnode==0)THEN
         lb = snl + 1

         wice_soisno(lb) = wice_soisno(lb)+deltim*pg_snow
         dz_soisno(lb) = dz_soisno(lb)+dz_snowf*deltim
         z_soisno(lb) = zi_soisno(lb) - 0.5*dz_soisno(lb)
         zi_soisno(lb-1) = zi_soisno(lb) - dz_soisno(lb)

         ! update fsno by new snow event, add to previous fsno
         ! shape factor for accumulation of snow = 0.1
         fsno = 1. - (1. - tanh(0.1*pg_snow*deltim))*(1. - fsno)
         fsno = min(1., fsno)

      ENDIF

   END SUBROUTINE newsnow

   SUBROUTINE relocate_soil_frost_ice(maxsnl, porsl1, snl, zi, z, dz, t, wliq, wice, &
                                     fiold, imelt, snofrz, snw_rds, scv, snowdp, &
                                     mss_bcpho, mss_bcphi, mss_ocpho, mss_ocphi, &
                                     mss_dst1, mss_dst2, mss_dst3, mss_dst4, &
                                     trc_wice, trc_wliq, trc_solid, trc_scv)
      USE MOD_Const_Physical, only: denice, cpice, cpliq
      USE MOD_Namelist, only: DEF_USE_SNICAR
      integer, intent(in) :: maxsnl
      real(r8), intent(in) :: porsl1
      integer, intent(inout) :: snl
      real(r8), intent(inout) :: zi(maxsnl:0), z(maxsnl+1:1), dz(maxsnl+1:1)
      real(r8), intent(inout) :: t(maxsnl+1:1), wliq(maxsnl+1:1), wice(maxsnl+1:1)
      real(r8), intent(inout) :: fiold(maxsnl+1:1)
      integer, intent(inout) :: imelt(maxsnl+1:1)
      real(r8), intent(inout) :: snofrz(maxsnl+1:0), snw_rds(maxsnl+1:0)
      real(r8), intent(inout) :: scv, snowdp
      real(r8), intent(inout) :: mss_bcpho(maxsnl+1:0), mss_bcphi(maxsnl+1:0)
      real(r8), intent(inout) :: mss_ocpho(maxsnl+1:0), mss_ocphi(maxsnl+1:0)
      real(r8), intent(inout) :: mss_dst1(maxsnl+1:0), mss_dst2(maxsnl+1:0)
      real(r8), intent(inout) :: mss_dst3(maxsnl+1:0), mss_dst4(maxsnl+1:0)
      real(r8), intent(inout), optional :: trc_wice(:,maxsnl+1:), trc_wliq(:,maxsnl+1:)
      real(r8), intent(inout), optional :: trc_solid(:,maxsnl+1:), trc_scv(:)
      real(r8) :: excess, fraction, heat_capacity, added_depth
      integer :: top

      excess = max(wice(1) - denice*porsl1*dz(1), 0._r8)
      IF (excess <= 0._r8) RETURN
      fraction = excess/wice(1)
      added_depth = excess/denice

      IF (snl == 0) THEN
         IF (present(trc_wice)) THEN
            IF (.not. present(trc_scv)) ERROR STOP 'frost ice: missing thin-snow tracer store'
            IF (snowdp + added_depth >= 0.01_r8) THEN
               trc_wice(:,0) = trc_scv + fraction*trc_wice(:,1)
               trc_scv = 0._r8
               IF (present(trc_wliq)) trc_wliq(:,0) = 0._r8
               IF (present(trc_solid)) trc_solid(:,0) = 0._r8
            ELSE
               trc_scv = trc_scv + fraction*trc_wice(:,1)
            ENDIF
            trc_wice(:,1) = (1._r8-fraction)*trc_wice(:,1)
         ENDIF
         scv = scv + excess
         snowdp = snowdp + added_depth
         IF (snowdp >= 0.01_r8) THEN
            snl = -1
            zi(0) = 0._r8
            dz(0) = snowdp
            z(0) = -0.5_r8*dz(0)
            zi(-1) = -dz(0)
            t(0) = t(1)
            wice(0) = scv
            wliq(0) = 0._r8
            fiold(0) = 1._r8
            imelt(0) = 0
            snofrz(0) = 0._r8
            IF (DEF_USE_SNICAR) THEN
               snw_rds(0) = 54.526_r8
               mss_bcpho(0) = 0._r8; mss_bcphi(0) = 0._r8
               mss_ocpho(0) = 0._r8; mss_ocphi(0) = 0._r8
               mss_dst1(0) = 0._r8; mss_dst2(0) = 0._r8
               mss_dst3(0) = 0._r8; mss_dst4(0) = 0._r8
            ENDIF
         ENDIF
      ELSE
         top = snl + 1
         IF (present(trc_wice)) THEN
            trc_wice(:,top) = trc_wice(:,top) + fraction*trc_wice(:,1)
            trc_wice(:,1) = (1._r8-fraction)*trc_wice(:,1)
         ENDIF
         heat_capacity = cpice*wice(top) + cpliq*wliq(top)
         t(top) = (heat_capacity*t(top) + cpice*excess*t(1)) / &
                  (heat_capacity + cpice*excess)
         wice(top) = wice(top) + excess
         dz(top) = dz(top) + added_depth
         z(top) = zi(top) - 0.5_r8*dz(top)
         zi(top-1) = zi(top) - dz(top)
         fiold(top) = wice(top)/(wice(top)+wliq(top))
         scv = scv + excess
         snowdp = snowdp + added_depth
      ENDIF
      wice(1) = wice(1) - excess
   END SUBROUTINE relocate_soil_frost_ice

END MODULE MOD_NewSnow
! ---------- EOP ------------
