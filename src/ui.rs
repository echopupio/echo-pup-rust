//! 终端管理界面（TUI）

use crate::hotkey;
use crate::menu_core::{
    model_size_from_file_name, whisper_model_path_from_file_name, EditableField, MenuAction,
    MenuCore, DOWNLOAD_MODEL_SIZES, MENU_ITEMS,
};
use crate::model_download;
use anyhow::{anyhow, Context, Result};
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
    ModifierKeyCode, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Terminal;
use std::io::Stdout;
use std::time::Duration;

struct AppState {
    menu: MenuCore,
    selected: usize,
    input_target: Option<EditableField>,
    input_buffer: String,
    llm_form: Option<LlmFormDraft>,
    should_quit: bool,
}

#[derive(Debug, Clone)]
struct LlmFormDraft {
    provider: String,
    model: String,
    api_base: String,
    api_key_env: String,
    step: usize,
}

impl LlmFormDraft {
    fn current_field(&self) -> EditableField {
        match self.step {
            0 => EditableField::LlmProvider,
            1 => EditableField::LlmModel,
            2 => EditableField::LlmApiBase,
            _ => EditableField::LlmApiKeyEnv,
        }
    }

    fn current_value(&self) -> String {
        match self.current_field() {
            EditableField::LlmProvider => self.provider.clone(),
            EditableField::LlmModel => self.model.clone(),
            EditableField::LlmApiBase => self.api_base.clone(),
            EditableField::LlmApiKeyEnv => self.api_key_env.clone(),
            _ => String::new(),
        }
    }

    fn set_current_value(&mut self, value: String) {
        match self.current_field() {
            EditableField::LlmProvider => self.provider = value,
            EditableField::LlmModel => self.model = value,
            EditableField::LlmApiBase => self.api_base = value,
            EditableField::LlmApiKeyEnv => self.api_key_env = value,
            _ => {}
        }
    }

    fn current_label(&self) -> &'static str {
        match self.current_field() {
            EditableField::LlmProvider => "llm.provider",
            EditableField::LlmModel => "llm.model",
            EditableField::LlmApiBase => "llm.api_base",
            EditableField::LlmApiKeyEnv => "llm.api_key_env",
            _ => "llm",
        }
    }
}

