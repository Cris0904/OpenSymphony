//! Desktop settings service - local, non-secret configuration persistence.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::types::{CommandResult, DesktopError};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default)]
    pub values: HashMap<String, SettingValue>,
}

impl AppSettings {
    fn load_or_default(path: &PathBuf) -> Self {
        match fs::read_to_string(path) {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to parse settings at {path:?}: {e}");
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        }
    }

    fn save(&self, path: &PathBuf) -> Result<(), DesktopError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| DesktopError::Settings {
                message: format!("failed to create settings dir: {e}"),
            })?;
        }
        let content = serde_json::to_string_pretty(self).map_err(|e| DesktopError::Settings {
            message: format!("failed to serialize settings: {e}"),
        })?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, &content).map_err(|e| DesktopError::Settings {
            message: format!("failed to write settings: {e}"),
        })?;
        fs::rename(&tmp, path).map_err(|e| DesktopError::Settings {
            message: format!("failed to persist settings: {e}"),
        })?;
        Ok(())
    }
}

pub struct SettingsManager {
    settings: Mutex<AppSettings>,
    path: PathBuf,
}

impl SettingsManager {
    pub fn new() -> Result<Self, DesktopError> {
        let path = settings_path().map_err(|e| DesktopError::Settings {
            message: format!("failed to resolve settings path: {e}"),
        })?;
        let settings = Mutex::new(AppSettings::load_or_default(&path));
        Ok(Self { settings, path })
    }

    pub fn get(&self, key: &str) -> Option<SettingValue> {
        self.settings.lock().unwrap().values.get(key).cloned()
    }

    pub fn set(&self, key: &str, value: SettingValue) -> Result<(), DesktopError> {
        {
            let mut s = self.settings.lock().unwrap();
            s.values.insert(key.to_string(), value);
        }
        self.save()
    }

    fn save(&self) -> Result<(), DesktopError> {
        let s = self.settings.lock().unwrap();
        s.save(&self.path)
    }
}

fn global_manager() -> &'static SettingsManager {
    use std::sync::OnceLock;
    static M: OnceLock<SettingsManager> = OnceLock::new();
    M.get_or_init(|| {
        SettingsManager::new().unwrap_or_else(|e| {
            eprintln!("Warning: settings unavailable: {e}");
            SettingsManager {
                settings: Mutex::new(AppSettings::default()),
                path: PathBuf::from("/dev/null"),
            }
        })
    })
}

fn settings_path() -> Result<PathBuf, String> {
    let proj = directories::ProjectDirs::from("dev", "opensymphony", "app")
        .ok_or("could not determine project directories")?;
    Ok(proj.config_dir().join("settings.json"))
}

#[derive(Debug, Deserialize)]
pub struct GetSettingRequest {
    pub key: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type", content = "value")]
pub enum SettingValue {
    Text(String),
    Flag(bool),
    Number(f64),
}

#[derive(Debug, Serialize)]
pub struct GetSettingResponse {
    pub value: Option<SettingValue>,
}

#[derive(Debug, Deserialize)]
pub struct SetSettingRequest {
    pub key: String,
    pub value: SettingValue,
}

#[derive(Debug, Serialize)]
pub struct SetSettingResponse {
    pub persisted: bool,
}

#[tauri::command]
pub async fn get_setting(req: GetSettingRequest) -> CommandResult<GetSettingResponse> {
    let mgr = global_manager();
    Ok(GetSettingResponse { value: mgr.get(&req.key) })
}

#[tauri::command]
pub async fn set_setting(req: SetSettingRequest) -> CommandResult<SetSettingResponse> {
    let mgr = global_manager();
    mgr.set(&req.key, req.value)?;
    Ok(SetSettingResponse { persisted: true })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_setting_value_serialization() {
        let t = SettingValue::Text("hello".into());
        let j = serde_json::to_string(&t).unwrap();
        assert!(j.contains("Text"));
    }

    #[test]
    fn test_app_settings_default() {
        let s = AppSettings::default();
        assert!(s.values.is_empty());
    }
}
