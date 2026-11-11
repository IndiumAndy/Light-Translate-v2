pub mod coordinator;
pub mod dictionary;
pub mod extraction;
pub mod overlay;
pub mod platform;
pub mod storage;
pub mod translation;

use coordinator::{
    compute_popup_rect, CoordinatorStateMachine, PopupCardViewState, RequestGate,
    StateTransitionAction,
};
use dictionary::WordBook;
use extraction::resolver::OcrClusterer;
use extraction::{
    is_allowed_input_text, ExtractionCompleteness, ExtractionFailure, ExtractionScope,
    ExtractionSnapshot, ScreenPoint, ScreenRect,
};
use futures_util::StreamExt;
use overlay::{
    build_initial_overlay_blocks, compute_overlay_window_layout, crop_captured_image,
    encode_rgba_to_png, CaptureOverlayViewState, CapturedImage,
};
use parking_lot::Mutex;
use platform::{
    create_platform_backend, PermissionsStatus, PlatformBackend, PlatformInputEvent, ShortcutSpec,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use storage::{
    AppSettings, AppStorage, SaveOutcome, SavedEntry, SecretStore, SystemKeyringStore,
    UsageStatsSummary,
};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, State};
use translation::{
    stream_llm_completion, test_llm_connection, translate_blocks_llm_batch, translate_fast_machine,
    CacheKey, ConnectionTestResult, LlmProviderKind, QueryMode, TranslationCache,
};
use uuid::Uuid;

const DEFAULT_POPUP_WIDTH: f64 = 360.0;

fn estimate_initial_popup_height(card: &PopupCardViewState) -> f64 {
    let mut h = if card.scope == ExtractionScope::Word {
        76.0
    } else {
        112.0
    };
    if let Some(dict) = &card.dictionary {
        h += (dict.senses.len().min(4) as f64) * 24.0 + 8.0;
    }
    if card.interactive {
        h += 38.0;
    }
    h.clamp(60.0, 260.0)
}

fn format_capture_tray_label(capture_shortcut: &str) -> String {
    match ShortcutSpec::parse(capture_shortcut) {
        Some(spec) => format!(
            "框选截图翻译 ({})",
            spec.format_display(cfg!(target_os = "macos"))
        ),
        None => "框选截图翻译".into(),
    }
}

#[derive(Default)]
pub struct OverlayRuntimeState {
    pub view: CaptureOverlayViewState,
    pub window_rect: Option<ScreenRect>,
    pub frozen_monitor: Option<CapturedImage>,
    pub cropped_png: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureActivatePayload {
    pub monitor: ScreenRect,
    pub has_backdrop: bool,
}

pub struct AppController {
    pub app: AppHandle,
    pub platform: Arc<dyn PlatformBackend>,
    pub wordbook: WordBook,
    pub storage: AppStorage,
    pub secrets: Box<dyn SecretStore>,
    pub cache: TranslationCache,
    pub gate: RequestGate,
    pub overlay_gate: RequestGate,
    pub state: Mutex<CoordinatorStateMachine>,
    pub overlay_state: Mutex<OverlayRuntimeState>,
    pub trigger_press_count: AtomicU64,
    pub tray_toggle_item: Mutex<Option<CheckMenuItem<tauri::Wry>>>,
    pub tray_capture_item: Mutex<Option<MenuItem<tauri::Wry>>>,
}

pub(crate) fn execute_hover_extraction(
    platform: &dyn PlatformBackend,
    pointer: ScreenPoint,
    sentence_mode: bool,
    ocr_enabled: bool,
    gen: u64,
) -> Result<ExtractionSnapshot, ExtractionFailure> {
    // 1. Selection-first routing ONLY when the pointer is resting on the active selection itself
    let focused_id = platform.focused_app_id();
    let point_id = platform.app_id_at_point(pointer);
    if focused_id.is_some() && focused_id == point_id {
        if let Ok(Some(sel_snap)) = platform.extract_hovered_selection(pointer, gen) {
            return Ok(sel_snap);
        }
    }

    // 2. Native Accessibility hover extraction
    match platform.extract_hover_accessibility(pointer, sentence_mode, gen) {
        Ok(ax_snap) => {
            // When hovering for a single word (!sentence_mode), if Accessibility could only return a
            // multi-word heading/label without character-level position mapping (e.g. Chrome `chrome://flags`
            // headings like "Prompt API Multimodal Input"), refine to the exact hovered word via local OCR;
            // gracefully keep `ax_snap` if OCR is disabled or unavailable.
            if !sentence_mode && ocr_enabled && ax_snap.scope == ExtractionScope::Label {
                if let Ok(ocr_snap) = platform.extract_hover_ocr(pointer, false, gen) {
                    return Ok(ocr_snap);
                }
            }
            Ok(ax_snap)
        }
        Err(err) if ocr_enabled && err.should_fallback_to_ocr() => {
            // 3. Instant zero-cooldown local offline OCR fallback!
            platform.extract_hover_ocr(pointer, sentence_mode, gen)
        }
        Err(err) => Err(err),
    }
}

impl AppController {
    pub fn sync_tray_items(&self, settings: &AppSettings) {
        if let Some(toggle) = self.tray_toggle_item.lock().as_ref() {
            let _ = toggle.set_checked(settings.enabled);
        }
        if let Some(capture) = self.tray_capture_item.lock().as_ref() {
            let _ = capture.set_text(format_capture_tray_label(&settings.capture_shortcut));
        }
    }
    pub fn write_diagnostics(&self, last_event: &str, last_outcome: &str, target_app: Option<&str>) {
        let perms = self.platform.permissions_status(false);
        let payload = serde_json::json!({
            "pid": std::process::id(),
            "accessibility": perms.accessibility,
            "screenRecording": perms.screen_recording,
            "triggerPressCount": self.trigger_press_count.load(Ordering::Relaxed),
            "lastEvent": last_event,
            "lastOutcome": last_outcome,
            "lastTargetApp": target_app,
        });
        if let Ok(bytes) = serde_json::to_vec_pretty(&payload) {
            let _ = std::fs::write(self.storage.directory().join("Diagnostics.json"), bytes);
        }
    }

    pub fn emit_popup_state(&self, card: &PopupCardViewState) {
        let _ = self.app.emit_to("popup", "popup://state", card);
    }

    pub fn emit_overlay_state(&self, view: &CaptureOverlayViewState) {
        let _ = self.app.emit_to("overlay", "overlay://state", view);
    }

    pub fn record_usage(&self, source_text: &str) {
        if let Some(summary) = self.storage.record_translation_usage(source_text) {
            let _ = self.app.emit_to("main", "stats://updated", &summary);
        }
    }

    pub fn dismiss_overlay(&self) {
        self.overlay_gate.invalidate();
        let had_visible = {
            let mut guard = self.overlay_state.lock();
            let was_vis = guard.view.visible;
            guard.view = CaptureOverlayViewState::default();
            guard.window_rect = None;
            guard.cropped_png = None;
            was_vis
        };
        if let Some(win) = self.app.get_webview_window("overlay") {
            self.platform.hide_overlay(&win);
        }
        if had_visible {
            self.emit_overlay_state(&CaptureOverlayViewState::default());
        }
    }

