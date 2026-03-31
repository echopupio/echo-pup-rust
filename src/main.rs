//! EchoPup - AI Voice Dictation Tool

mod asr;
mod audio;
mod commit;
mod config;
mod hotkey;
mod input;
mod llm;
mod menu_core;
mod model_download;
mod runtime;
mod session;
mod status_indicator;
mod stt;
mod ui;
mod vad;

use crate::commit::TextCommitBackend as _;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use parking_lot::Mutex;
use std::io::{IsTerminal, Write};
#[cfg(all(unix, not(target_os = "macos")))]
use std::os::fd::{AsRawFd, RawFd};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{error, info, warn};

#[cfg(target_os = "macos")]
const MAC_OSASCRIPT_PATH: &str = "/usr/bin/osascript";
#[cfg(target_os = "macos")]
const MAC_AFPLAY_PATH: &str = "/usr/bin/afplay";
#[cfg(target_os = "macos")]
const MAC_SOUND_RECORDING_START: &str = "/System/Library/Sounds/Tink.aiff";
#[cfg(target_os = "macos")]
const MAC_SOUND_RECORDING_END: &str = "/System/Library/Sounds/Pop.aiff";

#[derive(Parser)]
#[command(name = "echopup")]
#[command(about = "AI Voice Dictation Tool", long_about = None)]
struct Cli {
    #[arg(short, long, default_value = "~/.echopup/config.toml")]
    config: String,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    Run,
    Start,
    Stop,
    Status,
    Restart,
    Ui {
        #[command(subcommand)]
        command: Option<UiCommands>,
    },
    Test,
    Config {
        show: bool,
        init: bool,
    },
    DownloadModel {
        size: String,
    },
    #[command(hide = true)]
    StatusIndicator,
}

#[derive(Subcommand)]
enum UiCommands {
    Start,
    Stop,
    Status,
    Restart,
}

#[cfg(all(unix, not(target_os = "macos")))]
struct TerminalEchoGuard {
    fd: RawFd,
    original: Option<libc::termios>,
}

#[cfg(all(unix, not(target_os = "macos")))]
impl TerminalEchoGuard {
    fn try_disable_stdin_echo() -> Result<Option<Self>> {
        let fd = std::io::stdin().as_raw_fd();
        let is_tty = unsafe { libc::isatty(fd) };
        if is_tty != 1 {
            return Ok(None);
        }

        let mut current = std::mem::MaybeUninit::<libc::termios>::uninit();
        let read_res = unsafe { libc::tcgetattr(fd, current.as_mut_ptr()) };
        if read_res != 0 {
            return Err(anyhow::anyhow!(
                "读取终端属性失败: {}",
                std::io::Error::last_os_error()
            ));
        }
        let current = unsafe { current.assume_init() };

        let mut no_echo = current;
        no_echo.c_lflag &= !libc::ECHO;
        let set_res = unsafe { libc::tcsetattr(fd, libc::TCSANOW, &no_echo) };
        if set_res != 0 {
            return Err(anyhow::anyhow!(
                "关闭终端输入回显失败: {}",
                std::io::Error::last_os_error()
            ));
        }

        info!("终端输入回显已关闭（TTY）");
        Ok(Some(Self {
            fd,
            original: Some(current),
        }))
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
impl Drop for TerminalEchoGuard {
    fn drop(&mut self) {
        if let Some(original) = self.original.as_ref() {
            let restore_res = unsafe { libc::tcsetattr(self.fd, libc::TCSANOW, original) };
            if restore_res != 0 {
                warn!("恢复终端输入回显失败: {}", std::io::Error::last_os_error());
            } else {
                info!("终端输入回显已恢复");
            }
        }
    }
}

fn clear_terminal_artifacts_if_tty() {
    if !std::io::stdout().is_terminal() {
        return;
    }
    // 尽量清掉功能键在前台终端里留下的转义序列/乱码
    print!("\r\x1b[2K");
    let _ = std::io::Write::flush(&mut std::io::stdout());
}

fn set_status_indicator_state(
    indicator: &Arc<Mutex<status_indicator::StatusIndicatorClient>>,
    state: status_indicator::IndicatorState,
) {
    let mut guard = indicator.lock();
    guard.send(state);
}

fn send_status_snapshot(
    indicator: &Arc<Mutex<status_indicator::StatusIndicatorClient>>,
    snapshot_state: &Arc<Mutex<menu_core::MenuSnapshot>>,
    status: impl Into<String>,
) {
    let snapshot = {
        let mut snapshot = snapshot_state.lock();
        snapshot.status = status.into();
        snapshot.clone()
    };
    let mut guard = indicator.lock();
    guard.send_snapshot(&snapshot);
}

fn whisper_fallback_candidates(file_name: &str) -> Vec<&'static str> {
    match file_name {
        "ggml-medium.bin" => vec![
            "ggml-medium.bin",
            "ggml-large-v3-turbo.bin",
            "ggml-large-v3.bin",
        ],
        "ggml-large-v3-turbo.bin" => vec![
            "ggml-large-v3-turbo.bin",
            "ggml-medium.bin",
            "ggml-large-v3.bin",
        ],
        "ggml-large-v3.bin" => vec![
            "ggml-large-v3.bin",
            "ggml-large-v3-turbo.bin",
            "ggml-medium.bin",
        ],
        _ => Vec::new(),
    }
}

fn resolve_runtime_model_path(model_path: &str) -> Result<String> {
    let requested_path = Path::new(model_path);
    if requested_path.is_file() {
        return Ok(model_path.to_string());
    }

    let requested_name = requested_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("未找到 Whisper 模型: {}", model_path))?;
    let parent_dir = requested_path.parent();
    for candidate in whisper_fallback_candidates(requested_name) {
        if let Some(parent_dir) = parent_dir {
            let candidate_path = parent_dir.join(candidate);
            if candidate_path.is_file() {
                return Ok(candidate_path.to_string_lossy().into_owned());
            }
        }
    }

    if let Ok(model_dir) = runtime::model_dir() {
        for candidate in whisper_fallback_candidates(requested_name) {
            let candidate_path = model_dir.join(candidate);
            if candidate_path.is_file() {
                return Ok(candidate_path.to_string_lossy().into_owned());
            }
        }
    }

    Err(anyhow::anyhow!("未找到 Whisper 模型: {}", model_path))
}

fn log_asr_runtime(label: &str, engine: &dyn asr::AsrEngine) {
    let runtime = engine.runtime_info();
    info!(
        "ASR {}: backend={}, model={}, detail={}, threads={}",
        label,
        runtime.backend.label(),
        runtime.model,
        runtime.detail.unwrap_or_else(|| "n/a".to_string()),
        runtime.threads.unwrap_or_default()
    );
}

fn transcribe_final_with_session(
    asr_engine: &mut dyn asr::AsrEngine,
    audio: &[f32],
) -> Result<String> {
    let session_config = asr::AsrSessionConfig {
        min_partial_samples: audio.len().max(1),
        max_partial_window_samples: audio.len().max(1),
    };

    match asr_engine.start_session(session_config) {
        Ok(mut session) => {
            session.accept_audio(audio)?;
            session.finalize(Arc::new(AtomicBool::new(false)))
        }
        Err(err) => {
            warn!(
                "ASR backend {} 不支持 final session，回退到整段转写: {}",
                asr_engine.backend_kind().label(),
                err
            );
            asr_engine.transcribe(audio)
        }
    }
}

fn detach_thread_handle(handle: &Arc<Mutex<Option<std::thread::JoinHandle<()>>>>) {
    if let Some(join_handle) = handle.lock().take() {
        std::thread::spawn(move || {
            let _ = join_handle.join();
        });
    }
}

fn join_thread_handle(handle: &Arc<Mutex<Option<std::thread::JoinHandle<()>>>>) {
    if let Some(join_handle) = handle.lock().take() {
        let _ = join_handle.join();
    }
}

