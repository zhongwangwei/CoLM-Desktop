#include <define.h>

MODULE MOD_PHSRootfluxBalance
! Helper routines for keeping plant-hydraulics root uptake consistent with
! the final canopy transpiration in the extended interception pathway.
!
! The extended interception schemes (Noah-MP/MATSIRO/VIC/JULES) can alter the
! wet-canopy evaporation and dry-canopy transpiration weights during the leaf
! temperature solve.  PlantHydraulicStress_twoleaf returns a layer-resolved
! rootflux before the final canopy transpiration is fully settled, so the sum
! of rootflux can differ slightly from etr.  This helper preserves the layer
! partitioning as much as possible while enforcing sum(rootflux) = etr.

   USE MOD_Precision

   IMPLICIT NONE

   PRIVATE
   PUBLIC :: balance_phs_rootflux

CONTAINS

   SUBROUTINE balance_phs_rootflux(ipatch, p_iam_glb, etr, rootflux, fallback_weights, context)
      integer,  intent(in)    :: ipatch
      integer,  intent(in)    :: p_iam_glb
      real(r8), intent(in)    :: etr
      real(r8), intent(inout) :: rootflux(:)
      real(r8), intent(in)    :: fallback_weights(:)
      character(len=*), intent(in), optional :: context

      real(r8), parameter :: tol = 1.e-7_r8
      real(r8), parameter :: tiny_flux = 1.e-15_r8
      integer,  parameter :: warn_limit = 5
      integer,  save :: warn_count = 0
      real(r8) :: sum_flux
      real(r8) :: sum_pos_flux
      real(r8) :: sum_weight

      sum_flux = sum(rootflux)
      IF (abs(etr - sum_flux) <= tol) RETURN

      IF (warn_count < warn_limit) THEN
         IF (present(context)) THEN
            write(6,*) 'Warning: adjusting vegetation PHS rootflux balance ', &
               trim(context), ipatch, p_iam_glb, etr, sum_flux, abs(etr - sum_flux)
         ELSE
            write(6,*) 'Warning: adjusting vegetation PHS rootflux balance ', &
               ipatch, p_iam_glb, etr, sum_flux, abs(etr - sum_flux)
         ENDIF
      ELSEIF (warn_count == warn_limit) THEN
         write(6,*) 'Warning: suppressing further vegetation PHS rootflux balance messages on this MPI task.'
      ENDIF
      warn_count = warn_count + 1

      sum_pos_flux = sum(rootflux, rootflux > 0._r8)
      IF (abs(sum_pos_flux) > tiny_flux) THEN
         rootflux(:) = max(rootflux(:), 0._r8) * (etr / sum_pos_flux)
      ELSE
         sum_weight = sum(fallback_weights)
         IF (abs(sum_weight) > tiny_flux) THEN
            rootflux(:) = etr * fallback_weights(:) / sum_weight
         ELSE
            rootflux(:) = etr / real(size(rootflux), r8)
         ENDIF
      ENDIF

   END SUBROUTINE balance_phs_rootflux

END MODULE MOD_PHSRootfluxBalance
