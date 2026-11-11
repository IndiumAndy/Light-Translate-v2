use crate::translation::{FastMtEngine, LlmProviderKind, LlmRequestConfig};
use chrono::{DateTime, TimeZone, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub const KEYRING_SERVICE: &str = "com.atat.lighttranslate.v2";
/// Seconds between Unix epoch (1970-01-01) and Apple Foundation reference date (2001-01-01).
const APPLE_REFERENCE_EPOCH_OFFSET_SECS: f64 = 978_307_200.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TriggerModifier {
    AltOrOption,
    LeftOption,
    RightOption,
    Control,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderEndpointSettings {
    pub base_url: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    pub enabled: bool,
    pub trigger_modifier: TriggerModifier,
    pub hover_dwell_ms: u64,
    pub dismiss_on_trigger_release: bool,
    pub linger_dismiss_ms: u64,
    pub debounce_radius_pt: f64,
    pub double_tap_selection: bool,
    pub selection_shortcut: String,
    pub capture_shortcut: String,
    pub enable_ocr_fallback: bool,
    pub enable_clipboard_fallback: bool,
    pub enable_fast_mt: bool,
    pub fast_mt_engine: FastMtEngine,
    pub word_fast_only: bool,
    pub enable_llm_translation: bool,
    pub active_provider: LlmProviderKind,
    pub gemini: ProviderEndpointSettings,
    pub deepseek: ProviderEndpointSettings,
    pub openai_compatible: ProviderEndpointSettings,
    pub target_language: String,
    pub excluded_apps: Vec<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            trigger_modifier: TriggerModifier::AltOrOption,
            hover_dwell_ms: 150,
            dismiss_on_trigger_release: true,
            linger_dismiss_ms: 2500,
            debounce_radius_pt: 8.0,
            double_tap_selection: true,
            selection_shortcut: if cfg!(target_os = "macos") {
                "Alt+Command+T".into()
            } else {
                "Alt+Shift+T".into()
            },
            capture_shortcut: if cfg!(target_os = "macos") {
                "Control+Command+S".into()
            } else {
                "Alt+Shift+S".into()
            },
            enable_ocr_fallback: true,
            enable_clipboard_fallback: false,
            enable_fast_mt: true,
            fast_mt_engine: FastMtEngine::GoogleFree,
            word_fast_only: false,
            enable_llm_translation: true,
            active_provider: LlmProviderKind::Gemini,
            gemini: ProviderEndpointSettings {
                base_url: LlmProviderKind::Gemini.default_base_url().into(),
                model: LlmProviderKind::Gemini.default_model().into(),
            },
            deepseek: ProviderEndpointSettings {
                base_url: LlmProviderKind::Deepseek.default_base_url().into(),
                model: LlmProviderKind::Deepseek.default_model().into(),
            },
            openai_compatible: ProviderEndpointSettings {
                base_url: LlmProviderKind::OpenaiCompatible.default_base_url().into(),
                model: LlmProviderKind::OpenaiCompatible.default_model().into(),
            },
            target_language: "zh-Hans".into(),
            excluded_apps: vec![
                "com.1password.1password".into(),
                "com.bitwarden.desktop".into(),
                "KeePassXC".into(),
            ],
        }
    }
}

impl AppSettings {
    pub fn endpoint_for(&self, provider: LlmProviderKind) -> &ProviderEndpointSettings {
        match provider {
            LlmProviderKind::Gemini => &self.gemini,
            LlmProviderKind::Deepseek => &self.deepseek,
            LlmProviderKind::OpenaiCompatible => &self.openai_compatible,
        }
    }

    pub fn build_llm_config(&self, api_key: String) -> LlmRequestConfig {
        let ep = self.endpoint_for(self.active_provider);
        LlmRequestConfig {
            provider: self.active_provider,
            base_url: if ep.base_url.trim().is_empty() {
                self.active_provider.default_base_url().into()
            } else {
                ep.base_url.trim().to_string()
            },
            model: if ep.model.trim().is_empty() {
                self.active_provider.default_model().into()
            } else {
                ep.model.trim().to_string()
            },
            api_key,
            target_language: self.target_language.clone(),
        }
    }

    pub fn is_app_excluded(&self, app_id: Option<&str>) -> bool {
        let Some(id) = app_id.map(str::trim).filter(|s| !s.is_empty()) else {
            return false;
        };
        self.excluded_apps
            .iter()
            .any(|ex| ex.eq_ignore_ascii_case(id))
    }
}

// ============================================================================
// Secret Store (Keyring + In-Memory Fallback/Test Seam)
// ============================================================================

pub trait SecretStore: Send + Sync {
    fn get_secret(&self, account: &str) -> Option<String>;
    fn set_secret(&self, account: &str, value: &str) -> Result<(), String>;
    fn delete_secret(&self, account: &str) -> Result<(), String>;
}

pub struct SystemKeyringStore {
    /// Session-level mirror so repeated hover lookups never pay IPC round-trips to Keychain/LSASS.
    cache: RwLock<HashMap<String, String>>,
}

