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
//! colm-cli remote-bench-references --host H --root R
//! colm-cli remote-bench-register   --host H --root R --json '{"name","root_dir","data_type",…}'
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
    INSPECT_SCRIPT, OPENBENCH_VERSION, PREFETCH_SCRIPT, PREPARE_SCRIPT, PROGRAM_ENV,
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

/// `remote-bench-run`：写下配置，把单点模拟并成站点文件，再 `openbench check`、`openbench run`。
/// 作为服务器上的作业提交（按服务器的调度系统），返回作业号。
pub(super) fn run(opts: &Opts) -> Result<()> {
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let base = super::remote_cmd::root_of(opts)?;
    let root = bench_root(opts)?;
    let request: RunRequest = read_json(opts)?;
    let body = run_body(&root, &request)?;
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
    let id = unique_job("run")?;
    let submitted = job::submit_plain(&ssh, &base, &id, &body, &spec)?;
    println!(
        "{}",
        json!({
            "job": id,
            "submitted": submitted,
            "dir": format!("{root}/projects/{}", request.name),
        })
    );
    Ok(())
}

fn run_body(root: &str, request: &RunRequest) -> Result<String> {
    ensure!(
        valid_name(&request.name),
        "a comparison name may only use letters, digits, - and _: {:?}",
        request.name
    );
    ensure!(
        request.config.len() <= 512 * 1024,
        "the configuration exceeds 512 KiB"
    );
    let project = format!("{root}/projects/{}", request.name);
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
        body.push_str(&python_here(
            root,
            PREPARE_SCRIPT,
            &[
                &site.history_dir,
                &site.prefix,
                &format!("{project}/sim/{}.nc", site.id),
                &format!("{project}/sim/{}.csv", site.id),
                &site.station,
            ],
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
    let status = job::status(&ssh, &base, &id, lines)?;
    println!(
        "{}",
        json!({ "job": id, "status": status, "ended": job::ended(&status) })
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
pub(super) fn fetch(opts: &Opts) -> Result<()> {
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let root = bench_root(opts)?;
    let name = opts.need_str("--name")?;
    ensure!(valid_name(&name), "bad comparison name {name:?}");
    let out = std::path::PathBuf::from(opts.need_str("--out")?);
    ensure!(
        out.is_absolute(),
        "--out must be an absolute local directory"
    );
    let project = format!("{root}/projects/{name}");
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
