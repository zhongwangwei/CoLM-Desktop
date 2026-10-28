//! 比较页在服务器上评估（OpenBench）：格点算例的 history 往往就在服务器上，拉回本机评估不现实。
//!
//! ```text
//! colm-cli remote-bench-status     --host H --root R
//! colm-cli remote-bench-install    --host H --root R --step uv|python|openbench [--source 服务器上的源码目录]
//!                                  [--index-url PyPI 镜像]
//! colm-cli remote-bench-run        --host H --root R --json '{"name","config","sites","ref_root"}' [调度选项同 remote-run]
//! colm-cli remote-bench-job        --host H --root R --job ID [--lines N]
//! colm-cli remote-bench-uninstall  --host H --root R --what env|all
//! colm-cli remote-bench-cancel     --host H --root R --job ID
//! colm-cli remote-bench-inspect    --host H --root R --dir 服务器上的 history 或算例目录
//! colm-cli remote-bench-metric-options --host H --root R
//! colm-cli remote-bench-references --host H --root R
//! colm-cli remote-bench-register   --host H --root R --json '{"name","root_dir","data_type",…}'
//! colm-cli remote-bench-register-model-variable --host H --root R --json '{"model","variable","varname","unit"}'
//! colm-cli remote-bench-scan       --host H --root R --dir 服务器上的参考数据目录
//! colm-cli remote-bench-fetch      --host H --root R --name N --out 本机目录 [--run 作业号]
//! ```
//!
//! 比较目录里每次运行一个冻结快照 `<R>/openbench/projects/<名字>/runs/<作业号>/`（配置、站点/输入
//! 清单、meta 与 output 都在里面），成功才原子更新 `latest-success.json`；`remote-bench-fetch` 默认
//! 取回它指向的那次运行，`--run` 指定取回某次。旧目录没有 `runs/`，取回仍按项目根 `output/` 兼容。
//!
//! 环境装在 `<R>/openbench/`，布局、固定版本、校验和、环境变量与两段 Python 脚本都取自 `colm-bench`，
//! 和本机那份一模一样。装环境和跑评估是服务器上的作业（`<R>/jobs/<作业号>/`，与模拟作业同一套）：关掉应用
//! 也能接着看日志、取消。装环境一律直接在这台机器上跑（集群的计算节点多半不能上网）；评估按服务器设置的
//! 调度系统提交。读 history、读登记表、登记、扫描是几秒钟的事，直接经 ssh 跑。

use anyhow::{bail, ensure, Context, Result};
use colm_bench::{
    platform_from_uname, posix_value, strict_json, uv_archive, valid_name, valid_station,
    INSPECT_SCRIPT, METRIC_SCRIPT, OPENBENCH_VERSION, PREFETCH_SCRIPT, PREPARE_SCRIPT, PROGRAM_ENV,
    PYTHON_VERSION, UV_ENV, UV_VERSION,
};
use colm_remote::job::{self, Spec};
use colm_remote::sched::{self, Resources, Scheduler};
use colm_remote::ssh::{quote, Ssh};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use super::Opts;

/// 服务器上的环境根目录。
fn bench_root(opts: &Opts) -> Result<String> {
    Ok(format!("{}/openbench", super::remote_cmd::root_of(opts)?))
}

/// 服务器上的绝对路径：拼进脚本前挡住控制字符。
fn server_path(path: &str, what: &str) -> Result<()> {
    ensure!(
        path.starts_with('/') && !path.chars().any(char::is_control),
        "{what} must be an absolute server path without control characters, got {path:?}"
    );
    Ok(())
}

/// `export` 一组环境变量（取值都按 `quote` 转义）。
fn exports(root: &str, table: &[(&str, colm_bench::EnvValue)]) -> String {
    table
        .iter()
        .map(|(key, value)| format!("export {key}={}\n", quote(&posix_value(root, *value))))
        .collect()
}

fn program_env(root: &str) -> String {
    exports(root, &PROGRAM_ENV)
}

/// 专用环境里的程序，带上 [`PROGRAM_ENV`]。
fn env_program(root: &str, name: &str) -> String {
    quote(&format!("{root}/env/bin/{name}"))
}

/// 在服务器上跑一段 Python（脚本经 here-document 传，参数各自转义）。
fn python_here(root: &str, script: &str, args: &[&str], tag: &str) -> String {
    let args = args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ");
    format!(
        "{} - {args} <<'{tag}'\n{}\n{tag}\n",
        env_program(root, "python"),
        script.trim_matches('\n')
    )
}

fn unique_job(kind: &str) -> Result<String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    Ok(format!("bench-{kind}-{}-{nanos}", std::process::id()))
}

/// `remote-bench-status`：装到了哪一步、有哪些比较、服务器有没有 curl 与 sha256sum。
pub(super) fn status(opts: &Opts) -> Result<()> {
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let root = bench_root(opts)?;
    let text = ssh.run_ok(&status_script(&root))?;
    println!("{}", parse_status(&root, &text));
    Ok(())
}

/// 状态脚本（经 ssh 跑，输出由 [`parse_status`] 解析）。抽出来是为了能在本地 bash 沙箱里真跑：
/// 新式比较的报告只在 `runs/<run_id>/output` 里，靠 `latest-success.json` 找到；旧的 `output/` 扫描
/// 只能给没有 `runs/` 的旧项目用，否则会把上一次运行的报告当成这一次的（C07）。
fn status_script(root: &str) -> String {
    format!(
        r#"OB={ob}
echo "uname=$(uname -s -m)"
echo "curl=$(command -v curl || command -v wget || true)"
echo "sha=$(command -v sha256sum || command -v shasum || true)"
if [ -x "$OB/tools/uv" ]; then echo "uv=$("$OB/tools/uv" --version 2>/dev/null | awk '{{print $2}}')"; fi
if [ -x "$OB/env/bin/python" ]; then
{env}echo "python=$("$OB/env/bin/python" --version 2>&1)"
echo "openbench=$("$OB/env/bin/python" -c 'import openbench; print(openbench.__version__)' 2>/dev/null)"
fi
for d in "$OB"/projects/*/; do
  [ -d "$d" ] || continue
  pname=$(basename "$d")
  report=
  if [ -d "$d/runs" ]; then
    rid=
    csha=
    fat=
    if [ -f "$d/latest-success.json" ]; then
      rid=$(sed -n 's/^ *"run_id": *"\([^"]*\)".*/\1/p' "$d/latest-success.json" | head -n 1)
      rel=$(sed -n 's/^ *"report": *"\([^"]*\)".*/\1/p' "$d/latest-success.json" | head -n 1)
      csha=$(sed -n 's/^ *"config_sha": *"\([^"]*\)".*/\1/p' "$d/latest-success.json" | head -n 1)
      fat=$(sed -n 's/^ *"finished_at": *"\([^"]*\)".*/\1/p' "$d/latest-success.json" | head -n 1)
      case "$rid" in ''|*[!A-Za-z0-9._-]*) rid=;; esac
      if [ -n "$rid" ] && [ -n "$rel" ] && [ -f "$d/runs/$rid/$rel" ]; then report="$d/runs/$rid/$rel"; fi
    fi
    echo "project=$pname|$report|$rid|$csha|$fat"
    continue
  fi
  report=$(ls "$d"output/*/reports/evaluation_report_standalone.html "$d"output/*/reports/evaluation_report.html 2>/dev/null | head -n 1)
  echo "project=$pname|$report"
done
true
"#,
        ob = quote(root),
        env = program_env(root),
    )
}

fn parse_status(root: &str, text: &str) -> serde_json::Value {
    let mut out = json!({
        "root": root,
        "platform": null,
        "downloader": null,
        "checksum": null,
        "uv_version": null,
        "python": null,
        "openbench": null,
        "uv_pinned": UV_VERSION,
        "openbench_pinned": OPENBENCH_VERSION,
        "projects": [],
    });
    let mut projects = Vec::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        let set = |out: &mut serde_json::Value, field: &str| {
            if !value.is_empty() {
                out[field] = value.into();
            }
        };
        match key {
            "uname" => {
                out["platform"] = platform_from_uname(value).into();
                out["uname"] = value.into();
            }
            "curl" => set(&mut out, "downloader"),
            "sha" => set(&mut out, "checksum"),
            "uv" => set(&mut out, "uv_version"),
            "python" => set(&mut out, "python"),
            "openbench" => set(&mut out, "openbench"),
            "project" => {
                // 旧式项目只有 `名字|报告`；新式项目在后面续上最新成功运行的身份。
                let mut fields = value.split('|');
                let name = fields.next().unwrap_or_default();
                let report = fields.next().unwrap_or_default();
                let run_id = fields.next().unwrap_or_default();
                let config_sha = fields.next().unwrap_or_default();
                let finished_at = fields.next().unwrap_or_default();
                let report = (!report.is_empty()).then_some(report);
                let mut entry = json!({ "name": name, "report": report });
                if !run_id.is_empty() {
                    entry["latest_success"] = json!({
                        "run_id": run_id,
                        "config_sha": config_sha,
                        "report": report,
                        "finished_at": (!finished_at.is_empty()).then_some(finished_at),
                    });
                }
                projects.push(entry);
            }
            _ => {}
        }
    }
    out["projects"] = projects.into();
    out
}

/// 装环境的一步，作为服务器上的作业提交；返回作业号。
pub(super) fn install(opts: &Opts) -> Result<()> {
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let root = bench_root(opts)?;
    let step = opts.need_str("--step")?;
    let body = match step.as_str() {
        "uv" => {
            let uname = ssh.run_ok("uname -s -m")?;
            let platform = platform_from_uname(&uname)
                .with_context(|| format!("no pinned uv for this server ({})", uname.trim()))?;
            let (archive, sha) = uv_archive(platform).context("no pinned uv archive")?;
            install_uv_body(&root, archive, sha)
        }
        "python" => format!(
            "{uv_env}{uv} venv --python {PYTHON_VERSION} --python-preference only-managed --allow-existing {env}\n",
            uv_env = exports(&root, &UV_ENV),
            uv = quote(&format!("{root}/tools/uv")),
            env = quote(&format!("{root}/env")),
        ),
        "openbench" => {
            let target = match opts.get("--source").filter(|s| !s.trim().is_empty()) {
                None => format!("colm-openbench=={OPENBENCH_VERSION}"),
                Some(dir) => {
                    server_path(&dir, "--source")?;
                    format!("-e {}", quote(&dir))
                }
            };
            let index = match opts.get("--index-url").filter(|u| !u.trim().is_empty()) {
                None => String::new(),
                Some(url) => {
                    ensure!(colm_bench::valid_index_url(&url), "bad package index {url:?}");
                    format!(" --default-index {}", quote(&url))
                }
            };
            format!(
                "test -x {python} || {{ echo '先装专用 Python' >&2; exit 1; }}\n{uv_env}{uv} pip install --python {python}{index} {target}\n{env}echo '预取地图底图'\n{prefetch}",
                python = quote(&format!("{root}/env/bin/python")),
                uv_env = exports(&root, &UV_ENV),
                uv = quote(&format!("{root}/tools/uv")),
                env = program_env(&root),
                prefetch = python_here(&root, PREFETCH_SCRIPT, &[], "COLM_BENCH_PREFETCH"),
            )
        }
        other => bail!("no such install step: {other}"),
    };
    let id = unique_job(&step)?;
    let spec = Spec {
        name: "colm-bench".into(),
        ..Spec::bare()
    };
    let submitted = job::submit_plain(
        &ssh,
        super::remote_cmd::root_of(opts)?.as_str(),
        &id,
        &body,
        &spec,
    )?;
    println!("{}", json!({ "job": id, "submitted": submitted }));
    Ok(())
}

