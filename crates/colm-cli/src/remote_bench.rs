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
//! colm-cli remote-bench-fetch      --host H --root R --name N --out 本机目录
//! ```
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
    let script = format!(
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
  r=$(ls "$d"output/*/reports/evaluation_report_standalone.html "$d"output/*/reports/evaluation_report.html 2>/dev/null | head -n 1)
  echo "project=$(basename "$d")|$r"
done
true
"#,
        ob = quote(&root),
        env = program_env(&root),
    );
    let text = ssh.run_ok(&script)?;
    println!("{}", parse_status(&root, &text));
    Ok(())
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
                let (name, report) = value.split_once('|').unwrap_or((value, ""));
                projects.push(json!({
                    "name": name,
                    "report": (!report.is_empty()).then_some(report),
                }));
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

// ---- 同名比较的活动作业保护（GUI-07） ----
//
// 客户端已经按"本机记着的作业身份"阻止了重复提交，但身份只存在发起提交的那台电脑上：换一台电脑、
// 手工清掉 localStorage、或两次提交交错时，服务器仍会被要求写同一个成果目录。这里在服务器侧的
// 比较目录里放一个原子占位（`mkdir` 只有一个进程能成功），提交前必须先占住它。
// 占位里的作业号由提交者先本地生成，所以占位写下的就是最终作业号；调用方据此用普通远程运行
// 有的 `job::report` 核对终态，不另造一套状态判定（查询失败会变成错误，不会落成 lost）。

/// 比较目录里的活动作业占位目录。
const ACTIVE_JOB: &str = ".active-job";

fn placeholder_path(project: &str) -> String {
    format!("{project}/{ACTIVE_JOB}")
}

/// 固定错误原文：前端按"这个比较还有未完成的评估作业"前缀映射成双语提示，改字前先搜前端。
fn active_job_error(id: &str) -> String {
    format!(
        "这个比较还有未完成的评估作业（作业号 {id}）；先核对终态或停止它，避免重复写同一个成果目录。"
    )
}

/// 服务器上一个比较的目录：`<openbench>/projects/<名字>`。
fn project_dir(root: &str, name: &str) -> Result<String> {
    ensure!(
        valid_name(name),
        "a comparison name may only use letters, digits, - and _: {name:?}"
    );
    Ok(format!("{root}/projects/{name}"))
}

/// 原子占住比较目录：`mkdir` 成功才算占住。已有占位时输出 `busy <作业号>`，让调用方先核对它是否
/// 真的还在跑；占位目录在、作业号迟迟写不进来（上个提交在 mkdir 与写号之间崩了）输出 `empty`。
/// 写号后要读回核对：与"清陈旧占位"并发交错时，一方可能在另一方写号前把占位删掉，两边都以为自己
/// 占住了 —— 读回验证能把这种窗口收掉，两方至多一方输出 acquired。
fn acquire_script(project: &str, id: &str) -> String {
    let placeholder = quote(&placeholder_path(project));
    format!(
        r#"mkdir -p {project} || {{ echo 'cannot create the comparison folder' >&2; exit 1; }}
if mkdir {placeholder} 2>/dev/null; then
  if printf '%s\n' {id} > {placeholder}/id 2>/dev/null && [ "$(cat {placeholder}/id 2>/dev/null)" = {id} ]; then
    echo acquired
  else
    echo lost
  fi
  exit 0
fi
for i in 1 2 3 4 5 6 7 8 9 10; do
  existing=$(cat {placeholder}/id 2>/dev/null || true)
  if [ -n "$existing" ]; then
    printf 'busy %s\n' "$existing"
    exit 0
  fi
  sleep 0.1
done
echo empty
"#,
        project = quote(project),
        id = quote(id),
    )
}

