use crate::translation::{FastMtEngine, LlmProviderKind, LlmRequestConfig};
use chrono::{DateTime, Datelike, Duration as ChronoDuration, Local, NaiveDate, TimeZone, Utc};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
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

// ============================================================================
// Daily Usage Statistics & Heatmap Analytics (`UsageStats.json`)
// ============================================================================

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DailyUsageRecord {
    pub word_count: u64,
    pub char_count: u64,
    pub query_count: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct UsageStatsFile {
    pub daily: BTreeMap<String, DailyUsageRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeatmapDayCell {
    pub date: String,
    /// ISO weekday: 1 = Monday .. 7 = Sunday
    pub weekday: u8,
    pub word_count: u64,
    pub char_count: u64,
    pub query_count: u64,
    /// Visual intensity bucket: 0 (none) ..= 4 (peak)
    pub level: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageStatsSummary {
    pub today_date: String,
    pub today_words: u64,
    pub today_chars: u64,
    pub today_queries: u64,
    pub total_words: u64,
    pub total_chars: u64,
    pub total_queries: u64,
    pub active_days: u32,
    pub current_streak: u32,
    pub longest_streak: u32,
    pub best_day_date: Option<String>,
    pub best_day_words: u64,
    pub this_week_words: u64,
    pub last_week_words: u64,
    /// 365 days ending at `today_date` (chronological order)
    pub heatmap: Vec<HeatmapDayCell>,
}

fn is_cjk_unit_char(c: char) -> bool {
    matches!(
        c as u32,
        0x3400..=0x4DBF   // CJK Unified Ideographs Extension A
        | 0x4E00..=0x9FFF // CJK Unified Ideographs
        | 0xF900..=0xFAFF // CJK Compatibility Ideographs
        | 0x3040..=0x309F // Hiragana
        | 0x30A0..=0x30FF // Katakana
        | 0xAC00..=0xD7AF // Hangul Syllables
        | 0x20000..=0x2A6DF // CJK Extension B
    )
}

/// Counts mixed-language translation units (`word_count`, `char_count`):
/// - English / Latin / numeric tokens count as 1 unit per word (`don't`, `v2.0`, `cross-platform` = 1 each).
/// - CJK ideographs / Kana / Hangul count as 1 unit per character.
/// - `char_count` is the total number of non-whitespace characters.
pub fn count_translation_units(text: &str) -> (u64, u64) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return (0, 0);
    }

    let mut units = 0u64;
    let mut non_ws_chars = 0u64;
    let mut in_word = false;
    let mut chars = trimmed.chars().peekable();

    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            in_word = false;
            continue;
        }
        non_ws_chars += 1;

        if is_cjk_unit_char(c) {
            in_word = false;
            units += 1;
        } else if c.is_alphanumeric() {
            if !in_word {
                units += 1;
                in_word = true;
            }
        } else if in_word && matches!(c, '\'' | '’' | '-' | '.' | '_') {
            // Keep internal connectors inside a word if followed immediately by an alphanumeric char
            if !chars.peek().is_some_and(|next| next.is_alphanumeric() && !is_cjk_unit_char(*next)) {
                in_word = false;
            }
        } else {
            in_word = false;
        }
    }

    (units, non_ws_chars)
}

const USAGE_DEDUP_WINDOW: Duration = Duration::from_secs(60);
const USAGE_DEDUP_CAPACITY: usize = 64;

pub struct AppStorage {
    dir: PathBuf,
    settings: RwLock<AppSettings>,
    entries: RwLock<Vec<SavedEntry>>,
    usage: RwLock<BTreeMap<String, DailyUsageRecord>>,
    recent_queries: Mutex<VecDeque<(String, Instant)>>,
}

impl AppStorage {
    pub const SETTINGS_FILE: &'static str = "Settings.json";
    pub const SAVED_ENTRIES_FILE: &'static str = "SavedEntries.json";
    pub const USAGE_STATS_FILE: &'static str = "UsageStats.json";

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

        let usage_path = dir.join(Self::USAGE_STATS_FILE);
        let usage = fs::read_to_string(&usage_path)
            .ok()
            .and_then(|s| serde_json::from_str::<UsageStatsFile>(&s).ok())
            .map(|f| f.daily)
            .unwrap_or_default();