/// 下载 uv、按写死的 SHA-256 核对、解压。日志行与本机的写法一致（前端按同一套文字译成英文）。
fn install_uv_body(root: &str, archive: &str, sha: &str) -> String {
    let url = format!("https://github.com/astral-sh/uv/releases/download/{UV_VERSION}/{archive}");
    format!(
        r#"mkdir -p {tools}
cd {tools}
echo "下载 {url}"
if command -v curl >/dev/null 2>&1; then curl -fsSL --retry 3 -o {archive} {url_q}; else wget -q -O {archive} {url_q}; fi
if command -v sha256sum >/dev/null 2>&1; then printf '%s  %s\n' {sha} {archive} | sha256sum -c -; else printf '%s  %s\n' {sha} {archive} | shasum -a 256 -c -; fi
echo "校验通过，解压"
rm -rf unpack && mkdir unpack
tar -xzf {archive} -C unpack
cp "$(find unpack -type f -name uv | head -n 1)" uv
chmod 755 uv
rm -rf unpack {archive}
echo "uv {UV_VERSION} 装好了"
"#,
        tools = quote(&format!("{root}/tools")),
        archive = quote(archive),
        url_q = quote(&url),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Site {
    id: String,
    station: String,
    history_dir: String,
    prefix: String,
    /// 站点经纬度（十进制度）：PREPARE 优先用它们写站点清单，缺省时回退到 history 文件里的坐标。
    /// 与 Tauri 侧 `BenchSite` 的契约一致：两个参数要么都给、要么都不给。
    #[serde(default)]
    lon: Option<f64>,
    #[serde(default)]
    lat: Option<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunRequest {
    name: String,
    config: String,
    #[serde(default)]
    sites: Vec<Site>,
    #[serde(default)]
    ref_root: Option<String>,
}

fn read_json<T: serde::de::DeserializeOwned>(opts: &Opts) -> Result<T> {
    let text = opts.need_str("--json")?;
    ensure!(text.len() <= 1024 * 1024, "request JSON exceeds 1 MiB");
    Ok(serde_json::from_str(&text)?)
}

// ---- 同名比较的活动作业保护（GUI-07 服务器侧） ----
//
// 客户端已经按"本机记着的作业身份"阻止了重复提交，但身份只存在发起提交的那台电脑上：换一台电脑、
// 手工清掉 localStorage、或两次提交交错时，服务器仍会被要求写同一个成果目录。这里在服务器侧的
// 比较目录里放一个原子占位（`mkdir` 只有一个进程能成功），提交前必须先占住它。
//
// C16：光有占位不够。旧实现的"cat 作业号后 rm -rf"在"核对"和"删除"之间留了一个窗口：另一个提交
// 可以把陈旧占位清掉、占住新的一位，旧作业收尾时再按旧窗口里的作业号删，就会删掉别人的新占位。
// 现在给比较目录加一把可移植的 `mkdir` 自旋锁（macOS 测试机没有 flock，也不引入 flock），所有会改
// `.active-job` 的路径 —— 占位、陈旧清理、提交失败释放、作业体 EXIT trap —— 都在锁内"核对所有权
// 再删"。锁只保护毫秒级的 cat/rm/写 intent；查作业状态要 SSH 往返，一律在锁外做。
//
// C09：提交前先把"提交意图"（预生成作业号、时间、配置哈希）写进占位，再发提交命令。SSH 在服务器
// 收下提交之后断线时，回执会丢；这时按作业号有界重查，确认作业真的存在就保留占位、按作业号继续
// 跟踪，只有连续多次明确查无记录才判未接受并释放占位。

/// 比较目录里的活动作业占位目录。
const ACTIVE_JOB: &str = ".active-job";
/// 占位旁边的一把 `mkdir` 自旋锁：持有者把"令牌 + 接管时间"写进 `owner`。
const ACTIVE_JOB_LOCK: &str = ".active-job.lock";
/// 锁等待轮数（每轮 50 ms，约 5 秒）。锁内只有几个毫秒级命令，等这么久说明持有者卡死或留了死锁。
const LOCK_TRIES: u32 = 100;
/// `owner` 里的接管时间超过这么多秒就当死锁回收。正常持锁远小于一秒，30 秒已经非常宽容。
const LOCK_STALE_SECONDS: u64 = 30;
/// 一次远程运行的身份目录与文件（与窗口进程 `bench_runs.rs` 的布局一致，取回时按同一套名字找）。
const RUNS_DIR: &str = "runs";
const RUN_CONFIG_FILE: &str = "openbench.yaml";
const RUN_META_FILE: &str = "meta.json";
const RUN_SITES_FILE: &str = "sites.json";
const RUN_INPUTS_FILE: &str = "inputs.json";
const LATEST_SUCCESS_FILE: &str = "latest-success.json";
/// 失败原因写进 meta 的上限（字符）：作业日志可能很长，meta 只用来诊断。
const RUN_ERROR_LIMIT: usize = 4_000;

fn placeholder_path(project: &str) -> String {
    format!("{project}/{ACTIVE_JOB}")
}

/// 固定错误原文：前端按"这个比较还有未完成的评估作业"前缀映射成双语提示，改字前先搜前端。
fn active_job_error(id: &str) -> String {
    format!(
        "这个比较还有未完成的评估作业（作业号 {id}）；先核对终态或停止它，避免重复写同一个成果目录。"
    )
}

/// 活动作业锁一直拿不到时的固定错误原文：前端按"这个比较的活动作业锁一直被占用"映射成双语提示。
fn lock_timeout_error() -> String {
    "这个比较的活动作业锁一直被占用，请稍后再提交。".to_string()
}

/// 服务器上一个比较的目录：`<openbench>/projects/<名字>`。
fn project_dir(root: &str, name: &str) -> Result<String> {
    ensure!(
        valid_name(name),
        "a comparison name may only use letters, digits, - and _: {name:?}"
    );
    Ok(format!("{root}/projects/{name}"))
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

/// UNIX 秒 → `2026-10-28T13:23:45Z`。窗口进程的 `bench_runs.rs` 有一份同样算法的实现：cli 是独立
/// 可执行文件，不为一个时间格式去依赖窗口进程；两处都只是民用历公式，改格式要一起改。
fn iso8601_utc(unix: u64) -> String {
    let days = (unix / 86_400) as i64;
    let seconds = unix % 86_400;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = (z - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = if month_part < 10 {
        month_part + 3
    } else {
        month_part - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds / 3_600,
        (seconds % 3_600) / 60,
        seconds % 60
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 配置没写 `project.output_dir` 时 OpenBench 的默认输出根（相对运行目录）。
const DEFAULT_OUTPUT_DIR: &str = "output";

/// 远程 run 的 `project.output_dir`：只允许本次运行目录内的相对路径（服务器是 POSIX，统一用 `/`）。
/// 配置由窗口进程生成/校验，这里再挡一次：CLI 也能被直接调用，不能靠调用方保证。
fn output_dir_from_config(config: &str) -> Result<String> {
    let mut in_project = false;
    let mut seen_project = false;
    let mut project_child_indent: Option<usize> = None;
    let mut found: Option<String> = None;
    for (index, raw) in config.lines().enumerate() {
        let line = strip_yaml_comment(raw.trim_end_matches('\r'));
        if line.trim().is_empty() {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim();
        // 多文档 YAML 里 OpenBench 只读第一份，而我们会把每份的 project 都扫进来 —— 不猜。
        if trimmed == "---" || trimmed == "..." {
            bail!(
                "project.output_dir 不合格（配置第 {} 行）：不支持多文档 YAML",
                index + 1
            );
        }
        let Some((key, value)) = split_mapping(trimmed) else {
            if trimmed.contains("output_dir") {
                bail!(
                    "project.output_dir 不合格（配置第 {} 行）：无法识别的写法",
                    index + 1
                );
            }
            continue;
        };
        if indent == 0 {
            if key == "project" {
                if seen_project {
                    bail!("project.output_dir 不合格：project 段只能有一个");
                }
                seen_project = true;
                // 内联 `project: {output_dir: /tmp}` 会绕过逐行检查，而 OpenBench 仍会读它。
                if !value.trim().is_empty() {
                    bail!(
                        "project.output_dir 不合格（配置第 {} 行）：project 段必须用块写法",
                        index + 1
                    );
                }
                in_project = true;
                project_child_indent = None;
                continue;
            }
            in_project = false;
            project_child_indent = None;
            if key == "output_dir" {
                bail!("project.output_dir 必须写在 project 段下面");
            }
            continue;
        }
        if !in_project {
            if key == "output_dir" {
                bail!("project.output_dir 必须写在 project 段下面");
            }
            continue;
        }
        // `<<` 合并键能把别处的 output_dir 注进 project，逐行检查看不见；OpenBench 却会展开它。
        if key == "<<" {
            bail!("project.output_dir 不合格：不支持 YAML 的 `<<` 合并键");
        }
        let child_indent = *project_child_indent.get_or_insert(indent);
        if key != "output_dir" {
            if trimmed.contains("output_dir") {
                bail!(
                    "project.output_dir 不合格（配置第 {} 行）：无法识别的写法",
                    index + 1
                );
            }
            continue;
        }
        if indent != child_indent {
            bail!("project.output_dir 只能是 project 的直属键");
        }
        if found.is_some() {
            bail!("project.output_dir 只能写一次");
        }
        found = Some(
            match yaml_scalar(value)
                .with_context(|| format!("project.output_dir 不合格（配置第 {} 行）", index + 1))?
            {
                None => DEFAULT_OUTPUT_DIR.to_owned(),
                Some(raw) => normalize_output_dir(&raw).with_context(|| {
                    format!("project.output_dir 不合格（配置第 {} 行）", index + 1)
                })?,
            },
        );
    }
    Ok(found.unwrap_or_else(|| DEFAULT_OUTPUT_DIR.to_owned()))
}

fn normalize_output_dir(raw: &str) -> Result<String> {
    let text = raw.trim();
    ensure!(!text.is_empty(), "output_dir 不能是空路径");
    ensure!(!text.contains('\0'), "output_dir 不能含 NUL 字符");
    // `~` 一旦被对面的路径库展开就跑到运行目录外；这里只接受字面相对目录名。
    ensure!(
        !text.starts_with('~'),
        "output_dir 不支持 ~ 展开（{text}），请写运行目录内的普通相对路径"
    );
    ensure!(
        !text.starts_with('/'),
        "output_dir 不能是绝对路径（{text}），请改用运行目录内的相对路径"
    );
    let mut parts: Vec<&str> = Vec::new();
    for part in text.split('/') {
        match part {
            "" | "." => {}
            ".." => ensure!(
                parts.pop().is_some(),
                "output_dir 不能跳出运行目录（{text}），请改用运行目录内的相对路径"
            ),
            other => {
                ensure!(
                    !other.contains('\\'),
                    "服务器上的 output_dir 只用 / 分隔（{text}）"
                );
                ensure!(
                    !other.chars().any(char::is_control),
                    "output_dir 不能含控制字符"
                );
                parts.push(other);
            }
        }
    }
    ensure!(
        !parts.is_empty(),
        "output_dir 不能是运行目录本身（{text}），请指定一个子目录（例如 output）"
    );
    Ok(parts.join("/"))
}

/// 去掉 YAML 行尾注释：`#` 只有不在引号里、且前面是空白（或行首）时才是注释。
fn strip_yaml_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut quote = None;
    for (index, byte) in bytes.iter().copied().enumerate() {
        match quote {
            Some(current) if byte == current => quote = None,
            Some(_) => {}
            None if byte == b'"' || byte == b'\'' => quote = Some(byte),
            None if byte == b'#' && (index == 0 || bytes[index - 1].is_ascii_whitespace()) => {
                return &line[..index];
            }
            None => {}
        }
    }
    line
}

/// 拆一行 `键: 值`；冒号须后跟空白或行尾，引号里的冒号跳过。
fn split_mapping(line: &str) -> Option<(&str, &str)> {
    let bytes = line.as_bytes();
    let mut quote = None;
    for (index, byte) in bytes.iter().copied().enumerate() {
        match quote {
            Some(current) if byte == current => quote = None,
            Some(_) => {}
            None if byte == b'"' || byte == b'\'' => quote = Some(byte),
            None if byte == b':' => {
                let next = bytes.get(index + 1).copied();
                if next.is_none() || next.is_some_and(|c| c.is_ascii_whitespace()) {
                    let key = line[..index].trim();
                    return Some((strip_key_quotes(key), line[index + 1..].trim()));
                }
            }
            None => {}
        }
    }
    None
}

fn strip_key_quotes(key: &str) -> &str {
    let bytes = key.as_bytes();
    if key.len() >= 2
        && ((bytes[0] == b'"' && bytes[key.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[key.len() - 1] == b'\''))
    {
        &key[1..key.len() - 1]
    } else {
        key
    }
}

/// YAML 标量 → 字符串；`None` 表示 null/空值。流式/块式/锚点写法不猜（可能藏绝对路径）。
fn yaml_scalar(value: &str) -> Result<Option<String>> {
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("~") || value.eq_ignore_ascii_case("null") {
        return Ok(None);
    }
    match value.chars().next().expect("non-empty") {
        '"' => serde_json::from_str::<String>(value)
            .map(Some)
            .map_err(|error| anyhow::anyhow!("双引号字符串读不懂（{error}）")),
        '\'' => {
            ensure!(
                value.len() >= 2 && value.ends_with('\''),
                "单引号字符串没有收尾"
            );
            let inner = &value[1..value.len() - 1];
            ensure!(
                !inner.replace("''", "").contains('\''),
                "单引号字符串里的引号要写成两个单引号"
            );
            Ok(Some(inner.replace("''", "'")))
        }
        '[' | '{' | '&' | '*' | '!' | '|' | '>' => {
            bail!("不支持 YAML 的流式/块式/锚点写法，请写成普通字符串")
        }
        _ => Ok(Some(value.to_owned())),
    }
}

/// 占位锁的 shell 函数：`_colm_lock_acquire` 成功返回 0、超时返回 1；`_colm_lock_release` 只释放
/// 自己令牌的锁。抽成函数是为了让"占位/清理/释放"三段脚本共用同一份实现。
///
/// 为什么是 `mkdir` 自旋锁：macOS 自带 bash 没有 `flock`，测试机与真实服务器也可能一个 macOS 一个
/// Linux；`mkdir` 是所有 POSIX shell 都有的原子操作，同一个路径只有一个进程能建成。
///
/// 死锁回收：`owner` 第一行是令牌、第二行是接管时的 unix 秒。超过 [`LOCK_STALE_SECONDS`] 的锁由
/// "回收标记目录"（`mkdir`，同样原子）串行化：只有持有回收标记的进程能删锁，删前再读一次令牌与
/// 时间 —— 否则两个进程同时判陈旧，会把对方刚建的新锁也删掉。owner 里还没有令牌（刚 mkdir、还没
/// 写 owner 的窗口）时按目录 mtime 判断，只回收超过一分钟的，避免误删并发进程刚建的空锁。
fn lock_helpers(project: &str, token: &str) -> String {
    let lock = quote(&format!("{project}/{ACTIVE_JOB_LOCK}"));
    let reclaim = quote(&format!("{project}/{ACTIVE_JOB_LOCK}.reclaim"));
    let owner = quote(&format!("{project}/{ACTIVE_JOB_LOCK}/owner"));
    format!(
        r#"_colm_lock_acquire() {{
  _colm_try=0
  while [ "$_colm_try" -lt {tries} ]; do
    if mkdir {lock} 2>/dev/null; then
      printf '%s\n%s\n' {token} "$(date +%s)" > {owner} 2>/dev/null
      if [ "$(sed -n 1p {owner} 2>/dev/null)" = {token} ]; then return 0; fi
    fi
    if mkdir {reclaim} 2>/dev/null; then
      _colm_owner=$(sed -n 1p {owner} 2>/dev/null)
      _colm_stamp=$(sed -n 2p {owner} 2>/dev/null)
      case "$_colm_stamp" in ''|*[!0-9]*) _colm_stamp=0;; esac
      if [ -z "$_colm_owner" ]; then
        if find {lock} -maxdepth 0 -mmin +0 2>/dev/null | grep -q .; then rm -rf {lock}; fi
      elif [ "$(( $(date +%s) - _colm_stamp ))" -gt {stale} ]; then
        rm -rf {lock}
      fi
      rm -rf {reclaim}
    elif find {reclaim} -maxdepth 0 -mmin +0 2>/dev/null | grep -q .; then
      rm -rf {reclaim}
    fi
    _colm_try=$((_colm_try + 1))
    sleep 0.05
  done
  return 1
}}
_colm_lock_release() {{
  if [ "$(sed -n 1p {owner} 2>/dev/null)" = {token} ]; then rm -rf {lock}; fi
}}
"#,
        tries = LOCK_TRIES,
        stale = LOCK_STALE_SECONDS,
    )
}

/// 提交意图（写进占位里的 `intent`）：C09 提交回执丢失时，凭预生成的作业号与时间在宽限内核对。
/// 用 `serde_json` 的 pretty 输出是有意的：清理/释放脚本用 `sed -n 's/^ *"id": *"..."'` 读它，
/// 一行一个字段才不会被转义后的引号搞错。
fn submit_intent(id: &str, created_unix: u64, config_sha: &str) -> String {
    let value = json!({
        "id": id,
        "created_unix": created_unix,
        "config_sha": config_sha,
    });
    serde_json::to_string_pretty(&value).expect("a JSON value always serialises")
}

/// 原子占住比较目录：`mkdir` 成功才算占住。已有占位时输出 `busy <作业号>`（号来自 `id` 或 `intent`），
/// 锁拿不到时输出 `lock_timeout`，占位竞争失败输出 `lost`。拿到锁之后"读旧号 → 清理 → 建新占位 →
/// 写 id/intent → 读回核对"一气呵成；写号后要读回核对，与陈旧回收交错时两方至多一方报 acquired。
fn acquire_script(project: &str, id: &str, intent: &str) -> String {
    let placeholder = quote(&placeholder_path(project));
    format!(
        r#"mkdir -p {project} || {{ echo 'cannot create the comparison folder' >&2; exit 1; }}
{helpers}if ! _colm_lock_acquire; then echo lock_timeout; exit 0; fi
_colm_existing=$(cat {placeholder}/id 2>/dev/null)
if [ -z "$_colm_existing" ]; then
  _colm_existing=$(sed -n 's/^ *"id": *"\([^"]*\)".*/\1/p' {placeholder}/intent 2>/dev/null | head -n 1)
fi
if [ -n "$_colm_existing" ]; then
  _colm_lock_release
  printf 'busy %s\n' "$_colm_existing"
  exit 0
fi
rm -rf {placeholder}
if ! mkdir {placeholder} 2>/dev/null; then _colm_lock_release; echo lost; exit 0; fi
printf '%s\n' {id} > {placeholder}/id 2>/dev/null
printf '%s' {intent_q} > {placeholder}/intent 2>/dev/null
if [ "$(cat {placeholder}/id 2>/dev/null)" = {id} ]; then
  _colm_lock_release
  echo acquired
else
  _colm_lock_release
  echo lost
fi
"#,
        project = quote(project),
        helpers = lock_helpers(project, id),
        id = quote(id),
        intent_q = quote(intent),
    )
}

/// 释放占位的 shell 片段（不带锁函数定义，方便同时用于独立脚本与作业体里的 EXIT trap 函数）。
///
/// 锁内先核对"占位现在记的作业号还是不是自己的"再删：这正是 C16 要堵的窗口 —— 旧清理/旧 trap
/// 拿着过期的作业号删掉新一轮提交刚占的位。锁超时（有别的进程卡住）时保留占位不删，留给下一次
/// 提交按陈旧占位回收；绝不能为了"清理干净"而无锁删除别人的占位。
fn release_body(
    project: &str,
    id: &str,
    on_timeout: &str,
    on_released: &str,
    on_kept: &str,
) -> String {
    let placeholder = quote(&placeholder_path(project));
    // bash 不接受空分支（`else fi` 是语法错误），不需要输出的分支写成 `:`。
    let on_timeout = if on_timeout.is_empty() {
        ":"
    } else {
        on_timeout
    };
    let on_released = if on_released.is_empty() {
        ":"
    } else {
        on_released
    };
    let on_kept = if on_kept.is_empty() { ":" } else { on_kept };
    format!(
        r#"if _colm_lock_acquire; then
  _colm_current=$(cat {placeholder}/id 2>/dev/null)
  if [ -z "$_colm_current" ]; then
    _colm_current=$(sed -n 's/^ *"id": *"\([^"]*\)".*/\1/p' {placeholder}/intent 2>/dev/null | head -n 1)
  fi
  if [ -n "$_colm_current" ] && [ "$_colm_current" = {id} ]; then
    rm -rf {placeholder}
    {on_released}
  else
    {on_kept}
  fi
  _colm_lock_release
else
  {on_timeout}
fi
"#,
        id = quote(id),
    )
}

/// 提交失败/陈旧回收时的释放脚本：输出 `released` / `kept` / `lock_timeout` 供调用方判断。
fn release_script(project: &str, id: &str) -> String {
    format!(
        "{}{}",
        lock_helpers(project, id),
        release_body(
            project,
            id,
            "echo lock_timeout",
            "echo released",
            "echo kept"
        )
    )
}

/// 作业体 EXIT trap 用的释放函数：与 [`release_script`] 同一份 `release_body`，只是把诊断写到 stderr
/// （作业日志），不往 stdout 掺作业输出。
fn release_function(project: &str, id: &str) -> String {
    format!(
        "_colm_release_active_job() {{\n{}}}\n",
        release_body(
            project,
            id,
            "echo 'warning: 比较的活动作业锁超时，这次作业没有释放占位；下次提交会按陈旧占位回收' >&2",
            "",
            "",
        )
    )
}

/// 占位里是坏作业号（写坏了）时清掉：锁内确认当前内容仍是坏的才删，不会把并发新占位一起清掉。
fn clear_invalid_placeholder_script(project: &str) -> String {
    let placeholder = quote(&placeholder_path(project));
    format!(
        r#"{helpers}if ! _colm_lock_acquire; then echo lock_timeout; exit 0; fi
_colm_current=$(cat {placeholder}/id 2>/dev/null)
if [ -z "$_colm_current" ]; then
  _colm_current=$(sed -n 's/^ *"id": *"\([^"]*\)".*/\1/p' {placeholder}/intent 2>/dev/null | head -n 1)
fi
case "$_colm_current" in
  ''|*[!A-Za-z0-9._-]*) rm -rf {placeholder}; echo cleared;;
  *) echo kept;;
esac
_colm_lock_release
"#,
        helpers = lock_helpers(project, "colm-clear-invalid"),
    )
}

/// 只读查看占位（取回报告前用）：不创建、不删除任何东西。`none` 没有占位；`busy <作业号>` 有
/// （作业号也可以来自只有 intent 的占位）；`empty` 占位在但作业号没写成（崩溃残留，当作没有活动作业）。
fn active_job_probe_script(project: &str) -> String {
    let placeholder = quote(&placeholder_path(project));
    format!(
        r#"if [ ! -d {placeholder} ]; then
  echo none
  exit 0
fi
for i in 1 2 3 4 5 6 7 8 9 10; do
  existing=$(cat {placeholder}/id 2>/dev/null)
  if [ -z "$existing" ]; then
    existing=$(sed -n 's/^ *"id": *"\([^"]*\)".*/\1/p' {placeholder}/intent 2>/dev/null | head -n 1)
  fi
  if [ -n "$existing" ]; then
    printf 'busy %s\n' "$existing"
    exit 0
  fi
  sleep 0.1
done
echo empty
"#
    )
}

#[derive(Debug, PartialEq, Eq)]
enum Slot {
    /// 这次占住了。
    Acquired,
    /// 已被别人的作业占着，值是占位里记的作业号。
    Busy(String),
    /// 锁等待超时（有别的进程卡在锁里）。
    LockTimeout,
    /// `mkdir`/读回核对没成功（并发清洗），让调用方重试。
    Lost,
}

fn parse_slot(output: &str) -> Option<Slot> {
    let line = output
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    match line {
        "acquired" => Some(Slot::Acquired),
        "lost" => Some(Slot::Lost),
        "lock_timeout" => Some(Slot::LockTimeout),
        _ => line
            .strip_prefix("busy ")
            .map(|id| Slot::Busy(id.trim().to_owned())),
    }
}

/// 取回前的只读探针输出：`None` = 没有活动占位（或只剩崩溃残留的 `empty`），`Some(id)` = 有作业号。
fn parse_probe(output: &str) -> Option<Option<String>> {
    let line = output
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    if line == "none" || line == "empty" {
        Some(None)
    } else {
        line.strip_prefix("busy ")
            .map(|id| Some(id.trim().to_owned()))
    }
}

/// 一次比较作业的收尾结论（R09）。
///
/// `EndedUnknown` 专门表示"进程/队列已经不在了，却没留下退出码"：既不能永远等（页面会一直转、
/// 活动身份挡住重提），也不能编造成败。bare 调度器的 `kill -0` 探测失败走 `lost_confirmed
/// "the job process is gone"`；调度器确认结束但没有可识别的终态词也落在这里。COMPLETED 例外：
/// 那只是退出码还没经 NFS 回来，继续等退出码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JobEnd {
    Succeeded,
    Failed,
    Cancelled,
    EndedUnknown,
}

impl JobEnd {
    fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::EndedUnknown => "unknown",
        }
    }

    fn from_report(report: &job::Report) -> Option<Self> {
        match report.status.state {
            job::State::Finished { exit_code } => Some(if exit_code == 0 {
                Self::Succeeded
            } else {
                Self::Failed
            }),
            job::State::Lost => match job::ended(&report.status) {
                Some("cancelled") => Some(Self::Cancelled),
                Some(_) => Some(Self::Failed),
                None => {
                    let detail = report
                        .status
                        .detail
                        .as_deref()
                        .unwrap_or_default()
                        .to_ascii_uppercase();
                    (report.terminal && !detail.contains("COMPLETED")).then_some(Self::EndedUnknown)
                }
            },
            job::State::Queued | job::State::Running | job::State::Unknown => None,
        }
    }
}

