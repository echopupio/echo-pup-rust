//! 共享模型下载能力（TUI / 状态栏菜单复用）

#![allow(dead_code)]

use anyhow::{anyhow, Context, Result};
use reqwest::header::{CONTENT_RANGE, RANGE};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::runtime;

pub const DOWNLOAD_LOG_MAX_LINES: usize = 120;
pub const DOWNLOAD_CHUNK_SIZE: u64 = 4 * 1024 * 1024;
pub const DOWNLOAD_PARALLEL_CHUNK_SIZE: u64 = 16 * 1024 * 1024;
pub const DOWNLOAD_MAX_CONCURRENT_REQUESTS: usize = 8;
pub const DOWNLOAD_NO_PROGRESS_TIMEOUT_SECS: u64 = 300;
pub const DOWNLOAD_MAX_RETRIES: usize = 6;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadState {
    pub model_size: String,
    pub model_file_name: String,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub in_progress: bool,
}

#[derive(Debug, Clone)]
pub enum DownloadEvent {
    Started {
        model_size: String,
        model_file_name: String,
        downloaded: u64,
        total: Option<u64>,
    },
    Progress {
        downloaded: u64,
        total: Option<u64>,
    },
    Finished,
    Failed(String),
    Log(String),
}

pub struct DownloadStart {
    pub state: DownloadState,
    pub rx: Receiver<DownloadEvent>,
    pub initial_logs: Vec<String>,
}

#[derive(Debug, Clone)]
struct DownloadPartPlan {
    index: usize,
    start: u64,
    end: u64,
    path: PathBuf,
}

fn normalize_content_length(content_length: Option<u64>) -> Option<u64> {
    content_length.filter(|&len| len > 0)
}

fn build_http_client(ignore_env_proxy: bool) -> Result<reqwest::blocking::Client> {
    let mut builder = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .timeout(None);
    if ignore_env_proxy {
        builder = builder.no_proxy();
    }
    builder.build().context("初始化 HTTP 客户端失败")
}

fn is_likely_proxy_error(err: &reqwest::Error) -> bool {
    let msg = err.to_string().to_ascii_lowercase();
    msg.contains("proxy")
        || msg.contains("tunnel")
        || msg.contains("127.0.0.1:7892")
        || msg.contains("127.0.0.1")
}

fn cleanup_empty_tmp_file_after_failure(model_file_name: &str, tx: &mpsc::Sender<DownloadEvent>) {
    let Ok(model_dir) = runtime::model_dir() else {
        return;
    };
    let tmp_file = model_dir.join(format!("{}.part", model_file_name));
    let Ok(meta) = fs::metadata(&tmp_file) else {
        return;
    };
    if meta.len() > 0 {
        return;
    }
    if fs::remove_file(&tmp_file).is_ok() {
        let _ = tx.send(DownloadEvent::Log(format!(
            "[clean] 下载失败后已删除空临时文件: {}",
            tmp_file.display()
        )));
    }
}

fn cleanup_parallel_parts_dir(parts_dir: &Path) {
    if parts_dir.exists() {
        let _ = fs::remove_dir_all(parts_dir);
    }
}

fn build_parallel_download_plan(
    total_size: u64,
    chunk_size: u64,
    parts_dir: &Path,
) -> Vec<DownloadPartPlan> {
    if total_size == 0 || chunk_size == 0 {
        return Vec::new();
    }

    let mut plans = Vec::new();
    let mut start = 0u64;
    let mut index = 0usize;
    while start < total_size {
        let end = start
            .saturating_add(chunk_size)
            .saturating_sub(1)
            .min(total_size.saturating_sub(1));
        plans.push(DownloadPartPlan {
            index,
            start,
            end,
            path: parts_dir.join(format!("part-{:04}", index)),
        });
        start = end.saturating_add(1);
        index += 1;
    }
    plans
}

fn should_try_parallel_download(total_size: Option<u64>, resume_size: u64) -> bool {
    resume_size == 0
        && total_size.is_some_and(|total| total >= DOWNLOAD_PARALLEL_CHUNK_SIZE.saturating_mul(2))
}

fn emit_parallel_progress(
    downloaded: &Arc<AtomicU64>,
    reported: &Arc<AtomicU64>,
    total_size: u64,
    tx: &mpsc::Sender<DownloadEvent>,
) {
    let current = downloaded.load(Ordering::SeqCst);
    let mut last = reported.load(Ordering::SeqCst);
    while current > last {
        match reported.compare_exchange(last, current, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => {
                let _ = tx.send(DownloadEvent::Progress {
                    downloaded: current,
                    total: Some(total_size),
                });
                break;
            }
            Err(actual) => last = actual,
        }
    }
}

fn probe_range_support(
    client: &mut reqwest::blocking::Client,
    using_direct_client: &mut bool,
    model_url: &str,
    tx: &mpsc::Sender<DownloadEvent>,
) -> Result<bool> {
    let mut attempt = 0usize;
    let range_text = "bytes=0-0";
    loop {
        attempt += 1;
        let _ = tx.send(DownloadEvent::Log(format!(
            "[probe] range={} attempt={}/{}",
            range_text, attempt, DOWNLOAD_MAX_RETRIES
        )));

        let response = match client.get(model_url).header(RANGE, range_text).send() {
            Ok(resp) => resp,
            Err(err) => {
                if !*using_direct_client && is_likely_proxy_error(&err) {
                    let _ = tx.send(DownloadEvent::Log(format!(
                        "[proxy] range probe 走代理失败，切换直连重试: {}",
                        err
                    )));
                    *client = build_http_client(true)?;
                    *using_direct_client = true;
                    continue;
                }
                if attempt >= DOWNLOAD_MAX_RETRIES {
                    let _ = tx.send(DownloadEvent::Log(format!(
                        "[probe] range 探测失败，回退串行下载: {}",
                        err
                    )));
                    return Ok(false);
                }
                std::thread::sleep(Duration::from_secs(attempt as u64));
                continue;
            }
        };

        let status = response.status();
        let _ = tx.send(DownloadEvent::Log(format!(
            "[probe] status={} content-range={}",
            status,
            response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("无")
        )));

        if status == StatusCode::PARTIAL_CONTENT {
            return Ok(true);
        }

        if status.is_success() {
            let _ = tx.send(DownloadEvent::Log(format!(
                "[probe] 服务端未返回 206（status={}），回退串行下载",
                status
            )));
            return Ok(false);
        }

        if attempt >= DOWNLOAD_MAX_RETRIES {
            let _ = tx.send(DownloadEvent::Log(format!(
                "[probe] 探测失败，回退串行下载: HTTP {}",
                status
            )));
            return Ok(false);
        }
        std::thread::sleep(Duration::from_secs(attempt as u64));
    }
}

