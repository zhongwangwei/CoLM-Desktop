use super::*;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use colm_remote::ssh::quote;
use serde_json::json;

fn request(config: &str) -> RunRequest {
    RunRequest {
        name: "versions_vs_flux".into(),
        config: config.into(),
        sites: vec![Site {
            id: "AT_Neu".into(),
            station: "AT-Neu".into(),
            history_dir: "/data/x y/out/AT-Neu/history".into(),
            prefix: "AT-Neu_hist_".into(),
            lon: None,
            lat: None,
        }],
        ref_root: Some("/data/ref".into()),
    }
}

/// 不需要 PREPARE 的请求（没有单点模拟）：端到端沙箱里只演 check/run 这一步。
fn frozen_request(config: &str) -> RunRequest {
    RunRequest {
        name: "versions_vs_flux".into(),
        config: config.into(),
        sites: Vec::new(),
        ref_root: None,
    }
}

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("colm-bench-{tag}-{}-{nanos}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 经 stdin 把脚本交给 bash：和 `Ssh::run` 一样，避免把整段脚本塞进命令行参数。
fn bash_with_stdin(args: &[&str], script: &str) -> (bool, String, String) {
    use std::io::Write;
    let mut child = Command::new("bash")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(script.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn bash_script(script: &str) -> (bool, String, String) {
    let output = Command::new("bash").arg("-c").arg(script).output().unwrap();
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// 生成的作业脚本整份交给 `bash -n` 查语法：作业体是拼出来的 shell，引号或 here-document 出错要在这里暴露。
#[cfg(unix)]
fn assert_bash_parses(body: &str) {
    let script = job::job_script("/srv/colm", "bench-run-1", body, &Spec::bare()).unwrap();
    let (success, _, stderr) = bash_with_stdin(&["-n"], &script);
    assert!(success, "{stderr}\n{script}");
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_str(&read(path)).unwrap()
}

#[test]
fn unix_seconds_become_the_same_iso_timestamp_the_window_process_writes() {
    assert_eq!(iso8601_utc(0), "1970-01-01T00:00:00Z");
    assert_eq!(iso8601_utc(1_700_000_000), "2023-11-14T22:13:20Z");
    assert_eq!(iso8601_utc(1_700_000_001), "2023-11-14T22:13:21Z");
}

#[test]
fn the_submit_intent_records_the_pre_generated_job_id_before_submitting() {
    let intent = submit_intent("bench-run-7-123", 1_700_000_000, "abc123");
    let value: serde_json::Value = serde_json::from_str(&intent).unwrap();
    assert_eq!(value["id"], "bench-run-7-123");
    assert_eq!(value["created_unix"], 1_700_000_000u64);
    assert_eq!(value["config_sha"], "abc123");
    // 清理/释放脚本用 `sed -n 's/^ *"id": *"..."'` 读它：必须一行一个字段。
    assert!(
        intent
            .lines()
            .any(|line| line.trim_start().starts_with("\"id\":")),
        "{intent}"
    );
}

// ---- R03：远程 output_dir 只能落在本次运行目录内 ----

#[test]
fn remote_output_dir_is_normalized_inside_the_run_directory() {
    // 没写 project.output_dir：沿用 OpenBench 的默认 output。
    assert_eq!(output_dir_from_config("x: 1\n").unwrap(), "output");
    assert_eq!(
        output_dir_from_config("project:\n  name: cmp\n  output_dir: ./custom-output/\n").unwrap(),
        "custom-output"
    );
    assert_eq!(
        output_dir_from_config("project:\n  output_dir: \"a/b\"\n").unwrap(),
        "a/b"
    );
    // 词法上能落回运行目录的 `..` 安全归一，不拒绝。
    assert_eq!(
        output_dir_from_config("project:\n  output_dir: sim/../out\n").unwrap(),
        "out"
    );
    for bad in [
        "../shared",
        "/srv/shared",
        "a/../../out",
        ".",
        "sim/..",
        "~/shared",
    ] {
        let error =
            output_dir_from_config(&format!("project:\n  output_dir: {bad}\n")).unwrap_err();
        let error = error.to_string();
        assert!(error.contains("output_dir"), "{bad:?}: {error}");
    }
    // `output_dir:` 空值在 YAML 是 null：用默认 output，不拒绝。
    assert_eq!(
        output_dir_from_config("project:\n  output_dir:\n").unwrap(),
        "output"
    );
    // 不写在 project 段的 output_dir 不能猜：OpenBench 不会读它，猜错就绕过了隔离。
    assert!(output_dir_from_config("output_dir: /tmp/x\nproject:\n  name: cmp\n").is_err());
    // 嵌在 project 的子块里同样不是 project.output_dir。
    assert!(output_dir_from_config("project:\n  nested:\n    output_dir: ./x\n").is_err());
    // 这些写法能让 OpenBench 看到另一份 output_dir 而逐行扫描看不到：一律拒绝，不猜。
    for unsupported in [
        "project: {name: cmp, output_dir: /tmp/x}\n",
        "project:\n  <<: *defaults\n",
        "---\nproject:\n  output_dir: ./x\n",
        "project:\n  name: cmp\nproject:\n  output_dir: ./x\n",
    ] {
        assert!(
            output_dir_from_config(unsupported).is_err(),
            "{unsupported:?}"
        );
    }
}

/// R03：运行记录里的输出根/报告路径要拼进取回清单，绝对路径与 `..` 不能通过。
#[test]
fn recorded_run_paths_stay_inside_the_run_directory() {
    assert_eq!(
        run_relative_path("results/demo/reports/evaluation_report.html", "report").unwrap(),
        "results/demo/reports/evaluation_report.html"
    );
    for bad in ["../shared", "/tmp/x", "a/../../x", ".", "a//../../x"] {
        assert!(run_relative_path(bad, "report").is_err(), "{bad:?}");
    }
}

#[test]
fn the_run_job_records_and_uses_the_configured_output_dir() {
    let body = run_body(
        "/srv/colm",
        "/srv/colm/openbench",
        &request("project:\n  name: cmp\n  output_dir: ./results\n"),
        "bench-run-1",
        0,
    )
    .unwrap();
    let project = "/srv/colm/openbench/projects/versions_vs_flux";
    let run = format!("{project}/runs/bench-run-1");
    assert!(body.contains("\"output_dir\": \"results\""), "{body}");
    // R04：准备结果落进本次运行目录；配置里的 fulllist 位置只放一份指向它的清单副本。
    assert!(body.contains(&format!("'{run}/sim/AT_Neu.nc'")), "{body}");
    assert!(body.contains(&format!("'{run}/sim/AT_Neu.csv'")), "{body}");
    assert!(
        !body.contains(&format!("'{project}/sim/AT_Neu.nc'")),
        "共享目录不能再是准备结果的落点：{body}"
    );
    assert!(
        body.contains(&format!(
            "cp '{run}/sim/AT_Neu.csv' '{project}/sim/AT_Neu.csv'"
        )),
        "{body}"
    );
    #[cfg(unix)]
    assert_bash_parses(&body);
}

// ---- 远程 run 的作业体（C07） ----

#[test]
fn the_run_job_freezes_a_run_snapshot_and_runs_inside_it() {
    let config = "general:\n  name: \"versions_vs_flux\"\n  note: it's $HOME `x`\n";
    let body = run_body(
        "/srv/colm",
        "/srv/colm/openbench",
        &request(config),
        "bench-run-9",
        1_700_000_000,
    )
    .unwrap();
    let project = "/srv/colm/openbench/projects/versions_vs_flux";
    let run = format!("{project}/runs/bench-run-9");
    // 冻结文件都写在这次运行的目录里；配置用 printf 原样落盘（不经过 here-document / 变量展开）。
    assert!(
        body.contains(&format!("mkdir -p '{project}/runs'")),
        "{body}"
    );
    assert!(body.contains(&format!("mkdir '{run}'")), "{body}");
    assert!(body.contains(&format!("cd '{run}'")), "{body}");
    assert!(body.contains("> openbench.yaml"), "{body}");
    assert!(body.contains("> sites.json"), "{body}");
    assert!(body.contains("> inputs.json"), "{body}");
    assert!(body.contains("> meta.json"), "{body}");
    assert!(body.contains("\"config_sha\":"), "{body}");
    assert!(
        body.contains("\"created_at\": \"2023-11-14T22:13:20Z\""),
        "{body}"
    );
    assert!(body.contains("\"state\": \"running\""), "{body}");
    // check/run 必须在这次运行的目录里执行（./output 落在 run 目录），不能回到项目根。
    let cd = body.find(&format!("cd '{run}'")).unwrap();
    let check = body.find(" check openbench.yaml").unwrap();
    let run_stage = body.find(" run openbench.yaml").unwrap();
    assert!(cd < check && check < run_stage, "{body}");
    // R04：准备结果在本次运行目录里冻结，配置文本指向的项目 sim/ 只放指向它的清单副本。
    assert!(
        body.contains(&format!("mkdir -p '{project}/sim'")),
        "{body}"
    );
    assert!(body.contains(&format!("'{run}/sim/AT_Neu.nc'")), "{body}");
    assert!(
        body.contains(&format!(
            "cp '{run}/sim/AT_Neu.csv' '{project}/sim/AT_Neu.csv'"
        )),
        "{body}"
    );
    assert!(
        body.contains("export OPENBENCH_REF_ROOT='/data/ref'"),
        "{body}"
    );
    assert!(
        body.contains("export OPENBENCH_HOME='/srv/colm/openbench/home'"),
        "{body}"
    );
    #[cfg(unix)]
    assert_bash_parses(&body);
}

#[test]
fn the_run_job_installs_its_finish_trap_before_doing_any_work() {
    let body = run_body(
        "/srv/colm",
        "/srv/colm/openbench",
        &frozen_request("x"),
        "bench-run-7",
        1_700_000_000,
    )
    .unwrap();
    // 收尾 trap 在真正干活之前装好：评估失败/取消时也要写 meta、释放占位。
    let trap = body.find("trap _colm_finish_run EXIT").unwrap();
    let first_work = body.find("test -x").unwrap();
    assert!(trap < first_work, "{body}");
    assert!(body.contains("_colm_finish_run() {"), "{body}");
    assert!(body.contains("_colm_release_active_job"), "{body}");
    // 锁内核对所有权再释放，且 lock helper 在 trap 之前定义。
    let lock = body.find("_colm_lock_acquire() {").unwrap();
    assert!(lock < trap, "{body}");
    assert!(body.contains("rm -rf"), "{body}");
    #[cfg(unix)]
    assert_bash_parses(&body);
}

#[test]
fn the_run_job_refuses_unsafe_names_paths_and_terminators() {
    let base = "/srv/colm";
    let root = "/srv/colm/openbench";
    let mut bad = request("x");
    bad.name = "../escape".into();
    assert!(run_body(base, root, &bad, "bench-run-1", 0).is_err());
    let mut bad = request("x");
    bad.sites[0].station = "AT,Neu".into();
    assert!(run_body(base, root, &bad, "bench-run-1", 0).is_err());
    let mut bad = request("x");
    bad.sites[0].history_dir = "relative/history".into();
    assert!(run_body(base, root, &bad, "bench-run-1", 0).is_err());
    let mut bad = request("x");
    bad.sites[0].prefix = "a*b_hist_".into();
    assert!(run_body(base, root, &bad, "bench-run-1", 0).is_err());
    let mut bad = request("x");
    bad.ref_root = Some("data/ref".into());
    assert!(run_body(base, root, &bad, "bench-run-1", 0).is_err());
    // 配置里有 NUL 字节时 shell 无法原样写下，宁可提前拒绝。
    let mut bad = request("a\0b");
    bad.sites.clear();
    assert!(run_body(base, root, &bad, "bench-run-1", 0).is_err());
}

#[test]
fn the_run_job_refuses_an_oversized_configuration() {
    let mut huge = request("x");
    huge.config = "a".repeat(512 * 1024 + 1);
    huge.sites.clear();
    assert!(run_body("/srv/colm", "/srv/colm/openbench", &huge, "bench-run-1", 0).is_err());
}

/// PREPARE 的签名是 `folder prefix out_nc out_csv station [lon] [lat]`：经纬度是成对的可选参数。
/// 缺省时必须停在站点名（多传空参数会让脚本把空串当坐标）；两个都给时按位置紧跟在站点名后面。
#[test]
fn station_coordinates_reach_prepare_only_when_both_are_given() {
    let base = "/srv/colm";
    let root = "/srv/colm/openbench";
    let plain = run_body(base, root, &request("x"), "bench-run-1", 0).unwrap();
    assert!(!plain.contains("'-77.5'"), "{plain}");
    let mut located = request("x");
    located.sites[0].lon = Some(-77.5);
    located.sites[0].lat = Some(40.25);
    let body = run_body(base, root, &located, "bench-run-1", 0).unwrap();
    assert!(
        body.contains("'AT-Neu' '-77.5' '40.25'"),
        "经纬度要成对跟在 station 后面：{body}"
    );
    #[cfg(unix)]
    assert_bash_parses(&body);
    // 只给一半：签名是成对的，宁可拒绝也不写出半份坐标。
    let mut half = request("x");
    half.sites[0].lon = Some(-77.5);
    assert!(run_body(base, root, &half, "bench-run-1", 0).is_err());
    // 范围与 Tauri 侧一致：|lat| <= 90、|lon| <= 360。
    let mut out_of_range = request("x");
    out_of_range.sites[0].lon = Some(400.0);
    out_of_range.sites[0].lat = Some(0.0);
    assert!(run_body(base, root, &out_of_range, "bench-run-1", 0).is_err());
}

// ---- 远程 run 的端到端沙箱：冻结、成功指针、失败只写自己 meta（C07） ----

/// 找一个能跑收尾脚本的 python3。优先 PATH 上的（开发机常见 miniforge/homebrew），再退回系统
/// 路径；`/usr/bin/python3` 可能只是 Xcode 的"请安装命令行工具"壳，所以不排在最前。
#[cfg(unix)]
fn system_python3() -> Option<PathBuf> {
    let found = Command::new("bash")
        .arg("-c")
        .arg("command -v python3")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|path| !path.is_empty());
    found
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .or_else(|| {
            [
                "/opt/homebrew/bin/python3",
                "/usr/local/bin/python3",
                "/usr/bin/python3",
            ]
            .iter()
            .map(PathBuf::from)
            .find(|path| path.is_file())
        })
}

#[cfg(unix)]
fn write_executable(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// 一个假"服务器根"：`<base>/openbench/env/bin/{python,openbench}` + `<base>/jobs/<作业号>/`。
/// `openbench` 是本地 bash 替身，`python` 转给系统 python3（收尾脚本只依赖标准库）。
#[cfg(unix)]
struct RunSandbox {
    base: PathBuf,
    root: PathBuf,
}

#[cfg(unix)]
impl RunSandbox {
    fn new(tag: &str, failing: bool) -> Self {
        Self::new_with_output(tag, failing, "output")
    }

    /// 指定 OpenBench 的输出目录（R03：自定义 output_dir 的收尾也要能找到报告）。
    fn new_with_output(tag: &str, failing: bool, output_dir: &str) -> Self {
        let base = temp_dir(&format!("run-{tag}"));
        let root = base.join("openbench");
        std::fs::create_dir_all(root.join("env/bin")).unwrap();
        std::fs::create_dir_all(root.join("projects")).unwrap();
        let python = system_python3().expect("the run-job sandbox needs a python3");
        write_executable(
            &root.join("env/bin/python"),
            &format!(
                "#!/bin/bash\nexec {} \"$@\"\n",
                quote(&python.to_string_lossy())
            ),
        );
        let stub = if failing {
            r#"#!/bin/bash
case "$1" in
check) echo 'check ok'; exit 0;;
run) echo 'boom: metric missing' >&2; exit 3;;
*) echo "unexpected stage $*" >&2; exit 2;;
esac
"#
            .to_owned()
        } else {
            format!(
                r#"#!/bin/bash
case "$1" in
check) echo 'check ok'; exit 0;;
run)
  mkdir -p {output_dir}/demo/reports
  printf '<html>fresh report</html>\n' > {output_dir}/demo/reports/evaluation_report_standalone.html
  echo 'run ok'
  exit 0;;
*) echo "unexpected stage $*" >&2; exit 2;;
esac
"#
            )
        };
        write_executable(&root.join("env/bin/openbench"), &stub);
        Self { base, root }
    }

    fn base_text(&self) -> String {
        self.base.to_string_lossy().into_owned()
    }

    fn root_text(&self) -> String {
        self.root.to_string_lossy().into_owned()
    }

    fn project(&self) -> PathBuf {
        self.root.join("projects/versions_vs_flux")
    }

    fn claim(&self, id: &str, intent: &str) -> String {
        let (ok, out, err) = bash_script(&acquire_script(
            &self.project().to_string_lossy(),
            id,
            intent,
        ));
        assert!(ok, "{err}");
        out.trim().to_owned()
    }

    /// 像真实作业那样执行作业体：`job_script` 外壳（`cd <jobs>/<id>` + `set -e` 子 shell + 写
    /// `exit_code`），stdout/stderr 写进 `<jobs>/<id>/log`（收尾脚本从那里取失败输出）。
    fn run_job(&self, id: &str, body: &str) -> (String, Option<String>) {
        use std::io::Write;
        use std::process::Command;
        let job = self.base.join("jobs").join(id);
        std::fs::create_dir_all(&job).unwrap();
        let script = job::job_script(&self.base_text(), id, body, &Spec::bare()).unwrap();
        let out = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(job.join("log"))
            .unwrap();
        let mut child = Command::new("bash")
            .arg("-s")
            .stdin(Stdio::piped())
            .stdout(out.try_clone().unwrap())
            .stderr(out)
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(script.as_bytes())
            .unwrap();
        child.wait().unwrap();
        let log = std::fs::read_to_string(job.join("log")).unwrap_or_default();
        let code = std::fs::read_to_string(job.join("exit_code"))
            .ok()
            .map(|text| text.trim().to_owned());
        (log, code)
    }
}

