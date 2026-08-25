import assert from 'node:assert/strict';
import {
  fieldLabel,
  fortranNumberInputValue,
  isCommonField,
  optionLabel,
  technicalFieldHint,
} from '../dist/app/param-presentation.js';

assert.equal(fortranNumberInputValue('-1.0_r8'), '-1.0');
assert.equal(fortranNumberInputValue('2.e-4_r8'), '2.0e-4');
assert.equal(fortranNumberInputValue('.5D+01'), '0.5e+01');
assert.equal(fortranNumberInputValue('not-a-number'), null);

assert.equal(fieldLabel('DEF_Runoff_SCHEME', 'zh'), '产流方案');
assert.equal(fieldLabel('DEF_Runoff_SCHEME', 'en'), 'Runoff scheme');
assert.match(optionLabel('DEF_Runoff_SCHEME', '3', 'zh'), /Simple VIC/);
assert.match(optionLabel('DEF_Runoff_SCHEME', '3', 'zh'), /3/);
assert.match(optionLabel('DEF_precip_phase_discrimination_scheme', 'I', 'zh'), /湿球温度/);
assert.match(optionLabel('DEF_SSP', 'off', 'zh'), /关闭/);
assert.match(optionLabel('DEF_DS_precipitation_adjust_scheme', 'II', 'zh'), /MicroMet/);
assert.match(optionLabel('DEF_DS_longwave_adjust_scheme', 'I', 'zh'), /TopoSCALE/);
assert.match(optionLabel('DEF_USE_Campbell_SOIL_MODEL', '.false.', 'zh'), /van Genuchten/);
assert.equal(fieldLabel('DEF_RSS_SCHEME', 'zh'), '土壤表面阻抗方案');
assert.equal(optionLabel('DEF_USE_SNICAR', '.true.', 'zh'), '启用');
assert.equal(optionLabel('DEF_USE_SNICAR', '.false.', 'en'), 'Disabled');
assert.match(optionLabel('DEF_USE_Forcing_Downscaling', '.true.', 'zh'), /自动关闭简化方案/);
assert.match(optionLabel('DEF_USE_Forcing_Downscaling_Simple', '.true.', 'zh'), /自动关闭完整方案/);
assert.equal(fieldLabel('DEF_SOIL_REFL_SCHEME', 'zh'), '土壤反照率来源');
assert.equal(fieldLabel('GUI_STOMATAL_CONDUCTANCE_SCHEME', 'zh'), '气孔导度方案');
assert.match(optionLabel('GUI_STOMATAL_CONDUCTANCE_SCHEME', 'BALL_BERRY', 'zh'), /Ball–Berry/);
assert.match(optionLabel('GUI_STOMATAL_CONDUCTANCE_SCHEME', 'MEDLYN', 'zh'), /Medlyn/);
assert.match(optionLabel('GUI_STOMATAL_CONDUCTANCE_SCHEME', 'WUE', 'zh'), /水分利用效率/);
assert.match(fieldLabel('DEF_BALL_BERRY_GRADM', 'zh'), /gradm/);
assert.match(fieldLabel('DEF_BALL_BERRY_BINTER', 'zh'), /binter/);
assert.match(fieldLabel('DEF_MEDLYN_G1', 'en'), /g1/);
assert.match(fieldLabel('DEF_MEDLYN_G0', 'en'), /g0/);
assert.match(fieldLabel('DEF_WUE_LAMBDA', 'zh'), /lambda/);
assert.equal(fieldLabel('DEF_LC_VMAX25', 'zh'), '25 ℃ 最大羧化速率（μmol m⁻² s⁻¹）');
assert.equal(fieldLabel('DEF_LC_D50', 'en'), '50% rooting depth (cm)');
assert.match(optionLabel('DEF_LC_C3C4', '0', 'zh'), /C4/);
assert.match(optionLabel('DEF_LC_C3C4', '1', 'en'), /C3/);
for (const field of [
  'DEF_BALL_BERRY_GRADM', 'DEF_BALL_BERRY_BINTER',
  'DEF_MEDLYN_G1', 'DEF_MEDLYN_G0', 'DEF_WUE_LAMBDA',
  'DEF_TUNING_ZLND', 'DEF_TUNING_CAPR', 'DEF_TUNING_SMPMAX', 'DEF_PH_ROOT_RADIUS',
  'DEF_OZONE_KO3', 'DEF_DS_SHORTWAVE_SIMPLE_LIMIT',
  'DEF_LC_HTOP0', 'DEF_LC_VMAX25', 'DEF_LC_C3C4', 'DEF_LC_PSI50_ROOT',
]) assert.equal(isCommonField(field), false, `${field} must stay expert-only`);
assert.equal(fieldLabel('DEF_TUNING_ZLND', 'zh'), '土壤空气动力粗糙长度（m）');
assert.equal(fieldLabel('DEF_TUNING_CAPR', 'zh'), '首层土温到地表温度调节因子');
assert.equal(fieldLabel('DEF_TUNING_SIMPLE_VIC_DS', 'zh'), 'Simple VIC 基流比例 Ds');
assert.equal(fieldLabel('DEF_TUNING_IRRIGATION_START_SEC', 'en'), 'Local irrigation start time (second of day)');
assert.equal(fieldLabel('DEF_TUNING_CROP_PLANTING_DAY', 'zh'), '作物种植日（年内日序）');
assert.equal(fieldLabel('DEF_TUNING_CROP_PLANTING_DAY', 'en'), 'Crop planting day (day of year)');
assert.equal(isCommonField('DEF_TUNING_IRRIGATION_PONDMX'), false);
assert.equal(isCommonField('DEF_TUNING_CROP_PLANTING_DAY'), false);
assert.equal(fieldLabel('DEF_PH_ROOT_RADIUS', 'en'), 'Fine-root radius (m)');
assert.equal(fieldLabel('DEF_OZONE_KO3', 'zh'), '臭氧气孔阻力系数');
assert.equal(fieldLabel('DEF_DS_SHORTWAVE_SIMPLE_LIMIT', 'en'), 'Simple-mode shortwave correction limit');
assert.match(optionLabel('DEF_USE_OZONEDATA', '.false.', 'zh'), /100 ppbv/);

