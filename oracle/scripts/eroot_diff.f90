! `MOD_Eroot:eroot` 的随机差分驱动（配对物
! `crates/colm-core/examples/eroot_probe.rs`）。
!
! 为什么单列：`eroot` 的两个输出里，**只有 `rstfac` 被黄金窗口走到**
! （气孔阻力用）。`rootr`（逐层根阻力份额）与 `etrc`（最大可能蒸腾率）
! 只在 `DEF_USE_PLANTHYDRAULICS = .false.` 那一支里被用 —— 三个黄金算例
! 全开植物水力，所以这两个输出在本机**从来没有端到端信号**（第 381 轮就是
! 在 PHS 关掉时先差零的符号、第 11 步起放大到水文量上）。
!
! 覆盖：两种土壤水力方案（Campbell / van Genuchten）× 两种 `DEF_RSTFAC`
! ×（1000 组均匀随机 + 1000 组**边界/零值**取值）。边界那一批是刻意加的：
! `etrc = trsmx0*(...)` 这类乘积在零上会带符号，随机取值几乎撞不到。
!
! **抽签次数与顺序必须与 Rust 侧逐条对齐**，改一边就得同步改另一边。
! 输出：`k` 后 12 列十六进制 —— `rootr(1:10)`、`etrc`、`rstfac`。
PROGRAM erd
  USE MOD_Eroot, only: eroot
  USE MOD_Namelist, only: DEF_RSTFAC, DEF_USE_Campbell_SOIL_MODEL
  IMPLICIT NONE
  INTEGER, PARAMETER :: r8 = 8, nl = 10
  INTEGER(8) :: S
  INTEGER :: i, k, nlen, j
  REAL(r8) :: trsmx0
  REAL(r8) :: porsl(nl), bsw(nl), theta_r(nl), alpha_vgm(nl), n_vgm(nl)
  REAL(r8) :: L_vgm(nl), sc_vgm(nl), fc_vgm(nl), psi0(nl), rootfr(nl)
  REAL(r8) :: dz_soisno(nl), t_soisno(nl), wliq_soisno(nl)
  REAL(r8) :: rootr(nl), etrc, rstfac
  REAL(r8) :: denh2o
  CHARACTER(LEN=256) :: dir

  denh2o = 1000.0_r8
  S = 20260925_8
  CALL GET_ENVIRONMENT_VARIABLE('EROOT_OUT', dir, nlen)
  IF (nlen <= 0) dir = '/tmp/gf/eroot'
  OPEN(66, FILE=TRIM(dir)//'/eroot.txt', STATUS='REPLACE')
  DO k = 0, 3
     DEF_USE_Campbell_SOIL_MODEL = (MOD(k, 2) == 0)
     DEF_RSTFAC = 1 + k/2
     DO i = 1, 2000
        trsmx0 = uni()*1.0e-6_r8
        DO j = 1, nl
           IF (i <= 1000) THEN            ! 均匀随机
              porsl(j)   = 0.35_r8 + uni()*0.25_r8
              bsw(j)     = 2.0_r8 + uni()*10.0_r8
              theta_r(j) = 0.02_r8 + uni()*0.10_r8
              alpha_vgm(j) = 5.0e-4_r8 + uni()*7.5e-3_r8
              n_vgm(j)   = 1.2_r8 + uni()*1.5_r8
              L_vgm(j)   = 0.5_r8 + uni()*1.5_r8
              sc_vgm(j)  = 0.1_r8 + uni()*0.7_r8
              fc_vgm(j)  = 0.1_r8 + uni()*0.8_r8
              psi0(j)    = -(10.0_r8 + uni()*990.0_r8)
              rootfr(j)  = uni()
              dz_soisno(j) = 0.05_r8 + uni()*0.5_r8
              t_soisno(j)  = 250.0_r8 + uni()*70.0_r8
              wliq_soisno(j) = uni()*porsl(j)*dz_soisno(j)*denh2o
           ELSE                            ! 边界/零值：
              porsl(j)   = MERGE(0.30_r8, 0.60_r8, MOD(j,2)==0)
              bsw(j)     = MERGE(2.0_r8, 12.0_r8, MOD(j,2)==0)
              theta_r(j) = MERGE(0.0_r8, 0.12_r8, MOD(j,2)==0)
              alpha_vgm(j) = MERGE(5.0e-4_r8, 8.0e-3_r8, MOD(j,2)==0)
              n_vgm(j)   = MERGE(1.2_r8, 2.7_r8, MOD(j,2)==0)
              L_vgm(j)   = MERGE(0.5_r8, 2.0_r8, MOD(j,2)==0)
              sc_vgm(j)  = MERGE(0.1_r8, 0.8_r8, MOD(j,2)==0)
              fc_vgm(j)  = MERGE(0.1_r8, 0.9_r8, MOD(j,2)==0)
              psi0(j)    = MERGE(-10.0_r8, -1000.0_r8, MOD(j,2)==0)
              rootfr(j)  = MERGE(0.0_r8, 1.0_r8, MOD(j,3)==0)
              dz_soisno(j) = MERGE(0.05_r8, 0.55_r8, MOD(j,2)==0)
              ! 一半全冻、一半全干：`wliq = 0` 会把 `etrc`/`rootr` 推到零上
              t_soisno(j)  = MERGE(260.0_r8, 300.0_r8, MOD(j,2)==0)
              wliq_soisno(j) = MERGE(0.0_r8, porsl(j)*dz_soisno(j)*denh2o, MOD(j,3)==0)
           ENDIF
        ENDDO
        CALL eroot (nl, trsmx0, porsl, bsw, theta_r, alpha_vgm, n_vgm, L_vgm, sc_vgm, &
             fc_vgm, psi0, rootfr, dz_soisno, t_soisno, wliq_soisno, rootr, etrc, rstfac)
        WRITE(66,'(I2,1X,12(1X,Z17))') k, (B(rootr(j)), j = 1, nl), B(etrc), B(rstfac)
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
END PROGRAM erd