    pub fn handle_input_event(self: &Arc<Self>, event: PlatformInputEvent) {
        let settings = self.storage.settings();

        match event {
            PlatformInputEvent::EscapePressed => {
                // 1. Hide fullscreen capture selector if active
                if let Some(cap) = self.app.get_webview_window("capture") {
                    if cap.is_visible().unwrap_or(false) {
                        self.overlay_state.lock().frozen_monitor = None;
                        let _ = cap.hide();
                        return;
                    }
                }
                // 2. Dismiss in-place screenshot translation overlay if visible
                if self.overlay_state.lock().view.visible {
                    self.dismiss_overlay();
                    return;
                }
                // 3. Dismiss hover/selection popup card
                let action = self.state.lock().on_escape();
                self.apply_transition(action);
            }
            PlatformInputEvent::ShortcutCapture => {
                if settings.enabled {
                    let ctrl = Arc::clone(self);
                    tauri::async_runtime::spawn(async move {
                        ctrl.open_capture_overlay(false).await;
                    });
                }
            }
            PlatformInputEvent::DoubleTapSelection { pointer }
            | PlatformInputEvent::ShortcutSelection { pointer } => {
                if settings.enabled {
                    let ctrl = Arc::clone(self);
                    tauri::async_runtime::spawn(async move {
                        ctrl.run_selection_flow(pointer, true).await;
                    });
                }
            }
            PlatformInputEvent::OtherKeyPressed => {
                let action = self.state.lock().on_other_key_pressed();
                self.apply_transition(action);
            }
            PlatformInputEvent::TriggerPressed {
                sentence_modifier,
                pointer,
            } => {
                self.trigger_press_count.fetch_add(1, Ordering::Relaxed);
                if !settings.enabled {
                    self.write_diagnostics("triggerPressed", "disabledInSettings", None);
                    return;
                }
                self.write_diagnostics("triggerPressed", "dwellArmed", None);
                let action = self.state.lock().on_trigger_pressed(
                    pointer,
                    sentence_modifier,
                    Instant::now(),
                    settings.hover_dwell_ms,
                );
                self.apply_transition(action);
            }
            PlatformInputEvent::TriggerReleased { pointer, clean_tap } => {
                let (action, card_opt, cancel_inflight_hover) = {
                    let mut guard = self.state.lock();
                    let act = guard.on_trigger_released(
                        pointer,
                        clean_tap,
                        Instant::now(),
                        settings.dismiss_on_trigger_release,
                        settings.linger_dismiss_ms,
                    );
                    let updated_card = guard.card.visible.then(|| guard.card.clone());
                    let cancel_inflight = settings.dismiss_on_trigger_release
                        && matches!(act, StateTransitionAction::None)
                        && !guard.card.visible;
                    (act, updated_card, cancel_inflight)
                };
                if cancel_inflight_hover {
                    self.gate.invalidate();
                }
                self.apply_transition(action);
                if let Some(card) = card_opt {
                    self.emit_popup_state(&card);
                }
            }
            PlatformInputEvent::SentenceModifierChanged { active, pointer } => {
                if !settings.enabled {
                    return;
                }
                let action = self
                    .state
                    .lock()
                    .on_sentence_modifier_changed(active, pointer);
                self.apply_transition(action);
            }
            PlatformInputEvent::PointerMoved { pointer } => {
                let action = self.state.lock().on_pointer_moved(
                    pointer,
                    Instant::now(),
                    settings.hover_dwell_ms,
                    settings.linger_dismiss_ms,
                    settings.debounce_radius_pt,
                );
                self.apply_transition(action);
            }
            PlatformInputEvent::GlobalMouseDown { pointer } => {
                let dismiss_ov = {
                    let guard = self.overlay_state.lock();
                    guard.view.visible
                        && !guard.view.pinned
                        && guard.window_rect.is_none_or(|r| !r.contains(pointer))
                };
                if dismiss_ov {
                    self.dismiss_overlay();
                }
                let action = self.state.lock().on_global_mouse_down(pointer);
                self.apply_transition(action);
            }
        }
    }

    pub fn on_timer_tick(self: &Arc<Self>) {
        let settings = self.storage.settings();
        let action = self
            .state
            .lock()
            .on_tick(Instant::now(), settings.linger_dismiss_ms);
        self.apply_transition(action);
    }

    fn apply_transition(self: &Arc<Self>, action: StateTransitionAction) {
        match action {
            StateTransitionAction::None => {}
            StateTransitionAction::StartHoverExtraction {
                pointer,
                sentence_mode,
            } => {
                let ctrl = Arc::clone(self);
                tauri::async_runtime::spawn(async move {
                    ctrl.run_hover_flow(pointer, sentence_mode).await;
                });
            }
            StateTransitionAction::ExpandCurrentCardToSentence => {
                let ctrl = Arc::clone(self);
                tauri::async_runtime::spawn(async move {
                    ctrl.expand_current_card_to_sentence().await;
                });
            }
            StateTransitionAction::SetPopupInteractive(interactive) => {
                if let Some(popup) = self.app.get_webview_window("popup") {
                    self.platform.set_popup_interactive(&popup, interactive);
                }
            }
            StateTransitionAction::DismissPopup => {
                self.gate.invalidate();
                if let Some(popup) = self.app.get_webview_window("popup") {
                    self.platform.hide_popup(&popup);
                }
                self.emit_popup_state(&PopupCardViewState::default());
            }
        }
    }

    pub async fn run_selection_flow(
        self: &Arc<Self>,
        pointer: ScreenPoint,
        explicit_trigger: bool,
    ) {
        let settings = self.storage.settings();
        let focused_app = self.platform.focused_app_id();
        if settings.is_app_excluded(focused_app.as_deref()) {
            return;
        }

        let gen = self.gate.advance();
        let platform = Arc::clone(&self.platform);
        let allow_clip = explicit_trigger && settings.enable_clipboard_fallback;

        let res = tokio::task::spawn_blocking(move || {
            platform.extract_selection(pointer, gen, allow_clip)
        })
        .await
        .ok();

        if !self.gate.is_current(gen) {
            return;
        }

        match res {
            Some(Ok(Some(snapshot))) => {
                self.present_and_translate(snapshot, true).await;
            }
            Some(Err(failure)) if explicit_trigger => {
                self.present_status_card(gen, pointer, failure.user_message());
            }
            _ => {}
        }
    }

    pub async fn run_hover_flow(self: &Arc<Self>, pointer: ScreenPoint, sentence_mode: bool) {
        let settings = self.storage.settings();
        let target_app = self.platform.app_id_at_point(pointer);
        if settings.is_app_excluded(target_app.as_deref()) {
            return;
        }

        let gen = self.gate.advance();
        let platform = Arc::clone(&self.platform);
        let ocr_enabled = settings.enable_ocr_fallback;

        let extraction = tokio::task::spawn_blocking(move || {
            execute_hover_extraction(platform.as_ref(), pointer, sentence_mode, ocr_enabled, gen)
        })
        .await
        .unwrap_or(Err(ExtractionFailure::NoText));

        if !self.gate.is_current(gen) {
            return;
        }

        match extraction {
            Ok(snapshot) => {
                let outcome_str = format!("ok:{:?}:{:?}", snapshot.source, snapshot.scope);
                self.write_diagnostics("hoverExtraction", &outcome_str, target_app.as_deref());
                let force_interactive = !settings.dismiss_on_trigger_release
                    && snapshot.scope == ExtractionScope::Selection;
                self.present_and_translate(snapshot, force_interactive).await;
            }
            Err(ExtractionFailure::PermissionDenied) => {
                self.write_diagnostics("hoverExtraction", "err:PermissionDenied", target_app.as_deref());
                self.present_status_card(gen, pointer, ExtractionFailure::PermissionDenied.user_message());
            }
            Err(other) => {
                let err_str = format!("err:{:?}", other);
                self.write_diagnostics("hoverExtraction", &err_str, target_app.as_deref());
                // Silent miss over blank desktop/margins so normal modifier usage is never noisy
            }
        }
    }

    pub async fn expand_current_card_to_sentence(self: &Arc<Self>) {
        let (maybe_ctx, pointer, anchor_rect, source, trigger_held) = {
            let guard = self.state.lock();
            if !guard.card.visible {
                return;
            }
            (
                guard.card.context_sentence.clone(),
                guard.pointer,
                guard.active_anchor_rect,
                guard.card.source,
                guard.trigger_held,
            )
        };

        if let Some(sentence) = maybe_ctx.filter(|s| !s.trim().is_empty()) {
            let gen = self.gate.advance();
            let snapshot = ExtractionSnapshot {
                generation: gen,
                text: sentence,
                context: None,
                sentence_completeness: Some(ExtractionCompleteness::Complete),
                scope: ExtractionScope::Sentence,
                completeness: ExtractionCompleteness::Complete,
                source,
                anchor_rect,
                pointer,
                app_id: None,
            };
            let force_interactive =
                !trigger_held || !self.storage.settings().dismiss_on_trigger_release;
            self.present_and_translate(snapshot, force_interactive).await;
        } else {
            // Re-extract in sentence mode if no cached sentence context was attached
            self.run_hover_flow(pointer, true).await;
        }
    }

