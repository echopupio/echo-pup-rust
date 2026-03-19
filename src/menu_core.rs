//! 共享菜单业务内核（TUI / 状态栏）

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::sync::mpsc::{Receiver, TryRecvError};

use crate::config::Config;
use crate::hotkey;
use crate::model_download::{
    self, DownloadEvent, DownloadStart, DownloadState, DOWNLOAD_LOG_MAX_LINES,
};

pub const MENU_ITEMS: [&str; 15] = [
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditableField {
    Hotkey,
    LlmProvider,
    LlmModel,
    LlmApiBase,
    LlmApiKeyEnv,
    WhisperModelPath,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MenuAction {
    ToggleLlmEnabled,
    ToggleTextCorrectionEnabled,
    ToggleVadEnabled,
    SetField { field: EditableField, value: String },
    DownloadModel { size: String },
    RefreshLocalModels,
    SaveConfig,
    QuitUi,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MenuSnapshot {
    pub config_path: String,
    pub status: String,
    pub dirty: bool,
    pub should_quit_ui: bool,

    pub hotkey: String,
    pub llm_enabled: bool,
    pub text_correction_enabled: bool,
    pub vad_enabled: bool,
    pub llm_provider: String,
    pub llm_model: String,
    pub llm_api_base: String,
    pub llm_api_key_env: String,
    pub whisper_model_path: String,

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

            hotkey: self.config.hotkey.key.clone(),
            llm_enabled: self.config.llm.enabled,
            text_correction_enabled: self.config.text_correction.enabled,
            vad_enabled: self.config.audio.vad_enabled,
            llm_provider: self.config.llm.provider.clone(),
            llm_model: self.config.llm.model.clone(),
            llm_api_base: self.config.llm.api_base.clone(),
            llm_api_key_env: self.config.llm.api_key_env.clone(),
            whisper_model_path: self.config.whisper.model_path.clone(),

            local_models: self.local_models.clone(),
            download: self.download.clone(),
            download_logs: self.download_logs.clone(),
        }
    }

    pub fn current_value(&self, field: EditableField) -> String {
        match field {
            EditableField::Hotkey => self.config.hotkey.key.clone(),
            EditableField::LlmProvider => self.config.llm.provider.clone(),
            EditableField::LlmModel => self.config.llm.model.clone(),
            EditableField::LlmApiBase => self.config.llm.api_base.clone(),
            EditableField::LlmApiKeyEnv => self.config.llm.api_key_env.clone(),
            EditableField::WhisperModelPath => self.config.whisper.model_path.clone(),
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
                self.dirty = true;
                self.status = format!("LLM 开关 => {}", self.config.llm.enabled);
                Ok(self.status.clone())
            }
            MenuAction::ToggleTextCorrectionEnabled => {
                self.config.text_correction.enabled = !self.config.text_correction.enabled;
                self.dirty = true;
                self.status = format!("文本纠错开关 => {}", self.config.text_correction.enabled);
                Ok(self.status.clone())
            }
            MenuAction::ToggleVadEnabled => {
                self.config.audio.vad_enabled = !self.config.audio.vad_enabled;
                self.dirty = true;
                self.status = format!("VAD 开关 => {}", self.config.audio.vad_enabled);
                Ok(self.status.clone())
            }
            MenuAction::SetField { field, value } => {
                let trimmed = value.trim().to_string();
                if trimmed.is_empty() {
                    return Err(anyhow!("输入不能为空"));
                }

                if field == EditableField::Hotkey {
                    if let Err(err) = hotkey::validate_hotkey_config(&trimmed) {
                        return Err(anyhow!("热键不安全: {}", err));
                    }
                }

                match field {
                    EditableField::Hotkey => self.config.hotkey.key = trimmed,
                    EditableField::LlmProvider => self.config.llm.provider = trimmed,
                    EditableField::LlmModel => self.config.llm.model = trimmed,
                    EditableField::LlmApiBase => self.config.llm.api_base = trimmed,
                    EditableField::LlmApiKeyEnv => self.config.llm.api_key_env = trimmed,
                    EditableField::WhisperModelPath => self.config.whisper.model_path = trimmed,
                }

                self.dirty = true;
                self.status = "编辑已应用（记得保存配置）".to_string();
                Ok(self.status.clone())
            }
            MenuAction::DownloadModel { size } => {
                if self
                    .download
                    .as_ref()
                    .map(|d| d.in_progress)
                    .unwrap_or(false)
                {
                    self.status = "已有下载任务进行中，请等待完成".to_string();
                    return Ok(self.status.clone());
                }

                let DownloadStart {
                    state,
                    rx,
                    initial_logs,
                } = model_download::start_model_download(&size)?;

                self.download = Some(state);
                self.download_logs.clear();
                self.download_logs.extend(initial_logs);
                self.download_rx = Some(rx);
                self.status = format!("正在下载模型 {} ...", size);
                Ok(self.status.clone())
            }
            MenuAction::RefreshLocalModels => {
                self.local_models = model_download::list_local_models();
                self.status = "已刷新本地模型列表".to_string();
                Ok(self.status.clone())
            }
            MenuAction::SaveConfig => {
                self.config.save(&self.config_path)?;
                self.dirty = false;
                self.status = format!("配置已保存: {}", self.config_path);
                Ok(self.status.clone())
            }
            MenuAction::QuitUi => {
                self.should_quit_ui = true;
                self.status = "已退出 UI".to_string();
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

    #[test]
    fn test_menu_toggle_switches() {
        let mut core = MenuCore::new("/tmp/echopup-menu-core-toggle.toml").unwrap();

        let r1 = core.execute(MenuAction::ToggleLlmEnabled);
        assert!(r1.ok);
        assert!(r1.snapshot.llm_enabled);

        let r2 = core.execute(MenuAction::ToggleTextCorrectionEnabled);
        assert!(r2.ok);
        assert!(!r2.snapshot.text_correction_enabled);

        let r3 = core.execute(MenuAction::ToggleVadEnabled);
        assert!(r3.ok);
        assert!(r3.snapshot.vad_enabled);
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
        assert_eq!(MENU_ITEMS.len(), 15);
        assert_eq!(MENU_ITEMS[0], "切换 LLM 开关");
        assert_eq!(MENU_ITEMS[8], "编辑 Whisper model_path");
        assert_eq!(MENU_ITEMS[14], "退出 UI");
    }

    #[test]
    fn test_phase_e_save_roundtrip() {
        let config_path = temp_config_path("phase-e-save");
        let mut core = MenuCore::new(&config_path).unwrap();

        let r1 = core.execute(MenuAction::ToggleLlmEnabled);
        assert!(r1.ok);
        let r2 = core.execute(MenuAction::SetField {
            field: EditableField::LlmProvider,
            value: "ollama".to_string(),
        });
        assert!(r2.ok);
        let r3 = core.execute(MenuAction::SaveConfig);
        assert!(r3.ok);

        let reloaded = Config::load(&config_path).unwrap();
        assert!(reloaded.llm.enabled);
        assert_eq!(reloaded.llm.provider, "ollama");

        let _ = std::fs::remove_file(&config_path);
    }

    #[test]
    fn test_phase_e_download_singleton_guard() {
        let mut core = MenuCore::new("/tmp/echopup-menu-core-guard.toml").unwrap();
        core.download = Some(DownloadState {
            model_size: "large-v3".to_string(),
            model_file_name: "ggml-large-v3.bin".to_string(),
            downloaded: 1024,
            total: Some(2048),
            in_progress: true,
        });

        let r = core.execute(MenuAction::DownloadModel {
            size: "turbo".to_string(),
        });

        assert!(r.ok);
        assert!(r.message.contains("已有下载任务进行中"));
    }
}
