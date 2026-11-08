# 附录 C：CLI 命令参考

## C.1 命令索引

下表直接来自 `colm-cli --help`（与程序内 `USAGE` 文本一致）。`用法` 列原样保留命令与位置参数；所有命令共享的选项（`--host`/`--root`/`--model-root` 等）见 C.2。

| # | 用法 | 用途 |
|---|---|---|
| 1 | `colm-cli scan    --dir <Sitedata 目录> [--forcing-dir <Forcing 目录>]` | 列出目录下的站点；--quick 跳过强迫场，只读站点文件 |
| 2 | `colm-cli site-new --out <site.nc> --lon <度> --lat <度> [--landtype N] [--crop 1] [--overwrite 1]` | 建一份站点文件：经纬度必给，其余从 rawdata 抽或用 标称假设。--landtype 不给就不写，让 CoLM 回落 |
| 3 | `colm-cli site-pfts <site.nc> [--crop 1] [--landtype N]` |  |
| 4 | `colm-cli case-land-classes <case-dir>     # land classes present in a gridded case's landdata (JSON)` | 输出 PFT/PC 站点的非零 PFT 类型与归一化比例 |
| 5 | `colm-cli new     --site <site.nc> --out <dir> [--name N] [--start Y-M-D] [--end Y-M-D]` | 在 ../Forcing/ 下找，那两套约定只覆盖 PLUMBER2 与 Urban-PLUMBER 城市站点由文件形状自动识别。两个目录都可选：预抽表盖住的 21 个 Urban-PLUMBER 站不给也能跑，表外的站点才要 --rawdata |
| 6 | `colm-cli run     <case-dir> --kernel <dir> [--stream 1] [--force 1] [--ranks N]` | --model-version 用一个工作区（修改版）自己编出的程序跑；跑完写 out/<名>/produced-by.json --force 不读取或写入指纹，三段全部重跑 --stage 只运行指定阶段；与 --force 合用时强制重跑该阶段 --record-input-fingerprints 1 记录文件指纹（非驱动输入/程序/输出；默认关闭；驱动数据始终跳过） --stream 把子进程每一行原样转发出来（GUI 用；终端下嫌吵） --ranks >1：Fortran 内核用 MPI 启动；Rust 引擎只在远程由 COLM_MPIEXEC（srun/mpiexec）启动，默认 1 --preprocessors 选 mksrfdata/mkinidata 的实现，--engine 选 colm 主循环； 两者默认都是 rust。Rust 主循环覆盖 SinglePoint + LCT 土面 patch， 跑不了的算例会在第一段之前被预检拒绝，并提示改用 --engine fortran HYPERSPECTRAL PFT/PC：--highres-params 包含 fsds/、 leaf_optical_properties/ 和 water_params.txt；--soil-hyper-albedo-dir 包含 colm_soil_albedo_400nm.nc ... colm_soil_albedo_2500nm.nc |
| 7 | `colm-cli model-list\|prune\|get\|create\|draft\|save-draft\|freeze\|migrate-case\|rollback\|make-run\|attach-network` | create JSON: {name,note,parent:{id,revision},workspace?}; save-draft: ModelDraft + --expected-generation N freeze: --model-id ID --asset-root DIR [--keep-draft 1]; migrate-case/rollback: <case-dir> prune JSON: {keep:[{id,revision}],dry_run?}: drop unused old official revisions make-run: <case-dir> --out DIR --operation-id ID; attach-network: --model-id ID --expected-generation N + hybrid-install flags |
| 8 | `colm-cli model-code-ls\|model-code-file --model-root DIR --version ID --revision N [--dir D \| --path P]` | 只读：冻结版本的代码（修改版按冻结的那次提交读），列目录或读一个文件 |
| 9 | `colm-cli model-changes --model-root DIR --version ID --revision N [--against ID --against-revision N]` | 只读：代码相对官方源码改了的文件，参数/物理方案/过程文件/AI 网络相对父版本（或 --against）的不同，来源研究 |
| 10 | `colm-cli study-save-version <study-dir> --member best\|ID --name NAME --model-root DIR [--note TEXT]` |  |
| 11 | `colm-cli version-check --name N --kernel <dir> [--engine rust\|fortran]` |  |
| 12 | `colm-cli produced-by <case-dir>` |  |
| 13 | `colm-cli fingerprint-policy` | 查询驱动数据永久排除策略，无需读取算例或数据 |
| 14 | `colm-cli full-input-identity <case-dir>` | 只读核验配置与非驱动输入的 SHA256 内容身份（JSON；驱动数据始终跳过） |
| 15 | `colm-cli metrics <case-dir> --obs <Flux.nc> [--spinup N] [--from UNIX] [--to UNIX]` |  |
| 16 | `colm-cli hybrid-install <case-dir> --model <x.onnx\|x.mlp.json> --slot pft\|land_class` | 把训练好的模型装到算例上：拷进 models/、写 hybrid.toml（含 sha256）。 transform: identity\|clamp\|sigmoid\|softplus；见 docs/design-hybrid.md |
| 17 | `colm-cli hybrid-check <case-dir> --kernel <dir>` | 加载模型、取特征并推理，打印各插槽的行数与特征/输出范围（JSON），不模拟。 需要算例已做完前处理（mkinidata）；单点与空间算例都支持 |
| 18 | `colm-cli hybrid-climate <case-dir> --kernel <dir>` | 在运行时段内累积强迫，给每个 patch 算气候特征（clim_tair、clim_tair_amplitude、 clim_prec、clim_swdown、clim_vpd），写到 <case>/hybrid_climate/；不模拟 |
| 19 | `colm-cli hybrid-fit --studies <dir,dir,...> --network <net.json> --kernel <dir> --out <x.mlp.json>` | 两步训练的第二步：读各站调优 Study 的最优参数，在基础算例上取逐行特征，拟合网络； 两个以上 Study 时做留一站交叉验证，并与“其余站均值”基准比较；不如基准时警告， 加 --require-gate 1 时报错且不写模型。net.json 与 Study 的 hybrid 段同格式 |
| 20 | `colm-cli hybrid-studies <dir> [--depth N]` | 列出目录下（默认最多 4 层）的调优 Study：状态、站点、率定的参数，供两步法挑选 |
| 21 | `colm-cli hybrid-info <case-dir>` | 以 JSON 列出算例的混合模型配置与模型校验结果（没有配置时 installed 为 false） |
| 22 | `colm-cli hybrid-remove <case-dir>` | 删掉 hybrid.toml 及它引用的 models/ 下的模型与标准化文件 |
| 23 | `colm-cli evaluation-catalog <case-dir> --obs <Flux.nc>` | 列出全部支持的评估变量及当前算例/观测是否可用 |
| 24 | `colm-cli evaluation-plan <case-dir> --obs <Flux.nc> --kernel <dir>` | 不依赖 history，按 case.nml + 内核预览可评估目标 |
| 25 | `colm-cli ws-create\|ws-note\|ws-list\|ws-status\|ws-search\|ws-read\|ws-symbols\|ws-patch\|ws-ls\|ws-file\|ws-write\|ws-snapshot\|ws-put\|ws-put-dir\|ws-rm\|ws-revert\|ws-build-engine\|ws-build-kernel\|` | 开发工作区：助手改源码、编译、测试、对照、登记实验内核（用法见 ws_cmd.rs 头部） |
| 26 | `colm-cli history-subset <case-dir> --out <dir> [--vars f_a,f_b] [--from YYYY-MM] [--to YYYY-MM] [--compress N] [--list 1]` | 把 history 裁成只含所选变量与月份的压缩小文件（远程取回结果用，R5） |
| 27 | `colm-cli history-catalog <case-dir>` |  |
| 28 | `colm-cli history-map <case-dir> --var <name> --index <step>` | 格点变量某一时间步的整张图（大网格隔行隔列抽稀） |
| 29 | `colm-cli history-repos --history-root <dir> [--desktop-path P] [--upstream-path P]` | 双仓库注册表：路径/身份/默认分支/同步记录/历史覆盖（只读本地 git） |
| 30 | `colm-cli history-sync --history-root <dir> --repository desktop\|colm_upstream\|both [--offline]` | 初始化/更新应用管理的历史缓存；--offline 绝不 fetch/clone |
| 31 | `colm-cli history-track --history-root <dir> --repository selected\|desktop\|colm_upstream\|both` | 变量定位 + 修改历史 + 未提交 diff + vendor/上游对应关系（只读本地 git） |
| 32 | `colm-cli history-commit --history-root <dir> --repository desktop\|colm_upstream --sha <40 hex>` | 单个 commit 的详情与相关代码 diff |
| 33 | `colm-cli series  <case-dir> --vars f_rnet,f_fsena [--from UNIX] [--to UNIX]` |  |
| 34 | `colm-cli study-parameters` | 输出可采样专家参数元数据 |
| 35 | `colm-cli remote-mkdir --host <服务器> --path <现有父目录> --name <新目录名>` | 独占创建一个文件夹，不覆盖已有路径、不创建父目录 |
| 36 | `colm-cli remote-history-cleanup --host <服务器> --root <工作目录> --job <作业> --operation <运行> --expected-path <目录> [--whole-case 1] [--delete 1]` |  |
| 37 | `colm-cli remote-history-cleanup --host <服务器> --root <工作目录> --project 1 --expected-path <项目目录> [--delete 1]` |  |
| 38 | `colm-cli remote-read <运行目录> --command history-catalog\|series\|energy-summary [--vars …] [--from …] [--to …] [--max-points N]` |  |
| 39 | `colm-cli remote-engine-progress --host <服务器> --root <工作目录>` |  |
| 40 | `colm-cli remote-load --host <服务器>` | 只读：登录节点负载与内存、各分区占用、排队与自己的作业 默认预检；只清理身份已核实且已完成的独占服务器运行目录 |
| 41 | `colm-cli study-preflight <case-root> --spec study.json` | 只校验 Study，不创建目录 |
| 42 | `colm-cli study-create <case-root> --spec study.json` | 仅站点模式：创建不确定性分析/参数调优 Study，写采样设计 |
| 43 | `colm-cli study-status <study-dir>` | 输出 Study manifest 与成员状态 |
| 44 | `colm-cli study-run <study-dir> --kernel <dir> [--stream 1] [--preprocessors rust\|fortran]` | 串行运行尚未完成的成员算例 |
| 45 | `colm-cli study-export <study-dir> --out <dir>` | 导出 manifest、samples、status、report.md/html |
| 46 | `colm-cli study-apply-preview <study-dir> --member <id\|best>` | 只读预览最佳/指定候选会改哪些参数 |
| 47 | `colm-cli study-pause\|study-resume\|study-cancel <study-dir>` | 请求暂停派发、恢复派发或取消尚未开始任务 |
| 48 | `colm-cli study-finalize-cancel <study-dir> --pid <pid>` | GUI 杀死对应调度进程后持久化取消终态 |
| 49 | `colm-cli study-finalize-idle-cancel <study-dir>` | GUI 无活动登记时，确认调度器已退出后持久化取消终态 |
| 50 | `colm-cli study-retry <study-dir> [--include-review 1]` | 把失败成员（可选含待复核成员）重新放回队列 |
| 51 | `colm-cli study-apply <study-dir> --member <id\|best> --out <dir> [--name N]` | 将指定成员参数写入新的独立算例 |
| 52 | `colm-cli study-result <study-dir> --path <result.json>` |  |
| 53 | `colm-cli study-verified-side <study-dir>` | 只读返回单侧研究的版本、成绩与已核对的完整输入证据（schema 1 JSON） |
| 54 | `colm-cli study-verify-comparison <study-dir> [--member <id>[,<id>...]]` | 只读核验本地研究证据，供远程比较检查使用（schema 1 JSON）； --member 让指定成员也按 best/baseline 的标准核验原始输出 读取并校验单个成员结果 |
| 55 | `colm-cli all     --site <site.nc> --out <dir> --kernel <dir> [--obs <Flux.nc>] [--name N]` |  |
| 56 | `colm-cli observation-table-probe <obs.csv\|txt\|tsv> [--json 1]` | 探测可选验证数据，自动识别站点/时间/评估变量 |
| 57 | `colm-cli observation-table-convert <obs.csv\|txt\|tsv> <Observation-dir>` | 输出 <site>_Flux.nc；已有同名文件默认拒绝，--overwrite 1 才覆盖；缺 QC 时有限值=0/缺失=1 |
| 58 | `colm-cli forcing-probe   <met.nc> [--json 1]` |  |
| 59 | `colm-cli forcing-table-probe <forcing.csv\|txt\|tsv> [--json 1]` | 自动识别分隔符、站点/时间/经纬度列与变量候选 |
| 60 | `colm-cli forcing-table-convert <src.csv\|txt\|tsv> <Forcing-dir>` | 按站点拆分到 .colm-tabular，缺失整行时间补成缺测值 |
| 61 | `colm-cli netcdf-probe    <data.nc> [--json 1]` | 探测一份强迫场文件：八个槽位各猜到了什么变量， 猜不到就是 null；三个观测高度缺失时也是 null， 不是 NaN —— GUI 前处理页据此决定问不问用户 |
| 62 | `colm-cli mesh-new --out <mesh.nc> --nlon N --nlat N [--grid-kind latlon\|unstructured]` | 生成 GRIDBASED landmask 或 int64 UNSTRUCTURED elmindex；预检也接受既有 int32 网格；无 bbox/SHP 时为全球 |
| 63 | `colm-cli spatial-preflight --grid-kind latlon\|unstructured\|catchment --input <mesh.nc>` | 在启动 CoLM 前校验空间文件字段并记录 sha256 |
| 64 | `colm-cli spatial-new --grid-kind latlon\|unstructured\|catchment --mesh <mesh.nc>` | 空间算例暂不支持参数调优和不确定性分析 创建不含 SITE_* 的空间算例；latlon 需要 dlon/dlat |
| 65 | `colm-cli forcing-convert <src.nc> <dst.nc> [--slot N=name:units[+extra] ...] [--height V,T,Q]` | 供 GUI 走 sidecar 调用；没给 --slot 的槽位走自动匹配 |
| 66 | `colm-cli forcing-gap-probe <src.nc> [--slot N=name:units[+extra] ...]` | 只诊断缺口与时区，不修改源文件 |
| 67 | `colm-cli forcing-repair <src.nc> <dst.nc> [--slot N=name:units[+extra] ...]` | 写一份带逐时 QC 的修复中间文件，原文件不动 |
| 68 | `colm-cli era5land-download <dst-dir> --lat LAT --lon LON --start YYYY-MM-DD --end YYYY-MM-DD` | 用本机 Python cdsapi 和 ~/.cdsapirc 下载对应 0.1° 格点 |

