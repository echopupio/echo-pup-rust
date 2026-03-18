//! 终端管理界面（TUI）

use crate::config::Config;
use crate::runtime;
use anyhow::{anyhow, Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Terminal;
use reqwest::header::{CONTENT_RANGE, RANGE};
use reqwest::StatusCode;
use std::fs::{self, OpenOptions};
use std::io::{Read, Stdout, Write};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

const MENU_ITEMS: [&str; 15] = [
    "切换 LLM 开关",
    "切换文本纠错开关",
    "切换 VAD 开关",
    "编辑热键",
    "编辑 LLM provider",
    "编辑 LLM model",
    "编辑 LLM api_base",
    "编辑 LLM api_key_env",
    "编辑 Whisper model_path",
    "下载模型 large-v3",
    "下载模型 turbo",
    "下载模型 medium",
    "刷新本地模型列表",
    "保存配置",
    "退出 UI",
];
const DOWNLOAD_LOG_MAX_LINES: usize = 120;
const DOWNLOAD_CHUNK_SIZE: u64 = 4 * 1024 * 1024;
const DOWNLOAD_NO_PROGRESS_TIMEOUT_SECS: u64 = 45;
const DOWNLOAD_MAX_RETRIES: usize = 6;

#[derive(Clone, Copy)]
enum InputTarget {
    Hotkey,
    LlmProvider,
    LlmModel,
    LlmApiBase,
    LlmApiKeyEnv,
    WhisperModelPath,
}

struct DownloadState {
    model_size: String,
    model_file_name: String,
    downloaded: u64,
    total: Option<u64>,
    in_progress: bool,
}

enum DownloadEvent {
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

struct AppState {
    config: Config,
    config_path: String,
    selected: usize,
    status: String,
    local_models: Vec<String>,
    input_target: Option<InputTarget>,
    input_buffer: String,
    download_logs: Vec<String>,
    download: Option<DownloadState>,
    download_rx: Option<Receiver<DownloadEvent>>,
    should_quit: bool,
}

impl AppState {
    fn new(config_path: &str) -> Result<Self> {
        let config = Config::load(config_path)?;
        let local_models = list_local_models();
        Ok(Self {
            config,
            config_path: config_path.to_string(),
            selected: 0,
            status: "就绪。方向键选择，Enter执行，q退出。".to_string(),
            local_models,
            input_target: None,
            input_buffer: String::new(),
            download_logs: Vec::new(),
            download: None,
            download_rx: None,
            should_quit: false,
        })
    }

    fn selected_label(&self) -> &str {
        MENU_ITEMS[self.selected]
    }
}

fn append_download_log(app: &mut AppState, line: impl Into<String>) {
    app.download_logs.push(line.into());
    if app.download_logs.len() > DOWNLOAD_LOG_MAX_LINES {
        let drain_len = app.download_logs.len() - DOWNLOAD_LOG_MAX_LINES;
        app.download_logs.drain(0..drain_len);
    }
}

pub fn run_ui(config_path: &str) -> Result<()> {
    let mut app = AppState::new(config_path)?;
    let mut stdout = std::io::stdout();
    enable_raw_mode().context("启用 raw mode 失败")?;
    stdout
        .execute(EnterAlternateScreen)
        .context("进入备用屏幕失败")?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("初始化 TUI 终端失败")?;

    let run_result = run_app(&mut terminal, &mut app);

    disable_raw_mode().ok();
    terminal.backend_mut().execute(LeaveAlternateScreen).ok();
    terminal.show_cursor().ok();

    run_result
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut AppState) -> Result<()> {
    while !app.should_quit {
        drain_download_events(app);

        terminal
            .draw(|f| draw_ui(f, app))
            .context("绘制 TUI 失败")?;

        if event::poll(Duration::from_millis(200)).context("读取事件失败")? {
            if let Event::Key(key) = event::read().context("读取按键失败")? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                if app.input_target.is_some() {
                    handle_input_mode_key(app, key.code)?;
                } else {
                    handle_menu_mode_key(app, key.code)?;
                }
            }
        }
    }

    Ok(())
}

fn draw_ui(frame: &mut ratatui::Frame, app: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(6),
        ])
        .split(frame.area());

    let title = Paragraph::new("EchoPup 管理界面 (echopup ui)")
        .block(Block::default().borders(Borders::ALL).title("TUI"));
    frame.render_widget(title, chunks[0]);

    let body_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(chunks[1]);

    let items: Vec<ListItem> = MENU_ITEMS
        .iter()
        .map(|item| ListItem::new((*item).to_string()))
        .collect();
    let menu = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("菜单"))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol(">> ");
    let mut state = ListState::default();
    state.select(Some(app.selected));
    frame.render_stateful_widget(menu, body_chunks[0], &mut state);

    let models = if app.local_models.is_empty() {
        "（未发现本地模型）".to_string()
    } else {
        app.local_models.join("\n")
    };
    let download_logs = if app.download_logs.is_empty() {
        "（暂无下载日志）".to_string()
    } else {
        app.download_logs
            .iter()
            .rev()
            .take(10)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    };

    let summary = format!(
        "当前配置\n\nhotkey: {}\nllm.enabled: {}\ntext_correction.enabled: {}\naudio.vad_enabled: {}\nllm.provider: {}\nllm.model: {}\nllm.api_base: {}\nllm.api_key_env: {}\nwhisper.model_path: {}\n\n本地模型:\n{}\n\n下载日志:\n{}",
        app.config.hotkey.key,
        app.config.llm.enabled,
        app.config.text_correction.enabled,
        app.config.audio.vad_enabled,
        app.config.llm.provider,
        app.config.llm.model,
        app.config.llm.api_base,
        app.config.llm.api_key_env,
        app.config.whisper.model_path,
        models,
        download_logs
    );
    let detail = Paragraph::new(summary)
        .block(Block::default().borders(Borders::ALL).title("详情"))
        .wrap(Wrap { trim: true });
    frame.render_widget(detail, body_chunks[1]);

    let bottom_text = if let Some(target) = app.input_target {
        format!(
            "输入模式（{}）: {}  [Enter保存 / Esc取消]",
            input_target_label(target),
            app.input_buffer
        )
    } else {
        format!("状态: {}\n当前选中: {}", app.status, app.selected_label())
    };

    if let Some(download) = app.download.as_ref() {
        let bottom_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(3)])
            .split(chunks[2]);

        let bottom =
            Paragraph::new(bottom_text).block(Block::default().borders(Borders::ALL).title("状态"));
        frame.render_widget(bottom, bottom_chunks[0]);

        let (ratio, label) = download_ratio_label(download);
        let gauge_title = if download.in_progress {
            format!(
                "下载进度 [{} -> {}]",
                download.model_size, download.model_file_name
            )
        } else {
            format!("下载结果 [{}]", download.model_size)
        };

        let gauge = Gauge::default()
            .block(Block::default().borders(Borders::ALL).title(gauge_title))
            .ratio(ratio)
            .label(label);
        frame.render_widget(gauge, bottom_chunks[1]);
    } else {
        let bottom =
            Paragraph::new(bottom_text).block(Block::default().borders(Borders::ALL).title("状态"));
        frame.render_widget(bottom, chunks[2]);
    }
}