#[cfg(unix)]
impl Drop for RunSandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

#[cfg(unix)]
#[test]
fn a_successful_remote_run_freezes_its_snapshot_and_moves_the_success_pointer() {
    if system_python3().is_none() {
        eprintln!("skipping: no python3 available for the run-job sandbox");
        return;
    }
    let sandbox = RunSandbox::new("success", false);
    let id = "bench-run-1-1";
    let created_unix = 1_700_000_000u64;
    let config = "general:\n  name: \"versions_vs_flux\"\n  note: it's $HOME `x`\n";
    let config_sha = sha256_hex(config.as_bytes());
    let request = frozen_request(config);
    let body = run_body(
        &sandbox.base_text(),
        &sandbox.root_text(),
        &request,
        id,
        created_unix,
    )
    .unwrap();
    assert_eq!(
        sandbox.claim(id, &submit_intent(id, created_unix, &config_sha)),
        "acquired"
    );
    let (log, code) = sandbox.run_job(id, &body);
    assert_eq!(code.as_deref(), Some("0"), "{log}");

    let project = sandbox.project();
    let run = project.join("runs").join(id);
    // 冻结的配置逐字节等于提交的文本（printf '%s'，不补换行、不展开变量）。
    assert_eq!(
        std::fs::read(run.join("openbench.yaml")).unwrap(),
        config.as_bytes()
    );
    assert_eq!(read_json(&run.join("sites.json")), json!([]));
    let inputs = read_json(&run.join("inputs.json"));
    assert_eq!(inputs["config_sha"], config_sha);
    let meta = read_json(&run.join("meta.json"));
    assert_eq!(meta["run_id"], id);
    assert_eq!(meta["state"], "succeeded");
    assert_eq!(meta["config_sha"], config_sha);
    assert_eq!(meta["created_at"], "2023-11-14T22:13:20Z");
    assert_eq!(
        meta["report"],
        "output/demo/reports/evaluation_report_standalone.html"
    );
    // 只有成功才写指针，且报告路径相对这次运行的目录。
    let latest = read_json(&project.join("latest-success.json"));
    assert_eq!(latest["run_id"], id);
    assert_eq!(latest["config_sha"], config_sha);
    assert_eq!(
        latest["report"],
        "output/demo/reports/evaluation_report_standalone.html"
    );
    assert!(latest["finished_at"].as_str().unwrap().ends_with('Z'));
    assert!(latest["finished_unix"].as_u64().unwrap() > 0);
    // 占位与锁都释放（锁内核对所有权后删）。
    assert!(!project.join(".active-job").exists());
    assert!(!project.join(".active-job.lock").exists());
}

