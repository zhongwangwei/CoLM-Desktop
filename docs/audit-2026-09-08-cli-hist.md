# CLI / History 审查与修复报告（2026-09-08）

范围：`crates/colm-cli` 与 `crates/colm-hist`。复核了 2026-08-26 报告中已列 CLI/HIST 项，未重复上报其已修问题。本报告只记录本轮在该范围内新确认并修复的问题，以及仍需跨模块/后续裁决的风险。

## 本轮确认并修复

| 文件/模块 | 严重度 | 复现/证据 | 修复 | 测试 |
|---|---|---|---|---|
| `crates/colm-hist/src/pair.rs` 观测配对 | S3 | `qc == 0` 且值为 `+Inf` 的观测样本满足旧条件 `value > FILL_VALUE + 1.0`，会进入配对并污染 RMSE/KGE 等科学指标。 | 观测值进入配对前必须 `is_finite()`；不改变原有 QC、fill value 和半小时/小时聚合规则。 | `pair::pair_tests::nonfinite_observations_never_enter_metrics`；`cargo test -p colm-hist --lib`；真实 `PLUMBER2_ROOT=/Users/zhongwangwei/Desktop/colm-rust/PLUMBER2s cargo test -p oracle --test metrics`。 |
| `crates/colm-cli/src/main.rs` metrics 边界 | S3 | 模型变量或观测变量若不是与 `time` 等长的一维序列，底层配对函数无法返回错误；静默截断会生成看似有效的部分指标，panic 又缺少用户上下文。 | 在 `compute_metric_rows` 调用配对前新增显式长度合同：模型值数必须等于模型时间步；观测 value/QC 必须等于观测 time。 | `history_tests::metric_pairing_rejects_mismatched_lengths_before_pairing`；`cargo test -p colm-cli --bin colm-cli history_tests::`；`cargo test -p colm-cli --bin colm-cli`。 |
| `crates/colm-cli/src/main.rs` history 时间轴 | S3 | `check_increasing()` 只检查 `w[1] <= w[0]`，`NaN` 可绕过比较，`Inf` 也会进入 history catalog/series 的 Unix 秒转换。 | history time 轴先逐点拒绝非有限值，再检查严格递增。 | `history_tests::a_time_axis_that_does_not_increase_is_refused`；`cargo test -p colm-cli --bin colm-cli`。 |
| `crates/colm-cli/src/main.rs` `clear_history()` | S2 | `std::fs::read_dir(out/history)` 会跟随 symlink；当 `history` 或祖先 `case/out` 是 symlink 时，重跑 `colm` 前清理旧 history 会删除外部目录中的 `*_hist_*.nc`。 | 清理前检查 CLI 管理边界 `case/out`、`case/out/<case>`、`history` 三层，任一 symlink 均拒绝。 | `history_tests::clear_history_refuses_a_symlinked_history_directory`；`history_tests::clear_history_refuses_a_symlinked_output_ancestor`；`cargo test -p colm-cli --bin colm-cli`。 |
| `crates/colm-cli/src/main.rs` `run_case()` 输出目录合同 | S2 | CLI 用 `Layout::out()/DEF_CASE_NAME` 做指纹、跳过判定和 history 清理；Fortran 实际读 `DEF_dir_output`。手工或 GUI 修改成自定义目录会让 CLI 管理路径与模型输出路径分叉。 | `run_case()` 运行前读取 `case.nml`，只接受 `DEF_dir_output` 等于 CLI 管理的 `case/out`（支持 `out/`、`./out`、绝对等价路径），其他值 fail-fast。 | `history_tests::run_rejects_custom_output_dirs_before_using_cli_history_paths`；`history_tests::output_dir_normalization_handles_root_current_dir_and_trailing_slashes`；`cargo test -p colm-cli --bin colm-cli`。 |
| `crates/colm-cli/src/main.rs` `cmd_new` / `cmd_spatial_new` | S2 | `--name` 或默认 case name 会进入 `DEF_CASE_NAME` 和产物路径；共享 `colm_case::case_name()` 已校验读取端，但 CLI 创建入口仍需在写盘前拒绝非法名称。 | `cmd_new`、`cmd_spatial_new` 在首次写入/stage 前调用 `colm_case::validate_case_name`，禁止空、首尾空白、`.`、`..`、路径分隔符、冒号、控制字符、超过 256 字节。 | `history_tests::cli_case_name_inputs_use_the_shared_single_component_rule`；`cargo test -p colm-cli --bin colm-cli`。 |
| `crates/colm-cli/src/study/engine.rs` / `materialize.rs` / `runner.rs` Study 命名入口 | S2 | Study base case 目录名、apply 的 `--name`、物化的 `member-site` 会进入新 case 的 `DEF_CASE_NAME`。非法目录名可能到物化中途才失败或形成危险名称。 | Study base case 解析、member materialize、apply 输出 case name 全部复用 `colm_case::validate_case_name`。 | `study::engine::tests::base_case_names_must_be_single_safe_components`；`cargo test -p colm-cli --bin colm-cli study::engine::tests::`；`cargo test -p colm-cli --test study_cli_tuning`。 |

