! Land-history restart sidecar. This list mirrors the a_* declarations above.
! Only globally allocated fields are persisted; an IO rank need not own vectors.

   SUBROUTINE history_acc_manifest(names, ranks, urban, local, dims, nfields)
   IMPLICIT NONE
   character(len=64), intent(out) :: names(700)
   integer, intent(out) :: ranks(700), local(700), dims(2,700), nfields
   logical, intent(out) :: urban(700)
   integer :: i
      names = ''; ranks = 0; local = 0; dims = 0; urban = .false.; i = 0
      i=i+1; names(i)='nac_ln'; ranks(i)=1; urban(i)=.false.
      IF (allocated(nac_ln)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='nac_dt'; ranks(i)=1; urban(i)=.false.
      IF (allocated(nac_dt)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_us'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_us)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_vs'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_vs)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_t'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_t)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_q'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_q)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_prc'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_prc)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_prl'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_prl)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_pbot'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_pbot)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_frl'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_frl)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_solarin'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_solarin)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_hpbl'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_hpbl)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_taux'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_taux)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_tauy'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_tauy)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_fsena'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_fsena)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_lfevpa'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_lfevpa)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_fevpa'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_fevpa)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_fsenl'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_fsenl)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_fevpl'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_fevpl)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_etr'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_etr)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_fseng'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_fseng)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_fevpg'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_fevpg)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_fgrnd'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_fgrnd)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_sabvsun'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_sabvsun)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_sabvsha'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_sabvsha)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_sabg'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_sabg)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_olrg'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_olrg)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_rnet'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_rnet)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_xerr'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_xerr)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_zerr'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_zerr)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_rsur'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_rsur)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_rsur_se'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_rsur_se)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_rsur_ie'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_rsur_ie)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_rsub'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_rsub)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_rnof'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_rnof)) THEN
         local(i)=1
      ENDIF
#if (defined CatchLateralFlow)
      i=i+1; names(i)='a_xwsur'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_xwsur)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_xwsub'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_xwsub)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_fldarea'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_fldarea)) THEN
         local(i)=1
      ENDIF
#endif
      i=i+1; names(i)='a_qintr'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_qintr)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_qinfl'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_qinfl)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_qdrip'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_qdrip)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_rstfacsun'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_rstfacsun)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_rstfacsha'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_rstfacsha)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_gssun'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_gssun)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_gssha'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_gssha)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_rss'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_rss)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_wdsrf'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_wdsrf)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_zwt'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_zwt)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_wa'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_wa)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_wat'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_wat)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_wetwat'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_wetwat)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_assim'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_assim)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_respc'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_respc)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_assimsun'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_assimsun)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_assimsha'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_assimsha)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_etrsun'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_etrsun)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_etrsha'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_etrsha)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_qcharge'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_qcharge)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_t_grnd'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_t_grnd)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_tleaf'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_tleaf)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_ldew'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_ldew)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_ldew_rain'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_ldew_rain)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_ldew_snow'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_ldew_snow)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_scv'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_scv)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_snowdp'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_snowdp)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_fsno'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_fsno)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_frcsat'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_frcsat)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_sigf'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_sigf)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_green'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_green)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_lai'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_lai)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_laisun'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_laisun)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_laisha'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_laisha)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_sai'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_sai)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_alb'; ranks(i)=3; urban(i)=.false.
      IF (allocated(a_alb)) THEN
         local(i)=1
         dims(1,i)=size(a_alb,1)
         dims(2,i)=size(a_alb,2)
      ENDIF
#if (defined HYPERSPECTRAL)
      i=i+1; names(i)='a_alb_hires'; ranks(i)=3; urban(i)=.false.
      IF (allocated(a_alb_hires)) THEN
         local(i)=1
         dims(1,i)=size(a_alb_hires,1)
         dims(2,i)=size(a_alb_hires,2)
      ENDIF
      i=i+1; names(i)='a_reflectance_out'; ranks(i)=3; urban(i)=.false.
      IF (allocated(a_reflectance_out)) THEN
         local(i)=1
         dims(1,i)=size(a_reflectance_out,1)
         dims(2,i)=size(a_reflectance_out,2)
      ENDIF
      i=i+1; names(i)='a_transmittance_out'; ranks(i)=3; urban(i)=.false.
      IF (allocated(a_transmittance_out)) THEN
         local(i)=1
         dims(1,i)=size(a_transmittance_out,1)
         dims(2,i)=size(a_transmittance_out,2)
      ENDIF
#endif
      i=i+1; names(i)='a_emis'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_emis)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_z0m'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_z0m)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_trad'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_trad)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_tref'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_tref)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_t2m_wmo'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_t2m_wmo)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_qref'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_qref)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_rain'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_rain)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_snow'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_snow)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_o3uptakesun'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_o3uptakesun)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_o3uptakesha'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_o3uptakesha)) THEN
         local(i)=1
      ENDIF
#if (defined DataAssimilation)
      i=i+1; names(i)='a_h2osoi_ens'; ranks(i)=3; urban(i)=.false.
      IF (allocated(a_h2osoi_ens)) THEN
         local(i)=1
         dims(1,i)=size(a_h2osoi_ens,1)
         dims(2,i)=size(a_h2osoi_ens,2)
      ENDIF
      i=i+1; names(i)='a_t_brt_smap_ens'; ranks(i)=3; urban(i)=.false.
      IF (allocated(a_t_brt_smap_ens)) THEN
         local(i)=1
         dims(1,i)=size(a_t_brt_smap_ens,1)
         dims(2,i)=size(a_t_brt_smap_ens,2)
      ENDIF
      i=i+1; names(i)='a_t_brt_fy3d_ens'; ranks(i)=3; urban(i)=.false.
      IF (allocated(a_t_brt_fy3d_ens)) THEN
         local(i)=1
         dims(1,i)=size(a_t_brt_fy3d_ens,1)
         dims(2,i)=size(a_t_brt_fy3d_ens,2)
      ENDIF
      i=i+1; names(i)='a_t_brt_smap'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_t_brt_smap)) THEN
         local(i)=1
         dims(1,i)=size(a_t_brt_smap,1)
      ENDIF
      i=i+1; names(i)='a_t_brt_fy3d'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_t_brt_fy3d)) THEN
         local(i)=1
         dims(1,i)=size(a_t_brt_fy3d,1)
      ENDIF
      i=i+1; names(i)='a_wliq_soisno_ens'; ranks(i)=3; urban(i)=.false.
      IF (allocated(a_wliq_soisno_ens)) THEN
         local(i)=1
         dims(1,i)=size(a_wliq_soisno_ens,1)
         dims(2,i)=size(a_wliq_soisno_ens,2)
      ENDIF
      i=i+1; names(i)='a_wice_soisno_ens'; ranks(i)=3; urban(i)=.false.
      IF (allocated(a_wice_soisno_ens)) THEN
         local(i)=1
         dims(1,i)=size(a_wice_soisno_ens,1)
         dims(2,i)=size(a_wice_soisno_ens,2)
      ENDIF
      i=i+1; names(i)='a_t_soisno_ens'; ranks(i)=3; urban(i)=.false.
      IF (allocated(a_t_soisno_ens)) THEN
         local(i)=1
         dims(1,i)=size(a_t_soisno_ens,1)
         dims(2,i)=size(a_t_soisno_ens,2)
      ENDIF
#endif
      IF ((DEF_URBAN_RUN)) THEN
      i=i+1; names(i)='a_t_room'; ranks(i)=1; urban(i)=.true.
      IF (allocated(a_t_room)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_tafu'; ranks(i)=1; urban(i)=.true.
      IF (allocated(a_tafu)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_fhac'; ranks(i)=1; urban(i)=.true.
      IF (allocated(a_fhac)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_fwst'; ranks(i)=1; urban(i)=.true.
      IF (allocated(a_fwst)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_fach'; ranks(i)=1; urban(i)=.true.
      IF (allocated(a_fach)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_fahe'; ranks(i)=1; urban(i)=.true.
      IF (allocated(a_fahe)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_fhah'; ranks(i)=1; urban(i)=.true.
      IF (allocated(a_fhah)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_vehc'; ranks(i)=1; urban(i)=.true.
      IF (allocated(a_vehc)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_meta'; ranks(i)=1; urban(i)=.true.
      IF (allocated(a_meta)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_senroof'; ranks(i)=1; urban(i)=.true.
      IF (allocated(a_senroof)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_senwsun'; ranks(i)=1; urban(i)=.true.
      IF (allocated(a_senwsun)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_senwsha'; ranks(i)=1; urban(i)=.true.
      IF (allocated(a_senwsha)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_sengimp'; ranks(i)=1; urban(i)=.true.
      IF (allocated(a_sengimp)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_sengper'; ranks(i)=1; urban(i)=.true.
      IF (allocated(a_sengper)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_senurbl'; ranks(i)=1; urban(i)=.true.
      IF (allocated(a_senurbl)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_lfevproof'; ranks(i)=1; urban(i)=.true.
      IF (allocated(a_lfevproof)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_lfevpgimp'; ranks(i)=1; urban(i)=.true.
      IF (allocated(a_lfevpgimp)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_lfevpgper'; ranks(i)=1; urban(i)=.true.
      IF (allocated(a_lfevpgper)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_lfevpurbl'; ranks(i)=1; urban(i)=.true.
      IF (allocated(a_lfevpurbl)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_troof'; ranks(i)=1; urban(i)=.true.
      IF (allocated(a_troof)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_twall'; ranks(i)=1; urban(i)=.true.
      IF (allocated(a_twall)) THEN
         local(i)=1
      ENDIF
      ENDIF
      IF (((DEF_USE_PFT .or. DEF_USE_PC))) THEN
      i=i+1; names(i)='a_lai_enftemp'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_lai_enftemp)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_lai_enfboreal'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_lai_enfboreal)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_lai_dnfboreal'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_lai_dnfboreal)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_lai_ebftrop'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_lai_ebftrop)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_lai_ebftemp'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_lai_ebftemp)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_lai_dbftrop'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_lai_dbftrop)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_lai_dbftemp'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_lai_dbftemp)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_lai_dbfboreal'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_lai_dbfboreal)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_lai_ebstemp'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_lai_ebstemp)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_lai_dbstemp'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_lai_dbstemp)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_lai_dbsboreal'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_lai_dbsboreal)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_lai_c3arcgrass'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_lai_c3arcgrass)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_lai_c3grass'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_lai_c3grass)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_lai_c4grass'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_lai_c4grass)) THEN
         local(i)=1
      ENDIF
      ENDIF
      IF ((DEF_USE_BGC)) THEN
      i=i+1; names(i)='a_leafc'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_leafc)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_leafc_storage'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_leafc_storage)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_leafc_xfer'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_leafc_xfer)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_frootc'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_frootc)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_frootc_storage'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_frootc_storage)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_frootc_xfer'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_frootc_xfer)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_livestemc'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_livestemc)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_livestemc_storage'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_livestemc_storage)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_livestemc_xfer'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_livestemc_xfer)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_deadstemc'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_deadstemc)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_deadstemc_storage'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_deadstemc_storage)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_deadstemc_xfer'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_deadstemc_xfer)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_livecrootc'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_livecrootc)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_livecrootc_storage'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_livecrootc_storage)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_livecrootc_xfer'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_livecrootc_xfer)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_deadcrootc'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_deadcrootc)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_deadcrootc_storage'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_deadcrootc_storage)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_deadcrootc_xfer'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_deadcrootc_xfer)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_grainc'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_grainc)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_grainc_storage'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_grainc_storage)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_grainc_xfer'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_grainc_xfer)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_leafn'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_leafn)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_leafn_storage'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_leafn_storage)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_leafn_xfer'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_leafn_xfer)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_frootn'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_frootn)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_frootn_storage'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_frootn_storage)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_frootn_xfer'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_frootn_xfer)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_livestemn'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_livestemn)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_livestemn_storage'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_livestemn_storage)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_livestemn_xfer'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_livestemn_xfer)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_deadstemn'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_deadstemn)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_deadstemn_storage'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_deadstemn_storage)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_deadstemn_xfer'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_deadstemn_xfer)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_livecrootn'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_livecrootn)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_livecrootn_storage'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_livecrootn_storage)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_livecrootn_xfer'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_livecrootn_xfer)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_deadcrootn'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_deadcrootn)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_deadcrootn_storage'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_deadcrootn_storage)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_deadcrootn_xfer'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_deadcrootn_xfer)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_grainn'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_grainn)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_grainn_storage'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_grainn_storage)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_grainn_xfer'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_grainn_xfer)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_retransn'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_retransn)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_gpp'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_gpp)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_downreg'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_downreg)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_ar'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_ar)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_cwdprod'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_cwdprod)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_cwddecomp'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_cwddecomp)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_hr'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_hr)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_fpg'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_fpg)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_fpi'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_fpi)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_totvegc'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_totvegc)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_totlitc'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_totlitc)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_totcwdc'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_totcwdc)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_totsomc'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_totsomc)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_totcolc'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_totcolc)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_totvegn'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_totvegn)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_totlitn'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_totlitn)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_totcwdn'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_totcwdn)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_totsomn'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_totsomn)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_totcoln'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_totcoln)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_gpp_enftemp'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_gpp_enftemp)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_gpp_enfboreal'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_gpp_enfboreal)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_gpp_dnfboreal'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_gpp_dnfboreal)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_gpp_ebftrop'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_gpp_ebftrop)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_gpp_ebftemp'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_gpp_ebftemp)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_gpp_dbftrop'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_gpp_dbftrop)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_gpp_dbftemp'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_gpp_dbftemp)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_gpp_dbfboreal'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_gpp_dbfboreal)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_gpp_ebstemp'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_gpp_ebstemp)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_gpp_dbstemp'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_gpp_dbstemp)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_gpp_dbsboreal'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_gpp_dbsboreal)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_gpp_c3arcgrass'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_gpp_c3arcgrass)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_gpp_c3grass'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_gpp_c3grass)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_gpp_c4grass'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_gpp_c4grass)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_npp_enftemp'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_npp_enftemp)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_npp_enfboreal'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_npp_enfboreal)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_npp_dnfboreal'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_npp_dnfboreal)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_npp_ebftrop'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_npp_ebftrop)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_npp_ebftemp'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_npp_ebftemp)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_npp_dbftrop'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_npp_dbftrop)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_npp_dbftemp'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_npp_dbftemp)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_npp_dbfboreal'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_npp_dbfboreal)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_npp_ebstemp'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_npp_ebstemp)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_npp_dbstemp'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_npp_dbstemp)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_npp_dbsboreal'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_npp_dbsboreal)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_npp_c3arcgrass'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_npp_c3arcgrass)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_npp_c3grass'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_npp_c3grass)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_npp_c4grass'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_npp_c4grass)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_npptoleafc_enftemp'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_npptoleafc_enftemp)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_npptoleafc_enfboreal'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_npptoleafc_enfboreal)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_npptoleafc_dnfboreal'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_npptoleafc_dnfboreal)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_npptoleafc_ebftrop'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_npptoleafc_ebftrop)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_npptoleafc_ebftemp'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_npptoleafc_ebftemp)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_npptoleafc_dbftrop'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_npptoleafc_dbftrop)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_npptoleafc_dbftemp'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_npptoleafc_dbftemp)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_npptoleafc_dbfboreal'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_npptoleafc_dbfboreal)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_npptoleafc_ebstemp'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_npptoleafc_ebstemp)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_npptoleafc_dbstemp'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_npptoleafc_dbstemp)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_npptoleafc_dbsboreal'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_npptoleafc_dbsboreal)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_npptoleafc_c3arcgrass'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_npptoleafc_c3arcgrass)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_npptoleafc_c3grass'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_npptoleafc_c3grass)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_npptoleafc_c4grass'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_npptoleafc_c4grass)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_leafc_enftemp'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_leafc_enftemp)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_leafc_enfboreal'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_leafc_enfboreal)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_leafc_dnfboreal'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_leafc_dnfboreal)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_leafc_ebftrop'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_leafc_ebftrop)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_leafc_ebftemp'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_leafc_ebftemp)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_leafc_dbftrop'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_leafc_dbftrop)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_leafc_dbftemp'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_leafc_dbftemp)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_leafc_dbfboreal'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_leafc_dbfboreal)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_leafc_ebstemp'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_leafc_ebstemp)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_leafc_dbstemp'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_leafc_dbstemp)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_leafc_dbsboreal'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_leafc_dbsboreal)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_leafc_c3arcgrass'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_leafc_c3arcgrass)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_leafc_c3grass'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_leafc_c3grass)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_leafc_c4grass'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_leafc_c4grass)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_O2_DECOMP_DEPTH_UNSAT'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_O2_DECOMP_DEPTH_UNSAT)) THEN
         local(i)=1
         dims(1,i)=size(a_O2_DECOMP_DEPTH_UNSAT,1)
      ENDIF
      i=i+1; names(i)='a_CONC_O2_UNSAT'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_CONC_O2_UNSAT)) THEN
         local(i)=1
         dims(1,i)=size(a_CONC_O2_UNSAT,1)
      ENDIF
      ENDIF
