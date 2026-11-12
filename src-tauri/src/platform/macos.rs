use crate::extraction::resolver::{
    is_single_word_label, resolve_word_in_utf16, OcrClusterer, OcrLine, SentenceResolver,
    SentenceWindow, Utf16Range,
};
use crate::extraction::{
    is_allowed_input_text, ExtractionCompleteness, ExtractionFailure, ExtractionScope,
    ExtractionSnapshot, ExtractionSource, ScreenPoint, ScreenRect,
};
use crate::overlay::CapturedImage;
use crate::platform::{
    InputEventCallback, PermissionsStatus, PlatformBackend, PlatformInputEvent, ShortcutSpec,
};
use crate::storage::TriggerModifier;
use parking_lot::Mutex;
use serde::Deserialize;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::LazyLock;

unsafe extern "C" {
    fn lt_macos_start_input_monitor(callback: extern "C" fn(*const c_char));
    fn lt_macos_update_trigger_config(
        modifier_kind: i32,
        double_tap_selection: bool,
        selection_mods: u32,
        selection_key: *const c_char,
        capture_mods: u32,
        capture_key: *const c_char,
    );
    fn lt_macos_set_shortcut_recording(active: bool);
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
    fn lt_macos_capture_region_rgba(
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        encode_jpeg: bool,
        out_rgba: *mut *mut u8,
        out_pw: *mut u32,
        out_ph: *mut u32,
        out_jpeg: *mut *mut u8,
        out_jpeg_len: *mut usize,
    ) -> bool;
    fn lt_macos_ocr_rgba_json(
        rgba: *const u8,
        pw: u32,
        ph: u32,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    ) -> *mut c_char;
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
    fn lt_macos_configure_overlay_window(ns_window_ptr: *mut std::ffi::c_void);
    fn lt_macos_present_overlay_window(
        ns_window_ptr: *mut std::ffi::c_void,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    );
    fn lt_macos_hide_overlay_window(ns_window_ptr: *mut std::ffi::c_void);
    fn lt_macos_free_string(ptr: *mut c_char);
    fn lt_macos_free_bytes(ptr: *mut u8);
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
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    pointer_inside: Option<bool>,
    rect: Option<ScreenRect>,
    #[serde(default)]
    elem_rect: Option<ScreenRect>,
    app_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeHoverAxResponse {
    status: String,
    #[allow(dead_code)]
    role: Option<String>,
    window_text: Option<String>,
    window_start: Option<usize>,
    element_total: Option<usize>,
    hit_offset: Option<usize>,
    hit_length: Option<usize>,
    paragraph_text: Option<String>,
    paragraph_hit_offset: Option<usize>,
    label: Option<String>,
    word_rect: Option<ScreenRect>,
    char_rect: Option<ScreenRect>,
    elem_rect: Option<ScreenRect>,
    rect: Option<ScreenRect>,
    app_id: Option<String>,
}

fn is_single_line_input_role(role: Option<&str>) -> bool {
    matches!(
        role,
        Some("AXTextField" | "AXSearchField" | "AXComboBox")
    )
}

fn resolve_selection_response(
    resp: NativeSelectionResponse,
    pointer: ScreenPoint,
    generation: u64,
    require_pointer_inside: bool,
) -> Result<Option<ExtractionSnapshot>, ExtractionFailure> {
    match resp.status.as_str() {
        "permissionDenied" => Err(ExtractionFailure::PermissionDenied),
        "blockedSensitive" => Err(ExtractionFailure::BlockedSensitive),
        "ok" => {
            if require_pointer_inside {
                if is_single_line_input_role(resp.role.as_deref()) {
                    let hit_box = resp
                        .elem_rect
                        .filter(|r| r.is_valid())
                        .or_else(|| resp.rect.filter(|r| r.is_valid()));
                    let inside = hit_box.is_some_and(|r| r.inset_by(-16.0, -16.0).contains(pointer));
                    if !inside {
                        return Ok(None);
                    }
                } else if resp.role.is_none() {
                    let inside = match resp.pointer_inside {
                        Some(flag) => flag,
                        None => resp
                            .rect
                            .is_some_and(|r| r.is_valid() && r.inset_by(-6.0, -6.0).contains(pointer)),
                    };
                    if !inside {
                        return Ok(None);
                    }
                }
                // For document/content roles (AXWebArea, AXTextArea, AXStaticText, AXGroup, etc.),
                // any active non-empty selection in the focused app under the pointer is prioritized
                // even when dragging ends outside text bounds or AX bounds are unavailable.
            }
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

fn resolve_hover_ax_response(
    resp: NativeHoverAxResponse,
    pointer: ScreenPoint,
    sentence_mode: bool,
    generation: u64,
) -> Result<ExtractionSnapshot, ExtractionFailure> {
    match resp.status.as_str() {
        "permissionDenied" => Err(ExtractionFailure::PermissionDenied),
        "blockedSensitive" => Err(ExtractionFailure::BlockedSensitive),
        "rangeResolved" => {
            let hit_len = resp.hit_length.unwrap_or(0);
            if hit_len == 0 || hit_len > 128 {
                return Err(ExtractionFailure::PositionUnresolved);
            }

            let hit_offset = resp.hit_offset.unwrap_or(0);
            let valid_word_rect = resp.word_rect.filter(|r| r.is_valid());
            let valid_char_rect = resp.char_rect.filter(|r| r.is_valid());
            let valid_elem_rect = resp
                .elem_rect
                .or(resp.rect)
                .filter(|r| r.is_valid());

            // Geometric verification: reject clamped/bogus character indices far from `pointer`
            if let Some(cr) = valid_char_rect {
                if !cr.inset_by(-24.0, -14.0).contains(pointer) {
                    return Err(ExtractionFailure::PositionUnresolved);
                }
            } else if let Some(wr) = valid_word_rect {
                if !wr.inset_by(-24.0, -14.0).contains(pointer) {
                    return Err(ExtractionFailure::PositionUnresolved);
                }
            } else if let Some(er) = valid_elem_rect {
                if !er.inset_by(-8.0, -8.0).contains(pointer) {
                    return Err(ExtractionFailure::PositionUnresolved);
                }
                if hit_offset == 0 && er.width > 80.0 && (pointer.x - er.x) > 48.0 {
                    return Err(ExtractionFailure::PositionUnresolved);
                }
            }

            let window_text = resp.window_text.unwrap_or_default();
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
                        anchor_rect: valid_elem_rect.or(valid_word_rect).or(valid_char_rect),
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
                // Never use a multi-word container's `elem_rect` as a word `anchor_rect`
                let compact_single_elem = valid_elem_rect.filter(|r| {
                    is_single_word_label(&window_text) && r.width <= 160.0 && r.height <= 44.0
                });
                let word_anchor = valid_word_rect.or(valid_char_rect).or(compact_single_elem);

                return Ok(ExtractionSnapshot {
                    generation,
                    text: word,
                    context: ctx_text,
                    sentence_completeness: ctx_comp,
                    scope: ExtractionScope::Word,
                    completeness: ExtractionCompleteness::Complete,
                    source: ExtractionSource::Accessibility,
                    anchor_rect: word_anchor,
                    pointer,
                    app_id: resp.app_id,
                });
            }

            Err(ExtractionFailure::PositionUnresolved)
        }
        "labelResolved" => {
            let label = resp.label.unwrap_or_default().trim().to_string();
            if label.is_empty() {
                return Err(ExtractionFailure::NoText);
            }
            let elem_rect = resp.elem_rect.or(resp.rect).filter(|r| r.is_valid());
            if let Some(er) = elem_rect {
                if !er.inset_by(-6.0, -6.0).contains(pointer) {
                    return Err(ExtractionFailure::PositionUnresolved);
                }
            }
            let single_word = is_single_word_label(&label);
            let scope = if single_word {
                ExtractionScope::Word
            } else if sentence_mode {
                ExtractionScope::Sentence
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
                anchor_rect: elem_rect,
                pointer,
                app_id: resp.app_id,
            })
        }
        "notSupported" => Err(ExtractionFailure::NotSupported),
        _ => Err(ExtractionFailure::PositionUnresolved),
    }
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

    fn update_trigger_config(
        &self,
        modifier: TriggerModifier,
        double_tap_selection: bool,
        selection_shortcut: &str,
        capture_shortcut: &str,
    ) {
        let (sel_mods, sel_key) = match ShortcutSpec::parse(selection_shortcut) {
            Some(spec) => (
                spec.modifier_mask(),
                CString::new(spec.key).unwrap_or_default(),
            ),
            None => (0, CString::default()),
        };
        let (cap_mods, cap_key) = match ShortcutSpec::parse(capture_shortcut) {
            Some(spec) => (
                spec.modifier_mask(),
                CString::new(spec.key).unwrap_or_default(),
            ),
            None => (0, CString::default()),
        };

        unsafe {
            lt_macos_update_trigger_config(
                modifier_to_code(modifier),
                double_tap_selection,
                sel_mods,
                sel_key.as_ptr(),
                cap_mods,
                cap_key.as_ptr(),
            );
        }
    }

    fn set_shortcut_recording(&self, active: bool) {
        unsafe {
            lt_macos_set_shortcut_recording(active);
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

        resolve_selection_response(resp, pointer, generation, false)
    }

    fn extract_hovered_selection(
        &self,
        pointer: ScreenPoint,
        generation: u64,
    ) -> Result<Option<ExtractionSnapshot>, ExtractionFailure> {
        let raw = take_owned_string(unsafe {
            lt_macos_extract_selection_json(pointer.x, pointer.y, false)
        })
        .ok_or(ExtractionFailure::NoText)?;

        let resp: NativeSelectionResponse =
            serde_json::from_str(&raw).map_err(|_| ExtractionFailure::NoText)?;

        resolve_selection_response(resp, pointer, generation, true)
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

        resolve_hover_ax_response(resp, pointer, sentence_mode, generation)
    }

    fn extract_hover_ocr(
        &self,
        pointer: ScreenPoint,
        sentence_mode: bool,
        generation: u64,
    ) -> Result<ExtractionSnapshot, ExtractionFailure> {
        let raw = take_owned_string(unsafe {
            lt_macos_ocr_at_point_json(pointer.x, pointer.y, sentence_mode)
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

    fn capture_region_image(
        &self,
        rect: ScreenRect,
        include_jpeg_preview: bool,
    ) -> Option<CapturedImage> {
        let mut rgba_ptr: *mut u8 = std::ptr::null_mut();
        let mut pw: u32 = 0;
        let mut ph: u32 = 0;
        let mut jpeg_ptr: *mut u8 = std::ptr::null_mut();
        let mut jpeg_len: usize = 0;

        let ok = unsafe {
            lt_macos_capture_region_rgba(
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                include_jpeg_preview,
                &mut rgba_ptr,
                &mut pw,
                &mut ph,
                &mut jpeg_ptr,
                &mut jpeg_len,
            )
        };
        if !ok || rgba_ptr.is_null() || pw == 0 || ph == 0 {
            if !rgba_ptr.is_null() {
                unsafe { lt_macos_free_bytes(rgba_ptr) };
            }
            if !jpeg_ptr.is_null() {
                unsafe { lt_macos_free_bytes(jpeg_ptr) };
            }
            return None;
        }

        let byte_len = (pw as usize) * (ph as usize) * 4;
        let rgba = unsafe { std::slice::from_raw_parts(rgba_ptr, byte_len) }.to_vec();
        unsafe { lt_macos_free_bytes(rgba_ptr) };

        let preview_jpeg = if !jpeg_ptr.is_null() && jpeg_len > 0 {
            let j = unsafe { std::slice::from_raw_parts(jpeg_ptr, jpeg_len) }.to_vec();
            unsafe { lt_macos_free_bytes(jpeg_ptr) };
            Some(j)
        } else {
            if !jpeg_ptr.is_null() {
                unsafe { lt_macos_free_bytes(jpeg_ptr) };
            }
            None
        };

        Some(CapturedImage {
            rect,
            pixel_width: pw,
            pixel_height: ph,
            rgba,
            preview_jpeg,
        })
    }

    fn ocr_captured_image(&self, image: &CapturedImage) -> Result<Vec<OcrLine>, ExtractionFailure> {
        let expected = (image.pixel_width as usize) * (image.pixel_height as usize) * 4;
        if image.pixel_width < 4 || image.pixel_height < 4 || image.rgba.len() < expected {
            return Err(ExtractionFailure::NoText);
        }

        let raw = take_owned_string(unsafe {
            lt_macos_ocr_rgba_json(
                image.rgba.as_ptr(),
                image.pixel_width,
                image.pixel_height,
                image.rect.x,
                image.rect.y,
                image.rect.width,
                image.rect.height,
            )
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
        Ok(resp.lines)
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

    fn configure_overlay_window(&self, window: &tauri::WebviewWindow) {
        if let Ok(ns_win) = window.ns_window() {
            unsafe {
                lt_macos_configure_overlay_window(ns_win);
            }
        }
    }

    fn present_overlay(&self, window: &tauri::WebviewWindow, rect: ScreenRect) {
        if let Ok(ns_win) = window.ns_window() {
            unsafe {
                lt_macos_present_overlay_window(ns_win, rect.x, rect.y, rect.width, rect.height);
            }
        }
    }

    fn hide_overlay(&self, window: &tauri::WebviewWindow) {
        if let Ok(ns_win) = window.ns_window() {
            unsafe {
                lt_macos_hide_overlay_window(ns_win);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_length_or_clamped_ax_range_on_chrome_flags_heading() {
        let input_pointer = ScreenPoint::new(412.0, 331.0);
        let heading_rect = ScreenRect::new(180.0, 320.0, 260.0, 22.0);

        // Repro 1: Chrome `chrome://flags` heading returns zero-length CFRange(0, 0) while cursor is on "Input"
        let zero_len_resp = NativeHoverAxResponse {
            status: "rangeResolved".into(),
            role: Some("AXStaticText".into()),
            window_text: Some("Prompt API Multimodal Input".into()),
            window_start: Some(0),
            element_total: Some(27),
            hit_offset: Some(0),
            hit_length: Some(0),
            paragraph_text: None,
            paragraph_hit_offset: None,
            label: None,
            word_rect: None,
            char_rect: None,
            elem_rect: Some(heading_rect),
            rect: Some(heading_rect),
            app_id: Some("com.google.Chrome".into()),
        };
        assert_eq!(
            resolve_hover_ax_response(zero_len_resp, input_pointer, false, 1),
            Err(ExtractionFailure::PositionUnresolved)
        );

        // Repro 2: Control clamps RangeForPosition to offset 0 ("P" of "Prompt") far left of pointer
        let clamped_resp = NativeHoverAxResponse {
            status: "rangeResolved".into(),
            role: Some("AXStaticText".into()),
            window_text: Some("Prompt API Multimodal Input".into()),
            window_start: Some(0),
            element_total: Some(27),
            hit_offset: Some(0),
            hit_length: Some(1),
            paragraph_text: None,
            paragraph_hit_offset: None,
            label: None,
            word_rect: Some(ScreenRect::new(180.0, 320.0, 58.0, 20.0)),
            char_rect: Some(ScreenRect::new(180.0, 320.0, 9.0, 20.0)),
            elem_rect: Some(heading_rect),
            rect: Some(ScreenRect::new(180.0, 320.0, 58.0, 20.0)),
            app_id: Some("com.google.Chrome".into()),
        };
        assert_eq!(
            resolve_hover_ax_response(clamped_resp, input_pointer, false, 2),
            Err(ExtractionFailure::PositionUnresolved)
        );

        // Repro 3: BoundsForRange unavailable, offset 0 reported while pointer is 232pt right of heading origin
        let unverified_zero_offset = NativeHoverAxResponse {
            status: "rangeResolved".into(),
            role: Some("AXStaticText".into()),
            window_text: Some("Prompt API Multimodal Input".into()),
            window_start: Some(0),
            element_total: Some(27),
            hit_offset: Some(0),
            hit_length: Some(1),
            paragraph_text: None,
            paragraph_hit_offset: None,
            label: None,
            word_rect: None,
            char_rect: None,
            elem_rect: Some(heading_rect),
            rect: None,
            app_id: Some("com.google.Chrome".into()),
        };
        assert_eq!(
            resolve_hover_ax_response(unverified_zero_offset, input_pointer, false, 3),
            Err(ExtractionFailure::PositionUnresolved)
        );
    }

    #[test]
    fn resolves_exact_hovered_word_and_never_uses_multiword_container_rect_as_word_anchor() {
        let input_pointer = ScreenPoint::new(412.0, 331.0);
        let input_word_rect = ScreenRect::new(392.0, 320.0, 42.0, 20.0);
        let heading_rect = ScreenRect::new(180.0, 320.0, 260.0, 22.0);

        // Offset 24 is 'p' inside "Prompt API Multimodal Input" ("Inp|ut")
        let valid_resp = NativeHoverAxResponse {
            status: "rangeResolved".into(),
            role: Some("AXStaticText".into()),
            window_text: Some("Prompt API Multimodal Input".into()),
            window_start: Some(0),
            element_total: Some(27),
            hit_offset: Some(24),
            hit_length: Some(1),
            paragraph_text: None,
            paragraph_hit_offset: None,
            label: None,
            word_rect: Some(input_word_rect),
            char_rect: Some(ScreenRect::new(408.0, 320.0, 8.0, 20.0)),
            elem_rect: Some(heading_rect),
            rect: Some(input_word_rect),
            app_id: Some("com.google.Chrome".into()),
        };
        let snap = resolve_hover_ax_response(valid_resp, input_pointer, false, 7)
            .expect("must resolve exact word Input");
        assert_eq!(snap.text, "Input");
        assert_eq!(snap.scope, ExtractionScope::Word);
        assert_eq!(snap.anchor_rect, Some(input_word_rect));

        // Even if wordRect/charRect are missing for a valid non-zero offset, multi-word elemRect is NOT used as word anchor
        let no_sub_bounds = NativeHoverAxResponse {
            status: "rangeResolved".into(),
            role: Some("AXStaticText".into()),
            window_text: Some("Prompt API Multimodal Input".into()),
            window_start: Some(0),
            element_total: Some(27),
            hit_offset: Some(24),
            hit_length: Some(1),
            paragraph_text: None,
            paragraph_hit_offset: None,
            label: None,
            word_rect: None,
            char_rect: None,
            elem_rect: Some(heading_rect),
            rect: None,
            app_id: Some("com.google.Chrome".into()),
        };
        let snap2 = resolve_hover_ax_response(no_sub_bounds, input_pointer, false, 8)
            .expect("must resolve word without container anchor");
        assert_eq!(snap2.text, "Input");
        assert_eq!(snap2.anchor_rect, None);
    }

    #[test]
    fn classifies_multiword_and_slug_labels_for_ocr_refinement_and_filters_distant_selection() {
        let pt = ScreenPoint::new(412.0, 331.0);
        let rect = ScreenRect::new(180.0, 320.0, 260.0, 22.0);

        let multi = NativeHoverAxResponse {
            status: "labelResolved".into(),
            role: Some("AXStaticText".into()),
            window_text: None,
            window_start: None,
            element_total: None,
            hit_offset: None,
            hit_length: None,
            paragraph_text: None,
            paragraph_hit_offset: None,
            label: Some("Prompt API Multimodal Input".into()),
            word_rect: None,
            char_rect: None,
            elem_rect: Some(rect),
            rect: Some(rect),
            app_id: Some("com.google.Chrome".into()),
        };
        let multi_snap = resolve_hover_ax_response(multi, pt, false, 1).unwrap();
        assert_eq!(multi_snap.scope, ExtractionScope::Label);

        let slug = NativeHoverAxResponse {
            status: "labelResolved".into(),
            role: Some("AXLink".into()),
            window_text: None,
            window_start: None,
            element_total: None,
            hit_offset: None,
            hit_length: None,
            paragraph_text: None,
            paragraph_hit_offset: None,
            label: Some("#prompt-api-for-gemini-nano-multimodal-input".into()),
            word_rect: None,
            char_rect: None,
            elem_rect: Some(rect),
            rect: Some(rect),
            app_id: Some("com.google.Chrome".into()),
        };
        let slug_snap = resolve_hover_ax_response(slug, pt, false, 2).unwrap();
        assert_eq!(slug_snap.scope, ExtractionScope::Label);

        let single_btn = NativeHoverAxResponse {
            status: "labelResolved".into(),
            role: Some("AXButton".into()),
            window_text: None,
            window_start: None,
            element_total: None,
            hit_offset: None,
            hit_length: None,
            paragraph_text: None,
            paragraph_hit_offset: None,
            label: Some("Default".into()),
            word_rect: None,
            char_rect: None,
            elem_rect: Some(ScreenRect::new(400.0, 320.0, 64.0, 24.0)),
            rect: Some(ScreenRect::new(400.0, 320.0, 64.0, 24.0)),
            app_id: Some("com.google.Chrome".into()),
        };
        let single_snap = resolve_hover_ax_response(single_btn, pt, false, 3).unwrap();
        assert_eq!(single_snap.scope, ExtractionScope::Word);

        // Distant selection in Chrome search box must NOT hijack hover, while explicit selection trigger still reads it
        let distant_sel = NativeSelectionResponse {
            status: "ok".into(),
            text: Some("Prompt".into()),
            source: Some("accessibility".into()),
            role: Some("AXSearchField".into()),
            pointer_inside: Some(false),
            rect: Some(ScreenRect::new(240.0, 110.0, 56.0, 20.0)),
            elem_rect: Some(ScreenRect::new(220.0, 102.0, 320.0, 32.0)),
            app_id: Some("com.google.Chrome".into()),
        };
        assert_eq!(resolve_selection_response(distant_sel, pt, 4, true), Ok(None));

        // Web/body selection (AXWebArea) without bounds or with cursor slightly past end MUST route to Selection!
        let web_body_sel = NativeSelectionResponse {
            status: "ok".into(),
            text: Some("Selected article sentence".into()),
            source: Some("accessibility".into()),
            role: Some("AXWebArea".into()),
            pointer_inside: Some(false),
            rect: None,
            elem_rect: None,
            app_id: Some("com.google.Chrome".into()),
        };
        assert_eq!(
            resolve_selection_response(web_body_sel, pt, 5, true)
                .unwrap()
                .unwrap()
                .text,
            "Selected article sentence"
        );

        let explicit_sel = NativeSelectionResponse {
            status: "ok".into(),
            text: Some("Prompt".into()),
            source: Some("accessibility".into()),
            role: Some("AXSearchField".into()),
            pointer_inside: Some(false),
            rect: Some(ScreenRect::new(240.0, 110.0, 56.0, 20.0)),
            elem_rect: Some(ScreenRect::new(220.0, 102.0, 320.0, 32.0)),
            app_id: Some("com.google.Chrome".into()),
        };
        assert_eq!(
            resolve_selection_response(explicit_sel, pt, 6, false)
                .unwrap()
                .unwrap()
                .text,
            "Prompt"
        );
    }
}