/// R03：自定义 output_dir 的一次运行，meta 与指针里记的都是实际落盘的报告路径，
/// 收尾脚本按它找报告，而不是死看 `output/`。
#[cfg(unix)]
#[test]
fn a_custom_output_dir_run_records_and_finds_its_own_report() {
    if system_python3().is_none() {
        eprintln!("skipping: no python3 available for the run-job sandbox");
        return;
    }
    let sandbox = RunSandbox::new_with_output("custom", false, "results/nested");
    let id = "bench-run-5-5";
    let config = "project:\n  name: versions_vs_flux\n  output_dir: ./results/nested\n";
    let config_sha = sha256_hex(config.as_bytes());
    let request = frozen_request(config);
    let body = run_body(
        &sandbox.base_text(),
        &sandbox.root_text(),
        &request,
        id,
        1_700_000_000,
    )
    .unwrap();
    assert_eq!(
        sandbox.claim(id, &submit_intent(id, 1_700_000_000, &config_sha)),
        "acquired"
    );
    let (log, code) = sandbox.run_job(id, &body);
    assert_eq!(code.as_deref(), Some("0"), "{log}");

    let project = sandbox.project();
    let run = project.join("runs").join(id);
    let meta = read_json(&run.join("meta.json"));
    assert_eq!(meta["output_dir"], "results/nested");
    assert_eq!(
        meta["report"],
        "results/nested/demo/reports/evaluation_report_standalone.html"
    );
    let latest = read_json(&project.join("latest-success.json"));
    assert_eq!(
        latest["report"],
        "results/nested/demo/reports/evaluation_report_standalone.html"
    );
    assert!(run
        .join("results/nested/demo/reports/evaluation_report_standalone.html")
        .is_file());
}

#[cfg(unix)]
#[test]
fn a_failed_remote_run_writes_only_its_own_meta_and_keeps_the_success_pointer() {
    if system_python3().is_none() {
        eprintln!("skipping: no python3 available for the run-job sandbox");
        return;
    }
    let sandbox = RunSandbox::new("failed", true);
    let id = "bench-run-2-2";
    let project = sandbox.project();
    std::fs::create_dir_all(project.join("runs")).unwrap();
    let previous = "{\n  \"run_id\": \"bench-run-0\",\n  \"config_sha\": \"old\",\n  \"report\": \"output/old/reports/evaluation_report_standalone.html\",\n  \"finished_at\": \"2026-10-01T00:00:00Z\",\n  \"finished_unix\": 1\n}\n";
    std::fs::write(project.join("latest-success.json"), previous).unwrap();
    let request = frozen_request("x: 1\n");
    let body = run_body(
        &sandbox.base_text(),
        &sandbox.root_text(),
        &request,
        id,
        1_700_000_000,
    )
    .unwrap();
    assert_eq!(
        sandbox.claim(
            id,
            &submit_intent(id, 1_700_000_000, &sha256_hex(b"x: 1\n"))
        ),
        "acquired"
    );
    let (log, code) = sandbox.run_job(id, &body);
    assert_eq!(code.as_deref(), Some("3"), "{log}");
    let meta = read_json(&project.join("runs").join(id).join("meta.json"));
    assert_eq!(meta["state"], "failed");
    assert_eq!(meta["report"], serde_json::Value::Null);
    let error = meta["error"].as_str().unwrap();
    assert!(error.starts_with("这次运行失败（退出码 3）"), "{error}");
    assert!(error.contains("boom: metric missing"), "{error}");
    // 上次成功的指针一个字节都没动。
    assert_eq!(read(&project.join("latest-success.json")), previous);
    assert!(!project.join(".active-job").exists());
}

#[cfg(unix)]
#[test]
fn a_run_job_that_cannot_start_still_releases_the_placeholder() {
    if system_python3().is_none() {
        eprintln!("skipping: no python3 available for the run-job sandbox");
        return;
    }
    let sandbox = RunSandbox::new("noenv", false);
    std::fs::remove_file(sandbox.root.join("env/bin/openbench")).unwrap();
    let id = "bench-run-3-3";
    let request = frozen_request("x: 1\n");
    let body = run_body(
        &sandbox.base_text(),
        &sandbox.root_text(),
        &request,
        id,
        1_700_000_000,
    )
    .unwrap();
    assert_eq!(
        sandbox.claim(
            id,
            &submit_intent(id, 1_700_000_000, &sha256_hex(b"x: 1\n"))
        ),
        "acquired"
    );
    let (log, code) = sandbox.run_job(id, &body);
    assert_eq!(code.as_deref(), Some("1"), "{log}");
    assert!(log.contains("先装 OpenBench"), "{log}");
    // 环境没装、作业体第一步就失败：EXIT trap 仍要释放占位（旧实现把 trap 放在 env 检查之后）。
    assert!(!sandbox.project().join(".active-job").exists());
    assert!(!sandbox.project().join("runs").join(id).exists());
}

// ---- GUI-07 服务器侧：同名比较的活动作业占位（C16 锁 + C09 意图） ----

/// 用状态脚本真实的输出格式构造 [`job::Report`]：占位判定只看解析结果，不需要真的连服务器。
fn job_report(
    state: &str,
    detail: Option<&str>,
    terminal: bool,
    cancel_requested: bool,
) -> job::Report {
    let mut text = format!("state={state}\n");
    if let Some(detail) = detail {
        text.push_str(&format!("detail={detail}\n"));
    }
    if terminal {
        text.push_str("terminal=confirmed\n");
    }
    if cancel_requested {
        text.push_str("cancel_requested=yes\n");
    }
    job::parse_report(&text).unwrap()
}

#[test]
fn an_unfinished_comparison_job_has_one_stable_refusal_prefix() {
    let message = active_job_error("bench-run-84692-1792749735388362000");
    assert_eq!(
        message,
        "这个比较还有未完成的评估作业（作业号 bench-run-84692-1792749735388362000）；\
         先核对终态或停止它，避免重复写同一个成果目录。"
    );
    // 前端按这个前缀把中文原句映射成英文提示；改字要同步改比较页的映射。
    assert!(
        message.starts_with("这个比较还有未完成的评估作业（作业号 "),
        "{message}"
    );
    assert!(
        lock_timeout_error().starts_with("这个比较的活动作业锁一直被占用"),
        "{}",
        lock_timeout_error()
    );
}

