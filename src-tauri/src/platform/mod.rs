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
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum PlatformInputEvent {
    TriggerPressed {
        sentence_modifier: bool,
        pointer: ScreenPoint,
    },
    TriggerReleased {
        pointer: ScreenPoint,
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

pub type InputEventCallback = Arc<dyn Fn(PlatformInputEvent) + Send + Sync + 'static>;

/// Unified cross-platform boundary implemented by `macos.rs`, `windows.rs`, and `linux.rs`.
pub trait PlatformBackend: Send + Sync {
    /// Start global low-level modifier, pointer, and hotkey monitoring.
    fn start_input_monitor(&self, callback: InputEventCallback);

    /// Update active trigger modifier / double-tap configuration dynamically when user saves Settings.
    fn update_trigger_config(&self, modifier: TriggerModifier, double_tap_selection: bool);

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
