#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;

use crate::extraction::{ExtractionFailure, ExtractionSnapshot, ScreenPoint, ScreenRect};
use crate::storage::TriggerModifier;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "kind")]
pub enum PlatformInputEvent {
    TriggerPressed {
        sentence_modifier: bool,
        pointer: ScreenPoint,
    },
    TriggerReleased {
        pointer: ScreenPoint,
        #[serde(default)]
        clean_tap: bool,
    },
    SentenceModifierChanged {
        active: bool,
        pointer: ScreenPoint,
    },
    DoubleTapSelection {
        pointer: ScreenPoint,
    },
    PointerMoved {
        pointer: ScreenPoint,
    },
    EscapePressed,
    OtherKeyPressed,
    GlobalMouseDown {
        pointer: ScreenPoint,
    },
    ShortcutSelection {
        pointer: ScreenPoint,
    },
    ShortcutCapture,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionsStatus {
    pub platform: String,
    pub accessibility: bool,
    pub screen_recording: bool,
}

pub const SHORTCUT_MOD_CTRL: u32 = 1 << 0;
pub const SHORTCUT_MOD_ALT: u32 = 1 << 1;
pub const SHORTCUT_MOD_SHIFT: u32 = 1 << 2;
pub const SHORTCUT_MOD_META: u32 = 1 << 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutSpec {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
    pub key: String,
}

impl ShortcutSpec {
    pub fn parse(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }

        let mut ctrl = false;
        let mut alt = false;
        let mut shift = false;
        let mut meta = false;
        let mut key: Option<String> = None;

        for part in trimmed.split('+').map(str::trim).filter(|s| !s.is_empty()) {
            match part.to_ascii_lowercase().as_str() {
                "control" | "ctrl" | "⌃" => ctrl = true,
                "alt" | "option" | "opt" | "⌥" => alt = true,
                "shift" | "⇧" => shift = true,
                "command" | "cmd" | "meta" | "super" | "win" | "⌘" => meta = true,
                other => {
                    let canonical = canonicalize_shortcut_key(other)?;
                    key = Some(canonical);
                }
            }
        }

        let key = key?;
        let is_fn_key = key.starts_with('F') && key.len() >= 2 && key[1..].chars().all(|c| c.is_ascii_digit());
        if !ctrl && !alt && !meta && !is_fn_key {
            return None;
        }

        Some(Self {
            ctrl,
            alt,
            shift,
            meta,
            key,
        })
    }

    pub fn modifier_mask(&self) -> u32 {
        let mut mask = 0;
        if self.ctrl {
            mask |= SHORTCUT_MOD_CTRL;
        }
        if self.alt {
            mask |= SHORTCUT_MOD_ALT;
        }
        if self.shift {
            mask |= SHORTCUT_MOD_SHIFT;
        }
        if self.meta {
            mask |= SHORTCUT_MOD_META;
        }
        mask
    }

    pub fn format_display(&self, is_macos: bool) -> String {
        let key_disp = match self.key.as_str() {
            "SPACE" => "Space",
            "GRAVE" => "`",
            "MINUS" => "-",
            "EQUAL" => "=",
            "BRACKETLEFT" => "[",
            "BRACKETRIGHT" => "]",
            "BACKSLASH" => "\\",
            "SEMICOLON" => ";",
            "QUOTE" => "'",
            "COMMA" => ",",
            "PERIOD" => ".",
            "SLASH" => "/",
            other => other,
        };

        if is_macos {
            let mut out = String::new();
            if self.ctrl {
                out.push('⌃');
            }
            if self.alt {
                out.push('⌥');
            }
            if self.shift {
                out.push('⇧');
            }
            if self.meta {
                out.push('⌘');
            }
            out.push_str(key_disp);
            out
        } else {
            let mut parts = Vec::new();
            if self.ctrl {
                parts.push("Ctrl");
            }
            if self.alt {
                parts.push("Alt");
            }
            if self.shift {
                parts.push("Shift");
            }
            if self.meta {
                parts.push("Win");
            }
            parts.push(key_disp);
            parts.join("+")
        }
    }