#[test]
fn only_a_confirmed_terminal_job_frees_the_stale_placeholder() {
    // 排队中/运行中：拒绝重复提交，错误里带的就是占位记的作业号。
    for state in ["queued", "running"] {
        let report = job_report(state, None, false, false);
        assert_eq!(
            active_job_refusal("bench-run-1", &report, false).as_deref(),
            Some(active_job_error("bench-run-1").as_str()),
            "{state}"
        );
    }
    // 取消请求在途、调度器尚未确认：仍然是排队/运行，占位不能放，也不能判终态。
    for state in ["queued", "running"] {
        let cancelling = job_report(state, Some("PENDING"), false, true);
        assert!(
            cancelling.cancel_requested && !cancelling.terminal,
            "{state}"
        );
        assert!(active_job_refusal("bench-run-1", &cancelling, true).is_some());
    }
    // 已确认终态：有退出码，或调度器确认 CANCELLED/FAILED/COMPLETED → 陈旧占位可以清掉重新提交。
    for (state, detail) in [
        ("finished 0", None),
        ("finished 2", None),
        ("lost", Some("CANCELLED by 1026|0:0")),
        ("lost", Some("TIMEOUT|0:0")),
        ("lost", Some("COMPLETED|0:0")),
        ("lost", Some("FAILED (LSF EXIT)")),
    ] {
        let report = job_report(state, detail, true, false);
        assert!(report.terminal, "{state} {detail:?}");
        assert_eq!(
            active_job_refusal("bench-run-1", &report, true),
            None,
            "{state} {detail:?}"
        );
        assert_eq!(
            active_job_refusal("bench-run-1", &report, false),
            None,
            "{state} {detail:?}"
        );
    }
    // 调度器回答了"查不到这个作业"（terminal=false）：不是查询失败，但也没确认怎么结束 ——
    // 不能据此放行重复提交（审查报告第 6 条：以前所有 Lost 都当可结束）。
    let lost = job_report("lost", None, false, false);
    assert!(active_job_refusal("bench-run-1", &lost, true).is_some());
    // Unknown 不能一律当终态：作业目录还在，可能只是这次查不到，宁可拒绝。
    let unknown = job_report("unknown", None, false, false);
    assert!(active_job_refusal("bench-run-1", &unknown, true).is_some());
    // 作业目录都没有：占位是提交在 mkdir 与 submit 之间崩掉的残留，可以清掉。
    assert_eq!(active_job_refusal("bench-run-1", &unknown, false), None);
}

#[test]
fn only_a_provably_absent_job_may_be_reclaimed_by_the_grace_rule() {
    // C09 的恢复路径：只有"作业确实不存在"的证据才允许按陈旧回收。
    assert!(abandoned_placeholder(
        &job_report("unknown", None, false, false),
        false
    ));
    assert!(!abandoned_placeholder(
        &job_report("unknown", None, false, false),
        true
    ));
    assert!(abandoned_placeholder(
        &job_report("lost", None, false, false),
        true
    ));
    assert!(!abandoned_placeholder(
        &job_report("lost", Some("COMPLETED|0:0"), true, false),
        true
    ));
    for state in ["queued", "running", "finished 0"] {
        assert!(!abandoned_placeholder(
            &job_report(state, None, false, false),
            false
        ));
    }
}

// ---- R09：已终止、结果未知 ----

#[test]
fn a_job_that_ended_without_an_exit_code_is_terminal_with_an_unknown_result() {
    use JobEnd::{Cancelled, EndedUnknown, Failed, Succeeded};
    let cases: &[(&str, Option<&str>, bool, Option<JobEnd>)] = &[
        ("queued", None, false, None),
        ("running", Some("RUNNING"), false, None),
        ("unknown", None, false, None),
        ("finished 0", None, false, Some(Succeeded)),
        ("finished 3", None, false, Some(Failed)),
        ("lost", Some("CANCELLED by 1026|0:0"), true, Some(Cancelled)),
        ("lost", Some("FAILED (LSF EXIT)"), true, Some(Failed)),
        // COMPLETED 只是退出码还没经 NFS 回来：继续等退出码，不能当成"结果未知"。
        ("lost", Some("COMPLETED|0:0"), true, None),
        // 进程没了、也没有退出码：R09 的"已终止、结果未知"。
        (
            "lost",
            Some("the job process is gone"),
            true,
            Some(EndedUnknown),
        ),
    ];
    for (state, detail, terminal, expected) in cases {
        let report = job_report(state, *detail, *terminal, false);
        assert_eq!(
            JobEnd::from_report(&report),
            *expected,
            "{state} {detail:?} terminal={terminal}"
        );
    }

    // 状态查询把这条路径交给前端：ended=unknown + result_unknown，不再只有 terminal=true。
    let gone = job_report("lost", Some("the job process is gone"), true, false);
    let status = job_status_json("bench-run-1", &gone);
    assert_eq!(status["ended"], "unknown");
    assert_eq!(status["result_unknown"], true);
    assert_eq!(status["terminal"], true);
    // 占位不能继续挡住新提交：进程已经没了，陈旧占位按"重跑"回收。
    assert!(!placeholder_unfinished(&gone, true));
    assert_eq!(active_job_refusal("bench-run-1", &gone, true), None);

    // 正常成功/失败/取消的 JSON 不回归。
    let finished = job_status_json("bench-run-1", &job_report("finished 0", None, false, false));
    assert_eq!(finished["ended"], "succeeded");
    assert_eq!(finished["result_unknown"], false);
    let failed = job_status_json("bench-run-1", &job_report("finished 2", None, false, false));
    assert_eq!(failed["ended"], "failed");
    let cancelled = job_status_json(
        "bench-run-1",
        &job_report("lost", Some("CANCELLED by 1026|0:0"), true, false),
    );
    assert_eq!(cancelled["ended"], "cancelled");
}

#[test]
fn stopping_a_job_whose_process_is_gone_advances_instead_of_failing() {
    // Stop 撞上已经消失的 PID：状态已经是终态，返回可行动的结论而不是 kill 的错误。
    let gone = job_report("lost", Some("the job process is gone"), true, false);
    let outcome = cancel_outcome("bench-run-1", &gone).unwrap();
    assert_eq!(outcome["request_accepted"], false);
    assert_eq!(outcome["ended"], "unknown");
    assert_eq!(outcome["result_unknown"], true);
    assert!(
        outcome["note"].as_str().unwrap().contains("已经不在"),
        "{outcome}"
    );

    // 作业还在排队/运行而取消失败：不能吞掉，返回 None 让调用方报错。
    for state in ["queued", "running"] {
        assert!(
            cancel_outcome("bench-run-1", &job_report(state, None, false, false)).is_none(),
            "{state}"
        );
    }
    // COMPLETED 但没有退出码：调度系统确认结束了，Stop 不该报错；结果仍按等待退出码处理。
    let completed = job_report("lost", Some("COMPLETED|0:0"), true, false);
    let outcome = cancel_outcome("bench-run-1", &completed).unwrap();
    assert_eq!(outcome["request_accepted"], false);
    assert_eq!(outcome["result_unknown"], false);
}

#[test]
fn a_comparison_folder_is_the_only_place_a_placeholder_can_live() {
    assert_eq!(
        project_dir("/srv/colm/openbench", "versions_vs_flux").unwrap(),
        "/srv/colm/openbench/projects/versions_vs_flux"
    );
    for bad in ["", "../escape", "a/b", "a b", "名字"] {
        assert!(project_dir("/srv/colm/openbench", bad).is_err(), "{bad:?}");
    }
}

#[test]
fn the_claim_script_takes_the_slot_under_a_lock_and_records_the_intent() {
    let project = "/srv/colm/openbench/projects/versions_vs_flux";
    let intent = submit_intent("bench-run-1", 1_700_000_000, "abc");
    let script = acquire_script(project, "bench-run-1", &intent);
    // `mkdir` 是原子的：只有一个提交能建成；所有占位改动都在 `_colm_lock_acquire` 之后。
    let lock = script.find("_colm_lock_acquire() {").unwrap();
    let mkdir = script
        .find("if ! mkdir '/srv/colm/openbench/projects/versions_vs_flux/.active-job'")
        .unwrap();
    let lock_use = script.find("if ! _colm_lock_acquire; then").unwrap();
    let write_intent = script
        .find("> '/srv/colm/openbench/projects/versions_vs_flux/.active-job'/intent")
        .unwrap();
    let verify = script.find("[ \"$(cat").unwrap();
    assert!(lock < lock_use && lock_use < mkdir, "{script}");
    assert!(mkdir < write_intent && write_intent < verify, "{script}");
    assert!(script.contains("echo acquired"), "{script}");
    assert!(script.contains("echo lost"), "{script}");
    assert!(script.contains("echo lock_timeout"), "{script}");
    assert!(script.contains("printf 'busy %s\\n'"), "{script}");
    assert!(
        script.contains("mkdir -p '/srv/colm/openbench/projects/versions_vs_flux'"),
        "{script}"
    );
    #[cfg(unix)]
    assert_bash_parses(&script);
}

#[test]
fn the_release_command_only_removes_the_placeholder_it_owns() {
    let script = release_script("/srv/colm dir/openbench/projects/x y", "bench-run-1");
    assert!(script.contains("_colm_lock_acquire() {"), "{script}");
    assert!(
        script.contains("\"$_colm_current\" = 'bench-run-1'"),
        "{script}"
    );
    // 先核对作业号再删：上一轮作业收尾时不能删掉新一轮提交占的位。
    assert!(
        script.contains("rm -rf '/srv/colm dir/openbench/projects/x y/.active-job'"),
        "{script}"
    );
    assert!(
        script.contains("echo released") && script.contains("echo kept"),
        "{script}"
    );
    assert!(script.contains("echo lock_timeout"), "{script}");
    #[cfg(unix)]
    assert_bash_parses(&script);
}

#[test]
fn placeholder_output_is_parsed_strictly() {
    // 远程脚本的 stdout 可能带环境准备的杂音，取最后一个非空行。
    assert_eq!(
        parse_slot("module: loaded\nacquired\n\n"),
        Some(Slot::Acquired)
    );
    assert_eq!(
        parse_slot("busy bench-run-2\n"),
        Some(Slot::Busy("bench-run-2".into()))
    );
    assert_eq!(parse_slot("lock_timeout\n"), Some(Slot::LockTimeout));
    assert_eq!(parse_slot("lost\n"), Some(Slot::Lost));
    // 看不懂的输出不能当成占住：否则会白白提交一份、写同一个成果目录。
    assert_eq!(parse_slot("something else\n"), None);
    assert_eq!(parse_slot(""), None);
    assert_eq!(parse_probe("none\n"), Some(None));
    assert_eq!(parse_probe("empty\n"), Some(None));
    assert_eq!(
        parse_probe("busy bench-run-3\n"),
        Some(Some("bench-run-3".into()))
    );
    assert_eq!(parse_probe("?"), None);
}

