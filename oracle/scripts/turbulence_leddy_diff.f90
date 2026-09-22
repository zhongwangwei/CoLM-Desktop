! MOD_TurbulenceLEddy 的随机差分驱动（LZD2022 大涡近地层方案）。
! 配对物 crates/colm-core/examples/leddy_probe.rs；两侧共用同一串 LCG，
! 抽签次数与顺序必须逐条对齐。
!
! 覆盖模块的两个 PUBLIC 入口：
!   moninobuk_leddy  → 7 个输出（ustar/fh2m/fq2m/fm10m/fm/fh/fq）
!   moninobukm_leddy → 10 个输出（ustar/fh2m/fq2m/fmtop/fm/fh/fq/fht/fqt/phih）
! 行首三个数字是**只在上游侧**算的诊断位（下游不比它，只用来给失配分组）：
!   ib = 0 不稳定 / 1 稳定（obu 的符号）
!   jb = 1 表示 Bm < 0.2722，即 Bm2 = max(Bm,0.2722) 的钳位生效
!   kb = 动量廓线走的那一支（1: zeta<zetam2；2: zetam2<=zeta<0；3: 0<=zeta<=1；4: zeta>1）
!   zc = 1 表示稳定侧 zetazi 被上钳到 200（不稳侧两个钳位在 hu>=1、|obu|<=505 的
!        取值域内不可达，这一列就是为了把这句话变成可核对的计数，而不是估计）
PROGRAM leddy
  USE MOD_TurbulenceLEddy, only: moninobuk_leddy, moninobukm_leddy
  IMPLICIT NONE
  INTEGER, PARAMETER :: r8=8
  INTEGER(8) :: S
  INTEGER :: i, ib, jb, kb, zc, nlen
  CHARACTER(LEN=256) :: dir
  REAL(r8) :: hu,ht,hq,displa,z0m,z0h,z0q,obu,um,hpbl,displat,z0mt,htop
  REAL(r8) :: ustar,fh2m,fq2m,fm10m,fm,fh,fq
  REAL(r8) :: ustar2,fh2m2,fq2m2,fmtop,fm2,fh2,fq2,fht,fqt,phih
  REAL(r8) :: zeta, zetazi, Bm, Bm2, zetam, zetam2
  S = 20250508_8
  CALL GET_ENVIRONMENT_VARIABLE('LEDDY_OUT', dir, nlen)
  IF (nlen <= 0) dir = '/tmp/gf/leddy_diff'
  OPEN(66, FILE=TRIM(dir)//'/leddy.txt', STATUS='REPLACE')
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
     hpbl = 10.0_r8 + uni()*2990.0_r8
     CALL moninobuk_leddy(hu,ht,hq,displa,z0m,z0h,z0q,obu,um,hpbl, &
                          ustar,fh2m,fq2m,fm10m,fm,fh,fq)
     CALL moninobukm_leddy(hu,ht,hq,displa,z0m,z0h,z0q,obu,um,displat,z0mt,hpbl, &
                           ustar2,fh2m2,fq2m2,htop,fmtop,fm2,fh2,fq2,fht,fqt,phih)
     ib = 1
     IF (obu < 0._r8) ib = 0
     zetazi = max(5._r8*hu, hpbl)/obu
     zc = 0
     IF (zetazi >= 0._r8) THEN
        IF (zetazi > 200._r8) zc = 1
        zetazi = min(200._r8, max(zetazi, 1.e-5_r8))
     ELSE
        zetazi = max(-1.e4_r8, min(zetazi, -1.e-5_r8))
     ENDIF
     Bm = 0.0047_r8*(-zetazi) + 0.1854_r8
     jb = 0
     IF (Bm < 0.2722_r8) jb = 1
     Bm2 = max(Bm, 0.2722_r8)
     zetam = 0.5_r8*Bm**4*(-16._r8 - sqrt(256._r8 + 4._r8/Bm**4))
     zetam2 = min(zetam, -0.13_r8)
     zeta = (hu-displa)/obu
     IF (zeta < zetam2) THEN
        kb = 1
     ELSEIF (zeta < 0._r8) THEN
        kb = 2
     ELSEIF (zeta <= 1._r8) THEN
        kb = 3
     ELSE
        kb = 4
     ENDIF
     WRITE(66,'(I1,I1,I1,I1,1X,7Z17,1X,10Z17)') ib, jb, kb, zc, &
        B(ustar),B(fh2m),B(fq2m),B(fm10m),B(fm),B(fh),B(fq), &
        B(ustar2),B(fh2m2),B(fq2m2),B(fmtop),B(fm2),B(fh2),B(fq2),B(fht),B(fqt),B(phih)
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
END PROGRAM leddy
