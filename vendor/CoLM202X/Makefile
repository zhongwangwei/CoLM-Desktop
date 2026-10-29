# Makefile for CoLM program

include include/Makeoptions
HEADER = include/define.h
# TRACER became MaterialTransport upstream and every material-transport
# preprocessing guard was expanded to a runtime switch here (DEF_USE_ISOTOPE /
# DEF_SOLUTE_NAMES / DEF_USE_METHANE in main/MATERIAL/MOD_MT_Namelist.F90), so
# the whole main/MATERIAL object set always builds. The old
# TRACER_ENABLED/METHANE_ENABLED probes are gone with the macros.

INCLUDE_DIR = -Iinclude -I.bld/ -I${NETCDF_INC}
VPATH = include : share : mksrfdata : mkinidata \
	: main : main/MATERIAL : main/MATERIAL/GENERIC_WATER : main/MATERIAL/FAMILY_ISOTOPE : main/MATERIAL/FAMILY_SOLUTE : main/MATERIAL/FAMILY_PARTICLE : main/MATERIAL/FAMILY_GAS : main/HYDRO : main/BGC : main/URBAN : main/LULCC : main/DA \
	: main/ParaOpt : extends/CaMa/src : postprocess : .bld

# ********** Targets ALL **********
.PHONY: all
all : mkdir_build mksrfdata.x mkinidata.x colm.x postprocess.x lib
	@echo ''
	@echo '*******************************************************'
	@echo '*                                                     *'
	@echo '*        Making all CoLM programs successfully.       *'
	@echo '*                                                     *'
	@echo '*******************************************************'
# ******* End of Targets ALL ******

.PHONY: mkdir_build
mkdir_build :
	mkdir -p .bld

OBJS_SHARED =    \
				  MOD_Precision.o              \
				  MOD_SPMD_Task.o              \
				  MOD_Filesystem.o             \
				  CoLM_Mkdir.o                 \
				  MOD_Namelist.o               \
				  MOD_Vars_Global.o            \
				  MOD_Const_Physical.o         \
				  MOD_Const_LC.o               \
				  MOD_Utils.o                  \
				  MOD_IncompleteGamma.o        \
				  MOD_UserDefFun.o             \
				  MOD_TimeManager.o            \
				  MOD_Const_PFT.o              \
				  MOD_NetCDFSerial.o           \
				  MOD_Block.o                  \
				  MOD_Grid.o                   \
				  MOD_Pixel.o                  \
				  MOD_DataType.o               \
				  MOD_NetCDFPoint.o            \
				  MOD_NetCDFBlock.o            \
				  MOD_CatchmentDataReadin.o    \
				  MOD_5x5DataReadin.o          \
				  MOD_Mesh.o                   \
				  MOD_Pixelset.o               \
				  MOD_NetCDFVector.o           \
				  MOD_RangeCheck.o             \
				  MOD_SpatialMapping.o         \
				  MOD_WorkerPushData.o         \
				  MOD_AggregationRequestData.o \
				  MOD_PixelsetShared.o         \
				  MOD_LandElm.o                \
				  MOD_LandHRU.o                \
				  MOD_LandPatch.o              \
				  MOD_Land2mWMO.o              \
				  MOD_LandCrop.o               \
				  MOD_LandPFT.o                \
				  MOD_LandUrban.o              \
				  MOD_Urban_Const_LCZ.o        \
				  MOD_SingleSrfdata.o          \
				  MOD_SrfdataDiag.o            \
				  MOD_SrfdataRestart.o         \
				  MOD_ElmVector.o              \
				  MOD_HRUVector.o              \
				  MOD_MeshFilter.o             \
				  MOD_RegionClip.o

$(filter-out CoLM_Mkdir.o,${OBJS_SHARED}) : %.o : %.F90 ${HEADER}
	${FF} -c ${FOPTS} $(INCLUDE_DIR) -o .bld/$@ $< ${MOD_CMD}.bld

# MOD_Filesystem's portable mkdir (replaces CALL system('mkdir -p ...')).
CoLM_Mkdir.o: share/CoLM_Mkdir.c | mkdir_build
	${CC} ${CFLAGS} -c -o .bld/$@ $<

OBJS_SHARED_T = $(addprefix .bld/,${OBJS_SHARED})

# Material transport settings, also read by mksrfdata (methane surface data);
# every file is empty unless MaterialTransport is defined.
OBJS_MT_CONFIG = \
				  MOD_MT_Namelist.o                \
				  MOD_MT_Defs.o                    \
				  MOD_MT_NcIO.o                    \
				  MOD_MT_Methane_Registry.o        \
				  MOD_MT_Methane_Const.o           \
				  MOD_MT_Methane_Preprocessing.o   \
				  MOD_MT_Methane_PHMapping.o


${OBJS_MT_CONFIG} : %.o : %.F90 ${HEADER} ${OBJS_SHARED}
	${FF} -c ${FOPTS} $(INCLUDE_DIR) -o .bld/$@ $< ${MOD_CMD}.bld

OBJS_MT_CONFIG_T = $(addprefix .bld/,${OBJS_MT_CONFIG})

OBJS_MKSRFDATA = \
				  Aggregation_PercentagesPFT.o           \
				  Aggregation_LAI.o                      \
				  Aggregation_SoilHyperAlbedo.o          \
				  Aggregation_SoilBrightness.o           \
				  Aggregation_LakeDepth.o                \
				  Aggregation_ForestHeight.o             \
				  Aggregation_CanopyStructure.o          \
				  Aggregation_SoilParameters.o           \
				  Aggregation_DBedrock.o                 \
				  Aggregation_Topography.o               \
				  Aggregation_TopoWetness.o              \
				  Aggregation_TopographyFactors.o        \
				  Aggregation_TopographyFactors_Simple.o \
				  Aggregation_Urban.o                    \
				  Aggregation_SoilTexture.o              \
				  MOD_Lulcc_TransferTrace.o              \
				  Aggregation_LakeSoilC.o                \
				  Aggregation_MethanePH.o                \
				  MKSRFDATA.o

MKSRFDATA.o: MOD_MT_Methane_Preprocessing.o

$(OBJS_MKSRFDATA) : %.o : %.F90 ${HEADER} ${OBJS_SHARED} ${OBJS_MT_CONFIG}
	${FF} -c ${FOPTS} $(INCLUDE_DIR) -o .bld/$@ $< ${MOD_CMD}.bld

OBJS_MKSRFDATA_T = $(addprefix .bld/,${OBJS_MKSRFDATA})

# ------- Target 1: mksrfdata --------
mksrfdata.x : mkdir_build ${HEADER} ${OBJS_SHARED} ${OBJS_MT_CONFIG} ${OBJS_MKSRFDATA} ${OBJS_UNITCATCHMENT}
	@echo ''
	@echo 'making CoLM surface data start >>>>>>>>>>>>>>>>>>>>>>>>>>>>>>'
	@echo ''
	${FF} ${FOPTS} ${OBJS_SHARED_T} ${OBJS_MT_CONFIG_T} ${OBJS_MKSRFDATA_T} ${OBJS_UNITCATCHMENT_T} -o run/mksrfdata.x ${LDFLAGS}
	@echo ''
	@echo '<<<<<<<<<<<<<<<<<<<<<<<<<< making CoLM surface data completed!'
	@echo ''
# ----- End of Target 1 mksrfdata ----