/// 处理音频数据：转写 -> LLM 整理 -> 谐音纠错 -> 键盘输入
/// is_vad_triggered: 是否由 VAD 自动触发（用于日志区分）
fn process_audio(
    audio_data: &[f32],
    asr_runtime: &Arc<Mutex<Option<Box<dyn asr::AsrEngine>>>>,
    llm: &Arc<Mutex<Option<llm::LLMRewrite>>>,
    post_processor: &Arc<stt::TextPostProcessor>,
    text_commit: &Arc<Mutex<Box<dyn commit::TextCommitBackend>>>,
    recognition_session: &Arc<Mutex<session::RecognitionSession>>,
    is_vad_triggered: bool,
    e2e_start: Instant,
    desktop_notify_enabled: bool,
) -> bool {
    let trigger_type = if is_vad_triggered {
        "VAD自动"
    } else {
        "热键松开"
    };

    if audio_data.is_empty() {
        info!("[{}] 录音数据为空", trigger_type);
        desktop_notify(desktop_notify_enabled, "EchoPup", "未检测到语音输入");
        return false;
    }
    info!("[{}] 录音完成，采样点: {}", trigger_type, audio_data.len());

    let mut llm_ms = 0u128;
    let mut postprocess_ms = 0u128;
    let mut type_ms = 0u128;
    let mut stt_backend = "unknown".to_string();

    // 1. 音频转写
    // 为避免轻音/尾音被误裁剪，这里关闭“转写前二次 VAD 裁剪”
    let processed_audio = audio_data;
    let mut final_text = String::new();
    let mut transcribe_success = false;
    let stt_start = Instant::now();
    let mut stt_model = "uninitialized".to_string();
    let mut stt_detail = "unknown".to_string();
    let mut stt_threads = 0;

    {
        let mut asr_guard = asr_runtime.lock();
        if let Some(ref mut asr_engine) = *asr_guard {
            let runtime = asr_engine.runtime_info();
            stt_backend = runtime.backend.label().to_string();
            stt_model = runtime.model;
            stt_detail = runtime.detail.unwrap_or_else(|| "n/a".to_string());
            stt_threads = runtime.threads.unwrap_or_default();
            match transcribe_final_with_session(asr_engine.as_mut(), processed_audio) {
                Ok(text) => {
                    // 过滤无效结果
                    let trimmed = text.trim();
                    if trimmed.is_empty() || trimmed == "[BLANK_AUDIO]" {
                        info!("转写结果为空或无效（可能没有说话或音量太小）");
                        desktop_notify(desktop_notify_enabled, "EchoPup", "未识别到有效语音");
                        return false;
                    }
                    info!("转写完成: {}", text);
                    final_text = text;
                    transcribe_success = true;
                }
                Err(e) => {
                    error!("转写失败: {}", e);
                }
            }
        } else {
            error!("ASR 运行时未初始化");
        }
    }
    let stt_ms = stt_start.elapsed().as_millis();

    if !transcribe_success {
        desktop_notify(
            desktop_notify_enabled,
            "EchoPup",
            "语音识别失败，请查看日志",
        );
        info!(
            "[{}] 性能埋点: backend={} model={} detail={} threads={} stt_ms={} llm_ms={} postprocess_ms={} type_ms={} e2e_ms={}",
            trigger_type,
            stt_backend,
            stt_model,
            stt_detail,
            stt_threads,
            stt_ms,
            llm_ms,
            postprocess_ms,
            type_ms,
            e2e_start.elapsed().as_millis()
        );
        return false;
    }

    // 2. LLM 整理（如果启用）
    let llm_enabled = {
        let llm_guard = llm.lock();
        llm_guard.as_ref().map(|l| l.is_enabled()).unwrap_or(false)
    };

    if llm_enabled {
        let llm_start = Instant::now();
        let llm_guard = llm.lock();
        if let Some(ref llm) = *llm_guard {
            match llm.rewrite(&final_text) {
                Ok(rewritten) => {
                    info!("LLM 整理完成: {}", rewritten);
                    final_text = rewritten;
                }
                Err(e) => {
                    error!("LLM 整理失败: {}，使用原始转写结果", e);
                }
            }
        }
        llm_ms = llm_start.elapsed().as_millis();
    }

    // 3. 谐音纠错（规则映射）
    let postprocess_start = Instant::now();
    let corrected = post_processor.process(&final_text);
    if corrected != final_text {
        info!("谐音纠错已应用");
        final_text = corrected;
    }
    postprocess_ms = postprocess_start.elapsed().as_millis();

    // 4. 文本提交
    let commit_action = {
        let mut session_guard = recognition_session.lock();
        session_guard.prepare_final_commit(&final_text)
    };
    let Some(commit_action) = commit_action else {
        info!("最终结果为空或与本次会话已提交内容重复，跳过文本提交");
        return false;
    };

    let type_start = Instant::now();
    let mut type_success = false;
    {
        let mut commit_guard = text_commit.lock();
        match commit_guard.apply(commit_action) {
            Ok(_) => {
                info!("文本已提交");
                type_success = true;
                desktop_notify(
                    desktop_notify_enabled,
                    "EchoPup",
                    &format!("识别完成，已输入 {} 字", final_text.chars().count()),
                );
            }
            Err(e) => {
                error!("文本提交失败: {}", e);
                desktop_notify(desktop_notify_enabled, "EchoPup", "识别完成，但输入失败");
            }
        }
    }
    type_ms = type_start.elapsed().as_millis();

    info!(
        "[{}] 性能埋点: backend={} model={} detail={} threads={} stt_ms={} llm_ms={} postprocess_ms={} type_ms={} e2e_ms={}",
        trigger_type,
        stt_backend,
        stt_model,
        stt_detail,
        stt_threads,
        stt_ms,
        llm_ms,
        postprocess_ms,
        type_ms,
        e2e_start.elapsed().as_millis()
    );

    type_success
}

fn desktop_notify(enabled: bool, title: &str, body: &str) {
    if !enabled {
        return;
    }

    if let Err(err) = send_desktop_notify(title, body) {
        warn!("桌面通知发送失败: {}", err);
    }
}