impl Default for SystemKeyringStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemKeyringStore {
    pub fn new() -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
        }
    }
}

impl SecretStore for SystemKeyringStore {
    fn get_secret(&self, account: &str) -> Option<String> {
        if let Some(cached) = self.cache.read().get(account) {
            return (!cached.is_empty()).then(|| cached.clone());
        }
        let entry = keyring::Entry::new(KEYRING_SERVICE, account).ok()?;
        match entry.get_password() {
            Ok(pwd) => {
                let trimmed = pwd.trim().to_string();
                self.cache.write().insert(account.to_string(), trimmed.clone());
                (!trimmed.is_empty()).then_some(trimmed)
            }
            Err(_) => None,
        }
    }

    fn set_secret(&self, account: &str, value: &str) -> Result<(), String> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return self.delete_secret(account);
        }
        let entry =
            keyring::Entry::new(KEYRING_SERVICE, account).map_err(|e| e.to_string())?;
        entry.set_password(trimmed).map_err(|e| e.to_string())?;
        self.cache
            .write()
            .insert(account.to_string(), trimmed.to_string());
        Ok(())
    }

    fn delete_secret(&self, account: &str) -> Result<(), String> {
        self.cache.write().remove(account);
        if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, account) {
            let _ = entry.delete_credential();
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct MemorySecretStore {
    inner: RwLock<HashMap<String, String>>,
}

impl SecretStore for MemorySecretStore {
    fn get_secret(&self, account: &str) -> Option<String> {
        self.inner.read().get(account).cloned()
    }

    fn set_secret(&self, account: &str, value: &str) -> Result<(), String> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            self.inner.write().remove(account);
        } else {
            self.inner
                .write()
                .insert(account.to_string(), trimmed.to_string());
        }
        Ok(())
    }

    fn delete_secret(&self, account: &str) -> Result<(), String> {
        self.inner.write().remove(account);
        Ok(())
    }
}

// ============================================================================
// Saved Wordbook Entries + Legacy Swift `SavedEntries.json` Importer
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedEntry {
    pub id: Uuid,
    pub source: String,
    pub translation: String,
    pub context: String,
    pub created_at: DateTime<Utc>,
}

