//! Conservative deletion of exclusive server-native frozen runs.
use super::Opts;
use anyhow::{ensure, Result};
use colm_remote::{
    engine_inventory::LifecycleLock,
    job,
    ssh::{quote, Ssh},
};

const CLEANUP: &str = r#"
import hashlib, json, os, re, shutil, stat, sys, tempfile
root, job, operation, target, action, case_digest, mode = sys.argv[1:]
whole_case = bool(case_digest)
project = mode == 'project'
def plain(path):
    if not path.startswith('/') or path == '/' or any(p in ('', '.', '..') for p in path.split('/')[1:]):
        raise ValueError('unsafe absolute directory')
    current = '/'
    for part in path.split('/')[1:]:
        current = os.path.join(current, part)
        if stat.S_ISLNK(os.lstat(current).st_mode):
            raise ValueError('symbolic links are not allowed: ' + current)
    return path
def read_json(path):
    plain(path)
    with open(path) as source:
        return json.load(source)
def finished(directory, name):
    exit_path = directory + '/exit_code'
    if os.path.lexists(exit_path):
        plain(exit_path)
        with open(exit_path) as source:
            return re.fullmatch(r'-?\d+\s*', source.read(64)) is not None
    # A packed station cancelled before any node job took it never runs, so it never writes an
    # exit code; nothing can start it any more (colm_remote::pack).
    if os.path.isfile(directory + '/scheduler') and os.path.isfile(directory + '/pool'):
        with open(plain(directory + '/scheduler')) as source:
            scheduler = source.read().strip()
        with open(plain(directory + '/pool')) as source:
            pool = plain(source.read().strip())
        claim = pool + '/claimed/' + name
        return scheduler == 'slurm-pack' and os.path.isfile(claim + '/cancelled') and not os.path.exists(claim + '/carrier')
    return False
def marker(text, key):
    values = re.findall(r'^# ' + re.escape(key) + r': (.+)$', text, re.M)
    if len(values) != 1:
        raise ValueError('missing or ambiguous job identity: ' + key)
    return values[0]
plain(root)
plain(target)
original = os.lstat(target)
if not stat.S_ISDIR(original.st_mode):
    raise ValueError('run directory is missing')
if not os.path.isdir(root + '/.engine-lifecycle-lock'):
    raise ValueError('exclusive lifecycle lock is required')
if os.path.isdir(root + '/.engine-leases') and os.listdir(root + '/.engine-leases'):
    raise ValueError('submission is in progress or uncertain')
home = os.path.realpath(os.path.expanduser('~'))
reserved = ('engine', 'kernels', 'tools', 'jobs', 'pools', 'offline', 'target', '.engine-leases', '.engine-lifecycle-lock', 's', 'studies')
if (project or whole_case) and (target == home or home.startswith(target + '/') or target == root or target == root + '/cases' or any(target == root + '/' + name or target.startswith(root + '/' + name + '/') for name in reserved)):
    raise ValueError('protected home or server resource directory')
if project:
    # A whole project folder: every entry must be one of its cases, each owning itself. Anything
    # else means the folder holds more than the app made, so nothing is deleted.
    for name in sorted(os.listdir(target)):
        case = plain(target + '/' + name)
        if not stat.S_ISDIR(os.lstat(case).st_mode):
            raise ValueError('the project folder holds something that is not a case: ' + name)
        try:
            owner = read_json(case + '/.colm-desktop-server.json')
        except (OSError, ValueError):
            raise ValueError('the project folder holds something that is not a case: ' + name)
        if owner.get('schema') != 1 or owner.get('remote_directory') != case or not os.path.isfile(plain(case + '/case.nml')):
            raise ValueError('the project folder holds something that is not a case: ' + name)
elif whole_case:
    base = target
    nml = plain(target + '/case.nml')
    with open(nml, 'rb') as source:
        if hashlib.sha256(source.read()).hexdigest() != case_digest:
            raise ValueError('case namelist changed during cleanup')
else:
    if os.path.basename(target) != operation or os.path.basename(os.path.dirname(target)) != 'runs':
        raise ValueError('only exclusive frozen server run directories can be deleted')
    base = os.path.dirname(os.path.dirname(target))
    manifest = read_json(target + '/model-run.json')
    if manifest.get('schema') != 1 or manifest.get('operation_id') != operation or manifest.get('run_case') != target:
        raise ValueError('frozen run ownership does not match')
if not project:
    owner = read_json(base + '/.colm-desktop-server.json')
    if owner.get('schema') != 1 or owner.get('remote_directory') != base:
        raise ValueError('server project ownership does not match')