fn detect_desktop_notify_capability() -> (bool, String) {
    #[cfg(target_os = "linux")]
    {
        if !command_exists("notify-send") {
            return (
                false,
                "未找到 notify-send（请安装 libnotify-bin）".to_string(),
            );
        }
        let has_display =
            std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some();
        if !has_display {
            return (
                false,
                "未检测到 DISPLAY/WAYLAND_DISPLAY（需要图形会话）".to_string(),
            );
        }
        return (true, "linux notify-send".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        if !std::path::Path::new(MAC_OSASCRIPT_PATH).is_file() {
            return (false, format!("未找到 osascript: {}", MAC_OSASCRIPT_PATH));
        }
        return (true, format!("macOS osascript ({})", MAC_OSASCRIPT_PATH));
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        (false, "当前系统未实现桌面通知后端".to_string())
    }
}

#[derive(Clone, Copy)]
enum FeedbackSoundEvent {
    RecordingStart,
    RecordingEnd,
}

fn detect_sound_feedback_capability(user_enabled: bool) -> (bool, String) {
    if !user_enabled {
        return (false, "已在配置中关闭 sound_enabled".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        if !std::path::Path::new(MAC_AFPLAY_PATH).is_file() {
            return (
                false,
                format!("未找到 afplay: {}（无法播放提示音）", MAC_AFPLAY_PATH),
            );
        }
        return (true, format!("macOS afplay ({})", MAC_AFPLAY_PATH));
    }

    #[cfg(target_os = "linux")]
    {
        if command_exists("paplay") {
            return (true, "linux paplay".to_string());
        }
        if command_exists("aplay") {
            return (true, "linux aplay".to_string());
        }
        return (
            false,
            "未找到 paplay/aplay（请安装 pulseaudio-utils 或 alsa-utils）".to_string(),
        );
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        (false, "当前系统未实现提示音后端".to_string())
    }
}

fn play_feedback_sound(enabled: bool, event: FeedbackSoundEvent) {
    if !enabled {
        return;
    }

    if let Err(err) = send_feedback_sound(event) {
        warn!("提示音播放失败: {}", err);
    }
}

fn send_feedback_sound(event: FeedbackSoundEvent) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let sound_path = match event {
            FeedbackSoundEvent::RecordingStart => MAC_SOUND_RECORDING_START,
            FeedbackSoundEvent::RecordingEnd => MAC_SOUND_RECORDING_END,
        };
        if !std::path::Path::new(sound_path).is_file() {
            anyhow::bail!("找不到系统提示音文件: {}", sound_path);
        }
        let _child = std::process::Command::new(MAC_AFPLAY_PATH)
            .arg(sound_path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .context("启动 afplay 失败")?;
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    {
        let _ = event;
        let mut child = if command_exists("paplay") {
            std::process::Command::new("paplay")
                .arg("/usr/share/sounds/freedesktop/stereo/message.oga")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .context("启动 paplay 失败")?
        } else if command_exists("aplay") {
            std::process::Command::new("aplay")
                .arg("/usr/share/sounds/alsa/Front_Center.wav")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .context("启动 aplay 失败")?
        } else {
            anyhow::bail!("未找到 paplay/aplay");
        };
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        return Ok(());
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = event;
        Ok(())
    }
}

fn print_macos_notification_setup_tip(show_tip: bool) {
    if !show_tip {
        return;
    }

    #[cfg(target_os = "macos")]
    {
        println!("提示: macOS 通知由 osascript 发送，通知来源通常显示为“脚本编辑器”。");
        println!(
            "若全屏时看不到横幅，请到“系统设置 -> 通知 -> 脚本编辑器”开启通知，并选择“横幅”或“提醒”。"
        );
        println!("若仍不弹出，请检查“专注模式”和“通知摘要”设置。");
    }
}

#[cfg(target_os = "linux")]
fn command_exists(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    if name.contains('/') {
        return std::path::Path::new(name).is_file();
    }
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path_var).any(|dir| dir.join(name).is_file())
}

fn send_desktop_notify(title: &str, body: &str) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        let status = std::process::Command::new("notify-send")
            .arg("-a")
            .arg("EchoPup")
            .arg("-u")
            .arg("low")
            .arg(title)
            .arg(body)
            .status()
            .context("执行 notify-send 失败")?;
        if !status.success() {
            anyhow::bail!("notify-send 返回非零状态: {}", status);
        }
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        let escaped_title = title.replace('"', "\\\"");
        let escaped_body = body.replace('"', "\\\"");
        let status = std::process::Command::new(MAC_OSASCRIPT_PATH)
            .arg("-e")
            .arg(format!(
                "display notification \"{}\" with title \"{}\"",
                escaped_body, escaped_title
            ))
            .status()
            .context("执行 osascript 失败")?;
        if !status.success() {
            anyhow::bail!("osascript 返回非零状态: {}", status);
        }
        return Ok(());
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (title, body);
        Ok(())
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    info!("EchoPup 启动中...");

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Run) => run_voice_input(&cli.config)?,
        Some(Commands::Start) => start_background_mode(&cli.config)?,
        Some(Commands::Stop) => stop_background_mode()?,
        Some(Commands::Status) => show_background_status()?,
        Some(Commands::Restart) => restart_background_mode(&cli.config)?,
        Some(Commands::Ui { command }) => handle_ui_command(&cli.config, command)?,
        Some(Commands::Test) => test_modules(&cli.config)?,
        Some(Commands::Config { show, init }) => {
            if init {
                let config = config::Config::default();
                config.save(&cli.config)?;
                info!("默认配置已保存到: {}", cli.config);
            }
            if show {
                let config = config::Config::load(&cli.config)?;
                println!("{:#?}", config);
            }
        }
        Some(Commands::DownloadModel { size }) => download_model_via_shared_runtime(&size)?,
        Some(Commands::StatusIndicator) => status_indicator::run_status_indicator_process()?,
        None => start_background_mode(&cli.config)?,
    }

    Ok(())
}

fn download_model_via_shared_runtime(size: &str) -> Result<()> {
    use model_download::{DownloadEvent, DownloadStart};

    info!("下载 Whisper {} 模型", size);

    let DownloadStart {
        rx, initial_logs, ..
    } = model_download::start_model_download(size)?;

    for line in initial_logs {
        println!("{}", line);
    }

    let mut latest_total = None::<u64>;
    let mut progress_active = false;

    loop {
        match rx.recv() {
            Ok(DownloadEvent::Started {
                downloaded, total, ..
            }) => {
                latest_total = total;
                if progress_active {
                    println!();
                    progress_active = false;
                }
                println!(
                    "[started] 已下载 {}，总大小 {}",
                    model_download::format_bytes(downloaded),
                    total
                        .map(model_download::format_bytes)
                        .unwrap_or_else(|| "未知".to_string())
                );
            }
            Ok(DownloadEvent::Progress { downloaded, total }) => {
                if total.is_some() {
                    latest_total = total;
                }

                let progress_text = match latest_total {
                    Some(total) if total > 0 => {
                        let ratio = (downloaded as f64 / total as f64).clamp(0.0, 1.0);
                        format!(
                            "{} / {} ({:.1}%)",
                            model_download::format_bytes(downloaded),
                            model_download::format_bytes(total),
                            ratio * 100.0
                        )
                    }
                    _ => format!("已下载 {}", model_download::format_bytes(downloaded)),
                };

                print!("\r[progress] {}", progress_text);
                let _ = std::io::stdout().flush();
                progress_active = true;
            }
            Ok(DownloadEvent::Finished) => {
                if progress_active {
                    println!();
                }
                println!("[finished] 下载完成");
                return Ok(());
            }
            Ok(DownloadEvent::Failed(err)) => {
                if progress_active {
                    println!();
                }
                return Err(anyhow::anyhow!("下载失败: {}", err));
            }
            Ok(DownloadEvent::Log(line)) => {
                if progress_active {
                    println!();
                    progress_active = false;
                }
                println!("{}", line);
            }
            Err(_) => {
                if progress_active {
                    println!();
                }
                return Err(anyhow::anyhow!("下载线程已断开"));
            }
        }
    }
}

fn start_background_mode(config_path: &str) -> Result<()> {
    validate_hotkey_before_background_start(config_path)?;
    let config = config::Config::load(config_path)?;

    if runtime::is_running()? {
        if let Some(pid) = runtime::running_instance_pid()? {
            println!("echopup 已在后台运行 (pid: {})，不会重复创建进程。", pid);
        } else {
            println!("echopup 已在后台运行，不会重复创建进程。");
        }
        println!("可使用 `echopup ui` 管理配置。");
        return Ok(());
    }

    let pid = runtime::spawn_background(config_path)?;
    let log_path = runtime::background_log_path()?;
    println!("echopup 已在后台启动 (pid: {})", pid);
    println!("日志文件: {}", log_path.display());
    print_macos_notification_setup_tip(config.feedback.notify_tip_on_start);

    println!("可使用 `echopup ui` 管理配置。");
    Ok(())
}

fn stop_background_mode() -> Result<()> {
    match runtime::stop_running_instance()? {
        Some(pid) => println!("echopup 已停止 (pid: {})", pid),
        None => println!("echopup 未在运行。"),
    }
    Ok(())
}

fn show_background_status() -> Result<()> {
    match runtime::running_instance_pid()? {
        Some(pid) => {
            println!("echopup 正在运行 (pid: {})", pid);
            if let Ok(log_path) = runtime::background_log_path() {
                println!("日志文件: {}", log_path.display());
            }
        }
        None => println!("echopup 未运行。"),
    }
    Ok(())
}

fn restart_background_mode(config_path: &str) -> Result<()> {
    validate_hotkey_before_background_start(config_path)?;
    let config = config::Config::load(config_path)?;

    if let Some(pid) = runtime::stop_running_instance()? {
        println!("已停止旧实例 (pid: {})", pid);
    }

    let pid = runtime::spawn_background(config_path)?;
    let log_path = runtime::background_log_path()?;
    println!("echopup 已重启并在后台运行 (pid: {})", pid);
    println!("日志文件: {}", log_path.display());
    print_macos_notification_setup_tip(config.feedback.notify_tip_on_start);
    println!("可使用 `echopup ui` 管理配置。");
    Ok(())
}

fn validate_hotkey_before_background_start(config_path: &str) -> Result<()> {
    let config = config::Config::load(config_path)?;
    if let Err(err) = hotkey::validate_hotkey_config(&config.hotkey.key) {
        anyhow::bail!(
            "热键配置不安全/不可用: {}。{}",
            err,
            hotkey::hotkey_policy_hint()
        );
    }
    Ok(())
}

