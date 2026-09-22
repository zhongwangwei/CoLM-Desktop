! MOD_PhaseChange:meltf 的随机差分驱动。
! 配对物 crates/colm-core/examples/phase_change_probe.rs；两侧共用同一串 LCG。
!
! 上游侧**不重编模块**：直接链接 `.bld` 下的内核产线对象（见 compare_phasechange.sh），
! 所以跑的是真正的内核代码。
!
! 配置：单个雪层配置 `nsnow = 2`（`lb = 1 - nsnow = -1`）、`nl_soil = 4`，
! 于是数组下界固定为 -1，哑元 `(lb:nl_soil)` 与实参形状一致。
! `patchtype` 逐档遍历（0..4），`is_dry_lake` 取 `.false.`。
!
! 输出（每行一个 case）：patchtype、配置位、以及 7 个量的位型：
!   t_soisno(1)、wliq_soisno(1)、wice_soisno(1)、scv、sm、xmf、imelt 的逐层和。
PROGRAM phasechange
  USE MOD_PhaseChange, only: meltf
  USE MOD_Namelist, only: DEF_USE_Campbell_SOIL_MODEL, DEF_USE_SUPERCOOL_WATER, DEF_SPLIT_SOILSNOW
  IMPLICIT NONE
  INTEGER, PARAMETER :: r8=8, NL=4, NSNOW=2
  INTEGER(8) :: S
  INTEGER :: i, ip, k, nlen, sum_imelt
  REAL(r8) :: sum_qthaw, sum_qfrz
  INTEGER :: lb
  LOGICAL :: dry
  CHARACTER(LEN=256) :: dir
  REAL(r8) :: deltim, hs, hs_soil, hs_snow, fsno, dhsdT
  REAL(r8) :: scv, snowdp, sm, xmf
  REAL(r8) :: qthaw(-1:NL), qfrz(-1:NL)
  REAL(r8) :: fact(-1:NL), brr(-1:NL), t_bef(-1:NL), t(-1:NL)
  REAL(r8) :: wliq(-1:NL), wice(-1:NL)
  INTEGER :: imelt(-1:NL)
  REAL(r8) :: porsl(NL), psi0(NL), bsw(NL), theta_r(NL), dz(NL)
  REAL(r8) :: alpha_vgm(NL), n_vgm(NL), L_vgm(NL), sc_vgm(NL), fc_vgm(NL)
  S = 20250512_8
  lb = 1 - NSNOW
  dry = .FALSE.
  CALL GET_ENVIRONMENT_VARIABLE('PC_OUT', dir, nlen)
  IF (nlen <= 0) dir = '/tmp/gf/pc_diff'
  OPEN(66, FILE=TRIM(dir)//'/pc.txt', STATUS='REPLACE')
  DO ip = 0, 4
     DO i = 1, 2000
        deltim = 1800._r8
        fact(lb:NL) = 100._r8 + uni()*2000._r8
        brr(lb:NL) = -50._r8 + uni()*100._r8
        t_bef(lb:NL) = 250._r8 + uni()*40._r8
        t(lb:NL) = t_bef(lb:NL) + (-0.5_r8 + uni())*2._r8
        wliq(lb:NL) = uni()*5._r8
        wice(lb:NL) = uni()*5._r8
        DO k = 1, NL
           porsl(k) = 0.3_r8 + uni()*0.3_r8
           psi0(k) = -(10._r8 + uni()*300._r8)
           bsw(k) = 2._r8 + uni()*8._r8
           theta_r(k) = 0.02_r8 + uni()*0.08_r8
           dz(k) = 0.05_r8 + uni()*0.4_r8
           alpha_vgm(k) = 0.005_r8 + uni()*0.05_r8
           n_vgm(k) = 1.2_r8 + uni()*1.5_r8
           L_vgm(k) = 0.3_r8 + uni()*0.7_r8
           sc_vgm(k) = 0.02_r8 + uni()*0.08_r8
           fc_vgm(k) = 0.1_r8 + uni()*0.3_r8
        ENDDO
        hs = -100._r8 + uni()*200._r8
        hs_soil = -100._r8 + uni()*200._r8
        hs_snow = -100._r8 + uni()*200._r8
        fsno = uni()
        dhsdT = -20._r8 + uni()*20._r8
        scv = uni()*20._r8
        snowdp = uni()*0.3_r8
        sm = 0._r8
        xmf = 0._r8
        qthaw(lb:NL) = 0._r8
        qfrz(lb:NL) = 0._r8
        imelt(lb:NL) = 0
        CALL meltf(ip, dry, lb, NL, deltim, &
                   fact, brr, hs, hs_soil, hs_snow, fsno, dhsdT, &
                   t_bef, t, wliq, wice, imelt, &
                   scv, snowdp, sm, xmf, porsl, psi0, &
                   bsw, theta_r, alpha_vgm, n_vgm, L_vgm, &
                   sc_vgm, fc_vgm, dz, &
                   qthaw, qfrz)
        sum_imelt = SUM(imelt(lb:NL))
        sum_qthaw = SUM(qthaw(lb:NL))
        sum_qfrz = SUM(qfrz(lb:NL))
        WRITE(66,'(I1,I1,1X,8Z17,I8)') ip, NSNOW, &
             B(t(lb)), B(wliq(lb)), B(wice(lb)), &
             B(scv), B(sm), B(xmf), B(sum_qthaw), B(sum_qfrz), sum_imelt
     ENDDO
  ENDDO
  CLOSE(66)
  PRINT *, 'campbell/vgm = ', DEF_USE_Campbell_SOIL_MODEL, &
           ' supercool = ', DEF_USE_SUPERCOOL_WATER, ' split = ', DEF_SPLIT_SOILSNOW
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
END PROGRAM phasechange
