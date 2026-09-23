! MOD_Hydro_SoilFunction 的随机差分驱动。
!
! 覆盖 `soil_psi_from_vliq` / `soil_hk_from_psi` / `soil_vliq_from_psi`，
! 两档模型（Campbell / van Genuchten）× (2500 组均匀随机 + 2500 组边界取值)。
! 配对物 crates/colm-core/examples/soil_hydro_fn_probe.rs；两侧共用同一串 LCG，
! 抽签次数与顺序必须逐条对齐，改一边就得同步改另一边。
!
! 三个函数**串联**调用：`psi = soil_psi_from_vliq(vliq,…)`，再把同一个 `psi`
! 喂给另外两个 —— 一处差就会在下游显形，而不是被各自的输入掩盖。
!
! 参数排布照 `MOD_Hydro_SoilFunction.F90` 的注释：
!   VG 时 prms(1..5) = alpha_vgm, n_vgm, L_vgm, sc_vgm, fc_vgm；
!   Campbell 时 prms(1) = bsw。
! **两档都先抽满 6 个数**（bsw/alpha/n/l/sc/fc）再按模型装进数组，
! 这样两档的抽签序列逐条对齐。
! 第三列 `flag` 标出走的是哪条早退分支（1 = vliq>=porsl、2 = vliq<=max(vl_r,1e-8)），
! 只用来确认边界路径真被抽到过，不参与筛选。
PROGRAM hf
  USE MOD_Hydro_SoilFunction, only: soil_psi_from_vliq, soil_hk_from_psi, soil_vliq_from_psi
  USE MOD_Namelist, only: DEF_USE_Campbell_SOIL_MODEL
  IMPLICIT NONE
  INTEGER, PARAMETER :: r8 = 8
  INTEGER(8) :: S
  INTEGER :: i, k, nlen, nprm, flag
  CHARACTER(LEN=256) :: dir
  REAL(r8) :: vliq, porsl, vl_r, psi_s, hksat, prms(5), psi, hk, vl
  REAL(r8) :: bsw, alpha_vgm, n_vgm, l_vgm, sc_vgm, fc_vgm
  REAL(r8), PARAMETER :: P_POR(5)   = [0.30_r8, 0.40_r8, 0.50_r8, 0.60_r8, 0.45_r8]
  REAL(r8), PARAMETER :: P_VLR(5)   = [0.02_r8, 0.05_r8, 0.10_r8, 0.15_r8, 1.0e-9_r8]
  REAL(r8), PARAMETER :: P_PSIS(5)  = [-1.0e4_r8, -1.0e3_r8, -100.0_r8, -5.0e3_r8, -50.0_r8]
  REAL(r8), PARAMETER :: P_HK(5)    = [1.0e-6_r8, 1.0e-5_r8, 1.0e-4_r8, 1.0e-3_r8, 5.0e-5_r8]
  REAL(r8), PARAMETER :: P_BSW(5)   = [2.0_r8, 4.0_r8, 7.0_r8, 11.0_r8, 14.0_r8]
  REAL(r8), PARAMETER :: P_ALPHA(5) = [1.0e-3_r8, 5.0e-3_r8, 1.0e-2_r8, 5.0e-2_r8, 0.1_r8]
  REAL(r8), PARAMETER :: P_N(5)     = [1.1_r8, 1.5_r8, 2.0_r8, 2.5_r8, 3.0_r8]
  REAL(r8), PARAMETER :: P_L(5)     = [0.5_r8, 0.7_r8, 0.9_r8, 1.0_r8, 0.6_r8]
  REAL(r8), PARAMETER :: P_SC(5)    = [0.2_r8, 0.4_r8, 0.6_r8, 0.8_r8, 1.0_r8]
  REAL(r8), PARAMETER :: P_FC(5)    = [0.2_r8, 0.4_r8, 0.6_r8, 0.8_r8, 1.0_r8]
  REAL(r8), PARAMETER :: P_VLIQ(5)  = [0.0_r8, 1.0e-12_r8, 0.05_r8, 0.25_r8, 0.99_r8]

  S = 20260808_8
  CALL GET_ENVIRONMENT_VARIABLE('HF_OUT', dir, nlen)
  IF (nlen <= 0) dir = '/tmp/gf/hf'
  OPEN(66, FILE=TRIM(dir)//'/hf.txt', STATUS='REPLACE')
  nprm = 5
  DO k = 0, 1
     DEF_USE_Campbell_SOIL_MODEL = (k == 1)
     DO i = 1, 5000
        IF (i <= 2500) THEN
           porsl  = 0.30_r8 + uni()*0.30_r8
           vl_r   = 0.005_r8 + uni()*0.15_r8
           psi_s  = -(100.0_r8 + uni()*1.0e4_r8)
           hksat  = 1.0e-6_r8 + uni()*1.0e-3_r8
           bsw    = 2.0_r8 + uni()*12.0_r8
           alpha_vgm = 1.0e-3_r8 + uni()*0.1_r8
           n_vgm  = 1.05_r8 + uni()*2.0_r8
           l_vgm  = 0.5_r8 + uni()*0.5_r8
           sc_vgm = 0.2_r8 + uni()*0.8_r8
           fc_vgm = 0.2_r8 + uni()*0.8_r8
           vliq   = uni()*porsl*1.2_r8
        ELSE
           porsl  = pick(P_POR)
           vl_r   = pick(P_VLR)
           psi_s  = pick(P_PSIS)
           hksat  = pick(P_HK)
           bsw    = pick(P_BSW)
           alpha_vgm = pick(P_ALPHA)
           n_vgm  = pick(P_N)
           l_vgm  = pick(P_L)
           sc_vgm = pick(P_SC)
           fc_vgm = pick(P_FC)
           vliq   = pick(P_VLIQ)*porsl
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
        psi = soil_psi_from_vliq(vliq, porsl, vl_r, psi_s, nprm, prms)
        hk  = soil_hk_from_psi(psi, psi_s, hksat, nprm, prms)
        vl  = soil_vliq_from_psi(psi, porsl, vl_r, psi_s, nprm, prms)
        flag = 0
        IF (vliq >= porsl) flag = 1
        IF (vliq <= max(vl_r, 1.0e-8_r8)) flag = 2
        WRITE(66,'(I2,1X,I1,1X,3Z17)') k, flag, B(psi), B(hk), B(vl)
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
END PROGRAM hf