fn handle_ui_command(config_path: &str, command: Option<UiCommands>) -> Result<()> {
    match command.unwrap_or(UiCommands::Start) {
        UiCommands::Start => run_ui_foreground(config_path),
        UiCommands::Stop => {
            match runtime::stop_ui_instance()? {
                Some(pid) => println!("echopup ui 已停止 (pid: {})", pid),
                None => println!("echopup ui 未运行。"),
            }
            Ok(())
        }
        UiCommands::Status => {
            match runtime::ui_running_pid()? {
                Some(pid) => println!("echopup ui 正在运行 (pid: {})", pid),
                None => println!("echopup ui 未运行。"),
            }
            Ok(())
        }
        UiCommands::Restart => {
            if let Some(pid) = runtime::stop_ui_instance()? {
                println!("已停止旧的 echopup ui (pid: {})", pid);
            }
            run_ui_foreground(config_path)
        }
    }
}

fn run_ui_foreground(config_path: &str) -> Result<()> {
    let (ui_guard, acquire_mode) = runtime::acquire_ui_guard_for_foreground()?;
    if matches!(acquire_mode, runtime::UiAcquireMode::TookOverPrevious) {
        println!("检测到已有 echopup ui，已切换到当前终端。");
    }
    let ui_result = ui::run_ui(config_path);
    drop(ui_guard);
    ui_result
}

fn build_whisper_asr_engine(
    whisper_cfg: &config::config::WhisperConfig,
) -> Result<asr::WhisperAsrEngine> {
    let mut runtime_cfg = whisper_cfg.clone();
    let resolved_model_path = resolve_runtime_model_path(&runtime_cfg.model_path)?;
    if resolved_model_path != runtime_cfg.model_path {
        warn!(
            "Whisper 模型 {} 不存在，自动回退到 {}",
            runtime_cfg.model_path, resolved_model_path
        );
        runtime_cfg.model_path = resolved_model_path;
    }
    runtime_cfg.sync_model_path_defaults_if_generic();

    let mut w = stt::WhisperSTT::with_options(
        &runtime_cfg.model_path,
        runtime_cfg.language.clone(),
        runtime_cfg.translate,
    )?;

    let strategy = match runtime_cfg.decoding_strategy.clone() {
        config::WhisperDecodingStrategy::Greedy => stt::DecodingStrategy::Greedy {
            best_of: runtime_cfg.greedy_best_of,
        },
        config::WhisperDecodingStrategy::BeamSearch => stt::DecodingStrategy::BeamSearch {
            beam_size: runtime_cfg.beam_size,
        },
    };
    w.set_decoding_strategy(strategy);
    w.set_temperature(runtime_cfg.temperature);
    w.set_no_context(runtime_cfg.no_context);
    w.set_suppress_nst(runtime_cfg.suppress_nst);
    w.set_n_threads(runtime_cfg.resolved_n_threads());
    w.set_initial_prompt(runtime_cfg.initial_prompt.clone());
    w.set_hotwords(runtime_cfg.hotwords.clone());
    Ok(asr::WhisperAsrEngine::new(w))
}

fn resolve_preview_model_path(whisper_cfg: &config::config::WhisperConfig) -> Option<String> {
    let current_path = Path::new(&whisper_cfg.model_path);
    let model_dir = current_path.parent()?;
    let file_name = current_path.file_name()?.to_str()?;
    let candidates: &[&str] = match file_name {
        "ggml-large-v3.bin" => &["ggml-large-v3-turbo.bin", "ggml-medium.bin"],
        "ggml-large-v3-turbo.bin" => &["ggml-medium.bin"],
        _ => &[],
    };

    for candidate in candidates {
        let candidate_path = model_dir.join(candidate);
        if candidate_path.is_file() {
            return Some(candidate_path.to_string_lossy().into_owned());
        }
    }

    None
}

