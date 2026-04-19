//! 共享菜单业务内核（TUI / 状态栏）

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::config::HotkeyTriggerMode;

#[allow(dead_code)]
pub const MENU_ITEMS: [&str; 4] = ["启用 LLM 润色", "启用文本纠错", "编辑 LLM 配置", "退出"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditableField {
    LlmProvider,
    LlmModel,
    LlmApiBase,
    LlmApiKey,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MenuAction {
    ToggleLlmEnabled,
    ToggleTextCorrectionEnabled,
    OpenConfigFolder,
    OpenModelFolder,
    SetField {
        field: EditableField,
        value: String,
    },
    SetLlmConfig {
        provider: String,
        model: String,
        api_base: String,
        api_key: String,
    },
    SetHotkeyTriggerMode {
        mode: HotkeyTriggerMode,
    },
    ReloadConfig,
    QuitUi,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MenuSnapshot {
    pub config_path: String,
    pub status: String,
    pub dirty: bool,
    pub should_quit_ui: bool,

    pub hotkey_trigger_mode: HotkeyTriggerMode,
    pub llm_enabled: bool,
    pub text_correction_enabled: bool,
    pub llm_provider: String,
    pub llm_model: String,
    pub llm_api_base: String,
    pub llm_api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MenuActionResult {
    pub ok: bool,
    pub message: String,
    pub quit_ui: bool,
    pub snapshot: MenuSnapshot,
}

pub struct MenuCore {
    config: Config,
    config_path: String,
    status: String,
    dirty: bool,
    should_quit_ui: bool,
}

impl MenuCore {
    pub fn new(config_path: &str) -> Result<Self> {
        let config = Config::load(config_path)?;
        Ok(Self {
            config,
            config_path: config_path.to_string(),
            status: "就绪。方向键选择，Enter执行，q退出。".to_string(),
            dirty: false,
            should_quit_ui: false,
        })
    }

    pub fn snapshot(&self) -> MenuSnapshot {
        MenuSnapshot {
            config_path: self.config_path.clone(),
            status: self.status.clone(),
            dirty: self.dirty,
            should_quit_ui: self.should_quit_ui,

            hotkey_trigger_mode: self.config.hotkey.trigger_mode,
            llm_enabled: self.config.llm.enabled,
            text_correction_enabled: self.config.text_correction.enabled,
            llm_provider: self.config.llm.provider.clone(),
            llm_model: self.config.llm.model.clone(),
            llm_api_base: self.config.llm.api_base.clone(),
            llm_api_key: self.config.llm.api_key.clone(),
        }
    }

    #[allow(dead_code)]
    pub fn current_value(&self, field: EditableField) -> String {
        match field {
            EditableField::LlmProvider => self.config.llm.provider.clone(),
            EditableField::LlmModel => self.config.llm.model.clone(),
            EditableField::LlmApiBase => self.config.llm.api_base.clone(),
            EditableField::LlmApiKey => self.config.llm.api_key.clone(),
        }
    }

    #[allow(dead_code)]
    pub fn should_quit_ui(&self) -> bool {
        self.should_quit_ui
    }

    pub fn set_status(&mut self, status: impl Into<String>) {
        self.status = status.into();
    }

    pub fn execute(&mut self, action: MenuAction) -> MenuActionResult {
        let result = self.execute_inner(action.clone());
        match result {
            Ok(message) => MenuActionResult {
                ok: true,
                quit_ui: matches!(action, MenuAction::QuitUi),
                message,
                snapshot: self.snapshot(),
            },
            Err(err) => {
                self.status = format!("操作失败: {}", err);
                MenuActionResult {
                    ok: false,
                    quit_ui: false,
                    message: err.to_string(),
                    snapshot: self.snapshot(),
                }
            }
        }
    }

    fn execute_inner(&mut self, action: MenuAction) -> Result<String> {
        match action {
            MenuAction::ToggleLlmEnabled => {
                self.config.llm.enabled = !self.config.llm.enabled;
                self.persist_config()?;
                self.status = format!("LLM 开关 => {}（已自动保存）", self.config.llm.enabled);
                Ok(self.status.clone())
            }
            MenuAction::ToggleTextCorrectionEnabled => {
                self.config.text_correction.enabled = !self.config.text_correction.enabled;
                self.persist_config()?;
                self.status = format!(
                    "文本纠错开关 => {}（已自动保存）",
                    self.config.text_correction.enabled
                );
                Ok(self.status.clone())
            }
            MenuAction::OpenConfigFolder => {
                self.status = "正在打开配置文件夹...".to_string();
                Ok(self.status.clone())
            }
            MenuAction::OpenModelFolder => {
                self.status = "正在打开模型文件夹...".to_string();
                Ok(self.status.clone())
            }
            MenuAction::SetField { field, value } => {
                let trimmed = value.trim().to_string();
                if trimmed.is_empty() {
                    return Err(anyhow!("输入不能为空"));
                }

                match field {
                    EditableField::LlmProvider => self.config.llm.provider = trimmed,
                    EditableField::LlmModel => self.config.llm.model = trimmed,
                    EditableField::LlmApiBase => self.config.llm.api_base = trimmed,
                    EditableField::LlmApiKey => self.config.llm.api_key = trimmed,
                }

                self.persist_config()?;
                self.status = "编辑已应用（已自动保存）".to_string();
                Ok(self.status.clone())
            }
            MenuAction::SetLlmConfig {
                provider,
                model,
                api_base,
                api_key,
            } => {
                let provider = provider.trim().to_string();
                let model = model.trim().to_string();
                let api_base = api_base.trim().to_string();
                let api_key = api_key.trim().to_string();

                if provider.is_empty() || model.is_empty() || api_base.is_empty() {
                    return Err(anyhow!("LLM 配置中 provider/model/api_base 不能为空"));
                }

                self.config.llm.provider = provider;
                self.config.llm.model = model;
                self.config.llm.api_base = api_base;
                self.config.llm.api_key = api_key;

                self.persist_config()?;
                self.status = "LLM 配置已更新（已自动保存）".to_string();
                Ok(self.status.clone())
            }
            MenuAction::SetHotkeyTriggerMode { mode } => {
                self.config.hotkey.trigger_mode = mode;
                self.persist_config()?;
                self.status = format!("热键触发模式已切换为 {}（已自动保存）", mode.label());
                Ok(self.status.clone())
            }
            MenuAction::ReloadConfig => {
                self.config = Config::load(&self.config_path)?;
                self.status = "配置文件已重载".to_string();
                Ok(self.status.clone())
            }
            MenuAction::QuitUi => {
                self.should_quit_ui = true;
                self.status = "收到退出指令".to_string();
                Ok(self.status.clone())
            }
        }
    }

    fn persist_config(&mut self) -> Result<()> {
        self.config.save(&self.config_path)?;
        self.dirty = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_config_path(suffix: &str) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let file = format!("echopup-menu-core-{}-{}.toml", suffix, nanos);
        std::env::temp_dir().join(file).display().to_string()
    }

    #[allow(dead_code)]
    fn temp_model_path(file_name: &str) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("echopup-menu-core-model-{}", nanos));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(file_name);
        std::fs::write(&path, b"test").unwrap();
        path.display().to_string()
    }

    #[test]
    fn test_menu_toggle_switches() {
        let mut core = MenuCore::new("/tmp/echopup-menu-core-toggle.toml").unwrap();
        let before = core.snapshot();

        let r1 = core.execute(MenuAction::ToggleLlmEnabled);
        assert!(r1.ok);
        assert_ne!(r1.snapshot.llm_enabled, before.llm_enabled);

        let r2 = core.execute(MenuAction::ToggleTextCorrectionEnabled);
        assert!(r2.ok);
        assert_ne!(
            r2.snapshot.text_correction_enabled,
            r1.snapshot.text_correction_enabled
        );
    }

    #[test]
    fn test_set_field_rejects_empty() {
        let mut core = MenuCore::new("/tmp/echopup-menu-core-empty.toml").unwrap();
        let r = core.execute(MenuAction::SetField {
            field: EditableField::LlmProvider,
            value: "  ".to_string(),
        });
        assert!(!r.ok);
        assert!(r.message.contains("输入不能为空"));
    }

    #[test]
    fn test_quit_ui_action() {
        let mut core = MenuCore::new("/tmp/echopup-menu-core-quit.toml").unwrap();
        let r = core.execute(MenuAction::QuitUi);
        assert!(r.ok);
        assert!(r.quit_ui);
        assert!(core.should_quit_ui());
    }

    #[test]
    fn test_phase_e_menu_contract_order() {
        assert_eq!(MENU_ITEMS.len(), 4);
        assert_eq!(MENU_ITEMS[0], "启用 LLM 润色");
        assert_eq!(MENU_ITEMS[3], "退出");
    }

    #[test]
    fn test_phase_e_auto_save_roundtrip() {
        let config_path = temp_config_path("phase-e-save");
        let mut core = MenuCore::new(&config_path).unwrap();

        let r1 = core.execute(MenuAction::ToggleLlmEnabled);
        assert!(r1.ok);
        let r2 = core.execute(MenuAction::SetField {
            field: EditableField::LlmProvider,
            value: "ollama".to_string(),
        });
        assert!(r2.ok);

        let reloaded = Config::load(&config_path).unwrap();
        assert!(reloaded.llm.enabled);
        assert_eq!(reloaded.llm.provider, "ollama");

        let _ = std::fs::remove_file(&config_path);
    }

    #[test]
    fn test_set_llm_form_auto_save() {
        let config_path = temp_config_path("phase-e-llm-form");
        let mut core = MenuCore::new(&config_path).unwrap();

        let r = core.execute(MenuAction::SetLlmConfig {
            provider: "openai".to_string(),
            model: "gpt-4.1-mini".to_string(),
            api_base: "https://api.openai.com/v1".to_string(),
            api_key: "test-key".to_string(),
        });
        assert!(r.ok);

        let reloaded = Config::load(&config_path).unwrap();
        assert_eq!(reloaded.llm.provider, "openai");
        assert_eq!(reloaded.llm.model, "gpt-4.1-mini");
        assert_eq!(reloaded.llm.api_base, "https://api.openai.com/v1");
        assert_eq!(reloaded.llm.api_key, "test-key");

        let _ = std::fs::remove_file(&config_path);
    }

    #[test]
    fn test_set_hotkey_trigger_mode_auto_save() {
        let config_path = temp_config_path("phase-e-hotkey-mode");
        let mut core = MenuCore::new(&config_path).unwrap();

        let result = core.execute(MenuAction::SetHotkeyTriggerMode {
            mode: HotkeyTriggerMode::HoldToRecord,
        });
        assert!(result.ok);
        assert_eq!(
            result.snapshot.hotkey_trigger_mode,
            HotkeyTriggerMode::HoldToRecord
        );

        let reloaded = Config::load(&config_path).unwrap();
        assert_eq!(
            reloaded.hotkey.trigger_mode,
            HotkeyTriggerMode::HoldToRecord
        );

        let _ = std::fs::remove_file(&config_path);
    }
}