    pub fn windows_vk(&self) -> Option<u32> {
        let k = self.key.as_str();
        if k.len() == 1 {
            let b = k.as_bytes()[0];
            if b.is_ascii_uppercase() || b.is_ascii_digit() {
                return Some(b as u32);
            }
        }
        if let Some(num_str) = k.strip_prefix('F') {
            if let Ok(n) = num_str.parse::<u32>() {
                if (1..=12).contains(&n) {
                    return Some(0x70 + n - 1);
                }
            }
        }
        match k {
            "SPACE" => Some(0x20),
            "SEMICOLON" => Some(0xBA),
            "EQUAL" => Some(0xBB),
            "COMMA" => Some(0xBC),
            "MINUS" => Some(0xBD),
            "PERIOD" => Some(0xBE),
            "SLASH" => Some(0xBF),
            "GRAVE" => Some(0xC0),
            "BRACKETLEFT" => Some(0xDB),
            "BACKSLASH" => Some(0xDC),
            "BRACKETRIGHT" => Some(0xDD),
            "QUOTE" => Some(0xDE),
            _ => None,
        }
    }
}

fn canonicalize_shortcut_key(token: &str) -> Option<String> {
    let lower = token.to_ascii_lowercase();
    if lower.len() == 1 {
        let ch = lower.chars().next()?;
        if ch.is_ascii_alphabetic() {
            return Some(ch.to_ascii_uppercase().to_string());
        }
        if ch.is_ascii_digit() {
            return Some(ch.to_string());
        }
    }
    if let Some(letter) = lower.strip_prefix("key") {
        if letter.len() == 1 && letter.chars().all(|c| c.is_ascii_alphabetic()) {
            return Some(letter.to_ascii_uppercase());
        }
    }
    if let Some(digit) = lower.strip_prefix("digit") {
        if digit.len() == 1 && digit.chars().all(|c| c.is_ascii_digit()) {
            return Some(digit.to_string());
        }
    }
    if let Some(fnum) = lower.strip_prefix('f') {
        if let Ok(n) = fnum.parse::<u8>() {
            if (1..=12).contains(&n) {
                return Some(format!("F{n}"));
            }
        }
    }
    match lower.as_str() {
        "space" => Some("SPACE".into()),
        "`" | "grave" | "backquote" => Some("GRAVE".into()),
        "-" | "minus" => Some("MINUS".into()),
        "=" | "equal" => Some("EQUAL".into()),
        "[" | "bracketleft" => Some("BRACKETLEFT".into()),
        "]" | "bracketright" => Some("BRACKETRIGHT".into()),
        "\\" | "backslash" => Some("BACKSLASH".into()),
        ";" | "semicolon" => Some("SEMICOLON".into()),
        "'" | "quote" => Some("QUOTE".into()),
        "," | "comma" => Some("COMMA".into()),
        "." | "period" => Some("PERIOD".into()),
        "/" | "slash" => Some("SLASH".into()),
        _ => None,
    }
}

pub type InputEventCallback = Arc<dyn Fn(PlatformInputEvent) + Send + Sync + 'static>;

/// Unified cross-platform boundary implemented by `macos.rs`, `windows.rs`, and `linux.rs`.
pub trait PlatformBackend: Send + Sync {
    /// Start global low-level modifier, pointer, and hotkey monitoring.
    fn start_input_monitor(&self, callback: InputEventCallback);

    /// Update active trigger modifier, double-tap, and global shortcuts dynamically when user saves Settings.
    fn update_trigger_config(
        &self,
        modifier: TriggerModifier,
        double_tap_selection: bool,
        selection_shortcut: &str,
        capture_shortcut: &str,
    );

    /// Pause global shortcut dispatch while user is actively recording a new shortcut in Settings.
    fn set_shortcut_recording(&self, _active: bool) {}

    /// Current pointer location in top-left logical screen coordinates.
    fn current_pointer(&self) -> ScreenPoint;

    /// Bounds of the display containing `point` (or primary monitor fallback).
    fn monitor_bounds_at(&self, point: ScreenPoint) -> ScreenRect;

    /// Inspect bundle ID (macOS) or process executable name (Windows/Linux) owning the window at `point`.
    fn app_id_at_point(&self, point: ScreenPoint) -> Option<String>;

    /// Inspect focused application's bundle ID / executable name.
    fn focused_app_id(&self) -> Option<String>;

    /// Read active user selection from the focused control (regardless of whether `pointer` is inside it!).
    /// When `allow_clipboard_fallback` is true and AX/UIA returns no selection on a non-password control,
    /// performs safe clipboard backup -> copy -> 30ms restore.
    fn extract_selection(
        &self,
        pointer: ScreenPoint,
        generation: u64,
        allow_clipboard_fallback: bool,
    ) -> Result<Option<ExtractionSnapshot>, ExtractionFailure>;

    /// Read hovered word (plus sentence context) or full sentence via native Accessibility / UIAutomation.
    fn extract_hover_accessibility(
        &self,
        pointer: ScreenPoint,
        sentence_mode: bool,
        generation: u64,
    ) -> Result<ExtractionSnapshot, ExtractionFailure>;