fn handle_menu_mode_key(app: &mut AppState, code: KeyCode) -> Result<()> {
    match code {
        KeyCode::Char('q') => app.should_quit = true,
        KeyCode::Up => {
            if app.selected == 0 {
                app.selected = MENU_ITEMS.len() - 1;
            } else {
                app.selected -= 1;
            }
        }
        KeyCode::Down => {
            app.selected = (app.selected + 1) % MENU_ITEMS.len();
        }
        KeyCode::Enter => execute_menu_action(app)?,
        _ => {}
    }
    Ok(())
}

fn handle_input_mode_key(app: &mut AppState, code: KeyCode) -> Result<()> {
    match code {
        KeyCode::Esc => {
            app.input_target = None;
            app.input_buffer.clear();
            app.status = "已取消编辑".to_string();
        }
        KeyCode::Enter => {
            apply_input(app)?;
        }
        KeyCode::Backspace => {
            app.input_buffer.pop();
        }
        KeyCode::Char(c) => {
            app.input_buffer.push(c);
        }
        _ => {}
    }
    Ok(())
}

fn execute_menu_action(app: &mut AppState) -> Result<()> {
    match app.selected {
        0 => {
            app.config.llm.enabled = !app.config.llm.enabled;
            app.status = format!("LLM 开关 => {}", app.config.llm.enabled);
        }
        1 => {
            app.config.text_correction.enabled = !app.config.text_correction.enabled;
            app.status = format!("文本纠错开关 => {}", app.config.text_correction.enabled);
        }
        2 => {
            app.config.audio.vad_enabled = !app.config.audio.vad_enabled;
            app.status = format!("VAD 开关 => {}", app.config.audio.vad_enabled);
        }
        3 => start_input(app, InputTarget::Hotkey, app.config.hotkey.key.clone()),
        4 => start_input(
            app,
            InputTarget::LlmProvider,
            app.config.llm.provider.clone(),
        ),
        5 => start_input(app, InputTarget::LlmModel, app.config.llm.model.clone()),
        6 => start_input(
            app,
            InputTarget::LlmApiBase,
            app.config.llm.api_base.clone(),
        ),
        7 => start_input(
            app,
            InputTarget::LlmApiKeyEnv,
            app.config.llm.api_key_env.clone(),
        ),
        8 => start_input(
            app,
            InputTarget::WhisperModelPath,
            app.config.whisper.model_path.clone(),
        ),
        9 => start_model_download(app, "large-v3")?,
        10 => start_model_download(app, "turbo")?,
        11 => start_model_download(app, "medium")?,
        12 => {
            app.local_models = list_local_models();
            app.status = "已刷新本地模型列表".to_string();
        }
        13 => {
            app.config.save(&app.config_path)?;
            app.status = format!("配置已保存: {}", app.config_path);
        }
        14 => app.should_quit = true,
        _ => {}
    }
    Ok(())
}

