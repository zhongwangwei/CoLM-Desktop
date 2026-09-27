MODULE CMF_CTRL_TIME_MOD
!==========================================================
!* PURPOSE: Manage time-related variables in CaMa-Flood
!
!* CONTAINS:
! -- CMF_TIME_NMLIST : Read setting from namelist
! -- CMF_TIME_INIT   : Initialize    time-related variables
! -- CMF_TIME_NEXT   : Set next-step time-related variables
! -- CMF_TIME_UPDATE : Update        time-related variables
!
! (C) D.Yamazaki & E. Dutra  (U-Tokyo/FCUL)  29Jul 2019
!             Adapted mostly from CMF v362 CONTROL0.F90
!
! Licensed under the Apache License, Version 2.0 (the "License");
!   You may not use this file except in compliance with the License.
!   You may obtain a copy of the License at: http://www.apache.org/licenses/LICENSE-2.0
!
! Unless required by applicable law or agreed to in writing, software distributed under the License is 
!  distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied. 
! See the License for the specific language governing permissions and limitations under the License.
!==========================================================
USE PARKIND1,                ONLY: JPIM, JPIB, JPRB, JPRM
USE YOS_CMF_INPUT,           ONLY: LOGNAM, CSETFILE
USE CMF_UTILS_MOD,           ONLY: MIN2DATE, DATE2MIN, SPLITDATE, SPLITHOUR
!================================================
IMPLICIT NONE
SAVE
!!=== NAMELIST/NSIMTIME/
INTEGER(KIND=JPIM)              :: SYEAR             !! START YEAR
INTEGER(KIND=JPIM)              :: SMON              !! START MONTH
INTEGER(KIND=JPIM)              :: SDAY              !! START DAY
INTEGER(KIND=JPIM)              :: SHOUR             !! START HOUR
INTEGER(KIND=JPIM)              :: SMIN
INTEGER(KIND=JPIM)              :: EYEAR             !! END   YEAR
INTEGER(KIND=JPIM)              :: EMON              !! END   MONTH
INTEGER(KIND=JPIM)              :: EDAY              !! END   DAY
INTEGER(KIND=JPIM)              :: EHOUR             !! END   HOUR 
INTEGER(KIND=JPIM)              :: EMIN

NAMELIST/NSIMTIME/ SYEAR,SMON,SDAY,SHOUR,SMIN, EYEAR,EMON,EDAY,EHOUR,EMIN

CONTAINS 
!####################################################################
! -- CMF_TIME_NMLIST : Read setting from namelist
! -- CMF_TIME_INIT   : Initialize    time-related variables
! -- CMF_TIME_NEXT   : Set next-step time-related variables
! -- CMF_TIME_UPDATE : Update        time-related variables
!
!####################################################################
SUBROUTINE CMF_TIME_NMLIST
! reed setting from namelist
! -- Called from CMF_DRV_NMLIST
!================================================
USE YOS_CMF_INPUT,      ONLY: NSETFILE
USE YOS_CMF_TIME,       ONLY: YYYY0, MM0, DD0
USE CMF_UTILS_MOD,      ONLY: INQUIRE_FID
IMPLICIT NONE
!================================================
WRITE(LOGNAM,*) ""
WRITE(LOGNAM,*) "!---------------------!"

!*** 0. Open namelist
NSETFILE=INQUIRE_FID()
IF( CSETFILE/="NONE" ) OPEN(NSETFILE,FILE=CSETFILE,STATUS="OLD")
IF( CSETFILE/="NONE" ) WRITE(LOGNAM,*) "CMF::TIME_NMLIST: namelist OPEN in unit: ", TRIM(CSETFILE), NSETFILE

!*** 1. set default value
SYEAR=2000
SMON=1
SDAY=1
SHOUR=0
SMIN=0
EYEAR=2001
EMON=1
EDAY=1
EHOUR=0
EMIN=0

!*** 2. read namelist
IF( CSETFILE/="NONE" ) REWIND(NSETFILE)
IF( CSETFILE/="NONE" ) READ(NSETFILE,NML=NSIMTIME)

WRITE(LOGNAM,*) "=== NAMELIST, NSIMTIME ==="
WRITE(LOGNAM,*) "SYEAR,SMON,SDAY,SHOUR,SMIN:", SYEAR,SMON,SDAY,SHOUR,SMIN
WRITE(LOGNAM,*) "EYEAR,EMON,EDAY,EHOUR,EMIN:", EYEAR,EMON,EDAY,EHOUR,EMIN

!*** 3. close namelist
IF( CSETFILE/="NONE" ) CLOSE(NSETFILE)