#ifdef CROP
      IF (DEF_USE_BGC) THEN
      i=i+1; names(i)='a_pdcorn'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_pdcorn)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_pdswheat'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_pdswheat)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_pdwwheat'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_pdwwheat)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_pdsoybean'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_pdsoybean)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_pdcotton'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_pdcotton)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_pdrice1'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_pdrice1)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_pdrice2'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_pdrice2)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_pdsugarcane'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_pdsugarcane)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_plantdate'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_plantdate)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_manunitro'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_manunitro)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_fertnitro_corn'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_fertnitro_corn)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_fertnitro_swheat'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_fertnitro_swheat)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_fertnitro_wwheat'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_fertnitro_wwheat)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_fertnitro_soybean'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_fertnitro_soybean)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_fertnitro_cotton'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_fertnitro_cotton)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_fertnitro_rice1'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_fertnitro_rice1)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_fertnitro_rice2'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_fertnitro_rice2)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_fertnitro_sugarcane'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_fertnitro_sugarcane)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_irrig_method_corn'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_irrig_method_corn)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_irrig_method_swheat'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_irrig_method_swheat)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_irrig_method_wwheat'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_irrig_method_wwheat)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_irrig_method_soybean'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_irrig_method_soybean)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_irrig_method_cotton'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_irrig_method_cotton)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_irrig_method_rice1'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_irrig_method_rice1)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_irrig_method_rice2'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_irrig_method_rice2)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_irrig_method_sugarcane'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_irrig_method_sugarcane)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_cphase'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_cphase)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_gddplant'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_gddplant)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_gddmaturity'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_gddmaturity)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_vf'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_vf)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_hui'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_hui)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_cropprod1c'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_cropprod1c)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_cropprod1c_loss'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_cropprod1c_loss)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_cropseedc_deficit'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_cropseedc_deficit)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_grainc_to_cropprodc'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_grainc_to_cropprodc)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_grainc_to_seed'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_grainc_to_seed)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_fert_to_sminn'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_fert_to_sminn)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_sum_irrig'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_sum_irrig)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_sum_deficit_irrig'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_sum_deficit_irrig)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_sum_irrig_count'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_sum_irrig_count)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_waterstorage'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_waterstorage)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_groundwater_demand'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_groundwater_demand)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_groundwater_supply'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_groundwater_supply)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_reservoirriver_demand'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_reservoirriver_demand)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_reservoirriver_supply'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_reservoirriver_supply)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_reservoir_supply'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_reservoir_supply)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_river_supply'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_river_supply)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_runoff_supply'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_runoff_supply)) THEN
         local(i)=1
      ENDIF
      ENDIF
#endif
      IF ((DEF_USE_BGC)) THEN
      i=i+1; names(i)='a_ndep_to_sminn'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_ndep_to_sminn)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_abm'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_abm)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_gdp'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_gdp)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_peatf'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_peatf)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_hdm'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_hdm)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_lnfm'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_lnfm)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_leafcCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_leafcCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_leafc_storageCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_leafc_storageCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_leafc_xferCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_leafc_xferCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_frootcCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_frootcCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_frootc_storageCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_frootc_storageCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_frootc_xferCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_frootc_xferCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_livestemcCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_livestemcCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_livestemc_storageCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_livestemc_storageCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_livestemc_xferCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_livestemc_xferCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_deadstemcCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_deadstemcCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_deadstemc_storageCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_deadstemc_storageCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_deadstemc_xferCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_deadstemc_xferCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_livecrootcCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_livecrootcCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_livecrootc_storageCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_livecrootc_storageCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_livecrootc_xferCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_livecrootc_xferCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_deadcrootcCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_deadcrootcCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_deadcrootc_storageCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_deadcrootc_storageCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_deadcrootc_xferCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_deadcrootc_xferCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_leafnCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_leafnCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_leafn_storageCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_leafn_storageCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_leafn_xferCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_leafn_xferCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_frootnCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_frootnCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_frootn_storageCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_frootn_storageCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_frootn_xferCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_frootn_xferCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_livestemnCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_livestemnCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_livestemn_storageCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_livestemn_storageCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_livestemn_xferCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_livestemn_xferCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_deadstemnCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_deadstemnCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_deadstemn_storageCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_deadstemn_storageCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_deadstemn_xferCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_deadstemn_xferCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_livecrootnCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_livecrootnCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_livecrootn_storageCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_livecrootn_storageCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_livecrootn_xferCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_livecrootn_xferCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_deadcrootnCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_deadcrootnCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_deadcrootn_storageCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_deadcrootn_storageCap)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_deadcrootn_xferCap'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_deadcrootn_xferCap)) THEN
         local(i)=1
      ENDIF
      ENDIF
      i=i+1; names(i)='a_ozone'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_ozone)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_t_soisno'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_t_soisno)) THEN
         local(i)=1
         dims(1,i)=size(a_t_soisno,1)
      ENDIF
      i=i+1; names(i)='a_wliq_soisno'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_wliq_soisno)) THEN
         local(i)=1
         dims(1,i)=size(a_wliq_soisno,1)
      ENDIF
      i=i+1; names(i)='a_wice_soisno'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_wice_soisno)) THEN
         local(i)=1
         dims(1,i)=size(a_wice_soisno,1)
      ENDIF
      i=i+1; names(i)='a_h2osoi'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_h2osoi)) THEN
         local(i)=1
         dims(1,i)=size(a_h2osoi,1)
      ENDIF
      i=i+1; names(i)='a_qlayer'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_qlayer)) THEN
         local(i)=1
         dims(1,i)=size(a_qlayer,1)
      ENDIF
      i=i+1; names(i)='a_lake_deficit'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_lake_deficit)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_rootr'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_rootr)) THEN
         local(i)=1
         dims(1,i)=size(a_rootr,1)
      ENDIF
      i=i+1; names(i)='a_BD_all'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_BD_all)) THEN
         local(i)=1
         dims(1,i)=size(a_BD_all,1)
      ENDIF
      i=i+1; names(i)='a_wfc'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_wfc)) THEN
         local(i)=1
         dims(1,i)=size(a_wfc,1)
      ENDIF
      i=i+1; names(i)='a_OM_density'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_OM_density)) THEN
         local(i)=1
         dims(1,i)=size(a_OM_density,1)
      ENDIF
      i=i+1; names(i)='a_vegwp'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_vegwp)) THEN
         local(i)=1
         dims(1,i)=size(a_vegwp,1)
      ENDIF
      i=i+1; names(i)='a_dz_lake'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_dz_lake)) THEN
         local(i)=1
         dims(1,i)=size(a_dz_lake,1)
      ENDIF
      i=i+1; names(i)='a_t_lake'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_t_lake)) THEN
         local(i)=1
         dims(1,i)=size(a_t_lake,1)
      ENDIF
      i=i+1; names(i)='a_lake_icefrac'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_lake_icefrac)) THEN
         local(i)=1
         dims(1,i)=size(a_lake_icefrac,1)
      ENDIF
      IF ((DEF_USE_BGC)) THEN
      i=i+1; names(i)='a_litr1c_vr'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_litr1c_vr)) THEN
         local(i)=1
         dims(1,i)=size(a_litr1c_vr,1)
      ENDIF
      i=i+1; names(i)='a_litr2c_vr'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_litr2c_vr)) THEN
         local(i)=1
         dims(1,i)=size(a_litr2c_vr,1)
      ENDIF
      i=i+1; names(i)='a_litr3c_vr'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_litr3c_vr)) THEN
         local(i)=1
         dims(1,i)=size(a_litr3c_vr,1)
      ENDIF
      i=i+1; names(i)='a_soil1c_vr'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_soil1c_vr)) THEN
         local(i)=1
         dims(1,i)=size(a_soil1c_vr,1)
      ENDIF
      i=i+1; names(i)='a_soil2c_vr'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_soil2c_vr)) THEN
         local(i)=1
         dims(1,i)=size(a_soil2c_vr,1)
      ENDIF
      i=i+1; names(i)='a_soil3c_vr'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_soil3c_vr)) THEN
         local(i)=1
         dims(1,i)=size(a_soil3c_vr,1)
      ENDIF
      i=i+1; names(i)='a_cwdc_vr'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_cwdc_vr)) THEN
         local(i)=1
         dims(1,i)=size(a_cwdc_vr,1)
      ENDIF
      i=i+1; names(i)='a_litr1n_vr'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_litr1n_vr)) THEN
         local(i)=1
         dims(1,i)=size(a_litr1n_vr,1)
      ENDIF
      i=i+1; names(i)='a_litr2n_vr'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_litr2n_vr)) THEN
         local(i)=1
         dims(1,i)=size(a_litr2n_vr,1)
      ENDIF
      i=i+1; names(i)='a_litr3n_vr'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_litr3n_vr)) THEN
         local(i)=1
         dims(1,i)=size(a_litr3n_vr,1)
      ENDIF
      i=i+1; names(i)='a_soil1n_vr'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_soil1n_vr)) THEN
         local(i)=1
         dims(1,i)=size(a_soil1n_vr,1)
      ENDIF
      i=i+1; names(i)='a_soil2n_vr'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_soil2n_vr)) THEN
         local(i)=1
         dims(1,i)=size(a_soil2n_vr,1)
      ENDIF
      i=i+1; names(i)='a_soil3n_vr'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_soil3n_vr)) THEN
         local(i)=1
         dims(1,i)=size(a_soil3n_vr,1)
      ENDIF
      i=i+1; names(i)='a_cwdn_vr'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_cwdn_vr)) THEN
         local(i)=1
         dims(1,i)=size(a_cwdn_vr,1)
      ENDIF
      i=i+1; names(i)='a_totsoiln_vr'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_totsoiln_vr)) THEN
         local(i)=1
         dims(1,i)=size(a_totsoiln_vr,1)
      ENDIF
      i=i+1; names(i)='a_litr1cCap_vr'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_litr1cCap_vr)) THEN
         local(i)=1
         dims(1,i)=size(a_litr1cCap_vr,1)
      ENDIF
      i=i+1; names(i)='a_litr2cCap_vr'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_litr2cCap_vr)) THEN
         local(i)=1
         dims(1,i)=size(a_litr2cCap_vr,1)
      ENDIF
      i=i+1; names(i)='a_litr3cCap_vr'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_litr3cCap_vr)) THEN
         local(i)=1
         dims(1,i)=size(a_litr3cCap_vr,1)
      ENDIF
      i=i+1; names(i)='a_soil1cCap_vr'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_soil1cCap_vr)) THEN
         local(i)=1
         dims(1,i)=size(a_soil1cCap_vr,1)
      ENDIF
      i=i+1; names(i)='a_soil2cCap_vr'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_soil2cCap_vr)) THEN
         local(i)=1
         dims(1,i)=size(a_soil2cCap_vr,1)
      ENDIF
      i=i+1; names(i)='a_soil3cCap_vr'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_soil3cCap_vr)) THEN
         local(i)=1
         dims(1,i)=size(a_soil3cCap_vr,1)
      ENDIF
      i=i+1; names(i)='a_cwdcCap_vr'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_cwdcCap_vr)) THEN
         local(i)=1
         dims(1,i)=size(a_cwdcCap_vr,1)
      ENDIF
      i=i+1; names(i)='a_litr1nCap_vr'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_litr1nCap_vr)) THEN
         local(i)=1
         dims(1,i)=size(a_litr1nCap_vr,1)
      ENDIF
      i=i+1; names(i)='a_litr2nCap_vr'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_litr2nCap_vr)) THEN
         local(i)=1
         dims(1,i)=size(a_litr2nCap_vr,1)
      ENDIF
      i=i+1; names(i)='a_litr3nCap_vr'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_litr3nCap_vr)) THEN
         local(i)=1
         dims(1,i)=size(a_litr3nCap_vr,1)
      ENDIF
      i=i+1; names(i)='a_soil1nCap_vr'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_soil1nCap_vr)) THEN
         local(i)=1
         dims(1,i)=size(a_soil1nCap_vr,1)
      ENDIF
      i=i+1; names(i)='a_soil2nCap_vr'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_soil2nCap_vr)) THEN
         local(i)=1
         dims(1,i)=size(a_soil2nCap_vr,1)
      ENDIF
      i=i+1; names(i)='a_soil3nCap_vr'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_soil3nCap_vr)) THEN
         local(i)=1
         dims(1,i)=size(a_soil3nCap_vr,1)
      ENDIF
      i=i+1; names(i)='a_cwdnCap_vr'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_cwdnCap_vr)) THEN
         local(i)=1
         dims(1,i)=size(a_cwdnCap_vr,1)
      ENDIF
      i=i+1; names(i)='a_t_scalar'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_t_scalar)) THEN
         local(i)=1
         dims(1,i)=size(a_t_scalar,1)
      ENDIF
      i=i+1; names(i)='a_w_scalar'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_w_scalar)) THEN
         local(i)=1
         dims(1,i)=size(a_w_scalar,1)
      ENDIF
      i=i+1; names(i)='a_sminn_vr'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_sminn_vr)) THEN
         local(i)=1
         dims(1,i)=size(a_sminn_vr,1)
      ENDIF
      ENDIF
      i=i+1; names(i)='a_ustar'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_ustar)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_ustar2'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_ustar2)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_tstar'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_tstar)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_qstar'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_qstar)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_zol'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_zol)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_rib'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_rib)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_fm'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_fm)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_fh'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_fh)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_fq'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_fq)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_us10m'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_us10m)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_vs10m'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_vs10m)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_fm10m'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_fm10m)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_sr'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_sr)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_solvd'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_solvd)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_solvi'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_solvi)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_solnd'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_solnd)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_solni'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_solni)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_srvd'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_srvd)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_srvi'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_srvi)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_srnd'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_srnd)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_srni'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_srni)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_solvdln'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_solvdln)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_solviln'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_solviln)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_solndln'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_solndln)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_solniln'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_solniln)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_srvdln'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_srvdln)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_srviln'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_srviln)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_srndln'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_srndln)) THEN
         local(i)=1
      ENDIF
      i=i+1; names(i)='a_srniln'; ranks(i)=1; urban(i)=.false.
      IF (allocated(a_srniln)) THEN
         local(i)=1
      ENDIF