fn start_input(app: &mut AppState, target: InputTarget, current_value: String) {
    app.input_target = Some(target);
    app.input_buffer = current_value;
    app.status = format!("编辑 {}", input_target_label(target));
}

fn apply_input(app: &mut AppState) -> Result<()> {
    let target = app
        .input_target
        .ok_or_else(|| anyhow!("当前不在输入模式"))?;
    let value = app.input_buffer.trim().to_string();

    if value.is_empty() {
        app.status = "输入不能为空".to_string();
        return Ok(());
    }

    match target {
        InputTarget::Hotkey => app.config.hotkey.key = value,
        InputTarget::LlmProvider => app.config.llm.provider = value,
        InputTarget::LlmModel => app.config.llm.model = value,
        InputTarget::LlmApiBase => app.config.llm.api_base = value,
        InputTarget::LlmApiKeyEnv => app.config.llm.api_key_env = value,
        InputTarget::WhisperModelPath => app.config.whisper.model_path = value,
    }

    app.input_target = None;
    app.input_buffer.clear();
    app.status = "编辑已应用（记得保存配置）".to_string();
    Ok(())
}

fn input_target_label(target: InputTarget) -> &'static str {
    match target {
        InputTarget::Hotkey => "hotkey",
        InputTarget::LlmProvider => "llm.provider",
        InputTarget::LlmModel => "llm.model",
        InputTarget::LlmApiBase => "llm.api_base",
        InputTarget::LlmApiKeyEnv => "llm.api_key_env",
        InputTarget::WhisperModelPath => "whisper.model_path",
    }
}

