// 历史输出变量的预设与统计。开关名即 `DEF_hist_vars%<name>` 的后缀。
export type HistoryVariable = {
  name: string;
  // 当前内核 + 配置下能否通过开关写出；不能的只读展示原因。
  settable: boolean;
  longName?: string | null;
  reason?: string | null;
};

// 推荐核心量：科研最常用的能量、辐射、水分、温度、气象与碳通量/状态量（带中文说明与分类）。
export interface HistoryVariableItem {
  name: string;
  label_zh: string;
  label_en: string;
  category: 'energy' | 'water' | 'radiation' | 'temperature' | 'carbon' | 'forcing' | 'other';
  long_name?: string;
  recommended?: boolean;
}

export const RECOMMENDED_VARIABLES: HistoryVariableItem[] = [
  // 能量通量
  {
    name: 'fsena',
    label_zh: '感热通量',
    label_en: 'Sensible heat flux',
    category: 'energy',
    long_name: 'sensible heat flux from canopy to atmosphere',
    recommended: true,
  },
  {
    name: 'fevpa',
    label_zh: '潜热通量',
    label_en: 'Latent heat flux',
    category: 'energy',
    long_name: 'latent heat flux from canopy to atmosphere',
    recommended: true,
  },
  {
    name: 'lfevpa',
    label_zh: '潜热通量（水当量）',
    label_en: 'Latent heat flux (water equiv)',
    category: 'energy',
    long_name: 'latent heat flux (water equivalent)',
    recommended: true,
  },
  {
    name: 'fgrnd',
    label_zh: '土壤地表热通量',
    label_en: 'Ground heat flux',
    category: 'energy',
    long_name: 'ground heat flux into soil surface',
    recommended: true,
  },
  {
    name: 'zerr',
    label_zh: '能量平衡残差',
    label_en: 'Energy balance residual',
    category: 'energy',
    long_name: 'the error of energy balance',
    recommended: true,
  },

  // 辐射平衡与反照率
  {
    name: 'olrg',
    label_zh: '地面出射长波辐射',
    label_en: 'Outgoing longwave radiation',
    category: 'radiation',
    long_name: 'outgoing longwave radiation at surface',
    recommended: true,
  },
  {
    name: 'sabg',
    label_zh: '地面吸收太阳短波辐射',
    label_en: 'Ground absorbed solar radiation',
    category: 'radiation',
    long_name: 'solar radiation absorbed by ground',
    recommended: true,
  },
  {
    name: 'sabvsun',
    label_zh: '阳叶吸收太阳辐射',
    label_en: 'Sunlit leaf absorbed solar',
    category: 'radiation',
    long_name: 'solar radiation absorbed by sunlit canopy',
    recommended: true,
  },
  {
    name: 'sabvsha',
    label_zh: '阴叶吸收太阳辐射',
    label_en: 'Shaded leaf absorbed solar',
    category: 'radiation',
    long_name: 'solar radiation absorbed by shaded canopy',
    recommended: true,
  },
  {
    name: 'alb',
    label_zh: '地表综合反照率',
    label_en: 'Surface albedo',
    category: 'radiation',
    long_name: 'averaged albedo',
    recommended: true,
  },

  // 水分平衡与水文通量
  {
    name: 'etr',
    label_zh: '植被实际蒸腾量',
    label_en: 'Canopy transpiration',
    category: 'water',
    long_name: 'vegetation transpiration rate',
    recommended: true,
  },
  {
    name: 'fevpl',
    label_zh: '冠层截留蒸发量',
    label_en: 'Canopy evaporation',
    category: 'water',
    long_name: 'canopy intercepted water evaporation',
    recommended: true,
  },
  {
    name: 'fevpg',
    label_zh: '地面直接蒸发量',
    label_en: 'Ground evaporation',
    category: 'water',
    long_name: 'ground surface water evaporation',
    recommended: true,
  },
  {
    name: 'wat',
    label_zh: '总土壤储水量',
    label_en: 'Total soil water storage',
    category: 'water',
    long_name: 'total water storage in soil column',
    recommended: true,
  },
  {
    name: 'wliq_soisno',
    label_zh: '土壤液态含水量',
    label_en: 'Soil liquid water content',
    category: 'water',
    long_name: 'liquid water content in soil layers',
    recommended: true,
  },
  {
    name: 'h2osoi',
    label_zh: '土壤体积含水率',
    label_en: 'Volumetric soil water',
    category: 'water',
    long_name: 'volumetric water in soil layers',
    recommended: true,
  },
  {
    name: 'rnof',
    label_zh: '总径流量',
    label_en: 'Total runoff',
    category: 'water',
    long_name: 'total surface and subsurface runoff',
    recommended: true,
  },
  {
    name: 'rsur',
    label_zh: '地表直接径流',
    label_en: 'Surface runoff',
    category: 'water',
    long_name: 'surface runoff',
    recommended: true,
  },
  {
    name: 'rsub',
    label_zh: '地下下渗径流',
    label_en: 'Subsurface drainage',
    category: 'water',
    long_name: 'subsurface drainage runoff',
    recommended: true,
  },
  {
    name: 'zwt',
    label_zh: '地下水位埋深',
    label_en: 'Water table depth',
    category: 'water',
    long_name: 'depth to water table',
    recommended: true,
  },
  {
    name: 'xerr',
    label_zh: '水分平衡残差',
    label_en: 'Water balance residual',
    category: 'water',
    long_name: 'the error of water balance',
    recommended: true,
  },

  // 地表温度
  {
    name: 't_grnd',
    label_zh: '地表土壤温度',
    label_en: 'Ground surface temperature',
    category: 'temperature',
    long_name: 'ground surface temperature',
    recommended: true,
  },
  {
    name: 't_soisno',
    label_zh: '各层土壤与积雪温度',
    label_en: 'Soil & snow temperature',
    category: 'temperature',
    long_name: 'soil and snow temperatures by layer',
    recommended: true,
  },
  {
    name: 'trad',
    label_zh: '地表辐射温度',
    label_en: 'Surface radiative temperature',
    category: 'temperature',
    long_name: 'radiative temperature of ground surface',
    recommended: true,
  },
  {
    name: 'tleaf',
    label_zh: '植被冠层叶片温度',
    label_en: 'Canopy leaf temperature',
    category: 'temperature',
    long_name: 'leaf temperature',
    recommended: true,
  },

  // 2米气象诊断与驱动参考
  {
    name: 'tref',
    label_zh: '2米空气温度',
    label_en: '2m air temperature',
    category: 'forcing',
    long_name: '2 m height air temperature',
    recommended: true,
  },
  {
    name: 'qref',
    label_zh: '2米空气比湿',
    label_en: '2m specific humidity',
    category: 'forcing',
    long_name: '2 m height air specific humidity',
    recommended: true,
  },
  {
    name: 'xy_solarin',
    label_zh: '地表入射短波辐射',
    label_en: 'Downward solar radiation',
    category: 'forcing',
    long_name: 'downward solar radiation at surface',
    recommended: true,
  },
  {
    name: 'xy_frl',
    label_zh: '地表入射长波辐射',
    label_en: 'Downward atmospheric longwave',
    category: 'forcing',
    long_name: 'atmospheric infrared (longwave) radiation',
    recommended: true,
  },
  {
    name: 'xy_rain',
    label_zh: '地面降雨通量',
    label_en: 'Rainfall rate',
    category: 'forcing',
    long_name: 'rain precipitation rate',
    recommended: true,
  },
  {
    name: 'xy_snow',
    label_zh: '地面降雪通量',
    label_en: 'Snowfall rate',
    category: 'forcing',
    long_name: 'snow precipitation rate',
    recommended: true,
  },
  {
    name: 'xy_pbot',
    label_zh: '表面大气气压',
    label_en: 'Surface pressure',
    category: 'forcing',
    long_name: 'atmospheric pressure at the surface',
    recommended: true,
  },
  {
    name: 'xy_us',
    label_zh: '东向表面风速',
    label_en: 'Eastward wind',
    category: 'forcing',
    long_name: 'wind in eastward direction',
    recommended: true,
  },
  {
    name: 'xy_vs',
    label_zh: '北向表面风速',
    label_en: 'Northward wind',
    category: 'forcing',
    long_name: 'wind in northward direction',
    recommended: true,
  },
  {
    name: 'xy_t',
    label_zh: '参考高度气温',
    label_en: 'Reference air temperature',
    category: 'forcing',
    long_name: 'temperature at reference height',
    recommended: true,
  },
  {
    name: 'xy_q',
    label_zh: '参考高度比湿',
    label_en: 'Reference air humidity',
    category: 'forcing',
    long_name: 'specific humidity at reference height',
    recommended: true,
  },

  // 碳循环与生态
  {
    name: 'gpp',
    label_zh: '总初级生产力 GPP',
    label_en: 'Gross primary production (GPP)',
    category: 'carbon',
    long_name: 'gross primary production (BGC)',
    recommended: true,
  },
  {
    name: 'assim',
    label_zh: '冠层光合/GPP同化率',
    label_en: 'Canopy GPP / assimilation',
    category: 'carbon',
    long_name: 'canopy assimilation rate',
    recommended: true,
  },
  {
    name: 'respc',
    label_zh: '植被自养呼吸量',
    label_en: 'Vegetation respiration',
    category: 'carbon',
    long_name: 'vegetation respiration',
    recommended: true,
  },
  {
    name: 'lai',
    label_zh: '叶面积指数',
    label_en: 'Leaf area index',
    category: 'carbon',
    long_name: 'leaf area index',
    recommended: true,
  },
];