    pub async fn explain_current_card(self: &Arc<Self>) -> Result<(), String> {
        let (gen, text, ctx) = {
            let mut guard = self.state.lock();
            if !guard.card.visible || guard.card.original_text.trim().is_empty() {
                return Err("当前没有可解释的卡片内容".into());
            }
            guard.card.is_explaining = true;
            guard.card.explanation = Some(String::new());
            guard.card.explanation_error = None;
            let snapshot_card = guard.card.clone();
            self.emit_popup_state(&snapshot_card);
            (
                guard.card.generation,
                guard.card.original_text.clone(),
                guard.card.context_sentence.clone(),
            )
        };

        let settings = self.storage.settings();
        let api_key = self
            .secrets
            .get_secret(settings.active_provider.secret_account())
            .unwrap_or_default();
        let llm_cfg = settings.build_llm_config(api_key);

        let cache_key = CacheKey::new(
            "explain",
            &llm_cfg.model,
            QueryMode::Explanation,
            &llm_cfg.target_language,
            &text,
            ctx.as_deref(),
        );
        if let Some(cached) = self.cache.get(&cache_key) {
            let mut guard = self.state.lock();
            if guard.card.generation == gen {
                guard.card.explanation = Some(cached);
                guard.card.is_explaining = false;
                let updated = guard.card.clone();
                drop(guard);
                self.emit_popup_state(&updated);
            }
            return Ok(());
        }

        let mut acc = String::new();
        let mut last_emit = Instant::now();
        let res = stream_llm_completion(
            &llm_cfg,
            &text,
            ctx.as_deref(),
            QueryMode::Explanation,
            |delta| {
                if !self.gate.is_current(gen) {
                    return;
                }
                acc.push_str(delta);
                if last_emit.elapsed() >= Duration::from_millis(32) {
                    last_emit = Instant::now();
                    let mut guard = self.state.lock();
                    if guard.card.generation == gen {
                        guard.card.explanation = Some(acc.clone());
                        let updated = guard.card.clone();
                        drop(guard);
                        self.emit_popup_state(&updated);
                    }
                }
            },
        )
        .await;

        if !self.gate.is_current(gen) {
            return Ok(());
        }

        let mut guard = self.state.lock();
        if guard.card.generation == gen {
            guard.card.is_explaining = false;
            match res {
                Ok((final_text, _)) => {
                    self.cache.insert(cache_key, final_text.clone());
                    guard.card.explanation = Some(final_text);
                }
                Err(err) => {
                    guard.card.explanation_error = Some(err.user_message());
                }
            }
            let updated = guard.card.clone();
            drop(guard);
            self.emit_popup_state(&updated);
        }

        Ok(())
    }

    pub async fn present_and_translate(
        self: &Arc<Self>,
        snapshot: ExtractionSnapshot,
        force_interactive: bool,
    ) {
        let settings = self.storage.settings();
        let gen = snapshot.generation;

        // Tier 1: < 5ms Offline WordBook lookup before first frame render
        let dict_entry = if matches!(snapshot.scope, ExtractionScope::Word | ExtractionScope::Label)
            || snapshot.text.split_whitespace().count() == 1
        {
            self.wordbook.lookup(&snapshot.text)
        } else {
            None
        };

        let run_llm = settings.enable_llm_translation
            && !(snapshot.scope == ExtractionScope::Word && settings.word_fast_only);

        let initial_card = PopupCardViewState {
            visible: true,
            generation: gen,
            original_text: snapshot.text.clone(),
            context_sentence: snapshot.context.clone(),
            sentence_completeness: snapshot.sentence_completeness,
            scope: snapshot.scope,
            completeness: snapshot.completeness,
            source: snapshot.source,
            dictionary: dict_entry,
            fast_translation: None,
            llm_translation: String::new(),
            is_streaming: run_llm,
            translation_error: None,
            explanation: None,
            is_explaining: false,
            explanation_error: None,
            status_banner: None,
            interactive: force_interactive,
            pinned: false,
            saved: false,
            ttft_ms: None,
        };

        let monitor = self.platform.monitor_bounds_at(snapshot.pointer);
        let initial_h = estimate_initial_popup_height(&initial_card);
        let popup_rect = compute_popup_rect(
            snapshot.anchor_rect,
            snapshot.pointer,
            monitor,
            DEFAULT_POPUP_WIDTH,
            initial_h,
        );

        let interactive = {
            let mut guard = self.state.lock();
            if !force_interactive && settings.dismiss_on_trigger_release && !guard.trigger_held {
                return;
            }
            guard.present_card(
                initial_card,
                snapshot.anchor_rect,
                popup_rect,
                force_interactive,
                Instant::now(),
                settings.linger_dismiss_ms,
            );
            let card_now = guard.card.clone();
            self.emit_popup_state(&card_now);
            card_now.interactive
        };

        if let Some(popup_win) = self.app.get_webview_window("popup") {
            self.platform
                .present_popup(&popup_win, popup_rect, interactive);
        }

        // Record daily translation word/character volume for the Home Heatmap
        self.record_usage(&snapshot.text);

        // Tier 2: Concurrent Fast Machine Translation (80 ~ 250ms)
        if settings.enable_fast_mt {
            let ctrl_fast = Arc::clone(self);
            let text_fast = snapshot.text.clone();
            let target_lang = settings.target_language.clone();
            let fast_engine = settings.fast_mt_engine;

            tauri::async_runtime::spawn(async move {
                let fast_key = CacheKey::new(
                    "fast_mt",
                    "gtx",
                    QueryMode::Sentence,
                    &target_lang,
                    &text_fast,
                    None,
                );
                let result = if let Some(cached) = ctrl_fast.cache.get(&fast_key) {
                    Ok(cached)
                } else {
                    let r = translate_fast_machine(&text_fast, &target_lang, fast_engine).await;
                    if let Ok(val) = &r {
                        ctrl_fast.cache.insert(fast_key, val.clone());
                    }
                    r
                };

                if !ctrl_fast.gate.is_current(gen) {
                    return;
                }
                if let Ok(fast_text) = result {
                    let mut guard = ctrl_fast.state.lock();
                    if guard.card.generation == gen {
                        guard.card.fast_translation = Some(fast_text);
                        let updated = guard.card.clone();
                        drop(guard);
                        ctrl_fast.emit_popup_state(&updated);
                    }
                }
            });
        }

        // Tier 3: Concurrent SSE Streaming LLM Translation (200 ~ 400ms TTFT)
        if run_llm {
            let ctrl_llm = Arc::clone(self);
            let query_mode = match snapshot.scope {
                ExtractionScope::Word => QueryMode::Word,
                ExtractionScope::Sentence | ExtractionScope::Label => QueryMode::Sentence,
                ExtractionScope::Selection | ExtractionScope::RegionOcr => QueryMode::Selection,
            };
            let api_key = self
                .secrets
                .get_secret(settings.active_provider.secret_account())
                .unwrap_or_default();
            let llm_cfg = settings.build_llm_config(api_key);
            let src_text = snapshot.text.clone();
            let src_ctx = snapshot.context.clone();

            tauri::async_runtime::spawn(async move {
                let cache_key = CacheKey::new(
                    format!("llm:{:?}", llm_cfg.provider),
                    &llm_cfg.model,
                    query_mode,
                    &llm_cfg.target_language,
                    &src_text,
                    src_ctx.as_deref(),
                );

                if let Some(cached) = ctrl_llm.cache.get(&cache_key) {
                    let mut guard = ctrl_llm.state.lock();
                    if guard.card.generation == gen {
                        guard.card.llm_translation = cached;
                        guard.card.is_streaming = false;
                        guard.card.ttft_ms = Some(0);
                        let updated = guard.card.clone();
                        drop(guard);
                        ctrl_llm.emit_popup_state(&updated);
                    }
                    return;
                }

                let mut stream_acc = String::new();
                let mut first_emitted = false;
                let mut last_emit = Instant::now();

                let outcome = stream_llm_completion(
                    &llm_cfg,
                    &src_text,
                    src_ctx.as_deref(),
                    query_mode,
                    |delta| {
                        if !ctrl_llm.gate.is_current(gen) {
                            return;
                        }
                        stream_acc.push_str(delta);
                        if !first_emitted || last_emit.elapsed() >= Duration::from_millis(28) {
                            first_emitted = true;
                            last_emit = Instant::now();
                            let mut guard = ctrl_llm.state.lock();
                            if guard.card.generation == gen {
                                guard.card.llm_translation = stream_acc.clone();
                                let updated = guard.card.clone();
                                drop(guard);
                                ctrl_llm.emit_popup_state(&updated);
                            }
                        }
                    },
                )
                .await;

                if !ctrl_llm.gate.is_current(gen) {
                    return;
                }

                let mut guard = ctrl_llm.state.lock();
                if guard.card.generation == gen {
                    guard.card.is_streaming = false;
                    match outcome {
                        Ok((final_text, ttft_ms)) => {
                            ctrl_llm.cache.insert(cache_key, final_text.clone());
                            guard.card.llm_translation = final_text;
                            guard.card.ttft_ms = Some(ttft_ms);
                        }
                        Err(err) => {
                            // Only surface Unconfigured error if Fast MT & Dictionary are also empty
                            if guard.card.fast_translation.is_none()
                                && guard.card.dictionary.is_none()
                                || !matches!(err, translation::TranslationError::Unconfigured)
                            {
                                guard.card.translation_error = Some(err.user_message());
                            }
                        }
                    }
                    let updated = guard.card.clone();
                    drop(guard);
                    ctrl_llm.emit_popup_state(&updated);
                }
            });
        }
    }