OBJS_BASIC =    \
				 MOD_Vector_ReadWrite.o          \
				 MOD_dataSpec_PDB.o              \
				 MOD_tav_abs.o                   \
				 MOD_prospect_DB.o               \
				 MOD_Catch_BasinNetwork.o        \
				 MOD_Catch_Vars_TimeVariables.o  \
				 MOD_Catch_Vars_1DFluxes.o       \
				 MOD_Grid_RiverLakePush.o        \
				 MOD_Grid_RiverLakeVectorIO.o    \
				 MOD_Grid_RiverLakeNetwork.o     \
				 MOD_Grid_Reservoir.o            \
				 MOD_Grid_ReservoirUpstream.o    \
				 MOD_Grid_RiverLakeLevee.o       \
				 MOD_Grid_RiverLakeBifurcation.o \
				 MOD_Grid_RiverLakeHistState.o   \
				 MOD_Grid_RiverLakeHistShard.o   \
				 MOD_Grid_RiverLakeHistRoute.o   \
				 MOD_MT_Solute.o                 \
				 MOD_MT_NitrogenN2O.o            \
				 MOD_MT_Lifecycle.o              \
				 MOD_Grid_RiverLakeTimeVars.o    \
				 MOD_MT_RiverLake.o              \
				 MOD_MT_Sediment.o               \
				 MOD_BGC_Vars_1DFluxes.o         \
				 MOD_BGC_Vars_1DPFTFluxes.o      \
				 MOD_BGC_Vars_PFTimeVariables.o  \
				 MOD_BGC_Vars_TimeInvariants.o   \
				 MOD_BGC_Vars_TimeVariables.o    \
				 MOD_Urban_Vars_1DFluxes.o       \
				 MOD_Urban_Vars_TimeVariables.o  \
				 MOD_Urban_Vars_TimeInvariants.o \
				 MOD_DA_Vars_1DFluxes.o          \
				 MOD_Vars_TimeInvariants.o       \
				 MOD_DA_Vars_TimeVariables.o     \
				 MOD_Vars_TimeVariables.o        \
				 MOD_Vars_1DPFTFluxes.o          \
				 MOD_Vars_1DFluxes.o             \
				 MOD_Vars_1DForcing.o            \
				 MOD_Hydro_SoilFunction.o        \
				 MOD_Hydro_SoilWater.o           \
				 MOD_Eroot.o                     \
				 MOD_Qsadv.o                     \
				 MOD_UserSpecifiedForcing.o      \
				 MOD_MT_Isotope_Registry.o       \
				 MOD_MT_Isotope_O18.o            \
				 MOD_MT_Isotope_HDO.o            \
				 MOD_MT_Isotope_Lulcc.o          \
				 MOD_MT_Isotope_SoilFlow.o       \
				 MOD_MT_Providers.o              \
				 MOD_MT_Frac.o                   \
				 MOD_MT_EvapLimit.o              \
				 MOD_MT_Vars.o                   \
				 MOD_MT_Conservation.o           \
				 MOD_MT_SoilInit.o               \
				 MOD_MT_ForcingInput.o           \
				 MOD_MT_Forcing.o                \
				 MOD_MT_Precip.o                 \
				 MOD_MT_Evapo.o                  \
				 MOD_MT_SoilWater.o              \
				 MOD_MT_NitrogenLeaching.o       \
				 MOD_MT_Snow.o                   \
				 MOD_MT_Rest.o                   \
				 MOD_LAIEmpirical.o              \
				 MOD_LAIReadin.o                 \
				 MOD_CropReadin.o                \
				 MOD_NitrifData.o                \
				 MOD_NdepData.o                  \
				 MOD_FireData.o                  \
				 MOD_OrbCoszen.o                 \
				 MOD_OrbCosazi.o                 \
				 MOD_HighRes_Parameters.o        \
				 MOD_3DCanopyRadiation.o         \
				 MOD_Aerosol.o                   \
				 MOD_SnowSnicar.o                \
				 MOD_Albedo.o                    \
				 MOD_SnowSnicar_HiRes.o          \
				 MOD_Albedo_HiRes.o              \
				 MOD_SnowFraction.o              \
				 MOD_Urban_LAIReadin.o           \
				 MOD_Urban_Shortwave.o           \
				 MOD_Urban_Albedo.o              \
				 MOD_MonthlyinSituCO2MaunaLoa.o  \
				 MOD_PercentagesPFTReadin.o      \
				 MOD_LakeDepthReadin.o           \
				 MOD_DBedrockReadin.o            \
				 MOD_SoilColorRefl.o             \
				 MOD_SoilParametersReadin.o      \
				 MOD_SoilTextureReadin.o         \
				 MOD_HtopReadin.o                \
				 MOD_UrbanReadin.o               \
				 MOD_BGC_CNSummary.o             \
				 MOD_IniTimeVariable.o           \
				 MOD_UrbanIniTimeVariable.o      \
				 MOD_ElementNeighbour.o          \
				 MOD_Catch_HillslopeNetwork.o    \
				 MOD_Catch_RiverLakeNetwork.o    \
				 MOD_Catch_Reservoir.o           \
				 MOD_VicParaReadin.o             \
				 MOD_Initialize.o


$(OBJS_BASIC) : %.o : %.F90 ${HEADER} ${OBJS_SHARED} ${OBJS_MT_CONFIG}
	${FF} -c ${FOPTS} $(INCLUDE_DIR) -o .bld/$@ $< ${MOD_CMD}.bld

OBJS_BASIC_T = $(addprefix .bld/,${OBJS_BASIC})

# MKSRFDATA 与 CoLMINI 都要调 unitcatchment_regional_build（vendor 扩展，
# 上游没有这两个模块）。放在单独变量里，同时挂到两个目标的链接行上：
# 放进 OBJS_MKINIDATA 会让 mksrfdata.x 链接不到，放进 OBJS_BASIC 又编译
# 顺序不对（它们 USE MOD_LandPatch/Mesh/LandElm，要等 BASIC 编完）。
OBJS_UNITCATCHMENT = \
				  MOD_UnitCatchmentSubset.o              \
				  MOD_UnitCatchmentRegional.o

$(OBJS_UNITCATCHMENT) : %.o : %.F90 ${HEADER} ${OBJS_SHARED} ${OBJS_MT_CONFIG} ${OBJS_BASIC}
	${FF} -c ${FOPTS} $(INCLUDE_DIR) -o .bld/$@ $< ${MOD_CMD}.bld

OBJS_UNITCATCHMENT_T = $(addprefix .bld/,${OBJS_UNITCATCHMENT})

OBJS_MKINIDATA = \
				  CoLMINI.o

MOD_UnitCatchmentRegional.o: MOD_UnitCatchmentSubset.o
CoLMINI.o: MOD_UnitCatchmentRegional.o
# MKSRFDATA 也调 unitcatchment_regional_build（vendor 扩展；上游没有这条
# 依赖）。它属于 mkinidata 阶段，后面的自动依赖块只在同一阶段内连边，
# 所以这一条必须手写，否则先编 mksrfdata.x 时找不到 .mod。
MKSRFDATA.o: MOD_UnitCatchmentRegional.o

$(OBJS_MKINIDATA) : %.o : %.F90 ${HEADER} ${OBJS_SHARED} ${OBJS_MT_CONFIG} ${OBJS_BASIC}
	${FF} -c ${FOPTS} $(INCLUDE_DIR) -o .bld/$@ $< ${MOD_CMD}.bld

OBJS_MKINIDATA_T = $(addprefix .bld/,${OBJS_MKINIDATA})

# -------- Target 2: mkinidata -------
mkinidata.x : mkdir_build ${HEADER} ${OBJS_SHARED} ${OBJS_MT_CONFIG} ${OBJS_BASIC} ${OBJS_UNITCATCHMENT} ${OBJS_MKINIDATA}
	@echo ''
	@echo 'making CoLM initial data start >>>>>>>>>>>>>>>>>>>>>>>>>>>>'
	@echo ''
	${FF} ${FOPTS} ${OBJS_SHARED_T} ${OBJS_MT_CONFIG_T} ${OBJS_BASIC_T} ${OBJS_UNITCATCHMENT_T} ${OBJS_MKINIDATA_T} -o run/mkinidata.x ${LDFLAGS}
	@echo ''
	@echo '<<<<<<<<<<<<<<<<<<<<<<<<< making CoLM initial data completed!'
	@echo ''
# ----- End of Target 2 mkinidata ----

CaMa = $(shell \
	CAMA_INIT=$$(grep -E '^\#(define|undef)[[:space:]]+CaMa_Flood' $(HEADER) | head -1 | grep -q 'define' && echo 1 || echo 0); \
	SP=$$(grep -E '^\#(define|undef)[[:space:]]+SinglePoint' $(HEADER) | tail -1 | grep -q 'define' && echo 1 || echo 0); \
	MP=$$(if [ $$SP -eq 1 ]; then echo 0; else grep -E '^\#(define|undef)[[:space:]]+USEMPI' $(HEADER) | head -1 | grep -q 'define' && echo 1 || echo 0; fi); \
	if [ $$CAMA_INIT -eq 0 ]; then echo NO; elif [ $$SP -eq 1 ] || [ $$MP -eq 0 ]; then echo NO; else echo YES; fi)
ifeq (${CaMa},YES)# Compile CoLM decoupled with river routing scheme (CaMa-Flood)
#
CAMA_VARS_OBJ=MOD_CaMa_Vars.o
CAMA_COUPLER_OBJ=MOD_CaMa_colmCaMa.o
CAMA_INTERFACE_OBJS=$(CAMA_VARS_OBJ) $(CAMA_COUPLER_OBJ)
OBJECTS_CAMA=\
				  parkind1.o              \
				  yos_cmf_input.o         \
				  yos_cmf_time.o          \
				  yos_cmf_map.o           \
				  yos_cmf_prog.o          \
				  yos_cmf_diag.o          \
				  cmf_utils_mod.o         \
				  cmf_coupling_budget_mod.o \
				  cmf_calc_outflw_mod.o   \
				  cmf_calc_pthout_mod.o   \
				  cmf_calc_fldstg_mod.o   \
				  cmf_calc_stonxt_mod.o   \
				  cmf_opt_outflw_mod.o    \
				  cmf_ctrl_tracer_mod.o   \
				  cmf_ctrl_mpi_mod.o      \
				  cmf_ctrl_damout_mod.o   \
				  cmf_ctrl_levee_mod.o    \
				  cmf_ctrl_forcing_mod.o  \
				  cmf_ctrl_boundary_mod.o \
				  cmf_ctrl_output_mod.o   \
				  cmf_ctrl_restart_mod.o  \
				  cmf_ctrl_sed_mod.o      \
				  cmf_calc_diag_mod.o     \
				  cmf_ctrl_physics_mod.o  \
				  cmf_ctrl_time_mod.o     \
				  cmf_ctrl_maps_mod.o     \
				  cmf_ctrl_vars_mod.o     \
				  cmf_ctrl_nmlist_mod.o   \
				  cmf_drv_control_mod.o   \
				  cmf_drv_advance_mod.o

cmf_coupling_budget_mod.o: parkind1.o

$(OBJECTS_CAMA) : %.o : %.F90 ${HEADER}
	$(FCMP)  -c ${FFLAGS} $(MODS) ${CFLAGS} $(INCLUDE_DIR) -o .bld/$@ $< ${MOD_CMD}.bld

OBJS_CAMA_T = $(addprefix .bld/,${OBJECTS_CAMA})

else
CAMA_VARS_OBJ=
# Without CaMa, MOD_CaMa_colmCaMa still carries get_fldevp (flood-plain evaporation)
# for GridRiverLakeFlow; MOD_Thermal and CoLMMAIN USE it there. Evaluated with cpp
# because the generated define.h sets GridRiverLakeFlow and then conditionally
# undefines it.
GRIDRIVER_ENABLED := $(shell printf '\043include "include/define.h"\n\043ifdef GridRiverLakeFlow\nYES\n\043else\nNO\n\043endif\n' | cpp -P -I. -Iinclude - | awk '/^(YES|NO)$$/{v=$$0} END{print v}')
ifeq (${GRIDRIVER_ENABLED},YES)
CAMA_COUPLER_OBJ=MOD_CaMa_colmCaMa.o
else
CAMA_COUPLER_OBJ=
endif
CAMA_INTERFACE_OBJS=$(CAMA_VARS_OBJ) $(CAMA_COUPLER_OBJ)
endif