    /// Seamless local offline OCR fallback around `pointer` (Vision on macOS, Windows.Media.Ocr on Windows).
    fn extract_hover_ocr(
        &self,
        pointer: ScreenPoint,
        sentence_mode: bool,
        generation: u64,
    ) -> Result<ExtractionSnapshot, ExtractionFailure>;

    /// Local offline OCR over a user-drawn screen rectangle (`Alt+Shift+S` / `⌃⌘S` capture mode).
    fn extract_region_ocr(
        &self,
        rect: ScreenRect,
        generation: u64,
    ) -> Result<ExtractionSnapshot, ExtractionFailure>;

    /// Check (and optionally prompt for) OS Accessibility / Screen Recording permissions.
    fn permissions_status(&self, prompt_accessibility: bool) -> PermissionsStatus;

    /// Explicitly request Screen Recording permission where applicable (macOS).
    fn request_screen_recording(&self) -> bool {
        true
    }

    /// Write plain text to system clipboard (explicit user Copy action only).
    fn write_clipboard(&self, text: &str) -> Result<(), String>;

    /// Apply native non-activating / tool-window styles to the pre-warmed `popup` WebviewWindow.
    fn configure_popup_window(&self, window: &tauri::WebviewWindow);

    /// Position and show `popup` without stealing focus or deactivating the active app.
    fn present_popup(&self, window: &tauri::WebviewWindow, rect: ScreenRect, interactive: bool);

    /// Toggle click-through (`interactive = false`) vs interactive hover/buttons (`interactive = true`).
    fn set_popup_interactive(&self, window: &tauri::WebviewWindow, interactive: bool);

    /// Hide `popup` immediately without activating any window.
    fn hide_popup(&self, window: &tauri::WebviewWindow);
}

pub fn create_platform_backend() -> Arc<dyn PlatformBackend> {
    #[cfg(target_os = "macos")]
    {
        Arc::new(macos::MacOsPlatform::new())
    }
    #[cfg(target_os = "windows")]
    {
        Arc::new(windows::WindowsPlatform::new())
    }
    #[cfg(target_os = "linux")]
    {
        Arc::new(linux::LinuxPlatform::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_camel_case_native_input_events() {
        let pressed: PlatformInputEvent = serde_json::from_str(
            r#"{"kind":"triggerPressed","sentenceModifier":true,"pointer":{"x":420.5,"y":310.0}}"#,
        )
        .expect("triggerPressed must deserialize camelCase sentenceModifier");
        assert_eq!(
            pressed,
            PlatformInputEvent::TriggerPressed {
                sentence_modifier: true,
                pointer: ScreenPoint::new(420.5, 310.0),
            }
        );

        let released: PlatformInputEvent = serde_json::from_str(
            r#"{"kind":"triggerReleased","cleanTap":true,"pointer":{"x":420.5,"y":310.0}}"#,
        )
        .expect("triggerReleased must deserialize camelCase cleanTap");
        assert_eq!(
            released,
            PlatformInputEvent::TriggerReleased {
                pointer: ScreenPoint::new(420.5, 310.0),
                clean_tap: true,
            }
        );

        let other: PlatformInputEvent =
            serde_json::from_str(r#"{"kind":"otherKeyPressed"}"#).expect("otherKeyPressed");
        assert_eq!(other, PlatformInputEvent::OtherKeyPressed);
    }

    #[test]
    fn parses_and_formats_global_shortcuts() {
        let cap = ShortcutSpec::parse("Control+Command+S").expect("valid macOS shortcut");
        assert_eq!(cap.format_display(true), "⌃⌘S");
        assert_eq!(cap.modifier_mask(), SHORTCUT_MOD_CTRL | SHORTCUT_MOD_META);
        assert_eq!(cap.key, "S");

        let win = ShortcutSpec::parse("Alt+Shift+A").expect("valid Alt+Shift shortcut");
        assert_eq!(win.format_display(false), "Alt+Shift+A");
        assert_eq!(win.windows_vk(), Some(0x41));

        let fn_key = ShortcutSpec::parse("F4").expect("standalone F-key allowed");
        assert_eq!(fn_key.format_display(true), "F4");

        // Plain single character or Shift+letter without primary modifier must be rejected
        assert!(ShortcutSpec::parse("S").is_none());
        assert!(ShortcutSpec::parse("Shift+S").is_none());
        assert!(ShortcutSpec::parse("").is_none());
    }
}
