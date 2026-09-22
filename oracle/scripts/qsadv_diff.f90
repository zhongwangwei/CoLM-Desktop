! MOD_Qsadv:qsadv 的随机差分驱动（配对物 crates/colm-core/examples/qsadv_probe.rs）。
! T 区间可用 QS_TMIN/QS_TMAX 覆盖，便于按分支复现：
!   冷支（不触发钳位）  QS_TMIN=210 QS_TMAX=273
!   暖支（不触发钳位）  QS_TMIN=274 QS_TMAX=345
! 实测：两支各自 20000/20000 全同；默认的 [150,350] 全量区间里约 25% 失配，
! 那正好是 |td|>=75 被钳到 ±75 的比例 —— 差异只在钳位那一支。
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
     BLOCK
        CHARACTER(LEN=32) :: env
        REAL(r8) :: tmin, tmax
        tmin = 150.0_r8; tmax = 350.0_r8
        CALL GET_ENVIRONMENT_VARIABLE('QS_TMIN', env)
        IF (LEN_TRIM(env) > 0) READ(env,*) tmin
        CALL GET_ENVIRONMENT_VARIABLE('QS_TMAX', env)
        IF (LEN_TRIM(env) > 0) READ(env,*) tmax
        T = tmin + uni()*(tmax-tmin)
     END BLOCK
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