    pub fn present_status_card(&self, gen: u64, pointer: ScreenPoint, message: &str) {
        let settings = self.storage.settings();
        let monitor = self.platform.monitor_bounds_at(pointer);
        let popup_rect = compute_popup_rect(None, pointer, monitor, 320.0, 52.0);
        let card = PopupCardViewState {
            visible: true,
            generation: gen,
            status_banner: Some(message.to_string()),
            interactive: true,
            ..Default::default()
        };
        {
            let mut guard = self.state.lock();
            if matches!(guard.phase, coordinator::CardPhase::Pinned { .. }) {
                return;
            }
            guard.present_card(
                card.clone(),
                None,
                popup_rect,
                true,
                Instant::now(),
                settings.linger_dismiss_ms,
            );
        }
        self.emit_popup_state(&card);
        if let Some(popup_win) = self.app.get_webview_window("popup") {
            self.platform.present_popup(&popup_win, popup_rect, true);
        }
    }

    pub async fn open_capture_overlay(self: &Arc<Self>, hide_main_first: bool) {
        if cfg!(target_os = "macos") && !self.platform.permissions_status(false).screen_recording {
            let _ = self.platform.request_screen_recording();
            let pt = self.platform.current_pointer();
            let gen = self.gate.advance();
            self.present_status_card(
                gen,
                pt,
                "框选截屏翻译需要「屏幕录制」权限，请在系统设置开启后重试",
            );
            return;
        }

        // Dismiss any active hover popup or previous screenshot overlay cleanly
        let pop_action = self.state.lock().dismiss();
        self.apply_transition(pop_action);
        self.dismiss_overlay();

        if hide_main_first {
            if let Some(main_win) = self.app.get_webview_window("main") {
                if main_win.is_visible().unwrap_or(false) {
                    let _ = main_win.hide();
                    tokio::time::sleep(Duration::from_millis(35)).await;
                }
            }
        }

        let pointer = self.platform.current_pointer();
        let monitor = self.platform.monitor_bounds_at(pointer);
        let platform = Arc::clone(&self.platform);

        // Pre-capture crisp freeze-frame of the active monitor so transient menus / video subtitles never vanish
        let frozen = tokio::task::spawn_blocking(move || {
            platform.capture_region_image(monitor, true)
        })
        .await
        .ok()
        .flatten();

        let has_backdrop = frozen.is_some();
        self.overlay_state.lock().frozen_monitor = frozen;

        if let Some(cap) = self.app.get_webview_window("capture") {
            let _ = cap.set_position(tauri::Position::Logical(tauri::LogicalPosition::new(
                monitor.x, monitor.y,
            )));
            let _ = cap.set_size(tauri::Size::Logical(tauri::LogicalSize::new(
                monitor.width,
                monitor.height,
            )));
            let _ = cap.show();
            let _ = cap.set_focus();
            let _ = self.app.emit_to(
                "capture",
                "capture://activate",
                CaptureActivatePayload {
                    monitor,
                    has_backdrop,
                },
            );
        }
    }