OBJS_MAIN = \
				MOD_Catch_HillslopeFlow.o                 \
				MOD_Catch_SubsurfaceFlow.o                \
				MOD_Catch_RiverLakeFlow.o                 \
				MOD_Catch_Hist.o                          \
				MOD_Catch_WriteParameters.o               \
				MOD_BGC_CNCStateUpdate1.o                 \
				MOD_BGC_CNCStateUpdate2.o                 \
				MOD_BGC_CNCStateUpdate3.o                 \
				MOD_BGC_CNNStateUpdate1.o                 \
				MOD_BGC_CNNStateUpdate2.o                 \
				MOD_BGC_CNNStateUpdate3.o                 \
				MOD_BGC_Soil_BiogeochemNStateUpdate1.o    \
				MOD_BGC_Soil_BiogeochemNitrifDenitrif.o   \
				MOD_BGC_Soil_BiogeochemCompetition.o      \
				MOD_BGC_Soil_BiogeochemDecompCascadeBGC.o \
				MOD_BGC_Soil_BiogeochemDecomp.o           \
				MOD_BGC_Soil_BiogeochemLittVertTransp.o   \
				MOD_BGC_Soil_BiogeochemNLeaching.o        \
				MOD_BGC_Soil_BiogeochemPotential.o        \
				MOD_BGC_Soil_BiogeochemVerticalProfile.o  \
				MOD_BGC_Veg_CNGapMortality.o              \
				MOD_BGC_Veg_CNGResp.o                     \
				MOD_BGC_Veg_CNMResp.o                     \
				MOD_BGC_Daylength.o                       \
				MOD_BGC_Veg_CNPhenology.o                 \
				MOD_BGC_Veg_NutrientCompetition.o         \
				MOD_BGC_Veg_CNVegStructUpdate.o           \
				MOD_BGC_CNAnnualUpdate.o                  \
				MOD_BGC_CNZeroFluxes.o                    \
				MOD_BGC_CNBalanceCheck.o                  \
				MOD_BGC_CNSASU.o                          \
				MOD_BGC_Veg_CNNDynamics.o                 \
				MOD_BGC_Veg_CNFireBase.o                  \
				MOD_BGC_Veg_CNFireLi2016.o                \
				MOD_Vars_2DForcing.o                      \
				MOD_ForcingDownscaling.o                  \
				MOD_Forcing.o                             \
				MOD_DA_TWS.o                              \
				MOD_DA_Const.o                            \
				MOD_DA_RTM.o                              \
				MOD_DA_EnKF.o                             \
				MOD_DA_SM.o                               \
				MOD_DA_Ensemble.o                         \
				MOD_DA_Main.o                             \
				MOD_Opt_Baseflow.o                        \
				MOD_ParameterOptimization.o               \
				MOD_AssimStomataConductance.o             \
				MOD_PlantHydraulic.o                      \
				MOD_FrictionVelocity.o                    \
				MOD_TurbulenceLEddy.o                     \
				MOD_Ozone.o                               \
				MOD_CanopyLayerProfile.o                  \
				MOD_LeafTemperature.o                     \
				MOD_LeafTemperaturePC.o                   \
				MOD_SoilThermalParameters.o               \
				MOD_Hydro_VIC_Variables.o                 \
				MOD_Hydro_VIC.o                           \
				MOD_Runoff.o                              \
				MOD_SoilSnowHydrology.o                   \
				MOD_SnowLayersCombineDivide.o             \
				MOD_PhaseChange.o                         \
				MOD_Glacier.o                             \
				MOD_Lake.o                                \
				MOD_SimpleOcean.o                         \
				MOD_GroundFluxes.o                        \
				MOD_GroundTemperature.o                   \
				MOD_LeafInterception.o                    \
				MOD_NetSolar.o                            \
				MOD_NetSolar_Hyper.o                      \
				MOD_WetBulb.o                             \
				MOD_RainSnowTemp.o                        \
				MOD_SoilSurfaceResistance.o               \
				MOD_NewSnow.o                             \
				$(CAMA_INTERFACE_OBJS)                    \
				MOD_Thermal.o                             \
				MOD_MT_LandPhase.o                        \
				MOD_Vars_1DAccFluxes.o                    \
				MOD_Irrigation.o                          \
				MOD_BGC_driver.o                          \
				MOD_HistWriteBack.o                       \
				MOD_HistGridded.o                         \
				MOD_HistVector.o                          \
				MOD_HistSingle.o                          \
				MOD_MT_Hist.o                             \
				MOD_MT_Methane_VegOverride.o              \
				MOD_MT_Methane_GIEMS.o                    \
				MOD_MT_Methane_pH.o                       \
				MOD_MT_Methane_State.o                    \
				MOD_MT_Methane_Microbes.o                 \
				MOD_MT_Methane_WetlandBGC.o               \
				MOD_MT_Methane_BgcLink.o                  \
				MOD_MT_Methane_AccFlux.o                  \
				MOD_MT_Methane_Physics.o                  \
				MOD_MT_Methane_Driver.o                   \
				MOD_MT_Methane_Hist.o                     \
				MOD_MT_Methane_Impl.o                     \
				MOD_MT_Methane.o                          \
				MT_RegisterProviders.o                    \
				MOD_MT_HostFixes.o                        \
				MOD_MT_SpecialPatches.o                   \
				MOD_Grid_RiverLakeHist.o                  \
				MOD_HistAccRestart.o                      \
				MOD_Hist.o                                \
				MOD_CheckEquilibrium.o                    \
				MOD_LightningData.o                       \
				MOD_Catch_LateralFlow.o                   \
				MOD_MT_RiverLakeCoupling.o                \
				MOD_Grid_RiverLakeFlow.o                  \
				MOD_Urban_Longwave.o                      \
				MOD_Urban_NetSolar.o                      \
				MOD_Urban_Flux.o                          \
				MOD_Urban_GroundFlux.o                    \
				MOD_Urban_RoofFlux.o                      \
				MOD_Urban_RoofTemperature.o               \
				MOD_Urban_WallTemperature.o               \
				MOD_Urban_PerviousTemperature.o           \
				MOD_Urban_ImperviousTemperature.o         \
				MOD_Urban_Hydrology.o                     \
				MOD_Urban_BEM.o                           \
				MOD_Urban_LUCY.o                          \
				MOD_Urban_Thermal.o                       \
				CoLMMAIN_Urban.o                          \
				MOD_Lulcc_Vars_TimeInvariants.o           \
				MOD_Lulcc_Vars_TimeVariables.o            \
				MOD_Lulcc_TransferTraceReadin.o           \
				MOD_Lulcc_MassEnergyConserve.o            \
				MOD_Lulcc_Initialize.o                    \
				MOD_MT_Lulcc.o                            \
				MOD_Lulcc_Driver.o                        \
				CoLMDRIVER.o                              \
				CoLMMAIN.o                                \
				CoLM.o

$(OBJS_MAIN) : %.o : %.F90 ${HEADER} ${OBJS_SHARED} ${OBJS_BASIC}
	${FF} -c ${FOPTS} $(INCLUDE_DIR) -o .bld/$@ $< ${MOD_CMD}.bld