## 复核未改项

| 模块 | 结论 |
|---|---|
| `study/checkpoint.rs` | generation 文件不可覆盖、payload hash、损坏 latest 回退和继续写入已有测试覆盖；本轮未发现需改动。 |
| `study/state.rs` | pause/cancel marker 与状态转换已有不可逆转换拒绝和 running→review 保护；本轮未新增修改。 |
| `study/runner.rs` result 读取 | `read_result` 拒绝绝对路径、`..`、canonicalize 后越界和 symlink escape；`result_files` 跳过 symlink；本轮复核后不改。 |
| `study/export.rs` | 导出目标不能嵌入 Study、重复导出是快照、CSV 字段有转义；本轮不改。 |
| `study/science.rs` / `colm-hist/src/metric.rs` | KGE β/α、常数序列、quantile、Spearman ties 已有单元测试；本轮新增真实 PLUMBER2 metrics 复跑通过。 |
| `fingerprint.rs` | 复核了 2026-08-26 已修的目录字段、forcing inventory、kernel stage identity 相关测试；本轮未重复修改。 |
| `observation_table.rs` | 时间解析、无效/歧义时间、multi-site convert 测试仍在 `colm-cli --bin` 全单元内通过；本轮未发现新 confirmed bug。 |

## 未验证/后续风险

1. **自定义 `DEF_dir_output` 尚未作为功能支持。** 本轮只是防止 CLI 管理路径与 Fortran 输出路径分叉时继续运行；GUI 若允许编辑该字段，产品层需决定是禁用编辑、同步 CLI layout，还是完整支持自定义输出目录。
2. **vendored Fortran `CALL system('mkdir -p ' // path)` 路径 shell 转义问题不在本 agent 修改范围。** 本轮只覆盖 CLI/HIST 边界，不声称内核对带空格或 shell 元字符路径全量安全。
3. **平台实测仍以 macOS 为主。** symlink 祖先防护的回归是 `#[cfg(unix)]`；Windows 由路径名冒号校验、现有 CI 和后续平台测试覆盖。
4. **真实 rawdata/runtime 只读核实但未启动完整物理内核长跑。** 本轮真实数据验证聚焦 metrics 配对路径；未做多小时 Fortran Study E2E。

## 验证记录

- `CARGO_TARGET_DIR=target/audit-cli-hist cargo test -p colm-hist --lib` → 35 passed。
- `CARGO_TARGET_DIR=target/audit-cli-hist cargo test -p colm-cli --bin colm-cli` → 156 passed。
- `CARGO_TARGET_DIR=target/audit-cli-hist cargo test -p colm-cli --test study_cli_tuning` → 6 passed。
- `CARGO_TARGET_DIR=target/audit-cli-hist cargo test -p colm-hist --test drift` → 1 passed。
- `PLUMBER2_ROOT=/Users/zhongwangwei/Desktop/colm-rust/PLUMBER2s CARGO_TARGET_DIR=target/audit-cli-hist cargo test -p oracle --test metrics -- --nocapture` → 4 passed，未把缺环境变量当通过。
- `CARGO_TARGET_DIR=target/audit-cli-hist cargo clippy -p colm-hist --all-targets -- -D warnings` → passed。
- `CARGO_TARGET_DIR=target/audit-cli-hist cargo clippy -p colm-cli --all-targets -- -D warnings` → passed。
- `cargo fmt --all --check` → passed。
- `git diff --check` → passed。
