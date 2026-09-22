! MOD_ForcingDownscaling:downscale_wind / downscale_wind_simple 的随机差分驱动。
! 配对物 crates/colm-core/examples/forcing_downscaling_probe.rs；两侧共用同一串 LCG。
!
! 本驱动**不自己编译模块**，而是直接链接 `.bld` 下的全部 `.o`（内核真实
! 产线对象，已带 GCC 默认的 FMA 收缩；只有 `CoLM.o` 那个 PROGRAM 被排除）。
! 因此这里跑的是真正的内核代码，连 namelist 都是真的 —— 不需要桩模块。
!
! 行首两位诊断位（只在上游侧算，用于给失配分组）：
!   f0 = 1 表示 us 恰好取到 0（`wind_dir = PI/2` 那条支）
!   f1 = 1 表示 cur_c = -1e36（缺失值支）
PROGRAM fdwind
  USE MOD_ForcingDownscaling, only: downscale_wind, downscale_wind_simple
  IMPLICIT NONE
  INTEGER, PARAMETER :: r8=8
  INTEGER(8) :: S
  INTEGER :: i, k, f0, f1, nlen
  CHARACTER(LEN=256) :: dir
  REAL(r8) :: us, vs, cur, usf, vsf, uss, vss
  REAL(r8) :: slpf(4), aspf(4), areaf(4), slps(9), areas(9)
  S = 20250509_8
  CALL GET_ENVIRONMENT_VARIABLE('FD_OUT', dir, nlen)
  IF (nlen <= 0) dir = '/tmp/gf/fd_diff'
  OPEN(66, FILE=TRIM(dir)//'/fdwind.txt', STATUS='REPLACE')
  DO i = 1, 20000
     us = -10._r8 + uni()*20._r8
     IF (MOD(i,37) == 0) us = 0._r8
     vs = -10._r8 + uni()*20._r8
     cur = -2._r8 + uni()*4._r8
     IF (MOD(i,17) == 0) cur = -1.0e36_r8
     DO k = 1, 4
        slpf(k) = uni()*1.5_r8
        aspf(k) = uni()*6.28318_r8
        areaf(k) = uni()
     ENDDO
     DO k = 1, 9
        slps(k) = uni()*1.5_r8
        areas(k) = uni()
     ENDDO
     IF (MOD(i,11) == 0) slps(1+MOD(i,9)) = -1.0e36_r8
     IF (MOD(i,13) == 0) areas(1+MOD(i,9)) = -1.0e36
     usf = us
     vsf = vs
     CALL downscale_wind(usf, vsf, slpf, aspf, areaf, cur)
     uss = us
     vss = vs
     CALL downscale_wind_simple(uss, vss, slps, areas, cur)
     f0 = 0
     IF (us == 0._r8) f0 = 1
     f1 = 0
     IF (cur == -1.0e36_r8) f1 = 1
     WRITE(66,'(I1,I1,1X,4Z17)') f0, f1, B(usf), B(vsf), B(uss), B(vss)
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
END PROGRAM fdwind
