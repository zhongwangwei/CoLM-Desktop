! `MOD_Hydro_SoilWater:get_water_equilibrium_state` 的随机差分驱动。
!
! 为什么单列一个：这个例程**只在冷启动**（`mkinidata/MOD_IniTimeVariable.F90:463`
! 的 `use_wtd` 分支）被调用，`colm.x` 运行时一次都不进 —— 黄金窗口与干湿窗
! 首分歧都对它**不敏感**，形状对不对只能靠这个闭环判。
!
! 配对物 crates/colm-core/examples/water_equilibrium_probe.rs；两侧共用同一串 LCG，
! 抽签次数与顺序必须逐条对齐，改一边就得同步改另一边：
! 每例先抽 1 次定 `nlev`、1 次定 `zwtmm`，再**逐层抽 11 次**
! （10 个层参数 + 1 个层厚），两档模型（Campbell / van Genuchten）各 4000 例。
!
! 前四个整数列：`k`（0=VG、1=Campbell）、`flag`（1 = `izwt == nlev+1`，走含水层分支）、
! `nlev`、`izwt`（水位所在层号）。`flag`/`izwt` 不参与筛选，只用来确认两条分支
! 都真被抽到过。
!
! 后面是十六进制位型：`wa`，然后逐层 `wliq / smp / hk`。
!
! `zwtmm` 一律取非负（真实调用点传的是水位埋深）：`izwt = 0` 在上游会去读
! `psi_s(0)`，越界；Rust 侧那边被 `(water_layer-1).min(layers-1)` 兜住，
! 两侧在这种输入上本来就不等价，不该拿来比。
PROGRAM eqw
  USE MOD_Hydro_SoilWater, only: get_water_equilibrium_state
  USE MOD_Namelist, only: DEF_USE_Campbell_SOIL_MODEL
  IMPLICIT NONE
  INTEGER, PARAMETER :: r8 = 8
  INTEGER, PARAMETER :: MAXLEV = 10
  INTEGER(8) :: S
  INTEGER :: i, k, j, nlen, nprm, nlev, izwt, flag
  CHARACTER(LEN=256) :: dir
  REAL(r8) :: zwtmm, wa
  REAL(r8) :: sp_zc(MAXLEV), sp_zi(0:MAXLEV)
  REAL(r8) :: porsl(MAXLEV), vl_r(MAXLEV), psi_s(MAXLEV), hksat(MAXLEV)
  REAL(r8) :: prms(5, MAXLEV)
  REAL(r8) :: wliq(MAXLEV), smp(MAXLEV), hk(MAXLEV)
  REAL(r8) :: bsw(MAXLEV), alpha_vgm(MAXLEV), n_vgm(MAXLEV)
  REAL(r8) :: l_vgm(MAXLEV), sc_vgm(MAXLEV), fc_vgm(MAXLEV)
  REAL(r8), PARAMETER :: P_NLEV(5)  = [1.0_r8, 2.0_r8, 3.0_r8, 5.0_r8, 10.0_r8]
  REAL(r8), PARAMETER :: P_ZWT(6)   = [0.0_r8, 1.0_r8, 50.0_r8, 500.0_r8, 2000.0_r8, 20000.0_r8]
  REAL(r8), PARAMETER :: P_DZ(6)    = [10.0_r8, 50.0_r8, 100.0_r8, 200.0_r8, 500.0_r8, 2000.0_r8]
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

  S = 20260825_8
  CALL GET_ENVIRONMENT_VARIABLE('EQW_OUT', dir, nlen)
  IF (nlen <= 0) dir = '/tmp/gf/eqw'
  OPEN(66, FILE=TRIM(dir)//'/eqw.txt', STATUS='REPLACE')
  nprm = 5
  DO k = 0, 1
     DEF_USE_Campbell_SOIL_MODEL = (k == 1)
     DO i = 1, 4000
        sp_zi(0) = 0.0_r8
        IF (i <= 2000) THEN
           nlev  = INT(pick(P_NLEV))
           zwtmm = uni()*20000.0_r8
           DO j = 1, nlev
              porsl(j)  = 0.30_r8 + uni()*0.30_r8
              vl_r(j)   = 0.005_r8 + uni()*0.15_r8
              psi_s(j)  = -(100.0_r8 + uni()*1.0e4_r8)
              hksat(j)  = 1.0e-6_r8 + uni()*1.0e-3_r8
              bsw(j)    = 2.0_r8 + uni()*12.0_r8
              alpha_vgm(j) = 1.0e-3_r8 + uni()*0.1_r8
              n_vgm(j)  = 1.05_r8 + uni()*2.0_r8
              l_vgm(j)  = 0.5_r8 + uni()*0.5_r8
              sc_vgm(j) = 0.2_r8 + uni()*0.8_r8
              fc_vgm(j) = 0.2_r8 + uni()*0.8_r8
              sp_zi(j)  = sp_zi(j-1) + 100.0_r8 + uni()*900.0_r8
           ENDDO
        ELSE
           nlev  = INT(pick(P_NLEV))
           zwtmm = pick(P_ZWT)
           DO j = 1, nlev
              porsl(j)  = pick(P_POR)
              vl_r(j)   = pick(P_VLR)
              psi_s(j)  = pick(P_PSIS)
              hksat(j)  = pick(P_HK)
              bsw(j)    = pick(P_BSW)
              alpha_vgm(j) = pick(P_ALPHA)
              n_vgm(j)  = pick(P_N)
              l_vgm(j)  = pick(P_L)
              sc_vgm(j) = pick(P_SC)
              fc_vgm(j) = pick(P_FC)
              sp_zi(j)  = sp_zi(j-1) + pick(P_DZ)
           ENDDO
        ENDIF
        DO j = 1, nlev
           sp_zc(j) = (sp_zi(j-1) + sp_zi(j))*0.5_r8
           IF (k == 1) THEN
              prms(1,j) = bsw(j)
           ELSE
              prms(1,j) = alpha_vgm(j)
              prms(2,j) = n_vgm(j)
              prms(3,j) = l_vgm(j)
              prms(4,j) = sc_vgm(j)
              prms(5,j) = fc_vgm(j)
           ENDIF
        ENDDO

        CALL get_water_equilibrium_state (zwtmm, nlev, wliq, smp, hk, wa, &
           sp_zc, sp_zi, porsl, vl_r, psi_s, hksat, nprm, prms)

        ! `findloc_ud(zwtmm >= sp_zi, back=.true.)` 的等价写法（只作诊断列）。
        izwt = 0
        DO j = 0, nlev
           IF (zwtmm >= sp_zi(j)) izwt = j + 1
        ENDDO
        flag = 0
        IF (izwt == nlev + 1) flag = 1

        WRITE(66,'(I3,1X,I3,1X,I3,1X,I3)',ADVANCE='NO') k, flag, nlev, izwt
        WRITE(66,'(1X,Z17)',ADVANCE='NO') B(wa)
        DO j = 1, nlev
           WRITE(66,'(3(1X,Z17))',ADVANCE='NO') B(wliq(j)), B(smp(j)), B(hk(j))
        ENDDO
        WRITE(66,*)
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
END PROGRAM eqw