#if (defined HYPERSPECTRAL)
      i=i+1; names(i)='a_sol_dir_ln_hires'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_sol_dir_ln_hires)) THEN
         local(i)=1
         dims(1,i)=size(a_sol_dir_ln_hires,1)
      ENDIF
      i=i+1; names(i)='a_sol_dif_ln_hires'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_sol_dif_ln_hires)) THEN
         local(i)=1
         dims(1,i)=size(a_sol_dif_ln_hires,1)
      ENDIF
      i=i+1; names(i)='a_sr_dir_ln_hires'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_sr_dir_ln_hires)) THEN
         local(i)=1
         dims(1,i)=size(a_sr_dir_ln_hires,1)
      ENDIF
      i=i+1; names(i)='a_sr_dif_ln_hires'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_sr_dif_ln_hires)) THEN
         local(i)=1
         dims(1,i)=size(a_sr_dif_ln_hires,1)
      ENDIF
#endif
      i=i+1; names(i)='a_sensors'; ranks(i)=2; urban(i)=.false.
      IF (allocated(a_sensors)) THEN
         local(i)=1
         dims(1,i)=size(a_sensors,1)
      ENDIF
      nfields=i
   END SUBROUTINE history_acc_manifest

   SUBROUTINE history_acc_transfer(file, writing, present_flags, dims)
   USE MOD_NetCDFVector
   USE MOD_LandPatch, only: landpatch
   USE MOD_LandUrban, only: landurban
   IMPLICIT NONE
   character(len=*), intent(in) :: file
   logical, intent(in) :: writing
   integer, intent(in) :: present_flags(700), dims(2,700)
   integer :: i
      i=0
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'nac_ln', landpatch, 'patch', nac_ln, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'nac_ln', landpatch, nac_ln, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'nac_dt', landpatch, 'patch', nac_dt, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'nac_dt', landpatch, nac_dt, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_us', landpatch, 'patch', a_us, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_us', landpatch, a_us, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_vs', landpatch, 'patch', a_vs, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_vs', landpatch, a_vs, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_t', landpatch, 'patch', a_t, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_t', landpatch, a_t, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_q', landpatch, 'patch', a_q, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_q', landpatch, a_q, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_prc', landpatch, 'patch', a_prc, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_prc', landpatch, a_prc, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_prl', landpatch, 'patch', a_prl, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_prl', landpatch, a_prl, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_pbot', landpatch, 'patch', a_pbot, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_pbot', landpatch, a_pbot, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_frl', landpatch, 'patch', a_frl, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_frl', landpatch, a_frl, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_solarin', landpatch, 'patch', a_solarin, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_solarin', landpatch, a_solarin, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_hpbl', landpatch, 'patch', a_hpbl, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_hpbl', landpatch, a_hpbl, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_taux', landpatch, 'patch', a_taux, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_taux', landpatch, a_taux, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_tauy', landpatch, 'patch', a_tauy, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_tauy', landpatch, a_tauy, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_fsena', landpatch, 'patch', a_fsena, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_fsena', landpatch, a_fsena, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_lfevpa', landpatch, 'patch', a_lfevpa, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_lfevpa', landpatch, a_lfevpa, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_fevpa', landpatch, 'patch', a_fevpa, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_fevpa', landpatch, a_fevpa, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_fsenl', landpatch, 'patch', a_fsenl, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_fsenl', landpatch, a_fsenl, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_fevpl', landpatch, 'patch', a_fevpl, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_fevpl', landpatch, a_fevpl, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_etr', landpatch, 'patch', a_etr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_etr', landpatch, a_etr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_fseng', landpatch, 'patch', a_fseng, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_fseng', landpatch, a_fseng, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_fevpg', landpatch, 'patch', a_fevpg, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_fevpg', landpatch, a_fevpg, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_fgrnd', landpatch, 'patch', a_fgrnd, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_fgrnd', landpatch, a_fgrnd, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_sabvsun', landpatch, 'patch', a_sabvsun, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_sabvsun', landpatch, a_sabvsun, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_sabvsha', landpatch, 'patch', a_sabvsha, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_sabvsha', landpatch, a_sabvsha, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_sabg', landpatch, 'patch', a_sabg, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_sabg', landpatch, a_sabg, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_olrg', landpatch, 'patch', a_olrg, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_olrg', landpatch, a_olrg, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_rnet', landpatch, 'patch', a_rnet, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_rnet', landpatch, a_rnet, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_xerr', landpatch, 'patch', a_xerr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_xerr', landpatch, a_xerr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_zerr', landpatch, 'patch', a_zerr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_zerr', landpatch, a_zerr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_rsur', landpatch, 'patch', a_rsur, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_rsur', landpatch, a_rsur, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_rsur_se', landpatch, 'patch', a_rsur_se, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_rsur_se', landpatch, a_rsur_se, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_rsur_ie', landpatch, 'patch', a_rsur_ie, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_rsur_ie', landpatch, a_rsur_ie, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_rsub', landpatch, 'patch', a_rsub, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_rsub', landpatch, a_rsub, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_rnof', landpatch, 'patch', a_rnof, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_rnof', landpatch, a_rnof, known_present=.true.)
         ENDIF
      ENDIF
#if (defined CatchLateralFlow)
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_xwsur', landpatch, 'patch', a_xwsur, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_xwsur', landpatch, a_xwsur, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_xwsub', landpatch, 'patch', a_xwsub, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_xwsub', landpatch, a_xwsub, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_fldarea', landpatch, 'patch', a_fldarea, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_fldarea', landpatch, a_fldarea, known_present=.true.)
         ENDIF
      ENDIF
#endif
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_qintr', landpatch, 'patch', a_qintr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_qintr', landpatch, a_qintr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_qinfl', landpatch, 'patch', a_qinfl, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_qinfl', landpatch, a_qinfl, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_qdrip', landpatch, 'patch', a_qdrip, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_qdrip', landpatch, a_qdrip, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_rstfacsun', landpatch, 'patch', a_rstfacsun, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_rstfacsun', landpatch, a_rstfacsun, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_rstfacsha', landpatch, 'patch', a_rstfacsha, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_rstfacsha', landpatch, a_rstfacsha, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_gssun', landpatch, 'patch', a_gssun, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_gssun', landpatch, a_gssun, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_gssha', landpatch, 'patch', a_gssha, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_gssha', landpatch, a_gssha, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_rss', landpatch, 'patch', a_rss, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_rss', landpatch, a_rss, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_wdsrf', landpatch, 'patch', a_wdsrf, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_wdsrf', landpatch, a_wdsrf, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_zwt', landpatch, 'patch', a_zwt, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_zwt', landpatch, a_zwt, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_wa', landpatch, 'patch', a_wa, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_wa', landpatch, a_wa, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_wat', landpatch, 'patch', a_wat, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_wat', landpatch, a_wat, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_wetwat', landpatch, 'patch', a_wetwat, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_wetwat', landpatch, a_wetwat, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_assim', landpatch, 'patch', a_assim, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_assim', landpatch, a_assim, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_respc', landpatch, 'patch', a_respc, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_respc', landpatch, a_respc, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_assimsun', landpatch, 'patch', a_assimsun, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_assimsun', landpatch, a_assimsun, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_assimsha', landpatch, 'patch', a_assimsha, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_assimsha', landpatch, a_assimsha, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_etrsun', landpatch, 'patch', a_etrsun, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_etrsun', landpatch, a_etrsun, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_etrsha', landpatch, 'patch', a_etrsha, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_etrsha', landpatch, a_etrsha, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_qcharge', landpatch, 'patch', a_qcharge, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_qcharge', landpatch, a_qcharge, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_t_grnd', landpatch, 'patch', a_t_grnd, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_t_grnd', landpatch, a_t_grnd, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_tleaf', landpatch, 'patch', a_tleaf, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_tleaf', landpatch, a_tleaf, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_ldew', landpatch, 'patch', a_ldew, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_ldew', landpatch, a_ldew, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_ldew_rain', landpatch, 'patch', a_ldew_rain, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_ldew_rain', landpatch, a_ldew_rain, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_ldew_snow', landpatch, 'patch', a_ldew_snow, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_ldew_snow', landpatch, a_ldew_snow, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_scv', landpatch, 'patch', a_scv, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_scv', landpatch, a_scv, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_snowdp', landpatch, 'patch', a_snowdp, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_snowdp', landpatch, a_snowdp, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_fsno', landpatch, 'patch', a_fsno, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_fsno', landpatch, a_fsno, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_frcsat', landpatch, 'patch', a_frcsat, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_frcsat', landpatch, a_frcsat, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_sigf', landpatch, 'patch', a_sigf, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_sigf', landpatch, a_sigf, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_green', landpatch, 'patch', a_green, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_green', landpatch, a_green, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_lai', landpatch, 'patch', a_lai, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_lai', landpatch, a_lai, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_laisun', landpatch, 'patch', a_laisun, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_laisun', landpatch, a_laisun, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_laisha', landpatch, 'patch', a_laisha, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_laisha', landpatch, a_laisha, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_sai', landpatch, 'patch', a_sai, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_sai', landpatch, a_sai, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_alb', dims(1,i))
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd2_a_alb', dims(2,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_alb', landpatch, 'patch', a_alb, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_alb', dims(1,i), dims(2,i), landpatch, a_alb)
         ENDIF
      ENDIF
#if (defined HYPERSPECTRAL)
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_alb_hires', dims(1,i))
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd2_a_alb_hires', dims(2,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_alb_hires', landpatch, 'patch', a_alb_hires, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_alb_hires', dims(1,i), dims(2,i), landpatch, a_alb_hires)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_reflectance_out', dims(1,i))
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd2_a_reflectance_out', dims(2,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_reflectance_out', landpatch, 'patch', a_reflectance_out, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_reflectance_out', dims(1,i), dims(2,i), landpatch, a_reflectance_out)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_transmittance_out', dims(1,i))
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd2_a_transmittance_out', dims(2,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_transmittance_out', landpatch, 'patch', a_transmittance_out, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_transmittance_out', dims(1,i), dims(2,i), landpatch, a_transmittance_out)
         ENDIF
      ENDIF
#endif
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_emis', landpatch, 'patch', a_emis, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_emis', landpatch, a_emis, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_z0m', landpatch, 'patch', a_z0m, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_z0m', landpatch, a_z0m, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_trad', landpatch, 'patch', a_trad, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_trad', landpatch, a_trad, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_tref', landpatch, 'patch', a_tref, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_tref', landpatch, a_tref, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_t2m_wmo', landpatch, 'patch', a_t2m_wmo, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_t2m_wmo', landpatch, a_t2m_wmo, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_qref', landpatch, 'patch', a_qref, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_qref', landpatch, a_qref, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_rain', landpatch, 'patch', a_rain, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_rain', landpatch, a_rain, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_snow', landpatch, 'patch', a_snow, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_snow', landpatch, a_snow, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_o3uptakesun', landpatch, 'patch', a_o3uptakesun, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_o3uptakesun', landpatch, a_o3uptakesun, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_o3uptakesha', landpatch, 'patch', a_o3uptakesha, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_o3uptakesha', landpatch, a_o3uptakesha, known_present=.true.)
         ENDIF
      ENDIF
#if (defined DataAssimilation)
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_h2osoi_ens', dims(1,i))
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd2_a_h2osoi_ens', dims(2,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_h2osoi_ens', landpatch, 'patch', a_h2osoi_ens, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_h2osoi_ens', dims(1,i), dims(2,i), landpatch, a_h2osoi_ens)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_t_brt_smap_ens', dims(1,i))
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd2_a_t_brt_smap_ens', dims(2,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_t_brt_smap_ens', landpatch, 'patch', a_t_brt_smap_ens, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_t_brt_smap_ens', dims(1,i), dims(2,i), landpatch, a_t_brt_smap_ens)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_t_brt_fy3d_ens', dims(1,i))
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd2_a_t_brt_fy3d_ens', dims(2,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_t_brt_fy3d_ens', landpatch, 'patch', a_t_brt_fy3d_ens, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_t_brt_fy3d_ens', dims(1,i), dims(2,i), landpatch, a_t_brt_fy3d_ens)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_t_brt_smap', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_t_brt_smap', landpatch, 'patch', a_t_brt_smap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_t_brt_smap', dims(1,i), landpatch, a_t_brt_smap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_t_brt_fy3d', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_t_brt_fy3d', landpatch, 'patch', a_t_brt_fy3d, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_t_brt_fy3d', dims(1,i), landpatch, a_t_brt_fy3d, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_wliq_soisno_ens', dims(1,i))
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd2_a_wliq_soisno_ens', dims(2,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_wliq_soisno_ens', landpatch, 'patch', a_wliq_soisno_ens, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_wliq_soisno_ens', dims(1,i), dims(2,i), landpatch, a_wliq_soisno_ens)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_wice_soisno_ens', dims(1,i))
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd2_a_wice_soisno_ens', dims(2,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_wice_soisno_ens', landpatch, 'patch', a_wice_soisno_ens, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_wice_soisno_ens', dims(1,i), dims(2,i), landpatch, a_wice_soisno_ens)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_t_soisno_ens', dims(1,i))
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd2_a_t_soisno_ens', dims(2,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_t_soisno_ens', landpatch, 'patch', a_t_soisno_ens, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_t_soisno_ens', dims(1,i), dims(2,i), landpatch, a_t_soisno_ens)
         ENDIF
      ENDIF
