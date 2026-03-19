//! 共享模型下载能力（TUI / 状态栏菜单复用）

use anyhow::{anyhow, Context, Result};
use reqwest::header::{CONTENT_RANGE, RANGE};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use crate::runtime;

pub const DOWNLOAD_LOG_MAX_LINES: usize = 120;
pub const DOWNLOAD_CHUNK_SIZE: u64 = 4 * 1024 * 1024;
pub const DOWNLOAD_NO_PROGRESS_TIMEOUT_SECS: u64 = 45;
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
        if let Err(err) = download_model_with_progress(
            model_size_owned.clone(),
            model_file_name_for_thread,
            tx.clone(),
        ) {
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

    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .timeout(None)
        .build()
        .context("初始化 HTTP 客户端失败")?;

    let mut total_size = None;
    match client.head(&model_url).send() {
        Ok(resp) => {
            let status = resp.status();
            let content_length = if status.is_success() {
                resp.content_length()
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
            let _ = tx.send(DownloadEvent::Log(format!("[head] 请求失败: {}", err)));
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

        let mut attempt = 0usize;
        let chunk_written: u64 = loop {
            attempt += 1;
            let _ = tx.send(DownloadEvent::Log(format!(
                "[get] range={} attempt={}/{} timeout={}s",
                range_text, attempt, DOWNLOAD_MAX_RETRIES, DOWNLOAD_NO_PROGRESS_TIMEOUT_SECS
            )));

            let mut response = match client
                .get(&model_url)
                .header(RANGE, range_text.clone())
                .timeout(Duration::from_secs(DOWNLOAD_NO_PROGRESS_TIMEOUT_SECS))
                .send()
            {
                Ok(resp) => resp,
                Err(err) => {
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
                        response.content_length().map(|len| {
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
                let n = response
                    .read(&mut buf)
                    .context("读取下载流失败（网络可能中断，可重新下载自动续传）")?;
                if n == 0 {
                    break;
                }
                writer
                    .write_all(&buf[..n])
                    .context("写入模型临时文件失败")?;
                downloaded += n as u64;
                written += n as u64;

                if last_emit.elapsed() >= Duration::from_millis(120) {
                    let _ = tx.send(DownloadEvent::Progress {
                        downloaded,
                        total: total_size,
                    });
                    last_emit = Instant::now();
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
}
