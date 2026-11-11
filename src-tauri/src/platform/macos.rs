use crate::extraction::resolver::{
    resolve_word_in_utf16, OcrClusterer, OcrLine, SentenceResolver, SentenceWindow, Utf16Range,
};
use crate::extraction::{
    is_allowed_input_text, ExtractionCompleteness, ExtractionFailure, ExtractionScope,
    ExtractionSnapshot, ExtractionSource, ScreenPoint, ScreenRect,
};
use crate::platform::{InputEventCallback, PermissionsStatus, PlatformBackend, PlatformInputEvent};
use crate::storage::TriggerModifier;
use parking_lot::Mutex;
use serde::Deserialize;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::LazyLock;

unsafe extern "C" {
    fn lt_macos_start_input_monitor(callback: extern "C" fn(*const c_char));
    fn lt_macos_update_trigger_config(modifier_kind: i32, double_tap_selection: bool);
    fn lt_macos_get_pointer(out_x: *mut f64, out_y: *mut f64);
    fn lt_macos_get_monitor_bounds(
        x: f64,
        y: f64,
        out_x: *mut f64,
        out_y: *mut f64,
        out_w: *mut f64,
        out_h: *mut f64,
    );
    fn lt_macos_app_id_at_point(x: f64, y: f64) -> *mut c_char;
    fn lt_macos_focused_app_id() -> *mut c_char;
    fn lt_macos_check_accessibility(prompt: bool) -> bool;
    fn lt_macos_check_screen_recording() -> bool;
    fn lt_macos_request_screen_recording() -> bool;
    fn lt_macos_write_clipboard(text_utf8: *const c_char) -> bool;
    fn lt_macos_extract_selection_json(
        x: f64,
        y: f64,
        allow_clipboard_fallback: bool,
    ) -> *mut c_char;
    fn lt_macos_extract_hover_ax_json(x: f64, y: f64) -> *mut c_char;
    fn lt_macos_ocr_at_point_json(x: f64, y: f64, sentence_mode: bool) -> *mut c_char;
    fn lt_macos_ocr_region_json(x: f64, y: f64, w: f64, h: f64) -> *mut c_char;
    fn lt_macos_configure_popup_window(ns_window_ptr: *mut std::ffi::c_void);
    fn lt_macos_present_popup_window(
        ns_window_ptr: *mut std::ffi::c_void,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        interactive: bool,
    );
    fn lt_macos_set_popup_interactive(ns_window_ptr: *mut std::ffi::c_void, interactive: bool);
    fn lt_macos_hide_popup_window(ns_window_ptr: *mut std::ffi::c_void);
    fn lt_macos_free_string(ptr: *mut c_char);
}

static INPUT_CALLBACK: LazyLock<Mutex<Option<InputEventCallback>>> =
    LazyLock::new(|| Mutex::new(None));

extern "C" fn on_native_input_event(json_ptr: *const c_char) {
    if json_ptr.is_null() {
        return;
    }
    let raw = unsafe { CStr::from_ptr(json_ptr) }.to_string_lossy();
    let event = match serde_json::from_str::<PlatformInputEvent>(&raw) {
        Ok(ev) => ev,
        Err(err) => {
            tracing::error!("Failed to parse native input event JSON ({err}): {raw}");
            return;
        }
    };
    let cb = INPUT_CALLBACK.lock().clone();
    if let Some(callback) = cb {
        callback(event);
    }
}

fn take_owned_string(ptr: *mut c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let s = unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned();
    unsafe { lt_macos_free_string(ptr) };
    Some(s)
}