#endif
      IF ((DEF_URBAN_RUN)) THEN
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_t_room', landurban, 'urban', a_t_room, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_t_room', landurban, a_t_room, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_tafu', landurban, 'urban', a_tafu, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_tafu', landurban, a_tafu, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_fhac', landurban, 'urban', a_fhac, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_fhac', landurban, a_fhac, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_fwst', landurban, 'urban', a_fwst, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_fwst', landurban, a_fwst, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_fach', landurban, 'urban', a_fach, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_fach', landurban, a_fach, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_fahe', landurban, 'urban', a_fahe, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_fahe', landurban, a_fahe, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_fhah', landurban, 'urban', a_fhah, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_fhah', landurban, a_fhah, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_vehc', landurban, 'urban', a_vehc, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_vehc', landurban, a_vehc, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_meta', landurban, 'urban', a_meta, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_meta', landurban, a_meta, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_senroof', landurban, 'urban', a_senroof, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_senroof', landurban, a_senroof, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_senwsun', landurban, 'urban', a_senwsun, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_senwsun', landurban, a_senwsun, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_senwsha', landurban, 'urban', a_senwsha, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_senwsha', landurban, a_senwsha, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_sengimp', landurban, 'urban', a_sengimp, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_sengimp', landurban, a_sengimp, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_sengper', landurban, 'urban', a_sengper, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_sengper', landurban, a_sengper, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_senurbl', landurban, 'urban', a_senurbl, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_senurbl', landurban, a_senurbl, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_lfevproof', landurban, 'urban', a_lfevproof, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_lfevproof', landurban, a_lfevproof, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_lfevpgimp', landurban, 'urban', a_lfevpgimp, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_lfevpgimp', landurban, a_lfevpgimp, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_lfevpgper', landurban, 'urban', a_lfevpgper, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_lfevpgper', landurban, a_lfevpgper, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_lfevpurbl', landurban, 'urban', a_lfevpurbl, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_lfevpurbl', landurban, a_lfevpurbl, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_troof', landurban, 'urban', a_troof, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_troof', landurban, a_troof, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_twall', landurban, 'urban', a_twall, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_twall', landurban, a_twall, known_present=.true.)
         ENDIF
      ENDIF
      ENDIF
      IF (((DEF_USE_PFT .or. DEF_USE_PC))) THEN
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_lai_enftemp', landpatch, 'patch', a_lai_enftemp, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_lai_enftemp', landpatch, a_lai_enftemp, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_lai_enfboreal', landpatch, 'patch', a_lai_enfboreal, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_lai_enfboreal', landpatch, a_lai_enfboreal, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_lai_dnfboreal', landpatch, 'patch', a_lai_dnfboreal, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_lai_dnfboreal', landpatch, a_lai_dnfboreal, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_lai_ebftrop', landpatch, 'patch', a_lai_ebftrop, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_lai_ebftrop', landpatch, a_lai_ebftrop, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_lai_ebftemp', landpatch, 'patch', a_lai_ebftemp, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_lai_ebftemp', landpatch, a_lai_ebftemp, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_lai_dbftrop', landpatch, 'patch', a_lai_dbftrop, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_lai_dbftrop', landpatch, a_lai_dbftrop, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_lai_dbftemp', landpatch, 'patch', a_lai_dbftemp, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_lai_dbftemp', landpatch, a_lai_dbftemp, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_lai_dbfboreal', landpatch, 'patch', a_lai_dbfboreal, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_lai_dbfboreal', landpatch, a_lai_dbfboreal, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_lai_ebstemp', landpatch, 'patch', a_lai_ebstemp, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_lai_ebstemp', landpatch, a_lai_ebstemp, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_lai_dbstemp', landpatch, 'patch', a_lai_dbstemp, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_lai_dbstemp', landpatch, a_lai_dbstemp, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_lai_dbsboreal', landpatch, 'patch', a_lai_dbsboreal, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_lai_dbsboreal', landpatch, a_lai_dbsboreal, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_lai_c3arcgrass', landpatch, 'patch', a_lai_c3arcgrass, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_lai_c3arcgrass', landpatch, a_lai_c3arcgrass, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_lai_c3grass', landpatch, 'patch', a_lai_c3grass, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_lai_c3grass', landpatch, a_lai_c3grass, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_lai_c4grass', landpatch, 'patch', a_lai_c4grass, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_lai_c4grass', landpatch, a_lai_c4grass, known_present=.true.)
         ENDIF
      ENDIF
      ENDIF
      IF ((DEF_USE_BGC)) THEN
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_leafc', landpatch, 'patch', a_leafc, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_leafc', landpatch, a_leafc, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_leafc_storage', landpatch, 'patch', a_leafc_storage, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_leafc_storage', landpatch, a_leafc_storage, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_leafc_xfer', landpatch, 'patch', a_leafc_xfer, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_leafc_xfer', landpatch, a_leafc_xfer, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_frootc', landpatch, 'patch', a_frootc, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_frootc', landpatch, a_frootc, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_frootc_storage', landpatch, 'patch', a_frootc_storage, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_frootc_storage', landpatch, a_frootc_storage, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_frootc_xfer', landpatch, 'patch', a_frootc_xfer, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_frootc_xfer', landpatch, a_frootc_xfer, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_livestemc', landpatch, 'patch', a_livestemc, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_livestemc', landpatch, a_livestemc, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_livestemc_storage', landpatch, 'patch', a_livestemc_storage, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_livestemc_storage', landpatch, a_livestemc_storage, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_livestemc_xfer', landpatch, 'patch', a_livestemc_xfer, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_livestemc_xfer', landpatch, a_livestemc_xfer, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_deadstemc', landpatch, 'patch', a_deadstemc, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_deadstemc', landpatch, a_deadstemc, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_deadstemc_storage', landpatch, 'patch', a_deadstemc_storage, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_deadstemc_storage', landpatch, a_deadstemc_storage, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_deadstemc_xfer', landpatch, 'patch', a_deadstemc_xfer, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_deadstemc_xfer', landpatch, a_deadstemc_xfer, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_livecrootc', landpatch, 'patch', a_livecrootc, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_livecrootc', landpatch, a_livecrootc, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_livecrootc_storage', landpatch, 'patch', a_livecrootc_storage, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_livecrootc_storage', landpatch, a_livecrootc_storage, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_livecrootc_xfer', landpatch, 'patch', a_livecrootc_xfer, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_livecrootc_xfer', landpatch, a_livecrootc_xfer, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_deadcrootc', landpatch, 'patch', a_deadcrootc, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_deadcrootc', landpatch, a_deadcrootc, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_deadcrootc_storage', landpatch, 'patch', a_deadcrootc_storage, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_deadcrootc_storage', landpatch, a_deadcrootc_storage, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_deadcrootc_xfer', landpatch, 'patch', a_deadcrootc_xfer, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_deadcrootc_xfer', landpatch, a_deadcrootc_xfer, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_grainc', landpatch, 'patch', a_grainc, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_grainc', landpatch, a_grainc, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_grainc_storage', landpatch, 'patch', a_grainc_storage, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_grainc_storage', landpatch, a_grainc_storage, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_grainc_xfer', landpatch, 'patch', a_grainc_xfer, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_grainc_xfer', landpatch, a_grainc_xfer, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_leafn', landpatch, 'patch', a_leafn, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_leafn', landpatch, a_leafn, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_leafn_storage', landpatch, 'patch', a_leafn_storage, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_leafn_storage', landpatch, a_leafn_storage, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_leafn_xfer', landpatch, 'patch', a_leafn_xfer, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_leafn_xfer', landpatch, a_leafn_xfer, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_frootn', landpatch, 'patch', a_frootn, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_frootn', landpatch, a_frootn, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_frootn_storage', landpatch, 'patch', a_frootn_storage, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_frootn_storage', landpatch, a_frootn_storage, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_frootn_xfer', landpatch, 'patch', a_frootn_xfer, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_frootn_xfer', landpatch, a_frootn_xfer, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_livestemn', landpatch, 'patch', a_livestemn, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_livestemn', landpatch, a_livestemn, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_livestemn_storage', landpatch, 'patch', a_livestemn_storage, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_livestemn_storage', landpatch, a_livestemn_storage, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_livestemn_xfer', landpatch, 'patch', a_livestemn_xfer, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_livestemn_xfer', landpatch, a_livestemn_xfer, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_deadstemn', landpatch, 'patch', a_deadstemn, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_deadstemn', landpatch, a_deadstemn, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_deadstemn_storage', landpatch, 'patch', a_deadstemn_storage, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_deadstemn_storage', landpatch, a_deadstemn_storage, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_deadstemn_xfer', landpatch, 'patch', a_deadstemn_xfer, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_deadstemn_xfer', landpatch, a_deadstemn_xfer, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_livecrootn', landpatch, 'patch', a_livecrootn, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_livecrootn', landpatch, a_livecrootn, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_livecrootn_storage', landpatch, 'patch', a_livecrootn_storage, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_livecrootn_storage', landpatch, a_livecrootn_storage, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_livecrootn_xfer', landpatch, 'patch', a_livecrootn_xfer, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_livecrootn_xfer', landpatch, a_livecrootn_xfer, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_deadcrootn', landpatch, 'patch', a_deadcrootn, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_deadcrootn', landpatch, a_deadcrootn, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_deadcrootn_storage', landpatch, 'patch', a_deadcrootn_storage, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_deadcrootn_storage', landpatch, a_deadcrootn_storage, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_deadcrootn_xfer', landpatch, 'patch', a_deadcrootn_xfer, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_deadcrootn_xfer', landpatch, a_deadcrootn_xfer, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_grainn', landpatch, 'patch', a_grainn, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_grainn', landpatch, a_grainn, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_grainn_storage', landpatch, 'patch', a_grainn_storage, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_grainn_storage', landpatch, a_grainn_storage, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_grainn_xfer', landpatch, 'patch', a_grainn_xfer, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_grainn_xfer', landpatch, a_grainn_xfer, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_retransn', landpatch, 'patch', a_retransn, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_retransn', landpatch, a_retransn, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_gpp', landpatch, 'patch', a_gpp, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_gpp', landpatch, a_gpp, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_downreg', landpatch, 'patch', a_downreg, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_downreg', landpatch, a_downreg, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_ar', landpatch, 'patch', a_ar, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_ar', landpatch, a_ar, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_cwdprod', landpatch, 'patch', a_cwdprod, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_cwdprod', landpatch, a_cwdprod, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_cwddecomp', landpatch, 'patch', a_cwddecomp, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_cwddecomp', landpatch, a_cwddecomp, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_hr', landpatch, 'patch', a_hr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_hr', landpatch, a_hr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_fpg', landpatch, 'patch', a_fpg, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_fpg', landpatch, a_fpg, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_fpi', landpatch, 'patch', a_fpi, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_fpi', landpatch, a_fpi, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_totvegc', landpatch, 'patch', a_totvegc, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_totvegc', landpatch, a_totvegc, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_totlitc', landpatch, 'patch', a_totlitc, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_totlitc', landpatch, a_totlitc, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_totcwdc', landpatch, 'patch', a_totcwdc, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_totcwdc', landpatch, a_totcwdc, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_totsomc', landpatch, 'patch', a_totsomc, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_totsomc', landpatch, a_totsomc, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_totcolc', landpatch, 'patch', a_totcolc, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_totcolc', landpatch, a_totcolc, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_totvegn', landpatch, 'patch', a_totvegn, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_totvegn', landpatch, a_totvegn, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_totlitn', landpatch, 'patch', a_totlitn, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_totlitn', landpatch, a_totlitn, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_totcwdn', landpatch, 'patch', a_totcwdn, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_totcwdn', landpatch, a_totcwdn, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_totsomn', landpatch, 'patch', a_totsomn, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_totsomn', landpatch, a_totsomn, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_totcoln', landpatch, 'patch', a_totcoln, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_totcoln', landpatch, a_totcoln, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_gpp_enftemp', landpatch, 'patch', a_gpp_enftemp, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_gpp_enftemp', landpatch, a_gpp_enftemp, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_gpp_enfboreal', landpatch, 'patch', a_gpp_enfboreal, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_gpp_enfboreal', landpatch, a_gpp_enfboreal, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_gpp_dnfboreal', landpatch, 'patch', a_gpp_dnfboreal, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_gpp_dnfboreal', landpatch, a_gpp_dnfboreal, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_gpp_ebftrop', landpatch, 'patch', a_gpp_ebftrop, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_gpp_ebftrop', landpatch, a_gpp_ebftrop, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_gpp_ebftemp', landpatch, 'patch', a_gpp_ebftemp, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_gpp_ebftemp', landpatch, a_gpp_ebftemp, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_gpp_dbftrop', landpatch, 'patch', a_gpp_dbftrop, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_gpp_dbftrop', landpatch, a_gpp_dbftrop, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_gpp_dbftemp', landpatch, 'patch', a_gpp_dbftemp, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_gpp_dbftemp', landpatch, a_gpp_dbftemp, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_gpp_dbfboreal', landpatch, 'patch', a_gpp_dbfboreal, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_gpp_dbfboreal', landpatch, a_gpp_dbfboreal, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_gpp_ebstemp', landpatch, 'patch', a_gpp_ebstemp, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_gpp_ebstemp', landpatch, a_gpp_ebstemp, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_gpp_dbstemp', landpatch, 'patch', a_gpp_dbstemp, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_gpp_dbstemp', landpatch, a_gpp_dbstemp, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_gpp_dbsboreal', landpatch, 'patch', a_gpp_dbsboreal, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_gpp_dbsboreal', landpatch, a_gpp_dbsboreal, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_gpp_c3arcgrass', landpatch, 'patch', a_gpp_c3arcgrass, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_gpp_c3arcgrass', landpatch, a_gpp_c3arcgrass, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_gpp_c3grass', landpatch, 'patch', a_gpp_c3grass, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_gpp_c3grass', landpatch, a_gpp_c3grass, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_gpp_c4grass', landpatch, 'patch', a_gpp_c4grass, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_gpp_c4grass', landpatch, a_gpp_c4grass, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_npp_enftemp', landpatch, 'patch', a_npp_enftemp, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_npp_enftemp', landpatch, a_npp_enftemp, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_npp_enfboreal', landpatch, 'patch', a_npp_enfboreal, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_npp_enfboreal', landpatch, a_npp_enfboreal, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_npp_dnfboreal', landpatch, 'patch', a_npp_dnfboreal, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_npp_dnfboreal', landpatch, a_npp_dnfboreal, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_npp_ebftrop', landpatch, 'patch', a_npp_ebftrop, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_npp_ebftrop', landpatch, a_npp_ebftrop, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_npp_ebftemp', landpatch, 'patch', a_npp_ebftemp, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_npp_ebftemp', landpatch, a_npp_ebftemp, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_npp_dbftrop', landpatch, 'patch', a_npp_dbftrop, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_npp_dbftrop', landpatch, a_npp_dbftrop, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_npp_dbftemp', landpatch, 'patch', a_npp_dbftemp, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_npp_dbftemp', landpatch, a_npp_dbftemp, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_npp_dbfboreal', landpatch, 'patch', a_npp_dbfboreal, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_npp_dbfboreal', landpatch, a_npp_dbfboreal, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_npp_ebstemp', landpatch, 'patch', a_npp_ebstemp, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_npp_ebstemp', landpatch, a_npp_ebstemp, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_npp_dbstemp', landpatch, 'patch', a_npp_dbstemp, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_npp_dbstemp', landpatch, a_npp_dbstemp, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_npp_dbsboreal', landpatch, 'patch', a_npp_dbsboreal, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_npp_dbsboreal', landpatch, a_npp_dbsboreal, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_npp_c3arcgrass', landpatch, 'patch', a_npp_c3arcgrass, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_npp_c3arcgrass', landpatch, a_npp_c3arcgrass, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_npp_c3grass', landpatch, 'patch', a_npp_c3grass, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_npp_c3grass', landpatch, a_npp_c3grass, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_npp_c4grass', landpatch, 'patch', a_npp_c4grass, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_npp_c4grass', landpatch, a_npp_c4grass, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_npptoleafc_enftemp', landpatch, 'patch', a_npptoleafc_enftemp, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_npptoleafc_enftemp', landpatch, a_npptoleafc_enftemp, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_npptoleafc_enfboreal', landpatch, 'patch', a_npptoleafc_enfboreal, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_npptoleafc_enfboreal', landpatch, a_npptoleafc_enfboreal, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_npptoleafc_dnfboreal', landpatch, 'patch', a_npptoleafc_dnfboreal, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_npptoleafc_dnfboreal', landpatch, a_npptoleafc_dnfboreal, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_npptoleafc_ebftrop', landpatch, 'patch', a_npptoleafc_ebftrop, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_npptoleafc_ebftrop', landpatch, a_npptoleafc_ebftrop, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_npptoleafc_ebftemp', landpatch, 'patch', a_npptoleafc_ebftemp, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_npptoleafc_ebftemp', landpatch, a_npptoleafc_ebftemp, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_npptoleafc_dbftrop', landpatch, 'patch', a_npptoleafc_dbftrop, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_npptoleafc_dbftrop', landpatch, a_npptoleafc_dbftrop, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_npptoleafc_dbftemp', landpatch, 'patch', a_npptoleafc_dbftemp, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_npptoleafc_dbftemp', landpatch, a_npptoleafc_dbftemp, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_npptoleafc_dbfboreal', landpatch, 'patch', a_npptoleafc_dbfboreal, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_npptoleafc_dbfboreal', landpatch, a_npptoleafc_dbfboreal, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_npptoleafc_ebstemp', landpatch, 'patch', a_npptoleafc_ebstemp, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_npptoleafc_ebstemp', landpatch, a_npptoleafc_ebstemp, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_npptoleafc_dbstemp', landpatch, 'patch', a_npptoleafc_dbstemp, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_npptoleafc_dbstemp', landpatch, a_npptoleafc_dbstemp, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_npptoleafc_dbsboreal', landpatch, 'patch', a_npptoleafc_dbsboreal, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_npptoleafc_dbsboreal', landpatch, a_npptoleafc_dbsboreal, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_npptoleafc_c3arcgrass', landpatch, 'patch', a_npptoleafc_c3arcgrass, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_npptoleafc_c3arcgrass', landpatch, a_npptoleafc_c3arcgrass, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_npptoleafc_c3grass', landpatch, 'patch', a_npptoleafc_c3grass, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_npptoleafc_c3grass', landpatch, a_npptoleafc_c3grass, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_npptoleafc_c4grass', landpatch, 'patch', a_npptoleafc_c4grass, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_npptoleafc_c4grass', landpatch, a_npptoleafc_c4grass, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_leafc_enftemp', landpatch, 'patch', a_leafc_enftemp, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_leafc_enftemp', landpatch, a_leafc_enftemp, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_leafc_enfboreal', landpatch, 'patch', a_leafc_enfboreal, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_leafc_enfboreal', landpatch, a_leafc_enfboreal, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_leafc_dnfboreal', landpatch, 'patch', a_leafc_dnfboreal, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_leafc_dnfboreal', landpatch, a_leafc_dnfboreal, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_leafc_ebftrop', landpatch, 'patch', a_leafc_ebftrop, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_leafc_ebftrop', landpatch, a_leafc_ebftrop, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_leafc_ebftemp', landpatch, 'patch', a_leafc_ebftemp, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_leafc_ebftemp', landpatch, a_leafc_ebftemp, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_leafc_dbftrop', landpatch, 'patch', a_leafc_dbftrop, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_leafc_dbftrop', landpatch, a_leafc_dbftrop, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_leafc_dbftemp', landpatch, 'patch', a_leafc_dbftemp, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_leafc_dbftemp', landpatch, a_leafc_dbftemp, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_leafc_dbfboreal', landpatch, 'patch', a_leafc_dbfboreal, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_leafc_dbfboreal', landpatch, a_leafc_dbfboreal, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_leafc_ebstemp', landpatch, 'patch', a_leafc_ebstemp, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_leafc_ebstemp', landpatch, a_leafc_ebstemp, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_leafc_dbstemp', landpatch, 'patch', a_leafc_dbstemp, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_leafc_dbstemp', landpatch, a_leafc_dbstemp, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_leafc_dbsboreal', landpatch, 'patch', a_leafc_dbsboreal, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_leafc_dbsboreal', landpatch, a_leafc_dbsboreal, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_leafc_c3arcgrass', landpatch, 'patch', a_leafc_c3arcgrass, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_leafc_c3arcgrass', landpatch, a_leafc_c3arcgrass, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_leafc_c3grass', landpatch, 'patch', a_leafc_c3grass, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_leafc_c3grass', landpatch, a_leafc_c3grass, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_leafc_c4grass', landpatch, 'patch', a_leafc_c4grass, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_leafc_c4grass', landpatch, a_leafc_c4grass, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_O2_DECOMP_DEPTH_UNSAT', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_O2_DECOMP_DEPTH_UNSAT', landpatch, 'patch', a_O2_DECOMP_DEPTH_UNSAT, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_O2_DECOMP_DEPTH_UNSAT', dims(1,i), landpatch, a_O2_DECOMP_DEPTH_UNSAT, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_CONC_O2_UNSAT', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_CONC_O2_UNSAT', landpatch, 'patch', a_CONC_O2_UNSAT, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_CONC_O2_UNSAT', dims(1,i), landpatch, a_CONC_O2_UNSAT, known_present=.true.)
         ENDIF
      ENDIF
      ENDIF
