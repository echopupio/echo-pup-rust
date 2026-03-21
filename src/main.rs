//! EchoPup - AI Voice Dictation Tool

mod audio;
mod config;
mod hotkey;
mod input;
mod llm;
mod menu_core;
mod model_download;
mod runtime;
mod status_indicator;
mod stt;
mod ui;
mod vad;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use parking_lot::Mutex;
use std::io::IsTerminal;
#[cfg(all(unix, not(target_os = "macos")))]
use std::os::fd::{AsRawFd, RawFd};
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

fn format_partial_status(text: &str) -> String {
    const MAX_CHARS: usize = 48;
    let clean = text.replace('\n', " ").trim().to_string();
    let mut chars = clean.chars();
    let clipped: String = chars.by_ref().take(MAX_CHARS).collect();
    if chars.next().is_some() {
        format!("识别中: {}…", clipped)
    } else {
        format!("识别中: {}", clipped)
    }
}

/// 处理音频数据：转写 -> LLM 整理 -> 谐音纠错 -> 键盘输入
/// is_vad_triggered: 是否由 VAD 自动触发（用于日志区分）
fn process_audio(
    audio_data: &[f32],
    whisper: &Arc<Mutex<Option<stt::WhisperSTT>>>,
    llm: &Arc<Mutex<Option<llm::LLMRewrite>>>,
    post_processor: &Arc<stt::TextPostProcessor>,
    keyboard: &Arc<Mutex<input::Keyboard>>,
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

    // 1. 音频转写 (Whisper)
    // 为避免轻音/尾音被误裁剪，这里关闭“转写前二次 VAD 裁剪”
    let processed_audio = audio_data;
    let mut final_text = String::new();
    let mut transcribe_success = false;
    let stt_start = Instant::now();

    {
        let mut whisper_guard = whisper.lock();
        if let Some(ref mut whisper) = *whisper_guard {
            match whisper.transcribe(processed_audio) {
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
            error!("Whisper 未初始化");
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
            "[{}] 性能埋点: stt_ms={} llm_ms={} postprocess_ms={} type_ms={} e2e_ms={}",
            trigger_type,
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

    // 4. 键盘输入
    let type_start = Instant::now();
    let mut type_success = false;
    {
        let mut keyboard_guard = keyboard.lock();
        match keyboard_guard.type_text(&final_text) {
            Ok(_) => {
                info!("文本已输入");
                type_success = true;
                desktop_notify(
                    desktop_notify_enabled,
                    "EchoPup",
                    &format!("识别完成，已输入 {} 字", final_text.chars().count()),
                );
            }
            Err(e) => {
                error!("键盘输入失败: {}", e);
                desktop_notify(desktop_notify_enabled, "EchoPup", "识别完成，但输入失败");
            }
        }
    }
    type_ms = type_start.elapsed().as_millis();

    info!(
        "[{}] 性能埋点: stt_ms={} llm_ms={} postprocess_ms={} type_ms={} e2e_ms={}",
        trigger_type,
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
        let status = if command_exists("paplay") {
            std::process::Command::new("paplay")
                .arg("/usr/share/sounds/freedesktop/stereo/message.oga")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .context("执行 paplay 失败")?
        } else if command_exists("aplay") {
            std::process::Command::new("aplay")
                .arg("/usr/share/sounds/alsa/Front_Center.wav")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .context("执行 aplay 失败")?
        } else {
            anyhow::bail!("未找到 paplay/aplay");
        };

        if !status.success() {
            anyhow::bail!("提示音命令返回非零状态: {}", status);
        }
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
        Some(Commands::DownloadModel { size }) => {
            info!("下载 Whisper {} 模型", size);
            println!("请运行: ./scripts/download_model.sh {}", size);
            println!("模型目录: ~/.echopup/models");
        }
        Some(Commands::StatusIndicator) => status_indicator::run_status_indicator_process()?,
        None => start_background_mode(&cli.config)?,
    }

    Ok(())
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

fn build_whisper_stt(whisper_cfg: &config::config::WhisperConfig) -> Result<stt::WhisperSTT> {
    let mut w = stt::WhisperSTT::with_options(
        &whisper_cfg.model_path,
        whisper_cfg.language.clone(),
        whisper_cfg.translate,
    )?;

    let strategy = match whisper_cfg.decoding_strategy.clone() {
        config::WhisperDecodingStrategy::Greedy => stt::DecodingStrategy::Greedy {
            best_of: whisper_cfg.greedy_best_of,
        },
        config::WhisperDecodingStrategy::BeamSearch => stt::DecodingStrategy::BeamSearch {
            beam_size: whisper_cfg.beam_size,
        },
    };
    w.set_decoding_strategy(strategy);
    w.set_temperature(whisper_cfg.temperature);
    w.set_no_context(whisper_cfg.no_context);
    w.set_suppress_nst(whisper_cfg.suppress_nst);
    w.set_n_threads(whisper_cfg.resolved_n_threads());
    w.set_initial_prompt(whisper_cfg.initial_prompt.clone());
    w.set_hotwords(whisper_cfg.hotwords.clone());
    Ok(w)
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
    whisper_runtime: &Arc<Mutex<Option<stt::WhisperSTT>>>,
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
            let whisper_cfg = cfg.whisper.effective();
            match build_whisper_stt(&whisper_cfg) {
                Ok(new_whisper) => {
                    let mut guard = whisper_runtime.lock();
                    *guard = Some(new_whisper);
                    info!("Whisper 模型已热更新: {}", whisper_cfg.model_path);
                }
                Err(err) => {
                    warn!("Whisper 热更新失败: {}", err);
                }
            }
        }
        menu_core::MenuAction::SetHotkeyTriggerMode { mode } => {
            *hotkey_trigger_mode_runtime.lock() = *mode;
            info!("热键触发模式已热更新为 {}", mode.label());
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

            let whisper_cfg = cfg.whisper.effective();
            match build_whisper_stt(&whisper_cfg) {
                Ok(new_whisper) => {
                    let mut guard = whisper_runtime.lock();
                    *guard = Some(new_whisper);
                    info!("Whisper 配置已重载: {}", whisper_cfg.model_path);
                }
                Err(err) => {
                    warn!("Whisper 重载失败: {}", err);
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

    let whisper_cfg = config.whisper.effective();

    // ===== 初始化模块 =====
    let recorder = Arc::new(audio::AudioRecorder::new(
        config.audio.sample_rate,
        config.audio.channels,
    )?);
    info!("音频录制器已初始化");

    if let Some(profile) = config.whisper.performance_profile {
        info!(
            "Whisper 性能档位: {:?}，模型: {}，策略: {:?}",
            profile, whisper_cfg.model_path, whisper_cfg.decoding_strategy
        );
    }

    let whisper = match build_whisper_stt(&whisper_cfg) {
        Ok(w) => {
            info!(
                "Whisper 线程数: {} (配置: {:?})",
                whisper_cfg.resolved_n_threads(),
                whisper_cfg.n_threads
            );
            info!("Whisper 已初始化");
            Some(w)
        }
        Err(e) => {
            warn!("Whisper 初始化失败: {}，语音转写功能不可用", e);
            None
        }
    };
    // 使用 Mutex 包装，以便在回调中共享（transcribe 需要 &mut self）
    let whisper = Arc::new(Mutex::new(whisper));

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

    let keyboard = Arc::new(Mutex::new(input::Keyboard::new()?));
    info!("键盘输入已初始化");

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
    let partial_callback_latest_text = Arc::new(Mutex::new(String::new()));

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
    let whisper_for_partial_start = whisper.clone();
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
    let partial_callback_latest_text_on_start = partial_callback_latest_text.clone();
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
                    if let Some(old_handle) = partial_stt_handle_on_start.lock().take() {
                        let _ = old_handle.join();
                    }

                    let recorder_partial = recorder_start.clone();
                    let whisper_partial = whisper_for_partial_start.clone();
                    let status_indicator_partial = status_indicator_on_start.clone();
                    let menu_snapshot_partial = menu_snapshot_on_start.clone();
                    let is_recording_partial = is_recording_start.clone();
                    let partial_stt_stop = partial_stt_stop_on_start.clone();
                    let handle = std::thread::spawn(move || {
                        let poll_interval = Duration::from_millis(500);
                        let min_samples = (recorder_partial.target_sample_rate() as usize)
                            .saturating_mul(800)
                            / 1000;
                        let mut last_snapshot_len = 0usize;
                        let mut last_text = String::new();

                        while is_recording_partial.load(Ordering::SeqCst)
                            && !partial_stt_stop.load(Ordering::SeqCst)
                        {
                            std::thread::sleep(poll_interval);
                            if !is_recording_partial.load(Ordering::SeqCst)
                                || partial_stt_stop.load(Ordering::SeqCst)
                            {
                                break;
                            }

                            let snapshot = recorder_partial.get_snapshot();
                            if snapshot.len() < min_samples || snapshot.len() <= last_snapshot_len {
                                continue;
                            }
                            last_snapshot_len = snapshot.len();

                            let partial_text = {
                                let mut whisper_guard = whisper_partial.lock();
                                if let Some(ref mut whisper) = *whisper_guard {
                                    whisper.transcribe_incremental(&snapshot).ok()
                                } else {
                                    None
                                }
                            };

                            let Some(partial_text) = partial_text else {
                                continue;
                            };
                            let trimmed = partial_text.trim();
                            if trimmed.is_empty() || trimmed == "[BLANK_AUDIO]" {
                                continue;
                            }
                            if trimmed == last_text {
                                continue;
                            }

                            last_text = trimmed.to_string();
                            send_status_snapshot(
                                &status_indicator_partial,
                                &menu_snapshot_partial,
                                format_partial_status(trimmed),
                            );
                        }
                    });
                    *partial_stt_handle_on_start.lock() = Some(handle);
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
    let whisper_stop = whisper.clone();
    let llm_stop = llm.clone();
    let post_processor_stop = post_processor.clone();
    let keyboard_stop = keyboard.clone();
    let is_recording_stop = is_recording.clone();
    let vad_triggered_stop = vad_triggered.clone();
    let recording_animation_stop = recording_animation.clone();
    let desktop_notify_on_stop = desktop_notify_enabled;
    let sound_feedback_on_stop = sound_feedback_enabled;
    let status_indicator_on_stop = status_indicator.clone();
    let stop_debounce_on_stop = stop_debounce_until.clone();
    let stop_recording_action: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        if is_recording_stop.load(Ordering::SeqCst) {
            *stop_debounce_on_stop.lock() = None;
            is_recording_stop.store(false, Ordering::SeqCst);
            recording_animation_stop.store(false, Ordering::SeqCst);
            print!("\r");
            clear_terminal_artifacts_if_tty();

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
                &whisper_stop,
                &llm_stop,
                &post_processor_stop,
                &keyboard_stop,
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
        let started_by_hold = {
            let mut state = press_state_on_release.lock();
            if !state.pressed {
                return;
            }
            state.pressed = false;
            state.sequence = state.sequence.wrapping_add(1);
            let started = state.started_by_hold_on_current_press;
            state.started_by_hold_on_current_press = false;
            started
        };

        if *mode_on_release.lock() == config::HotkeyTriggerMode::HoldToRecord
            && started_by_hold
            && is_recording_on_release.load(Ordering::SeqCst)
        {
            stop_action_on_release();
        }
    });

    // ===== 设置端点检测（VAD）回调 =====
    // 根据配置决定是否启用 VAD
    let vad_enabled = config.audio.vad_enabled;

    if vad_enabled {
        info!("端点检测已启用");

        let vad_recorder = recorder.clone();
        let vad_whisper = whisper.clone();
        let vad_llm = llm.clone();
        let vad_post_processor = post_processor.clone();
        let vad_keyboard = keyboard.clone();
        let vad_is_recording = is_recording.clone();
        let vad_triggered_callback = vad_triggered.clone();
        let vad_recording_animation = recording_animation.clone();
        let desktop_notify_on_vad = desktop_notify_enabled;
        let sound_feedback_on_vad = sound_feedback_enabled;
        let status_indicator_on_vad = status_indicator.clone();

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
                &vad_whisper,
                &vad_llm,
                &vad_post_processor,
                &vad_keyboard,
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
                        &whisper,
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

    println!("\n[3/4] 测试 Whisper...");
    let whisper_cfg = config.whisper.effective();
    match stt::WhisperSTT::new(&whisper_cfg.model_path) {
        Ok(w) => {
            if w.is_ready() {
                println!("  ✓ Whisper 模型加载成功");
            } else {
                println!("  ~ Whisper 模型未找到");
            }
        }
        Err(e) => println!("  ~ Whisper: {}", e),
    }

    println!("\n[4/4] 测试键盘输入...");
    match input::Keyboard::new() {
        Ok(mut k) => {
            println!("  ✓ 键盘输入初始化成功");
            k.type_text("Test")?;
            println!("    - 测试文本已输入");
        }
        Err(e) => println!("  ✗ 键盘输入初始化失败: {}", e),
    }

    println!("\n=== 测试完成 ===");
    Ok(())
}
