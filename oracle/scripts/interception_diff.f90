! `MOD_LeafInterception:LEAF_interception_CoLM2014` 的随机差分驱动（配对物
! `crates/colm-core/examples/interception_probe.rs`）。
!
! 为什么单列：第 326 轮修好 `p0`/`pinf`/`:328-329` 三处之后，湿窗 `over_tol` 1907→1287，
! 但雪窗 step 0 逐位全同、step 1 起仍有 1 ULP 的冠层水/蒸发差异 —— 剩下的 12 条 FMA
! （`ap`/`cp`/`aa1`/`bb1`/`drainage`/`qintr_snow`/`tex_snow` 等）只影响
! `ldew_rain`/`ldew_snow` 这些**分量**，history 不导出它们，所以只能建闭环逐位判。
!
! 上游侧用内核同款选项编译（**不加** `-ffp-contract=off`），驱动本身按仓库纪律加
! `-fwrapv -ffp-contract=off`。`LEAF_interception_CoLM2014` 在模块里是 PUBLIC，
! 所以不需要"拷贝+放行"。
!
! **抽签次数与顺序必须与 Rust 侧逐条对齐**，改一边就得同步改另一边。
! `DEF_VEG_SNOW`（`MOD_Namelist` 里的模块变量，默认 `.true.`，本例程 `USE` 它）
! 在 k=0/1 两档各跑一遍。
!
! 输出：`k` 后 8 列十六进制 —— `ldew`/`ldew_rain`/`ldew_snow`（更新后）、
! `pg_rain`/`pg_snow`/`qintr`/`qintr_rain`/`qintr_snow`。
PROGRAM icp
  USE MOD_LeafInterception, only: LEAF_interception_CoLM2014
  USE MOD_Namelist, only: DEF_VEG_SNOW
  IMPLICIT NONE
  INTEGER, PARAMETER :: r8 = 8
  INTEGER(8) :: S
  INTEGER :: i, k, nlen
  REAL(r8) :: deltim, dewmx, forc_us, forc_vs, chil, sigf, lai, sai, tair, tleaf
  REAL(r8) :: prc_rain, prc_snow, prl_rain, prl_snow, irrig, bifall
  REAL(r8) :: ldew, ldew_rain, ldew_snow, z0m, hu
  REAL(r8) :: pg_rain, pg_snow, qintr, qintr_rain, qintr_snow
  REAL(r8) :: r1, r2
  CHARACTER(LEN=256) :: dir

  S = 20260924_8
  CALL GET_ENVIRONMENT_VARIABLE('ICP_OUT', dir, nlen)
  IF (nlen <= 0) dir = '/tmp/gf/icp'
  OPEN(66, FILE=TRIM(dir)//'/icp.txt', STATUS='REPLACE')
  DO k = 0, 1
     DEF_VEG_SNOW = (k == 1)
     DO i = 1, 2000
        deltim  = 60.0_r8 + uni()*3540.0_r8
        dewmx   = 0.05_r8 + uni()*0.45_r8
        forc_us = (uni() - 0.5_r8)*20.0_r8
        forc_vs = (uni() - 0.5_r8)*20.0_r8
        chil    = -1.0_r8 + uni()*2.0_r8
        sigf    = uni()
        IF (uni() < 0.1_r8) THEN
           lai = 0.0_r8
           sai = 0.0_r8
        ELSE
           lai = uni()*6.0_r8
           sai = uni()*2.0_r8
        ENDIF
        tair    = 250.0_r8 + uni()*60.0_r8
        tleaf   = 250.0_r8 + uni()*60.0_r8
        prc_rain  = uni2()*2.0e-4_r8
        prc_snow  = uni2()*2.0e-4_r8
        prl_rain  = uni2()*2.0e-4_r8
        prl_snow  = uni2()*2.0e-4_r8
        irrig     = uni()*1.0e-5_r8
        bifall  = 50.0_r8 + uni()*300.0_r8
        ldew_rain = uni()*0.3_r8
        ldew_snow = uni()*0.3_r8
        IF (uni() < 0.5_r8) THEN
           ldew = ldew_rain + ldew_snow
        ELSE
           ldew = uni()*0.6_r8
        ENDIF
        z0m = 0.01_r8 + uni()*1.0_r8
        hu  = 1.0_r8 + uni()*40.0_r8
        CALL LEAF_interception_CoLM2014 (deltim, dewmx, forc_us, forc_vs, chil, sigf, &
             lai, sai, tair, tleaf, prc_rain, prc_snow, prl_rain, prl_snow, irrig, bifall, &
             ldew, ldew_rain, ldew_snow, z0m, hu, pg_rain, pg_snow, qintr, qintr_rain, qintr_snow)
        WRITE(66,'(I2,1X,8(1X,Z17))') k, B(ldew), B(ldew_rain), B(ldew_snow), &
             B(pg_rain), B(pg_snow), B(qintr), B(qintr_rain), B(qintr_snow)
     ENDDO
  ENDDO
  CLOSE(66)

CONTAINS
  REAL(r8) FUNCTION uni() RESULT(v)
    S = S*6364136223846793005_8 + 1442695040888963407_8
    v = REAL(ISHFT(S, -11), r8)/9007199254740992.0_r8
  END FUNCTION uni

  ! `uni()*uni()` 的两个乘法顺序在 Fortran 里没有规定，显式画到临时量里，
  ! 保证与 Rust 侧"先抽 a、再抽 b、然后相乘"逐位一致。
  REAL(r8) FUNCTION uni2() RESULT(v)
    REAL(r8) :: a, b
    a = uni()
    b = uni()
    v = a*b
  END FUNCTION uni2

  INTEGER(8) FUNCTION B(x) RESULT(bits)
    REAL(r8), INTENT(IN) :: x
    bits = TRANSFER(x, 0_8)
  END FUNCTION B
END PROGRAM icp
