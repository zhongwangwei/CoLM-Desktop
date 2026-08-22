//! Human-readable land-cover legends used by the site preprocessor.
//!
//! Values and English names follow CoLM's `MOD_Const_LC.F90` and
//! `preprocess/rd_land_types.F90`. Keep the numeric value because it is the
//! scientific input written to IGBP_classification or USGS_classification;
//! the GUI presents the name so users do not have to memorize codes.

const IGBP = [
  [1, '常绿针叶林', 'Evergreen Needleleaf Forests'],
  [2, '常绿阔叶林', 'Evergreen Broadleaf Forests'],
  [3, '落叶针叶林', 'Deciduous Needleleaf Forests'],
  [4, '落叶阔叶林', 'Deciduous Broadleaf Forests'],
  [5, '混交林', 'Mixed Forests'],
  [6, '密闭灌丛', 'Closed Shrublands'],
  [7, '稀疏灌丛', 'Open Shrublands'],
  [8, '木本稀树草原', 'Woody Savannas'],
  [9, '稀树草原', 'Savannas'],
  [10, '草地', 'Grasslands'],
  [11, '永久性湿地', 'Permanent Wetlands'],
  [12, '农田', 'Croplands'],
  [13, '城市与建成区', 'Urban and Built-up Lands'],
  [14, '农田/自然植被镶嵌区', 'Cropland/Natural Vegetation Mosaics'],
  [15, '永久冰雪', 'Permanent Snow and Ice'],
  [16, '裸地或稀疏植被', 'Barren or Sparsely Vegetated'],
  [17, '水体', 'Water Bodies'],
];

const USGS = [
  [1, '城市与建成区', 'Urban and Built-Up Land'],
  [2, '旱作农田与牧场', 'Dryland Cropland and Pasture'],
  [3, '灌溉农田与牧场', 'Irrigated Cropland and Pasture'],
  [4, '旱作/灌溉混合农田与牧场', 'Mixed Dryland/Irrigated Cropland and Pasture'],
  [5, '农田/草地镶嵌区', 'Cropland/Grassland Mosaic'],
  [6, '农田/林地镶嵌区', 'Cropland/Woodland Mosaic'],
  [7, '草地', 'Grassland'],
  [8, '灌丛', 'Shrubland'],
  [9, '灌丛/草地混合区', 'Mixed Shrubland/Grassland'],
  [10, '稀树草原', 'Savanna'],
  [11, '落叶阔叶林', 'Deciduous Broadleaf Forest'],
  [12, '落叶针叶林', 'Deciduous Needleleaf Forest'],
  [13, '常绿阔叶林', 'Evergreen Broadleaf Forest'],
  [14, '常绿针叶林', 'Evergreen Needleleaf Forest'],
  [15, '混交林', 'Mixed Forest'],
  [16, '内陆水体', 'Inland Water'],
  [17, '草本湿地', 'Herbaceous Wetland'],
  [18, '林木湿地', 'Wooded Wetland'],
  [19, '裸地或稀疏植被', 'Barren or Sparsely Vegetated'],
  [20, '草本苔原', 'Herbaceous Tundra'],
  [21, '林木苔原', 'Wooded Tundra'],
  [22, '混合苔原', 'Mixed Tundra'],
  [23, '裸地苔原', 'Bare Ground Tundra'],
  [24, '冰雪', 'Snow or Ice'],
];

const entries = rows => Object.freeze(rows.map(([value, zh, en]) => Object.freeze({ value, zh, en })));
const IGBP_CLASSES = entries(IGBP);
const USGS_CLASSES = entries(USGS);

export function landCoverClasses(mode) {
  return String(mode).toLowerCase() === 'usgs' ? USGS_CLASSES : IGBP_CLASSES;
}

export function landCoverLabel(item, locale = 'zh') {
  return `${item.value} · ${locale === 'en' ? item.en : item.zh}`;
}
