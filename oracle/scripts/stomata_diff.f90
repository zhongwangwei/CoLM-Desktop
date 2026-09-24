! `MOD_AssimStomataConductance:stomata` 的随机差分驱动（配对物
! `crates/colm-core/examples/stomata_probe.rs`）。
!
! 为什么单列：第 329 轮普查发现这条链的 `stomata` 有 **17 条 FMA**、
! `update_photosyn` 有 8 条，而本仓库 `photosynthesis.rs` 的 `mul_add` 是 **0** ——
! 雪窗 step-1 的 `f_rstfacsha/sun`/`f_gssun/sha` 差 1 ULP 的种子就在这里。
!
! 上游侧用内核同款选项编译（**不加** `-ffp-contract=off`），驱动本身按仓库纪律加
! `-fwrapv -ffp-contract=off`。`stomata` 在模块里是 PUBLIC。
!
! **抽签次数与顺序必须与 Rust 侧逐条对齐**，改一边就得同步改另一边。
! 三种气孔模型各跑 1000 例（`MOD_Namelist` 的模块开关）：
!   k=0 Ball-Berry（两个开关都 .false.）、k=1 Medlyn、k=2 WUE、
!   k=3 WUE + **黄金窗口的真实参数区间**（c3c4=1、par≤800、tlef∈[275,315]、rstfac∈[0.3,1]）
! 覆盖值全部设成哨兵 `-1`，让实参里的 `g1/g0/gradm/binter/lambda` 生效。
!
! 输出：`k` 后 3 列十六进制 —— `assim`/`respc`/`rst`。
PROGRAM stm
  USE MOD_AssimStomataConductance, only: stomata
  USE MOD_Namelist, only: DEF_USE_MEDLYNST, DEF_USE_WUEST, DEF_MEDLYN_G1, &
       DEF_MEDLYN_G0, DEF_WUE_LAMBDA, DEF_BALL_BERRY_GRADM, DEF_BALL_BERRY_BINTER
  IMPLICIT NONE
  INTEGER, PARAMETER :: r8 = 8
  INTEGER(8) :: S
  INTEGER :: i, k, nlen
  INTEGER :: c3c4
  REAL(r8) :: vmax25, effcon, slti, hlti, shti, hhti, trda, trdm, trop
  REAL(r8) :: g1, g0, gradm, binter, tm, psrf, po2m, pco2m, pco2a, ea, ei, tlef, par
  REAL(r8) :: o3coefv, o3coefg, lambda, rb, ra, rstfac
  REAL(r8) :: cint(3), assim, respc, rst
  CHARACTER(LEN=256) :: dir

  S = 20260924_8
  CALL GET_ENVIRONMENT_VARIABLE('STM_OUT', dir, nlen)
  IF (nlen <= 0) dir = '/tmp/gf/stm'
  OPEN(66, FILE=TRIM(dir)//'/stm.txt', STATUS='REPLACE')
  DEF_MEDLYN_G1 = -1.0_r8
  DEF_MEDLYN_G0 = -1.0_r8
  DEF_WUE_LAMBDA = -1.0_r8
  DEF_BALL_BERRY_GRADM = -1.0_r8
  DEF_BALL_BERRY_BINTER = -1.0_r8
  DO k = 0, 3
     DEF_USE_MEDLYNST = (k == 1)
     DEF_USE_WUEST = (k == 2) .OR. (k == 3)
     DO i = 1, 1000
        vmax25 = 10.0_r8 + uni()*80.0_r8
        effcon = 0.01_r8 + uni()*0.08_r8
        c3c4 = MERGE(1, 0, uni() < 0.5_r8)
        IF (k >= 2) c3c4 = 1
        slti = 0.1_r8 + uni()*0.3_r8
        hlti = 280.0_r8 + uni()*20.0_r8
        shti = 0.2_r8 + uni()*0.3_r8
        hhti = 300.0_r8 + uni()*30.0_r8
        trda = 1.0_r8 + uni()*0.5_r8
        trdm = 320.0_r8 + uni()*20.0_r8
        trop = 298.16_r8
        g1 = 1.0_r8 + uni()*10.0_r8
        g0 = uni()*0.1_r8
        gradm = 5.0_r8 + uni()*15.0_r8
        binter = uni()*0.05_r8
        tm = 250.0_r8 + uni()*60.0_r8
        psrf = 60000.0_r8 + uni()*50000.0_r8
        po2m = psrf*0.209_r8
        pco2m = 30.0_r8 + uni()*60.0_r8
        pco2a = pco2m*(0.9_r8 + uni()*0.15_r8)
        ea = uni()*2000.0_r8
        ei = 100.0_r8 + uni()*4000.0_r8
        ! k=3：按黄金窗口（CN-Cng-wet，C3 + 默认 WUE）的真实区间取一档。
        tlef = MERGE(275.0_r8, 250.0_r8, k == 3) + uni()*MERGE(40.0_r8, 60.0_r8, k == 3)
        par = uni()*MERGE(800.0_r8, 300.0_r8, k == 3)
        o3coefv = uni()
        o3coefg = uni()
        lambda = 100.0_r8 + uni()*5000.0_r8
        rb = 10.0_r8 + uni()*200.0_r8
        ra = 10.0_r8 + uni()*200.0_r8
        rstfac = MERGE(0.3_r8, 0.0_r8, k == 3) + uni()*MERGE(0.7_r8, 1.0_r8, k == 3)
        cint(1) = uni()
        cint(2) = uni()
        cint(3) = uni()
        CALL stomata (vmax25, effcon, c3c4, slti, hlti, shti, hhti, trda, trdm, trop, &
             g1, g0, gradm, binter, tm, psrf, po2m, pco2m, pco2a, ea, ei, tlef, par, &
             o3coefv, o3coefg, lambda, rb, ra, rstfac, cint, assim, respc, rst)
        WRITE(66,'(I2,1X,3(1X,Z17))') k, B(assim), B(respc), B(rst)
     ENDDO
  ENDDO
  CLOSE(66)

CONTAINS
  REAL(r8) FUNCTION uni() RESULT(v)
    S = S*6364136223846793005_8 + 1442695040888963407_8
    v = REAL(ISHFT(S, -11), r8)/9007199254740992.0_r8
  END FUNCTION uni

  INTEGER(8) FUNCTION B(x) RESULT(bits)
    REAL(r8), INTENT(IN) :: x
    bits = TRANSFER(x, 0_8)
  END FUNCTION B
END PROGRAM stm
