! `MOD_Hydro_SoilWater:flux_inside_hm_soil` 的随机差分驱动（配对物
! `crates/colm-core/examples/flux_inside_probe.rs`）。
!
! 为什么单列：第 305/13 轮逐条读反汇编，在这个函数里映射出两处内核会融合而本仓库
! 平铺的表达式（`r0` 的分母、`grad_psi>1` 支）；第 313 轮确认 `effective_hk_type`
! 被编译期钉在 `type_weighted_geometric_mean`，所以这两处在**活分支**上。
! 这个闭环就是在合成输入上逐位判它们（纯函数，输入最好造）。
!
! **抽签次数与顺序必须与 Rust 侧逐条对齐**，改一边就得同步改另一边：
! 每例先抽 `psi_s, hksat, bsw, alpha, n, L, sc, fc`（两档都抽满 8 个），
! 再抽 `dz, psi_u, psi_l, hk_u, hk_l`。
!
! 输出三列十六进制：`flux`、`grad_psi`（驱动自己算的，用来确认六条分支都抽到过）、
! `hk_u`；前两列是十进制 `k`（0=VG、1=Campbell）与 `flag`（1..5 = grad_psi 分支）。
PROGRAM fi
  USE MOD_Hydro_SoilWater, only: flux_inside_hm_soil
  USE MOD_Namelist, only: DEF_USE_Campbell_SOIL_MODEL
  IMPLICIT NONE
  INTEGER, PARAMETER :: r8 = 8
  INTEGER(8) :: S
  INTEGER :: i, k, nprm, flag
  CHARACTER(LEN=256) :: dir
  INTEGER :: nlen
  REAL(r8) :: psi_s, hksat, bsw, alpha_vgm, n_vgm, l_vgm, sc_vgm, fc_vgm
  REAL(r8) :: dz, psi_u, psi_l, hk_u, hk_l, grad_psi, flux, prms(5)
  REAL(r8), PARAMETER :: P_PSIS(5)  = [-1.0e4_r8, -1.0e3_r8, -100.0_r8, -5.0e3_r8, -50.0_r8]
  REAL(r8), PARAMETER :: P_HK(5)    = [1.0e-6_r8, 1.0e-5_r8, 1.0e-4_r8, 1.0e-3_r8, 5.0e-5_r8]
  REAL(r8), PARAMETER :: P_BSW(5)   = [2.0_r8, 4.0_r8, 7.0_r8, 11.0_r8, 14.0_r8]
  REAL(r8), PARAMETER :: P_ALPHA(5) = [1.0e-3_r8, 5.0e-3_r8, 1.0e-2_r8, 5.0e-2_r8, 0.1_r8]
  REAL(r8), PARAMETER :: P_N(5)     = [1.1_r8, 1.5_r8, 2.0_r8, 2.5_r8, 3.0_r8]
  REAL(r8), PARAMETER :: P_L(5)     = [0.5_r8, 0.7_r8, 0.9_r8, 1.0_r8, 0.6_r8]
  REAL(r8), PARAMETER :: P_SC(5)    = [0.2_r8, 0.4_r8, 0.6_r8, 0.8_r8, 1.0_r8]
  REAL(r8), PARAMETER :: P_FC(5)    = [0.2_r8, 0.4_r8, 0.6_r8, 0.8_r8, 1.0_r8]
  REAL(r8), PARAMETER :: P_DZ(5)    = [1.0e-3_r8, 1.0e-2_r8, 0.1_r8, 0.5_r8, 1.0_r8]
  REAL(r8), PARAMETER :: P_PSIU(5)  = [-1.0e4_r8, -1.0e3_r8, -100.0_r8, -1.0_r8, 0.0_r8]
  REAL(r8), PARAMETER :: P_PSIL(5)  = [-1.0e4_r8, -1.0e3_r8, -100.0_r8, -1.0_r8, 0.0_r8]
  REAL(r8), PARAMETER :: P_HKU(5)   = [1.0e-8_r8, 1.0e-6_r8, 1.0e-5_r8, 1.0e-4_r8, 1.0e-3_r8]

  S = 20260810_8
  CALL GET_ENVIRONMENT_VARIABLE('FI_OUT', dir, nlen)
  IF (nlen <= 0) dir = '/tmp/gf/fi'
  OPEN(66, FILE=TRIM(dir)//'/fi.txt', STATUS='REPLACE')
  DO k = 0, 1
     DEF_USE_Campbell_SOIL_MODEL = (k == 1)
     nprm = MERGE(1, 5, k == 1)
     DO i = 1, 6000
        IF (i <= 3000) THEN
           psi_s = -(100.0_r8 + uni()*1.0e4_r8)
           hksat = 1.0e-6_r8 + uni()*1.0e-3_r8
           bsw   = 2.0_r8 + uni()*12.0_r8
           alpha_vgm = 1.0e-3_r8 + uni()*0.1_r8
           n_vgm = 1.05_r8 + uni()*2.0_r8
           l_vgm = 0.5_r8 + uni()*0.5_r8
           sc_vgm = 0.2_r8 + uni()*0.8_r8
           fc_vgm = 0.2_r8 + uni()*0.8_r8
           dz    = 1.0e-3_r8 + uni()*1.0_r8
           psi_u = -(1.0_r8 + uni()*1.0e4_r8)
           psi_l = -(1.0_r8 + uni()*1.0e4_r8)
           hk_u  = 1.0e-9_r8 + uni()*1.0e-3_r8
           hk_l  = 1.0e-9_r8 + uni()*1.0e-3_r8
        ELSE
           psi_s = pick(P_PSIS)
           hksat = pick(P_HK)
           bsw   = pick(P_BSW)
           alpha_vgm = pick(P_ALPHA)
           n_vgm = pick(P_N)
           l_vgm = pick(P_L)
           sc_vgm = pick(P_SC)
           fc_vgm = pick(P_FC)
           dz    = pick(P_DZ)
           psi_u = pick(P_PSIU)
           psi_l = pick(P_PSIL)
           hk_u  = pick(P_HKU)
           hk_l  = pick(P_HKU)
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
        grad_psi = 1.0_r8 - (psi_l - psi_u)/dz
        flag = 5
        IF (grad_psi < 0.0_r8) THEN
           flag = 1
        ELSEIF (grad_psi == 0.0_r8) THEN
           flag = 2
        ELSEIF (grad_psi < 1.0_r8) THEN
           flag = 3
        ELSEIF (grad_psi == 1.0_r8) THEN
           flag = 4
        ENDIF
        flux = flux_inside_hm_soil (psi_s, hksat, nprm, prms, &
                                    dz, psi_u, psi_l, hk_u, hk_l)
        WRITE(66,'(I2,1X,I1,1X,3Z17)') k, flag, B(flux), B(grad_psi), B(hk_u)
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
END PROGRAM fi