fn download_part_with_retries(
    base_client: reqwest::blocking::Client,
    using_direct_client: bool,
    model_url: &str,
    plan: DownloadPartPlan,
    total_size: u64,
    tx: mpsc::Sender<DownloadEvent>,
    downloaded: Arc<AtomicU64>,
    reported: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
) -> Result<()> {
    let chunk_target_len = plan.end.saturating_sub(plan.start).saturating_add(1);
    let range_text = format!("bytes={}-{}", plan.start, plan.end);
    let mut client = base_client;
    let mut using_direct_client = using_direct_client;
    let mut attempt = 0usize;

    'attempt: loop {
        if cancel.load(Ordering::SeqCst) {
            return Ok(());
        }
        attempt += 1;
        let _ = tx.send(DownloadEvent::Log(format!(
            "[parallel {}] range={} attempt={}/{}",
            plan.index, range_text, attempt, DOWNLOAD_MAX_RETRIES
        )));

        let mut response = match client
            .get(model_url)
            .header(RANGE, range_text.clone())
            .send()
        {
            Ok(resp) => resp,
            Err(err) => {
                if !using_direct_client && is_likely_proxy_error(&err) {
                    let _ = tx.send(DownloadEvent::Log(format!(
                        "[parallel {}] 走代理失败，切换直连重试: {}",
                        plan.index, err
                    )));
                    client = build_http_client(true)?;
                    using_direct_client = true;
                    continue;
                }
                if attempt >= DOWNLOAD_MAX_RETRIES {
                    return Err(anyhow!(
                        "并发分片 {} 下载失败（{} 次重试后仍失败）: {}",
                        plan.index,
                        DOWNLOAD_MAX_RETRIES,
                        err
                    ));
                }
                std::thread::sleep(Duration::from_secs(attempt as u64));
                continue;
            }
        };

        let status = response.status();
        if status != StatusCode::PARTIAL_CONTENT {
            if attempt >= DOWNLOAD_MAX_RETRIES {
                return Err(anyhow!(
                    "服务端不支持并发分片下载（range={}, status={}）",
                    range_text,
                    status
                ));
            }
            std::thread::sleep(Duration::from_secs(attempt as u64));
            continue;
        }

        let mut writer = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&plan.path)
            .with_context(|| format!("打开分片文件失败: {}", plan.path.display()))?;

        let mut buf = [0u8; 64 * 1024];
        let mut written = 0u64;
        let mut last_emit = Instant::now();
        loop {
            if cancel.load(Ordering::SeqCst) {
                return Ok(());
            }

            let n = match response.read(&mut buf) {
                Ok(n) => n,
                Err(err) => {
                    if attempt >= DOWNLOAD_MAX_RETRIES {
                        return Err(anyhow!(
                            "并发分片 {} 读取失败（{} 次重试后仍失败）: {}",
                            plan.index,
                            DOWNLOAD_MAX_RETRIES,
                            err
                        ));
                    }
                    std::thread::sleep(Duration::from_secs(attempt as u64));
                    continue 'attempt;
                }
            };
            if n == 0 {
                break;
            }

            let remain = chunk_target_len.saturating_sub(written) as usize;
            let to_write = n.min(remain);
            if to_write == 0 {
                break;
            }
            writer
                .write_all(&buf[..to_write])
                .with_context(|| format!("写入分片文件失败: {}", plan.path.display()))?;
            written += to_write as u64;
            downloaded.fetch_add(to_write as u64, Ordering::SeqCst);

            if last_emit.elapsed() >= Duration::from_millis(120) {
                emit_parallel_progress(&downloaded, &reported, total_size, &tx);
                last_emit = Instant::now();
            }

            if written >= chunk_target_len {
                break;
            }
        }

        writer
            .flush()
            .with_context(|| format!("刷新分片文件失败: {}", plan.path.display()))?;

        if written != chunk_target_len {
            if attempt >= DOWNLOAD_MAX_RETRIES {
                return Err(anyhow!(
                    "分片 {} 下载不完整：期望 {}，实际 {}",
                    plan.index,
                    format_bytes(chunk_target_len),
                    format_bytes(written)
                ));
            }
            let _ = tx.send(DownloadEvent::Log(format!(
                "[parallel {}] 分片大小不完整，{} 秒后重试",
                plan.index, attempt
            )));
            std::thread::sleep(Duration::from_secs(attempt as u64));
            continue;
        }

        emit_parallel_progress(&downloaded, &reported, total_size, &tx);
        let _ = tx.send(DownloadEvent::Log(format!(
            "[parallel {}] 分片完成: {}",
            plan.index,
            format_bytes(written)
        )));
        return Ok(());
    }
}

