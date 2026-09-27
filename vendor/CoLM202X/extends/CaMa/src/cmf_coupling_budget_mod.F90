! Finite-volume exchange: routing stores m3, never regular-grid water depths.
MODULE CMF_COUPLING_BUDGET_MOD
USE, INTRINSIC :: ISO_FORTRAN_ENV, ONLY: real64
USE, INTRINSIC :: IEEE_ARITHMETIC, ONLY: IEEE_IS_FINITE
#ifdef SinglePrec_CMF
USE PARKIND1, ONLY: JPRB
#endif
IMPLICIT NONE
PRIVATE
PUBLIC :: budget_init, budget_publish, budget_runoff, budget_debit, sinks_prepaid
PUBLIC :: budget_set_strict, budget_unrouted
! The budget itself is always double precision.  When CaMa-Flood is built in
! single precision (SinglePrec_CMF) its prognostic arrays are REAL(JPRB), so the
! three routines that touch them get a JPRB specific that converts at the edge.
INTERFACE budget_publish
  MODULE PROCEDURE budget_publish_r8
#ifdef SinglePrec_CMF
  MODULE PROCEDURE budget_publish_rb
#endif
END INTERFACE
INTERFACE budget_runoff
  MODULE PROCEDURE budget_runoff_r8
#ifdef SinglePrec_CMF
  MODULE PROCEDURE budget_runoff_rb
#endif
END INTERFACE
INTERFACE budget_debit
  MODULE PROCEDURE budget_debit_r8
#ifdef SinglePrec_CMF
  MODULE PROCEDURE budget_debit_rb
#endif
END INTERFACE
LOGICAL :: sinks_prepaid = .FALSE.
! Land runoff over a grid cell that no routing cell receives cannot be routed.
! strict: stop; otherwise it is dropped and reported by budget_unrouted.
LOGICAL :: strict_recipients = .TRUE.
REAL(real64) :: unrouted = 0
INTEGER, ALLOCATABLE :: gx(:,:), gy(:,:)
REAL(real64), ALLOCATABLE :: weight(:,:), rowsum(:), colsum(:,:), gridarea(:,:), credit(:), gridcredit(:,:)
CONTAINS

SUBROUTINE budget_set_strict(strict)
LOGICAL, INTENT(IN) :: strict
strict_recipients = strict
END SUBROUTINE

FUNCTION budget_unrouted() RESULT(flux)
! Runoff [m3/s] that the last budget_runoff call could not route (cells without a recipient).
REAL(real64) :: flux
flux = unrouted
END FUNCTION

SUBROUTINE budget_init(x,y,w,area)
INTEGER, INTENT(IN) :: x(:,:), y(:,:)
REAL(real64), INTENT(IN) :: w(:,:), area(:,:)
INTEGER :: c,k,i,j
IF (ANY(.NOT.IEEE_IS_FINITE(w)) .OR. ANY(w<0)) ERROR STOP 'CaMa: invalid exchange weights'
IF (ANY(.NOT.IEEE_IS_FINITE(area)) .OR. ANY(area<=0)) ERROR STOP 'CaMa: invalid geometric grid area'
IF (ALLOCATED(gx)) DEALLOCATE(gx,gy,weight,rowsum,colsum,gridarea,credit,gridcredit)
gx=x; gy=y; weight=w; gridarea=area
rowsum=SUM(w,DIM=2)
ALLOCATE(colsum(SIZE(area,1),SIZE(area,2)),gridcredit(SIZE(area,1),SIZE(area,2)),credit(SIZE(x,1)))
colsum=0; gridcredit=0; credit=0
DO k=1,SIZE(x,2)
  DO c=1,SIZE(x,1)
    IF(w(c,k)<=0) CYCLE
    i=x(c,k); j=y(c,k)
    IF(i<1.OR.i>SIZE(area,1).OR.j<1.OR.j>SIZE(area,2)) ERROR STOP 'CaMa: exchange index out of range'
    colsum(i,j)=colsum(i,j)+w(c,k)
  ENDDO
ENDDO
END SUBROUTINE

SUBROUTINE budget_publish_r8(volume,floodarea,depth,fraction)
REAL(real64), INTENT(IN) :: volume(:),floodarea(:)
REAL(real64), INTENT(OUT) :: depth(:,:),fraction(:,:)
INTEGER :: c,k,i,j
REAL(real64) :: a
IF(ANY(.NOT.IEEE_IS_FINITE(volume)).OR.ANY(volume<0)) ERROR STOP 'CaMa: invalid flood storage'
IF(ANY(.NOT.IEEE_IS_FINITE(floodarea)).OR.ANY(floodarea<0)) ERROR STOP 'CaMa: invalid flood area'
! The matrix is an allocation geometry, not a replacement for catchment area.
! Unmapped donors retain their water in CaMa and issue no land credit.
credit=volume; gridcredit=0; fraction=0
DO k=1,SIZE(gx,2)
  DO c=1,SIZE(gx,1)
    IF(weight(c,k)<=0) CYCLE
    i=gx(c,k); j=gy(c,k); a=weight(c,k)/rowsum(c)
    gridcredit(i,j)=gridcredit(i,j)+volume(c)*a
    fraction(i,j)=fraction(i,j)+floodarea(c)*a
  ENDDO
