! `MOD_Hydro_SoilWater:water_balance` 的随机差分驱动（配对物
! `crates/colm-core/examples/water_balance_probe.rs`）。
!
! 为什么单列：第 309 轮在这个例程的汇编里认出 5 条"和（差）与乘积"形状的 FMA，
! 第 317 轮把它们逐条对回源表达式（每处只有一个乘积，所以"哪个乘积进 FMA"
! 没有歧义，要小心的只是加数是谁）。这个闭环在合成输入上逐位判这 5 处 ——
! 纯函数、输入最好造、不进混沌窗口。
!
! **私有例程的编译路线**（第 311/312 轮查定并实测）：`water_balance` 在模块里是
! `PRIVATE`，所以把模块**拷进 `$WORK`** 并把那一行改成 `PUBLIC`，用拷贝编 `.mod`
! （`-I$WORK` 在 `-I.bld` 之前），vendor 源树不动。
!
! 上游侧用内核真实选项编译（**不加** `-ffp-contract=off`），驱动本身按仓库纪律加
! `-fwrapv -ffp-contract=off`（否则驱动自己造出来的输入就可能与 Rust 侧不同）。
!
! **抽签次数与顺序必须与 Rust 侧逐条对齐**，改一边就得同步改另一边。
!
! 六种情形（`k`）覆盖两个上边界 × 两个下边界 × 排水子分支，并在每种情形里把
! 非饱和层比例取遍 `{0, 0.3, 0.7, 1}`：
!   k=0 定水头上/定水头下          k=3 降雨上/排放下（waquifer==0, q(ub)>=0）
!   k=1 降雨上/定水头下            k=4 定水头上/排放下（waquifer!=0）
!   k=2 定水头上/排放下（wa==0）   k=5 降雨上/排放下（waquifer!=0）
!
! 输出：每例先写 `k` 与 `nlev`，再写 `nlev+2` 列十六进制 `blc(0:nlev+1)`，
! 末尾一列 `solvable`（1/0）。
PROGRAM wb
  USE MOD_Hydro_SoilWater, only: water_balance, BC_FIX_HEAD, BC_RAINFALL, BC_DRAINAGE
  IMPLICIT NONE
  INTEGER, PARAMETER :: r8 = 8
  INTEGER, PARAMETER :: MAXLEV = 6
  INTEGER(8) :: S
  INTEGER :: i, k, nl, j, ubc, lbc, nlen
  REAL(r8) :: zi(0:MAXLEV), dz(MAXLEV)
  REAL(r8) :: dt, dp, dp_m1, wa, wa_m1, tol, ubc_val, lbc_val, sat_prob
  REAL(r8) :: vls(MAXLEV), wf(MAXLEV), vl(MAXLEV), wt(MAXLEV)
  REAL(r8) :: wf_m1(MAXLEV), vl_m1(MAXLEV), wt_m1(MAXLEV), q(0:MAXLEV)
  LOGICAL :: is_sat(MAXLEV), solvable
  REAL(r8) :: blc(0:MAXLEV+1)
  CHARACTER(LEN=256) :: dir
  REAL(r8), PARAMETER :: SAT(4) = [0.0_r8, 0.3_r8, 0.7_r8, 1.0_r8]

  S = 20260924_8
  CALL GET_ENVIRONMENT_VARIABLE('WB_OUT', dir, nlen)
  IF (nlen <= 0) dir = '/tmp/gf/wb'
  OPEN(66, FILE=TRIM(dir)//'/wb.txt', STATUS='REPLACE')
  DO k = 0, 5
     ubc = BC_FIX_HEAD
     lbc = BC_FIX_HEAD
     IF ((k == 1) .OR. (k == 3) .OR. (k == 5)) ubc = BC_RAINFALL
     IF (k >= 2) lbc = BC_DRAINAGE
     sat_prob = SAT(MOD(k, 4) + 1)
     DO i = 1, 2000
        nl = 1 + INT(uni()*6.0_r8)
        ! 界面深度是原始量、`dz` 由相邻界面相减得到 —— Rust 侧也是这样从
        ! `interface_depth_mm` 复原厚度的，两边必须看到逐位相同的厚度。
        zi(0) = 0.0_r8
        DO j = 1, nl
           zi(j) = zi(j-1) + (1.0e-3_r8 + uni()*1.0_r8)
        ENDDO
        DO j = 1, nl
           dz(j) = zi(j) - zi(j-1)
        ENDDO
        DO j = 1, nl
           vls(j) = 0.2_r8 + uni()*0.6_r8
        ENDDO
        DO j = 1, nl
           wf(j) = uni()*dz(j)*0.4_r8
        ENDDO
        DO j = 1, nl
           wt(j) = uni()*dz(j)*0.4_r8
        ENDDO
        DO j = 1, nl
           vl(j) = 0.05_r8 + uni()*0.5_r8
        ENDDO
        DO j = 1, nl
           wf_m1(j) = uni()*dz(j)*0.4_r8
        ENDDO
        DO j = 1, nl
           wt_m1(j) = uni()*dz(j)*0.4_r8
        ENDDO
        DO j = 1, nl
           vl_m1(j) = 0.05_r8 + uni()*0.5_r8
        ENDDO
        DO j = 1, nl
           is_sat(j) = uni() < sat_prob
        ENDDO
        DO j = 0, nl
           q(j) = (uni() - 0.5_r8)*1.0e-3_r8
        ENDDO
        dt = 1.0_r8 + uni()*100.0_r8
        dp = uni()*10.0_r8
        dp_m1 = uni()*10.0_r8
        wa = uni()*100.0_r8
        wa_m1 = uni()*100.0_r8
        tol = 1.0e-8_r8 + uni()*1.0e-4_r8
        ubc_val = (uni() - 0.3_r8)*1.0e-3_r8
        lbc_val = (uni() - 0.3_r8)*1.0e-3_r8
        ! 排水子分支 `waquifer == 0 && q(ub) >= 0` 只在 k=2/3 强制进入；
        ! 其余情形留给随机值（`wa==0` 恰好命中时两边也一致）。
        IF ((k == 2) .OR. (k == 3)) THEN
           wa = 0.0_r8
           q(nl) = ABS(q(nl))
        ENDIF
        CALL water_balance (1, nl, dz, dt, is_sat, vls, q, &
             ubc, ubc_val, lbc, lbc_val, &
             wf, vl, wt, dp, wa, &
             wf_m1, vl_m1, wt_m1, dp_m1, wa_m1, &
             blc, solvable, tol)
        WRITE(66,'(I2,1X,I1)',ADVANCE='NO') k, nl
        DO j = 0, nl+1
           WRITE(66,'(1X,Z17)',ADVANCE='NO') B(blc(j))
        ENDDO
        WRITE(66,'(1X,I1)') MERGE(1, 0, solvable)
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
END PROGRAM wb
