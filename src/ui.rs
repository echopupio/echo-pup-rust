//! 终端管理界面（TUI）

use crate::config::Config;
use anyhow::{anyhow, Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Terminal;
use std::io::Stdout;
use std::process::Command;
use std::time::Duration;

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

#[derive(Clone, Copy)]
enum InputTarget {
    Hotkey,
    LlmProvider,
    LlmModel,
    LlmApiBase,
    LlmApiKeyEnv,
    WhisperModelPath,
}

struct AppState {
    config: Config,
    config_path: String,
    selected: usize,
    status: String,
    local_models: Vec<String>,
    input_target: Option<InputTarget>,
    input_buffer: String,
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
            should_quit: false,
        })
    }

    fn selected_label(&self) -> &str {
        MENU_ITEMS[self.selected]
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
            Constraint::Length(4),
        ])
        .split(frame.area());

    let title = Paragraph::new("CatEcho 管理界面 (catecho ui)")
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

    let summary = format!(
        "当前配置\n\nhotkey: {}\nllm.enabled: {}\ntext_correction.enabled: {}\naudio.vad_enabled: {}\nllm.provider: {}\nllm.model: {}\nllm.api_base: {}\nllm.api_key_env: {}\nwhisper.model_path: {}\n\n本地模型:\n{}",
        app.config.hotkey.key,
        app.config.llm.enabled,
        app.config.text_correction.enabled,
        app.config.audio.vad_enabled,
        app.config.llm.provider,
        app.config.llm.model,
        app.config.llm.api_base,
        app.config.llm.api_key_env,
        app.config.whisper.model_path,
        models
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
    let bottom =
        Paragraph::new(bottom_text).block(Block::default().borders(Borders::ALL).title("状态"));
    frame.render_widget(bottom, chunks[2]);
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
        9 => download_whisper_model(app, "large-v3")?,
        10 => download_whisper_model(app, "turbo")?,
        11 => download_whisper_model(app, "medium")?,
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
    if let Ok(cwd) = std::env::current_dir() {
        let model_dir = cwd.join("models");
        if let Ok(entries) = std::fs::read_dir(model_dir) {
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

fn download_whisper_model(app: &mut AppState, model_size: &str) -> Result<()> {
    app.status = format!("正在下载模型 {} ...", model_size);

    let cwd = std::env::current_dir().context("获取当前目录失败")?;
    let script = cwd.join("scripts").join("download_model.sh");
    if !script.exists() {
        return Err(anyhow!("下载脚本不存在: {}", script.display()));
    }

    let output = Command::new("bash")
        .arg(script)
        .arg(model_size)
        .output()
        .context("执行模型下载脚本失败")?;

    if output.status.success() {
        app.local_models = list_local_models();
        app.status = format!("模型 {} 下载完成", model_size);
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        app.status = format!("下载失败: {}", stderr.trim());
        Ok(())
    }
}