// 即使上游新增字段尚未写专门翻译，也不能把 DEF_/USE_SITE_ 暴露为主标签。
for (const name of ['DEF_NEW_SOIL_SCHEME', 'USE_SITE_new_measurement', 'SITE_new_value']) {
  assert.ok(!fieldLabel(name, 'zh').includes('DEF_'), name);
  assert.ok(!fieldLabel(name, 'zh').includes('USE_SITE_'), name);
}
assert.match(technicalFieldHint('DEF_Runoff_SCHEME', 'zh'), /DEF_Runoff_SCHEME/);
assert.equal(isCommonField('DEF_Runoff_SCHEME'), true);
assert.equal(isCommonField('DEF_USE_LAI_SAI_MONTHLY'), false);

const params = await import('node:fs').then(fs =>
  fs.readFileSync(new URL('../dist/app/params.js', import.meta.url), 'utf8'));
assert.match(params, /fieldLabel\(e\.path, language\(\)\)/);
assert.match(params, /optionLabel\(e\.path, v, language\(\)\)/);
assert.doesNotMatch(params, /k\.textContent = e\.path;\s*\/\/ 主标签/s);
assert.match(params, /configure_cbl_batch/);
assert.match(params, /enabled\(inp\.value\) && e\.path === 'DEF_USE_CBL_HEIGHT'[\s\S]*pickParameterPath\('DEF_USE_CBL_HEIGHT', 'file'\)[\s\S]*configure_cbl_batch/);
const cbl = params.slice(
  params.indexOf("enabled(inp.value) && e.path === 'DEF_USE_CBL_HEIGHT'"),
  params.indexOf("e.path === 'DEF_USE_OZONESTRESS'"),
);
assert.match(cbl, /if \(dirs\.length !== 1\)/);
assert.match(cbl, /configure_cbl_batch', \{\s*dirs,/);
assert.doesNotMatch(cbl, /state\.selected/);
assert.match(params, /e\.path === 'DEF_USE_CBL_HEIGHT' && dirs\.length > 1\) inp\.disabled = true/);
assert.doesNotMatch(params, /选择\/更换边界层数据/);
assert.match(params, /configure_ozone_batch/);
assert.match(params, /if \(fieldState\?\.mixed\) inp\.disabled = true/);
assert.match(params, /if \(PATH_FIELDS\[e\.path\]\) inp\.readOnly = true/);
assert.match(params, /pick\.disabled = inp\.disabled/);
assert.match(params, /collapseStomatal/);
assert.match(params, /DEF_USE_MEDLYNST[\s\S]*DEF_USE_WUEST/);
assert.match(params, /STABLE_IN_PLACE_FIELDS[\s\S]*DEF_precip_phase_discrimination_scheme/);
const scope = params.slice(
  params.indexOf('function renderScope'),
  params.indexOf('export async function renderFields'),
);
assert.doesNotMatch(scope, /createElement\('button'\)|state\.batch/);
assert.match(scope, /除逐站点数据文件外/);
assert.doesNotMatch(scope, /innerHTML/, 'batch case names must be appended as text, not interpolated as HTML');
assert.match(scope, /bar\.append/, 'batch scope summary should preserve markup without unsafe name interpolation');

