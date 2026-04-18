//! 共享菜单业务内核（TUI / 状态栏）

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::sync::mpsc::{Receiver, TryRecvError};

use crate::config::Config;
use crate::config::HotkeyTriggerMode;
use crate::model_download::{self, DownloadEvent, DownloadState, DOWNLOAD_LOG_MAX_LINES};

pub const MENU_ITEMS: [&str; 5] = [
    "切换 LLM 开关",
    "切换文本纠错开关",
    "编辑 LLM 配置",
    "下载 ASR 模型",
    "退出",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditableField {
    LlmProvider,
    LlmModel,
    LlmApiBase,
    LlmApiKeyEnv,
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
        api_key_env: String,
    },
    SetHotkeyTriggerMode {
        mode: HotkeyTriggerMode,
    },
    DownloadModel,
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
    pub llm_api_key_env: String,

    pub local_models: Vec<String>,
    pub download: Option<DownloadState>,
    pub download_logs: Vec<String>,
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
    local_models: Vec<String>,
    download_logs: Vec<String>,
    download: Option<DownloadState>,
    download_rx: Option<Receiver<DownloadEvent>>,
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
            local_models: model_download::list_local_models(),
            download_logs: Vec::new(),
            download: None,
            download_rx: None,
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
            llm_api_key_env: self.config.llm.api_key_env.clone(),

            local_models: self.local_models.clone(),
            download: self.download.clone(),
            download_logs: self.download_logs.clone(),
        }
    }

    #[allow(dead_code)]
    pub fn current_value(&self, field: EditableField) -> String {
        match field {
            EditableField::LlmProvider => self.config.llm.provider.clone(),
            EditableField::LlmModel => self.config.llm.model.clone(),
            EditableField::LlmApiBase => self.config.llm.api_base.clone(),
            EditableField::LlmApiKeyEnv => self.config.llm.api_key_env.clone(),
        }
    }

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
                    EditableField::LlmApiKeyEnv => self.config.llm.api_key_env = trimmed,
                }

                self.persist_config()?;
                self.status = "编辑已应用（已自动保存）".to_string();
                Ok(self.status.clone())
            }
            MenuAction::SetLlmConfig {
                provider,
                model,
                api_base,
                api_key_env,
            } => {
                let provider = provider.trim().to_string();
                let model = model.trim().to_string();
                let api_base = api_base.trim().to_string();
                let api_key_env = api_key_env.trim().to_string();

                if provider.is_empty() || model.is_empty() || api_base.is_empty() {
                    return Err(anyhow!("LLM 配置中 provider/model/api_base 不能为空"));
                }

                self.config.llm.provider = provider;
                self.config.llm.model = model;
                self.config.llm.api_base = api_base;
                self.config.llm.api_key_env = api_key_env;

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
            MenuAction::DownloadModel => {
                use model_download::start_paraformer_model_download;
                match start_paraformer_model_download() {
                    Ok(start) => {
                        self.download = Some(start.state);
                        self.download_rx = Some(start.rx);
                        self.download_logs.clear();
                        self.download_logs.extend(start.initial_logs);
                        self.status = "正在下载 Sherpa Paraformer 模型...".to_string();
                        Ok(self.status.clone())
                    }
                    Err(err) => {
                        self.status = format!("启动下载失败: {}", err);
                        Err(anyhow!("启动下载失败: {}", err))
                    }
                }
            }
            MenuAction::ReloadConfig => {
                self.config = Config::load(&self.config_path)?;
                self.local_models = model_download::list_local_models();
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

    pub fn poll_download_events(&mut self) -> bool {
        let mut changed = false;
        let mut clear_rx = false;

        loop {
            let recv_result = {
                let Some(rx) = self.download_rx.as_ref() else {
                    break;
                };
                rx.try_recv()
            };

            match recv_result {
                Ok(event) => {
                    changed = true;
                    match event {
                        DownloadEvent::Started {
                            model_size,
                            model_file_name,
                            downloaded,
                            total,
                        } => {
                            self.download = Some(DownloadState {
                                model_size: model_size.clone(),
                                model_file_name,
                                downloaded,
                                total,
                                in_progress: true,
                            });
                            self.status = format!("正在下载模型 {} ...", model_size);
                            self.append_download_log(format!(
                                "[started] 已下载 {}，总大小 {}",
                                model_download::format_bytes(downloaded),
                                total
                                    .map(model_download::format_bytes)
                                    .unwrap_or_else(|| "未知".to_string())
                            ));
                        }
                        DownloadEvent::Progress { downloaded, total } => {
                            if let Some(download) = self.download.as_mut() {
                                download.downloaded = downloaded;
                                if total.is_some() {
                                    download.total = total;
                                }
                            }
                        }
                        DownloadEvent::Finished => {
                            let mut model_size = "unknown".to_string();
                            if let Some(download) = self.download.as_mut() {
                                model_size = download.model_size.clone();
                                download.in_progress = false;
                                if download.total.is_none() {
                                    download.total = Some(download.downloaded);
                                }
                            }
                            self.local_models = model_download::list_local_models();
                            self.status = format!("模型 {} 下载完成", model_size);
                            self.append_download_log("[finished] 下载完成".to_string());
                            clear_rx = true;
                        }
                        DownloadEvent::Failed(err) => {
                            if let Some(download) = self.download.as_mut() {
                                download.in_progress = false;
                            }
                            self.status = format!("下载失败: {}", err);
                            self.append_download_log(format!("[error] {}", err));
                            clear_rx = true;
                        }
                        DownloadEvent::Log(line) => {
                            self.append_download_log(line);
                        }
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if let Some(download) = self.download.as_mut() {
                        download.in_progress = false;
                    }
                    clear_rx = true;
                    break;
                }
            }
        }

        if clear_rx {
            self.download_rx = None;
            changed = true;
        }

        changed
    }

    fn append_download_log(&mut self, line: String) {
        self.download_logs.push(line);
        if self.download_logs.len() > DOWNLOAD_LOG_MAX_LINES {
            let drain_len = self.download_logs.len() - DOWNLOAD_LOG_MAX_LINES;
            self.download_logs.drain(0..drain_len);
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
        assert_eq!(MENU_ITEMS.len(), 5);
        assert_eq!(MENU_ITEMS[0], "切换 LLM 开关");
        assert_eq!(MENU_ITEMS[3], "下载 ASR 模型");
        assert_eq!(MENU_ITEMS[4], "退出");
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
            api_key_env: "OPENAI_API_KEY".to_string(),
        });
        assert!(r.ok);

        let reloaded = Config::load(&config_path).unwrap();
        assert_eq!(reloaded.llm.provider, "openai");
        assert_eq!(reloaded.llm.model, "gpt-4.1-mini");
        assert_eq!(reloaded.llm.api_base, "https://api.openai.com/v1");
        assert_eq!(reloaded.llm.api_key_env, "OPENAI_API_KEY");

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