/// 占位里的作业是否还没到确认终态。Queued/Running 一定是；Finished 与调度系统确认的终态
/// （CANCELLED/FAILED/COMPLETED…，见 [`job::Report::terminal`]）才是结束。Unknown 不能当终态 ——
/// 作业目录还在时可能只是这次查不到。Lost 但调度器只是回答"查不到记录"（terminal=false）同样不能
/// 当结束：宁可拒绝重复提交，也不能赌它已经跑完。查询失败更不会走到这里 —— [`job::report`] 直接返回
/// 错误，提交中止且不释放占位（审查报告第 6 条）。
fn placeholder_unfinished(report: &job::Report, job_dir_exists: bool) -> bool {
    // terminal 覆盖 COMPLETED 等"确认不会再写、但退出码可能还在路上"的终态：占位可以放，
    // 页面仍按 terminal 继续等退出码（两件事不能混）。
    if report.terminal || JobEnd::from_report(report).is_some() {
        return false;
    }
    match report.status.state {
        job::State::Unknown => job_dir_exists,
        job::State::Queued | job::State::Running | job::State::Lost => true,
        // Finished 一定 terminal，前面已经返回；保留显式分支，避免用 `_` 把拼错的状态吞掉。
        job::State::Finished { .. } => false,
    }
}

