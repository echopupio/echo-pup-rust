//! 运行时工具：单实例锁与后台启动

use anyhow::{Context, Result};
use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

const RUN_LOCK_FILE_NAME: &str = "catecho.lock";
const UI_LOCK_FILE_NAME: &str = "catecho-ui.pid";

fn runtime_dir() -> Result<PathBuf> {
    let path = dirs::home_dir()
        .context("无法获取用户目录")?
        .join(".catecho");
    std::fs::create_dir_all(&path).context("创建 ~/.catecho 目录失败")?;
    Ok(path)
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

pub struct UiGuard {
    lock_path: PathBuf,
    pid: u32,
}

pub enum UiAcquireMode {
    Fresh,
    TookOverPrevious,
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

impl Drop for UiGuard {
    fn drop(&mut self) {
        if let Ok(Some(pid)) = read_pid_file(&self.lock_path) {
            if pid == self.pid {
                let _ = std::fs::remove_file(&self.lock_path);
            }
        }
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

fn write_pid_file_create_new(path: &PathBuf, pid: u32) -> Result<bool> {
    match OpenOptions::new().create_new(true).write(true).open(path) {
        Ok(mut file) => {
            writeln!(file, "{}", pid).context("写入 UI 锁文件失败")?;
            file.flush().ok();
            Ok(true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(e).with_context(|| format!("创建 UI 锁文件失败: {}", path.display())),
    }
}

fn is_catecho_ui_process(pid: u32) -> bool {
    let output = Command::new("ps")
        .arg("-p")
        .arg(pid.to_string())
        .arg("-o")
        .arg("command=")
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let cmd = String::from_utf8_lossy(&out.stdout).to_lowercase();
            cmd.contains("catecho") && cmd.contains(" ui")
        }
        _ => false,
    }
}

/// 获取 UI 锁；若已有 UI，则接管到当前终端
pub fn acquire_ui_guard_for_foreground() -> Result<(UiGuard, UiAcquireMode)> {
    let lock_path = lock_file_path(UI_LOCK_FILE_NAME)?;
    let current_pid = std::process::id();

    if write_pid_file_create_new(&lock_path, current_pid)? {
        return Ok((
            UiGuard {
                lock_path,
                pid: current_pid,
            },
            UiAcquireMode::Fresh,
        ));
    }

    if let Ok(Some(pid)) = read_pid_file(&lock_path) {
        if pid != std::process::id() {
            if is_catecho_ui_process(pid) {
                let _ = Command::new("kill")
                    .arg("-TERM")
                    .arg(pid.to_string())
                    .status();
            } else {
                let _ = std::fs::remove_file(&lock_path);
            }
        }
    } else {
        let _ = std::fs::remove_file(&lock_path);
    }

    for _ in 0..50 {
        if write_pid_file_create_new(&lock_path, current_pid)? {
            return Ok((
                UiGuard {
                    lock_path,
                    pid: current_pid,
                },
                UiAcquireMode::TookOverPrevious,
            ));
        }
        if let Ok(Some(pid)) = read_pid_file(&lock_path) {
            if pid != current_pid && !is_catecho_ui_process(pid) {
                let _ = std::fs::remove_file(&lock_path);
            }
        } else {
            let _ = std::fs::remove_file(&lock_path);
        }
        thread::sleep(Duration::from_millis(100));
    }

    anyhow::bail!("catecho ui 正在运行，且当前无法接管")
}

/// 后台启动 catecho run
pub fn spawn_background(config_path: &str) -> Result<u32> {
    let exe = std::env::current_exe().context("获取当前可执行文件路径失败")?;
    let child = Command::new(exe)
        .arg("--config")
        .arg(config_path)
        .arg("run")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("后台启动 catecho 失败")?;

    Ok(child.id())
}
