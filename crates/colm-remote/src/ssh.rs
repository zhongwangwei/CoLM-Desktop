//! 经系统 `ssh` 在远程主机上执行脚本、传文件。

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

/// 一台远程主机（用户 ssh 配置里的别名，或 `user@host`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ssh {
    pub host: String,
}

/// 单引号转义，供拼进远程 shell 命令。
pub fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// 一次 ssh 调用的结果。
#[derive(Debug, Clone)]
pub struct Output {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Ssh {
    pub fn new(host: &str) -> Result<Self> {
        let host = host.trim();
        // 别名或 user@host；拒绝会被 ssh 当成选项的写法。
        if host.is_empty() || host.starts_with('-') || host.contains(char::is_whitespace) {
            bail!("not an ssh host: {host:?}");
        }
        Ok(Self {
            host: host.to_owned(),
        })
    }

    /// ssh 命令的公共部分：不交互、连接超时、保活；Unix 上复用连接（经跳板机时省掉每次握手）。
    fn command(&self) -> Command {
        let mut command = Command::new("ssh");
        command.args([
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=20",
            "-o",
            "ServerAliveInterval=30",
        ]);
        if cfg!(unix) {
            command.args([
                "-o",
                "ControlMaster=auto",
                "-o",
                "ControlPath=/tmp/colm-ssh-%C",
                "-o",
                "ControlPersist=120",
            ]);
        }
        command.arg(&self.host);
        command
    }

    /// 在远程主机上用 bash 执行一段脚本（经 stdin 传，免去层层转义）。
    pub fn run(&self, script: &str) -> Result<Output> {
        let mut child = self
            .command()
            .arg("bash -s")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("cannot start ssh (is OpenSSH installed?)")?;
        child
            .stdin
            .take()
            .context("no stdin")?
            .write_all(script.as_bytes())?;
        let output = child.wait_with_output()?;
        Ok(Output {
            success: output.status.success(),
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    /// 同上，失败时把 stderr 带出来。
    pub fn run_ok(&self, script: &str) -> Result<String> {
        let output = self.run(script)?;
        if !output.success {
            bail!("{}", connection_hint(&self.host, &output));
        }
        Ok(output.stdout)
    }

    /// 把本地 `base` 下的 `entries` 打包传到远程目录 `remote_dir`（不存在就建）。
    pub fn upload(&self, base: &Path, entries: &[String], remote_dir: &str) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        self.upload_with(
            Command::new("tar")
                .arg("-czf")
                .arg("-")
                .arg("-C")
                .arg(base)
                .args(entries),
            remote_dir,
        )
    }

    /// 把 `base` 下的一长串文件传上去：清单经临时文件交给 `tar -T`，不受命令行长度限制。
    pub fn upload_list(&self, base: &Path, files: &[String], remote_dir: &str) -> Result<()> {
        let list = std::env::temp_dir().join(format!("colm-upload-{}.txt", std::process::id()));
        std::fs::write(&list, files.join("\n") + "\n")?;
        let result = self.upload_with(
            Command::new("tar")
                .arg("-czf")
                .arg("-")
                .arg("-C")
                .arg(base)
                .arg("-T")
                .arg(&list),
            remote_dir,
        );
        let _ = std::fs::remove_file(&list);
        result
    }

    /// 把一个现成的 `.tar.gz` 传上去解开。
    pub fn upload_tarball(&self, tarball: &Path, remote_dir: &str) -> Result<()> {
        let file = std::fs::File::open(tarball)
            .with_context(|| format!("cannot read {}", tarball.display()))?;
        self.upload_stream(file.into(), remote_dir)
    }

    /// 把 `producer` 的输出（gzip 压缩的 tar 流）在远程解到 `remote_dir`。
    fn upload_with(&self, producer: &mut Command, remote_dir: &str) -> Result<()> {
        producer
            .env("COPYFILE_DISABLE", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut producer = producer.spawn().context("cannot start tar")?;
        let stream = producer.stdout.take().context("no tar output")?;
        let result = self.upload_stream(stream.into(), remote_dir);
        let producer = producer.wait_with_output()?;
        if !producer.status.success() {
            bail!(
                "packing failed: {}",
                String::from_utf8_lossy(&producer.stderr).trim()
            );
        }
        result
    }

    fn upload_stream(&self, stream: Stdio, remote_dir: &str) -> Result<()> {
        let remote = format!(
            "mkdir -p {dir} && tar -xzf - -C {dir}",
            dir = quote(remote_dir)
        );
        let ssh = self
            .command()
            .arg(remote)
            .stdin(stream)
            .output()
            .context("cannot start ssh")?;
        if !ssh.status.success() {
            bail!(
                "upload to {}:{remote_dir} failed: {}",
                self.host,
                String::from_utf8_lossy(&ssh.stderr).trim()
            );
        }
        Ok(())
    }

    /// 把远程目录 `remote_dir` 下的 `entries` 取回本地目录 `local_dir`。
    pub fn download(&self, remote_dir: &str, entries: &[String], local_dir: &Path) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        std::fs::create_dir_all(local_dir)
            .with_context(|| format!("cannot create {}", local_dir.display()))?;
        let list: Vec<String> = entries.iter().map(|e| quote(e)).collect();
        let remote = format!("tar -czf - -C {} {}", quote(remote_dir), list.join(" "));
        let mut ssh = self
            .command()
            .arg(remote)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("cannot start ssh")?;
        let stream = ssh.stdout.take().context("no ssh output")?;
        let tar = Command::new("tar")
            .arg("-xzf")
            .arg("-")
            .arg("-C")
            .arg(local_dir)
            .stdin(stream)
            .output()
            .context("cannot start tar")?;
        let ssh = ssh.wait_with_output()?;
        if !ssh.status.success() {
            bail!(
                "download from {}:{remote_dir} failed: {}",
                self.host,
                String::from_utf8_lossy(&ssh.stderr).trim()
            );
        }
        if !tar.status.success() {
            bail!(
                "tar failed: {}",
                String::from_utf8_lossy(&tar.stderr).trim()
            );
        }
        Ok(())
    }
}

/// ssh 失败时给用户的说明：连不上、要密码/口令、主机密钥未知，分别该怎么办。
pub fn connection_hint(host: &str, output: &Output) -> String {
    let stderr = output.stderr.trim();
    let hint = if output.code == Some(255) {
        if stderr.contains("Host key verification failed")
            || stderr.contains("REMOTE HOST IDENTIFICATION")
        {
            "the server's host key is unknown or changed; connect once in a terminal (ssh HOST) and check it"
        } else if stderr.contains("Permission denied") {
            "ssh needs a password or one-time code; log in once in a terminal (ssh HOST) so the connection can be reused, or set up a key"
        } else {
            "cannot connect; check the alias in ~/.ssh/config, the VPN or jump host, and that `ssh HOST` works in a terminal"
        }
    } else {
        "the remote command failed"
    };
    let tail: Vec<&str> = stderr.lines().rev().take(4).collect();
    format!(
        "{} ({}): {}",
        hint.replace("HOST", host),
        host,
        tail.into_iter().rev().collect::<Vec<_>>().join(" ")
    )
}

#[cfg(test)]
#[path = "ssh_tests.rs"]
mod ssh_tests;