/// 提交/取回前的拒绝判定：`Some(固定错误原文)` 表示拒绝。
fn active_job_refusal(id: &str, report: &job::Report, job_dir_exists: bool) -> Option<String> {
    if placeholder_unfinished(report, job_dir_exists) {
        Some(active_job_error(id))
    } else {
        None
    }
}

/// 占位可以按"陈旧"回收吗：只有"作业确实不存在"的两种证据 —— Unknown 且作业目录都没有（提交在
/// mkdir 与 launch 之间崩了），或 Lost 且调度系统没确认终态（两边都回答查无记录）。Queued/Running/
/// Finished 都不回收；Lost+terminal 是确认终态，走正常的"作业已结束"清理。
///
/// Lost+terminal=false 以前一律拒绝（审查报告第 6 条怕把暂时查不到当结束）；C09 的"回执丢了但作业
/// 从未出现在调度器"会留下这种占位，所以这里保留宽限（占位超过一分钟）+ 清理前再查一次：宽限内
/// 作业真的被收下就会变成 queued/running，不会误删。作业目录还在时 Lost 也照此处理 —— sacct 回看
/// 七天，一分钟后再查不到就确实没被调度过。
fn abandoned_placeholder(report: &job::Report, job_dir_exists: bool) -> bool {
    match report.status.state {
        job::State::Unknown => !job_dir_exists,
        job::State::Lost => !report.terminal,
        job::State::Queued | job::State::Running | job::State::Finished { .. } => false,
    }
}

fn job_dir_exists(ssh: &Ssh, base: &str, id: &str) -> Result<bool> {
    job::check_id(id)?;
    let script = format!(
        "test -d {} && echo yes || echo no\n",
        quote(&job::job_dir(base, id))
    );
    Ok(ssh.run_ok(&script)?.trim() == "yes")
}

/// 占位是否已经老到可以按"崩溃残留"回收。刚占住的占位在作业目录出现前本来就没有作业可查
/// （status=unknown），这时不能立刻清掉：并发的另一份提交可能刚占位、正在提交。用占位目录的修改
/// 时间留一分钟宽限；超过仍没有作业目录，才清除。
fn placeholder_expired_script(project: &str) -> String {
    let placeholder = quote(&placeholder_path(project));
    format!(
        "if [ -d {placeholder} ] && find {placeholder} -maxdepth 0 -mmin +1 -print -quit 2>/dev/null | grep -q .; then echo stale; else echo fresh; fi\n"
    )
}

/// 提交前占位：占住就返回；占位里是还在跑的作业就拒绝；占位里的作业已确认终态（但 trap 没跑）
/// 就清掉陈旧占位后重试。最多循环几次：每次冲突都说明另一提交在推进状态，反复冲突时让用户稍后再试，
/// 不无限等。
fn claim_active_job(ssh: &Ssh, base: &str, project: &str, id: &str, intent: &str) -> Result<()> {
    for _ in 0..4 {
        let output = ssh.run_ok(&acquire_script(project, id, intent))?;
        match parse_slot(&output).context("活动作业占位的响应读不懂")? {
            Slot::Acquired => return Ok(()),
            Slot::LockTimeout => bail!("{}", lock_timeout_error()),
            Slot::Lost => continue,
            Slot::Busy(existing) => {
                // 占位里的内容先按作业号校验，再拼进命令/错误信息。
                if job::check_id(&existing).is_err() {
                    ssh.run_ok(&clear_invalid_placeholder_script(project))?;
                    continue;
                }
                let report = job::report(ssh, base, &existing, 5)?;
                let dir_exists = if matches!(report.status.state, job::State::Unknown) {
                    job_dir_exists(ssh, base, &existing)?
                } else {
                    false
                };
                if let Some(error) = active_job_refusal(&existing, &report, dir_exists) {
                    // 宽限回收：作业确实不存在且占位超过一分钟，再核对一次状态仍不存在才清（C09：
                    // 提交回执丢失后，调度器最终会回答"从没收到过"，那种占位不能永远挡住重试）。
                    if abandoned_placeholder(&report, dir_exists)
                        && ssh.run_ok(&placeholder_expired_script(project))?.trim() == "stale"
                    {
                        let again = job::report(ssh, base, &existing, 5)?;
                        let again_dir = if matches!(again.status.state, job::State::Unknown) {
                            job_dir_exists(ssh, base, &existing)?
                        } else {
                            false
                        };
                        if abandoned_placeholder(&again, again_dir) {
                            ssh.run_ok(&release_script(project, &existing))?;
                            continue;
                        }
                    }
                    bail!("{error}");
                }
                ssh.run_ok(&release_script(project, &existing))?;
            }
        }
    }
    bail!("这个比较的活动作业占位一直在变化，请稍后再提交。")
}

/// 取回前读占位里的作业号（只读）。
fn active_job_id(ssh: &Ssh, project: &str) -> Result<Option<String>> {
    let output = ssh.run_ok(&active_job_probe_script(project))?;
    let Some(id) = parse_probe(&output).context("活动作业占位的响应读不懂")? else {
        return Ok(None);
    };
    // 写坏的占位不是真实作业号：当没有，不让一条坏数据挡住取回。
    Ok(job::check_id(&id).ok().map(|()| id))
}

// ---- C09：提交回执丢失后的有界重查 ----

/// 提交回执的分类。SSH 在"服务器已经收下提交"之后断线时，不能把作业当成没提交。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReceiptClass {
    /// 调度系统/作业目录已经确认这次提交存在：保留占位，前端按作业号跟踪。
    Accepted,
    /// 连续多次都明确查不到这次作业：判定未接受，释放占位并返回原提交错误。
    Absent,
    /// 查询本身失败或证据不够：不能确定，保留占位，返回 Ok 让前端继续按作业号核对。
    Uncertain,
}

/// 生产路径的重查次数与间隔：3 次、每次隔 1 秒（约 2 秒窗口，够调度器把刚提交的作业记进去）。
const RECEIPT_ATTEMPTS: usize = 3;
const RECEIPT_RETRY: std::time::Duration = std::time::Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReceiptProbe {
    /// 已经能查到这次作业（排队/运行/结束/调度器确认的终态）。
    Present,
    /// 明确查无记录：调度器回答没有这次作业，或提交脚本连作业目录都没建出来。
    Absent,
    /// 暂时说不清（作业目录在、状态还查不到）：不能当未接受，也不能当已接受。
    Inconclusive,
}

