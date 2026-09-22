! MOD_ForcingDownscaling:downscale_forcings 的 full 支（带可选实参）随机差分驱动。
! 配对物 crates/colm-core/examples/forcing_downscaling_shortwave_probe.rs；两侧共用
! 同一串 LCG，抽签次数与顺序必须逐条对齐。
!
! 给出 `area_type_c`/`svf_c`/`alb`/`sf_*_c` 四个可选实参之后，`downscale_forcings`
! 走 `downscale_shortwave`（地形/天空视域/反射那一整套），顺带把"present() 选支"这条
! 装配逻辑也验了。
!
! 阴影表有**两套**，由编译开关决定，必须与链接进来的模块对象一致：
!   - 不定义 SinglePoint（网格内核 `.bld` 就是这一档）：`sf_curve_c(16,3)`，48 个值；
!   - 定义 SinglePoint（单点内核）：`sf_lut_c(16,101)`，1616 个值。
! 两套都由 LCG 抽，各自一条流；探针用 `FD_VARIANT` 选同样的一套。
!
! 行首三位诊断位（只在上游侧算，用于给失配分组）：
!   f0 = alb 是 NaN（缺测反照率支）
!   f1 = svf 越界（>1 或 <0）
!   f2 = coszen 恰好 0（`toa_swrad.eq.0` 那条支）
PROGRAM fdsw
  USE MOD_ForcingDownscaling, only: downscale_forcings
  USE MOD_Namelist, only: DEF_DS_TEMP_LAPSE_RATE, DEF_DS_LONGWAVE_LAPSE_RATE, &
       DEF_DS_LONGWAVE_LIMIT, DEF_DS_SHORTWAVE_LIMIT, DEF_DS_SHORTWAVE_SIMPLE_LIMIT, &
       DEF_DS_precipitation_adjust_scheme, DEF_DS_longwave_adjust_scheme
  USE, INTRINSIC :: ieee_arithmetic, only: ieee_value, ieee_quiet_nan
  IMPLICIT NONE
  INTEGER, PARAMETER :: r8=8, NAZ=16
#ifdef SinglePoint
  INTEGER, PARAMETER :: NSHADOW=101
#else
  INTEGER, PARAMETER :: NSHADOW=3
#endif
  INTEGER(8) :: S
  INTEGER :: i, ia, iz, k, f0, f1, f2, nlen
  LOGICAL :: glaciers
  CHARACTER(LEN=256) :: dir
  REAL(r8) :: tpg, mxg, tg, thg, qg, pbotg, rhog, prcg, prlg, lwg, hgtg, swg, usg, vsg
  REAL(r8) :: tpc, cur, julian, coszen, cosazi
  REAL(r8) :: slp(4), asp(4), area(4), svf, alb, sf_tab(NAZ, NSHADOW)
  REAL(r8) :: tc, thc, qc, pbotc, rhoc, prcc, prlc, lwc, swc, usc, vsc
  S = 20250511_8
  CALL GET_ENVIRONMENT_VARIABLE('FD_OUT', dir, nlen)
  IF (nlen <= 0) dir = '/tmp/gf/fd_diff'
  OPEN(66, FILE=TRIM(dir)//'/fdsw.txt', STATUS='REPLACE')
  DEF_DS_precipitation_adjust_scheme = 'I'
  DEF_DS_longwave_adjust_scheme = 'II'
  DO i = 1, 2000
     tpg = uni()*3000._r8
     mxg = uni()*3000._r8
     tg = 250._r8 + uni()*70._r8
     thg = 250._r8 + uni()*80._r8
     qg = 0.0001_r8 + uni()*0.02_r8
     pbotg = 60000._r8 + uni()*41325._r8
     rhog = 0.7_r8 + uni()*0.7_r8
     prcg = uni()*0.001_r8
     prlg = uni()*0.001_r8
     lwg = 100._r8 + uni()*300._r8
     hgtg = 10._r8 + uni()*90._r8
     swg = uni()*1000._r8
     IF (MOD(i,7) == 0) swg = 0._r8
     IF (MOD(i,13) == 0) swg = uni()*1.0e-5_r8
     usg = -10._r8 + uni()*20._r8
     vsg = -10._r8 + uni()*20._r8
     tpc = uni()*3000._r8
     julian = 1._r8 + uni()*364._r8
     coszen = 0.05_r8 + uni()*0.95_r8
     IF (MOD(i,11) == 0) coszen = 0._r8
     cosazi = -1._r8 + uni()*2._r8
     cur = -2._r8 + uni()*2._r8
     DO k = 1, 4
        slp(k) = uni()*1.5_r8
        asp(k) = uni()*6.28318_r8
        area(k) = uni()
     ENDDO
     DEF_DS_TEMP_LAPSE_RATE = uni()*0.01_r8
     DEF_DS_LONGWAVE_LAPSE_RATE = uni()*0.05_r8
     DEF_DS_LONGWAVE_LIMIT = uni()
     DEF_DS_SHORTWAVE_LIMIT = uni()
     DEF_DS_SHORTWAVE_SIMPLE_LIMIT = uni()
     svf = -0.2_r8 + uni()*1.4_r8
     alb = uni()
     f0 = 0
     IF (MOD(i,4) == 0) THEN
        alb = ieee_value(0.0_r8, ieee_quiet_nan)
        f0 = 1
     ENDIF
     DO iz = 1, NSHADOW
        DO ia = 1, NAZ
           sf_tab(ia,iz) = uni()
        ENDDO
     ENDDO
     glaciers = (MOD(i,3) == 0)
     f1 = 0
     IF ((svf > 1._r8) .or. (svf < 0._r8)) f1 = 1
     f2 = 0
     IF (coszen == 0._r8) f2 = 1
     CALL downscale_forcings (glaciers, &
          tpg, mxg, tg, thg, qg, pbotg, rhog, prcg, prlg, lwg, hgtg, swg, usg, vsg, &
          slp, asp, cur, julian, coszen, cosazi, &
          tpc, tc, thc, qc, pbotc, rhoc, prcc, prlc, lwc, swc, usc, vsc, &
          area, svf, alb, sf_tab)
     WRITE(66,'(I1,I1,I1,1X,11Z17)') f0, f1, f2, &
          B(tc), B(thc), B(qc), B(pbotc), B(rhoc), B(prcc), B(prlc), &
          B(lwc), B(swc), B(usc), B(vsc)
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
END PROGRAM fdsw