MOD_Urban_Thermal.o: MOD_Urban_Flux.o
MOD_LeafTemperature.o MOD_LeafTemperaturePC.o: MOD_LeafInterception.o
MOD_Grid_RiverLakeNetwork.o: MOD_Grid_RiverLakePush.o
MOD_Grid_RiverLakeBifurcation.o MOD_Grid_RiverLakeHistRoute.o MOD_Grid_RiverLakeTimeVars.o: MOD_Grid_RiverLakeVectorIO.o
MOD_Grid_RiverLakeLevee.o: MOD_Grid_RiverLakeNetwork.o MOD_Vector_ReadWrite.o
MOD_Grid_RiverLakeBifurcation.o: MOD_Grid_Reservoir.o MOD_Grid_RiverLakeLevee.o
MOD_Grid_RiverLakeHistRoute.o: MOD_Grid_RiverLakeHistShard.o MOD_Grid_Reservoir.o
MOD_Grid_RiverLakeTimeVars.o: MOD_Grid_RiverLakeBifurcation.o MOD_Grid_RiverLakeLevee.o
MOD_Vars_TimeVariables.o: MOD_Grid_RiverLakeTimeVars.o
MOD_Grid_RiverLakeFlow.o: MOD_Grid_RiverLakeHist.o
MOD_Grid_ReservoirUpstream.o: MOD_Grid_RiverLakeNetwork.o
MOD_Grid_RiverLakeFlow.o: MOD_Grid_ReservoirUpstream.o
MOD_HistAccRestart.o: MOD_Vars_1DAccFluxes.o MOD_Grid_RiverLakeHistState.o MOD_Grid_RiverLakeVectorIO.o
MOD_Hist.o: MOD_HistAccRestart.o MOD_MT_HostFixes.o
# Material transport (main/MATERIAL); every file is empty unless MaterialTransport is defined.
MOD_MT_Conservation.o: MOD_MT_Defs.o MOD_MT_Frac.o MOD_MT_Namelist.o MOD_MT_Solute.o MOD_MT_Vars.o
MOD_MT_Evapo.o: MOD_MT_Defs.o MOD_MT_EvapLimit.o MOD_MT_Forcing.o MOD_MT_Frac.o MOD_MT_Namelist.o MOD_MT_Solute.o MOD_MT_Vars.o
MOD_MT_Forcing.o: MOD_MT_Defs.o MOD_MT_Namelist.o MOD_MT_ForcingInput.o MOD_MT_Frac.o MOD_MT_Providers.o MOD_MT_Vars.o MOD_UserSpecifiedForcing.o MOD_Vars_1DForcing.o
MOD_MT_ForcingInput.o: MOD_MT_Defs.o
MOD_MT_Frac.o: MOD_MT_Defs.o MOD_MT_Isotope_Registry.o MOD_MT_Namelist.o MOD_MT_Providers.o
MOD_MT_HostFixes.o: MOD_MT_Namelist.o MOD_HistGridded.o
MOD_MT_Hist.o: MOD_MT_Defs.o MOD_MT_Lifecycle.o MOD_MT_Vars.o MOD_HistGridded.o
MOD_MT_Isotope_HDO.o: MOD_MT_Defs.o MOD_MT_Isotope_Registry.o MOD_MT_Namelist.o
MOD_MT_Isotope_O18.o: MOD_MT_Defs.o MOD_MT_Isotope_Registry.o MOD_MT_Namelist.o
MOD_MT_Isotope_Registry.o: MOD_MT_Defs.o
MOD_MT_Isotope_Lulcc.o: MOD_MT_Defs.o MOD_MT_Namelist.o
MOD_MT_Isotope_SoilFlow.o: MOD_MT_Defs.o
MOD_MT_LandPhase.o: MOD_MT_Conservation.o MOD_MT_Defs.o MOD_MT_Evapo.o MOD_MT_Lifecycle.o MOD_MT_Precip.o MOD_MT_Rest.o MOD_MT_Snow.o MOD_MT_SoilInit.o MOD_MT_SoilWater.o MOD_MT_Solute.o MOD_MT_Vars.o
MOD_MT_Lifecycle.o: MOD_MT_Defs.o
MOD_MT_Lulcc.o: MOD_Lulcc_Vars_TimeInvariants.o MOD_Lulcc_Vars_TimeVariables.o MOD_Lulcc_TransferTraceReadin.o MOD_MT_Lifecycle.o MOD_MT_Vars.o MOD_MT_Forcing.o MOD_MT_Conservation.o MOD_Vars_TimeVariables.o MOD_MT_Rest.o MOD_MT_Solute.o MOD_MT_Defs.o
MOD_Lulcc_Driver.o: MOD_MT_Lulcc.o
MOD_MT_Precip.o: MOD_MT_Defs.o MOD_MT_Forcing.o MOD_MT_Solute.o MOD_MT_Vars.o
MOD_MT_Providers.o: MOD_MT_Isotope_HDO.o MOD_MT_Isotope_O18.o
MT_RegisterProviders.o: MOD_MT_Methane.o MOD_MT_Sediment.o
# methane (main/MATERIAL/FAMILY_GAS; empty unless MaterialTransport and BGC)
MOD_MT_Methane_pH.o MOD_MT_Methane_State.o: MOD_MT_NcIO.o
MOD_MT_Methane_Microbes.o: MOD_MT_Methane_State.o MOD_MT_NcIO.o
MOD_MT_Methane_WetlandBGC.o: MOD_MT_NcIO.o MOD_BGC_Soil_BiogeochemDecomp.o \
				MOD_BGC_Soil_BiogeochemDecompCascadeBGC.o MOD_BGC_Soil_BiogeochemPotential.o
MOD_MT_Methane_BgcLink.o: MOD_MT_Methane_pH.o MOD_MT_Methane_VegOverride.o MOD_MT_Methane_WetlandBGC.o
MOD_MT_Methane_AccFlux.o: MOD_MT_Methane_BgcLink.o MOD_MT_Methane_Microbes.o MOD_MT_Methane_State.o
MOD_MT_Methane_Physics.o: MOD_MT_Methane_GIEMS.o MOD_MT_Methane_State.o MOD_MT_Methane_VegOverride.o
MOD_MT_Methane_Driver.o: MOD_MT_Methane_BgcLink.o MOD_MT_Methane_Microbes.o MOD_MT_Methane_Physics.o
MOD_MT_Methane_Hist.o: MOD_MT_Hist.o MOD_HistGridded.o MOD_MT_Methane_AccFlux.o MOD_MT_Methane_BgcLink.o
MOD_MT_Methane_Impl.o: MOD_MT_Methane_BgcLink.o MOD_MT_Methane_Driver.o MOD_MT_Methane_WetlandBGC.o
MOD_MT_Methane.o: MOD_MT_Methane_Hist.o MOD_MT_Methane_Impl.o MOD_MT_Methane_AccFlux.o \
				MOD_MT_Methane_GIEMS.o MOD_MT_Methane_pH.o MOD_MT_Methane_Microbes.o
MOD_MT_RiverLake.o: MOD_MT_Defs.o MOD_MT_Solute.o MOD_MT_Namelist.o MOD_MT_NitrogenN2O.o MOD_Grid_Reservoir.o MOD_Grid_RiverLakeHistRoute.o MOD_Grid_RiverLakeHistState.o MOD_Grid_RiverLakeLevee.o MOD_Grid_RiverLakeNetwork.o MOD_Grid_RiverLakeTimeVars.o
MOD_MT_Sediment.o: MOD_MT_Defs.o MOD_MT_Lifecycle.o MOD_Grid_RiverLakeBifurcation.o MOD_Grid_RiverLakeHistRoute.o MOD_Grid_RiverLakeLevee.o MOD_Grid_RiverLakeNetwork.o MOD_Vector_ReadWrite.o
MOD_MT_RiverLakeCoupling.o: MOD_MT_Defs.o MOD_MT_Lifecycle.o MOD_MT_RiverLake.o MOD_MT_Solute.o MOD_MT_Vars.o MOD_Grid_RiverLakeBifurcation.o
MOD_MT_Rest.o: MOD_MT_Defs.o MOD_MT_Forcing.o MOD_MT_Lifecycle.o MOD_MT_Namelist.o MOD_MT_Solute.o MOD_MT_Vars.o MOD_Vars_TimeInvariants.o
MOD_MT_Solute.o: MOD_MT_Defs.o MOD_MT_Namelist.o
MOD_MT_NitrogenN2O.o: MOD_MT_Defs.o MOD_MT_Namelist.o
MOD_MT_Snow.o: MOD_MT_Defs.o MOD_MT_Solute.o MOD_MT_Vars.o
MOD_MT_SoilInit.o: MOD_MT_Defs.o MOD_MT_Isotope_Registry.o MOD_MT_Namelist.o MOD_MT_Providers.o MOD_MT_Vars.o
MOD_MT_SoilWater.o: MOD_MT_Defs.o MOD_MT_EvapLimit.o MOD_MT_Forcing.o MOD_MT_Frac.o MOD_MT_Namelist.o MOD_MT_Solute.o MOD_MT_Vars.o MOD_MT_Isotope_SoilFlow.o
MOD_MT_NitrogenLeaching.o: MOD_MT_Defs.o MOD_MT_Vars.o MOD_BGC_Vars_1DFluxes.o MOD_Vars_Global.o
MOD_BGC_Soil_BiogeochemNLeaching.o: MOD_MT_Namelist.o
MOD_BGC_CNNStateUpdate3.o: MOD_MT_Namelist.o
MOD_MT_SpecialPatches.o: MOD_MT_Conservation.o MOD_MT_Defs.o MOD_MT_EvapLimit.o MOD_MT_Forcing.o MOD_MT_Frac.o MOD_MT_Hist.o MOD_MT_Namelist.o MOD_MT_Solute.o MOD_MT_Vars.o
MOD_MT_Vars.o: MOD_MT_Defs.o MOD_MT_Namelist.o MOD_MT_Solute.o MOD_MT_Isotope_Lulcc.o
MOD_Vars_TimeVariables.o: MOD_MT_Rest.o MOD_MT_RiverLake.o
MOD_Grid_RiverLakeFlow.o: MOD_MT_RiverLakeCoupling.o
MOD_Grid_RiverLakeHist.o: MOD_MT_RiverLake.o
MOD_UserSpecifiedForcing.o: MOD_Qsadv.o

OBJS_MAIN_T = $(addprefix .bld/,${OBJS_MAIN})

# ------ Target 3: main --------

ifneq (${CaMa},YES)# Compile CoLM decoupled without river routing scheme (CaMa-Flood)

colm.x : mkdir_build ${HEADER} ${OBJS_SHARED} ${OBJS_MT_CONFIG} ${OBJS_BASIC} ${OBJS_MAIN}
	@echo ''
	@echo 'making CoLM start >>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>'
	@echo ''
	${FF} ${FOPTS} ${OBJS_SHARED_T} ${OBJS_MT_CONFIG_T} ${OBJS_BASIC_T} ${OBJS_MAIN_T} -o run/colm.x ${LDFLAGS}
	@echo ''
	@echo '<<<<<<<<<<<<<<<<<<<<<<<<<<<<< making CoLM completed!'
	@echo ''

else
colm.x : mkdir_build  ${HEADER} ${OBJS_SHARED} ${OBJS_MT_CONFIG} ${OBJECTS_CAMA} ${OBJS_BASIC} ${OBJS_MAIN}
	@echo ''
	@echo 'making CoLM with CaMa start >>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>'
	@echo ''
	${FF} ${FOPTS} ${OBJS_SHARED_T} ${OBJS_MT_CONFIG_T} ${OBJS_BASIC_T} ${OBJS_CAMA_T} ${OBJS_MAIN_T} -o run/colm.x ${LDFLAGS}

	@echo ''
	@echo '<<<<<<<<<<<<<<<<<<<<<<<<<<<< making CoLM with CaMa completed!'
	@echo ''

endif

# ----- End of Target 3 main -----