# The run must never contain the lifecycle/job registry that protects it.
if root == target or root.startswith(target + '/'):
    raise ValueError('run directory contains the server resource root')
jobs = root + '/jobs'
if os.path.lexists(jobs):
    plain(jobs)
seen = project or (whole_case and not job)
for name in os.listdir(jobs) if os.path.exists(jobs) else []:
    directory = plain(jobs + '/' + name)
    if not finished(directory, name):
        raise ValueError('job is not confirmed finished: ' + name)
    if name == job and not project:
        plain(directory + '/script.sh')
        with open(directory + '/script.sh') as source:
            text = source.read(2 * 1024 * 1024 + 1)
        expected = [target, target + '/runs/' + operation] if whole_case else [target]
        if len(text) > 2 * 1024 * 1024 or json.loads(marker(text, 'colm-run-case')) not in expected or marker(text, 'colm-operation-id') != operation:
            raise ValueError('job does not own this frozen run')
        seen = True
if not seen:
    raise ValueError('owning job is missing')
for current, dirs, files in os.walk(target, followlinks=False):
    for name in dirs + files:
        plain(os.path.join(current, name))
if action == 'delete':
    quarantine = tempfile.mkdtemp(prefix='.colm-delete-', dir=os.path.dirname(target))
    isolated = os.path.join(quarantine, 'run')
    os.rename(target, isolated)
    moved = os.lstat(isolated)
    if not stat.S_ISDIR(moved.st_mode) or (moved.st_dev, moved.st_ino) != (original.st_dev, original.st_ino):
        # Restoring could replace another concurrent creation; retain the quarantined item instead.
        raise ValueError('run directory changed during cleanup; nothing deleted; recover directory at: ' + isolated)
    shutil.rmtree(isolated)
    os.rmdir(quarantine)
    print(json.dumps({'removed': target}))
else:
    print(json.dumps({'path': target, 'can_delete': True}))
"#;

fn script(
    root: &str,
    id: &str,
    operation: &str,
    path: &str,
    delete: bool,
    case_digest: Option<&str>,
) -> Result<String> {
    if !id.is_empty() || case_digest.is_none() {
        job::check_id(id)?;
    }
    job::check_id(operation)?;
    for value in [root, path] {
        ensure!(
            value.starts_with('/')
                && value != "/"
                && !value.chars().any(char::is_control)
                && value
                    .split('/')
                    .skip(1)
                    .all(|p| !matches!(p, "" | "." | "..")),
            "unsafe cleanup path"
        );
    }
    Ok(invocation(
        root,
        id,
        operation,
        path,
        delete,
        case_digest.unwrap_or(""),
        "run",
    ))
}

fn invocation(
    root: &str,
    id: &str,
    operation: &str,
    path: &str,
    delete: bool,
    case_digest: &str,
    mode: &str,
) -> String {
    format!(
        "python3 - {} {} {} {} {} {} {mode} <<'COLM_HISTORY_CLEANUP'\n{}\nCOLM_HISTORY_CLEANUP\n",
        quote(root),
        quote(id),
        quote(operation),
        quote(path),
        if delete { "delete" } else { "preview" },
        quote(case_digest),
        CLEANUP
    )
}

/// 删除整个服务器项目文件夹：里面只能是这个项目的算例（各自的 `.colm-desktop-server.json`
/// 指向自己），服务器工作目录下的作业都已结束。
fn project_script(root: &str, path: &str, delete: bool) -> Result<String> {
    for value in [root, path] {
        ensure!(
            value.starts_with('/')
                && value != "/"
                && !value.chars().any(char::is_control)
                && value
                    .split('/')
                    .skip(1)
                    .all(|p| !matches!(p, "" | "." | "..")),
            "unsafe cleanup path"
        );
    }
    Ok(invocation(root, "", "project", path, delete, "", "project"))
}

fn validate_case_namelist(text: &str) -> Result<()> {
    let doc = colm_namelist::parse(text)?;
    ensure!(
        matches!(doc.get("DEF_CASE_NAME"), Some(colm_namelist::Value::Str(name)) if !name.trim().is_empty()),
        "case.nml must contain a nonempty DEF_CASE_NAME"
    );
    Ok(())
}