fn list_local_models() -> Vec<String> {
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

fn start_model_download(app: &mut AppState, model_size: &str) -> Result<()> {
    if app
        .download
        .as_ref()
        .map(|d| d.in_progress)
        .unwrap_or(false)
    {
        app.status = "已有下载任务进行中，请等待完成".to_string();
        return Ok(());
    }

    let model_file_name = resolve_model_file_name(model_size)
        .ok_or_else(|| anyhow!("不支持的模型大小: {}", model_size))?
        .to_string();

    let model_size_owned = model_size.to_string();
    let model_file_name_for_thread = model_file_name.clone();
    let model_url = model_download_url(&model_file_name);
    let model_dir = runtime::model_dir()?;
    let (tx, rx) = mpsc::channel();

    app.download = Some(DownloadState {
        model_size: model_size_owned.clone(),
        model_file_name,
        downloaded: 0,
        total: None,
        in_progress: true,
    });
    app.download_logs.clear();
    append_download_log(app, format!("[start] 准备下载模型 {}", model_size_owned));
    append_download_log(
        app,
        format!(
            "[equiv] curl -fL -C - -o \"{}/{}.part\" \"{}\"",
            model_dir.display(),
            model_file_name_for_thread,
            model_url
        ),
    );
    app.download_rx = Some(rx);
    app.status = format!("正在下载模型 {} ...", model_size_owned);

    std::thread::spawn(move || {
        if let Err(err) = download_model_with_progress(
            model_size_owned.clone(),
            model_file_name_for_thread,
            tx.clone(),
        ) {
            let _ = tx.send(DownloadEvent::Failed(err.to_string()));
        }
    });

    Ok(())
}

fn resolve_model_file_name(model_size: &str) -> Option<&'static str> {
    match model_size {
        "large" | "large-v3" => Some("ggml-large-v3.bin"),
        "turbo" | "large-v3-turbo" => Some("ggml-large-v3-turbo.bin"),
        "medium" => Some("ggml-medium.bin"),
        _ => None,
    }
}

fn model_download_url(model_file_name: &str) -> String {
    format!(
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
        model_file_name
    )
}

fn drain_download_events(app: &mut AppState) {
    let mut clear_rx = false;

    loop {
        let recv_result = {
            let Some(rx) = app.download_rx.as_ref() else {
                break;
            };
            rx.try_recv()
        };

        match recv_result {
            Ok(event) => match event {
                DownloadEvent::Started {
                    model_size,
                    model_file_name,
                    downloaded,
                    total,
                } => {
                    app.download = Some(DownloadState {
                        model_size: model_size.clone(),
                        model_file_name,
                        downloaded,
                        total,
                        in_progress: true,
                    });
                    app.status = format!("正在下载模型 {} ...", model_size);
                    append_download_log(
                        app,
                        format!(
                            "[started] 已下载 {}，总大小 {}",
                            format_bytes(downloaded),
                            total
                                .map(format_bytes)
                                .unwrap_or_else(|| "未知".to_string())
                        ),
                    );
                }
                DownloadEvent::Progress { downloaded, total } => {
                    if let Some(download) = app.download.as_mut() {
                        download.downloaded = downloaded;
                        if total.is_some() {
                            download.total = total;
                        }
                    }
                }
                DownloadEvent::Finished => {
                    let mut model_size = "unknown".to_string();
                    if let Some(download) = app.download.as_mut() {
                        model_size = download.model_size.clone();
                        download.in_progress = false;
                        if download.total.is_none() {
                            download.total = Some(download.downloaded);
                        }
                    }
                    app.local_models = list_local_models();
                    app.status = format!("模型 {} 下载完成", model_size);
                    append_download_log(app, "[finished] 下载完成".to_string());
                    clear_rx = true;
                }
                DownloadEvent::Failed(err) => {
                    if let Some(download) = app.download.as_mut() {
                        download.in_progress = false;
                    }
                    app.status = format!("下载失败: {}", err);
                    append_download_log(app, format!("[error] {}", err));
                    clear_rx = true;
                }
                DownloadEvent::Log(line) => {
                    append_download_log(app, line);
                }
            },
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                if let Some(download) = app.download.as_mut() {
                    download.in_progress = false;
                }
                clear_rx = true;
                break;
            }
        }
    }

    if clear_rx {
        app.download_rx = None;
    }
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

fn parse_total_from_content_range(content_range: &str) -> Option<u64> {
    // 形如: bytes 0-1023/2048
    content_range.rsplit('/').next()?.parse::<u64>().ok()
}