OBJS_POST1 = MOD_Concatenate.o HistConcatenate.o
OBJS_POST2 = MOD_Vector2Grid.o POST_Vector2Grid.o
OBJS_POST3 = SrfDataConcatenate.o
OBJS_POST4 = RiverHistConcatenate.o
OBJS_POST1_T = $(addprefix .bld/,${OBJS_POST1})
OBJS_POST2_T = $(addprefix .bld/,${OBJS_POST2})
OBJS_POST3_T = $(addprefix .bld/,${OBJS_POST3})
OBJS_POST4_T = $(addprefix .bld/,${OBJS_POST4})

$(OBJS_POST1):%.o:%.F90 ${HEADER}
	${FF} -c ${FOPTS} $(INCLUDE_DIR) -o .bld/$@ $< ${MOD_CMD}.bld

$(OBJS_POST2):%.o:%.F90 ${HEADER}
	${FF} -c ${FOPTS} $(INCLUDE_DIR) -o .bld/$@ $< ${MOD_CMD}.bld

$(OBJS_POST3):%.o:%.F90 ${HEADER}
	${FF} -c ${FOPTS} $(INCLUDE_DIR) -o .bld/$@ $< ${MOD_CMD}.bld

$(OBJS_POST4):%.o:%.F90 ${HEADER} ${OBJS_SHARED}
	${FF} -c ${FOPTS} $(INCLUDE_DIR) -o .bld/$@ $< ${MOD_CMD}.bld

hist_concatenate.x : ${HEADER} ${OBJS_SHARED} ${OBJS_POST1}
	${FF} ${FOPTS} ${OBJS_SHARED_T} ${OBJS_POST1_T} -o run/$@ ${LDFLAGS}

post_vector2grid.x : ${HEADER} ${OBJS_SHARED} ${OBJS_POST2}
	${FF} ${FOPTS} ${OBJS_SHARED_T} ${OBJS_POST2_T} -o run/$@ ${LDFLAGS}

srfdata_concatenate.x : ${HEADER} ${OBJS_SHARED} ${OBJS_POST3}
	${FF} ${FOPTS} ${OBJS_SHARED_T} ${OBJS_POST3_T} -o run/$@ ${LDFLAGS}

river_hist_concatenate.x : ${HEADER} ${OBJS_SHARED} ${OBJS_POST4}
	${FF} ${FOPTS} ${OBJS_SHARED_T} ${OBJS_POST4_T} -o run/$@ ${LDFLAGS}

