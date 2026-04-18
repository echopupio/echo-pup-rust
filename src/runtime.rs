//! 运行时工具：单实例锁与后台启动

use anyhow::{anyhow, Context, Result};
use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

const RUN_LOCK_FILE_NAME: &str = "echopup.lock";
const RUN_LOG_FILE_NAME: &str = "echopup.log";
const STOP_WAIT_RETRY: usize = 50;
const STOP_WAIT_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProcessIdentity {
    Match,
    Mismatch,
    Unknown,
}

fn runtime_dir_path() -> Result<PathBuf> {
    let path = dirs::home_dir()
        .context("无法获取用户目录")?
        .join(".echopup");
    Ok(path)
}

fn runtime_dir() -> Result<PathBuf> {
    let path = runtime_dir_path()?;
    std::fs::create_dir_all(&path).context("创建 ~/.echopup 目录失败")?;
    Ok(path)
}

pub fn model_dir() -> Result<PathBuf> {
    Ok(runtime_dir_path()?.join("models"))
}

pub fn background_log_path() -> Result<PathBuf> {
    Ok(runtime_dir_path()?.join(RUN_LOG_FILE_NAME))
}

fn lock_file_path(name: &str) -> Result<PathBuf> {
    Ok(runtime_dir()?.join(name))
}

fn try_acquire_lock(name: &str) -> Result<Option<File>> {
    let path = lock_file_path(name)?;
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("打开锁文件失败: {}", path.display()))?;

    match file.try_lock_exclusive() {
        Ok(()) => {
            file.set_len(0).ok();
            file.seek(SeekFrom::Start(0)).ok();
            let _ = writeln!(file, "{}", std::process::id());
            let _ = file.flush();
            Ok(Some(file))
        }
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
        Err(e) => Err(e).context("获取实例锁失败"),
    }
}

pub struct InstanceGuard {
    file: File,
}

impl InstanceGuard {
    /// 尝试获取实例锁
    pub fn try_acquire() -> Result<Option<Self>> {
        Ok(try_acquire_lock(RUN_LOCK_FILE_NAME)?.map(|file| Self { file }))
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

/// 检查当前是否已有实例在运行
pub fn is_running() -> Result<bool> {
    let path = lock_file_path(RUN_LOCK_FILE_NAME)?;
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("打开锁文件失败: {}", path.display()))?;

    match file.try_lock_exclusive() {
        Ok(()) => {
            file.unlock().ok();
            Ok(false)
        }
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(true),
        Err(e) => Err(e).context("检查实例状态失败"),
    }
}

fn read_pid_file(path: &PathBuf) -> Result<Option<u32>> {
    let mut file = OpenOptions::new()
        .create(false)
        .read(true)
        .open(path)
        .with_context(|| format!("打开锁文件失败: {}", path.display()))?;

    let mut content = String::new();
    let _ = file.read_to_string(&mut content);
    let pid = content.trim().parse::<u32>().ok();
    Ok(pid)
}

fn process_command(pid: u32) -> Option<String> {
    let output = Command::new("ps")
        .arg("-p")
        .arg(pid.to_string())
        .arg("-o")
        .arg("command=")
        .output();

    match output {
        Ok(out) if out.status.success() => {
            Some(String::from_utf8_lossy(&out.stdout).to_lowercase())
        }
        _ => None,
    }
}

fn classify_process(pid: u32, mode_fragment: &str) -> ProcessIdentity {
    if let Some(cmd) = process_command(pid) {
        let trimmed = cmd.trim();
        if trimmed.is_empty() {
            return ProcessIdentity::Unknown;
        }
        if !trimmed.contains("echopup") {
            return ProcessIdentity::Mismatch;
        }
        if trimmed.contains(mode_fragment) {
            return ProcessIdentity::Match;
        }
        return ProcessIdentity::Unknown;
    }

    ProcessIdentity::Unknown
}

fn run_process_identity(pid: u32) -> ProcessIdentity {
    classify_process(pid, " run")
}

fn send_term(pid: u32) -> Result<()> {
    let status = Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status()
        .with_context(|| format!("发送 TERM 信号失败: pid={}", pid))?;
    if !status.success() {
        return Err(anyhow!("终止进程失败: pid={}", pid));
    }
    Ok(())
}

pub fn running_instance_pid() -> Result<Option<u32>> {
    if !is_running()? {
        return Ok(None);
    }
    let lock_path = lock_file_path(RUN_LOCK_FILE_NAME)?;
    if !lock_path.exists() {
        return Ok(None);
    }
    read_pid_file(&lock_path)
}

pub fn stop_running_instance() -> Result<Option<u32>> {
    let Some(pid) = running_instance_pid()? else {
        return Ok(None);
    };

    if run_process_identity(pid) == ProcessIdentity::Mismatch {
        return Err(anyhow!(
            "检测到运行锁被占用，但 pid {} 不是 echopup run 进程",
            pid
        ));
    }

    send_term(pid)?;

    for _ in 0..STOP_WAIT_RETRY {
        if !is_running()? {
            return Ok(Some(pid));
        }
        thread::sleep(STOP_WAIT_INTERVAL);
    }

    Err(anyhow!("停止 echopup 超时: pid={}", pid))
}

/// 后台启动 echopup run
pub fn spawn_background(config_path: &str) -> Result<u32> {
    let exe = std::env::current_exe().context("获取当前可执行文件路径失败")?;
    let log_path = background_log_path()?;
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("打开后台日志文件失败: {}", log_path.display()))?;
    let log_file_stderr = log_file.try_clone().context("克隆后台日志文件句柄失败")?;

    let child = Command::new(exe)
        .arg("--config")
        .arg(config_path)
        .arg("run")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_file_stderr))
        .spawn()
        .context("后台启动 echopup 失败")?;

    Ok(child.id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_dir_is_under_echopup() {
        let model_dir = model_dir().unwrap();
        assert!(model_dir.ends_with(".echopup/models"));
    }
}
