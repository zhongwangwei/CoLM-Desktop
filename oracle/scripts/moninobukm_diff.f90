! MOD_FrictionVelocity:moninobukm 的随机差分驱动。
! 配对物是 crates/colm-core/examples/mo_probe.rs；两侧用同一串 LCG。
! 用法见 oracle/scripts/compare_moninobukm.sh。
! 实测（2026）：修掉 UNSTABLE_HEAT_COEFFICIENT 的 1 ULP 之后，10 个输出 20000/20000 全同。
! MOD_FrictionVelocity:moninobukm 的随机差分驱动（内核真实选项）
PROGRAM mo
  USE MOD_FrictionVelocity, only: moninobukm, moninobuk, kmoninobuk, kintmoninobuk, moninobukini
  IMPLICIT NONE
  INTEGER, PARAMETER :: r8 = 8
  INTEGER(8) :: S
  REAL(r8) :: hu,ht,hq,displa,z0m,z0h,z0q,obu,um,displat,z0mt,htop
  REAL(r8) :: ustar,fh2m,fq2m,fmtop,fm,fh,fq,fht,fqt,phih,zb
  REAL(r8) :: ustar2,fh2m2,fq2m2,fm10m2,fm2,fh2,fq2
  REAL(r8) :: kcob, kint, um2, obu2, zldis2, th2, thm2, thv2, dth2, dqh2, dthv2
  INTEGER :: ib
  INTEGER :: i
  S = 20250505_8
  OPEN(66, FILE='/tmp/gf/mo_diff/mo.txt', STATUS='REPLACE')
  DO i = 1, 20000
     displa = uni()*5.0_r8
     z0m = 0.001_r8 + uni()
     z0h = 0.001_r8 + uni()
     z0q = 0.001_r8 + uni()
     z0mt = 0.001_r8 + uni()
     htop = displa + 1.0_r8 + uni()*20.0_r8
     displat = displa + uni()*(htop-displa)
     IF (displat + z0mt <= displa) displat = displa + z0mt + 0.5_r8
     hu = displa + 1.0_r8 + uni()*40.0_r8
     ht = displa + 1.0_r8 + uni()*40.0_r8
     hq = displa + 1.0_r8 + uni()*40.0_r8
     um = 0.5_r8 + uni()*10.0_r8
     IF (uni() < 0.5_r8) THEN
        obu = -(5.0_r8 + uni()*500.0_r8)
     ELSE
        obu = 5.0_r8 + uni()*500.0_r8
     ENDIF
     CALL moninobukm(hu,ht,hq,displa,z0m,z0h,z0q,obu,um,displat,z0mt, &
                     ustar,fh2m,fq2m,htop,fmtop,fm,fh,fq,fht,fqt,phih)
     zb = (htop-displa)/obu
     ib = 1
     IF (zb < -0.465_r8) THEN
        ib = 1
     ELSEIF (zb < 0._r8) THEN
        ib = 2
     ELSEIF (zb <= 1._r8) THEN
        ib = 3
     ELSE
        ib = 4
     ENDIF
     CALL moninobuk(hu,ht,hq,displa,z0m,z0h,z0q,obu,um, &
                    ustar2,fh2m2,fq2m2,fm10m2,fm2,fh2,fq2)
     kcob  = kmoninobuk(displa,obu,ustar,ht)
     kint  = kintmoninobuk(displa,z0h,obu,ustar,ht,hq)
     th2   = 290.0_r8 + uni()*20.0_r8
     thm2  = th2
     thv2  = th2*(1.0_r8 + 0.61_r8*0.01_r8)
     dth2  = -2.0_r8 + uni()*4.0_r8
     dqh2  = -0.002_r8 + uni()*0.004_r8
     dthv2 = dth2*(1.0_r8 + 0.61_r8*0.01_r8) + 0.61_r8*th2*dqh2
     zldis2 = hu - displa
     CALL moninobukini(um,th2,thm2,thv2,dth2,dqh2,dthv2,zldis2,z0m,um2,obu2)
     WRITE(66,'(I2,1X,10Z17,1X,7Z17,1X,3Z17)') ib, &
        B(ustar),B(fh2m),B(fq2m),B(fmtop),B(fm),B(fh),B(fq),B(fht),B(fqt),B(phih), &
        B(ustar2),B(fh2m2),B(fq2m2),B(fm10m2),B(fm2),B(fh2),B(fq2), &
        B(kcob),B(kint),B(um2)
  ENDDO
  CLOSE(66)
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
END PROGRAM mo