pub(super) fn run(opts: &Opts) -> Result<()> {
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let root = opts.need_str("--root")?;
    if opts.get("--project").as_deref() == Some("1") {
        let path = opts.need_str("--expected-path")?;
        let delete = opts.get("--delete").as_deref() == Some("1");
        let command = project_script(&root, &path, delete)?;
        let mut lock = LifecycleLock::acquire_exclusive(&ssh, &root)?;
        let result = lock.run(|| -> Result<serde_json::Value> {
            Ok(serde_json::from_str(&ssh.run_ok(&command)?)?)
        })?;
        lock.finish()?;
        println!("{result}");
        return Ok(());
    }
    let whole_case = opts.get("--whole-case").as_deref() == Some("1");
    let path = opts.need_str("--expected-path")?;
    let id = if whole_case {
        opts.get("--job").unwrap_or_default()
    } else {
        opts.need_str("--job")?
    };
    let operation = opts.need_str("--operation")?;
    script(
        &root,
        &id,
        &operation,
        &path,
        false,
        whole_case.then_some("validate"),
    )?;
    let mut lock = LifecycleLock::acquire_exclusive(&ssh, &root)?;
    let result = lock.run(|| -> Result<serde_json::Value> {
        let digest = if whole_case {
            let text = ssh.run_ok(&format!("cat -- {}", quote(&format!("{path}/case.nml"))))?;
            validate_case_namelist(&text)?;
            Some(colm_kernel::sha256_hex(text.as_bytes()))
        } else {
            None
        };
        let command = script(
            &root,
            &id,
            &operation,
            &path,
            opts.get("--delete").as_deref() == Some("1"),
            digest.as_deref(),
        )?;
        Ok(serde_json::from_str(&ssh.run_ok(&command)?)?)
    })?;
    lock.finish()?;
    println!("{result}");
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf, process::Command};
    struct Fixture {
        base: PathBuf,
        root: PathBuf,
        target: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let base = std::env::temp_dir().join(format!(
                "colm-cleanup-{}",
                colm_history::Event::start("cleanup-test").operation_id
            ));
            fs::create_dir(&base).unwrap();
            let base = fs::canonicalize(base).unwrap();
            let root = base.join("resource");
            let project = base.join("project");
            let target = project.join("runs/op1");
            fs::create_dir_all(&target).unwrap();
            fs::create_dir_all(root.join("jobs/job1")).unwrap();
            fs::create_dir(root.join(".engine-lifecycle-lock")).unwrap();
            fs::write(
                project.join(".colm-desktop-server.json"),
                serde_json::json!({"schema":1,"remote_directory":project}).to_string(),
            )
            .unwrap();
            fs::write(
                target.join("model-run.json"),
                serde_json::json!({"schema":1,"operation_id":"op1","run_case":target}).to_string(),
            )
            .unwrap();
            fs::write(root.join("jobs/job1/exit_code"), "0\n").unwrap();
            fs::write(
                root.join("jobs/job1/script.sh"),
                format!(
                    "# colm-run-case: {}\n# colm-operation-id: op1\n",
                    serde_json::to_string(&target).unwrap()
                ),
            )
            .unwrap();
            Self { base, root, target }
        }
        fn run(&self, delete: bool) -> std::process::Output {
            Command::new("bash")
                .args([
                    "-c",
                    &script(
                        self.root.to_str().unwrap(),
                        "job1",
                        "op1",
                        self.target.to_str().unwrap(),
                        delete,
                        None,
                    )
                    .unwrap(),
                ])
                .output()
                .unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.base);
        }
    }
    #[test]
    fn remote_history_cleanup_preview_then_delete_owned_run() {
        let f = Fixture::new();
        let result = f.run(false);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(f.target.exists());
        let result = f.run(true);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(!f.target.exists());
        assert!(f.root.join("jobs/job1/script.sh").exists());
        assert!(f
            .target
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(".colm-desktop-server.json")
            .exists());
    }
    #[test]
    fn remote_history_cleanup_preserves_replacement_after_validation_race() {
        let f = Fixture::new();
        let command = script(
            f.root.to_str().unwrap(),
            "job1",
            "op1",
            f.target.to_str().unwrap(),
            true,
            None,
        )
        .unwrap();
        // Inject a competing filesystem mutation immediately before the real rename.
        let command = command.replace(
            "    os.rename(target, isolated)",
            r#"    os.rename(target, target + '.original')
    os.mkdir(target)
    with open(os.path.join(target, 'keep.txt'), 'w') as replacement:
        replacement.write('unconfirmed replacement')
    os.rename(target, isolated)"#,
        );
        let result = Command::new("bash")
            .args(["-c", &command])
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(!String::from_utf8_lossy(&result.stdout).contains("removed"));
        assert!(String::from_utf8_lossy(&result.stderr).contains("recover directory at:"));
        let parent = f.target.parent().unwrap();
        assert!(parent.join("op1.original/model-run.json").exists());
        let quarantine = fs::read_dir(parent)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".colm-delete-")
            })
            .unwrap();
        assert_eq!(
            fs::read_to_string(quarantine.join("run/keep.txt")).unwrap(),
            "unconfirmed replacement"
        );
    }
    fn whole_case(f: &Fixture, job: &str, delete: bool) -> std::process::Output {
        let project = f.target.parent().unwrap().parent().unwrap();
        let text = fs::read_to_string(project.join("case.nml")).unwrap();
        validate_case_namelist(&text).unwrap();
        let command = script(
            f.root.to_str().unwrap(),
            job,
            "op1",
            project.to_str().unwrap(),
            delete,
            Some(&colm_kernel::sha256_hex(text.as_bytes())),
        )
        .unwrap();
        Command::new("bash")
            .args(["-c", &command])
            .output()
            .unwrap()
    }
    #[test]
    fn remote_history_cleanup_whole_case_removes_runs_preserves_external_inputs() {
        let f = Fixture::new();
        let project = f.target.parent().unwrap().parent().unwrap();
        let input = f.base.join("forcing.nc");
        fs::write(&input, "shared data").unwrap();
        fs::write(
            project.join("case.nml"),
            format!(
                "&colm_inparm\n DEF_CASE_NAME = 'demo'\n DEF_dir_forcing = '{}'\n/\n",
                input.display()
            ),
        )
        .unwrap();
        fs::create_dir_all(project.join("runs/op2")).unwrap();
        fs::write(project.join("runs/op2/result.nc"), "result").unwrap();
        let preview = whole_case(&f, "job1", false);
        assert!(
            preview.status.success(),
            "{}",
            String::from_utf8_lossy(&preview.stderr)
        );
        assert!(project.join("runs/op2/result.nc").exists());
        let result = whole_case(&f, "job1", true);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(!project.exists());
        assert_eq!(fs::read_to_string(input).unwrap(), "shared data");
        assert!(f.root.join("jobs/job1/script.sh").exists());
    }
    #[test]
    fn remote_history_cleanup_whole_case_inside_resource_cases_is_allowed() {
        let mut f = Fixture::new();
        let old_project = f.target.parent().unwrap().parent().unwrap().to_path_buf();
        let project = f.root.join("cases/demo");
        fs::create_dir(f.root.join("cases")).unwrap();
        fs::rename(old_project, &project).unwrap();
        f.target = project.join("runs/op1");
        fs::write(
            project.join(".colm-desktop-server.json"),
            serde_json::json!({"schema":1,"remote_directory":project}).to_string(),
        )
        .unwrap();
        fs::write(
            project.join("case.nml"),
            "&colm_inparm\nDEF_CASE_NAME = 'demo'\n/\n",
        )
        .unwrap();
        let result = whole_case(&f, "", true);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(!project.exists());
        assert!(f.root.join("jobs/job1/script.sh").exists());
        assert!(f.root.join("cases").exists());
    }
    #[test]
    fn remote_history_cleanup_whole_case_without_job_requires_marker_and_no_active_jobs() {
        let f = Fixture::new();
        let project = f.target.parent().unwrap().parent().unwrap();
        fs::write(
            project.join("case.nml"),
            "&colm_inparm\nDEF_CASE_NAME = 'demo'\n/\n",
        )
        .unwrap();
        assert!(whole_case(&f, "", false).status.success());
        fs::create_dir(f.root.join("jobs/pending")).unwrap();
        assert!(!whole_case(&f, "", true).status.success());
        assert!(project.exists());
        fs::remove_dir(f.root.join("jobs/pending")).unwrap();
        fs::write(project.join(".colm-desktop-server.json"), "{}").unwrap();
        assert!(!whole_case(&f, "", true).status.success());
        assert!(project.exists());
        assert!(validate_case_namelist("not a namelist").is_err());
        assert!(validate_case_namelist("&colm_inparm\n DEF_CASE_NAME = ''\n/\n").is_err());
    }
    fn project(f: &Fixture, delete: bool) -> std::process::Output {
        let command = project_script(
            f.root.to_str().unwrap(),
            f.base.join("uuu").to_str().unwrap(),
            delete,
        )
        .unwrap();
        Command::new("bash")
            .args(["-c", &command])
            .output()
            .unwrap()
    }
    fn server_case(dir: &std::path::Path) {
        fs::create_dir_all(dir.join("runs/r1")).unwrap();
        fs::write(
            dir.join("case.nml"),
            "&colm_inparm\nDEF_CASE_NAME = 'x'\n/\n",
        )
        .unwrap();
        fs::write(
            dir.join(".colm-desktop-server.json"),
            serde_json::json!({"schema":1,"remote_directory":dir}).to_string(),
        )
        .unwrap();
    }
    #[test]
    fn remote_project_cleanup_deletes_only_a_folder_of_its_own_cases_once_jobs_are_done() {
        let f = Fixture::new();
        let folder = f.base.join("uuu");
        for site in ["AT-Neu", "AU-How"] {
            server_case(&folder.join(site));
        }
        let ok = |out: &std::process::Output| {
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        let refused = |out: &std::process::Output, why: &str| {
            assert!(!out.status.success());
            let err = String::from_utf8_lossy(&out.stderr);
            assert!(err.contains(why), "{err}");
        };
        ok(&project(&f, false));
        // Anything the app did not make keeps the whole folder.
        fs::write(folder.join("notes.txt"), "mine").unwrap();
        refused(&project(&f, true), "not a case: notes.txt");
        fs::remove_file(folder.join("notes.txt")).unwrap();
        fs::create_dir(folder.join("other")).unwrap();
        refused(&project(&f, true), "not a case: other");
        fs::remove_dir(folder.join("other")).unwrap();
        // A job still going on the server blocks it.
        fs::create_dir(f.root.join("jobs/run-live")).unwrap();
        refused(&project(&f, true), "not confirmed finished: run-live");
        // A packed station a node job took but that wrote no exit code is not finished either.
        let pool = f.root.join("pools/b1");
        fs::create_dir_all(pool.join("claimed/run-live")).unwrap();
        fs::write(f.root.join("jobs/run-live/scheduler"), "slurm-pack\n").unwrap();
        fs::write(
            f.root.join("jobs/run-live/pool"),
            format!("{}\n", pool.display()),
        )
        .unwrap();
        fs::write(pool.join("claimed/run-live/carrier"), "100\n").unwrap();
        refused(&project(&f, true), "not confirmed finished: run-live");
        // One cancelled before any node job took it can never start, so it does not block.
        fs::remove_file(pool.join("claimed/run-live/carrier")).unwrap();
        fs::write(pool.join("claimed/run-live/cancelled"), "").unwrap();
        ok(&project(&f, true));
        assert!(!folder.exists());
        assert!(f.root.join("jobs/job1/script.sh").exists());
        // The server's own directories are never a project.
        let out = Command::new("bash")
            .args([
                "-c",
                &project_script(
                    f.root.to_str().unwrap(),
                    f.root.join("jobs").to_str().unwrap(),
                    true,
                )
                .unwrap(),
            ])
            .output()
            .unwrap();
        refused(&out, "protected");
        assert!(f.root.join("jobs").exists());
    }
    #[test]
    fn remote_history_cleanup_refuses_lease_wrong_job_and_unsafe_paths() {
        assert!(script("/data", "job", "op", "/data/runs/../op", true, None).is_err());
        assert!(script("/", "job", "op", "/runs/op", true, None).is_err());
        assert!(script("/data", "../job", "op", "/data/runs/op", true, None).is_err());
        let f = Fixture::new();
        fs::create_dir(f.root.join(".engine-leases")).unwrap();
        fs::write(f.root.join(".engine-leases/active"), "").unwrap();
        assert!(!f.run(true).status.success());
        fs::remove_file(f.root.join(".engine-leases/active")).unwrap();
        fs::write(
            f.root.join("jobs/job1/script.sh"),
            "# colm-run-case: \"/other\"\n# colm-operation-id: op1\n",
        )
        .unwrap();
        assert!(!f.run(true).status.success());
        assert!(f.target.exists());
    }
    #[test]
    fn remote_history_cleanup_refuses_uncertain_jobs_identity_and_links() {
        let f = Fixture::new();
        fs::create_dir_all(f.root.join("jobs/pending")).unwrap();
        assert!(!f.run(true).status.success());
        fs::remove_dir(f.root.join("jobs/pending")).unwrap();
        fs::write(f.target.join("model-run.json"), "{}").unwrap();
        assert!(!f.run(true).status.success());
        assert!(f.target.exists());
        let f = Fixture::new();
        std::os::unix::fs::symlink(&f.root, f.target.join("input")).unwrap();
        assert!(!f.run(true).status.success());
        assert!(f.root.exists());
        assert!(f.target.exists());
    }
}