fn build_preview_whisper_asr_engine(
    whisper_cfg: &config::config::WhisperConfig,
) -> Result<asr::WhisperAsrEngine> {
    let mut preview_cfg = whisper_cfg.clone();
    let current_model_name = Path::new(&whisper_cfg.model_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if let Some(preview_model_path) = resolve_preview_model_path(whisper_cfg) {
        preview_cfg.model_path = preview_model_path;
    } else if current_model_name == "ggml-large-v3.bin" {
        anyhow::bail!("large-v3 预览需要已下载 turbo 或 medium 模型");
    }
    preview_cfg.n_threads =
        config::config::WhisperThreadSetting::Fixed(whisper_cfg.resolved_n_threads().clamp(1, 2));
    preview_cfg.decoding_strategy = config::WhisperDecodingStrategy::Greedy;
    preview_cfg.greedy_best_of = 1;
    build_whisper_asr_engine(&preview_cfg)
}

fn build_sherpa_sensevoice_asr_engine(
    config: &config::config::Config,
    preview: bool,
) -> Result<asr::sherpa_onnx::SherpaSenseVoiceEngine> {
    if config.audio.sample_rate != 16000 {
        anyhow::bail!(
            "sherpa SenseVoice 当前要求 audio.sample_rate=16000，当前为 {}",
            config.audio.sample_rate
        );
    }

    let mut num_threads = config.asr.sherpa.num_threads.max(1);
    if preview {
        num_threads = num_threads.clamp(1, 2);
    }

    asr::sherpa_onnx::SherpaSenseVoiceEngine::new(asr::sherpa_onnx::SherpaSenseVoiceConfig {
        model_path: config.asr.sherpa.model_path.clone(),
        tokens_path: config.asr.sherpa.tokens_path.clone(),
        language: config.asr.sherpa.language.clone(),
        use_itn: config.asr.sherpa.use_itn,
        provider: config.asr.sherpa.provider.clone(),
        num_threads,
        sample_rate: config.audio.sample_rate as i32,
    })
}

fn build_selected_asr_engine(
    config: &config::config::Config,
    preview: bool,
) -> Result<Box<dyn asr::AsrEngine>> {
    match config.asr.backend {
        config::AsrBackend::Whisper => {
            let whisper_cfg = config.whisper.effective();
            if preview {
                Ok(Box::new(build_preview_whisper_asr_engine(&whisper_cfg)?))
            } else {
                Ok(Box::new(build_whisper_asr_engine(&whisper_cfg)?))
            }
        }
        config::AsrBackend::SherpaSenseVoice => Ok(Box::new(build_sherpa_sensevoice_asr_engine(
            config, preview,
        )?)),
    }
}

fn build_asr_engine_with_fallback(
    config: &config::config::Config,
    preview: bool,
) -> Result<Box<dyn asr::AsrEngine>> {
    match build_selected_asr_engine(config, preview) {
        Ok(engine) => Ok(engine),
        Err(err)
            if config.asr.backend != config::AsrBackend::Whisper
                && config.asr.allow_fallback_to_whisper =>
        {
            warn!(
                "ASR 后端 {} 初始化失败: {}，自动回退到 Whisper",
                config.asr.backend.label(),
                err
            );
            let whisper_cfg = config.whisper.effective();
            if preview {
                Ok(Box::new(build_preview_whisper_asr_engine(&whisper_cfg)?))
            } else {
                Ok(Box::new(build_whisper_asr_engine(&whisper_cfg)?))
            }
        }
        Err(err) => Err(err),
    }
}

fn build_llm_runtime(llm_cfg: &config::config::LLMConfig) -> Option<llm::LLMRewrite> {
    if !llm_cfg.enabled {
        return None;
    }
    match llm::LLMRewrite::new(
        &llm_cfg.provider,
        &llm_cfg.api_base,
        &llm_cfg.api_key_env,
        &llm_cfg.model,
    ) {
        Ok(l) => Some(l),
        Err(err) => {
            warn!("LLM 热更新失败: {}", err);
            None
        }
    }
}

fn apply_runtime_menu_action(
    action: &menu_core::MenuAction,
    snapshot: &menu_core::MenuSnapshot,
    hotkey_listener: &mut hotkey::HotkeyListener,
    hotkey_trigger_mode_runtime: &Arc<Mutex<config::HotkeyTriggerMode>>,
    asr_runtime: &Arc<Mutex<Option<Box<dyn asr::AsrEngine>>>>,
    preview_asr_runtime: &Arc<Mutex<Option<Box<dyn asr::AsrEngine>>>>,
    llm_runtime: &Arc<Mutex<Option<llm::LLMRewrite>>>,
) -> Result<()> {
    match action {
        menu_core::MenuAction::SetField {
            field: menu_core::EditableField::Hotkey,
            ..
        } => {
            hotkey_listener.set_hotkey(&snapshot.hotkey)?;
            hotkey_listener.start()?;
            info!("热键已热更新为 {}", snapshot.hotkey);
        }
        menu_core::MenuAction::SetField {
            field: menu_core::EditableField::WhisperModelPath,
            ..
        }
        | menu_core::MenuAction::SwitchWhisperModel { .. } => {
            let cfg = config::Config::load(&snapshot.config_path)?;
            match build_asr_engine_with_fallback(&cfg, false) {
                Ok(new_asr) => {
                    log_asr_runtime("主转写运行时已热更新", new_asr.as_ref());
                    let mut guard = asr_runtime.lock();
                    *guard = Some(new_asr);
                    drop(guard);
                    let mut callback_guard = preview_asr_runtime.lock();
                    *callback_guard = build_asr_engine_with_fallback(&cfg, true).ok();
                }
                Err(err) => {
                    warn!("ASR 热更新失败: {}", err);
                    if let Some(current) = asr_runtime.lock().as_ref() {
                        let runtime = current.runtime_info();
                        warn!(
                            "ASR 热更新失败后继续使用当前运行时: backend={}, model={}",
                            runtime.backend.label(),
                            runtime.model
                        );
                    }
                }
            }
        }
        menu_core::MenuAction::SetHotkeyTriggerMode { mode } => {
            *hotkey_trigger_mode_runtime.lock() = *mode;
            info!("热键触发模式已热更新为 {}", mode.label());
        }
        menu_core::MenuAction::OpenConfigFolder => {
            #[cfg(target_os = "linux")]
            {
                status_indicator::open_config_folder_linux(&snapshot.config_path)?;
                info!("已打开配置文件夹: {}", snapshot.config_path);
            }
        }
        menu_core::MenuAction::OpenModelFolder => {
            #[cfg(target_os = "linux")]
            {
                status_indicator::open_model_folder_linux()?;
                info!("已打开模型文件夹");
            }
        }
        menu_core::MenuAction::SetField {
            field:
                menu_core::EditableField::LlmProvider
                | menu_core::EditableField::LlmModel
                | menu_core::EditableField::LlmApiBase
                | menu_core::EditableField::LlmApiKeyEnv,
            ..
        }
        | menu_core::MenuAction::SetLlmConfig { .. }
        | menu_core::MenuAction::ToggleLlmEnabled => {
            let cfg = config::Config::load(&snapshot.config_path)?;
            let mut guard = llm_runtime.lock();
            *guard = build_llm_runtime(&cfg.llm);
            info!("LLM 运行时配置已热更新");
        }
        menu_core::MenuAction::ReloadConfig => {
            let cfg = config::Config::load(&snapshot.config_path)?;

            hotkey_listener.set_hotkey(&cfg.hotkey.key)?;
            hotkey_listener.start()?;
            *hotkey_trigger_mode_runtime.lock() = cfg.hotkey.trigger_mode;

            match build_asr_engine_with_fallback(&cfg, false) {
                Ok(new_asr) => {
                    log_asr_runtime("主转写运行时已重载", new_asr.as_ref());
                    let mut guard = asr_runtime.lock();
                    *guard = Some(new_asr);
                    drop(guard);
                    let mut callback_guard = preview_asr_runtime.lock();
                    *callback_guard = build_asr_engine_with_fallback(&cfg, true).ok();
                }
                Err(err) => {
                    warn!("ASR 重载失败: {}", err);
                    if let Some(current) = asr_runtime.lock().as_ref() {
                        let runtime = current.runtime_info();
                        warn!(
                            "ASR 重载失败后继续使用当前运行时: backend={}, model={}",
                            runtime.backend.label(),
                            runtime.model
                        );
                    }
                }
            }

            let mut llm_guard = llm_runtime.lock();
            *llm_guard = build_llm_runtime(&cfg.llm);
            info!("配置文件重载并已应用到运行时");
        }
        _ => {}
    }
    Ok(())
}

fn run_voice_input(config_path: &str) -> Result<()> {
    let _instance_guard = match runtime::InstanceGuard::try_acquire()? {
        Some(guard) => guard,
        None => {
            println!("echopup 已在运行，不会启动新实例。");
            println!("可使用 `echopup ui` 管理配置。");
            return Ok(());
        }
    };

    // ===== 首次运行引导 =====
    if config::Config::is_first_run(config_path) {
        println!("");
        println!("===========================================");
        println!("🎉 欢迎使用 EchoPup！");
        println!("===========================================");
        println!("");
        println!("首次运行，请先配置 LLM 以启用文本整理功能。");
        println!("");
        println!("📝 配置示例 (Ollama - 本地部署):");
        println!("");
        println!("  [llm]");
        println!("  enabled = true");
        println!("  provider = \"ollama\"");
        println!("  model = \"llama3\"");
        println!("  api_base = \"http://localhost:11434/v1\"");
        println!("  api_key_env = \"\"");
        println!("");
        println!("📝 配置示例 (OpenAI):");
        println!("");
        println!("  [llm]");
        println!("  enabled = true");
        println!("  provider = \"openai\"");
        println!("  model = \"gpt-4o-mini\"");
        println!("  api_base = \"https://api.openai.com/v1\"");
        println!("  api_key_env = \"OPENAI_API_KEY\"");
        println!("");
        println!("💡 提示：");
        println!("  - Ollama: 从 https://ollama.com 下载安装");
        println!("  - 运行 'ollama serve' 启动 Ollama 服务");
        println!("  - 使用 'ollama pull llama3' 下载模型");
        println!("");
        println!(
            "编辑配置文件: {}",
            config_path.replace(
                "~",
                &dirs::home_dir().unwrap_or_default().display().to_string()
            )
        );
        println!("");
        println!("===========================================");
        println!("");

        // 如果默认配置下 LLM 未配置，也显示提示
        let default_config = config::Config::default();
        if !default_config.is_llm_configured() {
            info!("首次运行引导：LLM 未配置，将以基础模式运行（仅语音转文字）");
        }
    } else {
        // 非首次运行，检查 LLM 配置状态
        let config = config::Config::load(config_path)?;
        if !config.is_llm_configured() {
            info!("LLM 未配置，将以基础模式运行（仅语音转文字）");
        }
    }

    let config = config::Config::load(config_path)?;
    if let Err(err) = hotkey::validate_hotkey_config(&config.hotkey.key) {
        anyhow::bail!(
            "热键配置不安全/不可用: {}。{}",
            err,
            hotkey::hotkey_policy_hint()
        );
    }
    let mut menu_runtime = menu_core::MenuCore::new(config_path)?;
    print_macos_notification_setup_tip(config.feedback.notify_tip_on_start);

    let status_indicator = Arc::new(Mutex::new(status_indicator::StatusIndicatorClient::start(
        config.feedback.status_bar_enabled,
        config_path,
    )));
    let menu_snapshot_state = Arc::new(Mutex::new(menu_runtime.snapshot()));
    let status_indicator_enabled = {
        let guard = status_indicator.lock();
        guard.is_enabled()
    };
    if status_indicator_enabled {
        info!("状态栏反馈已启用: macOS 菜单栏");
        set_status_indicator_state(&status_indicator, status_indicator::IndicatorState::Idle);
        let snapshot = menu_snapshot_state.lock().clone();
        let mut guard = status_indicator.lock();
        guard.send_snapshot(&snapshot);
    } else if config.feedback.status_bar_enabled {
        warn!("状态栏反馈未启用（macOS 菜单栏子进程未启动）");
    } else {
        info!("状态栏反馈已关闭（feedback.status_bar_enabled=false）");
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    let _terminal_echo_guard = match TerminalEchoGuard::try_disable_stdin_echo() {
        Ok(guard) => guard,
        Err(err) => {
            warn!("无法关闭终端输入回显，将继续运行: {}", err);
            None
        }
    };

    // ===== 初始化模块 =====
    let recorder = Arc::new(audio::AudioRecorder::new(
        config.audio.sample_rate,
        config.audio.channels,
    )?);
    info!("音频录制器已初始化");

    match config.asr.backend {
        config::AsrBackend::Whisper => {
            let whisper_cfg = config.whisper.effective();
            if let Some(profile) = config.whisper.performance_profile {
                info!(
                    "Whisper 性能档位: {:?}，模型: {}，策略: {:?}",
                    profile, whisper_cfg.model_path, whisper_cfg.decoding_strategy
                );
            }
        }
        config::AsrBackend::SherpaSenseVoice => {
            info!(
                "Sherpa SenseVoice 已选中: model={}, tokens={}, provider={}",
                config.asr.sherpa.model_path,
                config.asr.sherpa.tokens_path,
                config.asr.sherpa.provider.as_deref().unwrap_or("cpu")
            );
        }
    }

    let asr_runtime = match build_asr_engine_with_fallback(&config, false) {
        Ok(engine) => {
            log_asr_runtime("主转写运行时已启用", engine.as_ref());
            info!("ASR 运行时已初始化");
            Some(engine)
        }
        Err(e) => {
            warn!("ASR 初始化失败: {}，语音转写功能不可用", e);
            None
        }
    };
    let asr_runtime = Arc::new(Mutex::new(asr_runtime));
    let preview_asr_runtime = match build_asr_engine_with_fallback(&config, true) {
        Ok(engine) => {
            log_asr_runtime("预览运行时已启用", engine.as_ref());
            Some(engine)
        }
        Err(e) => {
            warn!("ASR 预览运行时初始化失败: {}，将禁用流式预览", e);
            None
        }
    };
    let preview_asr_runtime = Arc::new(Mutex::new(preview_asr_runtime));

    let llm = if config.llm.enabled {
        match build_llm_runtime(&config.llm) {
            Some(l) => {
                info!("LLM 整理已初始化");
                Some(l)
            }
            None => {
                warn!("LLM 初始化失败，文本整理功能不可用");
                None
            }
        }
    } else {
        info!("LLM 整理未启用");
        None
    };
    // 使用 Mutex 包装，以便在回调中共享
    let llm = Arc::new(Mutex::new(llm));

    let post_processor = Arc::new(stt::TextPostProcessor::new(&config.text_correction));
    if config.text_correction.enabled {
        info!(
            "谐音纠错已启用，规则数: {}",
            config.text_correction.homophone_map.len()
        );
    } else {
        info!("谐音纠错未启用");
    }

    let text_commit = Arc::new(Mutex::new(
        Box::new(commit::InsertOnlyTextCommit::new().map_err(|e| {
            error!("文本提交后端初始化失败: {}", e);
            e
        })?) as Box<dyn commit::TextCommitBackend>,
    ));
    {
        let commit_guard = text_commit.lock();
        info!(
            "文本提交后端已初始化: backend={}, draft_replace_supported={}",
            commit_guard.backend_name(),
            commit_guard.supports_draft_replacement()
        );
    }

    let (desktop_notify_enabled, notify_desc) = detect_desktop_notify_capability();
    if desktop_notify_enabled {
        info!("桌面通知已启用: {}", notify_desc);
    } else {
        warn!("桌面通知未启用: {}", notify_desc);
    }
    #[cfg(target_os = "macos")]
    if desktop_notify_enabled {
        info!("提示：macOS 通知来源显示为“脚本编辑器”属于系统行为（由 osascript 发送）");
    }

    let (sound_feedback_enabled, sound_desc) =
        detect_sound_feedback_capability(config.feedback.sound_enabled);
    if sound_feedback_enabled {
        info!("提示音反馈已启用: {}", sound_desc);
    } else {
        warn!("提示音反馈未启用: {}", sound_desc);
    }

    // ===== 状态标记 =====
    let is_recording = Arc::new(AtomicBool::new(false));
    let vad_triggered = Arc::new(AtomicBool::new(false)); // VAD 触发标记
    let partial_stt_should_stop = Arc::new(AtomicBool::new(false));
    let partial_stt_handle = Arc::new(Mutex::new(None::<std::thread::JoinHandle<()>>));
    let partial_stt_callback_handle = Arc::new(Mutex::new(None::<std::thread::JoinHandle<()>>));
    let recognition_session = Arc::new(Mutex::new(session::RecognitionSession::new()));

    // 录音动画控制
    let recording_animation = Arc::new(AtomicBool::new(false));
    let animation_should_stop = Arc::new(AtomicBool::new(false));

    // 启动录音动画线程
    let anim_is_recording = is_recording.clone();
    let anim_should_stop = animation_should_stop.clone();
    let animation_handle = std::thread::spawn(move || {
        let chars = ['|', '/', '-', '\\'];
        let mut index = 0;
        loop {
            if anim_should_stop.load(Ordering::SeqCst) {
                break;
            }
            if anim_is_recording.load(Ordering::SeqCst) {
                print!("\r🔴 录音中... {}", chars[index]);
                std::io::Write::flush(&mut std::io::stdout()).ok();
                index = (index + 1) % chars.len();
                std::thread::sleep(std::time::Duration::from_millis(200));
            } else {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    });

    // ===== 设置热键回调 =====
    #[derive(Default)]
    struct HotkeyPressState {
        pressed: bool,
        sequence: u64,
        started_by_hold_on_current_press: bool,
    }
    let hold_to_record_duration = Duration::from_secs(1);
    let stop_press_debounce_window = Duration::from_millis(500);
    let hotkey_press_state = Arc::new(Mutex::new(HotkeyPressState::default()));
    let stop_debounce_until = Arc::new(Mutex::new(None::<Instant>));
    let hotkey_trigger_mode = Arc::new(Mutex::new(config.hotkey.trigger_mode));

    let recorder_start = recorder.clone();
    let preview_asr_runtime_on_start = preview_asr_runtime.clone();
    let is_recording_start = is_recording.clone();
    let recording_animation_start = recording_animation.clone();
    let desktop_notify_on_start = desktop_notify_enabled;
    let sound_feedback_on_start = sound_feedback_enabled;
    let status_indicator_on_start = status_indicator.clone();
    let menu_snapshot_on_start = menu_snapshot_state.clone();
    let stop_debounce_on_start = stop_debounce_until.clone();
    let partial_stt_stop_on_start = partial_stt_should_stop.clone();
    let partial_stt_handle_on_start = partial_stt_handle.clone();
    let partial_stt_callback_handle_on_start = partial_stt_callback_handle.clone();
    let recognition_session_on_start = recognition_session.clone();
    let start_recording_action: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        if !is_recording_start.load(Ordering::SeqCst) {
            clear_terminal_artifacts_if_tty();
            recording_animation_start.store(true, Ordering::SeqCst);
            info!("开始录音...");
            set_status_indicator_state(
                &status_indicator_on_start,
                status_indicator::IndicatorState::RecordingStart,
            );
            match recorder_start.start() {
                Ok(_) => {
                    is_recording_start.store(true, Ordering::SeqCst);
                    *stop_debounce_on_start.lock() =
                        Some(Instant::now() + stop_press_debounce_window);
                    set_status_indicator_state(
                        &status_indicator_on_start,
                        status_indicator::IndicatorState::Recording,
                    );
                    play_feedback_sound(
                        sound_feedback_on_start,
                        FeedbackSoundEvent::RecordingStart,
                    );
                    desktop_notify(desktop_notify_on_start, "EchoPup", "开始录音");

                    partial_stt_stop_on_start.store(false, Ordering::SeqCst);
                    recognition_session_on_start.lock().reset();
                    detach_thread_handle(&partial_stt_handle_on_start);
                    detach_thread_handle(&partial_stt_callback_handle_on_start);

                    let recorder_callback = recorder_start.clone();
                    let preview_asr_runtime_for_callback = preview_asr_runtime_on_start.clone();
                    let status_indicator_callback = status_indicator_on_start.clone();
                    let menu_snapshot_callback = menu_snapshot_on_start.clone();
                    let is_recording_callback = is_recording_start.clone();
                    let partial_stt_stop_callback = partial_stt_stop_on_start.clone();
                    let recognition_session_for_callback = recognition_session_on_start.clone();
                    let poll_interval = Duration::from_millis(500);
                    let min_samples = (recorder_callback.target_sample_rate() as usize)
                        .saturating_mul(800)
                        / 1000;
                    let max_preview_samples =
                        (recorder_callback.target_sample_rate() as usize).saturating_mul(3);
                    let callback_handle = std::thread::spawn(move || {
                        let mut preview_session: Box<dyn asr::AsrSession> = {
                            let asr_guard = preview_asr_runtime_for_callback.lock();
                            let Some(ref asr_engine) = *asr_guard else {
                                return;
                            };

                            match asr_engine.start_session(asr::AsrSessionConfig {
                                min_partial_samples: min_samples,
                                max_partial_window_samples: max_preview_samples,
                            }) {
                                Ok(session) => {
                                    info!(
                                        "ASR 预览会话已启动: backend={}, min_samples={}, window_samples={}",
                                        session.backend_kind().label(),
                                        min_samples,
                                        max_preview_samples
                                    );
                                    session
                                }
                                Err(err) => {
                                    warn!("创建 ASR 预览会话失败: {}", err);
                                    return;
                                }
                            }
                        };

                        let mut preview_cursor: audio::AudioChunkCursor =
                            recorder_callback.incremental_cursor();

                        while is_recording_callback.load(Ordering::SeqCst)
                            && !partial_stt_stop_callback.load(Ordering::SeqCst)
                        {
                            std::thread::sleep(poll_interval);
                            if !is_recording_callback.load(Ordering::SeqCst)
                                || partial_stt_stop_callback.load(Ordering::SeqCst)
                            {
                                break;
                            }

                            let new_samples = recorder_callback
                                .read_incremental_target_samples(&mut preview_cursor);
                            if new_samples.is_empty() {
                                continue;
                            }

                            if let Err(err) = preview_session.accept_audio(&new_samples) {
                                warn!("ASR 预览会话接收音频失败: {}", err);
                                break;
                            }

                            match preview_session.poll_partial(partial_stt_stop_callback.clone()) {
                                Ok(Some(text)) => {
                                    if partial_stt_stop_callback.load(Ordering::SeqCst) {
                                        break;
                                    }

                                    let update = {
                                        let mut session_guard =
                                            recognition_session_for_callback.lock();
                                        session_guard.update_partial(&text)
                                    };

                                    if let Some(update) = update {
                                        send_status_snapshot(
                                            &status_indicator_callback,
                                            &menu_snapshot_callback,
                                            update.status_text,
                                        );
                                    }
                                }
                                Ok(None) => {}
                                Err(err) => {
                                    warn!(
                                        "ASR 预览会话 partial 识别失败: backend={}, buffered_samples={}, err={}",
                                        preview_session.backend_kind().label(),
                                        preview_session.buffered_samples(),
                                        err
                                    );
                                }
                            }
                        }
                    });
                    *partial_stt_callback_handle_on_start.lock() = Some(callback_handle);
                }
                Err(e) => {
                    error!("开始录音失败: {}", e);
                    recording_animation_start.store(false, Ordering::SeqCst);
                    set_status_indicator_state(
                        &status_indicator_on_start,
                        status_indicator::IndicatorState::Failed,
                    );
                    desktop_notify(desktop_notify_on_start, "EchoPup", "开始录音失败");
                }
            }
        }
    });

    let recorder_stop = recorder.clone();
    let asr_runtime_on_stop = asr_runtime.clone();
    let llm_stop = llm.clone();
    let post_processor_stop = post_processor.clone();
    let text_commit_stop = text_commit.clone();
    let is_recording_stop = is_recording.clone();
    let vad_triggered_stop = vad_triggered.clone();
    let recording_animation_stop = recording_animation.clone();
    let desktop_notify_on_stop = desktop_notify_enabled;
    let sound_feedback_on_stop = sound_feedback_enabled;
    let status_indicator_on_stop = status_indicator.clone();
    let stop_debounce_on_stop = stop_debounce_until.clone();
    let partial_stt_stop_on_stop = partial_stt_should_stop.clone();
    let partial_stt_handle_on_stop = partial_stt_handle.clone();
    let partial_stt_callback_handle_on_stop = partial_stt_callback_handle.clone();
    let recognition_session_on_stop = recognition_session.clone();
    let stop_recording_action: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        info!("stop_recording_action 开始执行");
        if is_recording_stop.load(Ordering::SeqCst) {
            *stop_debounce_on_stop.lock() = None;
            is_recording_stop.store(false, Ordering::SeqCst);
            recording_animation_stop.store(false, Ordering::SeqCst);
            print!("\r");
            clear_terminal_artifacts_if_tty();
            partial_stt_stop_on_stop.store(true, Ordering::SeqCst);
            info!("stop_recording_action: 准备调用 recorder.stop()");
            let audio_data = match recorder_stop.stop() {
                Ok(data) => data,
                Err(e) => {
                    error!("停止录音失败: {}", e);
                    set_status_indicator_state(
                        &status_indicator_on_stop,
                        status_indicator::IndicatorState::Failed,
                    );
                    desktop_notify(desktop_notify_on_stop, "EchoPup", "停止录音失败");
                    return;
                }
            };
            info!("stop_recording_action: recorder.stop() 返回成功");
            recognition_session_on_stop.lock().clear_partials();
            info!("stop_recording_action: 预览线程转为后台回收");
            detach_thread_handle(&partial_stt_handle_on_stop);
            detach_thread_handle(&partial_stt_callback_handle_on_stop);
            play_feedback_sound(sound_feedback_on_stop, FeedbackSoundEvent::RecordingEnd);

            let is_vad = vad_triggered_stop.swap(false, Ordering::SeqCst);
            let e2e_start = Instant::now();
            set_status_indicator_state(
                &status_indicator_on_stop,
                status_indicator::IndicatorState::Transcribing,
            );
            desktop_notify(desktop_notify_on_stop, "EchoPup", "识别中...");

            let ok = process_audio(
                &audio_data,
                &asr_runtime_on_stop,
                &llm_stop,
                &post_processor_stop,
                &text_commit_stop,
                &recognition_session_on_stop,
                is_vad,
                e2e_start,
                desktop_notify_on_stop,
            );
            set_status_indicator_state(
                &status_indicator_on_stop,
                if ok {
                    status_indicator::IndicatorState::Completed
                } else {
                    status_indicator::IndicatorState::Failed
                },
            );
        }
    });

    let press_state_on_press = hotkey_press_state.clone();
    let start_action_on_press = start_recording_action.clone();
    let stop_action_on_press = stop_recording_action.clone();
    let is_recording_on_press = is_recording.clone();
    let stop_debounce_on_press = stop_debounce_until.clone();
    let mode_on_press = hotkey_trigger_mode.clone();
    let press_callback: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        let trigger_mode = *mode_on_press.lock();
        let seq = {
            let mut state = press_state_on_press.lock();
            if state.pressed {
                return;
            }
            state.pressed = true;
            state.sequence = state.sequence.wrapping_add(1);
            state.started_by_hold_on_current_press = false;
            state.sequence
        };

        if is_recording_on_press.load(Ordering::SeqCst) {
            if trigger_mode != config::HotkeyTriggerMode::PressToToggle {
                return;
            }
            if let Some(deadline) = *stop_debounce_on_press.lock() {
                if Instant::now() < deadline {
                    return;
                }
            }
            stop_action_on_press();
            return;
        }

        let press_state_timer = press_state_on_press.clone();
        let start_action_timer = start_action_on_press.clone();
        let is_recording_timer = is_recording_on_press.clone();
        std::thread::spawn(move || {
            std::thread::sleep(hold_to_record_duration);
            let should_start = {
                let mut state = press_state_timer.lock();
                if !(state.pressed && state.sequence == seq) {
                    false
                } else {
                    state.started_by_hold_on_current_press = true;
                    true
                }
            };
            if !should_start || is_recording_timer.load(Ordering::SeqCst) {
                return;
            }
            start_action_timer();
        });
    });

    let press_state_on_release = hotkey_press_state.clone();
    let is_recording_on_release = is_recording.clone();
    let stop_action_on_release = stop_recording_action.clone();
    let mode_on_release = hotkey_trigger_mode.clone();
    let release_callback: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        info!("release_callback triggered");
        let started_by_hold = {
            let mut state = press_state_on_release.lock();
            if !state.pressed {
                info!("release_callback: state.pressed was false, returning early");
                return;
            }
            state.pressed = false;
            state.sequence = state.sequence.wrapping_add(1);
            let started = state.started_by_hold_on_current_press;
            state.started_by_hold_on_current_press = false;
            info!("release_callback: started_by_hold={}", started);
            started
        };

        let mode = *mode_on_release.lock();
        let recording = is_recording_on_release.load(Ordering::SeqCst);
        info!(
            "release_callback: mode={:?}, started_by_hold={}, is_recording={}",
            mode, started_by_hold, recording
        );

        if mode == config::HotkeyTriggerMode::HoldToRecord && started_by_hold && recording {
            info!("release_callback: calling stop_action_on_release");
            stop_action_on_release();
        }
    });

    // ===== 设置端点检测（VAD）回调 =====
    // 根据配置决定是否启用 VAD
    let vad_enabled = config.audio.vad_enabled;

    if vad_enabled {
        info!("端点检测已启用");

        let vad_recorder = recorder.clone();
        let vad_asr_runtime = asr_runtime.clone();
        let vad_llm = llm.clone();
        let vad_post_processor = post_processor.clone();
        let vad_text_commit = text_commit.clone();
        let vad_is_recording = is_recording.clone();
        let vad_triggered_callback = vad_triggered.clone();
        let vad_recording_animation = recording_animation.clone();
        let desktop_notify_on_vad = desktop_notify_enabled;
        let sound_feedback_on_vad = sound_feedback_enabled;
        let status_indicator_on_vad = status_indicator.clone();
        let partial_stt_stop_on_vad = partial_stt_should_stop.clone();
        let partial_stt_handle_on_vad = partial_stt_handle.clone();
        let partial_stt_callback_handle_on_vad = partial_stt_callback_handle.clone();
        let recognition_session_on_vad = recognition_session.clone();

        let vad_callback: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            info!("端点检测：语音结束，触发自动转写");

            // 标记 VAD 已触发
            vad_triggered_callback.store(true, Ordering::SeqCst);

            // 停止录音
            if vad_is_recording.load(Ordering::SeqCst) {
                vad_is_recording.store(false, Ordering::SeqCst);
                // 停止动画
                vad_recording_animation.store(false, Ordering::SeqCst);
                print!("\r"); // 清除动画行
            }
            partial_stt_stop_on_vad.store(true, Ordering::SeqCst);

            // 获取音频数据并处理
            let audio_data = match vad_recorder.stop() {
                Ok(data) => data,
                Err(e) => {
                    error!("VAD 停止录音失败: {}", e);
                    set_status_indicator_state(
                        &status_indicator_on_vad,
                        status_indicator::IndicatorState::Failed,
                    );
                    desktop_notify(desktop_notify_on_vad, "EchoPup", "停止录音失败");
                    return;
                }
            };
            recognition_session_on_vad.lock().clear_partials();
            detach_thread_handle(&partial_stt_handle_on_vad);
            detach_thread_handle(&partial_stt_callback_handle_on_vad);
            play_feedback_sound(sound_feedback_on_vad, FeedbackSoundEvent::RecordingEnd);

            // 处理音频
            let e2e_start = Instant::now();
            set_status_indicator_state(
                &status_indicator_on_vad,
                status_indicator::IndicatorState::Transcribing,
            );
            desktop_notify(desktop_notify_on_vad, "EchoPup", "识别中...");
            let ok = process_audio(
                &audio_data,
                &vad_asr_runtime,
                &vad_llm,
                &vad_post_processor,
                &vad_text_commit,
                &recognition_session_on_vad,
                true,
                e2e_start,
                desktop_notify_on_vad,
            );
            set_status_indicator_state(
                &status_indicator_on_vad,
                if ok {
                    status_indicator::IndicatorState::Completed
                } else {
                    status_indicator::IndicatorState::Failed
                },
            );
        });

        // 配置 VAD 参数并启用
        let silence_threshold = config.audio.vad_silence_threshold_ms as u64;
        recorder.set_vad_params(silence_threshold, 0.01);
        recorder.set_vad_callback(move || {
            vad_callback();
        });
        recorder.enable_vad();
        info!(
            "端点检测参数：持续静音 {} ms 自动结束录音",
            silence_threshold
        );
    } else {
        info!("端点检测已关闭（vad_enabled=false）");
    }

    // ===== 初始化热键监听器 =====
    let mut hotkey = hotkey::HotkeyListener::new()?;
    hotkey.set_hotkey(&config.hotkey.key)?;
    hotkey.on_press(press_callback);
    hotkey.on_release(release_callback);
    hotkey.start()?;

    // ===== 设置 Ctrl+C 信号处理 =====
    let (tx, rx) = mpsc::channel::<()>();
    let tx_clone = tx.clone();
    ctrlc::set_handler(move || {
        let _ = tx_clone.send(());
    })
    .expect("Error setting Ctrl+C handler");

    info!("===========================================");
    info!("🎤 EchoPup 语音输入已启动");
    info!("   热键: {}", config.hotkey.key);
    info!("   模式: {}", config.hotkey.trigger_mode.label());
    info!("   长按 1 秒开始录音");
    match config.hotkey.trigger_mode {
        config::HotkeyTriggerMode::HoldToRecord => {
            info!("   松开热键后停止录音并开始转写");
        }
        config::HotkeyTriggerMode::PressToToggle => {
            info!("   松开后继续录音，下一次按下热键结束并转写");
        }
    }
    info!("   按 Ctrl+C 退出");
    info!("===========================================");

    // ===== 主循环 =====
    let mut shutdown_requested = false;
    loop {
        {
            let mut guard = status_indicator.lock();
            while let Some(action) = guard.try_recv_action() {
                let action_for_runtime = action.clone();
                let mut result = menu_runtime.execute(action);
                if result.ok {
                    if let Err(err) = apply_runtime_menu_action(
                        &action_for_runtime,
                        &result.snapshot,
                        &mut hotkey,
                        &hotkey_trigger_mode,
                        &asr_runtime,
                        &preview_asr_runtime,
                        &llm,
                    ) {
                        warn!("菜单动作热更新失败: {}", err);
                        menu_runtime.set_status(format!("已保存配置，但热更新失败: {}", err));
                        result.snapshot = menu_runtime.snapshot();
                    }
                }
                guard.send_action_result(&result);
                if result.quit_ui {
                    guard.close_ui();
                    shutdown_requested = true;
                    break;
                }
            }
        }

        if shutdown_requested {
            info!("收到菜单退出指令，准备退出主进程");
            break;
        }

        if menu_runtime.poll_download_events() {
            let snapshot = menu_runtime.snapshot();
            let mut guard = status_indicator.lock();
            guard.send_snapshot(&snapshot);
        }

        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(_) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                info!("收到退出信号，正在优雅退出...");
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // 继续运行
            }
        }
    }

    // 停止动画线程
    partial_stt_should_stop.store(true, Ordering::SeqCst);
    join_thread_handle(&partial_stt_handle);
    join_thread_handle(&partial_stt_callback_handle);
    animation_should_stop.store(true, Ordering::SeqCst);
    recording_animation.store(false, Ordering::SeqCst);
    let _ = animation_handle.join();
    set_status_indicator_state(&status_indicator, status_indicator::IndicatorState::Idle);

    info!("EchoPup 已退出");
    Ok(())
}