fn receipt_probe(report: &job::Report, job_dir_exists: bool) -> ReceiptProbe {
    match report.status.state {
        job::State::Queued | job::State::Running | job::State::Finished { .. } => {
            ReceiptProbe::Present
        }
        job::State::Lost => {
            // Lost+terminal 是调度系统确认的终态（CANCELLED/FAILED/COMPLETED…）：作业在服务器上存在过。
            // Lost 但没有终态是"两边都回答查无记录"，明确未接受。
            if report.terminal {
                ReceiptProbe::Present
            } else {
                ReceiptProbe::Absent
            }
        }
        job::State::Unknown => {
            // 作业目录由提交脚本在启动调度之前写出：目录在说明身份已经在服务器上，只是状态还查不到；
            // 目录都没有说明提交脚本根本没跑到这一步 —— 这次 SSH 失败发生在提交之前。
            if job_dir_exists {
                ReceiptProbe::Inconclusive
            } else {
                ReceiptProbe::Absent
            }
        }
    }
}

/// 有界重查（可注入查询与间隔，测试用 0 间隔）。"查询本身失败"直接当不确定：保留占位、返回 Ok，
/// 让前端按作业号继续核对 —— 绝不能把一次 SSH 抖动说成"没提交"再让用户重提，那会写同一个成果目录。
fn recheck_receipt<F>(probe: F) -> ReceiptClass
where
    F: FnMut() -> Result<ReceiptProbe>,
{
    recheck_receipt_with(RECEIPT_ATTEMPTS, RECEIPT_RETRY, probe)
}

fn recheck_receipt_with<F>(
    attempts: usize,
    retry: std::time::Duration,
    mut probe: F,
) -> ReceiptClass
where
    F: FnMut() -> Result<ReceiptProbe>,
{
    let mut absent = 0;
    for attempt in 0..attempts {
        if attempt > 0 {
            std::thread::sleep(retry);
        }
        match probe() {
            Err(_) => return ReceiptClass::Uncertain,
            Ok(ReceiptProbe::Present) => return ReceiptClass::Accepted,
            Ok(ReceiptProbe::Absent) => absent += 1,
            Ok(ReceiptProbe::Inconclusive) => {}
        }
    }
    if attempts > 0 && absent == attempts {
        ReceiptClass::Absent
    } else {
        ReceiptClass::Uncertain
    }
}

fn submission_probe(ssh: &Ssh, base: &str, id: &str) -> Result<ReceiptProbe> {
    let report = job::report(ssh, base, id, 5)?;
    let dir_exists = if matches!(report.status.state, job::State::Unknown) {
        job_dir_exists(ssh, base, id)?
    } else {
        false
    };
    Ok(receipt_probe(&report, dir_exists))
}

// ---- C07：远程 run 级身份 ----

/// 一次运行的元数据：`<project>/runs/<作业号>/meta.json`。state 先 `running`，作业收尾时由
/// `finish` 脚本改成 `succeeded`/`failed`；只有成功才写 `latest-success.json`。
fn run_meta_json(
    id: &str,
    request: &RunRequest,
    created_unix: u64,
    config_sha: &str,
    ref_root: Option<&str>,
    output_dir: &str,
) -> serde_json::Value {
    json!({
        "run_id": id,
        "project": request.name,
        "created_at": iso8601_utc(created_unix),
        "created_unix": created_unix,
        "state": "running",
        "config_sha": config_sha,
        "ref_root": ref_root,
        "output_dir": output_dir,
    })
}

fn run_dir(project: &str, id: &str) -> String {
    format!("{project}/{RUNS_DIR}/{id}")
}

/// 作业收尾脚本（在作业体的 EXIT trap 里跑）。成功时写自己的 meta，再原子更新 `latest-success.json`；
/// 失败只写自己的 meta（带截断的日志末尾），上次成功指针一个字节都不动。
///
/// 为什么用 Python 而不是 shell：失败原因来自作业日志（任意 OpenBench 输出），在 shell 里拼 JSON
/// 转义一定会踩到反引号/反斜杠/换行；OpenBench 环境里本来就有 Python（作业体已确认 `openbench` 可执行）。
/// 原子写用"同目录临时文件 + os.replace"：读的一方要么看到旧内容、要么看到新内容。
const FINISH_SCRIPT: &str = r#"import datetime, json, os, sys, time

run_dir, project_dir, job_log, output_dir = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
limit = int(sys.argv[5])
code = os.environ.get('COLM_BENCH_EXIT', '1').strip()


def write_json(path, value):
    staged = path + '.tmp-' + str(os.getpid())
    with open(staged, 'w', encoding='utf-8') as handle:
        json.dump(value, handle, indent=2, ensure_ascii=False)
        handle.write('\n')
    os.replace(staged, path)


meta_path = os.path.join(run_dir, 'meta.json')
try:
    with open(meta_path, encoding='utf-8') as handle:
        meta = json.load(handle)
except (OSError, ValueError) as error:
    print('服务器上的运行记录读不懂：%s' % error, file=sys.stderr)
    sys.exit(1)

finished_unix = int(time.time())
finished_at = datetime.datetime.fromtimestamp(
    finished_unix, datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')
meta['finished_at'] = finished_at
meta['finished_unix'] = finished_unix
if code == '0':
    report = None
    output = os.path.join(run_dir, output_dir)
    for current, dirs, files in os.walk(output):
        dirs.sort()
        for name in ('evaluation_report_standalone.html', 'evaluation_report.html'):
            if name in files:
                report = os.path.relpath(os.path.join(current, name), run_dir)
                break
        if report is not None:
            break
    meta['state'] = 'succeeded'
    meta['report'] = report
    meta.pop('error', None)
    write_json(meta_path, meta)
    write_json(os.path.join(project_dir, 'latest-success.json'), {
        'run_id': meta['run_id'],
        'config_sha': meta['config_sha'],
        'report': report,
        'finished_at': finished_at,
        'finished_unix': finished_unix,
    })
else:
    try:
        size = os.path.getsize(job_log)
        with open(job_log, 'rb') as handle:
            handle.seek(max(0, size - 8000))
            tail = handle.read().decode('utf-8', 'replace')
    except OSError:
        tail = ''
    lines = [line for line in tail.splitlines() if line.strip()]
    detail = '\n'.join(lines[-40:])
    if len(detail) > limit:
        detail = detail[-limit:]
    meta['state'] = 'failed'
    meta['report'] = None
    meta['error'] = '这次运行失败（退出码 %s）%s' % (
        code, ('；输出末尾：\n' + detail) if detail else '；作业日志里没有输出。')
    write_json(meta_path, meta)
"#;

/// 作业体里的收尾函数 + EXIT trap：先写 meta/指针，再在锁内释放占位，最后带着原退出码退出。
/// 只有作业体真的进了本次运行的目录（`_colm_run_ready=1`）才碰 meta：万一 `mkdir` 撞上同名目录，
/// 不能把别人的运行记录当成自己的改写。
fn finish_function(
    base: &str,
    root: &str,
    project: &str,
    id: &str,
    run_dir: &str,
    output_dir: &str,
) -> String {
    let run_dir_q = quote(run_dir);
    let job_log = format!("{}/log", job::job_dir(base, id));
    let limit = RUN_ERROR_LIMIT.to_string();
    let args = [
        run_dir,
        project,
        job_log.as_str(),
        output_dir,
        limit.as_str(),
    ];
    let python = format!(
        "COLM_BENCH_EXIT=\"$_colm_code\" {}",
        python_here(root, FINISH_SCRIPT, &args, "COLM_BENCH_FINISH")
    );
    format!(
        r#"_colm_finish_run() {{
  _colm_code=$?
  trap - EXIT
  set +e
  if [ "${{_colm_run_ready:-}}" = "1" ] && [ -f {run_dir_q}/meta.json ]; then
{python}    _colm_meta=$?
    if [ "$_colm_code" -eq 0 ] && [ "$_colm_meta" -ne 0 ]; then _colm_code=1; fi
  fi
  _colm_release_active_job
  exit "$_colm_code"
}}
trap _colm_finish_run EXIT
"#
    )
}

/// 站点经纬度是 PREPARE 末尾的成对可选参数（`… station [lon] [lat]`）。只给一个会写出半份坐标，
/// 所以在拼 argv 前就拒绝；范围与 Tauri 侧 `valid_coordinates` 一致（经度允许 0–360 写法）。
fn coordinate_pair(site: &Site) -> Result<Option<(f64, f64)>> {
    match (site.lon, site.lat) {
        (None, None) => Ok(None),
        (Some(lon), Some(lat)) => {
            ensure!(
                lon.is_finite() && lat.is_finite() && lon.abs() <= 360.0 && lat.abs() <= 90.0,
                "bad site coordinates for {:?}: lon={lon}, lat={lat}",
                site.id
            );
            Ok(Some((lon, lat)))
        }
        _ => bail!(
            "site {:?} gives only one of lon/lat; give both or neither",
            site.id
        ),
    }
}

fn to_pretty(value: &serde_json::Value) -> Result<String> {
    Ok(serde_json::to_string_pretty(value)?)
}

