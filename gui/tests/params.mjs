import assert from 'node:assert/strict';
import {
  fieldLabel,
  optionLabel,
  technicalFieldHint,
} from '../dist/app/param-presentation.js';

assert.equal(fieldLabel('DEF_Runoff_SCHEME', 'zh'), '产流方案');
assert.equal(fieldLabel('DEF_Runoff_SCHEME', 'en'), 'Runoff scheme');
assert.match(optionLabel('DEF_Runoff_SCHEME', '3', 'zh'), /Simple VIC/);
assert.match(optionLabel('DEF_Runoff_SCHEME', '3', 'zh'), /3/);
assert.match(optionLabel('DEF_precip_phase_discrimination_scheme', 'I', 'zh'), /湿球温度/);
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
assert.match(optionLabel('DEF_USE_OZONEDATA', '.false.', 'zh'), /100 ppbv/);

// 即使上游新增字段尚未写专门翻译，也不能把 DEF_/USE_SITE_ 暴露为主标签。
for (const name of ['DEF_NEW_SOIL_SCHEME', 'USE_SITE_new_measurement', 'SITE_new_value']) {
  assert.ok(!fieldLabel(name, 'zh').includes('DEF_'), name);
  assert.ok(!fieldLabel(name, 'zh').includes('USE_SITE_'), name);
}
assert.match(technicalFieldHint('DEF_Runoff_SCHEME', 'zh'), /DEF_Runoff_SCHEME/);

const params = await import('node:fs').then(fs =>
  fs.readFileSync(new URL('../dist/app/params.js', import.meta.url), 'utf8'));
assert.match(params, /fieldLabel\(e\.path, language\(\)\)/);
assert.match(params, /optionLabel\(e\.path, v, language\(\)\)/);
assert.doesNotMatch(params, /k\.textContent = e\.path;\s*\/\/ 主标签/s);
assert.match(params, /configure_cbl_batch/);
assert.match(params, /configure_ozone_batch/);
assert.match(params, /collapseStomatal/);
assert.match(params, /DEF_USE_MEDLYNST[\s\S]*DEF_USE_WUEST/);

console.log('params: scheme choices have readable labels while preserving raw CoLM values');