# ------ Target 4: postprocess --------
DEF = $(shell grep -i CATCHMENT include/define.h)
vector2grid = $(word 1, ${DEF})
ifneq (${vector2grid},\#define)
DEF = $(shell grep -i UNSTRUCTURED include/define.h)
vector2grid = $(word 1, ${DEF})
endif

.PHONY: postprocess.x
ifneq (${vector2grid},\#define)
postprocess.x : mkdir_build hist_concatenate.x river_hist_concatenate.x srfdata_concatenate.x
	@echo '<<<<<<<<<<<<<<<<<<<<<<<<< making CoLM postprocessing completed!'
else
postprocess.x : mkdir_build hist_concatenate.x river_hist_concatenate.x srfdata_concatenate.x post_vector2grid.x
	@echo '<<<<<<<<<<<<<<<<<<<<<<<<< making CoLM postprocessing completed!'
endif
# --- End of Target 4 postprocess ------

# ------ Target 5: static libs --------
.PHONY: lib
lib :
	@echo ''
	@echo 'making CoLM static library >>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>'
	mkdir -p lib
	cd lib && find ../.bld -name "*.o" ! -name "CoLM.o" ! -name "MKSRFDATA.o" ! -name "CoLMINI.o" -exec ln -sf {} ./ \;
	cd lib && ar rc libcolm.a *.o && ranlib libcolm.a
	ln -sf lib/libcolm.a ./libcolm.a
# ------End of Target 5: static libs --------

.PHONY: clean
clean :
	rm -rf .bld
	rm -rf lib libcolm.a
	rm -f run/mksrfdata.x run/mkinidata.x run/colm.x
	rm -f run/hist_concatenate.x run/river_hist_concatenate.x run/srfdata_concatenate.x run/post_vector2grid.x
	rm -f CaMa/src/*.o CaMa/src/*.mod CaMa/src/*.a

# Desktop-local ordering edges (MOD_Filesystem is USEd by MOD_Namelist; the
# FLAT_SPMD paths USE MOD_Vector_ReadWrite and MOD_Forcing).
MOD_Filesystem.o: MOD_SPMD_Task.o
MOD_Namelist.o: MOD_Filesystem.o
MOD_Utils.o: MOD_SPMD_Task.o
CoLMDRIVER.o: MOD_Forcing.o
MOD_Grid_RiverLakeNetwork.o: MOD_Vector_ReadWrite.o

# >>> generated by oracle/scripts/gen_fortran_deps.py -- do not edit by hand >>>
# Same-stage MODULE/USE prerequisites (cross-stage ordering comes from the pattern
# rules above).  Regenerate after every upstream sync.
$(OBJS_SHARED): | mkdir_build
$(OBJS_MT_CONFIG): | mkdir_build
$(OBJS_MKSRFDATA): | mkdir_build
$(OBJS_BASIC): | mkdir_build
$(OBJS_UNITCATCHMENT): | mkdir_build
$(OBJS_MKINIDATA): | mkdir_build
$(OBJECTS_CAMA): | mkdir_build
$(OBJS_MAIN): | mkdir_build
$(OBJS_POST1): | mkdir_build
$(OBJS_POST2): | mkdir_build
$(OBJS_POST3): | mkdir_build
$(OBJS_POST4): | mkdir_build
CoLM.o: MOD_CaMa_colmCaMa.o MOD_Catch_LateralFlow.o MOD_CheckEquilibrium.o MOD_DA_Main.o MOD_Forcing.o MOD_Grid_RiverLakeFlow.o MOD_Hist.o MOD_HistAccRestart.o MOD_HistWriteBack.o MOD_LightningData.o MOD_Lulcc_Driver.o MOD_MT_HostFixes.o MOD_MT_LandPhase.o MOD_Ozone.o MOD_ParameterOptimization.o MOD_Vars_1DAccFluxes.o MOD_Vars_2DForcing.o
CoLMDRIVER.o: MOD_AssimStomataConductance.o MOD_Forcing.o MOD_Grid_RiverLakeFlow.o MOD_MT_LandPhase.o MOD_MT_SpecialPatches.o
CoLMMAIN.o: MOD_CaMa_colmCaMa.o MOD_Glacier.o MOD_Grid_RiverLakeFlow.o MOD_Irrigation.o MOD_Lake.o MOD_LeafInterception.o MOD_MT_Hist.o MOD_MT_LandPhase.o MOD_MT_RiverLakeCoupling.o MOD_MT_SpecialPatches.o MOD_NetSolar.o MOD_NetSolar_Hyper.o MOD_NewSnow.o MOD_RainSnowTemp.o MOD_SimpleOcean.o MOD_SnowLayersCombineDivide.o MOD_SoilSnowHydrology.o MOD_Thermal.o
CoLMMAIN_Urban.o: MOD_Lake.o MOD_LeafInterception.o MOD_NewSnow.o MOD_RainSnowTemp.o MOD_SnowLayersCombineDivide.o MOD_Urban_Hydrology.o MOD_Urban_LUCY.o MOD_Urban_NetSolar.o MOD_Urban_Thermal.o
HistConcatenate.o: MOD_Concatenate.o
MKSRFDATA.o: MOD_Lulcc_TransferTrace.o
MOD_3DCanopyRadiation.o: MOD_Vars_TimeInvariants.o MOD_Vars_TimeVariables.o
MOD_5x5DataReadin.o: MOD_Block.o MOD_DataType.o MOD_Grid.o MOD_NetCDFSerial.o MOD_Precision.o MOD_SPMD_Task.o
MOD_Aerosol.o: MOD_Vars_1DForcing.o
MOD_AggregationRequestData.o: MOD_Block.o MOD_DataType.o MOD_Grid.o MOD_Mesh.o MOD_Pixel.o MOD_Pixelset.o MOD_Precision.o MOD_SPMD_Task.o MOD_UserDefFun.o MOD_Utils.o
MOD_Albedo.o: MOD_3DCanopyRadiation.o MOD_Aerosol.o MOD_SnowSnicar.o MOD_Vars_TimeInvariants.o MOD_Vars_TimeVariables.o
MOD_Albedo_HiRes.o: MOD_3DCanopyRadiation.o MOD_Aerosol.o MOD_HighRes_Parameters.o MOD_SnowSnicar_HiRes.o MOD_Vars_TimeInvariants.o MOD_Vars_TimeVariables.o
MOD_BGC_CNSummary.o: MOD_BGC_Vars_1DFluxes.o MOD_BGC_Vars_1DPFTFluxes.o MOD_BGC_Vars_PFTimeVariables.o MOD_BGC_Vars_TimeInvariants.o MOD_BGC_Vars_TimeVariables.o MOD_Vars_TimeInvariants.o MOD_Vars_TimeVariables.o
MOD_BGC_Veg_CNPhenology.o: MOD_BGC_Daylength.o
MOD_BGC_driver.o: MOD_BGC_CNAnnualUpdate.o MOD_BGC_CNBalanceCheck.o MOD_BGC_CNCStateUpdate1.o MOD_BGC_CNCStateUpdate2.o MOD_BGC_CNCStateUpdate3.o MOD_BGC_CNNStateUpdate1.o MOD_BGC_CNNStateUpdate2.o MOD_BGC_CNNStateUpdate3.o MOD_BGC_CNSASU.o MOD_BGC_CNZeroFluxes.o MOD_BGC_Soil_BiogeochemCompetition.o MOD_BGC_Soil_BiogeochemDecomp.o MOD_BGC_Soil_BiogeochemDecompCascadeBGC.o MOD_BGC_Soil_BiogeochemLittVertTransp.o MOD_BGC_Soil_BiogeochemNLeaching.o MOD_BGC_Soil_BiogeochemNStateUpdate1.o MOD_BGC_Soil_BiogeochemNitrifDenitrif.o MOD_BGC_Soil_BiogeochemPotential.o MOD_BGC_Soil_BiogeochemVerticalProfile.o MOD_BGC_Veg_CNFireBase.o MOD_BGC_Veg_CNFireLi2016.o MOD_BGC_Veg_CNGResp.o MOD_BGC_Veg_CNGapMortality.o MOD_BGC_Veg_CNMResp.o MOD_BGC_Veg_CNNDynamics.o MOD_BGC_Veg_CNPhenology.o MOD_BGC_Veg_CNVegStructUpdate.o MOD_BGC_Veg_NutrientCompetition.o MOD_Irrigation.o
MOD_Block.o: MOD_Namelist.o MOD_NetCDFSerial.o MOD_Precision.o MOD_SPMD_Task.o MOD_Utils.o
MOD_CaMa_colmCaMa.o: MOD_FrictionVelocity.o MOD_TurbulenceLEddy.o
MOD_CanopyLayerProfile.o: MOD_FrictionVelocity.o
MOD_Catch_LateralFlow.o: MOD_Catch_HillslopeFlow.o MOD_Catch_RiverLakeFlow.o MOD_Catch_SubsurfaceFlow.o MOD_Catch_WriteParameters.o MOD_Lake.o
MOD_Catch_Reservoir.o: MOD_Catch_BasinNetwork.o MOD_Catch_RiverLakeNetwork.o
MOD_Catch_RiverLakeNetwork.o: MOD_Catch_BasinNetwork.o MOD_Catch_HillslopeNetwork.o MOD_Catch_Vars_TimeVariables.o MOD_ElementNeighbour.o MOD_Vars_TimeInvariants.o MOD_Vector_ReadWrite.o
MOD_Catch_Vars_1DFluxes.o: MOD_Catch_BasinNetwork.o
MOD_Catch_Vars_TimeVariables.o: MOD_Catch_BasinNetwork.o MOD_Vector_ReadWrite.o
MOD_Catch_WriteParameters.o: MOD_Catch_SubsurfaceFlow.o
MOD_CatchmentDataReadin.o: MOD_Block.o MOD_DataType.o MOD_Grid.o MOD_NetCDFSerial.o MOD_SPMD_Task.o MOD_Utils.o
MOD_CheckEquilibrium.o: MOD_Forcing.o
MOD_Const_LC.o: MOD_Namelist.o MOD_Precision.o MOD_Vars_Global.o
MOD_Const_PFT.o: MOD_Namelist.o MOD_Precision.o MOD_TimeManager.o MOD_Vars_Global.o
MOD_Const_Physical.o: MOD_Precision.o
MOD_CropReadin.o: MOD_Vars_TimeInvariants.o MOD_Vars_TimeVariables.o
MOD_DA_Main.o: MOD_DA_Ensemble.o MOD_DA_SM.o MOD_DA_TWS.o
MOD_DA_RTM.o: MOD_DA_Const.o
MOD_DA_SM.o: MOD_DA_EnKF.o MOD_DA_RTM.o
MOD_DA_TWS.o: MOD_Forcing.o
MOD_DBedrockReadin.o: MOD_Vars_TimeInvariants.o
MOD_DataType.o: MOD_Block.o MOD_Grid.o MOD_Precision.o MOD_SPMD_Task.o MOD_Vars_Global.o
MOD_ElmVector.o: MOD_DataType.o MOD_LandCrop.o MOD_LandElm.o MOD_LandPatch.o MOD_Mesh.o MOD_Pixelset.o MOD_Precision.o MOD_SPMD_Task.o MOD_UserDefFun.o MOD_Utils.o
MOD_Eroot.o: MOD_Hydro_SoilFunction.o
MOD_Filesystem.o: MOD_SPMD_Task.o
MOD_FireData.o: MOD_Vars_TimeInvariants.o MOD_Vars_TimeVariables.o
MOD_Forcing.o: MOD_ForcingDownscaling.o MOD_Vars_2DForcing.o
MOD_Glacier.o: MOD_FrictionVelocity.o MOD_PhaseChange.o MOD_SnowLayersCombineDivide.o MOD_SoilSnowHydrology.o MOD_TurbulenceLEddy.o
MOD_Grid.o: MOD_Block.o MOD_Namelist.o MOD_NetCDFSerial.o MOD_Precision.o MOD_SPMD_Task.o MOD_UserDefFun.o MOD_Utils.o MOD_Vars_Global.o
MOD_Grid_Reservoir.o: MOD_Grid_RiverLakeNetwork.o
MOD_Grid_ReservoirUpstream.o: MOD_Grid_RiverLakeNetwork.o
MOD_Grid_RiverLakeBifurcation.o: MOD_Grid_Reservoir.o MOD_Grid_RiverLakeLevee.o MOD_Grid_RiverLakeNetwork.o MOD_Grid_RiverLakePush.o MOD_Grid_RiverLakeVectorIO.o
MOD_Grid_RiverLakeFlow.o: MOD_Forcing.o MOD_Grid_RiverLakeHist.o MOD_MT_RiverLakeCoupling.o
MOD_Grid_RiverLakeHist.o: MOD_Forcing.o MOD_HistGridded.o MOD_HistVector.o
MOD_Grid_RiverLakeHistRoute.o: MOD_Grid_Reservoir.o MOD_Grid_RiverLakeHistShard.o MOD_Grid_RiverLakeNetwork.o MOD_Grid_RiverLakeVectorIO.o MOD_Vector_ReadWrite.o
MOD_Grid_RiverLakeLevee.o: MOD_Grid_RiverLakeNetwork.o MOD_Vector_ReadWrite.o
MOD_Grid_RiverLakeNetwork.o: MOD_Grid_RiverLakePush.o MOD_Grid_RiverLakeVectorIO.o MOD_Vector_ReadWrite.o
MOD_Grid_RiverLakeTimeVars.o: MOD_Grid_Reservoir.o MOD_Grid_RiverLakeBifurcation.o MOD_Grid_RiverLakeLevee.o MOD_Grid_RiverLakeNetwork.o MOD_Grid_RiverLakeVectorIO.o MOD_Vector_ReadWrite.o
MOD_GroundFluxes.o: MOD_FrictionVelocity.o MOD_TurbulenceLEddy.o
MOD_GroundTemperature.o: MOD_PhaseChange.o MOD_SoilThermalParameters.o
MOD_HRUVector.o: MOD_DataType.o MOD_ElmVector.o MOD_LandCrop.o MOD_LandElm.o MOD_LandHRU.o MOD_LandPatch.o MOD_Mesh.o MOD_Precision.o MOD_SPMD_Task.o MOD_Utils.o
MOD_HighRes_Parameters.o: MOD_OrbCoszen.o MOD_dataSpec_PDB.o MOD_prospect_DB.o
MOD_Hist.o: MOD_Catch_Hist.o MOD_DA_TWS.o MOD_Forcing.o MOD_Grid_RiverLakeHist.o MOD_HistAccRestart.o MOD_HistGridded.o MOD_HistSingle.o MOD_HistVector.o MOD_MT_Hist.o MOD_MT_HostFixes.o MOD_Vars_1DAccFluxes.o
MOD_HistAccRestart.o: MOD_Vars_1DAccFluxes.o
MOD_HistGridded.o: MOD_Forcing.o MOD_HistWriteBack.o MOD_Vars_1DAccFluxes.o
MOD_HistSingle.o: MOD_Vars_1DAccFluxes.o
MOD_HistVector.o: MOD_Vars_1DAccFluxes.o
MOD_HtopReadin.o: MOD_Vars_TimeInvariants.o MOD_Vars_TimeVariables.o
MOD_Hydro_SoilWater.o: MOD_Hydro_SoilFunction.o
MOD_Hydro_VIC.o: MOD_Hydro_VIC_Variables.o
MOD_IniTimeVariable.o: MOD_Albedo.o MOD_Albedo_HiRes.o MOD_BGC_CNSummary.o MOD_Hydro_SoilWater.o MOD_SnowFraction.o MOD_Vars_TimeInvariants.o MOD_Vars_TimeVariables.o
MOD_Initialize.o: MOD_BGC_CNSummary.o MOD_Catch_BasinNetwork.o MOD_Catch_RiverLakeNetwork.o MOD_CropReadin.o MOD_DA_Vars_TimeVariables.o MOD_DBedrockReadin.o MOD_ElementNeighbour.o MOD_Grid_Reservoir.o MOD_Grid_RiverLakeLevee.o MOD_Grid_RiverLakeNetwork.o MOD_HighRes_Parameters.o MOD_HtopReadin.o MOD_Hydro_SoilFunction.o MOD_IniTimeVariable.o MOD_LAIEmpirical.o MOD_LAIReadin.o MOD_LakeDepthReadin.o MOD_OrbCoszen.o MOD_PercentagesPFTReadin.o MOD_SoilParametersReadin.o MOD_SoilTextureReadin.o MOD_UrbanIniTimeVariable.o MOD_UrbanReadin.o MOD_Urban_Albedo.o MOD_Urban_LAIReadin.o MOD_Vars_TimeInvariants.o MOD_Vars_TimeVariables.o MOD_VicParaReadin.o
MOD_LAIReadin.o: MOD_Vars_TimeInvariants.o MOD_Vars_TimeVariables.o
MOD_Lake.o: MOD_FrictionVelocity.o MOD_SnowLayersCombineDivide.o MOD_SoilSnowHydrology.o MOD_SoilThermalParameters.o MOD_TurbulenceLEddy.o
MOD_LakeDepthReadin.o: MOD_Vars_TimeInvariants.o
MOD_Land2mWMO.o: MOD_Const_LC.o MOD_DataType.o MOD_Grid.o MOD_LandElm.o MOD_LandPatch.o MOD_Mesh.o MOD_Namelist.o MOD_NetCDFBlock.o MOD_Pixelset.o MOD_Precision.o MOD_SPMD_Task.o MOD_UserDefFun.o MOD_Utils.o MOD_Vars_Global.o
MOD_LandCrop.o: MOD_5x5DataReadin.o MOD_Block.o MOD_DataType.o MOD_Grid.o MOD_LandElm.o MOD_LandHRU.o MOD_LandPatch.o MOD_Namelist.o MOD_NetCDFBlock.o MOD_PixelsetShared.o MOD_Precision.o MOD_SPMD_Task.o
MOD_LandElm.o: MOD_Mesh.o MOD_Pixelset.o MOD_Precision.o MOD_SPMD_Task.o
MOD_LandHRU.o: MOD_AggregationRequestData.o MOD_Block.o MOD_CatchmentDataReadin.o MOD_DataType.o MOD_Grid.o MOD_LandElm.o MOD_Mesh.o MOD_Namelist.o MOD_NetCDFSerial.o MOD_Pixelset.o MOD_Precision.o MOD_SPMD_Task.o MOD_Utils.o
MOD_LandPFT.o: MOD_5x5DataReadin.o MOD_AggregationRequestData.o MOD_Const_LC.o MOD_DataType.o MOD_Grid.o MOD_Land2mWMO.o MOD_LandCrop.o MOD_LandPatch.o MOD_Namelist.o MOD_Pixelset.o MOD_Precision.o MOD_SPMD_Task.o MOD_Vars_Global.o
MOD_LandPatch.o: MOD_5x5DataReadin.o MOD_AggregationRequestData.o MOD_Const_LC.o MOD_DataType.o MOD_Filesystem.o MOD_Grid.o MOD_LandElm.o MOD_LandHRU.o MOD_Mesh.o MOD_Namelist.o MOD_NetCDFBlock.o MOD_NetCDFVector.o MOD_Pixelset.o MOD_Precision.o MOD_SPMD_Task.o MOD_UserDefFun.o MOD_Utils.o MOD_Vars_Global.o
MOD_LandUrban.o: MOD_5x5DataReadin.o MOD_AggregationRequestData.o MOD_DataType.o MOD_Grid.o MOD_LandElm.o MOD_LandHRU.o MOD_LandPatch.o MOD_Mesh.o MOD_Namelist.o MOD_NetCDFBlock.o MOD_Pixelset.o MOD_Precision.o MOD_SPMD_Task.o MOD_Utils.o MOD_Vars_Global.o
MOD_LeafTemperature.o: MOD_AssimStomataConductance.o MOD_CanopyLayerProfile.o MOD_FrictionVelocity.o MOD_LeafInterception.o MOD_Ozone.o MOD_PlantHydraulic.o MOD_TurbulenceLEddy.o
MOD_LeafTemperaturePC.o: MOD_AssimStomataConductance.o MOD_CanopyLayerProfile.o MOD_FrictionVelocity.o MOD_LeafInterception.o MOD_Ozone.o MOD_PlantHydraulic.o MOD_TurbulenceLEddy.o
MOD_Lulcc_Driver.o: MOD_Lulcc_Initialize.o MOD_Lulcc_MassEnergyConserve.o MOD_Lulcc_TransferTraceReadin.o MOD_Lulcc_Vars_TimeInvariants.o MOD_Lulcc_Vars_TimeVariables.o MOD_MT_Lulcc.o MOD_Opt_Baseflow.o
MOD_Lulcc_Initialize.o: MOD_Lulcc_Vars_TimeInvariants.o MOD_Lulcc_Vars_TimeVariables.o
MOD_Lulcc_MassEnergyConserve.o: MOD_GroundTemperature.o MOD_Lulcc_TransferTraceReadin.o MOD_Lulcc_Vars_TimeInvariants.o MOD_Lulcc_Vars_TimeVariables.o
MOD_Lulcc_Vars_TimeVariables.o: MOD_Lulcc_Vars_TimeInvariants.o MOD_Opt_Baseflow.o
MOD_MT_Conservation.o: MOD_MT_Frac.o MOD_MT_Solute.o MOD_MT_Vars.o
MOD_MT_Defs.o: MOD_MT_Namelist.o
MOD_MT_Evapo.o: MOD_MT_EvapLimit.o MOD_MT_Forcing.o MOD_MT_Frac.o MOD_MT_Solute.o MOD_MT_Vars.o
MOD_MT_Forcing.o: MOD_MT_ForcingInput.o MOD_MT_Frac.o MOD_MT_Providers.o MOD_MT_Vars.o MOD_UserSpecifiedForcing.o MOD_Vars_1DForcing.o
MOD_MT_Frac.o: MOD_MT_Isotope_Registry.o MOD_MT_Providers.o
MOD_MT_Hist.o: MOD_HistGridded.o MOD_HistSingle.o MOD_HistVector.o MOD_Vars_1DAccFluxes.o
MOD_MT_HostFixes.o: MOD_HistGridded.o
MOD_MT_Isotope_HDO.o: MOD_MT_Isotope_Registry.o
MOD_MT_Isotope_O18.o: MOD_MT_Isotope_Registry.o
MOD_MT_Lulcc.o: MOD_Lulcc_TransferTraceReadin.o MOD_Lulcc_Vars_TimeInvariants.o MOD_Lulcc_Vars_TimeVariables.o
MOD_MT_Methane.o: MOD_MT_Methane_AccFlux.o MOD_MT_Methane_GIEMS.o MOD_MT_Methane_Hist.o MOD_MT_Methane_Impl.o MOD_MT_Methane_Microbes.o MOD_MT_Methane_Physics.o MOD_MT_Methane_State.o MOD_MT_Methane_VegOverride.o MOD_MT_Methane_WetlandBGC.o MOD_MT_Methane_pH.o
MOD_MT_Methane_AccFlux.o: MOD_MT_Methane_BgcLink.o MOD_MT_Methane_Microbes.o MOD_MT_Methane_State.o
MOD_MT_Methane_BgcLink.o: MOD_MT_Methane_VegOverride.o MOD_MT_Methane_WetlandBGC.o MOD_MT_Methane_pH.o
MOD_MT_Methane_Const.o: MOD_MT_Defs.o
MOD_MT_Methane_Driver.o: MOD_MT_Methane_BgcLink.o MOD_MT_Methane_Microbes.o MOD_MT_Methane_Physics.o MOD_MT_Methane_State.o MOD_MT_Methane_VegOverride.o
MOD_MT_Methane_Hist.o: MOD_HistGridded.o MOD_MT_Hist.o MOD_MT_Methane_AccFlux.o MOD_MT_Methane_BgcLink.o
MOD_MT_Methane_Impl.o: MOD_MT_Methane_BgcLink.o MOD_MT_Methane_Driver.o MOD_MT_Methane_Microbes.o MOD_MT_Methane_State.o MOD_MT_Methane_WetlandBGC.o
MOD_MT_Methane_Microbes.o: MOD_MT_Methane_State.o
MOD_MT_Methane_Physics.o: MOD_MT_Methane_GIEMS.o MOD_MT_Methane_State.o MOD_MT_Methane_VegOverride.o
MOD_MT_Methane_Preprocessing.o: MOD_MT_Defs.o MOD_MT_Methane_Const.o MOD_MT_Methane_Registry.o
MOD_MT_Methane_WetlandBGC.o: MOD_BGC_Soil_BiogeochemDecomp.o MOD_BGC_Soil_BiogeochemDecompCascadeBGC.o MOD_BGC_Soil_BiogeochemPotential.o
MOD_MT_NitrogenLeaching.o: MOD_BGC_Vars_1DFluxes.o MOD_MT_Vars.o
MOD_MT_Precip.o: MOD_MT_Forcing.o MOD_MT_Solute.o MOD_MT_Vars.o
MOD_MT_Providers.o: MOD_MT_Isotope_HDO.o MOD_MT_Isotope_O18.o
MOD_MT_Rest.o: MOD_MT_Forcing.o MOD_MT_Lifecycle.o MOD_MT_Solute.o MOD_MT_Vars.o MOD_Vars_TimeInvariants.o
MOD_MT_RiverLake.o: MOD_Grid_Reservoir.o MOD_Grid_RiverLakeHistRoute.o MOD_Grid_RiverLakeHistState.o MOD_Grid_RiverLakeLevee.o MOD_Grid_RiverLakeNetwork.o MOD_Grid_RiverLakeTimeVars.o MOD_MT_NitrogenN2O.o MOD_MT_Solute.o MOD_Vector_ReadWrite.o
MOD_MT_Sediment.o: MOD_Grid_RiverLakeBifurcation.o MOD_Grid_RiverLakeHistRoute.o MOD_Grid_RiverLakeLevee.o MOD_Grid_RiverLakeNetwork.o MOD_MT_Lifecycle.o MOD_Vector_ReadWrite.o
MOD_MT_Snow.o: MOD_MT_Solute.o MOD_MT_Vars.o
MOD_MT_SoilInit.o: MOD_MT_Isotope_Registry.o MOD_MT_Providers.o MOD_MT_Vars.o
MOD_MT_SoilWater.o: MOD_MT_EvapLimit.o MOD_MT_Forcing.o MOD_MT_Frac.o MOD_MT_Isotope_SoilFlow.o MOD_MT_Solute.o MOD_MT_Vars.o
MOD_MT_SpecialPatches.o: MOD_MT_Hist.o
MOD_MT_Vars.o: MOD_MT_Isotope_Lulcc.o MOD_MT_Solute.o
MOD_Mesh.o: MOD_Block.o MOD_CatchmentDataReadin.o MOD_DataType.o MOD_Grid.o MOD_Namelist.o MOD_NetCDFBlock.o MOD_Pixel.o MOD_Precision.o MOD_SPMD_Task.o MOD_Utils.o
MOD_MeshFilter.o: MOD_5x5DataReadin.o MOD_AggregationRequestData.o MOD_Block.o MOD_DataType.o MOD_Grid.o MOD_LandElm.o MOD_Mesh.o MOD_Namelist.o MOD_NetCDFBlock.o MOD_Precision.o MOD_SPMD_Task.o
MOD_Namelist.o: MOD_Filesystem.o MOD_Precision.o MOD_SPMD_Task.o
MOD_NdepData.o: MOD_BGC_Vars_1DFluxes.o MOD_BGC_Vars_TimeVariables.o MOD_Vars_TimeInvariants.o
MOD_NetCDFBlock.o: MOD_Block.o MOD_DataType.o MOD_Grid.o MOD_Namelist.o MOD_NetCDFSerial.o MOD_SPMD_Task.o
MOD_NetCDFPoint.o: MOD_Grid.o MOD_NetCDFSerial.o MOD_Precision.o MOD_Utils.o
MOD_NetCDFSerial.o: MOD_Precision.o MOD_SPMD_Task.o MOD_TimeManager.o MOD_Vars_Global.o
MOD_NetCDFVector.o: MOD_Block.o MOD_DataType.o MOD_NetCDFSerial.o MOD_Pixelset.o MOD_Precision.o MOD_SPMD_Task.o
MOD_NitrifData.o: MOD_BGC_Vars_TimeVariables.o MOD_Vars_TimeInvariants.o
MOD_ParameterOptimization.o: MOD_Opt_Baseflow.o
MOD_PercentagesPFTReadin.o: MOD_Vars_TimeInvariants.o
MOD_Pixel.o: MOD_Block.o MOD_Grid.o MOD_NetCDFSerial.o MOD_Precision.o MOD_SPMD_Task.o MOD_Utils.o
MOD_Pixelset.o: MOD_Block.o MOD_DataType.o MOD_Mesh.o MOD_Pixel.o MOD_Precision.o MOD_SPMD_Task.o MOD_Utils.o MOD_Vars_Global.o
MOD_PixelsetShared.o: MOD_AggregationRequestData.o MOD_DataType.o MOD_Grid.o MOD_Mesh.o MOD_Pixel.o MOD_Pixelset.o MOD_SPMD_Task.o MOD_Utils.o
MOD_RainSnowTemp.o: MOD_WetBulb.o
MOD_RangeCheck.o: MOD_Block.o MOD_DataType.o MOD_Namelist.o MOD_Precision.o MOD_SPMD_Task.o MOD_UserDefFun.o MOD_Vars_Global.o
MOD_RegionClip.o: MOD_Block.o MOD_Const_LC.o MOD_Filesystem.o MOD_Namelist.o MOD_NetCDFSerial.o MOD_Pixel.o MOD_SPMD_Task.o MOD_Utils.o
MOD_SPMD_Task.o: MOD_Precision.o
MOD_SimpleOcean.o: MOD_FrictionVelocity.o
MOD_SingleSrfdata.o: MOD_Block.o MOD_Const_LC.o MOD_Const_PFT.o MOD_Grid.o MOD_LandElm.o MOD_LandPFT.o MOD_LandPatch.o MOD_LandUrban.o MOD_Mesh.o MOD_Namelist.o MOD_NetCDFPoint.o MOD_NetCDFSerial.o MOD_Precision.o MOD_SPMD_Task.o MOD_TimeManager.o MOD_Urban_Const_LCZ.o MOD_Utils.o MOD_Vars_Global.o
MOD_SnowFraction.o: MOD_Vars_TimeInvariants.o MOD_Vars_TimeVariables.o
MOD_SnowSnicar.o: MOD_Aerosol.o
MOD_SnowSnicar_HiRes.o: MOD_Aerosol.o
MOD_SoilParametersReadin.o: MOD_SoilColorRefl.o MOD_Vars_TimeInvariants.o
MOD_SoilSnowHydrology.o: MOD_DA_TWS.o MOD_Hydro_VIC.o MOD_Hydro_VIC_Variables.o MOD_Opt_Baseflow.o MOD_Runoff.o
MOD_SoilTextureReadin.o: MOD_Vars_TimeInvariants.o
MOD_SpatialMapping.o: MOD_Block.o MOD_DataType.o MOD_Grid.o MOD_Mesh.o MOD_Namelist.o MOD_Pixel.o MOD_Pixelset.o MOD_Precision.o MOD_SPMD_Task.o MOD_Utils.o MOD_Vars_Global.o
MOD_SrfdataDiag.o: MOD_Block.o MOD_DataType.o MOD_Filesystem.o MOD_Grid.o MOD_LandCrop.o MOD_LandElm.o MOD_LandPFT.o MOD_LandPatch.o MOD_LandUrban.o MOD_Namelist.o MOD_NetCDFSerial.o MOD_SPMD_Task.o MOD_SpatialMapping.o
MOD_SrfdataRestart.o: MOD_Block.o MOD_Filesystem.o MOD_Mesh.o MOD_Namelist.o MOD_NetCDFSerial.o MOD_NetCDFVector.o MOD_Pixelset.o MOD_SPMD_Task.o MOD_Utils.o
MOD_Thermal.o: MOD_CaMa_colmCaMa.o MOD_FrictionVelocity.o MOD_GroundFluxes.o MOD_GroundTemperature.o MOD_LeafTemperature.o MOD_LeafTemperaturePC.o MOD_Ozone.o MOD_SoilSurfaceResistance.o
MOD_TimeManager.o: MOD_Precision.o MOD_UserDefFun.o
MOD_UnitCatchmentRegional.o: MOD_UnitCatchmentSubset.o
MOD_UrbanIniTimeVariable.o: MOD_Urban_Albedo.o
MOD_UrbanReadin.o: MOD_Urban_Vars_TimeInvariants.o MOD_Vars_TimeInvariants.o MOD_Vars_TimeVariables.o
MOD_Urban_Albedo.o: MOD_Urban_Shortwave.o
MOD_Urban_Const_LCZ.o: MOD_Precision.o
MOD_Urban_Flux.o: MOD_AssimStomataConductance.o MOD_CanopyLayerProfile.o MOD_FrictionVelocity.o
MOD_Urban_GroundFlux.o: MOD_FrictionVelocity.o
MOD_Urban_Hydrology.o: MOD_Lake.o MOD_SoilSnowHydrology.o
MOD_Urban_ImperviousTemperature.o: MOD_PhaseChange.o MOD_SoilThermalParameters.o
MOD_Urban_LAIReadin.o: MOD_Urban_Vars_TimeInvariants.o MOD_Vars_TimeInvariants.o MOD_Vars_TimeVariables.o
MOD_Urban_PerviousTemperature.o: MOD_PhaseChange.o MOD_SoilThermalParameters.o
MOD_Urban_RoofFlux.o: MOD_FrictionVelocity.o
MOD_Urban_RoofTemperature.o: MOD_PhaseChange.o
MOD_Urban_Shortwave.o: MOD_3DCanopyRadiation.o
MOD_Urban_Thermal.o: MOD_Lake.o MOD_Urban_BEM.o MOD_Urban_Flux.o MOD_Urban_GroundFlux.o MOD_Urban_ImperviousTemperature.o MOD_Urban_LUCY.o MOD_Urban_Longwave.o MOD_Urban_PerviousTemperature.o MOD_Urban_RoofTemperature.o MOD_Urban_WallTemperature.o
MOD_UserDefFun.o: MOD_Precision.o
MOD_UserSpecifiedForcing.o: MOD_Qsadv.o
MOD_Utils.o: MOD_Precision.o MOD_SPMD_Task.o
MOD_Vars_1DAccFluxes.o: MOD_Catch_Hist.o MOD_Forcing.o MOD_FrictionVelocity.o MOD_MT_LandPhase.o MOD_TurbulenceLEddy.o
MOD_Vars_1DFluxes.o: MOD_BGC_Vars_1DFluxes.o MOD_Catch_Vars_1DFluxes.o MOD_DA_Vars_1DFluxes.o MOD_Urban_Vars_1DFluxes.o MOD_Vars_1DPFTFluxes.o
MOD_Vars_1DPFTFluxes.o: MOD_BGC_Vars_1DPFTFluxes.o
MOD_Vars_Global.o: MOD_Namelist.o MOD_Precision.o
MOD_Vars_TimeInvariants.o: MOD_BGC_Vars_TimeInvariants.o MOD_Urban_Vars_TimeInvariants.o
MOD_Vars_TimeVariables.o: MOD_BGC_Vars_PFTimeVariables.o MOD_BGC_Vars_TimeVariables.o MOD_Catch_Vars_TimeVariables.o MOD_DA_Vars_TimeVariables.o MOD_Grid_RiverLakeTimeVars.o MOD_MT_Lifecycle.o MOD_MT_Rest.o MOD_MT_RiverLake.o MOD_Urban_Vars_TimeVariables.o MOD_Vars_TimeInvariants.o
MOD_VicParaReadin.o: MOD_Vars_TimeInvariants.o MOD_Vars_TimeVariables.o
MOD_WorkerPushData.o: MOD_DataType.o MOD_Grid.o MOD_Pixelset.o MOD_Precision.o MOD_SPMD_Task.o MOD_SpatialMapping.o MOD_Utils.o
MOD_prospect_DB.o: MOD_dataSpec_PDB.o MOD_tav_abs.o
MOD_tav_abs.o: MOD_dataSpec_PDB.o
MT_RegisterProviders.o: MOD_MT_Methane.o
POST_Vector2Grid.o: MOD_Vector2Grid.o
# <<< end of generated Fortran module prerequisites <<<

# Initialize NetCDF before the first input read in each kernel program.
MKSRFDATA.o CoLMINI.o CoLM.o: MOD_NetCDFSerial.o
