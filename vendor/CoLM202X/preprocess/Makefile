# Makefile for preprocess programs
# WARNING: obsolete standalone recipe.
# This Makefile references removed/renamed shared objects
# (../share/precision.o, ../io/spmd_task.o, ../io/ncio_serial.o) and is not
# a supported build path. The shared MOD_Filesystem imports used by current
# sources do not make this legacy recipe runnable; use the top-level Makefile
# unless the preprocessing data tooling is deliberately rebuilt.

include ../include/Makeoptions

all : rawdata_to_nc
.PHONY : all

OBJS_SHARED =  ../share/precision.o \
					../io/spmd_task.o  \
					../io/ncio_serial.o 

OBJS = rawdata_to_nc.o

rawdata_to_nc : $(OBJS) ${OBJS_SHARED}
	${FF} ${FOPTS} ${OBJS_SHARED} ${OBJS} -o $@ ${LDFLAGS}

$(OBJS) : %.o : %.F90 ${OBJS_SHARED}
	${FF} -c ${FOPTS} $(INCLUDE_DIR) -o $@ $< 

$(OBJS_SHARED) : %.o : %.F90
	${FF} -c ${FOPTS} $(INCLUDE_DIR) -o $@ $< 

.PHONY : clean
clean :
	${RM} -f *.o *.mod
