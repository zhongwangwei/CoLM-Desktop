program updphotosyn_diff
!-----------------------------------------------------------------------
! `MOD_AssimStomataConductance:update_photosyn` 的单点驱动。
!
! 给 `crates/colm-core/src/photosynthesis_tests.rs` 的
! `hydraulic_photosynthesis_update_matches_mod_assim_stomata_conductance`
! 提供**独立于 Rust** 的内核参考值：同一组合成输入下打印 `assim`/`respc`。
!
! 配置必须与 Rust 测试一致：`DEF_USE_WUEST = .true.`（Rust 侧 `use_wue: true`），
! `DEF_USE_MEDLYNST = .false.`，`c3c4 = 1`（C3，走 `:739` 的 `min(omc,ome)` 分支）。
!
! 编译（见 `oracle/scripts/compare_updphotosyn.sh`）：链 `.bld` 的产线对象，
! 驱动本身用 `-fwrapv -ffp-contract=off`。
!-----------------------------------------------------------------------
   USE MOD_Precision
   USE MOD_AssimStomataConductance
   USE MOD_Namelist, only: DEF_USE_WUEST, DEF_USE_MEDLYNST
   IMPLICIT NONE

   real(r8) :: cint(3), assim, respc
   integer :: c3c4

   DEF_USE_WUEST   = .true.
   DEF_USE_MEDLYNST = .false.
   c3c4 = 1
   cint = (/ 1.2_r8, 0.8_r8, 1.5_r8 /)

   ! 两个导度值：40000 µmol m-2 s-1（= 0.04 mol，物理上等价于旧测试的 0.04）
   ! 与 0.04 µmol m-2 s-1（旧的错误口径），好看出量纲差的影响。
   CALL run(40000._r8)
   CALL run(0.04_r8)

CONTAINS

   SUBROUTINE run(gsh2o)
      real(r8), intent(in) :: gsh2o

      CALL update_photosyn(300._r8, 21200._r8, 40._r8, 39._r8, 200._r8, 101325._r8, &
           0.8_r8, 2.0_r8, gsh2o, &
           0.05_r8, 60.e-6_r8, c3c4, 9.0_r8, 298.16_r8, 0.2_r8, 288.16_r8, &
           0.3_r8, 313.16_r8, 1.3_r8, 328.16_r8, cint, assim, respc)
      WRITE(*,'(A,ES25.17,A,ES25.17)') 'gsh2o=', gsh2o, ' assim=', assim
      WRITE(*,'(A,ES25.17,A,ES25.17)') 'gsh2o=', gsh2o, ' respc=', respc
   END SUBROUTINE run

end program updphotosyn_diff