    pub fn open_main_window(&self, tab: &str) {
        if let Some(win) = self.app.get_webview_window("main") {
            let _ = win.show();
            let _ = win.unminimize();
            let _ = win.set_focus();
            let _ = self.app.emit_to("main", "main://navigate", tab);
        }
    }
}

// ============================================================================
// Tauri IPC Commands
// ============================================================================

#[tauri::command]
fn get_popup_state(ctrl: State<'_, Arc<AppController>>) -> PopupCardViewState {
    ctrl.state.lock().card.clone()
}

#[tauri::command]
async fn popup_expand_sentence(ctrl: State<'_, Arc<AppController>>) -> Result<(), String> {
    ctrl.expand_current_card_to_sentence().await;
    Ok(())
}

#[tauri::command]
async fn popup_explain(ctrl: State<'_, Arc<AppController>>) -> Result<(), String> {
    ctrl.explain_current_card().await
}

#[tauri::command]
fn popup_toggle_pin(ctrl: State<'_, Arc<AppController>>) -> bool {
    let linger = ctrl.storage.settings().linger_dismiss_ms;
    let (pinned, card) = {
        let mut guard = ctrl.state.lock();
        let p = guard.toggle_pin(Instant::now(), linger);
        (p, guard.card.clone())
    };
    if let Some(popup) = ctrl.app.get_webview_window("popup") {
        ctrl.platform.set_popup_interactive(&popup, true);
    }
    ctrl.emit_popup_state(&card);
    pinned
}

#[tauri::command]
fn popup_copy_text(
    text: Option<String>,
    ctrl: State<'_, Arc<AppController>>,
) -> Result<String, String> {
    let to_copy = text.unwrap_or_else(|| {
        let guard = ctrl.state.lock();
        if !guard.card.llm_translation.trim().is_empty() {
            guard.card.llm_translation.clone()
        } else if let Some(fast) = &guard.card.fast_translation {
            fast.clone()
        } else {
            guard.card.original_text.clone()
        }
    });
    if to_copy.trim().is_empty() {
        return Err("没有可复制的内容".into());
    }
    ctrl.platform.write_clipboard(&to_copy)?;
    Ok(to_copy)
}

#[tauri::command]
fn popup_save_entry(ctrl: State<'_, Arc<AppController>>) -> Result<String, String> {
    let (source, translation, context) = {
        let guard = ctrl.state.lock();
        let src = guard.card.original_text.trim().to_string();
        let trans = if !guard.card.llm_translation.trim().is_empty() {
            guard.card.llm_translation.trim().to_string()
        } else if let Some(fast) = &guard.card.fast_translation {
            fast.trim().to_string()
        } else if let Some(dict) = &guard.card.dictionary {
            dict.senses
                .iter()
                .map(|s| format!("{} {}", s.part_of_speech, s.glosses.join("；")))
                .collect::<Vec<_>>()
                .join("  ")
        } else {
            String::new()
        };
        let ctx = guard.card.context_sentence.clone().unwrap_or_default();
        (src, trans, ctx)
    };

    if source.is_empty() || translation.is_empty() {
        return Err("译文尚未生成完毕，暂无法收藏".into());
    }

    let (outcome, _) = ctrl
        .storage
        .save_entry(SavedEntry::new(source, translation, context))?;

    {
        let mut guard = ctrl.state.lock();
        guard.card.saved = true;
        let updated = guard.card.clone();
        drop(guard);
        ctrl.emit_popup_state(&updated);
    }
    let _ = ctrl.app.emit_to("main", "wordbook://changed", ());

    Ok(match outcome {
        SaveOutcome::Inserted => "已加入生词收藏本".into(),
        SaveOutcome::AlreadySaved => "该词条已在收藏本中".into(),
    })
}

#[tauri::command]
fn popup_resize(height: f64, ctrl: State<'_, Arc<AppController>>) {
    let clamped_h = height.clamp(44.0, 520.0).round();
    let (maybe_new_rect, interactive) = {
        let mut guard = ctrl.state.lock();
        if !guard.card.visible {
            return;
        }
        if let Some(prev) = guard.popup_rect {
            if (prev.height - clamped_h).abs() < 0.5 {
                return;
            }
        }
        let width = guard
            .popup_rect
            .map(|r| r.width)
            .unwrap_or(DEFAULT_POPUP_WIDTH);
        let new_rect = match (guard.active_anchor_rect, guard.popup_rect, guard.card.pinned) {
            (Some(anchor), _, false) if anchor.is_valid() => {
                let anchor_center = ScreenPoint::new(anchor.mid_x(), anchor.mid_y());
                let monitor = ctrl.platform.monitor_bounds_at(anchor_center);
                compute_popup_rect(Some(anchor), anchor_center, monitor, width, clamped_h)
            }
            (_, Some(prev), _) => {
                let origin_pt = ScreenPoint::new(prev.mid_x(), prev.min_y());
                let monitor = ctrl.platform.monitor_bounds_at(origin_pt);
                let margin = 12.0;
                let max_y = (monitor.max_y() - margin - clamped_h).max(monitor.min_y() + margin);
                let y = prev.y.min(max_y).max(monitor.min_y() + margin);
                ScreenRect::new(prev.x, y.round(), width, clamped_h)
            }
            _ => {
                let pointer = guard.pointer;
                let monitor = ctrl.platform.monitor_bounds_at(pointer);
                compute_popup_rect(None, pointer, monitor, width, clamped_h)
            }
        };
        guard.popup_rect = Some(new_rect);
        (Some(new_rect), guard.card.interactive)
    };

    if let (Some(rect), Some(popup)) = (maybe_new_rect, ctrl.app.get_webview_window("popup")) {
        ctrl.platform.present_popup(&popup, rect, interactive);
    }
}

#[tauri::command]
fn popup_dismiss(ctrl: State<'_, Arc<AppController>>) {
    let action = ctrl.state.lock().dismiss();
    ctrl.apply_transition(action);
}

#[tauri::command]
async fn start_region_capture(ctrl: State<'_, Arc<AppController>>) -> Result<(), String> {
    ctrl.open_capture_overlay(true).await;
    Ok(())
}

#[tauri::command]
fn cancel_region_capture(ctrl: State<'_, Arc<AppController>>) {
    ctrl.overlay_state.lock().frozen_monitor = None;
    if let Some(cap) = ctrl.app.get_webview_window("capture") {
        let _ = cap.hide();
    }
}

#[tauri::command]
fn get_capture_backdrop(ctrl: State<'_, Arc<AppController>>) -> tauri::ipc::Response {
    let bytes = {
        let guard = ctrl.overlay_state.lock();
        match &guard.frozen_monitor {
            Some(img) => {
                if let Some(jpeg) = &img.preview_jpeg {
                    jpeg.clone()
                } else {
                    encode_rgba_to_png(img).unwrap_or_default()
                }
            }
            None => Vec::new(),
        }
    };
    tauri::ipc::Response::new(bytes)
}

#[tauri::command]
fn get_overlay_state(ctrl: State<'_, Arc<AppController>>) -> CaptureOverlayViewState {
    ctrl.overlay_state.lock().view.clone()
}

#[tauri::command]
fn get_overlay_crop_image(ctrl: State<'_, Arc<AppController>>) -> tauri::ipc::Response {
    let bytes = ctrl
        .overlay_state
        .lock()
        .cropped_png
        .clone()
        .unwrap_or_default();
    tauri::ipc::Response::new(bytes)
}

#[tauri::command]
fn overlay_toggle_pin(
    force_pinned: Option<bool>,
    ctrl: State<'_, Arc<AppController>>,
) -> bool {
    let (pinned, view) = {
        let mut guard = ctrl.overlay_state.lock();
        if !guard.view.visible {
            return false;
        }
        guard.view.pinned = force_pinned.unwrap_or(!guard.view.pinned);
        (guard.view.pinned, guard.view.clone())
    };
    ctrl.emit_overlay_state(&view);
    pinned
}

#[tauri::command]
fn overlay_move_by(dx: f64, dy: f64, ctrl: State<'_, Arc<AppController>>) {
    if dx.abs() < 0.01 && dy.abs() < 0.01 {
        return;
    }
    let (maybe_rect, maybe_view_pin_update) = {
        let mut guard = ctrl.overlay_state.lock();
        if !guard.view.visible {
            return;
        }
        let Some(prev) = guard.window_rect else {
            return;
        };
        let next = ScreenRect::new(
            (prev.x + dx).round(),
            (prev.y + dy).round(),
            prev.width,
            prev.height,
        );
        guard.window_rect = Some(next);
        // Dragging the overlay automatically pins it so releasing & clicking elsewhere won't accidentally close it
        let pin_changed = if !guard.view.pinned {
            guard.view.pinned = true;
            Some(guard.view.clone())
        } else {
            None
        };
        (Some(next), pin_changed)
    };

    if let Some(view) = maybe_view_pin_update {
        ctrl.emit_overlay_state(&view);
    }
    if let (Some(rect), Some(win)) = (maybe_rect, ctrl.app.get_webview_window("overlay")) {
        ctrl.platform.present_overlay(&win, rect);
    }
}

#[tauri::command]
fn overlay_dismiss(ctrl: State<'_, Arc<AppController>>) {
    ctrl.dismiss_overlay();
}

#[tauri::command]
fn get_usage_stats(ctrl: State<'_, Arc<AppController>>) -> UsageStatsSummary {
    ctrl.storage.usage_summary()
}

#[tauri::command]
async fn submit_region_capture(
    rect: ScreenRect,
    ctrl: State<'_, Arc<AppController>>,
) -> Result<(), String> {
    if let Some(cap) = ctrl.app.get_webview_window("capture") {
        let _ = cap.hide();
    }

    let gen = ctrl.overlay_gate.advance();
    let frozen_opt = ctrl.overlay_state.lock().frozen_monitor.take();
    if frozen_opt.is_none() {
        // Fallback when OS did not provide a pre-frozen backdrop: wait 1 frame for capture overlay to clear
        tokio::time::sleep(Duration::from_millis(35)).await;
    }

    let platform = Arc::clone(&ctrl.platform);
    let prep_result = tokio::task::spawn_blocking(move || {
        let cropped = match frozen_opt {
            Some(ref full) => crop_captured_image(full, rect),
            None => platform.capture_region_image(rect, false),
        };

        let Some(crop_img) = cropped else {
            return Err(ExtractionFailure::NoText);
        };

        let lines = platform.ocr_captured_image(&crop_img)?;
        let actual_rect = crop_img.rect;
        let seeds = OcrClusterer::cluster_region_blocks(&lines, actual_rect);
        if seeds.is_empty() {
            return Err(ExtractionFailure::NoText);
        }

        let blocks = build_initial_overlay_blocks(seeds, Some(&crop_img));
        let png_bytes = encode_rgba_to_png(&crop_img).ok();
        Ok((actual_rect, blocks, png_bytes))
    })
    .await
    .map_err(|e| e.to_string())?;

    if !ctrl.overlay_gate.is_current(gen) {
        return Ok(());
    }

    let (actual_rect, initial_blocks, png_bytes) = match prep_result {
        Ok(val) => val,
        Err(err) => {
            let status_gen = ctrl.gate.advance();
            ctrl.present_status_card(
                status_gen,
                ScreenPoint::new(rect.mid_x(), rect.max_y()),
                err.user_message(),
            );
            return Ok(());
        }
    };

    // Count total recognized words/characters across all blocks toward the daily Heatmap!
    let combined_source = initial_blocks
        .iter()
        .map(|b| b.source_text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    ctrl.record_usage(&combined_source);

    let settings = ctrl.storage.settings();
    let api_key = ctrl
        .secrets
        .get_secret(settings.active_provider.secret_account())
        .unwrap_or_default();
    let has_llm_key = !api_key.trim().is_empty();
    let run_llm = settings.enable_llm_translation && (has_llm_key || !settings.enable_fast_mt);

    let center_pt = ScreenPoint::new(actual_rect.mid_x(), actual_rect.mid_y());
    let monitor = ctrl.platform.monitor_bounds_at(center_pt);
    let layout = compute_overlay_window_layout(actual_rect, monitor);

    let initial_view = CaptureOverlayViewState {
        visible: true,
        generation: gen,
        pinned: false,
        is_refining: run_llm,
        error: None,
        image_offset_x: layout.image_offset_x,
        image_offset_y: layout.image_offset_y,
        image_width: layout.image_width,
        image_height: layout.image_height,
        toolbar_placement: layout.toolbar_placement,
        blocks: initial_blocks.clone(),
    };

    {
        let mut guard = ctrl.overlay_state.lock();
        guard.view = initial_view.clone();
        guard.window_rect = Some(layout.window_rect);
        guard.cropped_png = png_bytes;
    }

    if let Some(ov_win) = ctrl.app.get_webview_window("overlay") {
        ctrl.platform.present_overlay(&ov_win, layout.window_rect);
    }
    let _ = ctrl.app.emit_to("overlay", "overlay://new-image", gen);
    ctrl.emit_overlay_state(&initial_view);

    // Tier 1: Concurrent Fast Machine Translation (~120-250ms) so every English block flips to Chinese rapidly
    if settings.enable_fast_mt {
        let ctrl_fast = Arc::clone(&*ctrl);
        let target_lang = settings.target_language.clone();
        let fast_engine = settings.fast_mt_engine;
        let tasks: Vec<(usize, String)> = initial_blocks
            .iter()
            .map(|b| (b.id, b.source_text.clone()))
            .collect();

        tauri::async_runtime::spawn(async move {
            let mut stream = futures_util::stream::iter(tasks.into_iter().map(|(id, src)| {
                let ctrl_item = Arc::clone(&ctrl_fast);
                let tl = target_lang.clone();
                async move {
                    let fast_key = CacheKey::new(
                        "fast_mt",
                        "gtx",
                        QueryMode::Sentence,
                        &tl,
                        &src,
                        None,
                    );
                    let res = if let Some(cached) = ctrl_item.cache.get(&fast_key) {
                        Ok(cached)
                    } else {
                        let r = translate_fast_machine(&src, &tl, fast_engine).await;
                        if let Ok(val) = &r {
                            ctrl_item.cache.insert(fast_key, val.clone());
                        }
                        r
                    };
                    (id, res)
                }
            }))
            .buffer_unordered(6);

            while let Some((id, res)) = stream.next().await {
                if !ctrl_fast.overlay_gate.is_current(gen) {
                    return;
                }
                let mut guard = ctrl_fast.overlay_state.lock();
                if guard.view.generation != gen || !guard.view.visible {
                    return;
                }
                if let Some(block) = guard.view.blocks.iter_mut().find(|b| b.id == id) {
                    match res {
                        Ok(fast_txt) => {
                            // Only write Fast MT if LLM hasn't already populated a refined translation
                            if block.translation.is_empty() {
                                block.translation = fast_txt;
                            }
                            block.is_loading = false;
                        }
                        Err(err) => {
                            if !run_llm {
                                block.is_loading = false;
                                guard.view.error = Some(err.user_message());
                            }
                        }
                    }
                    let updated = guard.view.clone();
                    drop(guard);
                    ctrl_fast.emit_overlay_state(&updated);
                }
            }
        });
    }

    // Tier 2: Contextual LLM Refinement (streaming for single-block, 1-shot structured JSON batch for multi-block)
    if run_llm {
        let ctrl_llm = Arc::clone(&*ctrl);
        let llm_cfg = settings.build_llm_config(api_key);
        let tasks: Vec<(usize, String)> = initial_blocks
            .iter()
            .map(|b| (b.id, b.source_text.clone()))
            .collect();

        tauri::async_runtime::spawn(async move {
            if tasks.len() == 1 {
                let (only_id, only_src) = &tasks[0];
                let mut acc = String::new();
                let mut last_emit = Instant::now();
                let outcome = stream_llm_completion(
                    &llm_cfg,
                    only_src,
                    None,
                    QueryMode::Selection,
                    |delta| {
                        if !ctrl_llm.overlay_gate.is_current(gen) {
                            return;
                        }
                        acc.push_str(delta);
                        if last_emit.elapsed() >= Duration::from_millis(30) {
                            last_emit = Instant::now();
                            let mut guard = ctrl_llm.overlay_state.lock();
                            if guard.view.generation == gen && guard.view.visible {
                                if let Some(b) =
                                    guard.view.blocks.iter_mut().find(|b| b.id == *only_id)
                                {
                                    b.translation = acc.clone();
                                    b.is_loading = false;
                                }
                                let updated = guard.view.clone();
                                drop(guard);
                                ctrl_llm.emit_overlay_state(&updated);
                            }
                        }
                    },
                )
                .await;

                if !ctrl_llm.overlay_gate.is_current(gen) {
                    return;
                }
                let mut guard = ctrl_llm.overlay_state.lock();
                if guard.view.generation == gen && guard.view.visible {
                    guard.view.is_refining = false;
                    match outcome {
                        Ok((final_txt, _)) => {
                            if let Some(b) =
                                guard.view.blocks.iter_mut().find(|b| b.id == *only_id)
                            {
                                b.translation = final_txt;
                                b.is_loading = false;
                            }
                        }
                        Err(err) => {
                            let any_filled = guard
                                .view
                                .blocks
                                .iter()
                                .any(|b| !b.translation.trim().is_empty());
                            for b in &mut guard.view.blocks {
                                b.is_loading = false;
                            }
                            if !any_filled {
                                guard.view.error = Some(err.user_message());
                            }
                        }
                    }
                    let updated = guard.view.clone();
                    drop(guard);
                    ctrl_llm.emit_overlay_state(&updated);
                }
            } else {
                let outcome = translate_blocks_llm_batch(&llm_cfg, &tasks).await;
                if !ctrl_llm.overlay_gate.is_current(gen) {
                    return;
                }
                let mut guard = ctrl_llm.overlay_state.lock();
                if guard.view.generation == gen && guard.view.visible {
                    guard.view.is_refining = false;
                    match outcome {
                        Ok(map) => {
                            for b in &mut guard.view.blocks {
                                if let Some(refined) = map.get(&b.id) {
                                    b.translation = refined.clone();
                                }
                                b.is_loading = false;
                            }
                        }
                        Err(err) => {
                            let any_filled = guard
                                .view
                                .blocks
                                .iter()
                                .any(|b| !b.translation.trim().is_empty());
                            for b in &mut guard.view.blocks {
                                b.is_loading = false;
                            }
                            if !any_filled {
                                guard.view.error = Some(err.user_message());
                            }
                        }
                    }
                    let updated = guard.view.clone();
                    drop(guard);
                    ctrl_llm.emit_overlay_state(&updated);
                }
            }
        });
    }

    Ok(())
}

#[tauri::command]
fn get_settings(ctrl: State<'_, Arc<AppController>>) -> AppSettings {
    ctrl.storage.settings()
}

#[tauri::command]
fn save_settings(
    settings: AppSettings,
    ctrl: State<'_, Arc<AppController>>,
) -> Result<AppSettings, String> {
    let saved = ctrl.storage.update_settings(settings)?;
    ctrl.platform.update_trigger_config(
        saved.trigger_modifier,
        saved.double_tap_selection,
        &saved.selection_shortcut,
        &saved.capture_shortcut,
    );
    ctrl.sync_tray_items(&saved);
    let _ = ctrl.app.emit("settings://changed", &saved);
    Ok(saved)
}

#[tauri::command]
fn set_shortcut_recording(active: bool, ctrl: State<'_, Arc<AppController>>) {
    ctrl.platform.set_shortcut_recording(active);
}

#[tauri::command]
fn get_api_key_status(ctrl: State<'_, Arc<AppController>>) -> HashMap<String, bool> {
    let mut map = HashMap::new();
    for kind in [
        LlmProviderKind::Gemini,
        LlmProviderKind::Deepseek,
        LlmProviderKind::OpenaiCompatible,
    ] {
        let present = ctrl
            .secrets
            .get_secret(kind.secret_account())
            .is_some_and(|k| !k.trim().is_empty());
        let key_name = match kind {
            LlmProviderKind::Gemini => "gemini",
            LlmProviderKind::Deepseek => "deepseek",
            LlmProviderKind::OpenaiCompatible => "openaiCompatible",
        };
        map.insert(key_name.to_string(), present);
    }
    map
}

#[tauri::command]
fn set_api_key(
    provider: LlmProviderKind,
    api_key: String,
    ctrl: State<'_, Arc<AppController>>,
) -> Result<(), String> {
    ctrl.secrets.set_secret(provider.secret_account(), &api_key)?;
    ctrl.cache.clear();
    Ok(())
}

#[tauri::command]
async fn test_provider_connection(
    provider: LlmProviderKind,
    base_url: String,
    model: String,
    api_key_override: Option<String>,
    ctrl: State<'_, Arc<AppController>>,
) -> Result<ConnectionTestResult, String> {
    let api_key = api_key_override
        .filter(|s| !s.trim().is_empty())
        .or_else(|| ctrl.secrets.get_secret(provider.secret_account()))
        .unwrap_or_default();

    let cfg = translation::LlmRequestConfig {
        provider,
        base_url: if base_url.trim().is_empty() {
            provider.default_base_url().into()
        } else {
            base_url
        },
        model: if model.trim().is_empty() {
            provider.default_model().into()
        } else {
            model
        },
        api_key,
        target_language: ctrl.storage.settings().target_language,
    };

    Ok(test_llm_connection(&cfg).await)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualStreamEvent {
    pub request_id: String,
    pub dictionary: Option<dictionary::WordEntry>,
    pub fast_translation: Option<String>,
    pub llm_translation: String,
    pub done: bool,
    pub ttft_ms: Option<u64>,
    pub error: Option<String>,
}

#[tauri::command]
async fn manual_translate_stream(
    request_id: String,
    text: String,
    mode: QueryMode,
    ctrl: State<'_, Arc<AppController>>,
) -> Result<(), String> {
    let trimmed = text.trim().to_string();
    if !is_allowed_input_text(&trimmed) {
        return Err("请输入 1 ~ 5,000 字符以内的待译文本".into());
    }

    ctrl.record_usage(&trimmed);

    let settings = ctrl.storage.settings();
    let dict = if trimmed.split_whitespace().count() == 1 {
        ctrl.wordbook.lookup(&trimmed)
    } else {
        None
    };

    let _ = ctrl.app.emit_to(
        "main",
        "manual://progress",
        ManualStreamEvent {
            request_id: request_id.clone(),
            dictionary: dict.clone(),
            fast_translation: None,
            llm_translation: String::new(),
            done: false,
            ttft_ms: None,
            error: None,
        },
    );

    let mut fast_res = None;
    if settings.enable_fast_mt && mode != QueryMode::Explanation {
        if let Ok(ft) =
            translate_fast_machine(&trimmed, &settings.target_language, settings.fast_mt_engine)
                .await
        {
            fast_res = Some(ft.clone());
            let _ = ctrl.app.emit_to(
                "main",
                "manual://progress",
                ManualStreamEvent {
                    request_id: request_id.clone(),
                    dictionary: dict.clone(),
                    fast_translation: fast_res.clone(),
                    llm_translation: String::new(),
                    done: !settings.enable_llm_translation,
                    ttft_ms: None,
                    error: None,
                },
            );
        }
    }

    if !settings.enable_llm_translation {
        return Ok(());
    }

    let api_key = ctrl
        .secrets
        .get_secret(settings.active_provider.secret_account())
        .unwrap_or_default();
    let llm_cfg = settings.build_llm_config(api_key);
    let mut acc = String::new();
    let mut last_emit = Instant::now();

    let res = stream_llm_completion(&llm_cfg, &trimmed, None, mode, |delta| {
        acc.push_str(delta);
        if last_emit.elapsed() >= Duration::from_millis(28) {
            last_emit = Instant::now();
            let _ = ctrl.app.emit_to(
                "main",
                "manual://progress",
                ManualStreamEvent {
                    request_id: request_id.clone(),
                    dictionary: dict.clone(),
                    fast_translation: fast_res.clone(),
                    llm_translation: acc.clone(),
                    done: false,
                    ttft_ms: None,
                    error: None,
                },
            );
        }
    })
    .await;

    match res {
        Ok((final_text, ttft_ms)) => {
            let _ = ctrl.app.emit_to(
                "main",
                "manual://progress",
                ManualStreamEvent {
                    request_id,
                    dictionary: dict,
                    fast_translation: fast_res,
                    llm_translation: final_text,
                    done: true,
                    ttft_ms: Some(ttft_ms),
                    error: None,
                },
            );
        }
        Err(err) => {
            let _ = ctrl.app.emit_to(
                "main",
                "manual://progress",
                ManualStreamEvent {
                    request_id,
                    dictionary: dict,
                    fast_translation: fast_res,
                    llm_translation: acc,
                    done: true,
                    ttft_ms: None,
                    error: Some(err.user_message()),
                },
            );
        }
    }

    Ok(())
}

#[tauri::command]
fn list_saved_entries(ctrl: State<'_, Arc<AppController>>) -> Vec<SavedEntry> {
    ctrl.storage.list_entries()
}

#[tauri::command]
fn save_manual_entry(
    source: String,
    translation: String,
    context: String,
    ctrl: State<'_, Arc<AppController>>,
) -> Result<SavedEntry, String> {
    let (_, entry) = ctrl
        .storage
        .save_entry(SavedEntry::new(source, translation, context))?;
    Ok(entry)
}

#[tauri::command]
fn delete_saved_entry(id: String, ctrl: State<'_, Arc<AppController>>) -> Result<(), String> {
    let uuid = Uuid::parse_str(&id).map_err(|e| e.to_string())?;
    ctrl.storage.remove_entry(uuid)
}

#[tauri::command]
fn import_saved_entries(
    json_content: String,
    ctrl: State<'_, Arc<AppController>>,
) -> Result<usize, String> {
    ctrl.storage.import_entries_json(&json_content)
}

#[tauri::command]
fn export_saved_entries(ctrl: State<'_, Arc<AppController>>) -> Result<String, String> {
    serde_json::to_string_pretty(&ctrl.storage.list_entries()).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_permissions_status(
    prompt: bool,
    ctrl: State<'_, Arc<AppController>>,
) -> PermissionsStatus {
    ctrl.platform.permissions_status(prompt)
}

#[tauri::command]
fn request_screen_recording(ctrl: State<'_, Arc<AppController>>) -> PermissionsStatus {
    let _ = ctrl.platform.request_screen_recording();
    ctrl.platform.permissions_status(false)
}

#[tauri::command]
fn open_main_tab(tab: String, ctrl: State<'_, Arc<AppController>>) {
    ctrl.open_main_window(&tab);
}

// ============================================================================
// Application Bootstrap & System Tray Setup
// ============================================================================

fn setup_tray(app: &AppHandle, ctrl: &Arc<AppController>) -> tauri::Result<()> {
    let initial_settings = ctrl.storage.settings();
    let toggle_item = CheckMenuItem::with_id(
        app,
        "toggle_enabled",
        "启用取词与快捷键翻译",
        true,
        initial_settings.enabled,
        None::<&str>,
    )?;
    let home_item = MenuItem::with_id(app, "open_home", "翻译概览与热力图…", true, None::<&str>)?;
    let manual_item = MenuItem::with_id(app, "open_translate", "手动对照翻译…", true, None::<&str>)?;
    let capture_item = MenuItem::with_id(
        app,
        "trigger_capture",
        format_capture_tray_label(&initial_settings.capture_shortcut),
        true,
        None::<&str>,
    )?;
    *ctrl.tray_toggle_item.lock() = Some(toggle_item.clone());
    *ctrl.tray_capture_item.lock() = Some(capture_item.clone());
    let wordbook_item = MenuItem::with_id(app, "open_wordbook", "生词收藏本…", true, None::<&str>)?;
    let settings_item = MenuItem::with_id(app, "open_settings", "偏好设置…", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit_app", "退出 Light Translate", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &toggle_item,
            &PredefinedMenuItem::separator(app)?,
            &home_item,
            &manual_item,
            &capture_item,
            &wordbook_item,
            &settings_item,
            &PredefinedMenuItem::separator(app)?,
            &quit_item,
        ],
    )?;

    let tray_icon_bytes = include_bytes!("../icons/tray-icon.png");
    let icon = tauri::image::Image::from_bytes(tray_icon_bytes)?;

    let ctrl_menu = Arc::clone(ctrl);
    let toggle_clone = toggle_item.clone();

    TrayIconBuilder::new()
        .icon(icon)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("Light Translate 2.0")
        .menu(&menu)
        .show_menu_on_left_click(cfg!(target_os = "macos"))
        .on_tray_icon_event({
            let ctrl_click = Arc::clone(ctrl);
            move |_tray, event| {
                if !cfg!(target_os = "macos") {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        ctrl_click.open_main_window("home");
                    }
                }
            }
        })
        .on_menu_event(move |app_handle, event| match event.id().as_ref() {
            "toggle_enabled" => {
                let mut s = ctrl_menu.storage.settings();
                s.enabled = !s.enabled;
                let _ = toggle_clone.set_checked(s.enabled);
                let _ = ctrl_menu.storage.update_settings(s.clone());
                let _ = app_handle.emit("settings://changed", &s);
            }
            "open_home" => ctrl_menu.open_main_window("home"),
            "open_translate" => ctrl_menu.open_main_window("translate"),
            "trigger_capture" => {
                let ctrl_cap = Arc::clone(&ctrl_menu);
                tauri::async_runtime::spawn(async move {
                    ctrl_cap.open_capture_overlay(false).await;
                });
            }
            "open_wordbook" => ctrl_menu.open_main_window("wordbook"),
            "open_settings" => ctrl_menu.open_main_window("settings"),
            "quit_app" => app_handle.exit(0),
            _ => {}
        })
        .build(app)?;

    let _ = (PhysicalPosition::new(0, 0), PhysicalSize::new(0, 0));
    Ok(())
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "light_translate_v2=info".into()),
        )
        .init();

