! MOD_Qsadv:qsadv 的随机差分驱动（配对物 crates/colm-core/examples/qsadv_probe.rs）。
PROGRAM qsd
  USE MOD_Qsadv, only: qsadv
  IMPLICIT NONE
  INTEGER, PARAMETER :: r8 = 8
  INTEGER(8) :: S
  REAL(r8) :: T,p,es,esdT,qs,qsdT
  INTEGER :: i
  S = 20250506_8
  OPEN(66, FILE='/tmp/gf/qs_diff/qs.txt', STATUS='REPLACE')
  DO i = 1, 20000
     T = 150.0_r8 + uni()*200.0_r8
     p = 30000.0_r8 + uni()*80000.0_r8
     CALL qsadv(T,p,es,esdT,qs,qsdT)
     WRITE(66,'(6Z17)') B(T),B(p),B(es),B(esdT),B(qs),B(qsdT)
  ENDDO
  CLOSE(66)
  PRINT *, 'done'
CONTAINS
  FUNCTION uni() RESULT(v)
    REAL(r8) :: v
    S = S*6364136223846793005_8 + 1442695040888963407_8
    v = REAL(ISHFT(S,-11), r8)/9007199254740992.0_r8
  END FUNCTION uni
  FUNCTION B(x) RESULT(h)
    REAL(r8), INTENT(IN) :: x
    INTEGER(8) :: h
    h = TRANSFER(x,h)
  END FUNCTION B
END PROGRAM qsd