#[test]
fn the_probe_used_before_fetching_never_mutates_the_placeholder() {
    let script = active_job_probe_script("/srv/colm/openbench/projects/x");
    assert!(
        script.contains("echo none") && script.contains("printf 'busy %s\\n'"),
        "{script}"
    );
    // 取回是只读动作：创建、删除、改写占位都会直接破坏活动作业保护。
    for forbidden in ["mkdir", "rm -rf", "printf '%s'", "_colm_lock"] {
        assert!(!script.contains(forbidden), "{forbidden} in {script}");
    }
    // 只有 intent、还没有 id 的占位也要能读出预生成作业号（C09 的恢复路径）。
    assert!(script.contains("\"id\":"), "{script}");
    #[cfg(unix)]
    assert_bash_parses(&script);
}

#[test]
fn a_freshly_claimed_slot_is_not_mistaken_for_a_crash_leftover() {
    let script = placeholder_expired_script("/srv/colm openbench/projects/x");
    // 只看占位目录的修改时间：刚占住、作业还没提交出去的窗口里，并发的另一份提交不能清掉它。
    assert!(
        script.contains("find '/srv/colm openbench/projects/x/.active-job' -maxdepth 0 -mmin +1")
    );
    assert!(
        script.contains("echo stale") && script.contains("echo fresh"),
        "{script}"
    );
    #[cfg(unix)]
    assert_bash_parses(&script);
}

/// 在真实 bash 下跑一遍占位脚本（临时目录，不连 SSH）：证明 `mkdir` 原子占位、重复提交读到原
/// 作业号、释放只删自己的作业号、释放后能重新占位、只有 intent 的占位也能释放。
#[cfg(unix)]
#[test]
fn the_active_job_scripts_really_claim_and_release_on_disk() {
    let base = temp_dir("active-job");
    std::fs::create_dir_all(base.join("projects")).unwrap();
    let project = base.join("projects/versions_vs_flux");
    let project_text = project.to_string_lossy().into_owned();
    let intent = |id: &str| submit_intent(id, 1_700_000_000, "cfg");

    let first = bash_script(&acquire_script(
        &project_text,
        "bench-run-1",
        &intent("bench-run-1"),
    ));
    assert!(first.0, "{}", first.2);
    assert_eq!(first.1.trim(), "acquired");
    assert_eq!(read(&project.join(".active-job/id")).trim(), "bench-run-1");
    assert!(project.join(".active-job/intent").is_file());
    assert!(!project.join(".active-job.lock").exists());

    // 另一份提交拿同一个比较：看到占位里原作业号，并且不能覆盖它。
    let second = bash_script(&acquire_script(
        &project_text,
        "bench-run-2",
        &intent("bench-run-2"),
    ));
    assert_eq!(second.1.trim(), "busy bench-run-1");
    assert_eq!(read(&project.join(".active-job/id")).trim(), "bench-run-1");

    // 释放只认自己的作业号：别人的 release 不能删掉占位。
    let foreign = bash_script(&release_script(&project_text, "bench-run-2"));
    assert!(foreign.0, "{}", foreign.2);
    assert_eq!(foreign.1.trim(), "kept");
    assert!(
        project.join(".active-job").exists(),
        "a foreign release must not remove the claim"
    );
    let own = bash_script(&release_script(&project_text, "bench-run-1"));
    assert!(own.0, "{}", own.2);
    assert_eq!(own.1.trim(), "released");
    assert!(
        !project.join(".active-job").exists(),
        "the owner release removes the claim"
    );

    // 释放后可以重新占位；取回前的只读探针分别看到 none 与 busy <作业号>。
    assert_eq!(
        bash_script(&active_job_probe_script(&project_text))
            .1
            .trim(),
        "none"
    );
    assert_eq!(
        bash_script(&acquire_script(
            &project_text,
            "bench-run-3",
            &intent("bench-run-3")
        ))
        .1
        .trim(),
        "acquired"
    );
    assert_eq!(
        bash_script(&active_job_probe_script(&project_text))
            .1
            .trim(),
        "busy bench-run-3"
    );
    // 只有 intent 没有 id（占位在写 id 前崩掉）：probe 仍报作业号，release 按 intent 里的号删。
    std::fs::remove_file(project.join(".active-job/id")).unwrap();
    assert_eq!(
        bash_script(&active_job_probe_script(&project_text))
            .1
            .trim(),
        "busy bench-run-3"
    );
    let recovered = bash_script(&release_script(&project_text, "bench-run-3"));
    assert_eq!(recovered.1.trim(), "released");
    assert!(!project.join(".active-job").exists());

    let _ = std::fs::remove_dir_all(&base);
}

/// 两个进程抢同一把锁：临界区里 `mkdir guard` 只有一个能成功。锁失效时两边都会进临界区，
/// 第二次 mkdir 会打印 BROKEN。
#[cfg(unix)]
#[test]
fn the_active_job_lock_serialises_two_writers() {
    let base = temp_dir("lock-serial");
    let project = base.join("projects/x");
    std::fs::create_dir_all(&project).unwrap();
    let project_text = project.to_string_lossy().into_owned();
    let script = format!(
        r#"{helpers}
hold() {{
  if ! _colm_lock_acquire; then echo timeout; exit 1; fi
  if ! mkdir '{project}/guard' 2>/dev/null; then echo BROKEN; fi
  sleep 0.15
  rmdir '{project}/guard' 2>/dev/null
  _colm_lock_release
}}
hold & hold &
wait
"#,
        helpers = lock_helpers(&project_text, "one"),
        project = project_text,
    );
    let (success, out, err) = bash_script(&script);
    assert!(success, "{err}");
    assert!(!out.contains("BROKEN"), "两个进程同时进了临界区：{out}");
    assert!(!project.join(".active-job.lock").exists(), "锁要释放");
    let _ = std::fs::remove_dir_all(&base);
}

/// 进程被 SIGKILL 留下的锁（owner 时间很旧）要能被下一次提交回收；刚写的锁不能抢。
#[cfg(unix)]
#[test]
fn a_dead_holder_does_not_wedge_the_active_job_lock() {
    let base = temp_dir("lock-stale");
    let project = base.join("projects/x");
    std::fs::create_dir_all(&project).unwrap();
    let project_text = project.to_string_lossy().into_owned();
    let lock = project.join(".active-job.lock");
    std::fs::create_dir_all(&lock).unwrap();
    let now = unix_now();
    std::fs::write(lock.join("owner"), format!("dead-token\n{}\n", now - 120)).unwrap();
    let intent = submit_intent("bench-run-9", now, "cfg");
    let claim = bash_script(&acquire_script(&project_text, "bench-run-9", &intent));
    assert_eq!(claim.1.trim(), "acquired", "{}", claim.2);
    assert_eq!(read(&project.join(".active-job/id")).trim(), "bench-run-9");

    // 刚写下的锁（时间新鲜）不能被另一份提交当陈旧抢走。
    let fresh = bash_script(&acquire_script(&project_text, "bench-run-10", &intent));
    assert_eq!(fresh.1.trim(), "busy bench-run-9", "{}", fresh.2);
    let _ = std::fs::remove_dir_all(&base);
}