fn try_parallel_download(
    client: &reqwest::blocking::Client,
    using_direct_client: bool,
    model_url: &str,
    total_size: u64,
    tmp_file: &Path,
    tx: &mpsc::Sender<DownloadEvent>,
) -> Result<()> {
    let parts_dir = tmp_file.with_extension("parts");
    cleanup_parallel_parts_dir(&parts_dir);
    fs::create_dir_all(&parts_dir).context("创建并发分片目录失败")?;

    let plans = build_parallel_download_plan(total_size, DOWNLOAD_PARALLEL_CHUNK_SIZE, &parts_dir);
    if plans.len() < 2 {
        cleanup_parallel_parts_dir(&parts_dir);
        return Err(anyhow!("文件过小，不适合并发分片下载"));
    }

    let parallelism = plans.len().min(DOWNLOAD_MAX_CONCURRENT_REQUESTS).max(1);
    let _ = tx.send(DownloadEvent::Log(format!(
        "[parallel] 启用并发分片下载: workers={}, parts={}, part_size={}",
        parallelism,
        plans.len(),
        format_bytes(DOWNLOAD_PARALLEL_CHUNK_SIZE)
    )));

    let plans = Arc::new(plans);
    let next_index = Arc::new(AtomicUsize::new(0));
    let downloaded = Arc::new(AtomicU64::new(0));
    let reported = Arc::new(AtomicU64::new(0));
    let cancel = Arc::new(AtomicBool::new(false));
    let mut handles = Vec::with_capacity(parallelism);

    for _ in 0..parallelism {
        let worker_client = client.clone();
        let worker_model_url = model_url.to_string();
        let worker_tx = tx.clone();
        let worker_plans = plans.clone();
        let worker_next_index = next_index.clone();
        let worker_downloaded = downloaded.clone();
        let worker_reported = reported.clone();
        let worker_cancel = cancel.clone();
        handles.push(std::thread::spawn(move || -> Result<()> {
            loop {
                if worker_cancel.load(Ordering::SeqCst) {
                    return Ok(());
                }
                let idx = worker_next_index.fetch_add(1, Ordering::SeqCst);
                let Some(plan) = worker_plans.get(idx).cloned() else {
                    return Ok(());
                };
                if let Err(err) = download_part_with_retries(
                    worker_client.clone(),
                    using_direct_client,
                    &worker_model_url,
                    plan,
                    total_size,
                    worker_tx.clone(),
                    worker_downloaded.clone(),
                    worker_reported.clone(),
                    worker_cancel.clone(),
                ) {
                    worker_cancel.store(true, Ordering::SeqCst);
                    return Err(err);
                }
            }
        }));
    }

    let mut first_err = None;
    for handle in handles {
        match handle.join() {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                if first_err.is_none() {
                    first_err = Some(err);
                }
            }
            Err(_) => {
                if first_err.is_none() {
                    first_err = Some(anyhow!("并发下载线程异常退出"));
                }
            }
        }
    }

    if let Some(err) = first_err {
        cleanup_parallel_parts_dir(&parts_dir);
        return Err(err);
    }

    let _ = tx.send(DownloadEvent::Log(
        "[parallel] 所有分片已完成，开始合并".to_string(),
    ));
    let mut writer = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(tmp_file)
        .context("打开模型临时文件失败")?;

    for plan in plans.iter() {
        let mut reader = OpenOptions::new()
            .read(true)
            .open(&plan.path)
            .with_context(|| format!("打开分片文件失败: {}", plan.path.display()))?;
        std::io::copy(&mut reader, &mut writer)
            .with_context(|| format!("合并分片失败: {}", plan.path.display()))?;
    }
    writer.flush().context("刷新模型临时文件失败")?;
    cleanup_parallel_parts_dir(&parts_dir);
    emit_parallel_progress(&downloaded, &reported, total_size, tx);
    Ok(())
}

pub fn list_local_models() -> Vec<String> {
    let mut models = Vec::new();
    if let Ok(model_dir) = runtime::model_dir() {
        if let Ok(entries) = fs::read_dir(model_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.eq_ignore_ascii_case("bin"))
                    .unwrap_or(false)
                {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        models.push(name.to_string());
                    }
                }
            }
        }
    }
    models.sort();
    models
}

pub fn start_model_download(model_size: &str) -> Result<DownloadStart> {
    let model_file_name = resolve_model_file_name(model_size)
        .ok_or_else(|| anyhow!("不支持的模型大小: {}", model_size))?
        .to_string();

    let model_size_owned = model_size.to_string();
    let model_file_name_for_thread = model_file_name.clone();
    let model_url = model_download_url(&model_file_name);
    let model_dir = runtime::model_dir()?;
    let (tx, rx) = mpsc::channel();

    let initial_logs = vec![
        format!("[start] 准备下载模型 {}", model_size_owned),
        format!(
            "[equiv] curl -fL -C - -o \"{}/{}.part\" \"{}\"",
            model_dir.display(),
            model_file_name_for_thread,
            model_url
        ),
    ];

    let state = DownloadState {
        model_size: model_size_owned.clone(),
        model_file_name,
        downloaded: 0,
        total: None,
        in_progress: true,
    };

    std::thread::spawn(move || {
        let cleanup_name = model_file_name_for_thread.clone();
        if let Err(err) = download_model_with_progress(
            model_size_owned.clone(),
            model_file_name_for_thread,
            tx.clone(),
        ) {
            cleanup_empty_tmp_file_after_failure(&cleanup_name, &tx);
            let _ = tx.send(DownloadEvent::Failed(err.to_string()));
        }
    });

    Ok(DownloadStart {
        state,
        rx,
        initial_logs,
    })
}