fn modifier_to_code(m: TriggerModifier) -> i32 {
    match m {
        TriggerModifier::AltOrOption => 0,
        TriggerModifier::LeftOption => 1,
        TriggerModifier::RightOption => 2,
        TriggerModifier::Control => 3,
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeSelectionResponse {
    status: String,
    text: Option<String>,
    source: Option<String>,
    rect: Option<ScreenRect>,
    app_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeHoverAxResponse {
    status: String,
    window_text: Option<String>,
    window_start: Option<usize>,
    element_total: Option<usize>,
    hit_offset: Option<usize>,
    hit_length: Option<usize>,
    paragraph_text: Option<String>,
    paragraph_hit_offset: Option<usize>,
    label: Option<String>,
    rect: Option<ScreenRect>,
    app_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeOcrResponse {
    status: String,
    #[serde(default)]
    lines: Vec<OcrLine>,
    app_id: Option<String>,
}

pub struct MacOsPlatform;

impl Default for MacOsPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl MacOsPlatform {
    pub fn new() -> Self {
        Self
    }
}

impl PlatformBackend for MacOsPlatform {
    fn start_input_monitor(&self, callback: InputEventCallback) {
        *INPUT_CALLBACK.lock() = Some(callback);
        unsafe {
            lt_macos_start_input_monitor(on_native_input_event);
        }
    }

    fn update_trigger_config(&self, modifier: TriggerModifier, double_tap_selection: bool) {
        unsafe {
            lt_macos_update_trigger_config(modifier_to_code(modifier), double_tap_selection);
        }
    }

    fn current_pointer(&self) -> ScreenPoint {
        let (mut x, mut y) = (0.0, 0.0);
        unsafe {
            lt_macos_get_pointer(&mut x, &mut y);
        }
        ScreenPoint::new(x, y)
    }

    fn monitor_bounds_at(&self, point: ScreenPoint) -> ScreenRect {
        let (mut x, mut y, mut w, mut h) = (0.0, 0.0, 1440.0, 900.0);
        unsafe {
            lt_macos_get_monitor_bounds(point.x, point.y, &mut x, &mut y, &mut w, &mut h);
        }
        ScreenRect::new(x, y, w, h)
    }

    fn app_id_at_point(&self, point: ScreenPoint) -> Option<String> {
        take_owned_string(unsafe { lt_macos_app_id_at_point(point.x, point.y) })
    }

    fn focused_app_id(&self) -> Option<String> {
        take_owned_string(unsafe { lt_macos_focused_app_id() })
    }

    fn extract_selection(
        &self,
        pointer: ScreenPoint,
        generation: u64,
        allow_clipboard_fallback: bool,
    ) -> Result<Option<ExtractionSnapshot>, ExtractionFailure> {
        let raw = take_owned_string(unsafe {
            lt_macos_extract_selection_json(pointer.x, pointer.y, allow_clipboard_fallback)
        })
        .ok_or(ExtractionFailure::NoText)?;

        let resp: NativeSelectionResponse =
            serde_json::from_str(&raw).map_err(|_| ExtractionFailure::NoText)?;

        match resp.status.as_str() {
            "permissionDenied" => Err(ExtractionFailure::PermissionDenied),
            "blockedSensitive" => Err(ExtractionFailure::BlockedSensitive),
            "ok" => {
                let text = resp.text.unwrap_or_default();
                if !is_allowed_input_text(&text) {
                    return Err(ExtractionFailure::TooLong);
                }
                let source = match resp.source.as_deref() {
                    Some("clipboardFallback") => ExtractionSource::ClipboardFallback,
                    _ => ExtractionSource::Accessibility,
                };
                Ok(Some(ExtractionSnapshot {
                    generation,
                    text,
                    context: None,
                    sentence_completeness: None,
                    scope: ExtractionScope::Selection,
                    completeness: ExtractionCompleteness::Complete,
                    source,
                    anchor_rect: resp.rect,
                    pointer,
                    app_id: resp.app_id,
                }))
            }
            _ => Ok(None),
        }
    }

    fn extract_hover_accessibility(
        &self,
        pointer: ScreenPoint,
        sentence_mode: bool,
        generation: u64,
    ) -> Result<ExtractionSnapshot, ExtractionFailure> {
        let raw = take_owned_string(unsafe { lt_macos_extract_hover_ax_json(pointer.x, pointer.y) })
            .ok_or(ExtractionFailure::PositionUnresolved)?;

        let resp: NativeHoverAxResponse =
            serde_json::from_str(&raw).map_err(|_| ExtractionFailure::PositionUnresolved)?;

        match resp.status.as_str() {
            "permissionDenied" => Err(ExtractionFailure::PermissionDenied),
            "blockedSensitive" => Err(ExtractionFailure::BlockedSensitive),
            "rangeResolved" => {
                let window_text = resp.window_text.unwrap_or_default();
                let hit_offset = resp.hit_offset.unwrap_or(0);
                let hit_len = resp.hit_length.unwrap_or(1).max(1);
                let win_start = resp.window_start.unwrap_or(0);

                // Resolve surrounding sentence first (using parent-aggregated paragraph if leaf was fragmented!)
                let mut resolved_sentence = SentenceResolver::resolve(
                    &SentenceWindow::bounded(&window_text, win_start, resp.element_total),
                    hit_offset,
                );

                // Upgrade to cross-node parent paragraph when leaf sentence was incomplete/fragmented
                if resolved_sentence
                    .as_ref()
                    .is_none_or(|s| s.completeness != ExtractionCompleteness::Complete)
                {
                    if let (Some(para_text), Some(para_offset)) =
                        (&resp.paragraph_text, resp.paragraph_hit_offset)
                    {
                        if let Some(para_sent) = SentenceResolver::resolve(
                            &SentenceWindow::complete_block(para_text),
                            para_offset,
                        ) {
                            resolved_sentence = Some(para_sent);
                        }
                    }
                }

                if sentence_mode {
                    if let Some(sent) = resolved_sentence {
                        return Ok(ExtractionSnapshot {
                            generation,
                            text: sent.text,
                            context: None,
                            sentence_completeness: Some(sent.completeness),
                            scope: ExtractionScope::Sentence,
                            completeness: sent.completeness,
                            source: ExtractionSource::Accessibility,
                            anchor_rect: resp.rect,
                            pointer,
                            app_id: resp.app_id,
                        });
                    }
                }

                // Word extraction with punctuation/space span & ±1 edge tolerance
                if let Some((_, word)) = resolve_word_in_utf16(
                    &window_text,
                    Utf16Range::new(hit_offset, hit_len),
                    true,
                ) {
                    let (ctx_text, ctx_comp) = match resolved_sentence {
                        Some(s) => (Some(s.text), Some(s.completeness)),
                        None => (None, None),
                    };
                    return Ok(ExtractionSnapshot {
                        generation,
                        text: word,
                        context: ctx_text,
                        sentence_completeness: ctx_comp,
                        scope: ExtractionScope::Word,
                        completeness: ExtractionCompleteness::Complete,
                        source: ExtractionSource::Accessibility,
                        anchor_rect: resp.rect,
                        pointer,
                        app_id: resp.app_id,
                    });
                }

                Err(ExtractionFailure::PositionUnresolved)
            }
            "labelResolved" => {
                let label = resp.label.unwrap_or_default();
                if label.is_empty() {
                    return Err(ExtractionFailure::NoText);
                }
                let word_count = label.split_whitespace().count();
                let scope = if word_count <= 1 {
                    ExtractionScope::Word
                } else {
                    ExtractionScope::Label
                };
                Ok(ExtractionSnapshot {
                    generation,
                    text: label,
                    context: None,
                    sentence_completeness: None,
                    scope,
                    completeness: ExtractionCompleteness::Complete,
                    source: ExtractionSource::Accessibility,
                    anchor_rect: resp.rect,
                    pointer,
                    app_id: resp.app_id,
                })
            }
            "notSupported" => Err(ExtractionFailure::NotSupported),
            _ => Err(ExtractionFailure::PositionUnresolved),
        }
    }

    fn extract_hover_ocr(
        &self,
        pointer: ScreenPoint,
        sentence_mode: bool,
        generation: u64,
    ) -> Result<ExtractionSnapshot, ExtractionFailure> {
        let raw = take_owned_string(unsafe {
            lt_macos_ocr_at_point_json(pointer.x, pointer.y, true)
        })
        .ok_or(ExtractionFailure::NoText)?;

        let resp: NativeOcrResponse =
            serde_json::from_str(&raw).map_err(|_| ExtractionFailure::NoText)?;

        if resp.status == "permissionDenied" {
            return Err(ExtractionFailure::PermissionDenied);
        }
        if resp.lines.is_empty() {
            return Err(ExtractionFailure::NoText);
        }

        let hit = OcrClusterer::extract_at_point(pointer, &resp.lines)
            .ok_or(ExtractionFailure::NoText)?;

        if sentence_mode {
            let text = hit.sentence_text.unwrap_or(hit.word.text);
            return Ok(ExtractionSnapshot {
                generation,
                text,
                context: None,
                sentence_completeness: Some(hit.sentence_completeness),
                scope: ExtractionScope::Sentence,
                completeness: hit.sentence_completeness,
                source: ExtractionSource::Ocr,
                anchor_rect: Some(hit.sentence_rect),
                pointer,
                app_id: resp.app_id,
            });
        }

        Ok(ExtractionSnapshot {
            generation,
            text: hit.word.text,
            context: hit.sentence_text,
            sentence_completeness: Some(hit.sentence_completeness),
            scope: ExtractionScope::Word,
            completeness: ExtractionCompleteness::Complete,
            source: ExtractionSource::Ocr,
            anchor_rect: Some(hit.word.rect),
            pointer,
            app_id: resp.app_id,
        })
    }

    fn extract_region_ocr(
        &self,
        rect: ScreenRect,
        generation: u64,
    ) -> Result<ExtractionSnapshot, ExtractionFailure> {
        let raw = take_owned_string(unsafe {
            lt_macos_ocr_region_json(rect.x, rect.y, rect.width, rect.height)
        })
        .ok_or(ExtractionFailure::NoText)?;

        let resp: NativeOcrResponse =
            serde_json::from_str(&raw).map_err(|_| ExtractionFailure::NoText)?;

        if resp.status == "permissionDenied" {
            return Err(ExtractionFailure::PermissionDenied);
        }

        let text =
            OcrClusterer::assemble_region_text(&resp.lines).ok_or(ExtractionFailure::NoText)?;
        if !is_allowed_input_text(&text) {
            return Err(ExtractionFailure::TooLong);
        }

        Ok(ExtractionSnapshot {
            generation,
            text,
            context: None,
            sentence_completeness: None,
            scope: ExtractionScope::RegionOcr,
            completeness: ExtractionCompleteness::Complete,
            source: ExtractionSource::Ocr,
            anchor_rect: Some(rect),
            pointer: ScreenPoint::new(rect.mid_x(), rect.max_y()),
            app_id: None,
        })
    }

    fn permissions_status(&self, prompt_accessibility: bool) -> PermissionsStatus {
        unsafe {
            PermissionsStatus {
                platform: "macos".into(),
                accessibility: lt_macos_check_accessibility(prompt_accessibility),
                screen_recording: lt_macos_check_screen_recording(),
            }
        }
    }

    fn request_screen_recording(&self) -> bool {
        unsafe { lt_macos_request_screen_recording() }
    }

    fn write_clipboard(&self, text: &str) -> Result<(), String> {
        let c_str = CString::new(text).map_err(|e| e.to_string())?;
        let ok = unsafe { lt_macos_write_clipboard(c_str.as_ptr()) };
        if ok {
            Ok(())
        } else {
            Err("写入剪贴板失败".into())
        }
    }

    fn configure_popup_window(&self, window: &tauri::WebviewWindow) {
        if let Ok(ns_win) = window.ns_window() {
            unsafe {
                lt_macos_configure_popup_window(ns_win);
            }
        }
    }

    fn present_popup(&self, window: &tauri::WebviewWindow, rect: ScreenRect, interactive: bool) {
        if let Ok(ns_win) = window.ns_window() {
            unsafe {
                lt_macos_present_popup_window(
                    ns_win,
                    rect.x,
                    rect.y,
                    rect.width,
                    rect.height,
                    interactive,
                );
            }
        }
    }

    fn set_popup_interactive(&self, window: &tauri::WebviewWindow, interactive: bool) {
        if let Ok(ns_win) = window.ns_window() {
            unsafe {
                lt_macos_set_popup_interactive(ns_win, interactive);
            }
        }
    }

    fn hide_popup(&self, window: &tauri::WebviewWindow) {
        if let Ok(ns_win) = window.ns_window() {
            unsafe {
                lt_macos_hide_popup_window(ns_win);
            }
        }
    }
}