/// C16 的复现形状：旧作业的收尾（release id1）与新提交（锁内把占位换成 id2）交错。
/// 旧实现"cat id1 → rm"会把新占位删掉；新实现等锁、锁内核对，看到 id2 就保留。
#[cfg(unix)]
#[test]
fn an_old_exit_cannot_delete_a_new_claim() {
    let base = temp_dir("lock-interleave");
    let project = base.join("projects/x");
    std::fs::create_dir_all(&project).unwrap();
    let project_text = project.to_string_lossy().into_owned();
    let placeholder = project.join(".active-job");
    let intent = |id: &str| submit_intent(id, 1_700_000_000, "cfg");
    let claim = bash_script(&acquire_script(
        &project_text,
        "bench-run-1",
        &intent("bench-run-1"),
    ));
    assert_eq!(claim.1.trim(), "acquired");

    // 另一个提交持有锁 0.3 秒，把占位换成自己的作业号（模拟"陈旧清理后立刻重新占位"）。
    let script = format!(
        r#"{helpers}
if ! _colm_lock_acquire; then exit 1; fi
printf 'locked\n' > '{signal}'
sleep 0.3
rm -rf '{ph}'
mkdir '{ph}'
printf '%s\n' 'bench-run-2' > '{ph}/id'
printf '%s' {intent2} > '{ph}/intent'
_colm_lock_release
"#,
        helpers = lock_helpers(&project_text, "new"),
        signal = project.join("locked.signal").to_string_lossy(),
        ph = placeholder.to_string_lossy(),
        intent2 = quote(&intent("bench-run-2")),
    );
    let mut new_claim = Command::new("bash").arg("-c").arg(&script).spawn().unwrap();
    // 等新提交真的持有锁再让旧收尾去抢，否则测试变成"旧收尾先跑"的另一种合法交错。
    for _ in 0..100 {
        if project.join("locked.signal").exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(project.join("locked.signal").exists(), "新提交没拿到锁");
    let old = bash_script(&release_script(&project_text, "bench-run-1"));
    assert_eq!(old.1.trim(), "kept", "{}", old.2);
    new_claim.wait().unwrap();
    assert_eq!(read(&placeholder.join("id")).trim(), "bench-run-2");
    assert!(placeholder.exists(), "旧作业的收尾不能删掉新占位");
    let _ = std::fs::remove_dir_all(&base);
}

// ---- C09：提交回执丢失的三种分类 ----

fn receipt(
    sequence: Vec<Result<ReceiptProbe, &'static str>>,
) -> impl FnMut() -> Result<ReceiptProbe> {
    let mut sequence = sequence.into_iter();
    move || match sequence.next() {
        Some(Ok(probe)) => Ok(probe),
        Some(Err(_)) => Err(anyhow::anyhow!("scheduler is down")),
        None => Ok(ReceiptProbe::Inconclusive),
    }
}

#[test]
fn a_receipt_lost_after_the_server_accepted_the_job_is_kept() {
    use std::time::Duration;
    use ReceiptProbe::{Absent, Inconclusive, Present};
    let zero = Duration::ZERO;
    // 目的：只要有一次看到作业存在，就按"已接收但回执丢失"处理，不能判成未提交。
    assert_eq!(
        recheck_receipt_with(3, zero, receipt(vec![Ok(Absent), Ok(Present)])),
        ReceiptClass::Accepted
    );
    assert_eq!(
        recheck_receipt_with(
            3,
            zero,
            receipt(vec![Ok(Inconclusive), Ok(Inconclusive), Ok(Present)])
        ),
        ReceiptClass::Accepted
    );
    // 连续三次明确查无记录：判定未接受（调用方释放占位并返回原提交错误）。
    assert_eq!(
        recheck_receipt_with(3, zero, receipt(vec![Ok(Absent), Ok(Absent), Ok(Absent)])),
        ReceiptClass::Absent
    );
    // 查询本身失败：不确定，保留占位（不能把一次 SSH 抖动说成没提交）。
    assert_eq!(
        recheck_receipt_with(3, zero, receipt(vec![Err("down")])),
        ReceiptClass::Uncertain
    );
    // 证据不够（混着 inconclusive）：也不确定。
    assert_eq!(
        recheck_receipt_with(
            3,
            zero,
            receipt(vec![Ok(Absent), Ok(Inconclusive), Ok(Absent)])
        ),
        ReceiptClass::Uncertain
    );
}

#[test]
fn the_receipt_recheck_is_bounded() {
    use std::cell::Cell;
    let calls = Cell::new(0);
    let zero = std::time::Duration::ZERO;
    let verdict = recheck_receipt_with(3, zero, || {
        calls.set(calls.get() + 1);
        Ok(ReceiptProbe::Inconclusive)
    });
    assert_eq!(verdict, ReceiptClass::Uncertain);
    assert_eq!(calls.get(), 3, "有界重查最多 3 次");
}

#[test]
fn reports_map_to_the_three_receipt_verdicts() {
    // 排队/运行/结束/调度器确认的终态：作业确实在服务器上。
    for state in ["queued", "running", "finished 0"] {
        assert_eq!(
            receipt_probe(&job_report(state, None, false, false), false),
            ReceiptProbe::Present,
            "{state}"
        );
    }
    assert_eq!(
        receipt_probe(
            &job_report("lost", Some("CANCELLED by 1026|0:0"), true, false),
            false
        ),
        ReceiptProbe::Present
    );
    // 调度器确认"查无记录"：作业不存在。
    assert_eq!(
        receipt_probe(&job_report("lost", None, false, false), true),
        ReceiptProbe::Absent
    );
    // 作业目录都没有：提交脚本没跑到创建身份那一步。
    assert_eq!(
        receipt_probe(&job_report("unknown", None, false, false), false),
        ReceiptProbe::Absent
    );
    // 目录在、状态还查不到：不能当未提交，也不能当已接收。
    assert_eq!(
        receipt_probe(&job_report("unknown", None, false, false), true),
        ReceiptProbe::Inconclusive
    );
}

// ---- remote-bench-fetch：run 目标选择与取回清单（C07） ----

#[test]
fn only_relative_entries_are_passed_to_tar() {
    assert_eq!(
        clean_entries("output/demo/reports\n\n../escape\n/abs/openbench.yaml\n"),
        vec!["output/demo/reports".to_owned()]
    );
    assert!(require_report(&["output/demo/reports".to_owned()]).is_ok());
    let error = require_report(&["openbench.yaml".to_owned(), "meta.json".to_owned()])
        .unwrap_err()
        .to_string();
    assert!(
        error.starts_with("服务器上还没有可显示的评估报告"),
        "{error}"
    );
}

#[test]
fn the_success_pointer_and_run_meta_are_parsed_or_refused_with_one_prefix() {
    let latest = parse_latest_success(
        "{\n \"run_id\": \"bench-run-1\", \"config_sha\": \"abc\", \"report\": null\n}",
    )
    .unwrap();
    assert_eq!(latest.run_id, "bench-run-1");
    assert_eq!(latest.config_sha, "abc");
    let error = parse_latest_success("not json").unwrap_err().to_string();
    assert!(error.starts_with("服务器上的运行记录读不懂："), "{error}");

    assert!(succeeded_state(
        "bench-run-1",
        &parse_run_meta("{\"state\":\"succeeded\"}").unwrap()
    )
    .is_ok());
    let failed = succeeded_state(
        "bench-run-1",
        &parse_run_meta("{\"state\":\"failed\"}").unwrap(),
    )
    .unwrap_err()
    .to_string();
    assert!(
        failed.starts_with("这次运行没有成功，没有可显示的评估报告"),
        "{failed}"
    );
    // 旧运行记录没有 output_dir/report：按默认输出根取，报告路径交给通配清单（不拒绝）。
    let legacy_run = parse_run_meta("{\"state\":\"succeeded\"}").unwrap();
    assert_eq!(legacy_run.output_dir, "output");
    assert!(legacy_run.report.is_none());
    // 新运行记录带自定义输出根与报告路径，取回清单按它走。
    let custom = parse_run_meta(
        "{\"state\":\"succeeded\",\"output_dir\":\"results\",\
         \"report\":\"results/demo/reports/evaluation_report.html\"}",
    )
    .unwrap();
    assert_eq!(custom.output_dir, "results");
    assert_eq!(
        custom.report.as_deref(),
        Some("results/demo/reports/evaluation_report.html")
    );
    // 空状态（记录被人改坏）也要把话说清楚，不能伪装成成功。
    let unknown = succeeded_state("bench-run-1", &parse_run_meta("{}").unwrap())
        .unwrap_err()
        .to_string();
    assert!(unknown.contains("状态 未知"), "{unknown}");
}

/// run 目录的取回清单：冻结文件 + 这次运行自己的 reports/metrics/scores（不碰别的运行）。
#[cfg(unix)]
#[test]
fn the_fetch_listing_only_looks_inside_the_selected_run() {
    let base = temp_dir("fetch-list");
    let project = base.join("projects/p");
    let run = project.join("runs/bench-run-1");
    std::fs::create_dir_all(run.join("output/demo/reports")).unwrap();
    std::fs::create_dir_all(run.join("output/demo/metrics")).unwrap();
    for name in ["openbench.yaml", "meta.json", "sites.json", "inputs.json"] {
        std::fs::write(run.join(name), "{}").unwrap();
    }
    let (success, out, err) =
        bash_script(&run_listing_script(&run.to_string_lossy(), "output", None));
    assert!(success, "{err}");
    let entries = clean_entries(&out);
    for expected in [
        "openbench.yaml",
        "meta.json",
        "sites.json",
        "inputs.json",
        "output/demo/reports",
        "output/demo/metrics",
    ] {
        assert!(entries.iter().any(|entry| entry == expected), "{entries:?}");
    }
    assert!(require_report(&entries).is_ok());

    // 运行目录不存在：明确给哨兵，不能默默按别的路径取。
    let (_, missing, _) = bash_script(&run_listing_script(
        "/nope/runs/bench-run-1",
        "output",
        None,
    ));
    assert!(missing.contains(NO_SUCH_REMOTE_PATH), "{missing}");

    // 旧式项目：项目根 output/ 的老布局照旧能取。
    let legacy = base.join("projects/old");
    std::fs::create_dir_all(legacy.join("output/demo/reports")).unwrap();
    std::fs::write(legacy.join("openbench.yaml"), "x").unwrap();
    let (success, out, err) = bash_script(&legacy_listing_script(&legacy.to_string_lossy()));
    assert!(success, "{err}");
    let entries = clean_entries(&out);
    assert!(
        entries.iter().any(|entry| entry == "output/demo/reports"),
        "{entries:?}"
    );
    assert!(
        entries.iter().any(|entry| entry == "openbench.yaml"),
        "{entries:?}"
    );
    assert!(require_report(&entries).is_ok());

    // 有 runs/ 但这次运行没有报告：拒绝装一份只有指标的镜像。
    std::fs::remove_dir_all(run.join("output/demo/reports")).unwrap();
    let (_, out, _) = bash_script(&run_listing_script(&run.to_string_lossy(), "output", None));
    let error = require_report(&clean_entries(&out))
        .unwrap_err()
        .to_string();
    assert!(
        error.starts_with("服务器上还没有可显示的评估报告"),
        "{error}"
    );
    let _ = std::fs::remove_dir_all(&base);
}

/// R03：运行记录里写了自定义输出目录/报告时，取回清单按它取，不再猜 `output/*/reports`。
#[cfg(unix)]
#[test]
fn the_fetch_listing_follows_the_output_dir_recorded_in_the_run() {
    let base = temp_dir("fetch-list-custom");
    let run = base.join("projects/p/runs/bench-run-2");
    let report_rel = "results/demo/reports/evaluation_report_standalone.html";
    std::fs::create_dir_all(run.join("results/demo/reports")).unwrap();
    std::fs::write(run.join(report_rel), "<p>ok</p>").unwrap();
    std::fs::create_dir_all(run.join("results/demo/scores")).unwrap();
    let (success, out, err) = bash_script(&run_listing_script(
        &run.to_string_lossy(),
        "results",
        Some(report_rel),
    ));
    assert!(success, "{err}");
    let entries = clean_entries(&out);
    assert!(
        entries.iter().any(|entry| entry == report_rel),
        "{entries:?}"
    );
    assert!(
        entries.iter().any(|entry| entry == "results/demo/reports"),
        "{entries:?}"
    );
    assert!(
        entries.iter().any(|entry| entry == "results/demo/scores"),
        "{entries:?}"
    );
    require_recorded_report(&entries, Some(report_rel)).unwrap();

    // 记录里的报告被删了：只列到父目录也要明确拒绝，不能装一份没有报告的镜像。
    std::fs::remove_file(run.join(report_rel)).unwrap();
    let (success, out, err) = bash_script(&run_listing_script(
        &run.to_string_lossy(),
        "results",
        Some(report_rel),
    ));
    assert!(success, "{err}");
    let entries = clean_entries(&out);
    let error = require_recorded_report(&entries, Some(report_rel)).unwrap_err();
    assert!(error.to_string().contains("报告"), "{error}");
    let _ = std::fs::remove_dir_all(&base);
}

// ---- 状态页的报告发现：新式项目只认 latest-success（C07） ----

#[cfg(unix)]
#[test]
fn the_status_script_finds_reports_through_the_success_pointer_only() {
    let base = temp_dir("status");
    let projects = base.join("projects");
    let ok = projects.join("new_ok");
    std::fs::create_dir_all(ok.join("runs/bench-run-ok/output/demo/reports")).unwrap();
    std::fs::write(
        ok.join("runs/bench-run-ok/output/demo/reports/evaluation_report_standalone.html"),
        "<p>ok</p>",
    )
    .unwrap();
    let pointer = json!({
        "run_id": "bench-run-ok",
        "config_sha": "abc123",
        "report": "output/demo/reports/evaluation_report_standalone.html",
        "finished_at": "2026-10-28T00:00:00Z",
        "finished_unix": 1,
    });
    std::fs::write(
        ok.join("latest-success.json"),
        format!("{}\n", serde_json::to_string_pretty(&pointer).unwrap()),
    )
    .unwrap();
    // 失败/未成功过的新式项目：即使 output/ 里有旧报告也不能显示，免得把上次残留当成果。
    let failed = projects.join("new_failed");
    std::fs::create_dir_all(failed.join("runs/bench-run-fail/output/demo/reports")).unwrap();
    std::fs::write(
        failed.join("runs/bench-run-fail/output/demo/reports/evaluation_report.html"),
        "<p>old</p>",
    )
    .unwrap();
    // 旧式项目：没有 runs/，按项目根 output/ 发现。
    let legacy = projects.join("legacy");
    std::fs::create_dir_all(legacy.join("output/demo/reports")).unwrap();
    std::fs::write(
        legacy.join("output/demo/reports/evaluation_report.html"),
        "<p>legacy</p>",
    )
    .unwrap();

    let (success, text, err) = bash_script(&status_script(&base.to_string_lossy()));
    assert!(success, "{err}");
    let value = parse_status(&base.to_string_lossy(), &text);
    let find = |name: &str| {
        value["projects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|project| project["name"] == name)
            .unwrap()
            .clone()
    };
    let ok = find("new_ok");
    assert!(
        ok["report"]
            .as_str()
            .unwrap()
            .contains("/runs/bench-run-ok/output/demo/reports/evaluation_report_standalone.html"),
        "报告来自 latest-success 指向的 run output：{ok}"
    );
    assert_eq!(ok["latest_success"]["run_id"], "bench-run-ok");
    assert_eq!(ok["latest_success"]["config_sha"], "abc123");
    assert_eq!(ok["latest_success"]["finished_at"], "2026-10-28T00:00:00Z");
    let failed = find("new_failed");
    assert!(failed["report"].is_null(), "{failed}");
    assert!(failed["latest_success"].is_null(), "{failed}");
    let legacy = find("legacy");
    assert!(legacy["report"]
        .as_str()
        .unwrap()
        .contains("/legacy/output/demo/reports/evaluation_report.html"));
    assert!(legacy["latest_success"].is_null(), "{legacy}");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn server_status_lines_become_the_same_shape_as_the_local_status() {
    let text = "uname=Linux x86_64\ncurl=/usr/bin/curl\nsha=/usr/bin/sha256sum\nuv=0.12.5\npython=Python 3.12.14\nopenbench=3.0.8\nproject=a|/srv/ob/projects/a/runs/x/output/a/reports/evaluation_report_standalone.html|bench-run-x|abc123|2026-10-28T00:00:00Z\nproject=b|\nproject=c|/srv/ob/projects/c/output/c/reports/evaluation_report.html\n";
    let value = parse_status("/srv/ob", text);
    assert_eq!(value["platform"], "linux-x86_64");
    assert_eq!(value["uv_version"], "0.12.5");
    assert_eq!(value["openbench"], "3.0.8");
    assert_eq!(value["openbench_pinned"], OPENBENCH_VERSION);
    assert_eq!(value["projects"][0]["name"], "a");
    assert_eq!(
        value["projects"][0]["latest_success"]["run_id"],
        "bench-run-x"
    );
    assert_eq!(
        value["projects"][0]["latest_success"]["config_sha"],
        "abc123"
    );
    assert!(value["projects"][1]["report"].is_null());
    assert!(value["projects"][1]["latest_success"].is_null());
    assert_eq!(
        value["projects"][2]["report"],
        "/srv/ob/projects/c/output/c/reports/evaluation_report.html"
    );
    // 什么都没装：版本都是空的，平台照报。
    let empty = parse_status("/srv/ob", "uname=Linux aarch64\ncurl=\nsha=\n");
    assert!(empty["uv_version"].is_null() && empty["downloader"].is_null());
    assert_eq!(empty["platform"], "linux-aarch64");
}

// ---- 其余命令的原样回归 ----

#[test]
fn the_uv_install_verifies_the_pinned_checksum_before_unpacking() {
    let (archive, sha) = uv_archive("linux-x86_64").unwrap();
    let body = install_uv_body("/srv/colm/openbench", archive, sha);
    let verify = body.find(sha).unwrap();
    let unpack = body.find("tar -xzf").unwrap();
    assert!(verify < unpack, "{body}");
    assert!(body.contains(&format!("releases/download/{UV_VERSION}/{archive}")));
    assert!(body.contains("sha256sum -c -") && body.contains("shasum -a 256 -c -"));
    #[cfg(unix)]
    assert_bash_parses(&body);
}

#[test]
fn registration_passes_every_field_quoted() {
    let request = Register {
        name: "OpenBench_LH_Daily".into(),
        root_dir: "/data/ref/LH dir".into(),
        data_type: "stn".into(),
        tim_res: "Day".into(),
        years: (1996, 2022),
        category: None,
        fulllist: Some("/data/ref/list.csv".into()),
        variables: vec!["Latent_Heat:LE:W m-2".into()],
    };
    let script = register_script("/srv/colm/openbench", &request).unwrap();
    assert!(script.contains("--root-dir '/data/ref/LH dir'"), "{script}");
    assert!(script.contains("-v 'Latent_Heat:LE:W m-2'"), "{script}");
    assert!(script.contains("--years 1996 2022"), "{script}");
    let mut bad = request;
    bad.data_type = "grid; rm -rf /".into();
    assert!(register_script("/srv/colm/openbench", &bad).is_err());
}

/// OpenBench 3.0.8 的 tim_res 白名单（config/loader.py 的 VALID_TIM_RES_VALUES + Nmonth）：
/// 旧白名单只有 Hour/Day/Month/Year，登记 6Hour/8Day/climatology 参考数据会被本地挡掉。
#[test]
fn registration_accepts_every_time_resolution_openbench_3_0_8_knows() {
    let base = Register {
        name: "OpenBench_LH".into(),
        root_dir: "/data/ref".into(),
        data_type: "stn".into(),
        tim_res: "Day".into(),
        years: (1996, 2022),
        category: None,
        fulllist: None,
        variables: vec!["Latent_Heat:LE:W m-2".into()],
    };
    for tim_res in [
        "Month",
        "Day",
        "Hour",
        "Year",
        "3Hour",
        "6Hour",
        "8Day",
        "climatology-month",
        "climatology-year",
        "1month",
        "3month",
        "12month",
    ] {
        let request = Register {
            tim_res: tim_res.into(),
            ..base.clone()
        };
        let script = register_script("/srv/colm/openbench", &request)
            .unwrap_or_else(|error| panic!("{tim_res} 应该被接受：{error}"));
        assert!(
            script.contains(&format!("--tim-res {tim_res}")),
            "{tim_res}: {script}"
        );
    }
    // 大小写：固定值不区分（OpenBench 会归一成 canonical 名），Nmonth 则要求小写 month。
    for accepted in ["3hour", "Climatology-Year", "month", "3month"] {
        let request = Register {
            tim_res: accepted.into(),
            ..base.clone()
        };
        assert!(
            register_script("/srv/colm/openbench", &request).is_ok(),
            "{accepted}"
        );
    }
    for rejected in ["Week", "5Day", "0month", "3MONTH", "12", ""] {
        let request = Register {
            tim_res: rejected.into(),
            ..base.clone()
        };
        assert!(
            register_script("/srv/colm/openbench", &request).is_err(),
            "{rejected:?} 不该被接受"
        );
    }
}

#[test]
fn uninstall_removes_only_the_named_folders_under_the_environment_root() {
    let script = uninstall_script("/srv/colm/openbench", &["env", "tools"]);
    assert!(
        script.contains("rm -rf '/srv/colm/openbench/env' '/srv/colm/openbench/tools'"),
        "{script}"
    );
    // 登记表与比较不在其中。
    assert!(
        !script.contains("/home") && !script.contains("/projects"),
        "{script}"
    );
    #[cfg(unix)]
    assert_bash_parses(&script);
}

#[test]
fn the_openbench_install_prefetches_basemaps_for_offline_compute_nodes() {
    let body = python_here("/srv/ob", PREFETCH_SCRIPT, &[], "COLM_BENCH_PREFETCH");
    assert!(body.contains("natural_earth"), "{body}");
    #[cfg(unix)]
    assert_bash_parses(&body);
}

#[test]
fn a_model_variable_mapping_refuses_colons_and_trims_fields() {
    assert_eq!(
        model_variable_spec(" Latent_Heat", "f_lfevpa ", "W m-2").unwrap(),
        "Latent_Heat:f_lfevpa:W m-2"
    );
    for (variable, varname, unit) in [
        ("Latent_Heat", "f:lfevpa", "W m-2"),
        ("Latent_Heat", "f_lfevpa", "W m-2:x"),
        ("", "f_lfevpa", "W m-2"),
        ("Latent_Heat", "f_lfevpa", "W\nm-2"),
    ] {
        assert!(
            model_variable_spec(variable, varname, unit).is_err(),
            "{variable:?} {varname:?} {unit:?}"
        );
    }
}

// ---- 取消/查询失败在真实状态脚本下的整条链：用替身调度器跑 `job::status_script` ----

/// 状态脚本在真实 bash 下的沙箱：替身 squeue/sacct/scancel 读到 `$STUBS/*` 里的文件，和
/// `job_tests.rs` 的写法一致；这里只跑状态/取消脚本，不连 SSH。
#[cfg(unix)]
struct StatusSandbox {
    root: std::path::PathBuf,
    bin: std::path::PathBuf,
}

#[cfg(unix)]
impl StatusSandbox {
    fn new(tag: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("colm-bench-status-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let sandbox = Self { root, bin };
        sandbox.stub("squeue", "cat \"$STUBS/slurm_state\" 2>/dev/null; true");
        sandbox.stub("sacct", "cat \"$STUBS/sacct_out\" 2>/dev/null; true");
        sandbox.stub("scancel", "echo \"$@\" > \"$STUBS/cancelled\"");
        sandbox
    }

    fn stub(&self, name: &str, body: &str) {
        write_executable(&self.bin.join(name), &format!("#!/bin/bash\n{body}\n"));
    }

    fn set(&self, file: &str, value: &str) {
        std::fs::write(self.bin.join(file), value).unwrap();
    }

    fn unset(&self, file: &str) {
        let _ = std::fs::remove_file(self.bin.join(file));
    }

    fn work(&self) -> String {
        self.root.join("work").to_string_lossy().into_owned()
    }

    fn prepare_job(&self, id: &str) -> std::path::PathBuf {
        let dir = self.root.join("work/jobs").join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("scheduler"), "slurm").unwrap();
        std::fs::write(dir.join("sched_id"), "4242").unwrap();
        dir
    }

    /// 和 `Ssh::run` 一样把脚本经 stdin 交给 `bash -s`。
    fn run(&self, script: &str) -> (String, String, bool) {
        use std::io::Write;
        let mut child = Command::new("bash")
            .arg("-s")
            .env("STUBS", &self.bin)
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", self.bin.display()),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(script.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.success(),
        )
    }

    fn report(&self, id: &str) -> job::Report {
        let (out, err, ok) = self.run(&job::status_script(&self.work(), id, 5));
        assert!(ok, "{err}");
        job::parse_report(&out).unwrap_or_else(|error| panic!("{error}\n{out}"))
    }
}

#[cfg(unix)]
impl Drop for StatusSandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// 审查报告第 6 条：取消请求被调度系统收下时作业可能还 queued；此时不能算结束，占位也不能放。
/// 只有 sacct 确认 CANCELLED 后才结束并释放占位（以前请求一收下前端就写 cancelled、占位被清）。
#[cfg(unix)]
#[test]
fn a_cancelled_job_frees_the_placeholder_only_after_the_scheduler_confirms_it() {
    let sb = StatusSandbox::new("cancel");
    let id = "bench-run-1";
    sb.prepare_job(id);
    sb.set("slurm_state", "PENDING\n");
    let project = sb.root.join("work/openbench/projects/versions_vs_flux");
    std::fs::create_dir_all(&project).unwrap();
    let project_text = project.to_string_lossy().into_owned();
    let intent = submit_intent(id, unix_now(), "cfg");
    let claim = sb.run(&acquire_script(&project_text, id, &intent));
    assert!(claim.2, "{}", claim.1);
    assert!(project.join(".active-job").exists());

    let queued = sb.report(id);
    assert_eq!(queued.status.state, job::State::Queued);
    assert!(!queued.cancel_requested && !queued.terminal);
    assert!(placeholder_unfinished(&queued, true));

    // 服务器收下取消：作业还在排队，状态必须是"正在取消"而不是已取消。
    let requested = sb.run(&job::cancel_script(&sb.work(), id));
    assert!(requested.2, "{}", requested.1);
    assert!(
        requested.0.contains("cancellation_requested"),
        "{}",
        requested.0
    );
    assert_eq!(
        std::fs::read_to_string(sb.bin.join("cancelled"))
            .unwrap()
            .trim(),
        "4242"
    );
    let in_flight = sb.report(id);
    assert_eq!(in_flight.status.state, job::State::Queued);
    assert!(in_flight.cancel_requested, "前端要显示正在取消");
    assert!(!in_flight.terminal);
    assert_eq!(job::ended(&in_flight.status), None);
    assert!(placeholder_unfinished(&in_flight, true), "未确认前占位不放");

    // 调度器仍说在运行：还是不能结束。
    sb.set("slurm_state", "RUNNING\n");
    let running = sb.report(id);
    assert_eq!(running.status.state, job::State::Running);
    assert!(running.cancel_requested && !running.terminal);

    // sacct 确认 CANCELLED：这才算结束，占位才可以释放。
    sb.unset("slurm_state");
    sb.set("sacct_out", "CANCELLED by 1026|0:0\n");
    let confirmed = sb.report(id);
    assert_eq!(confirmed.status.state, job::State::Lost);
    assert!(confirmed.terminal);
    assert_eq!(job::ended(&confirmed.status), Some("cancelled"));
    assert!(!placeholder_unfinished(&confirmed, true));
    let released = sb.run(&release_script(&project_text, id));
    assert!(released.2, "{}", released.1);
    assert!(
        !project.join(".active-job").exists(),
        "确认终态后陈旧占位才清掉"
    );
}

/// 查询失败时状态脚本输出 `query=failed`，`job::report` 直接报错 —— 提交/取回流程在读取占位作业
/// 状态这一步就会带着可诊断错误中止，不会走到"释放占位"的分支。
#[cfg(unix)]
#[test]
fn a_failed_status_query_never_releases_the_placeholder() {
    let sb = StatusSandbox::new("query-down");
    let id = "bench-run-2";
    sb.prepare_job(id);
    let project = sb.root.join("work/openbench/projects/versions_vs_flux");
    std::fs::create_dir_all(&project).unwrap();
    let project_text = project.to_string_lossy().into_owned();
    let claim = sb.run(&acquire_script(
        &project_text,
        id,
        &submit_intent(id, unix_now(), "cfg"),
    ));
    assert!(claim.2, "{}", claim.1);

    sb.stub(
        "squeue",
        "echo 'slurm_load_jobs error: Unable to contact slurm controller' >&2; exit 1",
    );
    sb.stub("sacct", "echo 'slurmdbd is down' >&2; exit 1");
    let (out, err, ok) = sb.run(&job::status_script(&sb.work(), id, 5));
    assert!(ok, "{err}");
    let error = job::parse_report(&out).unwrap_err().to_string();
    assert!(error.starts_with("查询作业状态失败："), "{error}");
    assert!(
        error.contains("Unable to contact slurm controller"),
        "{error}"
    );
    assert!(project.join(".active-job").exists(), "查询失败不释放占位");
}
