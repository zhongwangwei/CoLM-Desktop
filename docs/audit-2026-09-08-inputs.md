# 输入模块审计报告（forcing / srfdata，2026-09-08）

范围：`crates/colm-forcing`、`crates/colm-srfdata` 源码/测试；历史报告 `docs/code-review-2026-08-26.md` 已复核，未重复上报其已修复项（UTC offset 整分钟、forcing 非有限 time、raster fill value 等）。外部数据只读：`PLUMBER2_ROOT=/Users/zhongwangwei/Desktop/colm-rust/PLUMBER2s`，`COLM_RAWDATA=/Volumes/data02-1/zhwei/CoLMrawdata`。

## 已确认并修复

| 文件/模块 | 严重度 | 问题与复现 | 修复 | 测试 | 未验证风险 |
|---|---:|---|---|---|---|
| `crates/colm-srfdata/src/raster.rs` | S2 | 公开 raster 抽取入口未拒绝 NaN/Inf/越界经纬度；`Grid::index_of` 的浮点转整数会把异常坐标静默夹到边界像元。`itime=0` 还会触发 1-based 下标下溢；`point_i32` 对 NaN/超 i32 像元可静默变成 0/饱和值。 | 在 `point_f64_on`、`tile_5x5_path` 入口统一校验经纬度；`read_pixel` 拒绝 0 下标；`point_i32`/`point_5x5_i32` 转 i32 前校验有限和范围。`tile_5x5_path` 最终签名为 `Result<(PathBuf, usize, usize)>`，外部调用需 `?`。 | `cargo test -p colm-srfdata --lib raster_tests`；`COLM_RAWDATA=... cargo test -p colm-srfdata --test raster`；`cargo check -p oracle`。 | `Grid::index_of` 仍是低层闭式索引且保持 tuple API；信任边界放在公开 raster 文件路径入口。 |
| `crates/colm-srfdata/src/site.rs` | S2 | `location`、`landtype_for_mode`、`read_inputs`、`pft_components` 把 NetCDF 地类值 `as i32`；NaN、小数、越界分类可能被写进 case 或误导 CROP/PFT 分支。 | 增加共享 `classification_value`，要求有限、整数、符合 IGBP 1..=17 / USGS 1..=24；`skeleton_with_mode` 写入前也按模式拒绝越界 landtype。 | `cargo test -p colm-srfdata --lib site::site_tests::landtype_readers_reject_non_integer_or_out_of_range_values -- --exact`；`cargo test -p colm-srfdata --lib`；`COLM_RAWDATA=... PLUMBER2_ROOT=... cargo test -p colm-srfdata --test real_sites`。 | 没有改变既有 IGBP/USGS 优先级；同时存在两种变量时仍优先 IGBP。 |
| `crates/colm-forcing/src/render.rs` | S3 | `DEF_dir_forcing` / `DEF_forcing%fprefix(1)` 直接拼 `'{path}'`；含单引号路径如 `O'Brien` 会生成不能按 Fortran namelist 字符串规则表示的文本。 | 本地最小 `quote_namelist_str`：单引号加倍后再包裹。未新增依赖。 | `cargo test -p colm-forcing --lib render::render_tests::paths_escape_fortran_single_quotes -- --exact`；`PLUMBER2_ROOT=... cargo test -p colm-forcing --test met --test real_forcing`。 | 与主审修复的 `colm-namelist` 字符串转义/解析保持同一 Fortran 规则；本模块仅负责输出。 |
| `crates/colm-forcing/src/tabular.rs` | S3 | CSV/TXT `landtype` 经 `consistent_integer` 先 `as i32` 后按 scheme 校验，`3000000000` 等值会饱和后进入错误的分类判断。 | 在共用整数解析处先拒绝 i32 范围外值，覆盖 probe 和 import。 | `cargo test -p colm-forcing --lib tabular::tabular_tests::land_cover_class_must_fit_before_scheme_validation -- --exact`；`cargo test -p colm-forcing --lib`。 | 仍只校验 landtype 的整数/范围；其他 forcing 数值保留现有单位/QC/gapfill 路径。 |

## 复核但未改

- `colm-forcing::met/check`：历史修复的非有限 time、cadence、height 合同仍有单元和真实 PLUMBER2 覆盖；未发现新缺陷。
- `colm-forcing::gapfill/convert/units/civil/slots`：抽查时间解析、单位换算、ERA5 donor、缺失值、slot 映射已有针对性覆盖；本轮没有扩大数值算法改动面。
- `colm-srfdata::grid/mesh/shapefile/texture/derive/urban_*`：已有边界、形状、WGS84、USDA/urban 表覆盖；本轮只在其上游公开输入处补缺失校验。

## 验证记录

- `cargo fmt --all --check` → pass
- `cargo test -p colm-forcing --lib` → 134 passed
- `cargo test -p colm-srfdata --lib` → 103 passed
- `cargo clippy -p colm-forcing -p colm-srfdata --all-targets -- -D warnings` → pass
- `PLUMBER2_ROOT=/Users/zhongwangwei/Desktop/colm-rust/PLUMBER2s cargo test -p colm-forcing --test met --test real_forcing -- --nocapture` → 8 passed
- `COLM_RAWDATA=/Volumes/data02-1/zhwei/CoLMrawdata cargo test -p colm-srfdata --test raster -- --nocapture` → 5 passed
- `COLM_RAWDATA=/Volumes/data02-1/zhwei/CoLMrawdata PLUMBER2_ROOT=/Users/zhongwangwei/Desktop/colm-rust/PLUMBER2s cargo test -p colm-srfdata --test real_sites -- --nocapture` → 6 passed, 90 站点扫描完成
- `cargo check -p oracle` → pass（验证 `tile_5x5_path -> Result` 的外部调用面）

## 备注

- 本分报告完成时未提交 git commit；后续提交与 PR 状态以 GitHub 为准。
- 工作树中有其他团队成员改动；本报告只覆盖上述输入模块修复。`oracle/src/bin/extract_urban_extra.rs` 的 `?` 调整由主审配合新 API 完成，我仅验证编译通过。