fn download_model_with_progress(
    model_size: String,
    model_file_name: String,
    tx: mpsc::Sender<DownloadEvent>,
) -> Result<()> {
    let model_dir = runtime::model_dir()?;
    fs::create_dir_all(&model_dir).context("创建 ~/.echopup/models 目录失败")?;

    let model_file = model_dir.join(&model_file_name);
    let tmp_file = model_dir.join(format!("{}.part", model_file_name));

    if model_file.exists() {
        let metadata = fs::metadata(&model_file).context("读取模型文件元数据失败")?;
        if metadata.len() > 0 {
            let _ = tx.send(DownloadEvent::Log(format!(
                "[skip] 模型已存在: {} ({})",
                model_file.display(),
                format_bytes(metadata.len())
            )));
            let _ = tx.send(DownloadEvent::Started {
                model_size,
                model_file_name,
                downloaded: metadata.len(),
                total: Some(metadata.len()),
            });
            let _ = tx.send(DownloadEvent::Finished);
            return Ok(());
        }
        let _ = tx.send(DownloadEvent::Log(format!(
            "[clean] 发现空模型文件，已删除: {}",
            model_file.display()
        )));
        let _ = fs::remove_file(&model_file);
    }

    let resume_size = fs::metadata(&tmp_file).map(|m| m.len()).unwrap_or(0);
    if resume_size == 0 && tmp_file.exists() {
        let _ = tx.send(DownloadEvent::Log(format!(
            "[clean] 发现空临时文件，已删除: {}",
            tmp_file.display()
        )));
        let _ = fs::remove_file(&tmp_file);
    }
    if resume_size > 0 {
        let _ = tx.send(DownloadEvent::Log(format!(
            "[resume] 发现临时文件: {} ({})",
            tmp_file.display(),
            format_bytes(resume_size)
        )));
    }

    let model_url = model_download_url(&model_file_name);
    let _ = tx.send(DownloadEvent::Log(format!(
        "[info] 目标文件: {}",
        model_file.display()
    )));
    let _ = tx.send(DownloadEvent::Log(format!(
        "[info] 模型地址: {}",
        model_url
    )));
    let _ = tx.send(DownloadEvent::Log(format!(
        "[equiv] curl -I \"{}\"",
        model_url
    )));

    let mut client = build_http_client(false)?;
    let mut using_direct_client = false;

    let mut total_size = None;
    match client.head(&model_url).send() {
        Ok(resp) => {
            let status = resp.status();
            let content_length = if status.is_success() {
                normalize_content_length(resp.content_length())
            } else {
                None
            };
            total_size = content_length;
            let _ = tx.send(DownloadEvent::Log(format!(
                "[head] status={} content-length={}",
                status,
                content_length
                    .map(format_bytes)
                    .unwrap_or_else(|| "未知".to_string())
            )));
        }
        Err(err) => {
            if is_likely_proxy_error(&err) {
                let _ = tx.send(DownloadEvent::Log(format!(
                    "[proxy] HEAD 走代理失败，尝试直连重试: {}",
                    err
                )));
                client = build_http_client(true)?;
                using_direct_client = true;
                match client.head(&model_url).send() {
                    Ok(resp) => {
                        let status = resp.status();
                        let content_length = if status.is_success() {
                            normalize_content_length(resp.content_length())
                        } else {
                            None
                        };
                        total_size = content_length;
                        let _ = tx.send(DownloadEvent::Log(format!(
                            "[head] (direct) status={} content-length={}",
                            status,
                            content_length
                                .map(format_bytes)
                                .unwrap_or_else(|| "未知".to_string())
                        )));
                    }
                    Err(err2) => {
                        let _ =
                            tx.send(DownloadEvent::Log(format!("[head] 直连重试失败: {}", err2)));
                    }
                }
            } else {
                let _ = tx.send(DownloadEvent::Log(format!("[head] 请求失败: {}", err)));
            }
        }
    }

    let _ = tx.send(DownloadEvent::Started {
        model_size,
        model_file_name,
        downloaded: resume_size,
        total: total_size,
    });
    let _ = tx.send(DownloadEvent::Log(format!(
        "[info] 续传起点: {}",
        format_bytes(resume_size)
    )));
    let _ = tx.send(DownloadEvent::Log(format!(
        "[equiv] curl -fL -C - -o \"{}\" \"{}\"",
        tmp_file.display(),
        model_url
    )));

    if should_try_parallel_download(total_size, resume_size) {
        match probe_range_support(&mut client, &mut using_direct_client, &model_url, &tx) {
            Ok(true) => match try_parallel_download(
                &client,
                using_direct_client,
                &model_url,
                total_size.unwrap_or(0),
                &tmp_file,
                &tx,
            ) {
                Ok(()) => {
                    let _ = tx.send(DownloadEvent::Log(
                        "[parallel] 并发分片下载完成，跳过串行回退".to_string(),
                    ));
                }
                Err(err) => {
                    let _ = tx.send(DownloadEvent::Log(format!(
                        "[parallel] 并发分片下载失败，回退串行: {}",
                        err
                    )));
                    cleanup_parallel_parts_dir(&tmp_file.with_extension("parts"));
                }
            },
            Ok(false) => {
                let _ = tx.send(DownloadEvent::Log(
                    "[parallel] 服务端或环境不适合并发分片，继续使用串行下载".to_string(),
                ));
            }
            Err(err) => {
                let _ = tx.send(DownloadEvent::Log(format!(
                    "[parallel] 并发分片探测异常，继续使用串行下载: {}",
                    err
                )));
            }
        }

        if tmp_file.exists() {
            let downloaded = fs::metadata(&tmp_file)
                .map(|m| m.len())
                .context("读取并发下载临时文件大小失败")?;
            if total_size.is_some_and(|total| downloaded >= total) {
                fs::rename(&tmp_file, &model_file).context("保存模型文件失败")?;
                let _ = tx.send(DownloadEvent::Log(format!(
                    "[save] 写入完成: {}",
                    model_file.display()
                )));
                let _ = tx.send(DownloadEvent::Log(format!(
                    "[save] 文件大小: {}",
                    format_bytes(downloaded)
                )));
                let _ = tx.send(DownloadEvent::Progress {
                    downloaded,
                    total: total_size.or(Some(downloaded)),
                });
                let _ = tx.send(DownloadEvent::Finished);
                return Ok(());
            }
        }
    }

    let mut writer = if resume_size > 0 {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&tmp_file)
            .context("打开模型临时文件失败")?
    } else {
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp_file)
            .context("打开模型临时文件失败")?
    };

    let mut downloaded = resume_size;
    let mut buf = [0u8; 64 * 1024];
    let mut last_emit = Instant::now();

    loop {
        if total_size.is_some_and(|total| downloaded >= total) {
            break;
        }

        let chunk_end = downloaded
            .saturating_add(DOWNLOAD_CHUNK_SIZE)
            .saturating_sub(1);
        let range_end = total_size
            .map(|total| chunk_end.min(total.saturating_sub(1)))
            .unwrap_or(chunk_end);
        let range_text = format!("bytes={}-{}", downloaded, range_end);

        let chunk_target_len = range_end.saturating_sub(downloaded).saturating_add(1);
        let mut attempt = 0usize;
        let chunk_written: u64 = 'attempt: loop {
            attempt += 1;
            let _ = tx.send(DownloadEvent::Log(format!(
                "[get] range={} attempt={}/{} timeout={}s",
                range_text, attempt, DOWNLOAD_MAX_RETRIES, DOWNLOAD_NO_PROGRESS_TIMEOUT_SECS
            )));

            let mut response = match client
                .get(&model_url)
                .header(RANGE, range_text.clone())
                .send()
            {
                Ok(resp) => resp,
                Err(err) => {
                    if !using_direct_client && is_likely_proxy_error(&err) {
                        let _ = tx.send(DownloadEvent::Log(format!(
                            "[proxy] GET 走代理失败，切换直连重试: {}",
                            err
                        )));
                        client = build_http_client(true)?;
                        using_direct_client = true;
                        continue;
                    }
                    if attempt >= DOWNLOAD_MAX_RETRIES {
                        return Err(anyhow!(
                            "下载失败（{} 次重试后仍失败）: {}",
                            DOWNLOAD_MAX_RETRIES,
                            err
                        ));
                    }
                    let _ = tx.send(DownloadEvent::Log(format!(
                        "[retry] 请求失败，{} 秒后重试: {}",
                        attempt, err
                    )));
                    std::thread::sleep(Duration::from_secs(attempt as u64));
                    continue;
                }
            };

            let status = response.status();
            let _ = tx.send(DownloadEvent::Log(format!(
                "[get] status={} content-length={}",
                status,
                response
                    .content_length()
                    .map(format_bytes)
                    .unwrap_or_else(|| "未知".to_string())
            )));

            if status == StatusCode::RANGE_NOT_SATISFIABLE {
                total_size = total_size.or_else(|| {
                    response
                        .headers()
                        .get(CONTENT_RANGE)
                        .and_then(|v| v.to_str().ok())
                        .and_then(parse_total_from_content_range)
                });
                break 0;
            }

            if !status.is_success() {
                if attempt >= DOWNLOAD_MAX_RETRIES {
                    return Err(anyhow!("下载失败，HTTP {}", status));
                }
                let _ = tx.send(DownloadEvent::Log(format!(
                    "[retry] HTTP {}，{} 秒后重试",
                    status, attempt
                )));
                std::thread::sleep(Duration::from_secs(attempt as u64));
                continue;
            }

            if downloaded > 0 && status != StatusCode::PARTIAL_CONTENT {
                if attempt >= DOWNLOAD_MAX_RETRIES {
                    return Err(anyhow!(
                        "服务端不支持续传（status={}），请删除 .part 后重试",
                        status
                    ));
                }
                let _ = tx.send(DownloadEvent::Log(format!(
                    "[retry] 服务端未返回 206（status={}），{} 秒后重试",
                    status, attempt
                )));
                std::thread::sleep(Duration::from_secs(attempt as u64));
                continue;
            }

            if total_size.is_none() {
                total_size = response
                    .headers()
                    .get(CONTENT_RANGE)
                    .and_then(|v| v.to_str().ok())
                    .and_then(parse_total_from_content_range)
                    .or_else(|| {
                        normalize_content_length(response.content_length()).map(|len| {
                            if status == StatusCode::PARTIAL_CONTENT {
                                downloaded + len
                            } else {
                                len
                            }
                        })
                    });
            }

            let mut written = 0u64;
            loop {
                let n = match response.read(&mut buf) {
                    Ok(n) => n,
                    Err(err) => {
                        if written > 0 {
                            let _ = tx.send(DownloadEvent::Log(format!(
                                "[warn] 本段已写入 {}，读取中断，继续下一段: {}",
                                format_bytes(written),
                                err
                            )));
                            break;
                        }
                        if attempt >= DOWNLOAD_MAX_RETRIES {
                            return Err(anyhow!(
                                "下载失败（{} 次重试后读取仍失败）: {}",
                                DOWNLOAD_MAX_RETRIES,
                                err
                            ));
                        }
                        let _ = tx.send(DownloadEvent::Log(format!(
                            "[retry] 读取失败，{} 秒后重试: {}",
                            attempt, err
                        )));
                        std::thread::sleep(Duration::from_secs(attempt as u64));
                        continue 'attempt;
                    }
                };
                if n == 0 {
                    break;
                }
                let remain = chunk_target_len.saturating_sub(written) as usize;
                let to_write = n.min(remain);
                if to_write == 0 {
                    break;
                }
                writer
                    .write_all(&buf[..to_write])
                    .context("写入模型临时文件失败")?;
                downloaded += to_write as u64;
                written += to_write as u64;

                if last_emit.elapsed() >= Duration::from_millis(120) {
                    let _ = tx.send(DownloadEvent::Progress {
                        downloaded,
                        total: total_size,
                    });
                    last_emit = Instant::now();
                }

                if written >= chunk_target_len {
                    break;
                }
            }
            writer.flush().context("刷新模型临时文件失败")?;

            if written == 0 {
                if total_size.is_some_and(|total| downloaded >= total) {
                    break 0;
                }
                if attempt >= DOWNLOAD_MAX_RETRIES {
                    return Err(anyhow!(
                        "下载失败（{} 次重试后仍无数据）",
                        DOWNLOAD_MAX_RETRIES
                    ));
                }
                let _ = tx.send(DownloadEvent::Log(format!(
                    "[retry] 本次未收到数据，{} 秒后重试",
                    attempt
                )));
                std::thread::sleep(Duration::from_secs(attempt as u64));
                continue;
            }

            break written;
        };

        if chunk_written == 0 && total_size.is_some_and(|total| downloaded >= total) {
            break;
        }
        if chunk_written == 0 && total_size.is_none() {
            break;
        }
    }

    if downloaded == 0 {
        return Err(anyhow!("下载失败：未接收到任何数据"));
    }

    fs::rename(&tmp_file, &model_file).context("保存模型文件失败")?;
    let _ = tx.send(DownloadEvent::Log(format!(
        "[save] 写入完成: {}",
        model_file.display()
    )));

    let final_size = fs::metadata(&model_file)
        .map(|m| m.len())
        .unwrap_or(downloaded);
    let _ = tx.send(DownloadEvent::Log(format!(
        "[save] 文件大小: {}",
        format_bytes(final_size)
    )));
    let _ = tx.send(DownloadEvent::Progress {
        downloaded: final_size,
        total: total_size.or(Some(final_size)),
    });
    let _ = tx.send(DownloadEvent::Finished);

    Ok(())
}

