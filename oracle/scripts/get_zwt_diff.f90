! `MOD_Hydro_SoilWater:get_zwt_from_wa`（含内联的 `secant_method_iteration`）的随机差分驱动。
!
! 为什么单列一个：这一段是干窗第 19 步那颗持久种子的所在地（第 301 轮 `vsf_wt_probe.sh`
! 把首差钉在 `soilwater_aquifer_exchange` 里改出来的 `zwt` 上），但它的形状改动**不能**
! 用黄金窗口判 —— 那三份窗口由 `f_vegwp` 混沌主导，改对末位会把轨道换一条、指标可能反向
! （第 301 轮实测：5 处形状让探针 12/240 → 7/240，却让干窗 `over_tol` 28 → 36）。
! 所以这里在**合成输入**上直接逐位比 `zwt`，不经过混沌。
!
! 配对物 `crates/colm-core/examples/get_zwt_probe.rs`：两侧共用同一串 LCG，
! **抽签次数与顺序必须逐条对齐**，改一边就得同步改另一边。
!
! 两档模型（Campbell / van Genuchten）× (2500 均匀随机 + 2500 边界取值)，
! 参数排布照 `MOD_Hydro_SoilFunction.F90`：VG 时 `prms(1..5) = alpha,n,L,sc,fc`；
! Campbell 时 `prms(1) = bsw`。两档都先抽满 10 个数再按模型装数组，抽签序列才对齐。
!
! 前两列是十进制：`k`（0=VG、1=Campbell）、`flag`（1 = `wa >= 0` 的早退分支）；
! 后面 5 列是十六进制位型：`wa`、`zmin`、`vl_s`、`psi_s`、`zwt`。
! 输入列也打出来是为了让"两侧确实在同一条用例上"可核对。
PROGRAM gz
  USE MOD_Hydro_SoilWater, only: get_zwt_from_wa
  USE MOD_Namelist, only: DEF_USE_Campbell_SOIL_MODEL
  IMPLICIT NONE
  INTEGER, PARAMETER :: r8 = 8
  INTEGER(8) :: S
  INTEGER :: i, k, nlen, nprm, flag
  CHARACTER(LEN=256) :: dir
  REAL(r8) :: vl_s, vl_r, psi_s, hksat, wa, zmin, tol_v, tol_z, zwt
  REAL(r8) :: bsw, alpha_vgm, n_vgm, l_vgm, sc_vgm, fc_vgm, prms(5)
  REAL(r8), PARAMETER :: P_VLS(5)   = [0.30_r8, 0.40_r8, 0.50_r8, 0.60_r8, 0.45_r8]
  REAL(r8), PARAMETER :: P_VLR(5)   = [0.02_r8, 0.05_r8, 0.10_r8, 0.15_r8, 1.0e-9_r8]
  REAL(r8), PARAMETER :: P_PSIS(5)  = [-1.0e4_r8, -1.0e3_r8, -100.0_r8, -5.0e3_r8, -50.0_r8]
  REAL(r8), PARAMETER :: P_HK(5)    = [1.0e-6_r8, 1.0e-5_r8, 1.0e-4_r8, 1.0e-3_r8, 5.0e-5_r8]
  REAL(r8), PARAMETER :: P_BSW(5)   = [2.0_r8, 4.0_r8, 7.0_r8, 11.0_r8, 14.0_r8]
  REAL(r8), PARAMETER :: P_ALPHA(5) = [1.0e-3_r8, 5.0e-3_r8, 1.0e-2_r8, 5.0e-2_r8, 0.1_r8]
  REAL(r8), PARAMETER :: P_N(5)     = [1.1_r8, 1.5_r8, 2.0_r8, 2.5_r8, 3.0_r8]
  REAL(r8), PARAMETER :: P_L(5)     = [0.5_r8, 0.7_r8, 0.9_r8, 1.0_r8, 0.6_r8]
  REAL(r8), PARAMETER :: P_SC(5)    = [0.2_r8, 0.4_r8, 0.6_r8, 0.8_r8, 1.0_r8]
  REAL(r8), PARAMETER :: P_FC(5)    = [0.2_r8, 0.4_r8, 0.6_r8, 0.8_r8, 1.0_r8]
  ! 前 4 个是负的（含水层亏缺，目标分支），后 2 个覆盖 `wa >= 0` 的早退。
  REAL(r8), PARAMETER :: P_WA(6)    = [-1.0e-6_r8, -1.0e-3_r8, -0.5_r8, -5.0_r8, 0.0_r8, 5.0_r8]
  REAL(r8), PARAMETER :: P_ZMIN(5)  = [10.0_r8, 100.0_r8, 500.0_r8, 1000.0_r8, 2000.0_r8]
  REAL(r8), PARAMETER :: P_TOLV(5)  = [1.0e-8_r8, 1.0e-6_r8, 1.0e-4_r8, 1.0e-3_r8, 1.0e-2_r8]
  REAL(r8), PARAMETER :: P_TOLZ(5)  = [1.0e-6_r8, 1.0e-4_r8, 1.0e-2_r8, 1.0_r8, 10.0_r8]

  S = 20260809_8
  CALL GET_ENVIRONMENT_VARIABLE('GZ_OUT', dir, nlen)
  IF (nlen <= 0) dir = '/tmp/gf/gz'
  OPEN(66, FILE=TRIM(dir)//'/gz.txt', STATUS='REPLACE')
  DO k = 0, 1
     DEF_USE_Campbell_SOIL_MODEL = (k == 1)
     nprm = MERGE(1, 5, k == 1)
     DO i = 1, 5000
        IF (i <= 2500) THEN
           vl_s  = 0.30_r8 + uni()*0.30_r8
           vl_r  = 0.005_r8 + uni()*0.15_r8
           psi_s = -(100.0_r8 + uni()*1.0e4_r8)
           hksat = 1.0e-6_r8 + uni()*1.0e-3_r8
           bsw   = 2.0_r8 + uni()*12.0_r8
           alpha_vgm = 1.0e-3_r8 + uni()*0.1_r8
           n_vgm = 1.05_r8 + uni()*2.0_r8
           l_vgm = 0.5_r8 + uni()*0.5_r8
           sc_vgm = 0.2_r8 + uni()*0.8_r8
           fc_vgm = 0.2_r8 + uni()*0.8_r8
           wa    = -(uni()*200.0_r8)
           zmin  = 10.0_r8 + uni()*2000.0_r8
           tol_v = 1.0e-6_r8 + uni()*1.0e-3_r8
           tol_z = 1.0e-4_r8 + uni()*1.0e-1_r8
        ELSE
           vl_s  = pick(P_VLS)
           vl_r  = pick(P_VLR)
           psi_s = pick(P_PSIS)
           hksat = pick(P_HK)
           bsw   = pick(P_BSW)
           alpha_vgm = pick(P_ALPHA)
           n_vgm = pick(P_N)
           l_vgm = pick(P_L)
           sc_vgm = pick(P_SC)
           fc_vgm = pick(P_FC)
           wa    = pick(P_WA)
           zmin  = pick(P_ZMIN)
           tol_v = pick(P_TOLV)
           tol_z = pick(P_TOLZ)
        ENDIF
        IF (k == 1) THEN
           prms(1) = bsw
        ELSE
           prms(1) = alpha_vgm
           prms(2) = n_vgm
           prms(3) = l_vgm
           prms(4) = sc_vgm
           prms(5) = fc_vgm
        ENDIF
        flag = 0
        IF (wa >= 0.0_r8) flag = 1
        zwt = -9999.0_r8
        CALL get_zwt_from_wa(vl_s, vl_r, psi_s, hksat, nprm, prms, &
                             tol_v, tol_z, wa, zmin, zwt)
        WRITE(66,'(I2,1X,I1,1X,5Z17)') k, flag, B(wa), B(zmin), B(vl_s), B(psi_s), B(zwt)
     ENDDO
  ENDDO
  CLOSE(66)

CONTAINS
  REAL(r8) FUNCTION uni() RESULT(v)
    S = S*6364136223846793005_8 + 1442695040888963407_8
    v = REAL(ISHFT(S, -11), r8)/9007199254740992.0_r8
  END FUNCTION uni

  REAL(r8) FUNCTION pick(pool) RESULT(v)
    REAL(r8), INTENT(IN) :: pool(:)
    INTEGER :: idx
    idx = MIN(INT(uni()*REAL(SIZE(pool), r8)) + 1, SIZE(pool))
    v = pool(idx)
  END FUNCTION pick

  INTEGER(8) FUNCTION B(x) RESULT(bits)
    REAL(r8), INTENT(IN) :: x
    bits = TRANSFER(x, 0_8)
  END FUNCTION B
END PROGRAM gz