/// 远程 run 的作业体：在 `<project>/runs/<作业号>/` 冻结配置/站点/输入/meta，`check`/`run` 都在这个
/// 目录里跑；输出根取配置里归一后的 `project.output_dir`（缺省 `output`），报告只从这一次的输出根
/// 发现；收尾由 EXIT trap 的 [`FINISH_SCRIPT`] 写 meta 与 `latest-success.json`。
///
/// R04：单点模拟的准备产物（`sim/<键>.nc`、`.csv`）也写进本次运行目录 —— 配置文本里的 `fulllist`
/// 是前端按项目目录算好的绝对路径（OpenBench 的 check 与 run 对相对路径解析不一致，2026-10-23
/// 窗口实测），所以运行目录里的清单再复制到 `<project>/sim/`；清单里的 DIR 指向运行目录里的 NC，
/// 下一次准备覆盖共享路径也改不到已归档运行的数据。
fn run_body(
    base: &str,
    root: &str,
    request: &RunRequest,
    id: &str,
    created_unix: u64,
) -> Result<String> {
    ensure!(
        request.config.len() <= 512 * 1024,
        "the configuration exceeds 512 KiB"
    );
    ensure!(
        !request.config.contains('\0'),
        "the configuration contains a NUL byte"
    );
    let project = project_dir(root, &request.name)?;
    let config_sha = sha256_hex(request.config.as_bytes());
    let output_dir = output_dir_from_config(&request.config)?;
    let ref_root = request.ref_root.as_deref().filter(|r| !r.trim().is_empty());
    if let Some(ref_root) = ref_root {
        server_path(ref_root, "ref_root")?;
    }
    // 先把所有站点校验完、把要冻结的清单拼好：请求不合法时在服务器上什么也不写。
    let mut prepares = Vec::new();
    let mut site_rows = Vec::new();
    let mut input_rows = Vec::new();
    for site in &request.sites {
        ensure!(valid_name(&site.id), "bad site key {:?}", site.id);
        ensure!(
            valid_station(&site.station),
            "unsuitable station name {:?}",
            site.station
        );
        server_path(&site.history_dir, "history_dir")?;
        ensure!(
            valid_name(site.prefix.trim_end_matches('_')),
            "bad history prefix {:?}",
            site.prefix
        );
        let coordinates = coordinate_pair(site)?;
        site_rows.push(json!({
            "id": site.id,
            "station": site.station,
            "lon": site.lon,
            "lat": site.lat,
        }));
        input_rows.push(json!({
            "id": site.id,
            "history_dir": site.history_dir,
            "prefix": site.prefix,
        }));
        prepares.push((site, coordinates));
    }
    let sites_json = to_pretty(&serde_json::Value::Array(site_rows))?;
    let inputs_json = to_pretty(&json!({
        "config_sha": config_sha,
        "ref_root": ref_root,
        "output_dir": output_dir,
        "sites": input_rows,
    }))?;
    let meta_json = to_pretty(&run_meta_json(
        id,
        request,
        created_unix,
        &config_sha,
        ref_root,
        &output_dir,
    ))?;
    let run_dir_text = run_dir(&project, id);
    let sim = quote(&format!("{project}/sim"));
    let mut body = String::new();
    // 环境变量要在 trap 与作业体之前生效（check/run 与收尾 python 都用它）。
    body.push_str(&program_env(root));
    body.push_str(&lock_helpers(&project, id));
    body.push_str(&release_function(&project, id));
    body.push_str(&finish_function(
        base,
        root,
        &project,
        id,
        &run_dir_text,
        &output_dir,
    ));
    body.push_str(&format!(
        r#"test -x {openbench} || {{ echo '先装 OpenBench' >&2; exit 1; }}
mkdir -p {sim}
mkdir -p {runs}
mkdir {run_dir} 2>/dev/null || {{ echo '这次运行的目录已经存在，不会覆盖别人的运行' >&2; exit 1; }}
cd {run_dir} || {{ echo '进不了这次运行的目录' >&2; exit 1; }}
_colm_run_ready=1
"#,
        openbench = env_program(root, "openbench"),
        sim = sim,
        runs = quote(&format!("{project}/{RUNS_DIR}")),
        run_dir = quote(&run_dir_text),
    ));
    // 配置原样冻结：`printf '%s'` 不解释内容，换行/`$`/反引号/单引号都按原字节落盘，配置哈希
    // 因此与落盘字节一一对应（服务器上 `sha256sum openbench.yaml` 可复算）。
    body.push_str(&format!(
        "printf '%s' {} > {RUN_CONFIG_FILE}\n",
        quote(&request.config)
    ));
    for (name, text) in [
        (RUN_SITES_FILE, &sites_json),
        (RUN_INPUTS_FILE, &inputs_json),
        (RUN_META_FILE, &meta_json),
    ] {
        body.push_str(&format!("printf '%s' {} > {name}\n", quote(text)));
    }
    for (site, coordinates) in prepares {
        body.push_str(&format!(
            "echo {}\n",
            quote(&format!("合并 {} 的 history", site.id))
        ));
        // 位置参数顺序由 PREPARE 的签名决定：folder prefix out_nc out_csv station [lon] [lat]。
        // 经纬度只在两个都给时追加（缺省时 Python 回退到 history 文件里的坐标）。
        // R04：准备产物落在本次运行目录；冻结的清单再复制到配置里的 fulllist 位置（项目 sim/），
        // 清单里的 DIR 指向运行目录里的 NC，后续准备覆盖共享路径也改不到已归档运行的数据。
        let frozen_nc = format!("{run_dir_text}/sim/{}.nc", site.id);
        let frozen_csv = format!("{run_dir_text}/sim/{}.csv", site.id);
        let mut prepare = vec![
            site.history_dir.clone(),
            site.prefix.clone(),
            frozen_nc,
            frozen_csv.clone(),
            site.station.clone(),
        ];
        if let Some((lon, lat)) = coordinates {
            prepare.push(lon.to_string());
            prepare.push(lat.to_string());
        }
        let prepare: Vec<&str> = prepare.iter().map(String::as_str).collect();
        body.push_str(&python_here(
            root,
            PREPARE_SCRIPT,
            &prepare,
            "COLM_BENCH_PREPARE",
        ));
        body.push_str(&format!(
            "cp {} {}\n",
            quote(&frozen_csv),
            quote(&format!("{project}/sim/{}.csv", site.id))
        ));
    }
    // 内置登记表的路径写成 `${OPENBENCH_REF_ROOT}/…`；不设它 `check` 直接报错（本机 2026-10-23 实测）。
    if let Some(ref_root) = ref_root {
        body.push_str(&format!("export OPENBENCH_REF_ROOT={}\n", quote(ref_root)));
    }
    for stage in ["check", "run"] {
        body.push_str(&format!(
            "echo 'openbench {stage} openbench.yaml'\n{} {stage} {RUN_CONFIG_FILE}\n",
            env_program(root, "openbench")
        ));
    }
    Ok(body)
}

/// `remote-bench-run`：写下配置，把单点模拟并成站点文件，再 `openbench check`、`openbench run`。
/// 作为服务器上的作业提交（按服务器的调度系统），返回作业号（也是这次运行的 run_id）。
///
/// 提交前先在服务器上原子占住同名比较（GUI-07 服务器侧保护），并写下提交意图（C09）：另一个作业
/// 还在跑就拒绝，避免两次提交交错写同一个成果目录；占位由作业体 EXIT trap 在锁内释放，提交失败时
/// 在这里（同样在锁内）释放。
pub(super) fn run(opts: &Opts) -> Result<()> {
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let base = super::remote_cmd::root_of(opts)?;
    let root = bench_root(opts)?;
    let request: RunRequest = read_json(opts)?;
    // 先校验并拼好作业体：名字/路径不合法时不占位，免得占位被一次不会提交的请求挡住。
    let project = project_dir(&root, &request.name)?;
    let id = unique_job("run")?;
    let created_unix = unix_now();
    let config_sha = sha256_hex(request.config.as_bytes());
    let intent = submit_intent(&id, created_unix, &config_sha);
    let body = run_body(&base, &root, &request, &id, created_unix)?;
    let scheduler = super::remote_cmd::parse_scheduler(opts, &ssh, &base)?;
    let resources = if scheduler == Scheduler::Bare {
        Resources {
            cpus: 1,
            env_script: opts.get("--env-script").filter(|t| !t.trim().is_empty()),
            ..Resources::default()
        }
    } else {
        super::remote_cmd::resources_from(opts, 4, 1, 1)?
    };
    let spec = Spec {
        scheduler,
        resources,
        name: sched::job_name(&format!("bench-{}", request.name)),
    };
    claim_active_job(&ssh, &base, &project, &id, &intent)?;
    let submitted = match job::submit_plain(&ssh, &base, &id, &body, &spec) {
        Ok(submitted) => submitted,
        Err(error) => {
            // C09：提交命令报错时不能直接当"没提交" —— SSH 可能是在服务器收下提交之后断的线。
            // 按作业号有界重查：确认存在就保留占位/意图、按作业号继续跟踪；明确查无记录才释放并
            // 返回原错误；查询失败当成不确定（同样保留），让前端继续核对。
            match recheck_receipt(|| submission_probe(&ssh, &base, &id)) {
                ReceiptClass::Absent => {
                    match ssh.run_ok(&release_script(&project, &id)) {
                        Ok(output) if output.trim() == "released" => {}
                        Ok(_) => eprintln!(
                            "warning: the comparison's active-job placeholder is held by another process; left it in place"
                        ),
                        Err(_) => eprintln!(
                            "warning: cannot release the comparison's active-job placeholder"
                        ),
                    }
                    return Err(error);
                }
                ReceiptClass::Accepted | ReceiptClass::Uncertain => {
                    // 保留占位与意图：前端拿到 job 后按作业号轮询，作业体收尾时 EXIT trap 会释放占位。
                    println!(
                        "{}",
                        json!({
                            "job": id,
                            "submitted": "unconfirmed",
                            "dir": project,
                            "run_id": id,
                            "config_sha": config_sha,
                        })
                    );
                    return Ok(());
                }
            }
        }
    };
    println!(
        "{}",
        json!({
            "job": id,
            "submitted": submitted,
            "dir": project,
            "run_id": id,
            "config_sha": config_sha,
        })
    );
    Ok(())
}

/// `remote-bench-uninstall`：删掉服务器上的环境。`env` 只删虚拟环境（OpenBench 与依赖），`all` 连 uv、专用
/// Python 和下载缓存一起删；登记表（`home/`）和比较（`projects/`）都留着，和本机的"删掉"一样。
pub(super) fn uninstall(opts: &Opts) -> Result<()> {
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let root = bench_root(opts)?;
    let what = opts.need_str("--what")?;
    let targets: &[&str] = match what.as_str() {
        "env" => &["env"],
        "all" => &["env", "python", "cache", "tools"],
        other => bail!("no such item: {other}"),
    };
    ssh.run_ok(&uninstall_script(&root, targets))?;
    println!("{}", json!({ "removed": targets }));
    Ok(())
}

fn uninstall_script(root: &str, targets: &[&str]) -> String {
    let paths = targets
        .iter()
        .map(|t| quote(&format!("{root}/{t}")))
        .collect::<Vec<_>>()
        .join(" ");
    // uv 装的 Python 有只读文件（缓存里的 wheel 解压目录），先放开写权限再删。
    format!("chmod -R u+w {paths} 2>/dev/null || true\nrm -rf {paths}\n")
}

/// `remote-bench-job`：作业状态与日志末尾。
///
/// 比原来的 JSON 多两个字段，给比较页判断"取消在途"与"已确认结束"：
/// - `cancel_requested`：取消请求已被调度系统收下，但作业可能还在排队/运行（前端显示"正在取消"）；
/// - `terminal`：调度系统确认作业已经结束（退出码或 CANCELLED/FAILED/COMPLETED…）。它是"能不能放开
///   重复提交/别再把还在跑当结束"的判断依据，不等于 `ended`：COMPLETED 但退出码还没到时会
///   terminal=true、ended=null，前端应继续等退出码，不写成功/失败。
/// - `result_unknown`（R09）：进程/队列已经不在、又没有退出码。`ended="unknown"` 是终态，
///   页面应把它归档并允许重新提交，而不是无限轮询。
///
/// 查询命令本身失败（调度器不可用等）时这里返回错误、进程非零退出：Tauri 把它交给前端，前端保留
/// 作业身份继续核对。绝不返回一个"lost"让前端/占位把暂时查不到当成作业已经没了。
pub(super) fn job_status(opts: &Opts) -> Result<()> {
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let base = super::remote_cmd::root_of(opts)?;
    let id = opts.need_str("--job")?;
    let lines = opts
        .get("--lines")
        .map(|v| v.parse())
        .transpose()
        .context("--lines must be a whole number")?
        .unwrap_or(80usize)
        .min(400);
    let report = job::report(&ssh, &base, &id, lines)?;
    println!("{}", job_status_json(&id, &report));
    Ok(())
}

/// 作业状态 JSON：纯函数，状态机单测直接覆盖各条收尾路径（R09）。
fn job_status_json(id: &str, report: &job::Report) -> serde_json::Value {
    let end = JobEnd::from_report(report);
    let result_unknown = end == Some(JobEnd::EndedUnknown);
    json!({
        "job": id,
        "status": &report.status,
        "ended": end.map(JobEnd::as_str),
        "terminal": report.terminal || result_unknown,
        "result_unknown": result_unknown,
        "cancel_requested": report.cancel_requested,
    })
}

/// 取消失败后的收尾：作业已经是终态（含"已终止、结果未知"）就返回可行动的结论，`None` 表示
/// 作业还在排队/运行、取消错误必须照实上报。纯函数，`cancel` 只在 `job::cancel` 报错时调用。
fn cancel_outcome(id: &str, report: &job::Report) -> Option<serde_json::Value> {
    let end = JobEnd::from_report(report);
    if !report.terminal && end.is_none() {
        return None;
    }
    let result_unknown = end == Some(JobEnd::EndedUnknown);
    let note = if result_unknown {
        "作业进程已经不在服务器上，没有留下退出码：这次运行按“已终止、结果未知”收尾，可以重新提交。"
    } else {
        "作业已经结束，取消请求不再需要。"
    };
    Some(json!({
        "job": id,
        "request_accepted": false,
        "ended": end.map(JobEnd::as_str),
        "result_unknown": result_unknown,
        "state": &report.status.state,
        "note": note,
    }))
}