ENDDO
depth=gridcredit/gridarea
fraction=MIN(1._real64,fraction/gridarea)
END SUBROUTINE

SUBROUTINE budget_runoff_r8(gridflow,flow)
! gridflow is the integrated flux from covered land ONLY [m3/s].
REAL(real64), INTENT(IN) :: gridflow(:,:)
REAL(real64), INTENT(OUT) :: flow(:)
INTEGER :: c,k,i,j
IF(ANY(.NOT.IEEE_IS_FINITE(gridflow))) ERROR STOP 'CaMa: nonfinite runoff'
IF(strict_recipients .AND. ANY(ABS(gridflow)>1.e-12_real64 .AND. colsum<=0)) ERROR STOP 'CaMa: runoff has no routing recipient'
unrouted = SUM(gridflow, MASK=(colsum<=0))
flow=0
DO k=1,SIZE(gx,2)
  DO c=1,SIZE(gx,1)
    IF(weight(c,k)<=0) CYCLE
    i=gx(c,k); j=gy(c,k)
    flow(c)=flow(c)+gridflow(i,j)*weight(c,k)/colsum(i,j)
  ENDDO
ENDDO
END SUBROUTINE

SUBROUTINE budget_debit_r8(evap,infil,storage,evap_used,infil_used)
! Return each withdrawal to the donors that actually supplied that grid.
! Debit BEFORE routing so accepted land sinks cannot be clipped after export.
REAL(real64), INTENT(IN) :: evap(:,:),infil(:,:)
REAL(real64), INTENT(INOUT) :: storage(:)
REAL(real64), INTENT(OUT) :: evap_used(:),infil_used(:)
INTEGER :: c,k,i,j
REAL(real64) :: share
IF(ANY(.NOT.IEEE_IS_FINITE(evap)).OR.ANY(.NOT.IEEE_IS_FINITE(infil))) ERROR STOP 'CaMa: nonfinite land sink'
IF(ANY(evap<0).OR.ANY(infil<0)) ERROR STOP 'CaMa: negative land sink'
IF(ANY(evap+infil>gridcredit+1.e-8_real64*MAX(1._real64,gridcredit))) &
  ERROR STOP 'CaMa: land exceeded its published grid credit'
evap_used=0; infil_used=0
DO k=1,SIZE(gx,2)
  DO c=1,SIZE(gx,1)
    IF(weight(c,k)<=0) CYCLE
    i=gx(c,k); j=gy(c,k)
    IF(gridcredit(i,j)<=0) CYCLE
    share=credit(c)*(weight(c,k)/rowsum(c))/gridcredit(i,j)
    evap_used(c)=evap_used(c)+evap(i,j)*share
    infil_used(c)=infil_used(c)+infil(i,j)*share
  ENDDO
ENDDO
IF(ANY(evap_used+infil_used>storage+1.e-8_real64*MAX(1._real64,storage))) &
  ERROR STOP 'CaMa: donor storage changed while land credit was outstanding'
storage=MAX(0._real64,storage-evap_used-infil_used)
END SUBROUTINE

#ifdef SinglePrec_CMF
SUBROUTINE budget_publish_rb(volume,floodarea,depth,fraction)
REAL(JPRB), INTENT(IN) :: volume(:),floodarea(:)
REAL(real64), INTENT(OUT) :: depth(:,:),fraction(:,:)
CALL budget_publish_r8(REAL(volume,real64),REAL(floodarea,real64),depth,fraction)
END SUBROUTINE

SUBROUTINE budget_runoff_rb(gridflow,flow)
REAL(real64), INTENT(IN) :: gridflow(:,:)
REAL(JPRB), INTENT(OUT) :: flow(:)
REAL(real64), ALLOCATABLE :: flow64(:)
ALLOCATE(flow64(SIZE(flow)))
CALL budget_runoff_r8(gridflow,flow64)
flow=REAL(flow64,JPRB)
END SUBROUTINE

SUBROUTINE budget_debit_rb(evap,infil,storage,evap_used,infil_used)
REAL(real64), INTENT(IN) :: evap(:,:),infil(:,:)
REAL(JPRB), INTENT(INOUT) :: storage(:)
REAL(real64), INTENT(OUT) :: evap_used(:),infil_used(:)
REAL(real64), ALLOCATABLE :: storage64(:)
storage64=REAL(storage,real64)
CALL budget_debit_r8(evap,infil,storage64,evap_used,infil_used)
storage=REAL(storage64,JPRB)
END SUBROUTINE
#endif
END MODULE