## C.2 完整 `--help` 原文

```text
usage:
  colm-cli scan    --dir <Sitedata 目录> [--forcing-dir <Forcing 目录>]
                   [--out sites.json] [--quick 1]
                   # 列出目录下的站点；--quick 跳过强迫场，只读站点文件
  colm-cli site-new --out <site.nc> --lon <度> --lat <度> [--landtype N] [--crop 1] [--overwrite 1]
                   [--rawdata <dir>]
                   [--mode igbp|usgs|pft|pc|urban|urban-igbp|urban-usgs|urban-pft|urban-pc] [--json 1]
                   # 建一份站点文件：经纬度必给，其余从 rawdata 抽或用
                   # 标称假设。--landtype 不给就不写，让 CoLM 回落
  colm-cli site-pfts <site.nc> [--crop 1] [--landtype N]
  colm-cli case-land-classes <case-dir>     # land classes present in a gridded case's landdata (JSON)
                   # 输出 PFT/PC 站点的非零 PFT 类型与归一化比例
  colm-cli new     --site <site.nc> --out <dir> [--name N] [--start Y-M-D] [--end Y-M-D]
                   [--met <Met.nc>]   # 前处理转出来的强迫场；不给就按命名约定
                                      # 在 ../Forcing/ 下找，那两套约定只覆盖
                                      # PLUMBER2 与 Urban-PLUMBER
                   [--spinup-years N] [--spinup-repeat N]   (默认 1 年 x 1 遍)
                   [--mode igbp|usgs|pft|pc|urban|urban-igbp|urban-usgs|urban-pft|urban-pc]
                   [--crop 1]   # CROP 内核：按 croptyp/pctcrop 审核农田站点
                   [--rawdata <dir>] [--runtime <dir>]
                   # 城市站点由文件形状自动识别。两个目录都可选：预抽表盖住的
                   # 21 个 Urban-PLUMBER 站不给也能跑，表外的站点才要 --rawdata
  colm-cli run     <case-dir> --kernel <dir> [--stream 1] [--force 1] [--ranks N]
                   [--stage mksrfdata|mkinidata|colm] [--preprocessors rust|fortran]
                   [--engine rust|fortran] [--record-input-fingerprints 0|1]
                   [--highres-params <dir>] [--soil-hyper-albedo-dir <dir>]
                   [--model-version official|<workspace>] [--workspace-root <dir>]
                   [--model-id ID --model-revision N --model-root DIR]
                   # --model-version 用一个工作区（修改版）自己编出的程序跑；跑完写 out/<名>/produced-by.json
                   # --force 不读取或写入指纹，三段全部重跑
                   # --stage 只运行指定阶段；与 --force 合用时强制重跑该阶段
                   # --record-input-fingerprints 1 记录文件指纹（非驱动输入/程序/输出；默认关闭；驱动数据始终跳过）
                   # --stream 把子进程每一行原样转发出来（GUI 用；终端下嫌吵）
                   # --ranks >1：Fortran 内核用 MPI 启动；Rust 引擎只在远程由 COLM_MPIEXEC（srun/mpiexec）启动，默认 1
                   # --preprocessors 选 mksrfdata/mkinidata 的实现，--engine 选 colm 主循环；
                   # 两者默认都是 rust。Rust 主循环覆盖 SinglePoint + LCT 土面 patch，
                   # 跑不了的算例会在第一段之前被预检拒绝，并提示改用 --engine fortran
                   # HYPERSPECTRAL PFT/PC：--highres-params 包含 fsds/、
                   # leaf_optical_properties/ 和 water_params.txt；--soil-hyper-albedo-dir
                   # 包含 colm_soil_albedo_400nm.nc ... colm_soil_albedo_2500nm.nc
  colm-cli model-list|prune|get|create|draft|save-draft|freeze|migrate-case|rollback|make-run|attach-network
                   --model-root DIR [--model-id ID --model-revision N] [--json JSON|-]
                   # create JSON: {name,note,parent:{id,revision},workspace?}; save-draft: ModelDraft + --expected-generation N
                   # freeze: --model-id ID --asset-root DIR [--keep-draft 1]; migrate-case/rollback: <case-dir>
                   # prune JSON: {keep:[{id,revision}],dry_run?}: drop unused old official revisions
                   # make-run: <case-dir> --out DIR --operation-id ID; attach-network: --model-id ID --expected-generation N + hybrid-install flags
  colm-cli model-code-ls|model-code-file --model-root DIR --version ID --revision N [--dir D | --path P]
                   # 只读：冻结版本的代码（修改版按冻结的那次提交读），列目录或读一个文件
  colm-cli model-changes --model-root DIR --version ID --revision N [--against ID --against-revision N]
                   # 只读：代码相对官方源码改了的文件，参数/物理方案/过程文件/AI 网络相对父版本（或 --against）的不同，来源研究
  colm-cli study-save-version <study-dir> --member best|ID --name NAME --model-root DIR [--note TEXT]
  colm-cli version-check --name N --kernel <dir> [--engine rust|fortran]
                   [--preprocessors rust|fortran] [--root <dir>]
  colm-cli produced-by <case-dir>
  colm-cli fingerprint-policy
                   # 查询驱动数据永久排除策略，无需读取算例或数据
  colm-cli full-input-identity <case-dir>
                   # 只读核验配置与非驱动输入的 SHA256 内容身份（JSON；驱动数据始终跳过）
  colm-cli metrics <case-dir> --obs <Flux.nc> [--spinup N] [--from UNIX] [--to UNIX]
                   [--json 1] [--corrected 1]
                   --corrected: 拿能量闭合订正后的观测比（Qle_cor / Qh_cor）
                   --summary-only: 只返回指标，不携带绘图用配对点
                   --obs-label-position: 观测标签=区间起点还是终点（缺省按文件属性识别）
                   --history-freq: 只作交叉核对；实际输出频率由 history 标签反推，不符时按 history 并告警
                   --min-coverage: 日/月/年聚合要求的最小有效观测覆盖率（缺省 0.5）
                   --pairs-var: 只评估指定观测变量；可重复给出
                   --max-points: 配对点保极值降采样上限（指标仍用完整样本）
  colm-cli hybrid-install <case-dir> --model <x.onnx|x.mlp.json> --slot pft|land_class
                   --features a,b,c --output NAME[:lo:hi[:transform]] [--output ...]
                   [--normalize <norm.json>] [--force 1]
                   # 把训练好的模型装到算例上：拷进 models/、写 hybrid.toml（含 sha256）。
                   # transform: identity|clamp|sigmoid|softplus；见 docs/design-hybrid.md
  colm-cli hybrid-check <case-dir> --kernel <dir>
                   # 加载模型、取特征并推理，打印各插槽的行数与特征/输出范围（JSON），不模拟。
                   # 需要算例已做完前处理（mkinidata）；单点与空间算例都支持
  colm-cli hybrid-climate <case-dir> --kernel <dir>
                   # 在运行时段内累积强迫，给每个 patch 算气候特征（clim_tair、clim_tair_amplitude、
                   # clim_prec、clim_swdown、clim_vpd），写到 <case>/hybrid_climate/；不模拟
  colm-cli hybrid-fit --studies <dir,dir,...> --network <net.json> --kernel <dir> --out <x.mlp.json>
                   [--weight pftfrac] [--epochs N] [--learning-rate X] [--ridge X] [--seed N] [--require-gate 1]
                   # 两步训练的第二步：读各站调优 Study 的最优参数，在基础算例上取逐行特征，拟合网络；
                   # 两个以上 Study 时做留一站交叉验证，并与“其余站均值”基准比较；不如基准时警告，
                   # 加 --require-gate 1 时报错且不写模型。net.json 与 Study 的 hybrid 段同格式
  colm-cli hybrid-studies <dir> [--depth N]
                   # 列出目录下（默认最多 4 层）的调优 Study：状态、站点、率定的参数，供两步法挑选
  colm-cli hybrid-info <case-dir>
                   # 以 JSON 列出算例的混合模型配置与模型校验结果（没有配置时 installed 为 false）
  colm-cli hybrid-remove <case-dir>
                   # 删掉 hybrid.toml 及它引用的 models/ 下的模型与标准化文件
  colm-cli evaluation-catalog <case-dir> --obs <Flux.nc>
                   # 列出全部支持的评估变量及当前算例/观测是否可用
  colm-cli evaluation-plan <case-dir> --obs <Flux.nc> --kernel <dir>
                   # 不依赖 history，按 case.nml + 内核预览可评估目标
  colm-cli ws-create|ws-note|ws-list|ws-status|ws-search|ws-read|ws-symbols|ws-patch|ws-ls|ws-file|ws-write|ws-snapshot|ws-put|ws-put-dir|ws-rm|ws-revert|ws-build-engine|ws-build-kernel|
                    ws-test|ws-run|ws-compare|ws-parity|ws-regress|ws-kernels|ws-export|ws-delete ...
                    # 开发工作区：助手改源码、编译、测试、对照、登记实验内核（用法见 ws_cmd.rs 头部）
  colm-cli history-subset <case-dir> --out <dir> [--vars f_a,f_b] [--from YYYY-MM] [--to YYYY-MM] [--compress N] [--list 1]
                    # 把 history 裁成只含所选变量与月份的压缩小文件（远程取回结果用，R5）
  colm-cli history-catalog <case-dir>
  colm-cli history-map <case-dir> --var <name> --index <step>
                   # 格点变量某一时间步的整张图（大网格隔行隔列抽稀）
  colm-cli history-repos --history-root <dir> [--desktop-path P] [--upstream-path P]
                   # 双仓库注册表：路径/身份/默认分支/同步记录/历史覆盖（只读本地 git）
  colm-cli history-sync --history-root <dir> --repository desktop|colm_upstream|both [--offline]
                   # 初始化/更新应用管理的历史缓存；--offline 绝不 fetch/clone
  colm-cli history-track --history-root <dir> --repository selected|desktop|colm_upstream|both
                   --symbol <symbol> [--source <dir>] [--path P] [--ref R] [--limit N] [--offset N]
                   [--model-json JSON] [--desktop-path P] [--upstream-path P]
                   # 变量定位 + 修改历史 + 未提交 diff + vendor/上游对应关系（只读本地 git）
  colm-cli history-commit --history-root <dir> --repository desktop|colm_upstream --sha <40 hex>
                   [--symbol S] [--path P] [--desktop-path P] [--upstream-path P]
                   # 单个 commit 的详情与相关代码 diff
  colm-cli series  <case-dir> --vars f_rnet,f_fsena [--from UNIX] [--to UNIX]
                   [--max-points N] [--out series.json]
  colm-cli study-parameters
                   # 输出可采样专家参数元数据
  colm-cli remote-mkdir --host <服务器> --path <现有父目录> --name <新目录名>
                   # 独占创建一个文件夹，不覆盖已有路径、不创建父目录
  colm-cli remote-history-cleanup --host <服务器> --root <工作目录> --job <作业> --operation <运行> --expected-path <目录> [--whole-case 1] [--delete 1]
  colm-cli remote-history-cleanup --host <服务器> --root <工作目录> --project 1 --expected-path <项目目录> [--delete 1]
  colm-cli remote-read <运行目录> --command history-catalog|series|energy-summary [--vars …] [--from …] [--to …] [--max-points N]
  colm-cli remote-engine-progress --host <服务器> --root <工作目录>
  colm-cli remote-load --host <服务器>
                   # 只读：登录节点负载与内存、各分区占用、排队与自己的作业
                   # 默认预检；只清理身份已核实且已完成的独占服务器运行目录
  colm-cli study-preflight <case-root> --spec study.json
                   # 只校验 Study，不创建目录
  colm-cli study-create <case-root> --spec study.json
                   # 仅站点模式：创建不确定性分析/参数调优 Study，写采样设计
  colm-cli study-status <study-dir>
                   # 输出 Study manifest 与成员状态
  colm-cli study-run <study-dir> --kernel <dir> [--stream 1] [--preprocessors rust|fortran]
                   [--engine rust|fortran]
                   # 串行运行尚未完成的成员算例
  colm-cli study-export <study-dir> --out <dir>
                   # 导出 manifest、samples、status、report.md/html
  colm-cli study-apply-preview <study-dir> --member <id|best>
                   # 只读预览最佳/指定候选会改哪些参数
  colm-cli study-pause|study-resume|study-cancel <study-dir>
                   # 请求暂停派发、恢复派发或取消尚未开始任务
  colm-cli study-finalize-cancel <study-dir> --pid <pid>
                   # GUI 杀死对应调度进程后持久化取消终态
  colm-cli study-finalize-idle-cancel <study-dir>
                   # GUI 无活动登记时，确认调度器已退出后持久化取消终态
  colm-cli study-retry <study-dir> [--include-review 1]
                   # 把失败成员（可选含待复核成员）重新放回队列
  colm-cli study-apply <study-dir> --member <id|best> --out <dir> [--name N]
                   # 将指定成员参数写入新的独立算例
  colm-cli study-result <study-dir> --path <result.json>
  colm-cli study-verified-side <study-dir>
                   # 只读返回单侧研究的版本、成绩与已核对的完整输入证据（schema 1 JSON）
  colm-cli study-verify-comparison <study-dir> [--member <id>[,<id>...]]
                   # 只读核验本地研究证据，供远程比较检查使用（schema 1 JSON）；
                   # --member 让指定成员也按 best/baseline 的标准核验原始输出
                   # 读取并校验单个成员结果
  colm-cli all     --site <site.nc> --out <dir> --kernel <dir> [--obs <Flux.nc>] [--name N]
                   [--start Y-M-D] [--end Y-M-D] [--spinup N]
                   [--preprocessors rust|fortran] [--engine rust|fortran]
  colm-cli observation-table-probe <obs.csv|txt|tsv> [--json 1]
                           # 探测可选验证数据，自动识别站点/时间/评估变量
  colm-cli observation-table-convert <obs.csv|txt|tsv> <Observation-dir>
                           --time-column COLUMN [--site-column COLUMN|--site-name NAME]
                           [--variable VAR=column[:qc_column] ...] [--json 1] [--overwrite 1]
                           # 输出 <site>_Flux.nc；已有同名文件默认拒绝，--overwrite 1 才覆盖；缺 QC 时有限值=0/缺失=1
  colm-cli forcing-probe   <met.nc> [--json 1]
  colm-cli forcing-table-probe <forcing.csv|txt|tsv> [--json 1]
                           # 自动识别分隔符、站点/时间/经纬度列与变量候选
  colm-cli forcing-table-convert <src.csv|txt|tsv> <Forcing-dir>
                           --time COLUMN [--site COLUMN]
                           [--lat-column COLUMN --lon-column COLUMN]
                           [--landtype-column COLUMN] [--offset-column COLUMN]
                           [--land-cover-scheme IGBP|USGS|PFT|PC|Urban]
                           [--lat LAT --lon LON] [--utc-offset HOURS]
                           [--step-seconds N] [--height V,T,Q]
                           --slot N=column:units[+extra] ... [--json 1]
                           # 按站点拆分到 .colm-tabular，缺失整行时间补成缺测值
  colm-cli netcdf-probe    <data.nc> [--json 1]
                           # 探测一份强迫场文件：八个槽位各猜到了什么变量，
                           # 猜不到就是 null；三个观测高度缺失时也是 null，
                           # 不是 NaN —— GUI 前处理页据此决定问不问用户
  colm-cli mesh-new --out <mesh.nc> --nlon N --nlat N [--grid-kind latlon|unstructured]
                    [--west W --east E --south S --north N | --shp basin.shp]
                    [--non-ocean-mask mask.nc --non-ocean-var non_ocean_mask]
                    # 生成 GRIDBASED landmask 或 int64 UNSTRUCTURED elmindex；预检也接受既有 int32 网格；无 bbox/SHP 时为全球
  colm-cli spatial-preflight --grid-kind latlon|unstructured|catchment --input <mesh.nc>
                    [--out manifest.json]
                    # 在启动 CoLM 前校验空间文件字段并记录 sha256
  colm-cli spatial-new --grid-kind latlon|unstructured|catchment --mesh <mesh.nc>
                    # 空间算例暂不支持参数调优和不确定性分析
                    --out <case-dir> --forcing <forcing.nml> --rawdata <dir> --runtime <dir>
                    --start YYYY-MM-DD --end YYYY-MM-DD --timestep <seconds>
                    [--name N] [--dlon degrees --dlat degrees] [--mode igbp|usgs|pft|pc]
                    # 创建不含 SITE_* 的空间算例；latlon 需要 dlon/dlat
  colm-cli forcing-convert <src.nc> <dst.nc> [--slot N=name:units[+extra] ...] [--height V,T,Q]
                           # 供 GUI 走 sidecar 调用；没给 --slot 的槽位走自动匹配
  colm-cli forcing-gap-probe <src.nc> [--slot N=name:units[+extra] ...]
                           [--short-gap N] [--utc-offset HOURS] [--lat LAT --lon LON] [--json 1]
                           # 只诊断缺口与时区，不修改源文件
  colm-cli forcing-repair <src.nc> <dst.nc> [--slot N=name:units[+extra] ...]
                           [--short-gap N] [--utc-offset HOURS] [--lat LAT --lon LON]
                           [--era5 FILE_OR_DIR] [--min-overlap N] [--json 1]
                           # 写一份带逐时 QC 的修复中间文件，原文件不动
  colm-cli era5land-download <dst-dir> --lat LAT --lon LON --start YYYY-MM-DD --end YYYY-MM-DD
                           # 用本机 Python cdsapi 和 ~/.cdsapirc 下载对应 0.1° 格点
```