/// `remote-bench-cancel`：结束作业（调度系统的交给它，后台进程结束整个进程组）。
///
/// Stop 撞上已经不存在的 PID 时不能只报 kill 错误（R09）：先查终态，确认作业已经结束就返回
/// "已终止、结果未知"（或已知的取消/失败），让页面收尾并放开重新提交；仍在排队/运行才照实报错。
pub(super) fn cancel(opts: &Opts) -> Result<()> {
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let base = super::remote_cmd::root_of(opts)?;
    let id = opts.need_str("--job")?;
    match job::cancel(&ssh, &base, &id) {
        Ok(()) => println!("{}", json!({ "job": id, "request_accepted": true })),
        Err(error) => {
            let report = job::report(&ssh, &base, &id, 5)
                .map_err(|probe| anyhow::anyhow!("{error}；核对作业终态也失败：{probe}"))?;
            match cancel_outcome(&id, &report) {
                Some(outcome) => println!("{outcome}"),
                None => return Err(error),
            }
        }
    }
    Ok(())
}

/// `remote-bench-inspect`：服务器上一个 history（或算例）目录的概况，同本机的 `bench_inspect`。
pub(super) fn inspect(opts: &Opts) -> Result<()> {
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let root = bench_root(opts)?;
    let dir = opts.need_str("--dir")?;
    server_path(&dir, "--dir")?;
    let script = format!(
        "{}{}",
        program_env(&root),
        python_here(&root, INSPECT_SCRIPT, &[&dir], "COLM_BENCH_INSPECT")
    );
    let output = ssh.run(&script)?;
    if !output.success {
        bail!(
            "{}",
            output
                .stderr
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("读不了这个目录")
        );
    }
    let last = output
        .stdout
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .context("no inspection result")?;
    let value: serde_json::Value = serde_json::from_str(last)?;
    println!("{value}");
    Ok(())
}

/// `remote-bench-metric-options`：服务器上已安装的 OpenBench 有哪些可选指标/评分。
pub(super) fn metric_options(opts: &Opts) -> Result<()> {
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let root = bench_root(opts)?;
    let text = ssh.run_ok(&python_here(
        &root,
        METRIC_SCRIPT,
        &[],
        "COLM_METRIC_OPTIONS",
    ))?;
    let value: serde_json::Value = serde_json::from_str(text.trim()).context("指标列表读不懂")?;
    println!("{value}");
    Ok(())
}

