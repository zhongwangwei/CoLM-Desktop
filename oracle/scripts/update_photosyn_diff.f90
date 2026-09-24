! `MOD_AssimStomataConductance:update_photosyn` 的随机差分驱动（配对物
! `crates/colm-core/examples/update_photosyn_probe.rs`）。
!
! 为什么单列：第 329 轮普查把这条链的 FMA 分成两段 —— `stomata` 17 条、
! `update_photosyn` 8 条。`stomata` 的闭环（`compare_stomata.sh`）已经建起来，
! 但没有一条闭环覆盖 `update_photosyn`，于是"改了 shape 到底对不对"只能靠黄金
! 湿窗口反推。这个驱动补上那一段：它同时覆盖 `calc_photo_params`
! （PRIVATE，但由 `update_photosyn` 内部调用）、`sortin` 的 6 次迭代、
! `coupled_assimilation` 的二次耦合，以及 `:223` 的 `range`。
!
! 上游侧用内核同款选项编译（**不加** `-ffp-contract=off`），驱动本身按仓库纪律加
! `-fwrapv -ffp-contract=off`。`update_photosyn` 在模块里是 PUBLIC，直接链 `.bld`。
!
! **抽签次数与顺序必须与 Rust 侧逐条对齐**，改一边就得同步改另一边。
! 三个模型块各 1000 例（`MOD_Namelist` 的模块开关）：
!   k=0 WUE + 黄金窗口真实区间（c3c4=1、par≤800、tlef∈[275,315]、rstfac∈[0.3,1]）
!   k=1 WUE、c3c4 随机（同时走 `min(omc,ome)` 与二次耦合两支）
!   k=2 Ball-Berry（WUE 关）⇒ 恒走 `coupled_assimilation` 的判别式分支
! 覆盖值全部设成哨兵 `-1`，让实参里的 gradm 生效。
!
! 输出：`k` 后 2 列十六进制 —— `assim`/`respc`。
PROGRAM upsyn
  USE MOD_AssimStomataConductance, only: update_photosyn
  USE MOD_Namelist, only: DEF_USE_MEDLYNST, DEF_USE_WUEST, DEF_MEDLYN_G1, &
       DEF_MEDLYN_G0, DEF_WUE_LAMBDA, DEF_BALL_BERRY_GRADM, DEF_BALL_BERRY_BINTER
  IMPLICIT NONE
  INTEGER, PARAMETER :: r8 = 8
  INTEGER(8) :: S
  INTEGER :: i, k, nlen
  INTEGER :: c3c4
  REAL(r8) :: vmax25, effcon, slti, hlti, shti, hhti, trda, trdm, trop
  REAL(r8) :: g1, g0, gradm, binter, psrf, po2m, pco2m, pco2a, ea, ei, tlef, par
  REAL(r8) :: lambda, rb, rstfac, gsh2o
  REAL(r8) :: cint(3), assim, respc
  CHARACTER(LEN=256) :: dir

  S = 20260925_8
  CALL GET_ENVIRONMENT_VARIABLE('UPSYN_OUT', dir, nlen)
  IF (nlen <= 0) dir = '/tmp/gf/upsyn'
  OPEN(66, FILE=TRIM(dir)//'/upsyn.txt', STATUS='REPLACE')
  DEF_MEDLYN_G1 = -1.0_r8
  DEF_MEDLYN_G0 = -1.0_r8
  DEF_WUE_LAMBDA = -1.0_r8
  DEF_BALL_BERRY_GRADM = -1.0_r8
  DEF_BALL_BERRY_BINTER = -1.0_r8
  DO k = 0, 2
     DEF_USE_MEDLYNST = .FALSE.
     DEF_USE_WUEST = (k /= 2)
     DO i = 1, 1000
        vmax25 = 10.0_r8 + uni()*80.0_r8
        effcon = 0.01_r8 + uni()*0.08_r8
        c3c4 = MERGE(1, 0, uni() < 0.5_r8)
        IF (k == 0) c3c4 = 1
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
        psrf = 60000.0_r8 + uni()*50000.0_r8
        po2m = psrf*0.209_r8
        pco2m = 30.0_r8 + uni()*60.0_r8
        pco2a = pco2m*(0.9_r8 + uni()*0.15_r8)
        ea = uni()*2000.0_r8
        ei = 100.0_r8 + uni()*4000.0_r8
        ! k=0：按黄金窗口（CN-Cng-wet，C3 + 默认 WUE）的真实区间取一档。
        tlef = MERGE(275.0_r8, 250.0_r8, k == 0) + uni()*MERGE(40.0_r8, 60.0_r8, k == 0)
        par = uni()*MERGE(800.0_r8, 300.0_r8, k == 0)
        lambda = 100.0_r8 + uni()*5000.0_r8
        rb = 10.0_r8 + uni()*200.0_r8
        rstfac = MERGE(0.3_r8, 0.0_r8, k == 0) + uni()*MERGE(0.7_r8, 1.0_r8, k == 0)
        cint(1) = uni()
        cint(2) = uni()
        cint(3) = uni()
        ! `gsh2o` 的数值口径抄调用点（`MOD_LeafTemperature_Extended.F90:908-911`：
        ! 逐叶 µmol 输出乘 laisun 后原样传入），不是哑元注释里的 "mol m-2 s-1"。
        gsh2o = 1.0_r8 + uni()*200.0_r8
        CALL update_photosyn (tlef, po2m, pco2m, pco2a, par, psrf, rstfac, rb, gsh2o, &
             effcon, vmax25, c3c4, gradm, trop, slti, hlti, shti, hhti, trda, trdm, cint, &
             assim, respc)
        WRITE(66,'(I2,1X,2(1X,Z17))') k, B(assim), B(respc)
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
END PROGRAM upsyn