        Self {
            dir,
            settings: RwLock::new(settings),
            entries: RwLock::new(entries),
            usage: RwLock::new(usage),
            recent_queries: Mutex::new(VecDeque::new()),
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

    /// Records a translation query into local daily stats (`UsageStats.json`) with a 60s
    /// duplicate suppression window so repeated accidental hovers on the same text don't inflate counts.
    pub fn record_translation_usage(&self, source_text: &str) -> Option<UsageStatsSummary> {
        let today = Local::now().date_naive();
        self.record_translation_usage_on_date(source_text, today, Instant::now())
    }

    pub fn record_translation_usage_on_date(
        &self,
        source_text: &str,
        local_date: NaiveDate,
        now: Instant,
    ) -> Option<UsageStatsSummary> {
        let (units, chars) = count_translation_units(source_text);
        if units == 0 && chars == 0 {
            return None;
        }

        let norm_key = source_text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();

        {
            let mut recent = self.recent_queries.lock();
            recent.retain(|(_, ts)| now.saturating_duration_since(*ts) <= USAGE_DEDUP_WINDOW);
            if recent.iter().any(|(prev, _)| prev == &norm_key) {
                return None;
            }
            if recent.len() >= USAGE_DEDUP_CAPACITY {
                recent.pop_front();
            }
            recent.push_back((norm_key, now));
        }

        let date_key = local_date.format("%Y-%m-%d").to_string();
        {
            let mut guard = self.usage.write();
            let entry = guard.entry(date_key).or_default();
            entry.word_count = entry.word_count.saturating_add(units.max(1));
            entry.char_count = entry.char_count.saturating_add(chars);
            entry.query_count = entry.query_count.saturating_add(1);

            let file_payload = UsageStatsFile {
                daily: guard.clone(),
            };
            if let Ok(bytes) = serde_json::to_vec_pretty(&file_payload) {
                let _ = atomic_write(&self.dir.join(Self::USAGE_STATS_FILE), &bytes);
            }
        }

        Some(self.usage_summary_for_date(local_date))
    }

    pub fn usage_summary(&self) -> UsageStatsSummary {
        self.usage_summary_for_date(Local::now().date_naive())
    }

    pub fn usage_summary_for_date(&self, today: NaiveDate) -> UsageStatsSummary {
        let guard = self.usage.read();
        let today_str = today.format("%Y-%m-%d").to_string();
        let today_rec = guard.get(&today_str).cloned().unwrap_or_default();

        let mut total_words = 0u64;
        let mut total_chars = 0u64;
        let mut total_queries = 0u64;
        let mut active_days = 0u32;
        let mut best_day_words = 0u64;
        let mut best_day_date: Option<String> = None;

        // Parse active dates in chronological order for longest streak calculation
        let mut active_dates: Vec<NaiveDate> = Vec::new();
        for (date_str, rec) in guard.iter() {
            total_words = total_words.saturating_add(rec.word_count);
            total_chars = total_chars.saturating_add(rec.char_count);
            total_queries = total_queries.saturating_add(rec.query_count);

            if rec.word_count > 0 || rec.query_count > 0 {
                active_days += 1;
                if rec.word_count >= best_day_words && rec.word_count > 0 {
                    best_day_words = rec.word_count;
                    best_day_date = Some(date_str.clone());
                }
                if let Ok(d) = NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                    active_dates.push(d);
                }
            }
        }
        active_dates.sort_unstable();

        // Longest streak across all recorded history
        let mut longest_streak = 0u32;
        let mut run_streak = 0u32;
        let mut prev_date: Option<NaiveDate> = None;
        for d in &active_dates {
            if let Some(prev) = prev_date {
                if *d == prev + ChronoDuration::days(1) {
                    run_streak += 1;
                } else if *d != prev {
                    run_streak = 1;
                }
            } else {
                run_streak = 1;
            }
            longest_streak = longest_streak.max(run_streak);
            prev_date = Some(*d);
        }

        // Current streak: starts at `today` if active today, else starts at `yesterday` if active yesterday
        // so opening the app in the morning doesn't show "0 day streak" before the first query!
        let has_activity = |d: NaiveDate| -> bool {
            let key = d.format("%Y-%m-%d").to_string();
            guard
                .get(&key)
                .is_some_and(|r| r.word_count > 0 || r.query_count > 0)
        };
        let mut current_streak = 0u32;
        let mut cursor = if has_activity(today) {
            Some(today)
        } else {
            let yesterday = today - ChronoDuration::days(1);
            has_activity(yesterday).then_some(yesterday)
        };
        while let Some(d) = cursor {
            if has_activity(d) {
                current_streak += 1;
                cursor = Some(d - ChronoDuration::days(1));
            } else {
                break;
            }
        }

        // Rolling 7-day comparison (this 7 days vs prior 7 days)
        let mut this_week_words = 0u64;
        let mut last_week_words = 0u64;
        for offset in 0..7 {
            let d1 = (today - ChronoDuration::days(offset))
                .format("%Y-%m-%d")
                .to_string();
            if let Some(r) = guard.get(&d1) {
                this_week_words = this_week_words.saturating_add(r.word_count);
            }
            let d2 = (today - ChronoDuration::days(offset + 7))
                .format("%Y-%m-%d")
                .to_string();
            if let Some(r) = guard.get(&d2) {
                last_week_words = last_week_words.saturating_add(r.word_count);
            }
        }

        // Adaptive 4-tier heatmap bucket thresholds anchored by peak daily volume (floor 360 words)
        let mut window_peak = 0u64;
        for offset in 0..365 {
            let d = (today - ChronoDuration::days(offset))
                .format("%Y-%m-%d")
                .to_string();
            if let Some(r) = guard.get(&d) {
                window_peak = window_peak.max(r.word_count);
            }
        }
        let scale_ref = window_peak.max(360);
        let t1 = (scale_ref / 12).max(15);
        let t2 = (scale_ref / 4).max(60);
        let t3 = (scale_ref * 6 / 10).max(180);

        let mut heatmap = Vec::with_capacity(365);
        for back in (0..365).rev() {
            let d = today - ChronoDuration::days(back);
            let date_str = d.format("%Y-%m-%d").to_string();
            let rec = guard.get(&date_str).cloned().unwrap_or_default();
            let level = if rec.word_count == 0 && rec.query_count == 0 {
                0
            } else if rec.word_count <= t1 {
                1
            } else if rec.word_count <= t2 {
                2
            } else if rec.word_count <= t3 {
                3
            } else {
                4
            };
            heatmap.push(HeatmapDayCell {
                date: date_str,
                weekday: d.weekday().number_from_monday() as u8,
                word_count: rec.word_count,
                char_count: rec.char_count,
                query_count: rec.query_count,
                level,
            });
        }

        UsageStatsSummary {
            today_date: today_str,
            today_words: today_rec.word_count,
            today_chars: today_rec.char_count,
            today_queries: today_rec.query_count,
            total_words,
            total_chars,
            total_queries,
            active_days,
            current_streak,
            longest_streak,
            best_day_date,
            best_day_words,
            this_week_words,
            last_week_words,
            heatmap,
        }
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
    fn smart_unit_counter_handles_english_cjk_hyphens_and_numbers() {
        assert_eq!(count_translation_units("   "), (0, 0));
        assert_eq!(count_translation_units("Hello, world!"), (2, 12));
        // Contractions, hyphens, and decimals stay unified as single words
        assert_eq!(
            count_translation_units("Don't break well-known v2.0 tokens."),
            (5, 31)
        );
        // Mixed CJK + English: 4 Chinese chars + "AI" (1) + 2 Chinese chars = 7 units
        assert_eq!(count_translation_units("实时流式 AI 翻译"), (7, 8));
    }

    #[test]
    fn usage_stats_deduplicates_within_window_and_tracks_streaks() {
        let temp_dir = std::env::temp_dir().join(format!("lt_v2_stats_{}", Uuid::new_v4()));
        let store = AppStorage::open_in(temp_dir.clone());

        let t0 = Instant::now();
        let d1 = NaiveDate::from_ymd_opt(2026, 11, 8).unwrap();
        let d2 = NaiveDate::from_ymd_opt(2026, 11, 9).unwrap();
        let d3 = NaiveDate::from_ymd_opt(2026, 11, 10).unwrap();

        assert!(store
            .record_translation_usage_on_date("ephemeral architecture", d1, t0)
            .is_some());
        // Duplicate within 60s is ignored
        assert!(store
            .record_translation_usage_on_date("  Ephemeral   Architecture ", d1, t0 + Duration::from_secs(15))
            .is_none());
        // Same text after >60s is counted again
        assert!(store
            .record_translation_usage_on_date("ephemeral architecture", d2, t0 + Duration::from_secs(65))
            .is_some());

        // Morning of d3 BEFORE any query today: yesterday's 2-day streak is still preserved!
        let morning_summary = store.usage_summary_for_date(d3);
        assert_eq!(morning_summary.today_words, 0);
        assert_eq!(morning_summary.current_streak, 2);
        assert_eq!(morning_summary.longest_streak, 2);
        assert_eq!(morning_summary.total_words, 4);

        // Query on d3 extends streak to 3!
        let after_d3 = store
            .record_translation_usage_on_date("Cross-platform Liquid Glass overlay", d3, t0 + Duration::from_secs(130))
            .unwrap();
        assert_eq!(after_d3.today_words, 4);
        assert_eq!(after_d3.current_streak, 3);
        assert_eq!(after_d3.longest_streak, 3);
        assert_eq!(after_d3.active_days, 3);
        assert_eq!(after_d3.heatmap.len(), 365);
        assert_eq!(after_d3.heatmap.last().unwrap().date, "2026-11-10");
        assert!(after_d3.heatmap.last().unwrap().level >= 1);

        // Reload from disk and verify UsageStats.json persisted atomically
        let reopened = AppStorage::open_in(temp_dir.clone());
        let reloaded = reopened.usage_summary_for_date(d3);
        assert_eq!(reloaded.total_words, 8);
        assert_eq!(reloaded.current_streak, 3);

        let _ = fs::remove_dir_all(&temp_dir);
    }

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