impl AppState {
    fn new(config_path: &str) -> Result<Self> {
        Ok(Self {
            menu: MenuCore::new(config_path)?,
            selected: 0,
            input_target: None,
            input_buffer: String::new(),
            llm_form: None,
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
    let _ = stdout.execute(PushKeyboardEnhancementFlags(
        KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
            | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
            | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES,
    ));

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("初始化 TUI 终端失败")?;

    let run_result = run_app(&mut terminal, &mut app);

    disable_raw_mode().ok();
    terminal
        .backend_mut()
        .execute(PopKeyboardEnhancementFlags)
        .ok();
    terminal.backend_mut().execute(LeaveAlternateScreen).ok();
    terminal.show_cursor().ok();

    run_result
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut AppState) -> Result<()> {
    while !app.should_quit {
        let _ = app.menu.poll_download_events();
        if app.menu.should_quit_ui() {
            app.should_quit = true;
        }

        terminal
            .draw(|f| draw_ui(f, app))
            .context("绘制 TUI 失败")?;

        if event::poll(Duration::from_millis(200)).context("读取事件失败")? {
            if let Event::Key(key) = event::read().context("读取按键失败")? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                if app.input_target.is_some() {
                    handle_input_mode_event(app, key)?;
                } else {
                    handle_menu_mode_key(app, key.code)?;
                }
            }
        }
    }

    Ok(())
}

fn draw_ui(frame: &mut ratatui::Frame, app: &AppState) {
    let snapshot = app.menu.snapshot();

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

    let models = if snapshot.local_models.is_empty() {
        "（未发现本地模型）".to_string()
    } else {
        snapshot.local_models.join("\n")
    };
    let download_logs = if snapshot.download_logs.is_empty() {
        "（暂无下载日志）".to_string()
    } else {
        snapshot
            .download_logs
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
        "当前配置\n\nhotkey: {}\nllm.enabled: {}\ntext_correction.enabled: {}\naudio.vad_enabled: {}\nllm.provider: {}\nllm.model: {}\nllm.api_base: {}\nllm.api_key_env: {}\nwhisper.model_path: {}\ndirty: {}\n\n本地模型:\n{}\n\n下载日志:\n{}",
        snapshot.hotkey,
        snapshot.llm_enabled,
        snapshot.text_correction_enabled,
        snapshot.vad_enabled,
        snapshot.llm_provider,
        snapshot.llm_model,
        snapshot.llm_api_base,
        snapshot.llm_api_key_env,
        snapshot.whisper_model_path,
        snapshot.dirty,
        models,
        download_logs
    );
    let detail = Paragraph::new(summary)
        .block(Block::default().borders(Borders::ALL).title("详情"))
        .wrap(Wrap { trim: true });
    frame.render_widget(detail, body_chunks[1]);

    let bottom_text = if let Some(target) = app.input_target {
        if target == EditableField::Hotkey {
            let captured = if app.input_buffer.is_empty() {
                "（未捕获）".to_string()
            } else {
                app.input_buffer.clone()
            };
            format!(
                "正在编辑 {}\n已捕获: {}\n按目标组合键进行捕获，Enter 保存，Esc 取消，Backspace 清空",
                input_target_label(target),
                captured
            )
        } else {
            format!(
                "正在编辑 {}\n当前输入: {}\nEnter 保存，Esc 取消",
                input_target_label(target),
                app.input_buffer
            )
        }
    } else {
        format!(
            "状态: {}\n当前选中: {}",
            snapshot.status,
            app.selected_label()
        )
    };

    if let Some(download) = snapshot.download.as_ref() {
        let bottom_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Length(3)])
            .split(chunks[2]);

        let status =
            Paragraph::new(bottom_text).block(Block::default().borders(Borders::ALL).title("状态"));
        frame.render_widget(status, bottom_chunks[0]);

        let (ratio, label) = model_download::download_ratio_label(download);
        let gauge = Gauge::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!("下载进度 ({})", download.model_size)),
            )
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
        KeyCode::Enter => execute_menu_action(app),
        _ => {}
    }
    Ok(())
}

fn handle_input_mode_event(app: &mut AppState, key: KeyEvent) -> Result<()> {
    if app.input_target == Some(EditableField::Hotkey) {
        handle_hotkey_capture_event(app, key)
    } else {
        handle_input_mode_key(app, key.code)
    }
}

