//! Persistent, isolated execution of pinned P2 workspaces on Linux servers.
use super::Opts;
use anyhow::{ensure, Context, Result};
use colm_remote::{
    auth, engine, job,
    ssh::{quote, Ssh},
    workspace::{self, Request},
};
use colm_workspace::{build, git, layout, Workspace};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Record {
    id: String,
    name: String,
    host: String,
    root: String,
    commit: String,
    base_commit: String,
    source_sha256: String,
    request: Request,
    created_at: u64,
    state: String,
    connection: Option<auth::Connection>,
    #[serde(default)]
    stale: bool,
    report_dir: PathBuf,
}
fn root(opts: &Opts) -> PathBuf {
    opts.get("--root")
        .map(PathBuf::from)
        .unwrap_or_else(layout::default_root)
}
fn records(ws: &Workspace) -> PathBuf {
    ws.reports().join("remote")
}
fn save(ws: &Workspace, r: &Record) -> Result<()> {
    let dir = records(ws).join(&r.id);
    std::fs::create_dir_all(&dir)?;
    let temp = dir.join(format!("record.{}.tmp", std::process::id()));
    std::fs::write(&temp, serde_json::to_vec_pretty(r)?)?;
    std::fs::rename(temp, dir.join("record.json"))?;
    Ok(())
}
fn load(ws: &Workspace, id: &str) -> Result<Record> {
    job::check_id(id)?;
    let mut r: Record =
        serde_json::from_slice(&std::fs::read(records(ws).join(id).join("record.json"))?)?;
    ensure!(
        r.id == id && r.name == ws.info.name,
        "job does not belong to this workspace"
    );
    validate_commit(&r.commit)?;
    validate_commit(&r.base_commit)?;
    workspace::check_absolute(&r.root)?;
    validate(&r.request)?;
    r.stale = ws.head()? != r.commit || git::is_dirty(&ws.src())?;
    Ok(r)
}
fn validate_commit(commit: &str) -> Result<()> {
    ensure!(
        [40, 64].contains(&commit.len()) && commit.bytes().all(|c| c.is_ascii_hexdigit()),
        "invalid pinned commit"
    );
    Ok(())
}
fn git_bytes(src: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let out = Command::new("git").arg("-C").arg(src).args(args).output()?;
    ensure!(
        out.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(out.stdout)
}
fn stage_source(ws: &Workspace, commit: &str, dest: &Path, name: &str) -> Result<String> {
    validate_commit(commit)?;
    let src = dest.join(name).join("src");
    std::fs::create_dir_all(&src)?;
    let archive = git_bytes(&ws.src(), &["archive", "--format=tar", commit])?;
    let links = workspace::unpack_source(&archive, &src)?;
    std::fs::write(
        dest.join(name).join("symlinks.json"),
        serde_json::to_vec(&links)?,
    )?;
    let tree = git_bytes(&ws.src(), &["ls-tree", "-rz", commit])?;
    let mut modes = std::collections::BTreeMap::new();
    for entry in tree.split(|b| *b == 0).filter(|e| !e.is_empty()) {
        let (header, path) = std::str::from_utf8(entry)?
            .split_once('\t')
            .context("invalid tree entry")?;
        if header.starts_with("100") {
            modes.insert(
                path,
                if header.starts_with("100755") {
                    0o755
                } else {
                    0o644
                },
            );
        }
    }
    std::fs::write(
        dest.join(name).join("modes.json"),
        serde_json::to_vec(&modes)?,
    )?;
    let raw = git_bytes(&ws.src(), &["cat-file", "commit", commit])?;
    std::fs::write(dest.join(name).join("commit.txt"), raw)?;
    let mut info = ws.info.clone();
    info.name = name.into();
    info.gates = Default::default();
    info.adopted.clear();
    info.branch = "remote-pinned".into();
    std::fs::write(
        dest.join(name).join("workspace.json"),
        serde_json::to_vec(&info)?,
    )?;
    Ok(workspace::sha256(&archive))
}
fn setup_repo(dir: &str, commit: &str) -> String {
    let d = quote(dir);
    let c = quote(commit);
    format!(
        r#"export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null
cd {d}/src
python3 - <<'COLM_LINK_PY'
import json,os
for path,mode in json.load(open('../modes.json')).items():
    os.chmod(path,mode)
for path,target in json.load(open('../symlinks.json')).items():
    os.symlink(target,path)
COLM_LINK_PY
git init -q
git -c core.autocrlf=false add -f --all
TREE=$(git write-tree)
EXPECTED=$(head -n 1 ../commit.txt | cut -d ' ' -f 2)
test "$TREE" = "$EXPECTED" || {{ echo 'source tree identity mismatch' >&2; exit 1; }}
ACTUAL=$(git hash-object -t commit -w ../commit.txt)
test "$ACTUAL" = {c}
printf '%s\n' {c} > .git/shallow
git update-ref HEAD {c}
"#
    )
}
fn submit(ws: &Workspace, opts: &Opts) -> Result<Value> {
    let request: Request = serde_json::from_str(&opts.need_str("--request")?)?;
    validate(&request)?;
    let commit = build::pin_commit(ws)?;
    let host = opts.need_str("--host")?;
    let remote_root = opts.need_str("--remote-root")?;
    workspace::check_absolute(&remote_root)?;
    let ssh = Ssh::new(&host)?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let id = format!("ws-{}-{nanos}", std::process::id());
    let staging = records(ws).join(&id).join("upload");
    std::fs::create_dir_all(&staging)?;
    let hash = stage_source(ws, &commit, &staging, "candidate")?;
    let base_hash = stage_source(ws, &ws.info.base_commit, &staging, "baseline")?;
    ensure!(
        build::still_at(ws, &commit)?,
        "workspace changed while preparing submission"
    );
    let mut record = Record {
        id: id.clone(),
        name: ws.info.name.clone(),
        host,
        root: remote_root,
        commit,
        base_commit: ws.info.base_commit.clone(),
        source_sha256: hash,
        request,
        created_at: layout::now(),
        state: "preparing".into(),
        connection: auth::current_connection(),
        stale: false,
        report_dir: records(ws).join(&id).join("results"),
    };
    save(ws, &record)?;
    let result = (|| -> Result<()> {
        let dir = job::job_dir(&record.root, &id);
        let remote_ws = format!("{dir}/workspaces");
        let resources = super::remote_cmd::resources_from(opts, 4, 1, 1)?;
        let cpus = resources.cpus;
        let env = resources.env_script.as_deref().unwrap_or("");
        ensure!(
            !env.lines().any(|l| l == "COLM_JOB_EOF"),
            "invalid environment script delimiter"
        );
        let fortran = if matches!(record.request.action.as_str(), "build-engine" | "test") {
            ""
        } else {
            "gfortran nf-config"
        };
        ssh.run_ok(&format!("set -e\n{env}\ntest \"$(uname -s)\" = Linux\nfor p in git cargo cc {fortran} bwrap python3 sha256sum; do command -v \"$p\" >/dev/null; done\nbwrap --ro-bind / / --unshare-net --unshare-pid --proc /proc -- /bin/true\nmkdir -p {}\nmkdir {}\n", quote(&format!("{}/jobs", record.root)), quote(&dir)))?;
        let spec = job::Spec {
            scheduler: super::remote_cmd::parse_scheduler(opts, &ssh, &record.root)?,
            resources,
            name: format!("colm-ws-{}", ws.info.name),
        };
        if let Some(case) = &record.request.case {
            ssh.run_ok(&format!(
                "test -f {}/case.nml && test \"$(realpath {})\" != \"$(realpath \"$HOME\")\"",
                quote(case),
                quote(case)
            ))?;
        }
        let app = super::remote_cmd::engine_source()?;
        if let engine::Source::Checkout(path) = &app {
            ensure!(
                !path.starts_with(ws.src()),
                "remote controller must come from application sources"
            );
        }
        let snapshot = engine::snapshot(&app)?;
        engine::upload(&ssh, &record.root, &snapshot)?;
        ssh.upload(
            &staging,
            &["candidate".into(), "baseline".into()],
            &remote_ws,
        )?;
        let identity = json!({"commit":record.commit,"base_commit":record.base_commit,"source_sha256":record.source_sha256,"baseline_sha256":base_hash,"request":record.request});
        std::fs::write(
            staging.join("provenance.json"),
            serde_json::to_vec(&identity)?,
        )?;
        std::fs::write(
            staging.join("request.json"),
            serde_json::to_vec(&record.request)?,
        )?;
        ssh.upload(
            &staging,
            &["provenance.json".into(), "request.json".into()],
            &dir,
        )?;
        let cli = format!(
            "{}/bin/colm-cli",
            engine::engine_dir(&record.root, &snapshot.id)
        );
        let mut body = format!(
            "export CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS={cpus} OMP_NUM_THREADS={cpus} GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null\necho bootstrap > phase\n{}",
            engine::ensure_script(&record.root, &snapshot.id, cpus).replace(&quote(&format!("{}/target", record.root.trim_end_matches('/'))), &quote(&format!("{dir}/bootstrap-target")))
        );
        body.push_str(&setup_repo(
            &format!("{remote_ws}/candidate"),
            &record.commit,
        ));
        body.push_str(&setup_repo(
            &format!("{remote_ws}/baseline"),
            &record.base_commit,
        ));
        // The controller receives only build configuration; source code cannot read SSH credentials.
        body.push_str(&sandbox_script(
            &dir,
            &cli,
            &remote_ws,
            cpus,
            record.request.case.as_deref(),
        ));
        record.state = "submitting".into();
        save(ws, &record)?;
        job::submit(&ssh, &record.root, &id, &body, &spec)?;
        record.state = "queued".into();
        save(ws, &record)?;
        Ok(())
    })();
    if let Err(error) = result {
        record.state = "unknown".into();
        save(ws, &record)?;
        return Ok(
            json!({"job":record,"status":{"state":"unknown","phase":"submission","log_tail":error.to_string()},"error":error.to_string()}),
        );
    }
    std::fs::remove_dir_all(&staging)?;
    Ok(json!({"job":record,"status":{"state":"queued","phase":"bootstrap","log_tail":""}}))
}
fn sandbox_script(dir: &str, cli: &str, workspace: &str, cpus: u32, case: Option<&str>) -> String {
    let controller = quote(cli.rsplit_once('/').expect("absolute controller path").0);
    let case_bind = case
        .map(|path| format!("--ro-bind {0} {0}", quote(path)))
        .unwrap_or_default();
    format!(
        r#"cd {d}
echo execute > phase
mkdir -p cargo-home home
REGISTRY_SOURCE="${{CARGO_HOME:-$HOME/.cargo}}/registry"
if [ -d "$REGISTRY_SOURCE" ]; then cp -a "$REGISTRY_SOURCE" cargo-home/; fi
SANDBOX=(bwrap --ro-bind / / --tmpfs /tmp --tmpfs /var/tmp --tmpfs /run --tmpfs "$HOME" --ro-bind-try "$HOME/.cargo/bin" "$HOME/.cargo/bin" --ro-bind-try "${{RUSTUP_HOME:-$HOME/.rustup}}" "${{RUSTUP_HOME:-$HOME/.rustup}}" --bind {d} {d} --ro-bind {controller} {controller} {case_bind} --dev /dev --proc /proc --unshare-net --unshare-pid --die-with-parent --clearenv --setenv PATH "$PATH" --setenv HOME {d}/home --setenv CARGO_HOME {d}/cargo-home --setenv RUSTUP_HOME "${{RUSTUP_HOME:-$HOME/.rustup}}" --setenv CARGO_NET_OFFLINE true --setenv CARGO_BUILD_JOBS {cpus} --setenv OMP_NUM_THREADS {cpus} --setenv GIT_CONFIG_GLOBAL /dev/null --setenv GIT_CONFIG_SYSTEM /dev/null)
if [ -n "${{CONDA_PREFIX:-}}" ] && [ "$CONDA_PREFIX" != "$HOME" ] && [ "$CONDA_PREFIX" != / ]; then
  SANDBOX+=(--ro-bind "$CONDA_PREFIX" "$CONDA_PREFIX" --setenv CONDA_PREFIX "$CONDA_PREFIX")
fi
for key in LD_LIBRARY_PATH LIBRARY_PATH PKG_CONFIG_PATH CPATH C_INCLUDE_PATH CPLUS_INCLUDE_PATH; do
  if [ -n "${{!key}}" ]; then SANDBOX+=(--setenv "$key" "${{!key}}"); fi
done
"${{SANDBOX[@]}}" -- {cli} remote-workspace --operation execute --root {wr} --name candidate --request-file {d}/request.json
echo complete > phase
"#,
        d = quote(dir),
        cli = quote(cli),
        wr = quote(workspace)
    )
}
fn validate(r: &Request) -> Result<()> {
    r.validate()?;
    build::check_preset(&r.preset)?;
    if r.action == "test" {
        colm_workspace::testrun::Kind::parse(
            r.kind.as_deref().unwrap_or("cargo"),
            r.package.as_deref().or(Some("colm-core")),
        )?;
    }
    if matches!(r.action.as_str(), "regress" | "verify") {
        ensure!(
            ["refactor", "physics"].contains(&r.kind.as_deref().unwrap_or("refactor")),
            "regression kind must be refactor or physics"
        );
    }
    Ok(())
}
fn verified_outer_root(opts: &Opts) -> Result<PathBuf> {
    ensure!(cfg!(target_os = "linux"), "remote execution requires Linux");
    let workspace_root = root(opts).canonicalize()?;
    let job_root = workspace_root
        .parent()
        .context("missing job root")?
        .to_owned();
    let mounts = std::fs::read_to_string("/proc/self/mountinfo")?;
    let escaped = job_root
        .to_string_lossy()
        .replace('\\', "\\134")
        .replace(' ', "\\040")
        .replace('\t', "\\011");
    let mount_options = |path: &str| {
        mounts
            .lines()
            .filter_map(|line| {
                let fields: Vec<_> = line.split_whitespace().collect();
                (fields.get(4) == Some(&path))
                    .then(|| fields.get(5).copied())
                    .flatten()
            })
            .next_back()
    };
    ensure!(
        mount_options("/").is_some_and(|s| s.split(',').any(|x| x == "ro")),
        "remote execute requires a readonly outer root mount"
    );
    ensure!(
        mount_options(&escaped).is_some_and(|s| s.split(',').any(|x| x == "rw")),
        "remote execute requires a private writable job mount"
    );
    let init = std::fs::read_to_string("/proc/1/comm")?;
    ensure!(
        ["bwrap", "colm-cli"].contains(&init.trim()),
        "remote execute requires a private PID namespace"
    );
    let devices = std::fs::read_to_string("/proc/net/dev")?;
    ensure!(
        devices
            .lines()
            .filter_map(|l| l.split_once(':'))
            .all(|(name, _)| name.trim() == "lo"),
        "remote execute requires an isolated network namespace"
    );
    ensure!(
        std::env::var_os("HOME").map(PathBuf::from) == Some(job_root.join("home"))
            && std::env::var_os("CARGO_HOME").map(PathBuf::from)
                == Some(job_root.join("cargo-home")),
        "remote execute requires private home and cargo directories"
    );
    Ok(job_root)
}
fn execute(opts: &Opts) -> Result<()> {
    let job_root = verified_outer_root(opts)?;
    colm_workspace::sandbox::with_outer_sandbox(&job_root, || execute_in_sandbox(opts))
}
fn execute_in_sandbox(opts: &Opts) -> Result<()> {
    let req: Request = serde_json::from_slice(&std::fs::read(opts.need_str("--request-file")?)?)?;
    validate(&req)?;
    let mut ws = Workspace::open(&root(opts), "candidate")?;
    let mut baseline = Workspace::open(&root(opts), "baseline")?;
    let mut results = Vec::new();
    let result_dir = ws.dir.parent().context("workspace parent")?.join("results");
    let mut keep = |value: Value| -> Result<()> {
        let ok = value
            .get("ok")
            .and_then(Value::as_bool)
            .or_else(|| value.pointer("/outcome/ok").and_then(Value::as_bool))
            .unwrap_or(false);
        results.push(value);
        std::fs::create_dir_all(&result_dir)?;
        std::fs::write(
            result_dir.join("outcomes.json"),
            serde_json::to_vec_pretty(&json!({"ok":ok,"results":results}))?,
        )?;
        ensure!(
            ok,
            "remote workspace check failed; inspect outcomes and logs"
        );
        Ok(())
    };
    let action = req.action.as_str();
    if matches!(action, "regress" | "verify") {
        keep(json!(build::build_engine(&mut baseline, false, None)?))?;
        keep(json!(build::build_kernel(
            &mut baseline,
            &req.preset,
            None
        )?))?;
    }
    if action != "build-kernel" && action != "test" {
        keep(json!(build::build_engine(&mut ws, false, None)?))?;
    }
    if matches!(
        action,
        "build-kernel" | "run" | "parity" | "regress" | "verify"
    ) {
        keep(json!(build::build_kernel(&mut ws, &req.preset, None)?))?;
    }
    if matches!(action, "test" | "verify") {
        let kind = if action == "verify" {
            "cargo"
        } else {
            req.kind.as_deref().unwrap_or("cargo")
        };
        let kind = colm_workspace::testrun::Kind::parse(
            kind,
            req.package.as_deref().or(Some("colm-core")),
        )?;
        keep(json!(colm_workspace::testrun::run(&mut ws, &kind, None)?))?;
    }
    let case = req.case.as_deref().map(Path::new);
    if action == "run" {
        keep(json!(colm_workspace::parity::run_case_with(
            &ws,
            case.unwrap(),
            req.engine.as_deref().unwrap_or("rust"),
            &req.preset,
            None
        )?))?;
    }
    if matches!(action, "parity" | "verify") {
        let options = colm_workspace::compare::Options {
            tolerance: colm_workspace::compare::Tolerance {
                rtol: req.rtol.unwrap_or(0.0),
                atol: req.atol.unwrap_or(0.0),
            },
            first_records: req.first_records,
            ignore: req.ignore.clone().unwrap_or_default(),
        };
        keep(json!(colm_workspace::parity::parity_check(
            &mut ws,
            case.unwrap(),
            &req.preset,
            &options,
            None
        )?))?;
    }
    if matches!(action, "regress" | "verify") {
        let base = colm_workspace::parity::Baseline {
            cli: baseline.bin().join("colm-cli"),
            kernel: baseline.kernels().join(&req.preset),
        };
        let kind = if req.kind.as_deref() == Some("physics") {
            colm_workspace::gates::ChangeKind::Physics
        } else {
            colm_workspace::gates::ChangeKind::Refactor
        };
        keep(json!(colm_workspace::parity::regress(
            &mut ws,
            case.unwrap(),
            &req.preset,
            req.engine.as_deref().unwrap_or("rust"),
            &base,
            kind,
            None
        )?))?;
    }
    Ok(())
}
fn compact_gates(value: &Value) -> Value {
    if value.get("commit").is_some() {
        return json!({"ok":value["ok"],"commit":value["commit"],"at":value["at"]});
    }
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), compact_gates(v)))
                .collect(),
        ),
        _ => value.clone(),
    }
}
fn fetch(ssh: &Ssh, ws: &Workspace, record: &Record, verify_source: bool) -> Result<Value> {
    let dir = job::job_dir(&record.root, &record.id);
    if verify_source {
        ssh.run_ok(&format!("set -e\nexport GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null\nfor pair in candidate:{} baseline:{}; do name=${{pair%%:*}}; expected=${{pair#*:}}; cd {}/workspaces/\"$name\"/src; test \"$(git rev-parse HEAD)\" = \"$expected\"; test -z \"$(git status --porcelain)\"; done\n", record.commit, record.base_commit, quote(&dir)))?;
    }
    let script = format!(
        r#"python3 - {dir} <<'COLM_FETCH_PY'
import os,sys,json,stat
root=os.path.realpath(sys.argv[1]); result={{}}; total=0
for rel in ['provenance.json','log','workspaces/results','workspaces/candidate/reports','workspaces/baseline/reports','workspaces/candidate/workspace.json','workspaces/baseline/workspace.json']:
    base=os.path.join(root,rel)
    paths=[]
    if os.path.isdir(base):
        for d,dirs,files in os.walk(base,followlinks=False):
            dirs[:]=[n for n in dirs if not os.path.islink(os.path.join(d,n))]
            paths.extend(os.path.join(d,n) for n in files)
    elif os.path.exists(base): paths=[base]
    for path in paths:
        if not stat.S_ISREG(os.lstat(path).st_mode) or os.path.commonpath([root,os.path.realpath(path)])!=root: raise RuntimeError('unsafe report path')
        data=open(path,'rb').read(16777217)
        total+=len(data)
        if len(data)>16777216 or total>67108864: raise RuntimeError('report exceeds transfer limit')
        result[os.path.relpath(path,root)]=data.decode('utf-8')
print(json.dumps(result))
COLM_FETCH_PY
"#,
        dir = quote(&dir)
    );
    let files: std::collections::BTreeMap<String, String> =
        serde_json::from_str(&ssh.run_ok(&script)?)?;
    let identity: Value =
        serde_json::from_str(files.get("provenance.json").context("missing provenance")?)?;
    ensure!(
        identity["commit"] == record.commit
            && identity["base_commit"] == record.base_commit
            && identity["source_sha256"] == record.source_sha256
            && identity["request"] == json!(record.request),
        "remote result provenance mismatch"
    );
    let outcomes: Value = files
        .get("workspaces/results/outcomes.json")
        .map(|s| serde_json::from_str(s))
        .transpose()?
        .unwrap_or(Value::Null);
    let summaries: Vec<Value> = outcomes["results"].as_array().into_iter().flatten().map(|v| json!({
        "ok":v.get("ok").or_else(||v.pointer("/outcome/ok")),
        "command":v["command"].as_str().map(|s|s.chars().take(200).collect::<String>()),
        "verdict":v["verdict"].as_str().map(|s|s.chars().take(1000).collect::<String>()),
        "first_difference":v["first_difference"],
        "report":v["report"],
        "compare": {"files":v["compare"]["files"],"differs":v["compare"]["differs"],"new_nonfinite":v["compare"]["new_nonfinite"],"structural_differences":v["compare"]["structural_differences"]}
    })).collect();
    let mut gates = serde_json::Map::new();
    for name in ["candidate", "baseline"] {
        if let Some(text) = files.get(&format!("workspaces/{name}/workspace.json")) {
            let info: Value = serde_json::from_str(text)?;
            gates.insert(name.into(), compact_gates(&info["gates"]));
        }
    }
    let evidence = json!({"execution_succeeded":record.state=="finished","commit":record.commit,"base_commit":record.base_commit,"stale":record.stale,"outcomes":summaries,"gates":gates});
    let out = &record.report_dir;
    ensure!(
        *out == records(ws).join(&record.id).join("results"),
        "unexpected report directory"
    );
    let mut prefix = ws.dir.clone();
    for component in Path::new("reports/remote")
        .components()
        .chain(Path::new(&record.id).components())
        .chain(Path::new("results").components())
    {
        prefix.push(component);
        if let Ok(meta) = std::fs::symlink_metadata(&prefix) {
            ensure!(
                !meta.file_type().is_symlink(),
                "local report symlink rejected"
            );
        }
    }
    for (path, text) in files {
        let path = Path::new(&path);
        workspace::safe_source_path(path)?;
        let dest = out.join(path);
        let mut cursor = out.clone();
        for component in path.components() {
            cursor.push(component);
            if let Ok(meta) = std::fs::symlink_metadata(&cursor) {
                ensure!(
                    !meta.file_type().is_symlink(),
                    "local report symlink rejected"
                );
            }
        }
        std::fs::create_dir_all(dest.parent().unwrap())?;
        std::fs::write(dest, text)?;
    }
    Ok(evidence)
}
pub(crate) fn run(opts: &Opts) -> Result<()> {
    let operation = opts.need_str("--operation")?;
    if operation == "execute" {
        return execute(opts);
    }
    let ws = Workspace::open(&root(opts), &opts.need_str("--name")?)?;
    if operation == "list" {
        let mut jobs = Vec::new();
        if records(&ws).exists() {
            for entry in std::fs::read_dir(records(&ws))? {
                let entry = entry?;
                if entry.path().join("record.json").is_file() {
                    jobs.push(load(&ws, &entry.file_name().to_string_lossy())?);
                }
            }
        }
        jobs.sort_by_key(|r| std::cmp::Reverse(r.created_at));
        println!("{}", json!({"jobs":jobs}));
        return Ok(());
    }
    if operation == "submit" {
        println!("{}", submit(&ws, opts)?);
        return Ok(());
    }
    ensure!(
        ["status", "cancel", "fetch"].contains(&operation.as_str()),
        "unknown remote workspace operation"
    );
    let mut record = load(&ws, &opts.need_str("--job")?)?;
    auth::check_connection(record.connection.as_ref())?;
    let ssh = Ssh::new(&record.host)?;
    if operation == "cancel" {
        job::cancel(&ssh, &record.root, &record.id)?;
    }
    let status = job::status(&ssh, &record.root, &record.id, 80)?;
    record.state = match status.state {
        job::State::Queued => "queued",
        job::State::Running => "running",
        job::State::Finished { exit_code: 0 } => "finished",
        job::State::Finished { .. } => "failed",
        job::State::Lost => "lost",
        job::State::Unknown => "unknown",
    }
    .into();
    save(&ws, &record)?;
    let evidence = if operation == "fetch" {
        ensure!(
            matches!(status.state, job::State::Finished { .. }),
            "only finished jobs can be fetched; unknown/lost jobs must be checked first"
        );
        fetch(
            &ssh,
            &ws,
            &record,
            matches!(status.state, job::State::Finished { exit_code: 0 }),
        )?
    } else {
        Value::Null
    };
    println!(
        "{}",
        json!({"job":record,"status":status,"evidence":evidence})
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (PathBuf, Workspace) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "colm-remote-workspace-{}-{unique}-{sequence}",
            std::process::id()
        ));
        let src = root.join("work/src");
        std::fs::create_dir_all(&src).unwrap();
        git::run(&src, &["init", "-q"]).unwrap();
        git::set_identity(&src).unwrap();
        std::fs::write(src.join("file.txt"), "original\n").unwrap();
        git::run(&src, &["add", "."]).unwrap();
        git::run(&src, &["commit", "-qm", "fixture"]).unwrap();
        let commit = git::head(&src).unwrap();
        let ws = Workspace {
            dir: root.join("work"),
            info: layout::Info {
                name: "work".into(),
                created_at: 0,
                origin: "fixture".into(),
                base_commit: commit,
                branch: "main".into(),
                gates: Default::default(),
                adopted: vec![],
            },
        };
        ws.save().unwrap();
        (root, ws)
    }
    #[test]
    fn records_survive_restart_and_mark_edits_stale() {
        let (root, ws) = fixture();
        let r = Record {
            id: "ws-test".into(),
            name: "work".into(),
            host: "server".into(),
            root: "/data/private".into(),
            commit: ws.head().unwrap(),
            base_commit: ws.info.base_commit.clone(),
            source_sha256: "abc".into(),
            request: serde_json::from_value(json!({"action":"test"})).unwrap(),
            created_at: 1,
            state: "submitting".into(),
            connection: None,
            stale: false,
            report_dir: records(&ws).join("ws-test/results"),
        };
        save(&ws, &r).unwrap();
        assert!(!load(&ws, "ws-test").unwrap().stale);
        std::fs::write(ws.src().join("file.txt"), "edited").unwrap();
        let loaded = load(&ws, "ws-test").unwrap();
        assert!(loaded.stale);
        assert_eq!(loaded.state, "submitting");
        assert!(load(&ws, "../ws-test").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    #[cfg(unix)]
    fn archive_reconstructs_exact_commit_without_history() {
        let (root, ws) = fixture();
        std::os::unix::fs::symlink("file.txt", ws.src().join("internal-link")).unwrap();
        git::run(&ws.src(), &["add", "internal-link"]).unwrap();
        git::run(&ws.src(), &["commit", "-qm", "internal link fixture"]).unwrap();
        let head = ws.head().unwrap();
        let stage = root.join("staging");
        stage_source(&ws, &head, &stage, "candidate").unwrap();
        let script = setup_repo(stage.join("candidate").to_str().unwrap(), &head);
        let status = Command::new("bash")
            .arg("-ec")
            .arg(script)
            .status()
            .unwrap();
        assert!(status.success());
        let repo = stage.join("candidate/src");
        assert_eq!(git::head(&repo).unwrap(), head);
        assert!(!git::is_dirty(&repo).unwrap());
        std::fs::remove_file(repo.join("internal-link")).unwrap();
        std::fs::write(repo.join("file.txt"), "tampered").unwrap();
        assert!(!Command::new("bash")
            .arg("-ec")
            .arg(setup_repo(stage.join("candidate").to_str().unwrap(), &head))
            .status()
            .unwrap()
            .success());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn committed_credentials_rejected_before_upload() {
        let (root, ws) = fixture();
        std::fs::write(ws.src().join(".env"), "secret").unwrap();
        git::run(&ws.src(), &["add", "-f", ".env"]).unwrap();
        git::run(&ws.src(), &["commit", "-qm", "secret fixture"]).unwrap();
        assert!(stage_source(&ws, &ws.head().unwrap(), &root.join("stage"), "candidate").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    #[cfg(unix)]
    fn sandbox_failure_stops_job_and_exit_code_is_preserved() {
        let (root, ws) = fixture();
        let jobdir = root.join("jobs/stub");
        std::fs::create_dir_all(&jobdir).unwrap();
        let bin = root.join("stub-bin");
        std::fs::create_dir_all(&bin).unwrap();
        let stub = bin.join("bwrap");
        std::fs::write(
            &stub,
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$STUB_ARGS\"\nexit 23\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        let script = job::job_script(
            root.to_str().unwrap(),
            "stub",
            &sandbox_script(
                jobdir.to_str().unwrap(),
                "/trusted/colm-cli",
                "/job/workspaces",
                2,
                None,
            ),
            &job::Spec::bare(),
        )
        .unwrap();
        assert!(script.contains("REGISTRY_SOURCE=\"${CARGO_HOME:-$HOME/.cargo}/registry\""));
        let status = Command::new("bash")
            .arg("-c")
            .arg(script)
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .env("HOME", &ws.dir)
            .env("STUB_ARGS", jobdir.join("args"))
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(
            std::fs::read_to_string(jobdir.join("exit_code"))
                .unwrap()
                .trim(),
            "23"
        );
        assert_eq!(
            std::fs::read_to_string(jobdir.join("phase"))
                .unwrap()
                .trim(),
            "execute"
        );
        let args = std::fs::read_to_string(jobdir.join("args")).unwrap();
        assert!(
            args.contains("--clearenv")
                && args.contains("--tmpfs")
                && args.contains("--unshare-net")
        );
        // Masking /tmp after the explicit mounts would hide valid scratch workspaces.
        let scratch = sandbox_script(
            "/tmp/colm/jobs/id",
            "/tmp/colm/engine/bin/colm-cli",
            "/tmp/colm/jobs/id/workspaces",
            2,
            Some("/tmp/site"),
        );
        let mask = scratch.find("--tmpfs /tmp").unwrap();
        for mount in [
            "--bind '/tmp/colm/jobs/id'",
            "--ro-bind '/tmp/colm/engine/bin'",
            "--ro-bind '/tmp/site'",
        ] {
            assert!(
                mask < scratch.find(mount).unwrap(),
                "mask must precede {mount}"
            );
        }
        assert!(args.contains("--unshare-pid"));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn fixed_recipe_rejects_bad_parameters() {
        for value in [
            json!({"action":"shell"}),
            json!({"action":"verify","case":"/case","kind":"cargo"}),
            json!({"action":"run","case":"../case"}),
            json!({"action":"test","package":"x;id"}),
            json!({"action":"test","preset":"invalid"}),
        ] {
            let req: Request = serde_json::from_value(value).unwrap();
            assert!(validate(&req).is_err());
        }
    }
}