export const coreHistoryVariables = RECOMMENDED_VARIABLES.map((item) => item.name);

export const historyKey = (name: string) => `DEF_hist_vars%${name}`;

export function historySelection(
  variables: readonly HistoryVariable[],
  values: Readonly<Record<string, string>>,
) {
  const available = variables.filter((v) => v.settable),
    core = new Set(coreHistoryVariables.map((name) => name.toLowerCase()));
  return {
    available,
    unavailable: variables.filter((v) => !v.settable),
    core: available.filter((v) => core.has(v.name.toLowerCase())),
    selected: available.filter((v) => values[historyKey(v.name)] === '.true.').length,
  };
}

// 预设只改可写的开关；写不出的变量保持原值，不制造“勾了却没有输出”。
export function historyPreset(
  variables: readonly HistoryVariable[],
  preset: 'core' | 'all' | 'none',
): Record<string, string> {
  const core = new Set(coreHistoryVariables.map((name) => name.toLowerCase()));
  return Object.fromEntries(
    variables
      .filter((v) => v.settable)
      .map((v) => [
        historyKey(v.name),
        preset === 'all' || (preset === 'core' && core.has(v.name.toLowerCase()))
          ? '.true.'
          : '.false.',
      ]),
  );
}

/**
 * 后端写不出某个历史变量的原因（`histvars.rs` 的 blocked_by）是中文；英文界面里按已知句式翻成英文，
 * 认不出的原样显示。原来英文界面直接显示"需要 DEF_USE_BGC"（2026-10-22 用户实测）。
 */
export function historyBlockedReason(reason: string, en: boolean): string {
  if (!en) return reason;
  if (reason === '需要 CH4 甲烷示踪物') return 'Requires the CH4 methane tracer';
  const kernel = /^本内核未编入：需要 (.+)$/.exec(reason);
  if (kernel) return `Not compiled into this kernel: requires ${kernel[1]}`;
  const needs = /^需要 (.+)$/.exec(reason);
  if (needs) return `Requires ${needs[1]}`;
  return reason;
}