impl SavedEntry {
    pub fn new(source: String, translation: String, context: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            source: source.trim().to_string(),
            translation: translation.trim().to_string(),
            context: context.trim().to_string(),
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveOutcome {
    Inserted,
    AlreadySaved,
}

pub struct AppStorage {
    dir: PathBuf,
    settings: RwLock<AppSettings>,
    entries: RwLock<Vec<SavedEntry>>,
}

impl AppStorage {
    pub const SETTINGS_FILE: &'static str = "Settings.json";
    pub const SAVED_ENTRIES_FILE: &'static str = "SavedEntries.json";

    pub fn open_default() -> Self {
        let base = dirs::data_dir()
            .or_else(dirs::config_dir)
            .unwrap_or_else(std::env::temp_dir)
            .join("LightTranslateV2");
        Self::open_in(base)
    }

    pub fn open_in(dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&dir);
        let settings_path = dir.join(Self::SETTINGS_FILE);
        let settings = fs::read_to_string(&settings_path)
            .ok()
            .and_then(|s| serde_json::from_str::<AppSettings>(&s).ok())
            .unwrap_or_default();

        let entries_path = dir.join(Self::SAVED_ENTRIES_FILE);
        let entries = fs::read_to_string(&entries_path)
            .ok()
            .and_then(|s| Self::parse_entries_flexible(&s).ok())
            .unwrap_or_default();

        Self {
            dir,
            settings: RwLock::new(settings),
            entries: RwLock::new(entries),
        }
    }

    pub fn directory(&self) -> &Path {
        &self.dir
    }

    pub fn settings(&self) -> AppSettings {
        self.settings.read().clone()
    }

    pub fn update_settings(&self, next: AppSettings) -> Result<AppSettings, String> {
        let json = serde_json::to_vec_pretty(&next).map_err(|e| e.to_string())?;
        atomic_write(&self.dir.join(Self::SETTINGS_FILE), &json)?;
        *self.settings.write() = next.clone();
        Ok(next)
    }

    pub fn list_entries(&self) -> Vec<SavedEntry> {
        let mut list = self.entries.read().clone();
        list.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        list
    }

    pub fn save_entry(&self, entry: SavedEntry) -> Result<(SaveOutcome, SavedEntry), String> {
        let mut guard = self.entries.write();
        if let Some(existing) = guard.iter().find(|e| {
            e.source.eq_ignore_ascii_case(&entry.source) && e.context.trim() == entry.context.trim()
        }) {
            return Ok((SaveOutcome::AlreadySaved, existing.clone()));
        }
        guard.push(entry.clone());
        let json = serde_json::to_vec_pretty(&*guard).map_err(|e| e.to_string())?;
        atomic_write(&self.dir.join(Self::SAVED_ENTRIES_FILE), &json)?;
        Ok((SaveOutcome::Inserted, entry))
    }

    pub fn remove_entry(&self, id: Uuid) -> Result<(), String> {
        let mut guard = self.entries.write();
        guard.retain(|e| e.id != id);
        let json = serde_json::to_vec_pretty(&*guard).map_err(|e| e.to_string())?;
        atomic_write(&self.dir.join(Self::SAVED_ENTRIES_FILE), &json)
    }

    /// Import entries from either v2 ISO-8601 JSON or legacy v1 Swift `HoverTranslate/SavedEntries.json`
    /// (which encodes `createdAt` as `CFAbsoluteTime` floating-point seconds since 2001-01-01 UTC).
    pub fn import_entries_json(&self, raw_json: &str) -> Result<usize, String> {
        let imported = Self::parse_entries_flexible(raw_json)?;
        let mut guard = self.entries.write();
        let mut added = 0usize;
        for item in imported {
            let duplicate = guard.iter().any(|e| {
                e.id == item.id
                    || (e.source.eq_ignore_ascii_case(&item.source)
                        && e.context.trim() == item.context.trim())
            });
            if !duplicate {
                guard.push(item);
                added += 1;
            }
        }
        if added > 0 {
            let json = serde_json::to_vec_pretty(&*guard).map_err(|e| e.to_string())?;
            atomic_write(&self.dir.join(Self::SAVED_ENTRIES_FILE), &json)?;
        }
        Ok(added)
    }

    pub fn parse_entries_flexible(raw_json: &str) -> Result<Vec<SavedEntry>, String> {
        if raw_json.trim().is_empty() {
            return Ok(Vec::new());
        }
        let arr: Vec<Value> = serde_json::from_str(raw_json)
            .map_err(|e| format!("无法解析生词本 JSON: {e}"))?;
        let mut out = Vec::with_capacity(arr.len());

        for item in arr {
            let source = item
                .get("source")
                .and_then(Value::as_str)
                .ok_or("缺少 source 字段")?
                .trim()
                .to_string();
            let translation = item
                .get("translation")
                .and_then(Value::as_str)
                .ok_or("缺少 translation 字段")?
                .trim()
                .to_string();
            let context = item
                .get("context")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            let id = item
                .get("id")
                .and_then(Value::as_str)
                .and_then(|s| Uuid::parse_str(s).ok())
                .unwrap_or_else(Uuid::new_v4);

            let created_at = match item.get("createdAt") {
                Some(Value::String(iso)) => iso
                    .parse::<DateTime<Utc>>()
                    .unwrap_or_else(|_| Utc::now()),
                Some(Value::Number(num)) => {
                    let cf_secs = num.as_f64().unwrap_or(0.0);
                    let unix_secs = cf_secs + APPLE_REFERENCE_EPOCH_OFFSET_SECS;
                    let secs = unix_secs.floor() as i64;
                    let nanos = ((unix_secs - unix_secs.floor()) * 1e9) as u32;
                    Utc.timestamp_opt(secs, nanos)
                        .single()
                        .unwrap_or_else(Utc::now)
                }
                _ => Utc::now(),
            };

            out.push(SavedEntry {
                id,
                source,
                translation,
                context,
                created_at,
            });
        }
        Ok(out)
    }
}

fn atomic_write(target: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = target.with_extension(format!("tmp.{}", Uuid::new_v4()));
    fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    fs::rename(&tmp, target).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        e.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_settings_and_legacy_swift_saved_entries_import() {
        let temp_dir = std::env::temp_dir().join(format!("lt_v2_test_{}", Uuid::new_v4()));
        let store = AppStorage::open_in(temp_dir.clone());

        let mut settings = store.settings();
        assert!(settings.dismiss_on_trigger_release);
        assert!(!settings.enable_clipboard_fallback);
        assert!(settings.enable_ocr_fallback);
        settings.hover_dwell_ms = 180;
        store.update_settings(settings).unwrap();

        // Older Settings.json missing `dismissOnTriggerRelease` must default to true
        let legacy_settings: AppSettings =
            serde_json::from_str(r#"{"lingerDismissMs":2500,"hoverDwellMs":150}"#).unwrap();
        assert!(legacy_settings.dismiss_on_trigger_release);

        let reopened = AppStorage::open_in(temp_dir.clone());
        assert_eq!(reopened.settings().hover_dwell_ms, 180);

        // Simulate legacy Swift JSONEncoder output (`CFAbsoluteTime` float for 2026-09-18)
        let legacy_json = r#"[
          {
            "id": "E621E1F8-C36C-495A-93FC-0C247A3E6E5F",
            "source": "ephemeral",
            "translation": "短暂的",
            "context": "An ephemeral session analysis database.",
            "createdAt": 811468800.5
          }
        ]"#;
        let added = reopened.import_entries_json(legacy_json).unwrap();
        assert_eq!(added, 1);
        // Re-importing same entry is idempotent
        assert_eq!(reopened.import_entries_json(legacy_json).unwrap(), 0);

        let list = reopened.list_entries();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].source, "ephemeral");

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
