#include <define.h>

#ifdef TRACER
SUBROUTINE register_all_tracer_providers ()
! mkinidata links the descriptor/lifecycle core but no runtime species.
   IMPLICIT NONE
END SUBROUTINE register_all_tracer_providers
#endif