/// 只读查看占位（取回报告前用）：不创建、不删除任何东西。`none` 没有占位；`busy <作业号>` 有；
/// `empty` 占位在但作业号没写成（崩溃残留，当作没有活动作业）。
fn active_job_probe_script(project: &str) -> String {
    let placeholder = quote(&placeholder_path(project));
    format!(
        r#"if [ ! -d {placeholder} ]; then
  echo none
  exit 0
fi
for i in 1 2 3 4 5 6 7 8 9 10; do
  existing=$(cat {placeholder}/id 2>/dev/null || true)
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

/// 释放占位（作业名对得上才删）：作业体退出时和提交失败时共用同一条命令。作业体以 EXIT trap 调用它，
/// 覆盖正常结束、失败退出和 SIGTERM 取消；SIGKILL/节点宕机不会跑 trap，那种残留由下一次提交按
/// "占位里的作业已确认终态"清掉。先用 `cat` 核对作业号，避免作业收尾与清陈旧占位交错时删掉别人的占位。
fn release_placeholder_command(project: &str, id: &str) -> String {
    let placeholder = quote(&placeholder_path(project));
    format!(
        "if [ \"$(cat {placeholder}/id 2>/dev/null)\" = {id} ]; then rm -rf {placeholder}; fi",
        id = quote(id)
    )
}

fn release_script(project: &str, id: &str) -> String {
    format!("{}\n", release_placeholder_command(project, id))
}

/// 作业体的第一行：作业退出时释放占位。整条清理命令再 `quote` 一次，因为 `trap '…'` 是单引号字符串，
/// 里面的路径/作业号不能直接再套单引号。
fn active_job_trap(project: &str, id: &str) -> String {
    format!(
        "trap {} EXIT\n",
        quote(&release_placeholder_command(project, id))
    )
}

/// 占位在但作业号没写成（崩溃残留）：清掉重试。调用方只在重试读号仍为空时才用，正常并发写号
/// 只有几微秒，不会被误删。
fn remove_placeholder_script(project: &str) -> String {
    format!("rm -rf {}\n", quote(&placeholder_path(project)))
}

#[derive(Debug, PartialEq, Eq)]
enum Slot {
    /// 这次占住了。
    Acquired,
    /// 已被别人的作业占着，值是占位里记的作业号。
    Busy(String),
    /// 占位存在但作业号没写成（崩溃残留）。
    Empty,
}

fn parse_slot(output: &str) -> Option<Slot> {
    let line = output
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    if line == "acquired" {
        Some(Slot::Acquired)
    } else if line == "empty" {
        Some(Slot::Empty)
    } else {
        line.strip_prefix("busy ")
            .map(|id| Slot::Busy(id.trim().to_owned()))
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

/// 占位里的作业是否还没到确认终态。Queued/Running 一定是；Finished 与调度系统确认的终态
/// （CANCELLED/FAILED/COMPLETED…，见 [`job::Report::terminal`]）才是结束。Unknown 不能当终态 ——
/// 作业目录还在时可能只是这次查不到。Lost 但调度器只是回答"查不到记录"（terminal=false）同样不能
/// 当结束：宁可拒绝重复提交，也不能赌它已经跑完。查询失败更不会走到这里 —— [`job::report`] 直接返回
/// 错误，提交中止且不释放占位（审查报告第 6 条）。
fn placeholder_unfinished(report: &job::Report, job_dir_exists: bool) -> bool {
    if report.terminal {
        return false;
    }
    match report.status.state {
        job::State::Unknown => job_dir_exists,
        job::State::Queued | job::State::Running | job::State::Lost => true,
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

fn job_dir_exists(ssh: &Ssh, base: &str, id: &str) -> Result<bool> {
    job::check_id(id)?;
    let script = format!(
        "test -d {} && echo yes || echo no\n",
        quote(&job::job_dir(base, id))
    );
    Ok(ssh.run_ok(&script)?.trim() == "yes")
}

/// 占位是否已经老到可以当"提交在 mkdir 与 submit 之间崩了"的残留。刚占住的占位在作业目录出现前
/// 本来就没有作业可查（status=unknown），这时不能立刻清掉：并发的另一份提交可能刚占位、正在提交。
/// 用占位目录的修改时间留一分钟宽限；超过仍没有作业目录，才清除。
fn placeholder_expired_script(project: &str) -> String {
    let placeholder = quote(&placeholder_path(project));
    format!(
        "if [ -d {placeholder} ] && find {placeholder} -maxdepth 0 -mmin +1 -print -quit 2>/dev/null | grep -q .; then echo stale; else echo fresh; fi\n"
    )
}

/// 提交前占位：占住就返回；占位里是还在跑的作业就拒绝；占位里的作业已确认终态（但 trap 没跑）
/// 就清掉陈旧占位后重试。最多循环几次：每次冲突都说明另一提交在推进状态，反复冲突时让用户稍后再试，
/// 不无限等。
fn claim_active_job(ssh: &Ssh, base: &str, project: &str, id: &str) -> Result<()> {
    for _ in 0..3 {
        let output = ssh.run_ok(&acquire_script(project, id))?;
        match parse_slot(&output).context("活动作业占位的响应读不懂")? {
            Slot::Acquired => return Ok(()),
            Slot::Empty => {
                ssh.run_ok(&remove_placeholder_script(project))?;
            }
            Slot::Busy(existing) => {
                // 占位里的内容先按作业号校验，再拼进命令/错误信息。
                if job::check_id(&existing).is_err() {
                    ssh.run_ok(&remove_placeholder_script(project))?;
                    continue;
                }
                let report = job::report(ssh, base, &existing, 5)?;
                let dir_exists = if matches!(report.status.state, job::State::Unknown) {
                    job_dir_exists(ssh, base, &existing)?
                } else {
                    false
                };
                if let Some(error) = active_job_refusal(&existing, &report, dir_exists) {
                    bail!("{error}");
                }
                // 作业号在、作业目录不在、状态还查不出来：可能是并发的另一份提交刚占位、还没提交出去。
                // 一分钟内不能清（否则两份提交都会提交成功、写同一个成果目录）；超时仍没有作业目录，
                // 才当"提交在 mkdir 与 submit 之间崩了"的残留清掉。
                if matches!(report.status.state, job::State::Unknown) && !dir_exists {
                    let expired = ssh.run_ok(&placeholder_expired_script(project))?;
                    if expired.trim() != "stale" {
                        bail!("{}", active_job_error(&existing));
                    }
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

/// `remote-bench-run`：写下配置，把单点模拟并成站点文件，再 `openbench check`、`openbench run`。
/// 作为服务器上的作业提交（按服务器的调度系统），返回作业号。
///
/// 提交前先在服务器上原子占住同名比较（GUI-07 服务器侧保护）：另一个作业还在跑就拒绝，避免两次
/// 提交交错写同一个成果目录；占位由作业体 EXIT trap 释放，提交失败时在这里释放。
pub(super) fn run(opts: &Opts) -> Result<()> {
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let base = super::remote_cmd::root_of(opts)?;
    let root = bench_root(opts)?;
    let request: RunRequest = read_json(opts)?;
    // 先校验并拼好作业体：名字/路径不合法时不占位，免得占位被一次不会提交的请求挡住。
    let project = project_dir(&root, &request.name)?;
    let id = unique_job("run")?;
    let body = format!(
        "{}{}",
        active_job_trap(&project, &id),
        run_body(&root, &request)?
    );
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
    claim_active_job(&ssh, &base, &project, &id)?;
    let submitted = match job::submit_plain(&ssh, &base, &id, &body, &spec) {
        Ok(submitted) => submitted,
        Err(error) => {
            // 提交失败：释放刚占的位，否则一次 SSH 抖动会让这个比较永远拒绝提交。释放失败时保留占位、
            // 留给下一次提交按陈旧判定清理；不能用清理失败盖掉真正的提交失败原因。
            if ssh.run_ok(&release_script(&project, &id)).is_err() {
                eprintln!("warning: cannot release the comparison's active-job placeholder");
            }
            return Err(error);
        }
    };
    println!(
        "{}",
        json!({
            "job": id,
            "submitted": submitted,
            "dir": project,
        })
    );
    Ok(())
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

fn run_body(root: &str, request: &RunRequest) -> Result<String> {
    ensure!(
        request.config.len() <= 512 * 1024,
        "the configuration exceeds 512 KiB"
    );
    let project = project_dir(root, &request.name)?;
    let mut body = format!(
        "{env}test -x {openbench} || {{ echo '先装 OpenBench' >&2; exit 1; }}\nmkdir -p {sim}\ncd {project_q}\n",
        env = program_env(root),
        openbench = env_program(root, "openbench"),
        sim = quote(&format!("{project}/sim")),
        project_q = quote(&project),
    );
    // 配置原样写下：经 here-document 传，定界符带上随机后缀，配置里出现同名行也截不断。
    let tag = format!("COLM_BENCH_CONFIG_{}", unique_job("cfg")?.replace('-', "_"));
    ensure!(
        !request.config.lines().any(|line| line == tag),
        "the configuration contains the here-document terminator"
    );
    body.push_str(&format!(
        "cat > openbench.yaml <<'{tag}'\n{}\n{tag}\n",
        request.config.trim_end_matches('\n')
    ));
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
        body.push_str(&format!(
            "echo {}\n",
            quote(&format!("合并 {} 的 history", site.id))
        ));
        // 位置参数顺序由 PREPARE 的签名决定：folder prefix out_nc out_csv station [lon] [lat]。
        // 经纬度只在两个都给时追加（缺省时 Python 回退到 history 文件里的坐标）。
        let mut prepare = vec![
            site.history_dir.clone(),
            site.prefix.clone(),
            format!("{project}/sim/{}.nc", site.id),
            format!("{project}/sim/{}.csv", site.id),
            site.station.clone(),
        ];
        if let Some((lon, lat)) = coordinate_pair(site)? {
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
    }
    // 内置登记表的路径写成 `${OPENBENCH_REF_ROOT}/…`；不设它 `check` 直接报错（本机 2026-10-23 实测）。
    if let Some(ref_root) = request.ref_root.as_deref().filter(|r| !r.trim().is_empty()) {
        server_path(ref_root, "ref_root")?;
        body.push_str(&format!("export OPENBENCH_REF_ROOT={}\n", quote(ref_root)));
    }
    for stage in ["check", "run"] {
        body.push_str(&format!(
            "echo 'openbench {stage} openbench.yaml'\n{} {stage} openbench.yaml\n",
            env_program(root, "openbench")
        ));
    }
    Ok(body)
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
    let status = &report.status;
    println!(
        "{}",
        json!({
            "job": id,
            "status": status,
            "ended": job::ended(status),
            "terminal": report.terminal,
            "cancel_requested": report.cancel_requested,
        })
    );
    Ok(())
}

/// `remote-bench-cancel`：结束作业（调度系统的交给它，后台进程结束整个进程组）。
pub(super) fn cancel(opts: &Opts) -> Result<()> {
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let base = super::remote_cmd::root_of(opts)?;
    let id = opts.need_str("--job")?;
    job::cancel(&ssh, &base, &id)?;
    println!("{}", json!({ "job": id, "request_accepted": true }));
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

#[derive(Deserialize)]
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
        ["Hour", "Day", "Month", "Year"].contains(&request.tim_res.as_str()),
        "bad time resolution {:?}",
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

/// `remote-bench-fetch`：只取回报告、指标与配置（`output/<项目>/{reports,metrics,scores}` 与 `openbench.yaml`），
/// 合并出来的站点文件和中间数据留在服务器上。
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
    let listing = ssh.run_ok(&format!(
        "cd {} && ls -d output/*/reports output/*/metrics output/*/scores openbench.yaml 2>/dev/null; true\n",
        quote(&project)
    ))?;
    let entries = listing
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.contains("..") && !l.starts_with('/'))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    ensure!(
        entries.iter().any(|e| e.ends_with("/reports")),
        "{name} on the server has no report yet"
    );
    ssh.download(&project, &entries, &out)?;
    println!("{}", json!({ "dir": out, "entries": entries }));
    Ok(())
}

#[cfg(test)]
#[path = "remote_bench_tests.rs"]
mod remote_bench_tests;