pub fn resolve_model_file_name(model_size: &str) -> Option<&'static str> {
    match model_size {
        "large" | "large-v3" => Some("ggml-large-v3.bin"),
        "turbo" | "large-v3-turbo" => Some("ggml-large-v3-turbo.bin"),
        "medium" => Some("ggml-medium.bin"),
        _ => None,
    }
}

pub fn model_download_url(model_file_name: &str) -> String {
    format!(
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
        model_file_name
    )
}

pub fn paraformer_model_files() -> &'static [&'static str] {
    &["encoder.onnx", "decoder.onnx", "tokens.txt"]
}

pub fn paraformer_model_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".echopup")
        .join("models")
        .join("asr")
        .join("sherpa-onnx-streaming-paraformer-bilingual-zh-en")
}

pub fn paraformer_model_download_url(file_name: &str) -> String {
    format!(
        "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-paraformer-bilingual-zh-en/resolve/main/{}",
        file_name
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SingleFileDownloadOutcome {
    Downloaded,
    Skipped,
}

fn paraformer_tmp_path(dest_path: &Path) -> PathBuf {
    let file_name = dest_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("model");
    dest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("{}.part", file_name))
}

fn should_reuse_existing_download(existing_size: u64, total_size: Option<u64>) -> bool {
    existing_size > 0 && total_size.is_some_and(|total| existing_size == total)
}

fn cleanup_empty_file(path: &Path, tx: &mpsc::Sender<DownloadEvent>) {
    let Ok(meta) = fs::metadata(path) else {
        return;
    };
    if meta.len() > 0 {
        return;
    }
    if fs::remove_file(path).is_ok() {
        let _ = tx.send(DownloadEvent::Log(format!(
            "[clean] 下载失败后已删除空文件: {}",
            path.display()
        )));
    }
}

fn probe_single_file_remote_size(
    client: &mut reqwest::blocking::Client,
    using_direct_client: &mut bool,
    url: &str,
    tx: &mpsc::Sender<DownloadEvent>,
) -> Option<u64> {
    let mut total_size = None;

    loop {
        match client.head(url).send() {
            Ok(resp) => {
                if resp.status().is_success() {
                    total_size = normalize_content_length(resp.content_length());
                    let _ = tx.send(DownloadEvent::Log(format!(
                        "[head] {} - size: {}",
                        url,
                        total_size
                            .map(|s| format_bytes(s))
                            .unwrap_or_else(|| "未知".to_string())
                    )));
                }
                break;
            }
            Err(err) => {
                if !*using_direct_client && is_likely_proxy_error(&err) {
                    *client = build_http_client(true).ok()?;
                    *using_direct_client = true;
                    continue;
                }
                let _ = tx.send(DownloadEvent::Log(format!("[head] 探测失败: {}", err)));
                break;
            }
        }
    }

    if total_size.is_some() {
        return total_size;
    }

    loop {
        match client.get(url).header(RANGE, "bytes=0-0").send() {
            Ok(resp) => {
                let status = resp.status();
                total_size = resp
                    .headers()
                    .get(CONTENT_RANGE)
                    .and_then(|v| v.to_str().ok())
                    .and_then(parse_total_from_content_range)
                    .or_else(|| {
                        if status.is_success() && status != StatusCode::PARTIAL_CONTENT {
                            normalize_content_length(resp.content_length())
                        } else {
                            None
                        }
                    });
                let _ = tx.send(DownloadEvent::Log(format!(
                    "[probe-size] status={} content-range={} total={}",
                    status,
                    resp.headers()
                        .get(CONTENT_RANGE)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("无"),
                    total_size
                        .map(|s| format_bytes(s))
                        .unwrap_or_else(|| "未知".to_string())
                )));
                break;
            }
            Err(err) => {
                if !*using_direct_client && is_likely_proxy_error(&err) {
                    *client = build_http_client(true).ok()?;
                    *using_direct_client = true;
                    continue;
                }
                let _ = tx.send(DownloadEvent::Log(format!(
                    "[probe-size] 探测失败: {}",
                    err
                )));
                break;
            }
        }
    }

    total_size
}

pub fn start_paraformer_model_download() -> Result<DownloadStart> {
    let model_dir = paraformer_model_dir();
    let (tx, rx) = mpsc::channel();

    let initial_logs = vec![
        "[start] 准备下载 Sherpa Paraformer 模型".to_string(),
        format!("[dir] 模型目录: {}", model_dir.display()),
        "[info] 将下载以下文件: encoder.onnx, decoder.onnx, tokens.txt".to_string(),
    ];

    let state = DownloadState {
        model_size: "paraformer".to_string(),
        model_file_name: "encoder.onnx".to_string(),
        downloaded: 0,
        total: None,
        in_progress: true,
    };

    std::thread::spawn(move || {
        if let Err(err) = download_paraformer_model_files(&model_dir, tx.clone()) {
            let _ = tx.send(DownloadEvent::Failed(err.to_string()));
        }
    });

    Ok(DownloadStart {
        state,
        rx,
        initial_logs,
    })
}

fn download_paraformer_model_files(
    model_dir: &Path,
    tx: mpsc::Sender<DownloadEvent>,
) -> Result<()> {
    // Create directory
    fs::create_dir_all(model_dir).context("创建模型目录失败")?;

    let files = paraformer_model_files();
    let total_files = files.len() as u32;

    for (index, file_name) in files.iter().enumerate() {
        let file_path = model_dir.join(file_name);
        let url = paraformer_model_download_url(file_name);

        let _ = tx.send(DownloadEvent::Log(format!(
            "[{}/{}] 准备下载: {}",
            index + 1,
            total_files,
            file_name
        )));

        match download_single_file(&url, &file_path, tx.clone()) {
            Ok(SingleFileDownloadOutcome::Downloaded) => {
                let _ = tx.send(DownloadEvent::Log(format!(
                    "[done] 下载完成: {}",
                    file_name
                )));
            }
            Ok(SingleFileDownloadOutcome::Skipped) => {}
            Err(err) => {
                cleanup_empty_file(&file_path, &tx);
                cleanup_empty_file(&paraformer_tmp_path(&file_path), &tx);
                let _ = tx.send(DownloadEvent::Log(format!(
                    "[error] 下载失败: {} - {}",
                    file_name, err
                )));
                return Err(err);
            }
        }
    }

    let _ = tx.send(DownloadEvent::Finished);
    Ok(())
}

fn download_single_file(
    url: &str,
    dest_path: &Path,
    tx: mpsc::Sender<DownloadEvent>,
) -> Result<SingleFileDownloadOutcome> {
    let mut client = build_http_client(false)?;
    let mut using_direct_client = false;

    // Probe for file size
    let mut total_size =
        probe_single_file_remote_size(&mut client, &mut using_direct_client, url, &tx);

    let tmp_path = paraformer_tmp_path(dest_path);
    if dest_path.exists() {
        let existing_size = fs::metadata(dest_path)
            .with_context(|| format!("读取目标文件元数据失败: {}", dest_path.display()))?
            .len();
        if should_reuse_existing_download(existing_size, total_size) {
            let _ = tx.send(DownloadEvent::Log(format!(
                "[skip] 文件已完整存在: {} ({})",
                dest_path.display(),
                format_bytes(existing_size)
            )));
            return Ok(SingleFileDownloadOutcome::Skipped);
        }

        let reason = if existing_size == 0 {
            "文件为空"
        } else if total_size.is_some_and(|total| existing_size != total) {
            "文件大小与远端不一致"
        } else {
            "无法确认远端大小"
        };
        let _ = tx.send(DownloadEvent::Log(format!(
            "[clean] 删除不完整目标文件: {} ({}, 当前大小 {})",
            dest_path.display(),
            reason,
            format_bytes(existing_size)
        )));
        fs::remove_file(dest_path)
            .with_context(|| format!("删除不完整目标文件失败: {}", dest_path.display()))?;
    }

    let mut resume_size = fs::metadata(&tmp_path).map(|m| m.len()).unwrap_or(0);
    if resume_size > 0 && should_reuse_existing_download(resume_size, total_size) {
        fs::rename(&tmp_path, dest_path).with_context(|| {
            format!(
                "恢复已完成的临时文件失败: {} -> {}",
                tmp_path.display(),
                dest_path.display()
            )
        })?;
        let _ = tx.send(DownloadEvent::Log(format!(
            "[save] 复用已完成临时文件: {}",
            dest_path.display()
        )));
        return Ok(SingleFileDownloadOutcome::Downloaded);
    }
    if resume_size == 0 && tmp_path.exists() {
        let _ = tx.send(DownloadEvent::Log(format!(
            "[clean] 删除空临时文件: {}",
            tmp_path.display()
        )));
        let _ = fs::remove_file(&tmp_path);
    } else if total_size.is_some_and(|total| resume_size > total) {
        let _ = tx.send(DownloadEvent::Log(format!(
            "[clean] 删除异常临时文件: {} (当前大小 {} 超过远端大小 {})",
            tmp_path.display(),
            format_bytes(resume_size),
            format_bytes(total_size.unwrap_or(0))
        )));
        fs::remove_file(&tmp_path)
            .with_context(|| format!("删除异常临时文件失败: {}", tmp_path.display()))?;
        resume_size = 0;
    } else if resume_size > 0 {
        let _ = tx.send(DownloadEvent::Log(format!(
            "[resume] 继续下载临时文件: {} ({})",
            tmp_path.display(),
            format_bytes(resume_size)
        )));
    }

    let mut writer = OpenOptions::new()
        .create(true)
        .write(true)
        .append(resume_size > 0)
        .truncate(resume_size == 0)
        .open(&tmp_path)
        .with_context(|| format!("打开临时文件失败: {}", tmp_path.display()))?;

    let mut downloaded: u64 = resume_size;
    let mut buf = [0u8; 64 * 1024];
    let mut last_emit = Instant::now();

    loop {
        if total_size.is_some_and(|t| downloaded >= t) {
            break;
        }

        let chunk_end = downloaded
            .saturating_add(DOWNLOAD_CHUNK_SIZE)
            .saturating_sub(1);
        let range_end = total_size
            .map(|t| chunk_end.min(t.saturating_sub(1)))
            .unwrap_or(chunk_end);
        let range_text = format!("bytes={}-{}", downloaded, range_end);

        let mut response = match client.get(url).header(RANGE, range_text.clone()).send() {
            Ok(resp) => resp,
            Err(err) => {
                if !using_direct_client && is_likely_proxy_error(&err) {
                    client = build_http_client(true)?;
                    using_direct_client = true;
                    continue;
                }
                return Err(anyhow!("下载请求失败: {} ({})", err, url));
            }
        };

        let status = response.status();
        if !status.is_success() && status != StatusCode::PARTIAL_CONTENT {
            return Err(anyhow!("HTTP 错误: {}", status));
        }

        if total_size.is_none() {
            total_size = response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(parse_total_from_content_range)
                .or_else(|| normalize_content_length(response.content_length()));
        }

        loop {
            let n = match response.read(&mut buf) {
                Ok(n) => n,
                Err(err) => return Err(anyhow!("读取失败: {} ({})", err, url)),
            };
            if n == 0 {
                break;
            }
            writer.write_all(&buf[..n])?;
            downloaded += n as u64;
        }
        writer.flush()?;

        if last_emit.elapsed() >= Duration::from_millis(120) {
            let _ = tx.send(DownloadEvent::Progress {
                downloaded,
                total: total_size,
            });
            last_emit = Instant::now();
        }

        if total_size.is_some_and(|t| downloaded >= t) {
            break;
        }
    }

    if downloaded == 0 {
        return Err(anyhow!("下载失败：未接收到任何数据"));
    }

    writer
        .flush()
        .with_context(|| format!("刷新临时文件失败: {}", tmp_path.display()))?;
    fs::rename(&tmp_path, dest_path).with_context(|| {
        format!(
            "保存文件失败: {} -> {}",
            tmp_path.display(),
            dest_path.display()
        )
    })?;
    let _ = tx.send(DownloadEvent::Progress {
        downloaded,
        total: total_size.or(Some(downloaded)),
    });
    let _ = tx.send(DownloadEvent::Log(format!(
        "[save] 文件已保存: {} ({})",
        dest_path.display(),
        format_bytes(downloaded)
    )));

    Ok(SingleFileDownloadOutcome::Downloaded)
}

pub fn parse_total_from_content_range(content_range: &str) -> Option<u64> {
    // 形如: bytes 0-1023/2048
    content_range.rsplit('/').next()?.parse::<u64>().ok()
}

pub fn download_ratio_label(download: &DownloadState) -> (f64, String) {
    match download.total {
        Some(total) if total > 0 => {
            let ratio = (download.downloaded as f64 / total as f64).clamp(0.0, 1.0);
            let label = format!(
                "{} / {} ({:.1}%)",
                format_bytes(download.downloaded),
                format_bytes(total),
                ratio * 100.0
            );
            (ratio, label)
        }
        _ => {
            let ratio = if download.in_progress { 0.0 } else { 1.0 };
            let label = format!("已下载 {}", format_bytes(download.downloaded));
            (ratio, label)
        }
    }
}

pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;

    let b = bytes as f64;
    if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= MB {
        format!("{:.2} MB", b / MB)
    } else if b >= KB {
        format!("{:.2} KB", b / KB)
    } else {
        format!("{} B", bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_name_resolution() {
        assert_eq!(
            resolve_model_file_name("large-v3"),
            Some("ggml-large-v3.bin")
        );
        assert_eq!(
            resolve_model_file_name("turbo"),
            Some("ggml-large-v3-turbo.bin")
        );
        assert_eq!(resolve_model_file_name("medium"), Some("ggml-medium.bin"));
        assert_eq!(resolve_model_file_name("unknown"), None);
        assert_eq!(
            model_download_url("ggml-large-v3.bin"),
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3.bin"
        );
    }

    #[test]
    fn test_parse_total_from_content_range() {
        assert_eq!(
            parse_total_from_content_range("bytes 0-1023/2048"),
            Some(2048)
        );
        assert_eq!(parse_total_from_content_range("bytes */2048"), Some(2048));
        assert_eq!(parse_total_from_content_range("invalid"), None);
    }

    #[test]
    fn test_download_ratio_label_and_format_bytes() {
        let with_total = DownloadState {
            model_size: "large-v3".to_string(),
            model_file_name: "ggml-large-v3.bin".to_string(),
            downloaded: 1024,
            total: Some(2048),
            in_progress: true,
        };
        let (ratio, label) = download_ratio_label(&with_total);
        assert!((ratio - 0.5).abs() < 1e-9);
        assert!(label.contains("50.0%"));

        let without_total = DownloadState {
            model_size: "large-v3".to_string(),
            model_file_name: "ggml-large-v3.bin".to_string(),
            downloaded: 2048,
            total: None,
            in_progress: false,
        };
        let (ratio2, label2) = download_ratio_label(&without_total);
        assert_eq!(ratio2, 1.0);
        assert!(label2.contains("已下载"));

        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(2048), "2.00 KB");
    }

    #[test]
    fn test_normalize_content_length() {
        assert_eq!(normalize_content_length(None), None);
        assert_eq!(normalize_content_length(Some(0)), None);
        assert_eq!(normalize_content_length(Some(1)), Some(1));
    }

    #[test]
    fn test_should_try_parallel_download() {
        assert!(should_try_parallel_download(
            Some(DOWNLOAD_PARALLEL_CHUNK_SIZE * 2),
            0
        ));
        assert!(!should_try_parallel_download(
            Some(DOWNLOAD_PARALLEL_CHUNK_SIZE),
            0
        ));
        assert!(!should_try_parallel_download(
            Some(DOWNLOAD_PARALLEL_CHUNK_SIZE * 2),
            1024
        ));
        assert!(!should_try_parallel_download(None, 0));
    }

    #[test]
    fn test_build_parallel_download_plan() {
        let parts_dir = std::env::temp_dir().join("echopup-model-download-plan-test");
        let plans = build_parallel_download_plan(10, 4, &parts_dir);
        assert_eq!(plans.len(), 3);
        assert_eq!((plans[0].start, plans[0].end), (0, 3));
        assert_eq!((plans[1].start, plans[1].end), (4, 7));
        assert_eq!((plans[2].start, plans[2].end), (8, 9));
        assert!(plans[0].path.ends_with("part-0000"));
        assert!(plans[2].path.ends_with("part-0002"));
    }

    #[test]
    fn test_paraformer_tmp_path_and_reuse_check() {
        let dest = Path::new("/tmp/paraformer/encoder.onnx");
        assert_eq!(
            paraformer_tmp_path(dest),
            PathBuf::from("/tmp/paraformer/encoder.onnx.part")
        );
        assert!(should_reuse_existing_download(1024, Some(1024)));
        assert!(!should_reuse_existing_download(0, Some(1024)));
        assert!(!should_reuse_existing_download(512, Some(1024)));
        assert!(!should_reuse_existing_download(1024, None));
    }
}
