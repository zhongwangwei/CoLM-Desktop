#include <define.h>

SUBROUTINE mt_register_providers ()
!-----------------------------------------------------------------------
!  Species with lifecycle callbacks. An external subroutine rather than a
!  module procedure because the providers USE MOD_MT_Lifecycle, which calls
!  this. A provider attaches only to a configured entry of the descriptor
!  table, so registering it with the species switched off does nothing.
!  Its own file, built with the main objects, because methane needs the
!  BGC soil modules there.
!-----------------------------------------------------------------------
#ifdef GridRiverLakeFlow
   USE MOD_MT_Sediment, only: register_sediment_mt_provider
#endif
   ! BGC 在 CoLM-Desktop 是运行时开关（DEF_USE_BGC），main/BGC/ 始终编译，
   ! 所以这里不能再按编译期 BGC 宏决定是否注册甲烷 provider：那样会在
   ! DEF_USE_BGC = .false. 的内核里把 provider 整个删掉。
   USE MOD_MT_Methane, only: ch4_register_mt_provider
   IMPLICIT NONE
#ifdef GridRiverLakeFlow
   CALL register_sediment_mt_provider ()
#endif
   CALL ch4_register_mt_provider ()
END SUBROUTINE mt_register_providers