fn test_modules(config_path: &str) -> Result<()> {
    println!("=== 测试各模块 ===\n");

    println!("[1/4] 测试配置模块...");
    let config = config::Config::load(config_path)?;
    println!("  ✓ 配置加载成功");

    println!("\n[2/4] 测试音频录制器...");
    match audio::AudioRecorder::new(config.audio.sample_rate, config.audio.channels) {
        Ok(_) => println!("  ✓ 音频录制器创建成功"),
        Err(e) => println!("  ✗ 音频录制器创建失败: {}", e),
    }

    println!("\n[3/4] 测试当前 ASR 后端...");
    match build_asr_engine_with_fallback(&config, false) {
        Ok(engine) => {
            let runtime = engine.runtime_info();
            println!(
                "  ✓ ASR 运行时已就绪: backend={}, model={}",
                runtime.backend.label(),
                runtime.model
            );
        }
        Err(e) => println!("  ~ ASR: {}", e),
    }

    println!("\n[4/4] 测试文本提交后端...");
    match commit::InsertOnlyTextCommit::new() {
        Ok(mut backend) => {
            println!("  ✓ 文本提交后端初始化成功");
            println!("    - 后端: {}", backend.backend_name());
            backend.apply(commit::CommitAction::CommitFinal {
                text: "Test".to_string(),
            })?;
            println!("    - 测试文本已输入");
        }
        Err(e) => println!("  ✗ 文本提交后端初始化失败: {}", e),
    }

    println!("\n=== 测试完成 ===");
    Ok(())
}