fn download_ratio_label(download: &DownloadState) -> (f64, String) {
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

fn format_bytes(bytes: u64) -> String {
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
    use crossterm::event::KeyCode;
    use std::sync::mpsc;

    fn make_app() -> AppState {
        AppState {
            config: Config::default(),
            config_path: "/tmp/echopup-ui-test.toml".to_string(),
            selected: 0,
            status: String::new(),
            local_models: vec![],
            input_target: None,
            input_buffer: String::new(),
            download_logs: vec![],
            download: None,
            download_rx: None,
            should_quit: false,
        }
    }

    #[test]
    fn test_menu_navigation_wrap_and_quit() {
        let mut app = make_app();

        handle_menu_mode_key(&mut app, KeyCode::Up).unwrap();
        assert_eq!(app.selected, MENU_ITEMS.len() - 1);

        handle_menu_mode_key(&mut app, KeyCode::Down).unwrap();
        assert_eq!(app.selected, 0);

        handle_menu_mode_key(&mut app, KeyCode::Char('q')).unwrap();
        assert!(app.should_quit);
    }

    #[test]
    fn test_apply_input_to_llm_model() {
        let mut app = make_app();
        app.input_target = Some(InputTarget::LlmModel);
        app.input_buffer = "  gpt-test-model  ".to_string();

        apply_input(&mut app).unwrap();

        assert_eq!(app.config.llm.model, "gpt-test-model");
        assert!(app.input_target.is_none());
        assert!(app.input_buffer.is_empty());
    }

    #[test]
    fn test_apply_input_rejects_empty_value() {
        let mut app = make_app();
        app.input_target = Some(InputTarget::LlmProvider);
        app.input_buffer = "   ".to_string();

        apply_input(&mut app).unwrap();

        assert_eq!(app.config.llm.provider, "openai");
        assert!(app.input_target.is_some());
        assert_eq!(app.status, "输入不能为空");
    }

    #[test]
    fn test_execute_menu_toggle_switches() {
        let mut app = make_app();

        app.selected = 0;
        execute_menu_action(&mut app).unwrap();
        assert!(app.config.llm.enabled);

        app.selected = 1;
        execute_menu_action(&mut app).unwrap();
        assert!(!app.config.text_correction.enabled);

        app.selected = 2;
        execute_menu_action(&mut app).unwrap();
        assert!(app.config.audio.vad_enabled);
    }

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
    fn test_drain_download_events_finish_path() {
        let mut app = make_app();
        let (tx, rx) = mpsc::channel();
        app.download_rx = Some(rx);

        tx.send(DownloadEvent::Started {
            model_size: "large-v3".to_string(),
            model_file_name: "ggml-large-v3.bin".to_string(),
            downloaded: 0,
            total: Some(100),
        })
        .unwrap();
        tx.send(DownloadEvent::Progress {
            downloaded: 100,
            total: Some(100),
        })
        .unwrap();
        tx.send(DownloadEvent::Finished).unwrap();

        drain_download_events(&mut app);

        assert!(app.download_rx.is_none());
        let download = app.download.as_ref().unwrap();
        assert!(!download.in_progress);
        assert_eq!(download.downloaded, 100);
        assert!(app.status.contains("下载完成"));
    }

    #[test]
    fn test_drain_download_events_failed_path() {
        let mut app = make_app();
        app.download = Some(DownloadState {
            model_size: "medium".to_string(),
            model_file_name: "ggml-medium.bin".to_string(),
            downloaded: 12,
            total: Some(100),
            in_progress: true,
        });

        let (tx, rx) = mpsc::channel();
        app.download_rx = Some(rx);
        tx.send(DownloadEvent::Failed("network error".to_string()))
            .unwrap();

        drain_download_events(&mut app);

        assert!(app.download_rx.is_none());
        assert_eq!(app.download.as_ref().unwrap().in_progress, false);
        assert!(app.status.contains("下载失败"));
    }

    #[test]
    fn test_drain_download_events_collect_logs() {
        let mut app = make_app();
        let (tx, rx) = mpsc::channel();
        app.download_rx = Some(rx);
        tx.send(DownloadEvent::Log("line-1".to_string())).unwrap();
        tx.send(DownloadEvent::Log("line-2".to_string())).unwrap();

        drain_download_events(&mut app);

        assert_eq!(app.download_logs.len(), 2);
        assert_eq!(app.download_logs[0], "line-1");
        assert_eq!(app.download_logs[1], "line-2");
    }
}