console.log('params: scheme choices have readable labels while preserving raw CoLM values');
assert.match(params, /process_parameter_files/);
assert.match(params, /set_process_parameter_field/);
assert.match(params, /expertCaseDir/);
assert.match(params, /修改站点/);
assert.match(params, /renderExpertFields/);
assert.match(params, /context_default/);
assert.match(params, /default_mixed/);
assert.match(params, /withContextDefaults/);
assert.match(params, /site_pfts/);
assert.match(params, /pft_parameter_states/);
assert.match(params, /set_pft_parameter_batch/);
assert.match(params, /不含该 PFT 的站点不会被修改/);
assert.match(params, /value: null, kernelDir/);
assert.match(params, /PFT_IDENTITY_FIELDS[\s\S]*SITE_fsitedata[\s\S]*SITE_landtype/);
assert.match(params, /invalidatePftSites\(dirs, changes\)/);
assert.match(params, /PFT_SITE_CACHE\.delete\(key\)[\s\S]*throw error/);
const pftRender = params.slice(
  params.indexOf('async function renderPftParameters'),
  params.indexOf('function publishFlows'),
);
assert.match(pftRender, /try \{\s*usable = await pftSites\(selectedCases\);\s*\} catch \(error\) \{\s*status\(error\);\s*return;/);
assert.doesNotMatch(pftRender, /loaded\.filter/, 'a failed All-sites PFT read must block editing, not drop that site');
assert.match(params, /\.filter\(id => id !== 0\)/);

assert.match(params, /const EXPERT_ALL = '__all__'/);
assert.match(params, /all\.textContent = language\(\) === 'en' \? 'All sites' : '全部站点'/);
assert.match(params, /function expertDirs\(\)[\s\S]*EXPERT_ALL[\s\S]*cases\.map\(c => c\.dir\)/);
assert.match(params, /set_process_parameter_field_batch/);
assert.match(params, /function commonProcessFiles/);
assert.match(params, /commonProcessFiles\(lists\)/);
assert.match(params, /const cases = currentCases\(\)/);
assert.match(params, /renderProcessPicker\(basic, parameterCases\)/);
assert.match(params, /appendDefaultValue\(v, entry\.path, entry\.default, entry\.kind\)/);
assert.match(params, /if \(showDefaults\) appendDefaultValue\([\s\S]*fieldState\?\.context_default \?\? meta\?\.default/);