!*** 4. Define base date for KMIN calculation
YYYY0=SYEAR
MM0=1
DD0=1
WRITE(LOGNAM,*) "TIME_NMLIST: YYYY0 MM0 DD0 set to : ", YYYY0, MM0, DD0

WRITE(LOGNAM,*) "CMF::TIME_NMLIST: end: "

END SUBROUTINE CMF_TIME_NMLIST
!####################################################################

SUBROUTINE CMF_TIME_SET_START_SECONDS(START_SEC)
IMPLICIT NONE
INTEGER(KIND=JPIM),INTENT(IN) :: START_SEC
IF (START_SEC < 0 .OR. START_SEC >= 86400 .OR. MOD(START_SEC,60_JPIM) /= 0) &
  ERROR STOP 'CaMa start time must be a minute within the day'
SHOUR=START_SEC/3600_JPIM
SMIN=MOD(START_SEC,3600_JPIM)/60_JPIM
END SUBROUTINE CMF_TIME_SET_START_SECONDS

SUBROUTINE CMF_TIME_SET_END_SECONDS(END_SEC)
IMPLICIT NONE
INTEGER(KIND=JPIM),INTENT(IN) :: END_SEC
IF (END_SEC < 0 .OR. END_SEC > 86400 .OR. MOD(END_SEC,60_JPIM) /= 0) &
  ERROR STOP 'CaMa end time must be minute-aligned within one day'
EHOUR=END_SEC/3600_JPIM
EMIN=MOD(END_SEC,3600_JPIM)/60_JPIM
END SUBROUTINE CMF_TIME_SET_END_SECONDS





!####################################################################
SUBROUTINE CMF_TIME_INIT
! initialize time-related valiable
! -- Called from CMF_DRV_INIT
!================================================
USE YOS_CMF_INPUT,      ONLY: DT
USE YOS_CMF_TIME,       ONLY: KSTEP, NSTEPS, KMIN, KMINNEXT,  KMINSTART, KMINEND, YYYY0
USE YOS_CMF_TIME,       ONLY: ISYYYYMMDD,ISHHMM,ISYYYY,ISMM,ISDD,ISHOUR,ISMIN     !! start date:hour
USE YOS_CMF_TIME,       ONLY: IEYYYYMMDD,IEHHMM,IEYYYY,IEMM,IEDD,IEHOUR,IEMIN     !! start date:hour
USE YOS_CMF_TIME,       ONLY: IYYYYMMDD, IHHMM, IYYYY, IMM, IDD, IHOUR, IMIN !! date:hour at start of time step
USE YOS_CMF_TIME,       ONLY: JYYYYMMDD, JHHMM, JYYYY, JMM, JDD, JHOUR, JMIN !! date:hour at end   of time step
IMPLICIT NONE
INTEGER(KIND=JPIB) :: ELAPSED_MIN_8, ELAPSED_SEC_8
!================================================
WRITE(LOGNAM,*) ""
WRITE(LOGNAM,*) "!---------------------!"

WRITE(LOGNAM,*) "CMF::TIME_INIT:  initialize time variables"
IF (SMIN < 0 .OR. SMIN >= 60 .OR. EMIN < 0 .OR. EMIN >= 60) &
  ERROR STOP 'CaMa start/end minute is out of range'

!*** 1. Start time & End Time
ISYYYYMMDD=SYEAR*10000+SMON*100+SDAY
ISHHMM=SHOUR*100_JPIM+SMIN
ISYYYY=SYEAR
ISMM  =SMON
ISDD  =SDAY
ISHOUR=SHOUR
ISMIN =SMIN

IEYYYYMMDD=EYEAR*10000+EMON*100+EDAY  !! End   time
IEHHMM=EHOUR*100_JPIM+EMIN
IEYYYY=EYEAR
IEMM  =EMON
IEDD  =EDAY
IEHOUR=EHOUR
IEMIN =EMIN

!*** 2. Initialize KMIN for START & END Time
KMINSTART=DATE2MIN(ISYYYYMMDD,ISHHMM)
KMINEND  =DATE2MIN(IEYYYYMMDD,IEHHMM)

KMIN=KMINSTART

WRITE(LOGNAM,*) 'Base Year YYYY0:',YYYY0
WRITE(LOGNAM,*) 'Start Date:',ISYYYYMMDD, ISHHMM, KMINSTART
WRITE(LOGNAM,*) 'End   Date:',IEYYYYMMDD, IEHHMM, KMINEND