    let app = tauri::Builder::default()
        .setup(|app| {
            let platform = create_platform_backend();
            let storage = AppStorage::open_default();
            let settings = storage.settings();

            platform.update_trigger_config(
                settings.trigger_modifier,
                settings.double_tap_selection,
                &settings.selection_shortcut,
                &settings.capture_shortcut,
            );

            let controller = Arc::new(AppController {
                app: app.handle().clone(),
                platform: Arc::clone(&platform),
                wordbook: WordBook::bundled(),
                storage,
                secrets: Box::new(SystemKeyringStore::new()),
                cache: TranslationCache::new(512, Duration::from_secs(30 * 60)),
                gate: RequestGate::new(),
                overlay_gate: RequestGate::new(),
                state: Mutex::new(CoordinatorStateMachine::new()),
                overlay_state: Mutex::new(OverlayRuntimeState::default()),
                trigger_press_count: AtomicU64::new(0),
                tray_toggle_item: Mutex::new(None),
                tray_capture_item: Mutex::new(None),
            });
            controller.write_diagnostics("startup", "ready", None);

            if let Some(popup) = app.get_webview_window("popup") {
                platform.configure_popup_window(&popup);
            }
            if let Some(overlay_win) = app.get_webview_window("overlay") {
                platform.configure_overlay_window(&overlay_win);
            }

            // Ensure main window opens focused on launch so the Heatmap is immediately visible,
            // and intercept its close button so closing hides to tray/Dock instead of quitting.
            if let Some(main_win) = app.get_webview_window("main") {
                let _ = main_win.show();
                let _ = main_win.set_focus();
                let main_clone = main_win.clone();
                main_win.on_window_event(move |ev| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = ev {
                        api.prevent_close();
                        let _ = main_clone.hide();
                    }
                });
            }

            setup_tray(app.handle(), &controller)?;

            // Start native input monitor
            let ctrl_input = Arc::clone(&controller);
            platform.start_input_monitor(Arc::new(move |ev| {
                ctrl_input.handle_input_event(ev);
            }));

            // Start 25ms state-machine heartbeat timer for hover dwell & linger auto-dismiss
            let ctrl_tick = Arc::clone(&controller);
            tauri::async_runtime::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_millis(25));
                loop {
                    interval.tick().await;
                    ctrl_tick.on_timer_tick();
                }
            });

            app.manage(controller);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_popup_state,
            popup_expand_sentence,
            popup_explain,
            popup_toggle_pin,
            popup_copy_text,
            popup_save_entry,
            popup_resize,
            popup_dismiss,
            start_region_capture,
            cancel_region_capture,
            get_capture_backdrop,
            submit_region_capture,
            get_overlay_state,
            get_overlay_crop_image,
            overlay_toggle_pin,
            overlay_move_by,
            overlay_dismiss,
            get_usage_stats,
            get_settings,
            save_settings,
            set_shortcut_recording,
            get_api_key_status,
            set_api_key,
            test_provider_connection,
            manual_translate_stream,
            list_saved_entries,
            save_manual_entry,
            delete_saved_entry,
            import_saved_entries,
            export_saved_entries,
            get_permissions_status,
            request_screen_recording,
            open_main_tab,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Light Translate v2");

    app.run(|app_handle, event| {
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen { .. } = event {
            if let Some(ctrl) = app_handle.try_state::<Arc<AppController>>() {
                let is_vis = app_handle
                    .get_webview_window("main")
                    .and_then(|w| w.is_visible().ok())
                    .unwrap_or(false);
                if !is_vis {
                    ctrl.open_main_window("home");
                } else if let Some(win) = app_handle.get_webview_window("main") {
                    let _ = win.set_focus();
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (app_handle, event);
    });
}

#[cfg(test)]
mod hover_flow_tests {
    use super::*;
    use crate::extraction::ExtractionSource;
    use crate::platform::{InputEventCallback, PermissionsStatus};
    use crate::storage::TriggerModifier;

    struct FakeHoverPlatform {
        focused_app: Option<String>,
        app_at_point: Option<String>,
        selection: Option<ExtractionSnapshot>,
        ax_result: Result<ExtractionSnapshot, ExtractionFailure>,
        ocr_result: Result<ExtractionSnapshot, ExtractionFailure>,
    }

    impl PlatformBackend for FakeHoverPlatform {
        fn start_input_monitor(&self, _callback: InputEventCallback) {}
        fn update_trigger_config(&self, _: TriggerModifier, _: bool, _: &str, _: &str) {}
        fn current_pointer(&self) -> ScreenPoint {
            ScreenPoint::new(412.0, 331.0)
        }
        fn monitor_bounds_at(&self, _: ScreenPoint) -> ScreenRect {
            ScreenRect::new(0.0, 0.0, 1440.0, 900.0)
        }
        fn app_id_at_point(&self, _: ScreenPoint) -> Option<String> {
            self.app_at_point.clone()
        }
        fn focused_app_id(&self) -> Option<String> {
            self.focused_app.clone()
        }
        fn extract_selection(
            &self,
            _: ScreenPoint,
            _: u64,
            _: bool,
        ) -> Result<Option<ExtractionSnapshot>, ExtractionFailure> {
            Ok(self.selection.clone())
        }
        fn extract_hover_accessibility(
            &self,
            _: ScreenPoint,
            _: bool,
            _: u64,
        ) -> Result<ExtractionSnapshot, ExtractionFailure> {
            self.ax_result.clone()
        }
        fn extract_hover_ocr(
            &self,
            _: ScreenPoint,
            _: bool,
            _: u64,
        ) -> Result<ExtractionSnapshot, ExtractionFailure> {
            self.ocr_result.clone()
        }
        fn extract_region_ocr(
            &self,
            _: ScreenRect,
            _: u64,
        ) -> Result<ExtractionSnapshot, ExtractionFailure> {
            Err(ExtractionFailure::NotSupported)
        }
        fn permissions_status(&self, _: bool) -> PermissionsStatus {
            PermissionsStatus {
                platform: "test".into(),
                accessibility: true,
                screen_recording: true,
            }
        }
        fn write_clipboard(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn configure_popup_window(&self, _: &tauri::WebviewWindow) {}
        fn present_popup(&self, _: &tauri::WebviewWindow, _: ScreenRect, _: bool) {}
        fn set_popup_interactive(&self, _: &tauri::WebviewWindow, _: bool) {}
        fn hide_popup(&self, _: &tauri::WebviewWindow) {}
    }

    #[test]
    fn chrome_flags_heading_refines_to_hovered_word_input_via_ocr_and_ignores_distant_selection() {
        let pointer = ScreenPoint::new(412.0, 331.0);
        let input_rect = ScreenRect::new(392.0, 320.0, 42.0, 20.0);

        let platform = FakeHoverPlatform {
            focused_app: Some("com.google.Chrome".into()),
            app_at_point: Some("com.google.Chrome".into()),
            // Distant "Prompt" selected in top search bar (y = 110) while hovering "Input" (y = 331)
            selection: Some(ExtractionSnapshot {
                generation: 1,
                text: "Prompt".into(),
                context: None,
                sentence_completeness: None,
                scope: ExtractionScope::Selection,
                completeness: ExtractionCompleteness::Complete,
                source: ExtractionSource::Accessibility,
                anchor_rect: Some(ScreenRect::new(240.0, 110.0, 56.0, 20.0)),
                pointer,
                app_id: Some("com.google.Chrome".into()),
            }),
            // AX has no RangeForPosition on Chrome heading, returning whole multi-word Label
            ax_result: Ok(ExtractionSnapshot {
                generation: 1,
                text: "Prompt API Multimodal Input".into(),
                context: None,
                sentence_completeness: None,
                scope: ExtractionScope::Label,
                completeness: ExtractionCompleteness::Complete,
                source: ExtractionSource::Accessibility,
                anchor_rect: Some(ScreenRect::new(180.0, 320.0, 260.0, 22.0)),
                pointer,
                app_id: Some("com.google.Chrome".into()),
            }),
            // Local Vision OCR extracts exact word "Input" at pointer
            ocr_result: Ok(ExtractionSnapshot {
                generation: 1,
                text: "Input".into(),
                context: Some("Prompt API Multimodal Input".into()),
                sentence_completeness: Some(ExtractionCompleteness::Partial),
                scope: ExtractionScope::Word,
                completeness: ExtractionCompleteness::Complete,
                source: ExtractionSource::Ocr,
                anchor_rect: Some(input_rect),
                pointer,
                app_id: Some("com.google.Chrome".into()),
            }),
        };

        let resolved = execute_hover_extraction(&platform, pointer, false, true, 1)
            .expect("hover extraction must succeed");
        assert_eq!(resolved.text, "Input");
        assert_eq!(resolved.scope, ExtractionScope::Word);
        assert_eq!(resolved.source, ExtractionSource::Ocr);
        assert_eq!(resolved.anchor_rect, Some(input_rect));

        // When OCR is disabled, gracefully keeps the AX multi-word label instead of failing
        let fallback_label = execute_hover_extraction(&platform, pointer, false, false, 2)
            .expect("must keep AX label when OCR is disabled");
        assert_eq!(fallback_label.text, "Prompt API Multimodal Input");
        assert_eq!(fallback_label.scope, ExtractionScope::Label);
    }
}
