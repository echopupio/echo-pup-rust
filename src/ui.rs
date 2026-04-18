//! 终端管理界面（TUI）

use crate::menu_core::{EditableField, MenuAction, MenuCore, MENU_ITEMS};
use crate::model_download;
use anyhow::{anyhow, Context, Result};
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
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
        }
    }

    fn set_current_value(&mut self, value: String) {
        match self.current_field() {
            EditableField::LlmProvider => self.provider = value,
            EditableField::LlmModel => self.model = value,
            EditableField::LlmApiBase => self.api_base = value,
            EditableField::LlmApiKeyEnv => self.api_key_env = value,
        }
    }

    fn current_label(&self) -> &'static str {
        match self.current_field() {
            EditableField::LlmProvider => "llm.provider",
            EditableField::LlmModel => "llm.model",
            EditableField::LlmApiBase => "llm.api_base",
            EditableField::LlmApiKeyEnv => "llm.api_key_env",
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
        "当前配置\n\nhotkey: ctrl (固定)\nllm.enabled: {}\ntext_correction.enabled: {}\nllm.provider: {}\nllm.model: {}\nllm.api_base: {}\nllm.api_key_env: {}\ndirty: {}\n\n本地模型:\n{}\n\n下载日志:\n{}",
        snapshot.llm_enabled,
        snapshot.text_correction_enabled,
        snapshot.llm_provider,
        snapshot.llm_model,
        snapshot.llm_api_base,
        snapshot.llm_api_key_env,
        snapshot.dirty,
        models,
        download_logs
    );
    let detail = Paragraph::new(summary)
        .block(Block::default().borders(Borders::ALL).title("详情"))
        .wrap(Wrap { trim: true });
    frame.render_widget(detail, body_chunks[1]);

    let bottom_text = if let Some(target) = app.input_target {
        format!(
            "正在编辑 {}\n当前输入: {}\nEnter 保存，Esc 取消",
            input_target_label(target),
            app.input_buffer
        )
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
    handle_input_mode_key(app, key.code)
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

fn execute_menu_action(app: &mut AppState) {
    match app.selected {
        0 => {
            app.menu.execute(MenuAction::ToggleLlmEnabled);
        }
        1 => {
            app.menu.execute(MenuAction::ToggleTextCorrectionEnabled);
        }
        2 => start_llm_form(app),
        3 => {
            app.menu.execute(MenuAction::DownloadModel);
        }
        4 => {
            let result = app.menu.execute(MenuAction::QuitUi);
            if result.quit_ui {
                app.should_quit = true;
            }
        }
        _ => {}
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
        EditableField::LlmProvider => "llm.provider",
        EditableField::LlmModel => "llm.model",
        EditableField::LlmApiBase => "llm.api_base",
        EditableField::LlmApiKeyEnv => "llm.api_key_env",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyCode;
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
    }
}
