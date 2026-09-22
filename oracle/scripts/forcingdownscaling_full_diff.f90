! MOD_ForcingDownscaling:downscale_forcings 的随机差分驱动（简单地形那一支）。
! 配对物 crates/colm-core/examples/forcing_downscaling_full_probe.rs；两侧共用同一串
! LCG，抽签次数与顺序必须逐条对齐。
!
! 与风场那个驱动一样，本驱动直接链接 `.bld` 下的内核产线对象，因此 `DEF_DS_*`
! 是**真的 namelist 变量**：这里逐组赋值，再调用。
!
! 只跑"不给可选实参"的调用形式（`downscale_forcings` 靠 `present()` 选支）：
! 于是走 simple shortwave；full 支（`sf_lut_c`/`svf_c`/`alb`）留给下一份驱动。
!
! 行首两位是配置标签（只在上游侧算，用于给失配分组）：
!   ip = 1/2 降水方案 I/II；il = 1/2 长波方案 I/II。
PROGRAM fdfull
  USE MOD_ForcingDownscaling, only: downscale_forcings
  USE MOD_Namelist, only: DEF_DS_TEMP_LAPSE_RATE, DEF_DS_LONGWAVE_LAPSE_RATE, &
       DEF_DS_LONGWAVE_LIMIT, DEF_DS_SHORTWAVE_LIMIT, DEF_DS_SHORTWAVE_SIMPLE_LIMIT, &
       DEF_DS_precipitation_adjust_scheme, DEF_DS_longwave_adjust_scheme
  IMPLICIT NONE
  INTEGER, PARAMETER :: r8=8
  INTEGER(8) :: S
  INTEGER :: i, ip, il, k, nlen
  LOGICAL :: glaciers
  CHARACTER(LEN=256) :: dir
  CHARACTER(LEN=5) :: psch(2), lsch(2)
  REAL(r8) :: tpg, mxg, tg, thg, qg, pbotg, rhog, prcg, prlg, lwg, hgtg, swg, usg, vsg
  REAL(r8) :: tpc, cur, julian, coszen, cosazi
  REAL(r8) :: slp(9), asp(9)
  REAL(r8) :: tc, thc, qc, pbotc, rhoc, prcc, prlc, lwc, swc, usc, vsc
  S = 20250510_8
  psch(1) = 'I';  psch(2) = 'II'
  lsch(1) = 'I';  lsch(2) = 'II'
  CALL GET_ENVIRONMENT_VARIABLE('FD_OUT', dir, nlen)
  IF (nlen <= 0) dir = '/tmp/gf/fd_diff'
  OPEN(66, FILE=TRIM(dir)//'/fdfull.txt', STATUS='REPLACE')
  DO i = 1, 5000
     tpg = uni()*3000._r8
     mxg = uni()*3000._r8
     IF (MOD(i,7) == 0) mxg = 0._r8
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
     usg = -10._r8 + uni()*20._r8
     vsg = -10._r8 + uni()*20._r8
     tpc = uni()*3000._r8
     julian = 1._r8 + uni()*364._r8
     coszen = 0.05_r8 + uni()*0.95_r8
     cosazi = -1._r8 + uni()*2._r8
     cur = -2._r8 + uni()*2._r8
     DO k = 1, 9
        slp(k) = uni()*1.5_r8
        asp(k) = uni()*6.28318_r8
     ENDDO
     DEF_DS_TEMP_LAPSE_RATE = uni()*0.01_r8
     DEF_DS_LONGWAVE_LAPSE_RATE = uni()*0.05_r8
     DEF_DS_LONGWAVE_LIMIT = uni()
     DEF_DS_SHORTWAVE_LIMIT = uni()
     DEF_DS_SHORTWAVE_SIMPLE_LIMIT = uni()
     glaciers = (MOD(i,3) == 0)
     DO ip = 1, 2
        DEF_DS_precipitation_adjust_scheme = psch(ip)
        DO il = 1, 2
           DEF_DS_longwave_adjust_scheme = lsch(il)
           CALL downscale_forcings (glaciers, &
                tpg, mxg, tg, thg, qg, pbotg, rhog, prcg, prlg, lwg, hgtg, swg, usg, vsg, &
                slp, asp, cur, julian, coszen, cosazi, &
                tpc, tc, thc, qc, pbotc, rhoc, prcc, prlc, lwc, swc, usc, vsc)
           WRITE(66,'(I1,I1,1X,11Z17)') ip, il, &
                B(tc), B(thc), B(qc), B(pbotc), B(rhoc), B(prcc), B(prlc), &
                B(lwc), B(swc), B(usc), B(vsc)
        ENDDO
     ENDDO
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
END PROGRAM fdfull
