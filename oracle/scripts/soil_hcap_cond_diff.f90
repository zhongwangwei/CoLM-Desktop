! MOD_SoilThermalParameters:soil_hcap_cond 的随机差分驱动。
! 8 档方案 × (2500 组均匀随机 + 2500 组边界取值)。
! 配对物 crates/colm-core/examples/soil_thermal_probe.rs；两侧共用同一串 LCG，
! 抽签次数与顺序必须逐条对齐，改一边就得同步改另一边。
!
! 第三列 flag 标出 sr=(vf_water+vf_ice)/vf_pores >= 1e-10 是否成立：不成立时上游
! 只把 ke 置 0（`MOD_SoilThermalParameters.F90:418-420`），thk 的赋值在门外的
! :422-517，所以 thk 照样有定义、照样参与比对。这一列只是让下游确认那条干土路径
! 真被走到了 —— 边界抽样若一次都没命中，就是抽样本身失效。
PROGRAM hc
  USE MOD_SoilThermalParameters, only: soil_hcap_cond
  USE MOD_Namelist, only: DEF_THERMAL_CONDUCTIVITY_SCHEME
  IMPLICIT NONE
  INTEGER, PARAMETER :: r8=8
  INTEGER(8) :: S
  INTEGER :: i, k, ip, nlen, flag
  CHARACTER(LEN=256) :: dir
  REAL(r8) :: vfg,vfo,vfs,vfp,wfg,wfs,ksol,csol,kdry,ksu,ksf,ba_a,ba_b,tg,vfw,vfi,hcap,thk
  REAL(r8), PARAMETER :: P_VFP(6) = [0.05_r8,0.10_r8,0.25_r8,0.40_r8,0.60_r8,0.90_r8]
  REAL(r8), PARAMETER :: P_VFG(5) = [0.0_r8,0.001_r8,0.2_r8,0.5_r8,0.9_r8]
  REAL(r8), PARAMETER :: P_VFO(3) = [0.0_r8,0.05_r8,0.5_r8]
  REAL(r8), PARAMETER :: P_VFS(5) = [0.0_r8,0.005_r8,0.02_r8,0.25_r8,0.5_r8]
  REAL(r8), PARAMETER :: P_WF(3)  = [0.0_r8,0.5_r8,1.0_r8]
  REAL(r8), PARAMETER :: P_KSOL(3)= [0.1_r8,1.0_r8,8.0_r8]
  REAL(r8), PARAMETER :: P_CSOL(2)= [1.0e5_r8,3.0e6_r8]
  REAL(r8), PARAMETER :: P_KD(3)  = [0.0_r8,0.05_r8,0.5_r8]
  REAL(r8), PARAMETER :: P_KS(3)  = [0.0_r8,0.5_r8,4.0_r8]
  REAL(r8), PARAMETER :: P_BA_A(2)= [0.0_r8,0.5_r8]
  REAL(r8), PARAMETER :: P_BA_B(2)= [1.0_r8,25.0_r8]
  REAL(r8), PARAMETER :: P_TG(7)  = [200.0_r8,250.0_r8,273.15_r8,273.16_r8,300.0_r8,350.0_r8,273.0_r8]
  REAL(r8), PARAMETER :: P_FI(5)  = [0.0_r8,1.0e-12_r8,0.25_r8,0.5_r8,1.0_r8]
  REAL(r8), PARAMETER :: P_FL(5)  = [0.0_r8,1.0e-12_r8,0.01_r8,0.5_r8,1.0_r8]
  S = 20250507_8
  CALL GET_ENVIRONMENT_VARIABLE('HC_OUT', dir, nlen)
  IF (nlen <= 0) dir = '/tmp/gf/hc'
  OPEN(66, FILE=TRIM(dir)//'/hc.txt', STATUS='REPLACE')
  DO k = 1, 8
     DEF_THERMAL_CONDUCTIVITY_SCHEME = k
     DO i = 1, 5000
        IF (i <= 2500) THEN
           vfg = uni()*0.4_r8
           vfo = uni()*0.3_r8
           vfs = uni()*0.3_r8
           vfp = 0.25_r8 + uni()*0.4_r8
           wfg = uni(); wfs = uni()
           ksol = 1.0_r8 + uni()*7.0_r8
           csol = 1.0e6_r8 + uni()*2.0e6_r8
           kdry = 0.05_r8 + uni()*0.45_r8
           ksu = 0.3_r8 + uni()*2.7_r8
           ksf = 0.5_r8 + uni()*3.5_r8
           ba_a = 0.1_r8 + uni()*0.4_r8
           ba_b = 5.0_r8 + uni()*20.0_r8
           tg = 230.0_r8 + uni()*90.0_r8
           vfi = uni()*vfp*0.5_r8
           vfw = uni()*(vfp - vfi)
        ELSE
           vfp = P(P_VFP, 6)
           vfg = P(P_VFG, 5)
           vfo = P(P_VFO, 3)
           vfs = P(P_VFS, 5)
           wfg = P(P_WF, 3); wfs = P(P_WF, 3)
           ksol = P(P_KSOL, 3)
           csol = P(P_CSOL, 2)
           kdry = P(P_KD, 3)
           ksu = P(P_KS, 3)
           ksf = P(P_KS, 3)
           ba_a = P(P_BA_A, 2)
           ba_b = P(P_BA_B, 2)
           tg = P(P_TG, 7)
           vfi = P(P_FI, 5)*vfp
           vfw = P(P_FL, 5)*(vfp - vfi)
        ENDIF
        CALL soil_hcap_cond(vfg,vfo,vfs,vfp,wfg,wfs,ksol,csol,kdry,ksu,ksf, &
                            ba_a,ba_b,tg,vfw,vfi,hcap,thk)
        IF ((vfw+vfi)/vfp >= 1.0e-10_r8) THEN
           flag = 1
        ELSE
           flag = 0
        ENDIF
        WRITE(66,'(I1,I1,1X,2Z17)') k, flag, B(hcap), B(thk)
     ENDDO
  ENDDO
  CLOSE(66)
  PRINT *, 'done'
CONTAINS
  FUNCTION uni() RESULT(v)
    REAL(r8) :: v
    S = S*6364136223846793005_8 + 1442695040888963407_8
    v = REAL(ISHFT(S,-11), r8)/9007199254740992.0_r8
  END FUNCTION uni
  FUNCTION P(pool, n) RESULT(v)
    INTEGER, INTENT(IN) :: n
    REAL(r8), INTENT(IN) :: pool(n)
    REAL(r8) :: v
    ip = MIN(INT(uni()*REAL(n,r8)) + 1, n)
    v = pool(ip)
  END FUNCTION P
  FUNCTION B(x) RESULT(h)
    REAL(r8), INTENT(IN) :: x
    INTEGER(8) :: h
    h = TRANSFER(x,h)
  END FUNCTION B
END PROGRAM hc
