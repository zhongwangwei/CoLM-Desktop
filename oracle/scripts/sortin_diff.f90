! `MOD_AssimStomataConductance:sortin` 的随机差分驱动（配对物
! `crates/colm-core/examples/sortin_probe.rs`）。
!
! 为什么单列：第 330–332 轮把 `stomata` 闭环从 498 推到 61，剩下的全在模型 0/1
! 的 `assim`，而它们唯一的共同路径就是这个 `sortin`。`sortin` 在模块里是 **PRIVATE**，
! 所以按第 311/312 轮验证过的"拷贝+放行"路线：把模块拷进 `$WORK`、把
! `PRIVATE :: sortin` 改成 `PUBLIC`，用拷贝编 `.mod`。
!
! 上游侧用内核同款选项编译（**不加** `-ffp-contract=off`），驱动本身按仓库纪律加
! `-fwrapv -ffp-contract=off`。抽签次数与顺序必须与 Rust 侧逐条对齐。
!
! 输出：`ic` 后 21 列十六进制 —— `eyy(1..6)`、`pco2y(1..6)`（调用后），再 9 个中间量
! `ac1,ac2,bc1,bc2,cc1,cc2,bterm,aterm,cterm`（`ic<4` 时为 0）。中间量由闭环脚本往
! **拷贝**里注入的 `dbg_intermediates` 模块数组导出，vendor 源树不动。
PROGRAM srt
  USE MOD_AssimStomataConductance, only: sortin, dbg_intermediates
  IMPLICIT NONE
  INTEGER, PARAMETER :: r8 = 8
  INTEGER, PARAMETER :: IT = 6
  INTEGER(8) :: S
  INTEGER :: i, j, nlen, ic
  REAL(r8) :: eyy(IT), pco2y(IT), range, gammas
  CHARACTER(LEN=256) :: dir

  S = 20260924_8
  CALL GET_ENVIRONMENT_VARIABLE('SRT_OUT', dir, nlen)
  IF (nlen <= 0) dir = '/tmp/gf/srt'
  OPEN(66, FILE=TRIM(dir)//'/srt.txt', STATUS='REPLACE')
  DO i = 1, 3000
     ic = 1 + INT(uni()*6.0_r8)
     range = (uni() - 0.3_r8)*200.0_r8
     gammas = 10.0_r8 + uni()*80.0_r8
     DO j = 1, IT
        eyy(j) = (uni() - 0.5_r8)*200.0_r8
        pco2y(j) = gammas + uni()*range
     ENDDO
     CALL sortin (eyy, pco2y, range, gammas, ic, IT)
     WRITE(66,'(I1,1X,21(1X,Z17))') ic, (B(eyy(j)), B(pco2y(j)), j=1,IT), &
          (B(dbg_intermediates(j)), j=1,9)
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
END PROGRAM srt