!*** 3. Calculate NSTEPS: time steps within simulation time
KSTEP=0 
IF (CSETFILE=="NONE") THEN
  IF (KMINEND <= KMINSTART .OR. DT <= 0) ERROR STOP 'CaMa invalid simulation window or timestep'
  ELAPSED_MIN_8=INT(KMINEND,KIND=JPIB)-INT(KMINSTART,KIND=JPIB)
  ELAPSED_SEC_8=ELAPSED_MIN_8*60_JPIB
  IF (REAL(ELAPSED_SEC_8,KIND=JPRB)/DT > REAL(HUGE(NSTEPS),KIND=JPRB)) &
    ERROR STOP 'CaMa timestep count exceeds integer range'
  NSTEPS=CEILING(REAL(ELAPSED_SEC_8,KIND=JPRB)/DT,KIND=JPIM)
ELSE
  NSTEPS=int ( ( (KMINEND-KMINSTART)*60_JPIM ) / DT )
ENDIF

WRITE(LOGNAM,*) 'NSTEPS    :',NSTEPS

!*** 4. Initial time step setting
IYYYYMMDD=ISYYYYMMDD
CALL SPLITDATE(IYYYYMMDD,IYYYY,IMM,IDD)
IHHMM=ISHHMM
CALL SPLITHOUR(IHHMM,IHOUR,IMIN)

! tentatively set KMINNEXT to KMIN (just within initialization phase)
KMINNEXT =KMIN
JYYYYMMDD=IYYYYMMDD
JHHMM=IHHMM
CALL SPLITDATE(JYYYYMMDD,JYYYY,JMM,JDD)
CALL SPLITHOUR(JHHMM,JHOUR,JMIN)

WRITE(LOGNAM,*) 'Initial Time Step Date:Hour :', IYYYYMMDD,'_',IHOUR,':',IMIN

!*** end 
WRITE(LOGNAM,*) "CMF::TIME_INIT: end"

END SUBROUTINE CMF_TIME_INIT
!####################################################################





!####################################################################
SUBROUTINE CMF_TIME_NEXT
! update time-related valiable
! -- Called from CMF_DRV_ADVANCE
!================================================
USE YOS_CMF_INPUT,      ONLY: DT
USE YOS_CMF_TIME,       ONLY: KSTEP, KMIN, KMINNEXT
USE YOS_CMF_TIME,       ONLY: IYYYYMMDD, IHHMM                               !! date:hour at start of time step
USE YOS_CMF_TIME,       ONLY: JYYYYMMDD, JYYYY, JMM, JDD, JHHMM, JHOUR, JMIN !! date:hour at end   of time step
IMPLICIT NONE
!================================================
WRITE(LOGNAM,*) ""
!*** 1. Advance KMIN, KSTEP
KSTEP=KSTEP+1
KMINNEXT=KMIN+INT(DT/60,JPIM)

WRITE(LOGNAM,*) "CMF::TIME_NEXT: ", KSTEP, KMIN, KMINNEXT, DT

!*** 2. Update J-time
CALL MIN2DATE(KMINNEXT,JYYYYMMDD,JHHMM)
CALL SPLITDATE(JYYYYMMDD,JYYYY,JMM,JDD)
CALL SPLITHOUR(JHHMM,JHOUR,JMIN)

WRITE(LOGNAM,*) "Strt of Tstep: KMIN,     IYYYYMMDD, IHHMM", KMIN,     IYYYYMMDD, IHHMM
WRITE(LOGNAM,*) "End  of Tstep: KMINNEXT, JYYYYMMDD, JHHMM", KMINNEXT, JYYYYMMDD, JHHMM


END SUBROUTINE CMF_TIME_NEXT
!####################################################################





!####################################################################
SUBROUTINE CMF_TIME_UPDATE
! update time-related valiable
! -- Called from CMF_DRV_ADVANCE
!================================================
USE YOS_CMF_TIME,       ONLY: KMIN, KMINNEXT
USE YOS_CMF_TIME,       ONLY: IYYYYMMDD, IYYYY, IMM, IDD, IHHMM, IHOUR, IMIN !! date:hour at start of time step
USE YOS_CMF_TIME,       ONLY: JYYYYMMDD, JYYYY, JMM, JDD, JHHMM, JHOUR, JMIN !! date:hour at end   of time step
IMPLICIT NONE
!================================================
WRITE(LOGNAM,*) ""
WRITE(LOGNAM,*) "CMF_TIME_UPDATE:"
!*** 1. Advance KMIN, KSTEP
KMIN=KMINNEXT

!*** 2. Update I-time
IYYYYMMDD=JYYYYMMDD
IYYYY=JYYYY
IMM  =JMM
IDD  =JDD
IHHMM=JHHMM
IHOUR=JHOUR
IMIN =JMIN

WRITE(LOGNAM,*) "Current time update: KMIN, IYYYYMMDD, IHHMM", KMIN, IYYYYMMDD, IHHMM

END SUBROUTINE CMF_TIME_UPDATE
!####################################################################

END MODULE CMF_CTRL_TIME_MOD