fn handle_input_mode_key(app: &mut AppState, code: KeyCode) -> Result<()> {
    match code {
        KeyCode::Esc => {
            app.llm_form = None;
            app.input_target = None;
            app.input_buffer.clear();
            app.menu.set_status("已取消编辑");
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

fn handle_hotkey_capture_event(app: &mut AppState, key: KeyEvent) -> Result<()> {
    match key.code {
        KeyCode::Esc if key.modifiers.is_empty() => {
            app.input_target = None;
            app.input_buffer.clear();
            app.menu.set_status("已取消热键编辑");
            return Ok(());
        }
        KeyCode::Enter if key.modifiers.is_empty() => {
            return apply_input(app);
        }
        KeyCode::Backspace if key.modifiers.is_empty() => {
            app.input_buffer.clear();
            app.menu.set_status("已清空捕获热键");
            return Ok(());
        }
        _ => {}
    }

    let Some(value) = key_event_to_hotkey(&key) else {
        app.menu.set_status("该按键暂不支持作为热键，请换一个");
        return Ok(());
    };

    let total_keys = hotkey_key_count(&value);
    if total_keys > 3 {
        app.menu
            .set_status(format!("热键最多支持 3 个键，当前为 {} 个", total_keys));
        return Ok(());
    }

    if let Err(err) = hotkey::validate_hotkey_config(&value) {
        app.menu.set_status(format!("热键不安全: {}", err));
        return Ok(());
    }

    app.input_buffer = value.clone();
    app.menu
        .set_status(format!("已捕获热键: {}（Enter 保存）", value));
    Ok(())
}

fn hotkey_key_count(value: &str) -> usize {
    value.split('+').filter(|s| !s.is_empty()).count()
}

fn key_event_to_hotkey(key: &KeyEvent) -> Option<String> {
    if let KeyCode::Modifier(modifier) = key.code {
        return modifier_only_hotkey(modifier).map(ToOwned::to_owned);
    }

    let mut parts: Vec<String> = Vec::new();

    if key.modifiers.contains(KeyModifiers::CONTROL) {
        parts.push("ctrl".to_string());
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        parts.push("alt".to_string());
    }
    if key.modifiers.contains(KeyModifiers::SHIFT) {
        parts.push("shift".to_string());
    }
    if key.modifiers.contains(KeyModifiers::SUPER) {
        parts.push("super".to_string());
    }

    if let Some(main) = key_code_to_hotkey_key(key.code) {
        parts.push(main);
        return Some(parts.join("+"));
    }

    None
}

fn modifier_only_hotkey(modifier: ModifierKeyCode) -> Option<&'static str> {
    match modifier {
        ModifierKeyCode::RightControl => Some("right_ctrl"),
        _ => None,
    }
}

fn key_code_to_hotkey_key(code: KeyCode) -> Option<String> {
    let key = match code {
        KeyCode::Char(c) => return char_to_hotkey_key(c),
        KeyCode::F(n) => return Some(format!("f{}", n)),
        KeyCode::Enter => "enter".to_string(),
        KeyCode::Tab | KeyCode::BackTab => "tab".to_string(),
        KeyCode::Backspace => "backspace".to_string(),
        KeyCode::Delete => "delete".to_string(),
        KeyCode::Insert => "insert".to_string(),
        KeyCode::Home => "home".to_string(),
        KeyCode::End => "end".to_string(),
        KeyCode::PageUp => "pageup".to_string(),
        KeyCode::PageDown => "pagedown".to_string(),
        KeyCode::Left => "left".to_string(),
        KeyCode::Right => "right".to_string(),
        KeyCode::Up => "up".to_string(),
        KeyCode::Down => "down".to_string(),
        KeyCode::Esc => "esc".to_string(),
        KeyCode::CapsLock => "capslock".to_string(),
        KeyCode::ScrollLock => "scrolllock".to_string(),
        KeyCode::NumLock => "numlock".to_string(),
        KeyCode::PrintScreen => "printscreen".to_string(),
        KeyCode::Pause => "pause".to_string(),
        _ => return None,
    };
    Some(key)
}

fn char_to_hotkey_key(c: char) -> Option<String> {
    let key = match c {
        'a'..='z' | '0'..='9' => c.to_string(),
        'A'..='Z' => c.to_ascii_lowercase().to_string(),
        ' ' => "space".to_string(),
        '+' | '=' => "equal".to_string(),
        '-' | '_' => "minus".to_string(),
        ',' | '<' => "comma".to_string(),
        '.' | '>' => "period".to_string(),
        ';' | ':' => "semicolon".to_string(),
        '/' | '?' => "slash".to_string(),
        '\'' | '"' => "quote".to_string(),
        '[' | '{' => "bracketleft".to_string(),
        ']' | '}' => "bracketright".to_string(),
        '\\' | '|' => "backslash".to_string(),
        '`' | '~' => "backquote".to_string(),
        _ => return None,
    };
    Some(key)
}

fn execute_menu_action(app: &mut AppState) {
    match app.selected {
        0 => {
            app.menu.execute(MenuAction::ToggleLlmEnabled);
        }
        1 => {
            app.menu.execute(MenuAction::ToggleTextCorrectionEnabled);
        }
        2 => {
            app.menu.execute(MenuAction::ToggleVadEnabled);
        }
        3 => start_input(app, EditableField::Hotkey),
        4 => start_llm_form(app),
        5 => switch_whisper_model(app),
        6 => start_download_model(app),
        7 => {
            let result = app.menu.execute(MenuAction::QuitUi);
            if result.quit_ui {
                app.should_quit = true;
            }
        }
        _ => {}
    }
}

fn start_input(app: &mut AppState, target: EditableField) {
    app.input_target = Some(target);
    app.input_buffer = app.menu.current_value(target);
    if target == EditableField::Hotkey {
        app.menu.set_status(format!(
            "请按下热键组合进行捕获（最多3键）。{}",
            hotkey::hotkey_policy_hint()
        ));
    } else {
        app.menu
            .set_status(format!("编辑 {}", input_target_label(target)));
    }
}

fn start_llm_form(app: &mut AppState) {
    let snapshot = app.menu.snapshot();
    let form = LlmFormDraft {
        provider: snapshot.llm_provider,
        model: snapshot.llm_model,
        api_base: snapshot.llm_api_base,
        api_key_env: snapshot.llm_api_key_env,
        step: 0,
    };
    app.llm_form = Some(form.clone());
    app.input_target = Some(form.current_field());
    app.input_buffer = form.current_value();
    app.menu
        .set_status("LLM 配置表单（1/4）：编辑 llm.provider");
}

fn switch_whisper_model(app: &mut AppState) {
    let snapshot = app.menu.snapshot();
    let current_file = std::path::Path::new(&snapshot.whisper_model_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let mut options = snapshot
        .local_models
        .into_iter()
        .filter(|name| name.ends_with(".bin"))
        .collect::<Vec<_>>();
    options.sort();
    options.dedup();
    if options.is_empty() {
        app.menu.set_status("未发现已下载的 Whisper 模型");
        return;
    }

    let current_index = options.iter().position(|m| m == current_file).unwrap_or(0);
    let next_file = options[(current_index + 1) % options.len()].clone();
    match whisper_model_path_from_file_name(&next_file) {
        Ok(model_path) => {
            app.menu
                .execute(MenuAction::SwitchWhisperModel { model_path });
        }
        Err(err) => {
            app.menu
                .set_status(format!("切换 Whisper 模型失败: {}", err));
        }
    }
}

fn start_download_model(app: &mut AppState) {
    let snapshot = app.menu.snapshot();
    let current_file = std::path::Path::new(&snapshot.whisper_model_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let preferred = model_size_from_file_name(current_file)
        .unwrap_or(DOWNLOAD_MODEL_SIZES[0])
        .to_string();
    app.menu
        .execute(MenuAction::DownloadModel { size: preferred });
}

fn apply_input(app: &mut AppState) -> Result<()> {
    let target = app
        .input_target
        .ok_or_else(|| anyhow!("当前不在输入模式"))?;
    let value = app.input_buffer.trim().to_string();

    if let Some(form) = app.llm_form.as_mut() {
        if value.is_empty() && target != EditableField::LlmApiKeyEnv {
            app.menu
                .set_status(format!("{} 不能为空", form.current_label()));
            return Ok(());
        }

        form.set_current_value(value);
        if form.step < 3 {
            form.step += 1;
            app.input_target = Some(form.current_field());
            app.input_buffer = form.current_value();
            app.menu.set_status(format!(
                "LLM 配置表单（{}/4）：编辑 {}",
                form.step + 1,
                form.current_label()
            ));
            return Ok(());
        }

        let result = app.menu.execute(MenuAction::SetLlmConfig {
            provider: form.provider.clone(),
            model: form.model.clone(),
            api_base: form.api_base.clone(),
            api_key_env: form.api_key_env.clone(),
        });
        if result.ok {
            app.llm_form = None;
            app.input_target = None;
            app.input_buffer.clear();
        }
        return Ok(());
    }

    let result = app.menu.execute(MenuAction::SetField {
        field: target,
        value,
    });
    if result.ok {
        app.input_target = None;
        app.input_buffer.clear();
    }
    Ok(())
}

fn input_target_label(target: EditableField) -> &'static str {
    match target {
        EditableField::Hotkey => "hotkey",
        EditableField::LlmProvider => "llm.provider",
        EditableField::LlmModel => "llm.model",
        EditableField::LlmApiBase => "llm.api_base",
        EditableField::LlmApiKeyEnv => "llm.api_key_env",
        EditableField::WhisperModelPath => "whisper.model_path",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, ModifierKeyCode};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_config_path() -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir()
            .join(format!("echopup-ui-test-{}.toml", nanos))
            .display()
            .to_string()
    }

    fn make_app() -> AppState {
        AppState {
            menu: MenuCore::new(&temp_config_path()).unwrap(),
            selected: 0,
            input_target: None,
            input_buffer: String::new(),
            llm_form: None,
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
        app.llm_form = Some(LlmFormDraft {
            provider: "openai".to_string(),
            model: "gpt-4o-mini".to_string(),
            api_base: "https://api.openai.com/v1".to_string(),
            api_key_env: "OPENAI_API_KEY".to_string(),
            step: 1,
        });
        app.input_target = Some(EditableField::LlmModel);
        app.input_buffer = "  gpt-test-model  ".to_string();

        apply_input(&mut app).unwrap();
        assert_eq!(app.input_target, Some(EditableField::LlmApiBase));
        assert_eq!(app.input_buffer, "https://api.openai.com/v1");
    }

    #[test]
    fn test_apply_input_rejects_empty_value() {
        let mut app = make_app();
        app.llm_form = Some(LlmFormDraft {
            provider: "openai".to_string(),
            model: "gpt-4o-mini".to_string(),
            api_base: "https://api.openai.com/v1".to_string(),
            api_key_env: "OPENAI_API_KEY".to_string(),
            step: 0,
        });
        app.input_target = Some(EditableField::LlmProvider);
        app.input_buffer = "   ".to_string();

        apply_input(&mut app).unwrap();

        assert_eq!(app.menu.current_value(EditableField::LlmProvider), "openai");
        assert!(app.input_target.is_some());
    }

    #[test]
    fn test_execute_menu_toggle_switches() {
        let mut app = make_app();
        let before = app.menu.snapshot();

        app.selected = 0;
        execute_menu_action(&mut app);
        let s1 = app.menu.snapshot();
        assert_ne!(s1.llm_enabled, before.llm_enabled);

        app.selected = 1;
        execute_menu_action(&mut app);
        let s2 = app.menu.snapshot();
        assert_ne!(s2.text_correction_enabled, s1.text_correction_enabled);

        app.selected = 2;
        execute_menu_action(&mut app);
        let s3 = app.menu.snapshot();
        assert_ne!(s3.vad_enabled, s2.vad_enabled);
    }

    #[test]
    fn test_key_event_to_hotkey_basic_combos() {
        let ctrl_space = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::CONTROL);
        assert_eq!(
            key_event_to_hotkey(&ctrl_space).as_deref(),
            Some("ctrl+space")
        );

        let ctrl_shift_a = KeyEvent::new(
            KeyCode::Char('A'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        assert_eq!(
            key_event_to_hotkey(&ctrl_shift_a).as_deref(),
            Some("ctrl+shift+a")
        );
    }

    #[test]
    fn test_key_event_to_hotkey_modifier_only() {
        let right_ctrl = KeyEvent::new(
            KeyCode::Modifier(ModifierKeyCode::RightControl),
            KeyModifiers::empty(),
        );
        assert_eq!(
            key_event_to_hotkey(&right_ctrl).as_deref(),
            Some("right_ctrl")
        );
    }

    #[test]
    fn test_hotkey_capture_limit_max_three_keys() {
        let mut app = make_app();
        app.input_target = Some(EditableField::Hotkey);
        app.input_buffer = "ctrl+space".to_string();

        let too_many = KeyEvent::new(
            KeyCode::Char('k'),
            KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT,
        );

        handle_hotkey_capture_event(&mut app, too_many).unwrap();

        assert_eq!(app.input_buffer, "ctrl+space");
    }

    #[test]
    fn test_hotkey_capture_rejects_unsafe_single_key() {
        let mut app = make_app();
        app.input_target = Some(EditableField::Hotkey);
        app.input_buffer = app.menu.current_value(EditableField::Hotkey);

        let unsafe_key = KeyEvent::new(KeyCode::Char('z'), KeyModifiers::empty());
        handle_hotkey_capture_event(&mut app, unsafe_key).unwrap();

        assert_eq!(
            app.input_buffer,
            app.menu.current_value(EditableField::Hotkey)
        );
    }

    #[test]
    fn test_apply_input_rejects_unsafe_hotkey() {
        let mut app = make_app();
        app.input_target = Some(EditableField::Hotkey);
        app.input_buffer = "z".to_string();

        apply_input(&mut app).unwrap();

        assert_eq!(
            app.menu.current_value(EditableField::Hotkey),
            MenuCore::new("/tmp/echopup-ui-test-default.toml")
                .unwrap()
                .current_value(EditableField::Hotkey)
        );
        assert!(app.input_target.is_some());
    }
}