#ifdef CROP
      IF (DEF_USE_BGC) THEN
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_pdcorn', landpatch, 'patch', a_pdcorn, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_pdcorn', landpatch, a_pdcorn, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_pdswheat', landpatch, 'patch', a_pdswheat, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_pdswheat', landpatch, a_pdswheat, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_pdwwheat', landpatch, 'patch', a_pdwwheat, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_pdwwheat', landpatch, a_pdwwheat, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_pdsoybean', landpatch, 'patch', a_pdsoybean, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_pdsoybean', landpatch, a_pdsoybean, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_pdcotton', landpatch, 'patch', a_pdcotton, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_pdcotton', landpatch, a_pdcotton, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_pdrice1', landpatch, 'patch', a_pdrice1, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_pdrice1', landpatch, a_pdrice1, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_pdrice2', landpatch, 'patch', a_pdrice2, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_pdrice2', landpatch, a_pdrice2, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_pdsugarcane', landpatch, 'patch', a_pdsugarcane, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_pdsugarcane', landpatch, a_pdsugarcane, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_plantdate', landpatch, 'patch', a_plantdate, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_plantdate', landpatch, a_plantdate, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_manunitro', landpatch, 'patch', a_manunitro, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_manunitro', landpatch, a_manunitro, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_fertnitro_corn', landpatch, 'patch', a_fertnitro_corn, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_fertnitro_corn', landpatch, a_fertnitro_corn, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_fertnitro_swheat', landpatch, 'patch', a_fertnitro_swheat, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_fertnitro_swheat', landpatch, a_fertnitro_swheat, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_fertnitro_wwheat', landpatch, 'patch', a_fertnitro_wwheat, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_fertnitro_wwheat', landpatch, a_fertnitro_wwheat, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_fertnitro_soybean', landpatch, 'patch', a_fertnitro_soybean, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_fertnitro_soybean', landpatch, a_fertnitro_soybean, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_fertnitro_cotton', landpatch, 'patch', a_fertnitro_cotton, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_fertnitro_cotton', landpatch, a_fertnitro_cotton, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_fertnitro_rice1', landpatch, 'patch', a_fertnitro_rice1, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_fertnitro_rice1', landpatch, a_fertnitro_rice1, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_fertnitro_rice2', landpatch, 'patch', a_fertnitro_rice2, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_fertnitro_rice2', landpatch, a_fertnitro_rice2, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_fertnitro_sugarcane', landpatch, 'patch', a_fertnitro_sugarcane, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_fertnitro_sugarcane', landpatch, a_fertnitro_sugarcane, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_irrig_method_corn', landpatch, 'patch', a_irrig_method_corn, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_irrig_method_corn', landpatch, a_irrig_method_corn, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_irrig_method_swheat', landpatch, 'patch', a_irrig_method_swheat, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_irrig_method_swheat', landpatch, a_irrig_method_swheat, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_irrig_method_wwheat', landpatch, 'patch', a_irrig_method_wwheat, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_irrig_method_wwheat', landpatch, a_irrig_method_wwheat, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_irrig_method_soybean', landpatch, 'patch', a_irrig_method_soybean, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_irrig_method_soybean', landpatch, a_irrig_method_soybean, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_irrig_method_cotton', landpatch, 'patch', a_irrig_method_cotton, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_irrig_method_cotton', landpatch, a_irrig_method_cotton, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_irrig_method_rice1', landpatch, 'patch', a_irrig_method_rice1, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_irrig_method_rice1', landpatch, a_irrig_method_rice1, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_irrig_method_rice2', landpatch, 'patch', a_irrig_method_rice2, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_irrig_method_rice2', landpatch, a_irrig_method_rice2, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_irrig_method_sugarcane', landpatch, 'patch', a_irrig_method_sugarcane, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_irrig_method_sugarcane', landpatch, a_irrig_method_sugarcane, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_cphase', landpatch, 'patch', a_cphase, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_cphase', landpatch, a_cphase, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_gddplant', landpatch, 'patch', a_gddplant, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_gddplant', landpatch, a_gddplant, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_gddmaturity', landpatch, 'patch', a_gddmaturity, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_gddmaturity', landpatch, a_gddmaturity, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_vf', landpatch, 'patch', a_vf, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_vf', landpatch, a_vf, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_hui', landpatch, 'patch', a_hui, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_hui', landpatch, a_hui, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_cropprod1c', landpatch, 'patch', a_cropprod1c, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_cropprod1c', landpatch, a_cropprod1c, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_cropprod1c_loss', landpatch, 'patch', a_cropprod1c_loss, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_cropprod1c_loss', landpatch, a_cropprod1c_loss, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_cropseedc_deficit', landpatch, 'patch', a_cropseedc_deficit, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_cropseedc_deficit', landpatch, a_cropseedc_deficit, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_grainc_to_cropprodc', landpatch, 'patch', a_grainc_to_cropprodc, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_grainc_to_cropprodc', landpatch, a_grainc_to_cropprodc, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_grainc_to_seed', landpatch, 'patch', a_grainc_to_seed, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_grainc_to_seed', landpatch, a_grainc_to_seed, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_fert_to_sminn', landpatch, 'patch', a_fert_to_sminn, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_fert_to_sminn', landpatch, a_fert_to_sminn, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_sum_irrig', landpatch, 'patch', a_sum_irrig, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_sum_irrig', landpatch, a_sum_irrig, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_sum_deficit_irrig', landpatch, 'patch', a_sum_deficit_irrig, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_sum_deficit_irrig', landpatch, a_sum_deficit_irrig, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_sum_irrig_count', landpatch, 'patch', a_sum_irrig_count, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_sum_irrig_count', landpatch, a_sum_irrig_count, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_waterstorage', landpatch, 'patch', a_waterstorage, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_waterstorage', landpatch, a_waterstorage, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_groundwater_demand', landpatch, 'patch', a_groundwater_demand, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_groundwater_demand', landpatch, a_groundwater_demand, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_groundwater_supply', landpatch, 'patch', a_groundwater_supply, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_groundwater_supply', landpatch, a_groundwater_supply, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_reservoirriver_demand', landpatch, 'patch', a_reservoirriver_demand, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_reservoirriver_demand', landpatch, a_reservoirriver_demand, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_reservoirriver_supply', landpatch, 'patch', a_reservoirriver_supply, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_reservoirriver_supply', landpatch, a_reservoirriver_supply, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_reservoir_supply', landpatch, 'patch', a_reservoir_supply, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_reservoir_supply', landpatch, a_reservoir_supply, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_river_supply', landpatch, 'patch', a_river_supply, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_river_supply', landpatch, a_river_supply, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_runoff_supply', landpatch, 'patch', a_runoff_supply, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_runoff_supply', landpatch, a_runoff_supply, known_present=.true.)
         ENDIF
      ENDIF
      ENDIF