/// `remote-bench-references`：服务器上的登记表（本机一份、服务器一份，各管各的）。
pub(super) fn references(opts: &Opts) -> Result<()> {
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let root = bench_root(opts)?;
    let text = ssh.run_ok(&format!(
        "{}{} ref list --format json\n",
        program_env(&root),
        env_program(&root, "openbench")
    ))?;
    let value: serde_json::Value = serde_json::from_str(&strict_json(&text))
        .map_err(|e| anyhow::anyhow!("参考数据列表读不懂：{e}"))?;
    println!("{value}");
    Ok(())
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Register {
    name: String,
    root_dir: String,
    data_type: String,
    tim_res: String,
    years: (i32, i32),
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    fulllist: Option<String>,
    variables: Vec<String>,
}

/// `remote-bench-register`：登记一个参考数据集（不交互），路径都是服务器上的。
pub(super) fn register(opts: &Opts) -> Result<()> {
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let root = bench_root(opts)?;
    let request: Register = read_json(opts)?;
    println!(
        "{}",
        json!({ "output": ssh.run_ok(&register_script(&root, &request)?)? })
    );
    Ok(())
}

/// OpenBench 3.0.8 的合法 `tim_res`（`openbench/config/loader.py` 的 `VALID_TIM_RES_VALUES`），
/// 外加 `Nmonth` 形式（正则 `[1-9]\d*month`，如 `3month` 的多月统计窗口）。旧白名单只有
/// Hour/Day/Month/Year，把 3Hour/6Hour/8Day 与 climatology 参考数据挡在本地、根本到不了 OpenBench。
const TIM_RES_VALUES: [&str; 9] = [
    "Month",
    "Day",
    "Hour",
    "Year",
    "3Hour",
    "6Hour",
    "8Day",
    "climatology-month",
    "climatology-year",
];

/// `Nmonth`：N 是正整数、`month` 小写 —— OpenBench 对 Nmonth 的正则大小写敏感（`3MONTH` 会被拒）。
fn is_n_month(value: &str) -> bool {
    let Some(number) = value.strip_suffix("month") else {
        return false;
    };
    !number.is_empty()
        && !number.starts_with('0')
        && number.bytes().all(|byte| byte.is_ascii_digit())
}

fn valid_tim_res(value: &str) -> bool {
    // 固定值的比较与 OpenBench 一致地不区分大小写（loader 会按 canonical 名归一）。
    let lower = value.to_ascii_lowercase();
    TIM_RES_VALUES
        .iter()
        .any(|known| known.to_ascii_lowercase() == lower)
        || is_n_month(value)
}

fn register_script(root: &str, request: &Register) -> Result<String> {
    ensure!(
        valid_name(&request.name),
        "bad reference name {:?}",
        request.name
    );
    ensure!(
        ["grid", "stn"].contains(&request.data_type.as_str()),
        "the type must be grid or stn"
    );
    ensure!(
        valid_tim_res(&request.tim_res),
        "the time resolution must be one of Month/Day/Hour/Year/3Hour/6Hour/8Day/climatology-month/climatology-year/Nmonth, got {:?}",
        request.tim_res
    );
    ensure!(!request.variables.is_empty(), "give at least one variable");
    server_path(&request.root_dir, "root_dir")?;
    let mut command = format!(
        "{} ref register {} --root-dir {} --data-type {} --tim-res {} --years {} {}",
        env_program(root, "openbench"),
        quote(&request.name),
        quote(&request.root_dir),
        request.data_type,
        request.tim_res,
        request.years.0,
        request.years.1
    );
    if let Some(category) = request.category.as_deref().filter(|c| !c.trim().is_empty()) {
        command.push_str(&format!(" --category {}", quote(category)));
    }
    if let Some(list) = request.fulllist.as_deref().filter(|f| !f.trim().is_empty()) {
        server_path(list, "fulllist")?;
        command.push_str(&format!(" --fulllist {}", quote(list)));
    }
    for variable in &request.variables {
        ensure!(
            !variable.chars().any(char::is_control),
            "bad variable {variable:?}"
        );
        command.push_str(&format!(" -v {}", quote(variable)));
    }
    Ok(format!("{}{command} 2>&1\n", program_env(root)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegisterModelVariable {
    model: String,
    variable: String,
    varname: String,
    unit: String,
}

/// OpenBench `-v` 的 `标准名:变量名:单位`。字段里带冒号会被它当成第 4、5 段（prefix/suffix）静默拆开，
/// 所以和本机命令一样直接拒绝；首尾空白去掉，免得登记成 " W m-2"。
fn model_variable_spec(variable: &str, varname: &str, unit: &str) -> Result<String> {
    let fields = [("variable", variable), ("varname", varname), ("unit", unit)];
    for (label, value) in fields {
        let value = value.trim();
        ensure!(
            !value.is_empty() && !value.chars().any(char::is_control) && !value.contains(':'),
            "bad {label} {value:?}"
        );
    }
    Ok(format!(
        "{}:{}:{}",
        variable.trim(),
        varname.trim(),
        unit.trim()
    ))
}

/// `remote-bench-register-model-variable`：给 OpenBench 的模型 profile 增加一个变量映射（新模块输出变量）。
pub(super) fn register_model_variable(opts: &Opts) -> Result<()> {
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let root = bench_root(opts)?;
    let request: RegisterModelVariable = read_json(opts)?;
    ensure!(
        valid_name(&request.model),
        "bad model profile name {:?}",
        request.model
    );
    let spec = model_variable_spec(&request.variable, &request.varname, &request.unit)?;
    let script = format!(
        "{}{} model register {} -v {} --append-only 2>&1\n",
        program_env(&root),
        env_program(&root, "openbench"),
        quote(&request.model),
        quote(&spec)
    );
    println!("{}", json!({ "output": ssh.run_ok(&script)? }));
    Ok(())
}

/// `remote-bench-scan`：预览一个参考数据目录能自动认出哪些（`--dry-run`，不写登记表）。
pub(super) fn scan(opts: &Opts) -> Result<()> {
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let root = bench_root(opts)?;
    let dir = opts.need_str("--dir")?;
    server_path(&dir, "--dir")?;
    let output = ssh.run(&format!(
        "{}{} ref scan {} --dry-run --allow-skip 2>&1\n",
        program_env(&root),
        env_program(&root, "openbench"),
        quote(&dir)
    ))?;
    if !output.success {
        bail!("扫描失败：{}", output.stdout.trim());
    }
    println!("{}", json!({ "output": output.stdout }));
    Ok(())
}

/// `remote-bench-fetch`：取回一次运行的报告、指标、配置与运行记录（`openbench.yaml`、`meta.json`、
/// `sites.json`、`inputs.json`），合并出来的站点文件和中间数据留在服务器上。
///
/// C07：默认取回 `latest-success.json` 指向的那次成功运行，`--run <作业号>` 指定取回某次运行。新式
/// 比较（有 `runs/`）不再按项目根 `output/` 猜报告 —— 只认这次运行自己的目录，失败运行留下的
/// `output/` 不会冒充成果。没有 `runs/` 的旧项目/旧镜像回退到旧的 `output/` 扫描。
///
/// 同名比较还有未确认终态的活动作业时拒绝取回（GUI-07 服务器侧）：作业正在写成果目录，这时取回
/// 只会拿到不完整的报告，还会让"本机旧报告仍是上次成功版本"的提示真假难分。
pub(super) fn fetch(opts: &Opts) -> Result<()> {
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let base = super::remote_cmd::root_of(opts)?;
    let root = bench_root(opts)?;
    let name = opts.need_str("--name")?;
    let project = project_dir(&root, &name)?;
    let out = std::path::PathBuf::from(opts.need_str("--out")?);
    ensure!(
        out.is_absolute(),
        "--out must be an absolute local directory"
    );
    if let Some(active) = active_job_id(&ssh, &project)? {
        let report = job::report(&ssh, &base, &active, 5)?;
        let dir_exists = if matches!(report.status.state, job::State::Unknown) {
            job_dir_exists(&ssh, &base, &active)?
        } else {
            false
        };
        if let Some(error) = active_job_refusal(&active, &report, dir_exists) {
            bail!("{error}");
        }
    }
    match resolve_target(&ssh, &project, opts.get("--run").as_deref())? {
        FetchTarget::Legacy => {
            let listing = ssh.run_ok(&legacy_listing_script(&project))?;
            let entries = clean_entries(&listing);
            require_report(&entries)?;
            ssh.download(&project, &entries, &out)?;
            println!(
                "{}",
                json!({
                    "dir": out,
                    "entries": entries,
                    "run_id": null,
                    "config_sha": null,
                    "state": null,
                    "legacy": true,
                })
            );
        }
        FetchTarget::Run {
            run_id,
            config_sha,
            state,
            output_dir,
            report,
        } => {
            let dir = run_dir(&project, &run_id);
            let listing = ssh.run_ok(&run_listing_script(&dir, &output_dir, report.as_deref()))?;
            ensure!(
                !listing.contains(NO_SUCH_REMOTE_PATH),
                "服务器上没有这次运行的快照：{run_id}"
            );
            let entries = clean_entries(&listing);
            require_recorded_report(&entries, report.as_deref())?;
            ssh.download(&dir, &entries, &out)?;
            println!(
                "{}",
                json!({
                    "dir": out,
                    "entries": entries,
                    "run_id": run_id,
                    "config_sha": config_sha,
                    "state": state,
                    "legacy": false,
                    "output_dir": output_dir,
                    "report": report,
                })
            );
        }
    }
    Ok(())
}

/// 旧式项目的取回清单：项目根 `output/` 下的 reports/metrics/scores 与 `openbench.yaml`。
fn legacy_listing_script(project: &str) -> String {
    format!(
        "cd {} && ls -d output/*/reports output/*/metrics output/*/scores openbench.yaml 2>/dev/null; true\n",
        quote(project)
    )
}

/// 一次运行的取回清单：冻结文件 + 这次运行自己的 reports/metrics/scores。`cd` 失败时输出哨兵，
/// 调用方据此报"没有这次运行的快照"，而不是拿一个空清单去装一份什么都没有的镜像。
///
/// R03：输出根取运行记录里的 `project.output_dir`（归一后的相对路径）。记录里有报告时直接列它的
/// 父目录与文件本身，不再猜 `output/*/reports`；旧运行记录没有 `report` 字段时退回输出根的通配。
/// 输出根已经过 `run_relative_path` 校验，拼进 shell 前只可能是目录名片段。
fn run_listing_script(run_dir: &str, output_dir: &str, report: Option<&str>) -> String {
    let out = quote(output_dir);
    let outputs = match report {
        Some(report) => {
            let mut paths = vec![quote(report)];
            // 报告在运行目录根下时没有父目录可列：`.` 进 tar 清单会把整个运行目录再打一次。
            if let Some((parent, _)) = report.rsplit_once('/') {
                if !parent.is_empty() {
                    paths.push(quote(parent));
                }
            }
            format!("{} {out}/*/metrics {out}/*/scores", paths.join(" "))
        }
        None => format!("{out}/*/reports {out}/*/metrics {out}/*/scores"),
    };
    format!(
        "cd {dir} || {{ echo '{missing}'; exit 0; }}\nls -d {files} {outputs} 2>/dev/null\ntrue\n",
        dir = quote(run_dir),
        missing = NO_SUCH_REMOTE_PATH,
        files = [
            RUN_CONFIG_FILE,
            RUN_META_FILE,
            RUN_SITES_FILE,
            RUN_INPUTS_FILE
        ]
        .join(" "),
    )
}

/// 读取远程文件时的哨兵：文件不存在时脚本输出它，调用方据此区分"空文件"与"没有文件"。
const NO_SUCH_REMOTE_PATH: &str = "COLM_NO_SUCH_PATH";

/// `latest-success.json`：只有成功才更新的指针。这里只取运行号与配置哈希（报告与时间由列表和状态
/// 页各自读）；字段与窗口进程 `bench_runs.rs` 的同名结构一致，多出来的字段 serde 直接忽略。
#[derive(Debug, Deserialize)]
struct LatestSuccess {
    run_id: String,
    #[serde(default)]
    config_sha: String,
}

/// 运行记录的只读子集：取回时确认这次运行真的成功过，并取出配置哈希、输出根与报告路径。
#[derive(Debug, Deserialize)]
struct RunMeta {
    #[serde(default)]
    run_id: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    config_sha: String,
    /// R03：本次运行的输出根（相对运行目录）。旧记录没这个字段，按 OpenBench 默认的 `output`。
    #[serde(default = "default_output_dir")]
    output_dir: String,
    /// 收尾脚本记下的报告路径（相对运行目录）；32a2e988 之后的新记录都有。
    #[serde(default)]
    report: Option<String>,
}

fn default_output_dir() -> String {
    DEFAULT_OUTPUT_DIR.to_owned()
}

#[derive(Debug, PartialEq, Eq)]
enum FetchTarget {
    /// 没有 `runs/` 的旧项目：按项目根 `output/` 取。
    Legacy,
    /// 新式比较的一次运行。
    Run {
        run_id: String,
        config_sha: String,
        state: String,
        output_dir: String,
        report: Option<String>,
    },
}

/// 运行记录里的相对路径（输出根/报告）：只能是运行目录内的 POSIX 相对路径。
/// 记录在服务器上生成，但可能被人手改；不校验就拼进 `ls`/`tar` 的清单，等于允许读运行目录之外。
fn run_relative_path(value: &str, what: &str) -> Result<String> {
    let text = value.trim();
    ensure!(!text.is_empty(), "运行记录里的 {what} 是空路径");
    ensure!(
        !text.starts_with('/'),
        "运行记录里的 {what} 不能是绝对路径：{text}"
    );
    let mut parts: Vec<&str> = Vec::new();
    for part in text.split('/') {
        match part {
            "" | "." => {}
            ".." => bail!("运行记录里的 {what} 不能跳出运行目录：{text}"),
            other => {
                ensure!(
                    !other.contains('\\') && !other.chars().any(char::is_control),
                    "运行记录里的 {what} 不是安全的相对路径：{text}"
                );
                parts.push(other);
            }
        }
    }
    ensure!(!parts.is_empty(), "运行记录里的 {what} 没有指向任何文件");
    Ok(parts.join("/"))
}

fn read_remote_file(ssh: &Ssh, path: &str) -> Result<Option<String>> {
    let text = ssh.run_ok(&format!(
        "if [ -f {path} ]; then cat {path}; else echo '{missing}'; fi\n",
        path = quote(path),
        missing = NO_SUCH_REMOTE_PATH,
    ))?;
    Ok((text.trim() != NO_SUCH_REMOTE_PATH).then_some(text))
}

fn remote_dir_exists(ssh: &Ssh, path: &str) -> Result<bool> {
    Ok(ssh
        .run_ok(&format!(
            "test -d {path} && echo yes || echo no\n",
            path = quote(path)
        ))?
        .trim()
        == "yes")
}

/// 列表输出里只留相对路径：绝对路径、`..` 与空行都不能进 `tar -C` 的清单。
fn clean_entries(listing: &str) -> Vec<String> {
    listing
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.contains("..") && !line.starts_with('/'))
        .map(str::to_owned)
        .collect()
}

/// 报告是这个命令唯一要给人看的东西：没有 `reports/` 目录时明确拒绝，而不是装一份只有指标的镜像
/// 把本机旧报告顶掉。
fn require_report(entries: &[String]) -> Result<()> {
    ensure!(
        entries.iter().any(|entry| entry.ends_with("/reports")),
        "服务器上还没有可显示的评估报告"
    );
    Ok(())
}

/// R03：运行记录里有报告路径时，清单里必须真的有这个文件；旧记录没有路径时按输出根的通配清单判。
/// 只列到父目录、文件被删的情况下不能放行 —— 那会装一份没有报告的镜像顶掉本机旧报告。
fn require_recorded_report(entries: &[String], report: Option<&str>) -> Result<()> {
    let Some(report) = report else {
        return require_report(entries);
    };
    ensure!(
        entries.iter().any(|entry| entry == report),
        "服务器上这次运行的报告不在了：{report}"
    );
    Ok(())
}

fn run_record_error(what: &str, detail: impl std::fmt::Display) -> anyhow::Error {
    anyhow::anyhow!("服务器上的运行记录读不懂：{what}（{detail}）")
}

fn parse_latest_success(text: &str) -> Result<LatestSuccess> {
    serde_json::from_str(text)
        .map_err(|error| run_record_error(&format!("{LATEST_SUCCESS_FILE} 解析失败"), error))
}

fn parse_run_meta(text: &str) -> Result<RunMeta> {
    serde_json::from_str(text)
        .map_err(|error| run_record_error(&format!("{RUN_META_FILE} 解析失败"), error))
}

/// 读某次运行的 meta：运行目录或 meta 缺失都算"没有这次运行的快照"（不能静默换成别的运行）。
fn read_run_meta(ssh: &Ssh, project: &str, run_id: &str) -> Result<RunMeta> {
    let dir = run_dir(project, run_id);
    let path = format!("{dir}/{RUN_META_FILE}");
    let Some(text) = read_remote_file(ssh, &path)? else {
        bail!("服务器上没有这次运行的快照：{run_id}");
    };
    let mut meta = parse_run_meta(&text)?;
    if !meta.run_id.is_empty() && meta.run_id != run_id {
        bail!(
            "服务器上的运行记录读不懂：{path} 里记的是另一运行（{}）",
            meta.run_id
        );
    }
    // 记录里的路径要拼进取回清单/`ls`：先按运行目录内的相对路径校验，坏记录直接拒绝。
    meta.output_dir = run_relative_path(&meta.output_dir, "output_dir")
        .with_context(|| format!("运行记录读不懂：{path}"))?;
    if let Some(report) = meta.report.as_deref() {
        meta.report = Some(
            run_relative_path(report, "report")
                .with_context(|| format!("运行记录读不懂：{path}"))?,
        );
    }
    Ok(meta)
}

/// 这次运行能不能当"成功成果"取回：只有 succeeded 才有报告，失败运行的 meta 只能留在服务器上诊断。
fn succeeded_state(run_id: &str, meta: &RunMeta) -> Result<()> {
    ensure!(
        meta.state == "succeeded",
        "这次运行没有成功，没有可显示的评估报告（运行 {run_id}，状态 {}）",
        if meta.state.is_empty() {
            "未知"
        } else {
            meta.state.as_str()
        }
    );
    Ok(())
}

fn resolve_target(ssh: &Ssh, project: &str, requested: Option<&str>) -> Result<FetchTarget> {
    let runs = format!("{project}/{RUNS_DIR}");
    if let Some(run_id) = requested {
        job::check_id(run_id).map_err(|_| anyhow::anyhow!("不是一次评估运行：{run_id}"))?;
        let meta = read_run_meta(ssh, project, run_id)?;
        succeeded_state(run_id, &meta)?;
        return Ok(FetchTarget::Run {
            run_id: run_id.to_owned(),
            config_sha: meta.config_sha,
            state: meta.state,
            output_dir: meta.output_dir,
            report: meta.report,
        });
    }
    if !remote_dir_exists(ssh, &runs)? {
        // 旧式项目/旧镜像：没有 run 身份，只能按旧路径取。
        return Ok(FetchTarget::Legacy);
    }
    let path = format!("{project}/{LATEST_SUCCESS_FILE}");
    let Some(text) = read_remote_file(ssh, &path)? else {
        bail!("这个比较还没有成功过的运行，取不回报告。");
    };
    let success = parse_latest_success(&text)?;
    job::check_id(&success.run_id).map_err(|_| {
        run_record_error(&format!("{path} 里的 run_id 不像作业号"), &success.run_id)
    })?;
    let meta = read_run_meta(ssh, project, &success.run_id)?;
    succeeded_state(&success.run_id, &meta)?;
    let mut config_sha = meta.config_sha;
    if config_sha.is_empty() {
        config_sha = success.config_sha;
    }
    Ok(FetchTarget::Run {
        run_id: success.run_id,
        config_sha,
        state: meta.state,
        output_dir: meta.output_dir,
        report: meta.report,
    })
}

#[cfg(test)]
#[path = "remote_bench_tests.rs"]
mod remote_bench_tests;