#endif
      IF ((DEF_USE_BGC)) THEN
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_ndep_to_sminn', landpatch, 'patch', a_ndep_to_sminn, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_ndep_to_sminn', landpatch, a_ndep_to_sminn, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_abm', landpatch, 'patch', a_abm, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_abm', landpatch, a_abm, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_gdp', landpatch, 'patch', a_gdp, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_gdp', landpatch, a_gdp, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_peatf', landpatch, 'patch', a_peatf, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_peatf', landpatch, a_peatf, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_hdm', landpatch, 'patch', a_hdm, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_hdm', landpatch, a_hdm, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_lnfm', landpatch, 'patch', a_lnfm, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_lnfm', landpatch, a_lnfm, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_leafcCap', landpatch, 'patch', a_leafcCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_leafcCap', landpatch, a_leafcCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_leafc_storageCap', landpatch, 'patch', a_leafc_storageCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_leafc_storageCap', landpatch, a_leafc_storageCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_leafc_xferCap', landpatch, 'patch', a_leafc_xferCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_leafc_xferCap', landpatch, a_leafc_xferCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_frootcCap', landpatch, 'patch', a_frootcCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_frootcCap', landpatch, a_frootcCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_frootc_storageCap', landpatch, 'patch', a_frootc_storageCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_frootc_storageCap', landpatch, a_frootc_storageCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_frootc_xferCap', landpatch, 'patch', a_frootc_xferCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_frootc_xferCap', landpatch, a_frootc_xferCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_livestemcCap', landpatch, 'patch', a_livestemcCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_livestemcCap', landpatch, a_livestemcCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_livestemc_storageCap', landpatch, 'patch', a_livestemc_storageCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_livestemc_storageCap', landpatch, a_livestemc_storageCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_livestemc_xferCap', landpatch, 'patch', a_livestemc_xferCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_livestemc_xferCap', landpatch, a_livestemc_xferCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_deadstemcCap', landpatch, 'patch', a_deadstemcCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_deadstemcCap', landpatch, a_deadstemcCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_deadstemc_storageCap', landpatch, 'patch', a_deadstemc_storageCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_deadstemc_storageCap', landpatch, a_deadstemc_storageCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_deadstemc_xferCap', landpatch, 'patch', a_deadstemc_xferCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_deadstemc_xferCap', landpatch, a_deadstemc_xferCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_livecrootcCap', landpatch, 'patch', a_livecrootcCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_livecrootcCap', landpatch, a_livecrootcCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_livecrootc_storageCap', landpatch, 'patch', a_livecrootc_storageCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_livecrootc_storageCap', landpatch, a_livecrootc_storageCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_livecrootc_xferCap', landpatch, 'patch', a_livecrootc_xferCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_livecrootc_xferCap', landpatch, a_livecrootc_xferCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_deadcrootcCap', landpatch, 'patch', a_deadcrootcCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_deadcrootcCap', landpatch, a_deadcrootcCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_deadcrootc_storageCap', landpatch, 'patch', a_deadcrootc_storageCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_deadcrootc_storageCap', landpatch, a_deadcrootc_storageCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_deadcrootc_xferCap', landpatch, 'patch', a_deadcrootc_xferCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_deadcrootc_xferCap', landpatch, a_deadcrootc_xferCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_leafnCap', landpatch, 'patch', a_leafnCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_leafnCap', landpatch, a_leafnCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_leafn_storageCap', landpatch, 'patch', a_leafn_storageCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_leafn_storageCap', landpatch, a_leafn_storageCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_leafn_xferCap', landpatch, 'patch', a_leafn_xferCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_leafn_xferCap', landpatch, a_leafn_xferCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_frootnCap', landpatch, 'patch', a_frootnCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_frootnCap', landpatch, a_frootnCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_frootn_storageCap', landpatch, 'patch', a_frootn_storageCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_frootn_storageCap', landpatch, a_frootn_storageCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_frootn_xferCap', landpatch, 'patch', a_frootn_xferCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_frootn_xferCap', landpatch, a_frootn_xferCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_livestemnCap', landpatch, 'patch', a_livestemnCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_livestemnCap', landpatch, a_livestemnCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_livestemn_storageCap', landpatch, 'patch', a_livestemn_storageCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_livestemn_storageCap', landpatch, a_livestemn_storageCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_livestemn_xferCap', landpatch, 'patch', a_livestemn_xferCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_livestemn_xferCap', landpatch, a_livestemn_xferCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_deadstemnCap', landpatch, 'patch', a_deadstemnCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_deadstemnCap', landpatch, a_deadstemnCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_deadstemn_storageCap', landpatch, 'patch', a_deadstemn_storageCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_deadstemn_storageCap', landpatch, a_deadstemn_storageCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_deadstemn_xferCap', landpatch, 'patch', a_deadstemn_xferCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_deadstemn_xferCap', landpatch, a_deadstemn_xferCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_livecrootnCap', landpatch, 'patch', a_livecrootnCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_livecrootnCap', landpatch, a_livecrootnCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_livecrootn_storageCap', landpatch, 'patch', a_livecrootn_storageCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_livecrootn_storageCap', landpatch, a_livecrootn_storageCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_livecrootn_xferCap', landpatch, 'patch', a_livecrootn_xferCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_livecrootn_xferCap', landpatch, a_livecrootn_xferCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_deadcrootnCap', landpatch, 'patch', a_deadcrootnCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_deadcrootnCap', landpatch, a_deadcrootnCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_deadcrootn_storageCap', landpatch, 'patch', a_deadcrootn_storageCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_deadcrootn_storageCap', landpatch, a_deadcrootn_storageCap, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_deadcrootn_xferCap', landpatch, 'patch', a_deadcrootn_xferCap, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_deadcrootn_xferCap', landpatch, a_deadcrootn_xferCap, known_present=.true.)
         ENDIF
      ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_ozone', landpatch, 'patch', a_ozone, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_ozone', landpatch, a_ozone, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_t_soisno', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_t_soisno', landpatch, 'patch', a_t_soisno, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_t_soisno', dims(1,i), landpatch, a_t_soisno, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_wliq_soisno', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_wliq_soisno', landpatch, 'patch', a_wliq_soisno, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_wliq_soisno', dims(1,i), landpatch, a_wliq_soisno, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_wice_soisno', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_wice_soisno', landpatch, 'patch', a_wice_soisno, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_wice_soisno', dims(1,i), landpatch, a_wice_soisno, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_h2osoi', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_h2osoi', landpatch, 'patch', a_h2osoi, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_h2osoi', dims(1,i), landpatch, a_h2osoi, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_qlayer', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_qlayer', landpatch, 'patch', a_qlayer, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_qlayer', dims(1,i), landpatch, a_qlayer, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_lake_deficit', landpatch, 'patch', a_lake_deficit, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_lake_deficit', landpatch, a_lake_deficit, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_rootr', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_rootr', landpatch, 'patch', a_rootr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_rootr', dims(1,i), landpatch, a_rootr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_BD_all', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_BD_all', landpatch, 'patch', a_BD_all, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_BD_all', dims(1,i), landpatch, a_BD_all, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_wfc', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_wfc', landpatch, 'patch', a_wfc, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_wfc', dims(1,i), landpatch, a_wfc, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_OM_density', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_OM_density', landpatch, 'patch', a_OM_density, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_OM_density', dims(1,i), landpatch, a_OM_density, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_vegwp', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_vegwp', landpatch, 'patch', a_vegwp, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_vegwp', dims(1,i), landpatch, a_vegwp, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_dz_lake', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_dz_lake', landpatch, 'patch', a_dz_lake, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_dz_lake', dims(1,i), landpatch, a_dz_lake, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_t_lake', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_t_lake', landpatch, 'patch', a_t_lake, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_t_lake', dims(1,i), landpatch, a_t_lake, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_lake_icefrac', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_lake_icefrac', landpatch, 'patch', a_lake_icefrac, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_lake_icefrac', dims(1,i), landpatch, a_lake_icefrac, known_present=.true.)
         ENDIF
      ENDIF
      IF ((DEF_USE_BGC)) THEN
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_litr1c_vr', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_litr1c_vr', landpatch, 'patch', a_litr1c_vr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_litr1c_vr', dims(1,i), landpatch, a_litr1c_vr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_litr2c_vr', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_litr2c_vr', landpatch, 'patch', a_litr2c_vr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_litr2c_vr', dims(1,i), landpatch, a_litr2c_vr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_litr3c_vr', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_litr3c_vr', landpatch, 'patch', a_litr3c_vr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_litr3c_vr', dims(1,i), landpatch, a_litr3c_vr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_soil1c_vr', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_soil1c_vr', landpatch, 'patch', a_soil1c_vr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_soil1c_vr', dims(1,i), landpatch, a_soil1c_vr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_soil2c_vr', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_soil2c_vr', landpatch, 'patch', a_soil2c_vr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_soil2c_vr', dims(1,i), landpatch, a_soil2c_vr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_soil3c_vr', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_soil3c_vr', landpatch, 'patch', a_soil3c_vr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_soil3c_vr', dims(1,i), landpatch, a_soil3c_vr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_cwdc_vr', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_cwdc_vr', landpatch, 'patch', a_cwdc_vr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_cwdc_vr', dims(1,i), landpatch, a_cwdc_vr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_litr1n_vr', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_litr1n_vr', landpatch, 'patch', a_litr1n_vr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_litr1n_vr', dims(1,i), landpatch, a_litr1n_vr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_litr2n_vr', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_litr2n_vr', landpatch, 'patch', a_litr2n_vr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_litr2n_vr', dims(1,i), landpatch, a_litr2n_vr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_litr3n_vr', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_litr3n_vr', landpatch, 'patch', a_litr3n_vr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_litr3n_vr', dims(1,i), landpatch, a_litr3n_vr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_soil1n_vr', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_soil1n_vr', landpatch, 'patch', a_soil1n_vr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_soil1n_vr', dims(1,i), landpatch, a_soil1n_vr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_soil2n_vr', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_soil2n_vr', landpatch, 'patch', a_soil2n_vr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_soil2n_vr', dims(1,i), landpatch, a_soil2n_vr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_soil3n_vr', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_soil3n_vr', landpatch, 'patch', a_soil3n_vr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_soil3n_vr', dims(1,i), landpatch, a_soil3n_vr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_cwdn_vr', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_cwdn_vr', landpatch, 'patch', a_cwdn_vr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_cwdn_vr', dims(1,i), landpatch, a_cwdn_vr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_totsoiln_vr', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_totsoiln_vr', landpatch, 'patch', a_totsoiln_vr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_totsoiln_vr', dims(1,i), landpatch, a_totsoiln_vr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_litr1cCap_vr', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_litr1cCap_vr', landpatch, 'patch', a_litr1cCap_vr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_litr1cCap_vr', dims(1,i), landpatch, a_litr1cCap_vr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_litr2cCap_vr', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_litr2cCap_vr', landpatch, 'patch', a_litr2cCap_vr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_litr2cCap_vr', dims(1,i), landpatch, a_litr2cCap_vr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_litr3cCap_vr', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_litr3cCap_vr', landpatch, 'patch', a_litr3cCap_vr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_litr3cCap_vr', dims(1,i), landpatch, a_litr3cCap_vr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_soil1cCap_vr', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_soil1cCap_vr', landpatch, 'patch', a_soil1cCap_vr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_soil1cCap_vr', dims(1,i), landpatch, a_soil1cCap_vr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_soil2cCap_vr', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_soil2cCap_vr', landpatch, 'patch', a_soil2cCap_vr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_soil2cCap_vr', dims(1,i), landpatch, a_soil2cCap_vr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_soil3cCap_vr', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_soil3cCap_vr', landpatch, 'patch', a_soil3cCap_vr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_soil3cCap_vr', dims(1,i), landpatch, a_soil3cCap_vr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_cwdcCap_vr', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_cwdcCap_vr', landpatch, 'patch', a_cwdcCap_vr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_cwdcCap_vr', dims(1,i), landpatch, a_cwdcCap_vr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_litr1nCap_vr', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_litr1nCap_vr', landpatch, 'patch', a_litr1nCap_vr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_litr1nCap_vr', dims(1,i), landpatch, a_litr1nCap_vr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_litr2nCap_vr', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_litr2nCap_vr', landpatch, 'patch', a_litr2nCap_vr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_litr2nCap_vr', dims(1,i), landpatch, a_litr2nCap_vr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_litr3nCap_vr', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_litr3nCap_vr', landpatch, 'patch', a_litr3nCap_vr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_litr3nCap_vr', dims(1,i), landpatch, a_litr3nCap_vr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_soil1nCap_vr', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_soil1nCap_vr', landpatch, 'patch', a_soil1nCap_vr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_soil1nCap_vr', dims(1,i), landpatch, a_soil1nCap_vr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_soil2nCap_vr', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_soil2nCap_vr', landpatch, 'patch', a_soil2nCap_vr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_soil2nCap_vr', dims(1,i), landpatch, a_soil2nCap_vr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_soil3nCap_vr', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_soil3nCap_vr', landpatch, 'patch', a_soil3nCap_vr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_soil3nCap_vr', dims(1,i), landpatch, a_soil3nCap_vr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_cwdnCap_vr', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_cwdnCap_vr', landpatch, 'patch', a_cwdnCap_vr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_cwdnCap_vr', dims(1,i), landpatch, a_cwdnCap_vr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_t_scalar', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_t_scalar', landpatch, 'patch', a_t_scalar, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_t_scalar', dims(1,i), landpatch, a_t_scalar, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_w_scalar', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_w_scalar', landpatch, 'patch', a_w_scalar, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_w_scalar', dims(1,i), landpatch, a_w_scalar, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_sminn_vr', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_sminn_vr', landpatch, 'patch', a_sminn_vr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_sminn_vr', dims(1,i), landpatch, a_sminn_vr, known_present=.true.)
         ENDIF
      ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_ustar', landpatch, 'patch', a_ustar, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_ustar', landpatch, a_ustar, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_ustar2', landpatch, 'patch', a_ustar2, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_ustar2', landpatch, a_ustar2, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_tstar', landpatch, 'patch', a_tstar, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_tstar', landpatch, a_tstar, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_qstar', landpatch, 'patch', a_qstar, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_qstar', landpatch, a_qstar, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_zol', landpatch, 'patch', a_zol, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_zol', landpatch, a_zol, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_rib', landpatch, 'patch', a_rib, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_rib', landpatch, a_rib, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_fm', landpatch, 'patch', a_fm, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_fm', landpatch, a_fm, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_fh', landpatch, 'patch', a_fh, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_fh', landpatch, a_fh, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_fq', landpatch, 'patch', a_fq, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_fq', landpatch, a_fq, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_us10m', landpatch, 'patch', a_us10m, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_us10m', landpatch, a_us10m, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_vs10m', landpatch, 'patch', a_vs10m, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_vs10m', landpatch, a_vs10m, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_fm10m', landpatch, 'patch', a_fm10m, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_fm10m', landpatch, a_fm10m, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_sr', landpatch, 'patch', a_sr, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_sr', landpatch, a_sr, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_solvd', landpatch, 'patch', a_solvd, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_solvd', landpatch, a_solvd, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_solvi', landpatch, 'patch', a_solvi, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_solvi', landpatch, a_solvi, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_solnd', landpatch, 'patch', a_solnd, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_solnd', landpatch, a_solnd, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_solni', landpatch, 'patch', a_solni, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_solni', landpatch, a_solni, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_srvd', landpatch, 'patch', a_srvd, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_srvd', landpatch, a_srvd, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_srvi', landpatch, 'patch', a_srvi, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_srvi', landpatch, a_srvi, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_srnd', landpatch, 'patch', a_srnd, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_srnd', landpatch, a_srnd, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_srni', landpatch, 'patch', a_srni, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_srni', landpatch, a_srni, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_solvdln', landpatch, 'patch', a_solvdln, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_solvdln', landpatch, a_solvdln, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_solviln', landpatch, 'patch', a_solviln, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_solviln', landpatch, a_solviln, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_solndln', landpatch, 'patch', a_solndln, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_solndln', landpatch, a_solndln, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_solniln', landpatch, 'patch', a_solniln, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_solniln', landpatch, a_solniln, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_srvdln', landpatch, 'patch', a_srvdln, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_srvdln', landpatch, a_srvdln, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_srviln', landpatch, 'patch', a_srviln, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_srviln', landpatch, a_srviln, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_srndln', landpatch, 'patch', a_srndln, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_srndln', landpatch, a_srndln, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) THEN
            CALL history_acc_write(file, 'a_srniln', landpatch, 'patch', a_srniln, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_srniln', landpatch, a_srniln, known_present=.true.)
         ENDIF
      ENDIF
#if (defined HYPERSPECTRAL)
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_sol_dir_ln_hires', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_sol_dir_ln_hires', landpatch, 'patch', a_sol_dir_ln_hires, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_sol_dir_ln_hires', dims(1,i), landpatch, a_sol_dir_ln_hires, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_sol_dif_ln_hires', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_sol_dif_ln_hires', landpatch, 'patch', a_sol_dif_ln_hires, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_sol_dif_ln_hires', dims(1,i), landpatch, a_sol_dif_ln_hires, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_sr_dir_ln_hires', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_sr_dir_ln_hires', landpatch, 'patch', a_sr_dir_ln_hires, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_sr_dir_ln_hires', dims(1,i), landpatch, a_sr_dir_ln_hires, known_present=.true.)
         ENDIF
      ENDIF
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_sr_dif_ln_hires', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_sr_dif_ln_hires', landpatch, 'patch', a_sr_dif_ln_hires, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_sr_dif_ln_hires', dims(1,i), landpatch, a_sr_dif_ln_hires, known_present=.true.)
         ENDIF
      ENDIF
#endif
      i=i+1
      IF (present_flags(i) /= 0) THEN
         IF (writing) CALL ncio_define_dimension_vector(file, landpatch, 'd1_a_sensors', dims(1,i))
         IF (writing) THEN
            CALL history_acc_write(file, 'a_sensors', landpatch, 'patch', a_sensors, dims(1,i), dims(2,i))
         ELSE
            CALL ncio_read_vector(file, 'a_sensors', dims(1,i), landpatch, a_sensors, known_present=.true.)
         ENDIF
      ENDIF
   END SUBROUTINE history_acc_transfer

   SUBROUTINE history_acc_write1(file, name, pixelset, dim, array, n1, n2)
   USE MOD_NetCDFVector
   USE MOD_Pixelset, only: pixelset_type
   IMPLICIT NONE
   character(len=*), intent(in) :: file, name, dim
   type(pixelset_type), intent(in) :: pixelset
   real(r8), allocatable, intent(inout) :: array(:)
   integer, intent(in) :: n1, n2
   logical :: temporary
      temporary=.not.allocated(array)
      IF (temporary) allocate(array(0))
      CALL ncio_write_vector(file, name, dim, pixelset, array)
      IF (temporary) deallocate(array)
   END SUBROUTINE history_acc_write1

   SUBROUTINE history_acc_write2(file, name, pixelset, dim, array, n1, n2)
   USE MOD_NetCDFVector
   USE MOD_Pixelset, only: pixelset_type
   IMPLICIT NONE
   character(len=*), intent(in) :: file, name, dim
   type(pixelset_type), intent(in) :: pixelset
   real(r8), allocatable, intent(inout) :: array(:,:)
   integer, intent(in) :: n1, n2
   logical :: temporary
      temporary=.not.allocated(array)
      IF (temporary) allocate(array(0,0))
      CALL ncio_write_vector(file, name, 'd1_'//trim(name), n1, dim, pixelset, array)
      IF (temporary) deallocate(array)
   END SUBROUTINE history_acc_write2

   SUBROUTINE history_acc_write3(file, name, pixelset, dim, array, n1, n2)
   USE MOD_NetCDFVector
   USE MOD_Pixelset, only: pixelset_type
   IMPLICIT NONE
   character(len=*), intent(in) :: file, name, dim
   type(pixelset_type), intent(in) :: pixelset
   real(r8), allocatable, intent(inout) :: array(:,:,:)
   integer, intent(in) :: n1, n2
   logical :: temporary
      temporary=.not.allocated(array)
      IF (temporary) allocate(array(0,0,0))
      CALL ncio_write_vector(file, name, 'd1_'//trim(name), n1, &
         'd2_'//trim(name), n2, dim, pixelset, array)
      IF (temporary) deallocate(array)
   END SUBROUTINE history_acc_write3

   integer FUNCTION history_acc_freq()
   USE MOD_Namelist, only: DEF_HIST_FREQ
   IMPLICIT NONE
      SELECT CASE (trim(adjustl(DEF_HIST_FREQ)))
      CASE ('TIMESTEP'); history_acc_freq=1
      CASE ('HOURLY'); history_acc_freq=2
      CASE ('DAILY'); history_acc_freq=3
      CASE ('MONTHLY'); history_acc_freq=4
      CASE ('YEARLY'); history_acc_freq=5
      CASE DEFAULT; history_acc_freq=0
      END SELECT
   END FUNCTION history_acc_freq

   SUBROUTINE history_acc_file(idate, site, dir_restart, file)
   IMPLICIT NONE
   integer, intent(in) :: idate(3)
   character(len=*), intent(in) :: site, dir_restart
   character(len=*), intent(out) :: file
   character(len=14) :: cdate
      write(cdate,'(i4.4,"-",i3.3,"-",i5.5)') idate
      file=trim(dir_restart)//'/'//cdate//'/'//trim(site)//'_restart_hist_'//cdate//'.nc'
   END SUBROUTINE history_acc_file

#ifdef GridRiverLakeFlow
   SUBROUTINE history_river_acc_file(idate, site, dir_restart, file)
   IMPLICIT NONE
   integer, intent(in) :: idate(3)
   character(len=*), intent(in) :: site, dir_restart
   character(len=*), intent(out) :: file
      CALL history_acc_file(idate, site, dir_restart, file)
      file=trim(file)//'.river'
   END SUBROUTINE history_river_acc_file

   SUBROUTINE restore_river_history_acc_restart(idate, site, dir_restart)
   USE MOD_SPMD_Task
   USE MOD_LandPatch, only: landpatch, numpatch
   USE MOD_NetCDFVector, only: ncio_read_vector, ncio_vector_var_present
   USE MOD_Grid_RiverLakeHistState, only: read_gridriverlake_hist_restart
   USE MOD_Grid_RiverLakeNetwork, only: totalnumucat
   USE MOD_NetCDFSerial, only: ncio_var_exist
   USE MOD_Tracer_RiverLake, only: read_tracer_history_acc_restart
   USE MOD_Tracer_Particle_Sediment, only: sediment_history_acc_sidecar
   IMPLICIT NONE
   integer, intent(in) :: idate(3)
   character(len=*), intent(in) :: site, dir_restart
   character(len=256) :: file, river_file
   real(r8), allocatable :: marker(:)
   integer :: required
   integer :: schema
   logical :: exists

      CALL history_acc_file(idate, site, dir_restart, file)
      IF (.not.ncio_vector_var_present(file, 'history_schema', landpatch)) RETURN
      CALL ncio_read_vector(file, 'history_schema', landpatch, marker, known_present=.true.)
      schema=1
      IF (p_is_worker .and. numpatch>0) THEN
         IF (any(marker==2._r8)) schema=2
      ENDIF
#ifdef USEMPI
      CALL mpi_allreduce(MPI_IN_PLACE, schema, 1, MPI_INTEGER, MPI_MAX, p_comm_glb, p_err)
#endif
      IF (allocated(marker)) deallocate(marker)
      IF (.not.ncio_vector_var_present(file, 'history_river_required', landpatch)) THEN
         IF (schema>=2) CALL CoLM_stop('committed river-history sidecar marker is missing')
         RETURN
      ENDIF
      CALL history_acc_require_shape(file, 'history_river_required', landpatch, 'patch', 1, 0, 0)
      CALL ncio_read_vector(file, 'history_river_required', landpatch, marker, known_present=.true.)
      required=0
      IF (p_is_worker .and. numpatch>0) THEN
         IF (any(marker/=0._r8 .and. marker/=1._r8)) &
            CALL CoLM_stop('invalid river-history sidecar marker')
         IF (any(marker==1._r8)) required=1
      ENDIF
#ifdef USEMPI
      CALL mpi_allreduce(MPI_IN_PLACE, required, 1, MPI_INTEGER, MPI_MAX, p_comm_glb, p_err)
#endif
      IF (allocated(marker)) deallocate(marker)
      IF (required==0) RETURN
      CALL history_river_acc_file(idate, site, dir_restart, river_file)
      exists=.false.
      IF (p_is_master) exists=ncio_var_exist(river_file, 'hist_acctime_ucat', readflag=.false.)
#ifdef USEMPI
      CALL mpi_bcast(exists, 1, MPI_LOGICAL, p_address_master, p_comm_glb, p_err)
#endif
      IF (totalnumucat>0 .and. .not.exists) &
         CALL CoLM_stop('committed river-history sidecar is missing')
      ! grid_riverlake_flow_init has already validated physical BIF identity
      ! and initialized tracer providers before this raw-window restoration.
      CALL read_gridriverlake_hist_restart(river_file, primary_identity_validated=.true., strict=.true.)
      IF (DEF_USE_TRACER) THEN
      CALL read_tracer_history_acc_restart(river_file)
      CALL sediment_history_acc_sidecar(river_file, .false.)
      ENDIF
   END SUBROUTINE restore_river_history_acc_restart
#endif

   SUBROUTINE history_primary_file(idate, lc_year, site, dir_restart, file)
   IMPLICIT NONE
   integer, intent(in) :: idate(3), lc_year
   character(len=*), intent(in) :: site, dir_restart
   character(len=*), intent(out) :: file
   character(len=14) :: cdate
   character(len=4) :: cyear
      write(cdate,'(i4.4,"-",i3.3,"-",i5.5)') idate
      write(cyear,'(i4.4)') lc_year
      file=trim(dir_restart)//'/'//cdate//'/'//trim(site)//'_restart_'//cdate//'_lc'//cyear//'.nc'
   END SUBROUTINE history_primary_file

   SUBROUTINE mark_history_acc_restart(idate, lc_year, site, dir_restart)
   USE MOD_SPMD_Task
   USE MOD_LandPatch, only: landpatch, numpatch
   USE MOD_NetCDFVector
   IMPLICIT NONE
   integer, intent(in) :: idate(3), lc_year
   character(len=*), intent(in) :: site, dir_restart
   character(len=256) :: file
   real(r8), allocatable :: marker(:)
      CALL history_primary_file(idate, lc_year, site, dir_restart, file)
      IF (p_is_worker) THEN
         allocate(marker(numpatch))
      ELSE
         allocate(marker(0))
      ENDIF
      marker=1._r8
      ! Require the pending sidecar before its final completion marker is written.
      CALL ncio_write_vector(file, 'history_sidecar_required', 'patch', landpatch, marker)
      deallocate(marker)
   END SUBROUTINE mark_history_acc_restart

   SUBROUTINE history_acc_require_shape(file, name, pixelset, vector_dim, rank, n1, n2)
   USE netcdf
   USE MOD_SPMD_Task
   USE MOD_Block, only: get_filename_block
   USE MOD_Pixelset, only: pixelset_type
   IMPLICIT NONE
   character(len=*), intent(in) :: file, name, vector_dim
   type(pixelset_type), intent(in) :: pixelset
   integer, intent(in) :: rank, n1, n2
   integer :: iblkgrp, iblk, jblk, ncid, varid, ndims, xtype, status, j, length
   integer, allocatable :: dimids(:)
   character(len=256) :: block_file
   character(len=NF90_MAX_NAME) :: actual_dim
   character(len=NF90_MAX_NAME) :: expected_dim(3)
   logical :: valid

      IF (.not.p_is_io) RETURN
      expected_dim(1)='d1_'//trim(name)
      expected_dim(2)='d2_'//trim(name)
      expected_dim(3)=vector_dim
      IF (rank==1) expected_dim(1)=vector_dim
      IF (rank==2) expected_dim(2)=vector_dim
      DO iblkgrp=1,pixelset%nblkgrp
         iblk=pixelset%xblkgrp(iblkgrp)
         jblk=pixelset%yblkgrp(iblkgrp)
         CALL get_filename_block(file, iblk, jblk, block_file)
         valid=.true.
         status=nf90_open(trim(block_file), NF90_NOWRITE, ncid)
         IF (status/=NF90_NOERR) valid=.false.
         IF (valid) THEN
            status=nf90_inq_varid(ncid, trim(name), varid)
            IF (status/=NF90_NOERR) valid=.false.
         ENDIF
         IF (valid) THEN
            status=nf90_inquire_variable(ncid, varid, xtype=xtype, ndims=ndims)
            IF (status/=NF90_NOERR) valid=.false.
         ENDIF
         IF (valid) THEN
            IF (xtype/=NF90_DOUBLE .or. ndims/=rank) valid=.false.
         ENDIF
         IF (valid) THEN
            allocate(dimids(rank))
            status=nf90_inquire_variable(ncid, varid, dimids=dimids)
            IF (status/=NF90_NOERR) valid=.false.
         ENDIF
         IF (valid) THEN
            DO j=1,rank
               status=nf90_inquire_dimension(ncid, dimids(j), name=actual_dim, len=length)
               IF (status/=NF90_NOERR) THEN
                  valid=.false.
                  EXIT
               ENDIF
               IF (trim(actual_dim)/=trim(expected_dim(j))) valid=.false.
               IF (j==1 .and. rank>1 .and. length/=n1) valid=.false.
               IF (j==2 .and. rank>2 .and. length/=n2) valid=.false.
               IF (j==rank .and. length/=pixelset%vecgs%vlen(iblk,jblk)) valid=.false.
            ENDDO
         ENDIF
         IF (allocated(dimids)) deallocate(dimids)
         IF (status==NF90_NOERR) status=nf90_close(ncid)
         IF (.not.valid .or. status/=NF90_NOERR) THEN
            write(*,'(A)') 'ERROR: incompatible land-history sidecar shape for '//trim(name)// &
               ' in '//trim(block_file)
            CALL CoLM_stop()
         ENDIF
      ENDDO
   END SUBROUTINE history_acc_require_shape

   SUBROUTINE write_history_acc_restart(idate, site, dir_restart)
   USE MOD_SPMD_Task
   USE MOD_LandPatch, only: landpatch, numpatch
   USE MOD_LandUrban, only: landurban
   USE MOD_NetCDFVector
   USE MOD_Tracer_Vars, only: tracer_history_required, tracer_history_write
   USE MOD_Tracer_Lifecycle, only: tracer_lifecycle_land_history_sidecar
#ifdef GridRiverLakeFlow
   USE MOD_Grid_RiverLakeHistState, only: acctime_ucat, write_gridriverlake_hist_restart
   USE MOD_Grid_RiverLakeNetwork, only: totalnumucat
   USE MOD_Grid_Reservoir, only: totalnumresv
   USE MOD_NetCDFSerial, only: ncio_create_file, ncio_define_dimension
   USE MOD_Tracer_RiverLake, only: write_tracer_history_acc_restart
   USE MOD_Tracer_Particle_Sediment, only: sediment_history_acc_sidecar
#endif
   IMPLICIT NONE
   integer, intent(in) :: idate(3)
   character(len=*), intent(in) :: site, dir_restart
   character(len=256) :: file
#ifdef GridRiverLakeFlow
   character(len=256) :: river_file
   integer :: river_active
#endif
   character(len=14) :: cdate
   character(len=64) :: names(700)
   logical :: urban(700)
   integer :: ranks(700), local(700), flags(700), dims(2,700), nfields
   real(r8), allocatable :: marker(:)
   integer :: window_active
      CALL history_acc_file(idate, site, dir_restart, file)
      write(cdate,'(i4.4,"-",i3.3,"-",i5.5)') idate
      IF (p_is_master) CALL system('mkdir -p '//trim(dir_restart)//'/'//cdate)
#ifdef USEMPI
      CALL mpi_barrier(p_comm_glb, p_err)
#endif
      window_active=0
      IF (p_is_worker) THEN
         IF (nac>0._r8) window_active=1
      ENDIF
#ifdef USEMPI
      CALL mpi_allreduce(MPI_IN_PLACE, window_active, 1, MPI_INTEGER, MPI_MAX, p_comm_glb, p_err)
#endif
      IF (window_active/=0) THEN
      CALL history_acc_manifest(names, ranks, urban, local, dims, nfields)
      flags=local
#ifdef USEMPI
      CALL mpi_allreduce(MPI_IN_PLACE, flags, 700, MPI_INTEGER, MPI_MAX, p_comm_glb, p_err)
      CALL mpi_allreduce(MPI_IN_PLACE, dims, 1400, MPI_INTEGER, MPI_MAX, p_comm_glb, p_err)
#endif
      ENDIF
      CALL ncio_create_file_vector(file, landpatch)
      CALL ncio_define_dimension_vector(file, landpatch, 'patch')
      IF (DEF_URBAN_RUN) THEN
      CALL ncio_define_dimension_vector(file, landurban, 'urban')
      ENDIF
      IF (p_is_worker) THEN
         allocate(marker(numpatch))
      ELSE
         allocate(marker(0))
      ENDIF
      marker=1._r8
#ifdef GridRiverLakeFlow
      marker=2._r8
#endif
      CALL ncio_write_vector(file, 'history_schema', 'patch', landpatch, marker)
      marker=real(history_acc_freq(),r8)
      CALL ncio_write_vector(file, 'history_freq', 'patch', landpatch, marker)
      marker=0._r8
      IF (p_is_worker) marker=nac
      CALL ncio_write_vector(file, 'history_nac', 'patch', landpatch, marker)
      IF (window_active/=0) CALL history_acc_transfer(file, .true., flags, dims)
      IF (DEF_USE_TRACER) THEN
      IF (window_active/=0 .and. tracer_history_required()) CALL tracer_history_write(file)
      IF (window_active/=0) CALL tracer_lifecycle_land_history_sidecar(file, .true.)
      ENDIF
#ifdef GridRiverLakeFlow
      river_active=0
      IF (p_is_worker .and. allocated(acctime_ucat)) THEN
         IF (any(acctime_ucat>0._r8)) river_active=1
      ENDIF
#ifdef USEMPI
      CALL mpi_allreduce(MPI_IN_PLACE, river_active, 1, MPI_INTEGER, MPI_MAX, p_comm_glb, p_err)
#endif
      marker=real(river_active,r8)
      CALL ncio_write_vector(file, 'history_river_required', 'patch', landpatch, marker)
      IF (river_active/=0) THEN
         CALL history_river_acc_file(idate, site, dir_restart, river_file)
         IF (p_is_master) THEN
            CALL ncio_create_file(river_file)
            IF (totalnumucat>0) CALL ncio_define_dimension(river_file, 'ucatch', totalnumucat)
            IF (totalnumresv>0) CALL ncio_define_dimension(river_file, 'reservoir', totalnumresv)
         ENDIF
#ifdef USEMPI
         CALL mpi_barrier(p_comm_glb, p_err)
#endif
         CALL write_gridriverlake_hist_restart(river_file)
      IF (DEF_USE_TRACER) THEN
         CALL write_tracer_history_acc_restart(river_file)
         CALL sediment_history_acc_sidecar(river_file, .true.)
      ENDIF
      ENDIF
#endif
      marker=0._r8
      CALL ncio_write_vector(file, 'history_complete', 'patch', landpatch, marker)
      deallocate(marker)
   END SUBROUTINE write_history_acc_restart

   SUBROUTINE complete_history_acc_restart(idate, site, dir_restart)
   USE MOD_SPMD_Task
   USE MOD_LandPatch, only: landpatch, numpatch
   USE MOD_NetCDFVector
   IMPLICIT NONE
   integer, intent(in) :: idate(3)
   character(len=*), intent(in) :: site, dir_restart
   character(len=256) :: file
   real(r8), allocatable :: marker(:)
      CALL history_acc_file(idate, site, dir_restart, file)
      IF (p_is_worker) THEN
         allocate(marker(numpatch))
      ELSE
         allocate(marker(0))
      ENDIF
      marker=1._r8
      CALL ncio_write_vector(file, 'history_complete', 'patch', landpatch, marker)
      deallocate(marker)
   END SUBROUTINE complete_history_acc_restart

   logical FUNCTION history_acc_any_file(file)
   USE MOD_SPMD_Task
   USE MOD_Block, only: get_filename_block
   USE MOD_LandPatch, only: landpatch
   USE MOD_LandUrban, only: landurban
   IMPLICIT NONE
   character(len=*), intent(in) :: file
   character(len=256) :: block_file
   integer :: iblkgrp, iblk, jblk, found, globally_found
   logical :: exists
      found=0
      IF (p_is_io) THEN
         DO iblkgrp=1,landpatch%nblkgrp
            iblk=landpatch%xblkgrp(iblkgrp)
            jblk=landpatch%yblkgrp(iblkgrp)
            CALL get_filename_block(file, iblk, jblk, block_file)
            inquire(file=trim(block_file), exist=exists)
            IF (exists) found=1
         ENDDO
      IF (DEF_URBAN_RUN) THEN
         DO iblkgrp=1,landurban%nblkgrp
            iblk=landurban%xblkgrp(iblkgrp)
            jblk=landurban%yblkgrp(iblkgrp)
            CALL get_filename_block(file, iblk, jblk, block_file)
            inquire(file=trim(block_file), exist=exists)
            IF (exists) found=1
         ENDDO
      ENDIF
      ENDIF
      globally_found=found
#ifdef USEMPI
      CALL mpi_allreduce(found, globally_found, 1, MPI_INTEGER, MPI_MAX, p_comm_glb, p_err)
#endif
      history_acc_any_file=globally_found/=0
   END FUNCTION history_acc_any_file

   SUBROUTINE read_history_acc_restart(idate, lc_year, site, dir_restart)
   USE, INTRINSIC :: ieee_arithmetic, only: ieee_is_finite
   USE MOD_SPMD_Task
   USE MOD_LandPatch, only: landpatch, numpatch
   USE MOD_LandUrban, only: landurban
   USE MOD_NetCDFVector
   USE MOD_Tracer_Vars, only: TRC_HISTORY_FIELDS, tracer_history_required, &
      tracer_history_manifest, tracer_history_restore
   USE MOD_Tracer_Lifecycle, only: tracer_lifecycle_land_history_sidecar
   IMPLICIT NONE
   integer, intent(in) :: idate(3), lc_year
   character(len=*), intent(in) :: site, dir_restart
   character(len=256) :: file, primary_file
   character(len=64) :: names(700), patch_names(704), urban_names(700)
   logical :: urban(700), markers(4), patch_present(704), urban_present(700), required, any_sidecar
   integer :: ranks(700), local(700), flags(700), dims(2,700), nfields
   integer :: i, npatch_names, nurban_names
   real(r8) :: nac_min, nac_max
   real(r8), allocatable :: marker(:)
   integer :: window_active
   character(len=64) :: trc_names(TRC_HISTORY_FIELDS)
   integer :: trc_ranks(TRC_HISTORY_FIELDS), trc_dims(2,TRC_HISTORY_FIELDS)
   logical :: trc_present(TRC_HISTORY_FIELDS)
      CALL history_acc_file(idate, site, dir_restart, file)
      CALL history_primary_file(idate, lc_year, site, dir_restart, primary_file)
      required=ncio_vector_var_present(primary_file, 'history_sidecar_required', landpatch)
      any_sidecar=history_acc_any_file(file)
      CALL ncio_vector_group_presence(file, &
         [character(len=64) :: 'history_schema', 'history_freq', 'history_nac', 'history_complete'], &
         landpatch, markers)
      IF (.not.any(markers)) THEN
         IF (any_sidecar) THEN
            IF (p_is_master) write(*,'(A)') 'ERROR: incomplete land-history restart sidecar.'
            CALL CoLM_stop()
         ENDIF
         IF (required) THEN
            IF (p_is_master) write(*,'(A)') 'ERROR: land restart requires a missing history sidecar.'
            CALL CoLM_stop()
         ENDIF
         IF (p_is_master) write(*,'(A)') &
            'Warning: legacy land restart lacks history accumulators; intra-period history may be partial.'
         RETURN
      ENDIF
      IF (.not.all(markers)) THEN
         IF (p_is_master) write(*,'(A)') 'ERROR: incomplete land-history restart sidecar.'
         CALL CoLM_stop()
      ENDIF
      CALL history_acc_require_shape(file, 'history_schema', landpatch, 'patch', 1, 0, 0)
      CALL history_acc_require_shape(file, 'history_freq', landpatch, 'patch', 1, 0, 0)
      CALL history_acc_require_shape(file, 'history_nac', landpatch, 'patch', 1, 0, 0)
      CALL history_acc_require_shape(file, 'history_complete', landpatch, 'patch', 1, 0, 0)
      CALL ncio_read_vector(file, 'history_schema', landpatch, marker, known_present=.true.)
      IF (p_is_worker .and. numpatch>0) THEN
         IF (any(.not.ieee_is_finite(marker))) THEN
            write(*,'(A)') 'ERROR: non-finite land-history schema marker.'
            CALL CoLM_stop()
         ENDIF
         IF (any(marker/=1._r8 .and. marker/=2._r8)) THEN
            write(*,'(A)') 'ERROR: unsupported land-history sidecar schema.'
            CALL CoLM_stop()
         ENDIF
      ENDIF
      CALL ncio_read_vector(file, 'history_complete', landpatch, marker, known_present=.true.)
      IF (p_is_worker .and. numpatch>0) THEN
         IF (any(.not.ieee_is_finite(marker))) THEN
            write(*,'(A)') 'ERROR: non-finite land-history commit marker.'
            CALL CoLM_stop()
         ENDIF
         IF (any(marker/=1._r8)) THEN
            write(*,'(A)') 'ERROR: incomplete land-history sidecar commit marker.'
            CALL CoLM_stop()
         ENDIF
      ENDIF
      CALL ncio_read_vector(file, 'history_freq', landpatch, marker, known_present=.true.)
      IF (p_is_worker .and. numpatch>0) THEN
         IF (any(.not.ieee_is_finite(marker))) THEN
            write(*,'(A)') 'ERROR: non-finite land-history frequency marker.'
            CALL CoLM_stop()
         ENDIF
         IF (any(marker/=real(history_acc_freq(),r8))) THEN
            write(*,'(A)') 'ERROR: history frequency changed across land restart.'
            CALL CoLM_stop()
         ENDIF
      ENDIF
      CALL ncio_read_vector(file, 'history_nac', landpatch, marker, known_present=.true.)
      IF (p_is_worker .and. numpatch>0) THEN
         IF (any(.not.ieee_is_finite(marker))) THEN
            write(*,'(A)') 'ERROR: non-finite land-history sample count.'
            CALL CoLM_stop()
         ENDIF
         IF (any(marker<0._r8) .or. any(marker/=anint(marker))) THEN
            write(*,'(A)') 'ERROR: invalid land-history sample count.'
            CALL CoLM_stop()
         ENDIF
         nac=marker(1)
         IF (any(marker/=nac)) THEN
            write(*,'(A)') 'ERROR: inconsistent history sample counts in sidecar.'
            CALL CoLM_stop()
         ENDIF
      ENDIF
      IF (p_is_worker) THEN
         nac_min=huge(nac_min)
         nac_max=-huge(nac_max)
         IF (numpatch>0) THEN
            nac_min=nac
            nac_max=nac
         ENDIF
#ifdef USEMPI
         CALL mpi_allreduce(MPI_IN_PLACE, nac_min, 1, MPI_REAL8, MPI_MIN, p_comm_worker, p_err)
         CALL mpi_allreduce(MPI_IN_PLACE, nac_max, 1, MPI_REAL8, MPI_MAX, p_comm_worker, p_err)
#endif
         IF (nac_max>=0._r8 .and. nac_min/=nac_max) THEN
            write(*,'(A)') 'ERROR: inconsistent land-history counts across workers.'
            CALL CoLM_stop()
         ENDIF
      ENDIF
      IF (allocated(marker)) deallocate(marker)
      window_active=0
      IF (p_is_worker) THEN
         IF (nac>0._r8) window_active=1
      ENDIF
#ifdef USEMPI
      CALL mpi_allreduce(MPI_IN_PLACE, window_active, 1, MPI_INTEGER, MPI_MAX, p_comm_glb, p_err)
#endif
      IF (window_active==0) RETURN
      CALL history_acc_manifest(names, ranks, urban, local, dims, nfields)
      flags=local
#ifdef USEMPI
      CALL mpi_allreduce(MPI_IN_PLACE, flags, 700, MPI_INTEGER, MPI_MAX, p_comm_glb, p_err)
      CALL mpi_allreduce(MPI_IN_PLACE, dims, 1400, MPI_INTEGER, MPI_MAX, p_comm_glb, p_err)
#endif
      npatch_names=0; nurban_names=0
      DO i=1,nfields
         IF (flags(i)==0) CYCLE
         IF (urban(i)) THEN
            nurban_names=nurban_names+1; urban_names(nurban_names)=names(i)
         ELSE
            npatch_names=npatch_names+1; patch_names(npatch_names)=names(i)
         ENDIF
      ENDDO
      IF (npatch_names>0) THEN
         CALL ncio_vector_group_presence(file, patch_names(1:npatch_names), &
            landpatch, patch_present(1:npatch_names))
         IF (.not.all(patch_present(1:npatch_names))) THEN
            IF (p_is_master) write(*,'(A)') 'ERROR: missing land-history accumulator in sidecar.'
            CALL CoLM_stop()
         ENDIF
      ENDIF
      IF (DEF_URBAN_RUN) THEN
      IF (nurban_names>0) THEN
         CALL ncio_vector_group_presence(file, urban_names(1:nurban_names), &
            landurban, urban_present(1:nurban_names))
         IF (.not.all(urban_present(1:nurban_names))) THEN
            IF (p_is_master) write(*,'(A)') 'ERROR: missing urban-history accumulator in sidecar.'
            CALL CoLM_stop()
         ENDIF
      ENDIF
      ENDIF
      DO i=1,nfields
         IF (flags(i)==0) CYCLE
         IF (urban(i)) THEN
            CALL history_acc_require_shape(file, names(i), landurban, 'urban', ranks(i), &
               dims(1,i), dims(2,i))
         ELSE
            CALL history_acc_require_shape(file, names(i), landpatch, 'patch', ranks(i), &
               dims(1,i), dims(2,i))
         ENDIF
      ENDDO
      ! The host denominator and generic Tracer numerators must refer to the
      ! same raw window. A legacy sidecar missing these fields cannot be mixed.
      IF (DEF_USE_TRACER) THEN
      IF (tracer_history_required()) THEN
         CALL tracer_history_manifest(trc_names, trc_ranks, trc_dims)
         CALL ncio_vector_group_presence(file, trc_names, landpatch, trc_present)
         IF (.not.all(trc_present)) &
            CALL CoLM_stop('generic land-tracer history missing from active restart window')
         DO i=1,TRC_HISTORY_FIELDS
            CALL history_acc_require_shape(file, trc_names(i), landpatch, 'patch', &
               trc_ranks(i), trc_dims(1,i), trc_dims(2,i))
         ENDDO
         CALL tracer_history_restore(file)
      ENDIF
      CALL tracer_lifecycle_land_history_sidecar(file, .false.)
      ENDIF
      CALL history_acc_transfer(file, .false., flags, dims)
   END SUBROUTINE read_history_acc_restart
