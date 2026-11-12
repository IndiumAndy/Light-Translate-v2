use crate::dictionary::WordEntry;
use crate::extraction::{
    ExtractionCompleteness, ExtractionScope, ExtractionSource, ScreenPoint, ScreenRect,
};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Atomic generation gate ensuring stale extractions or out-of-date streams never overwrite newer queries,
/// while guaranteeing that releasing the trigger key NEVER aborts an already-presented card's stream.
#[derive(Default)]
pub struct RequestGate {
    current: AtomicU64,
}

impl RequestGate {
    pub fn new() -> Self {
        Self {
            current: AtomicU64::new(0),
        }
    }

    pub fn advance(&self) -> u64 {
        self.current.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn current(&self) -> u64 {
        self.current.load(Ordering::SeqCst)
    }

    pub fn is_current(&self, generation: u64) -> bool {
        generation != 0 && self.current.load(Ordering::SeqCst) == generation
    }

    pub fn invalidate(&self) {
        self.current.fetch_add(1, Ordering::SeqCst);
    }
}

/// Complete state payload pushed to `popup.html` over Tauri events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PopupCardViewState {
    pub visible: bool,
    pub generation: u64,
    pub original_text: String,
    pub context_sentence: Option<String>,
    pub sentence_completeness: Option<ExtractionCompleteness>,
    pub scope: ExtractionScope,
    pub completeness: ExtractionCompleteness,
    pub source: ExtractionSource,
    pub dictionary: Option<WordEntry>,
    pub fast_translation: Option<String>,
    pub llm_translation: String,
    pub is_streaming: bool,
    pub translation_error: Option<String>,
    pub explanation: Option<String>,
    pub is_explaining: bool,
    pub explanation_error: Option<String>,
    pub status_banner: Option<String>,
    pub interactive: bool,
    pub pinned: bool,
    pub saved: bool,
    pub ttft_ms: Option<u64>,
}

impl Default for PopupCardViewState {
    fn default() -> Self {
        Self {
            visible: false,
            generation: 0,
            original_text: String::new(),
            context_sentence: None,
            sentence_completeness: None,
            scope: ExtractionScope::Word,
            completeness: ExtractionCompleteness::Complete,
            source: ExtractionSource::Accessibility,
            dictionary: None,
            fast_translation: None,
            llm_translation: String::new(),
            is_streaming: false,
            translation_error: None,
            explanation: None,
            is_explaining: false,
            explanation_error: None,
            status_banner: None,
            interactive: false,
            pinned: false,
            saved: false,
            ttft_ms: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardPhase {
    Hidden,
    /// Trigger key is currently held: card is visible in mouse pass-through mode (`ignoresMouseEvents = true`).
    HoldingPassThrough { generation: u64 },
    /// Trigger key released (or selection/capture triggered): card is interactive and stays open until `deadline`.
    /// Note: while `is_streaming` or `is_explaining` is true, auto-dismiss is deferred!
    Lingering { generation: u64, deadline: Instant },
    /// Mouse pointer is inside the card: auto-dismiss timer paused so user can read, copy, click `[译整句]`, etc.
    PointerInside { generation: u64 },
    /// User clicked Pin: stays open until unpinned, Esc, or explicit close.
    Pinned { generation: u64 },
}

/// Pure, deterministic state machine governing trigger dwell, 8pt debounce,
/// hold-release persistence ("松开修饰键绝不断开翻译"), and card auto-dismiss.
pub struct CoordinatorStateMachine {
    pub trigger_held: bool,
    pub sentence_modifier_held: bool,
    pub pointer: ScreenPoint,
    pub dwell_anchor: Option<ScreenPoint>,
    pub dwell_deadline: Option<Instant>,
    pub active_anchor_rect: Option<ScreenRect>,
    pub popup_rect: Option<ScreenRect>,
    pub phase: CardPhase,
    pub card: PopupCardViewState,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StateTransitionAction {
    None,
    StartHoverExtraction {
        pointer: ScreenPoint,
        sentence_mode: bool,
    },
    StartTapExtraction {
        pointer: ScreenPoint,
        sentence_mode: bool,
    },
    ExpandCurrentCardToSentence,
    SetPopupInteractive(bool),
    DismissPopup,
}
impl Default for CoordinatorStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl CoordinatorStateMachine {
    pub fn new() -> Self {
        Self {
            trigger_held: false,
            sentence_modifier_held: false,
            pointer: ScreenPoint::default(),
            dwell_anchor: None,
            dwell_deadline: None,
            active_anchor_rect: None,
            popup_rect: None,
            phase: CardPhase::Hidden,
            card: PopupCardViewState::default(),
        }
    }

    pub fn on_trigger_pressed(
        &mut self,
        pointer: ScreenPoint,
        sentence_modifier: bool,
        now: Instant,
        dwell_ms: u64,
    ) -> StateTransitionAction {
        self.trigger_held = true;
        self.sentence_modifier_held = sentence_modifier;
        self.pointer = pointer;

        // Do not cover or disturb a Pinned card unless pointer is outside it
        if matches!(self.phase, CardPhase::Pinned { .. }) {
            if self.popup_rect.is_some_and(|r| r.contains(pointer)) {
                return StateTransitionAction::None;
            }
        }

        self.dwell_anchor = Some(pointer);
        self.dwell_deadline = Some(now + Duration::from_millis(dwell_ms));
        StateTransitionAction::None
    }

    /// Handles trigger key release according to user preference:
    /// - When `dismiss_on_release` is `true` ("按住即看、松开即关"): immediately dismisses any
    ///   hover card in `HoldingPassThrough` and cancels pending dwell, while leaving `Pinned`
    ///   or explicit interactive (`Lingering`/`PointerInside`) cards untouched.
    /// - When `dismiss_on_release` is `false`: transitions `HoldingPassThrough` smoothly into
    ///   `Lingering` (`interactive = true`) while streaming continues uninterrupted, and supports
    ///   clean single-tap extraction.
    pub fn on_trigger_released(
        &mut self,
        pointer: ScreenPoint,
        clean_tap: bool,
        now: Instant,
        dismiss_on_release: bool,
        linger_ms: u64,
    ) -> StateTransitionAction {
        let had_pending_dwell = self.dwell_deadline.is_some();
        let was_sentence_mod = self.sentence_modifier_held;

        self.trigger_held = false;
        self.sentence_modifier_held = false;
        self.pointer = pointer;
        self.dwell_anchor = None;
        self.dwell_deadline = None;

        match self.phase {
            CardPhase::HoldingPassThrough { generation } => {
                if dismiss_on_release {
                    return self.dismiss();
                }
                self.card.interactive = true;
                if self.popup_rect.is_some_and(|r| r.contains(pointer)) {
                    self.phase = CardPhase::PointerInside { generation };
                } else {
                    self.phase = CardPhase::Lingering {
                        generation,
                        deadline: now + Duration::from_millis(linger_ms),
                    };
                }
                StateTransitionAction::SetPopupInteractive(true)
            }
            _ if clean_tap && had_pending_dwell => {
                StateTransitionAction::StartTapExtraction {
                    pointer,
                    sentence_mode: was_sentence_mod,
                }
            }
            _ => StateTransitionAction::None,
        }
    }

    pub fn on_other_key_pressed(&mut self) -> StateTransitionAction {
        self.dwell_anchor = None;
        self.dwell_deadline = None;
        StateTransitionAction::None
    }

    pub fn on_sentence_modifier_changed(
        &mut self,
        active: bool,
        pointer: ScreenPoint,
    ) -> StateTransitionAction {
        self.sentence_modifier_held = active;
        self.pointer = pointer;
        if active && self.trigger_held {
            // If a word card is already shown with sentence context, upgrade in-place immediately!
            if self.card.visible && self.card.scope == ExtractionScope::Word {
                if self.card.context_sentence.is_some() {
                    return StateTransitionAction::ExpandCurrentCardToSentence;
                }
            }
            // Otherwise trigger immediate sentence extraction without waiting another dwell
            self.dwell_deadline = None;
            return StateTransitionAction::StartHoverExtraction {
                pointer,
                sentence_mode: true,
            };
        }
        StateTransitionAction::None
    }

    pub fn on_pointer_moved(
        &mut self,
        pointer: ScreenPoint,
        now: Instant,
        dwell_ms: u64,
        linger_ms: u64,
        debounce_radius_pt: f64,
    ) -> StateTransitionAction {
        self.pointer = pointer;

        // 1. Handle hover enter/leave on interactive card when trigger is NOT held
        if !self.trigger_held {
            let inside_popup = self.popup_rect.is_some_and(|r| r.contains(pointer));
            match self.phase {
                CardPhase::Lingering { generation, .. } if inside_popup => {
                    self.phase = CardPhase::PointerInside { generation };
                }
                CardPhase::PointerInside { generation } if !inside_popup => {
                    self.phase = CardPhase::Lingering {
                        generation,
                        deadline: now + Duration::from_millis(linger_ms),
                    };
                }
                _ => {}
            }
            return StateTransitionAction::None;
        }

        // 2. Trigger IS held: enforce 8pt debounce radius & current word anchor rect
        if let Some(anchor_rect) = self.active_anchor_rect {
            let plausible_anchor = if self.card.scope == ExtractionScope::Word {
                anchor_rect.width <= 160.0 && anchor_rect.height <= 44.0
            } else {
                true
            };
            if plausible_anchor && anchor_rect.inset_by(-3.0, -3.0).contains(pointer) {
                // Still hovering on the exact same word/sentence -> keep steady!
                self.dwell_deadline = None;
                return StateTransitionAction::None;
            }
        }

        if let Some(anchor_pt) = self.dwell_anchor {
            if anchor_pt.distance_to(pointer) < debounce_radius_pt {
                // Micro hand tremor within 8pt -> do NOT reset dwell timer!
                return StateTransitionAction::None;
            }
        }

        // Pointer genuinely moved to a new spot while holding trigger -> reset dwell
        self.dwell_anchor = Some(pointer);
        self.dwell_deadline = Some(now + Duration::from_millis(dwell_ms));
        StateTransitionAction::None
    }

    pub fn on_tick(&mut self, now: Instant, linger_ms: u64) -> StateTransitionAction {
        // 1. Check pending hover dwell
        if self.trigger_held {
            if let Some(deadline) = self.dwell_deadline {
                if now >= deadline {
                    self.dwell_deadline = None;
                    return StateTransitionAction::StartHoverExtraction {
                        pointer: self.pointer,
                        sentence_mode: self.sentence_modifier_held,
                    };
                }
            }
        }

        // 2. Check lingering card auto-dismiss
        if let CardPhase::Lingering {
            generation,
            deadline,
        } = self.phase
        {
            if self.card.is_streaming || self.card.is_explaining {
                // Defer auto-dismiss while tokens are actively streaming so translation never cuts off!
                self.phase = CardPhase::Lingering {
                    generation,
                    deadline: now + Duration::from_millis(linger_ms),
                };
                return StateTransitionAction::None;
            }
            if now >= deadline {
                return self.dismiss();
            }
        }

        StateTransitionAction::None
    }

    pub fn on_global_mouse_down(&mut self, pointer: ScreenPoint) -> StateTransitionAction {
        if !self.card.visible {
            return StateTransitionAction::None;
        }
        if matches!(self.phase, CardPhase::Pinned { .. }) {
            return StateTransitionAction::None;
        }
        if self.popup_rect.is_some_and(|r| r.contains(pointer)) {
            return StateTransitionAction::None;
        }
        self.dismiss()
    }

    pub fn on_escape(&mut self) -> StateTransitionAction {
        if self.card.visible || self.dwell_deadline.is_some() {
            self.dwell_deadline = None;
            return self.dismiss();
        }
        StateTransitionAction::None
    }

    pub fn toggle_pin(&mut self, now: Instant, linger_ms: u64) -> bool {
        if !self.card.visible {
            return false;
        }
        let gen = self.card.generation;
        if self.card.pinned {
            self.card.pinned = false;
            self.phase = CardPhase::Lingering {
                generation: gen,
                deadline: now + Duration::from_millis(linger_ms),
            };
        } else {
            self.card.pinned = true;
            self.card.interactive = true;
            self.phase = CardPhase::Pinned { generation: gen };
        }
        self.card.pinned
    }

    pub fn present_card(
        &mut self,
        mut next_card: PopupCardViewState,
        anchor_rect: Option<ScreenRect>,
        popup_rect: ScreenRect,
        force_interactive: bool,
        now: Instant,
        linger_ms: u64,
    ) {
        let gen = next_card.generation;
        self.active_anchor_rect = anchor_rect;
        self.popup_rect = Some(popup_rect);

        if self.trigger_held && !force_interactive {
            next_card.interactive = false;
            self.phase = CardPhase::HoldingPassThrough { generation: gen };
        } else {
            next_card.interactive = true;
            self.phase = CardPhase::Lingering {
                generation: gen,
                deadline: now + Duration::from_millis(linger_ms),
            };
        }
        next_card.visible = true;
        self.card = next_card;
    }

    pub fn dismiss(&mut self) -> StateTransitionAction {
        self.phase = CardPhase::Hidden;
        self.active_anchor_rect = None;
        self.popup_rect = None;
        self.card = PopupCardViewState::default();
        StateTransitionAction::DismissPopup
    }
}

/// Computes an unobstructed screen position for the popup card near `anchor_rect` or `pointer`,
/// flipping above when near the bottom edge of the monitor.
pub fn compute_popup_rect(
    anchor_rect: Option<ScreenRect>,
    pointer: ScreenPoint,
    monitor: ScreenRect,
    width: f64,
    height: f64,
) -> ScreenRect {
    let margin = 12.0;
    let gap = 10.0;
    let target = anchor_rect
        .filter(|r| r.is_valid())
        .unwrap_or_else(|| ScreenRect::new(pointer.x - 8.0, pointer.y - 10.0, 16.0, 20.0));

    let mut x = target.min_x();
    if x + width > monitor.max_x() - margin {
        x = (monitor.max_x() - margin - width).max(monitor.min_x() + margin);
    }
    if x < monitor.min_x() + margin {
        x = monitor.min_x() + margin;
    }

    let below_y = target.max_y() + gap;
    let above_y = target.min_y() - gap - height;
    let y = if below_y + height <= monitor.max_y() - margin {
        below_y
    } else if above_y >= monitor.min_y() + margin {
        above_y
    } else {
        (monitor.max_y() - margin - height).max(monitor.min_y() + margin)
    };

    ScreenRect::new(x.round(), y.round(), width.round(), height.round())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn releasing_trigger_keeps_streaming_card_alive_and_switches_interactive() {
        let mut sm = CoordinatorStateMachine::new();
        let t0 = Instant::now();

        // 1. User presses Option and hovers
        sm.on_trigger_pressed(ScreenPoint::new(300.0, 200.0), false, t0, 150);
        // Micro-jitter of 5pt (< 8pt debounce) does not delay dwell!
        sm.on_pointer_moved(
            ScreenPoint::new(303.0, 204.0),
            t0 + Duration::from_millis(80),
            150,
            2500,
            8.0,
        );
        let act = sm.on_tick(t0 + Duration::from_millis(155), 2500);
        assert_eq!(
            act,
            StateTransitionAction::StartHoverExtraction {
                pointer: ScreenPoint::new(303.0, 204.0),
                sentence_mode: false
            }
        );

        // 2. Card is presented while streaming is still in progress
        let mut card = PopupCardViewState {
            generation: 1,
            original_text: "ephemeral".into(),
            context_sentence: Some("An ephemeral token.".into()),
            is_streaming: true,
            ..Default::default()
        };
        let popup_rect = ScreenRect::new(300.0, 225.0, 360.0, 180.0);
        sm.present_card(
            card.clone(),
            Some(ScreenRect::new(295.0, 195.0, 60.0, 18.0)),
            popup_rect,
            false,
            t0 + Duration::from_millis(165),
            2500,
        );
        assert!(!sm.card.interactive);

        // 3. When linger mode is active (`dismiss_on_release = false`), releasing Option keeps card alive
        let rel = sm.on_trigger_released(
            ScreenPoint::new(303.0, 204.0),
            false,
            t0 + Duration::from_millis(200),
            false,
            2500,
        );
        assert_eq!(rel, StateTransitionAction::SetPopupInteractive(true));
        assert!(sm.card.visible, "card MUST stay visible on key release");
        assert!(sm.card.interactive, "card switches to interactive mode");

        // 4. Even 4 seconds later, while `is_streaming == true`, card does NOT auto-dismiss!
        assert_eq!(
            sm.on_tick(t0 + Duration::from_millis(4200), 2500),
            StateTransitionAction::None
        );
        assert!(sm.card.visible);

        // 5. Stream finishes -> after linger_ms (2500ms) outside card, it cleanly dismisses
        sm.card.is_streaming = false;
        assert_eq!(
            sm.on_tick(t0 + Duration::from_millis(6800), 2500),
            StateTransitionAction::DismissPopup
        );
        assert!(!sm.card.visible);

        // Suppress unused warning
        card.visible = false;
    }

    #[test]
    fn pointer_entering_card_pauses_auto_dismiss() {
        let mut sm = CoordinatorStateMachine::new();
        let t0 = Instant::now();
        let popup = ScreenRect::new(100.0, 100.0, 360.0, 180.0);
        sm.present_card(
            PopupCardViewState {
                generation: 2,
                original_text: "hello".into(),
                ..Default::default()
            },
            None,
            popup,
            true,
            t0,
            2500,
        );

        // Move pointer inside popup at t0 + 1000ms
        sm.on_pointer_moved(ScreenPoint::new(200.0, 150.0), t0 + Duration::from_millis(1000), 150, 2500, 8.0);
        assert!(matches!(sm.phase, CardPhase::PointerInside { .. }));

        // At t0 + 10_000ms, still inside popup -> never dismisses!
        assert_eq!(
            sm.on_tick(t0 + Duration::from_millis(10_000), 2500),
            StateTransitionAction::None
        );
        assert!(sm.card.visible);
    }

    #[test]
    fn clean_single_tap_triggers_extraction_while_chord_cancels_dwell() {
        let mut sm = CoordinatorStateMachine::new();
        let t0 = Instant::now();
        let pt = ScreenPoint::new(420.0, 310.0);

        // Quick standalone tap (< 150ms dwell) with linger mode -> starts tap extraction immediately on release!
        sm.on_trigger_pressed(pt, false, t0, 150);
        let tap_act =
            sm.on_trigger_released(pt, true, t0 + Duration::from_millis(75), false, 2500);
        assert_eq!(
            tap_act,
            StateTransitionAction::StartTapExtraction {
                pointer: pt,
                sentence_mode: false
            }
        );

        // Quick standalone tap ALSO works when `dismiss_on_release = true`!
        sm.on_trigger_pressed(pt, false, t0 + Duration::from_millis(200), 150);
        let tap_act_dismiss_mode =
            sm.on_trigger_released(pt, true, t0 + Duration::from_millis(275), true, 2500);
        assert_eq!(
            tap_act_dismiss_mode,
            StateTransitionAction::StartTapExtraction {
                pointer: pt,
                sentence_mode: false
            }
        );

        // Option+Key chord (`on_other_key_pressed` before release) -> never false-triggers!
        sm.on_trigger_pressed(pt, false, t0 + Duration::from_millis(500), 150);
        sm.on_other_key_pressed();
        let chord_act =
            sm.on_trigger_released(pt, false, t0 + Duration::from_millis(580), false, 2500);
        assert_eq!(chord_act, StateTransitionAction::None);
    }

    #[test]
    fn releasing_trigger_immediately_dismisses_hover_card_when_enabled() {
        let mut sm = CoordinatorStateMachine::new();
        let t0 = Instant::now();
        let pt = ScreenPoint::new(260.0, 180.0);

        sm.on_trigger_pressed(pt, false, t0, 150);
        let _ = sm.on_tick(t0 + Duration::from_millis(160), 2500);

        sm.present_card(
            PopupCardViewState {
                generation: 1,
                original_text: "glass".into(),
                is_streaming: true,
                ..Default::default()
            },
            Some(ScreenRect::new(250.0, 172.0, 48.0, 18.0)),
            ScreenRect::new(250.0, 200.0, 360.0, 140.0),
            false,
            t0 + Duration::from_millis(170),
            2500,
        );
        assert!(matches!(sm.phase, CardPhase::HoldingPassThrough { .. }));

        // Releasing Option with `dismiss_on_release = true` closes hold-hover popup immediately!
        let rel = sm.on_trigger_released(pt, false, t0 + Duration::from_millis(240), true, 2500);
        assert_eq!(rel, StateTransitionAction::DismissPopup);
        assert!(!sm.card.visible);
        assert_eq!(sm.phase, CardPhase::Hidden);

        // Non-clean release before dwell (e.g., pointer moved or chord) cancels pending dwell cleanly
        sm.on_trigger_pressed(pt, false, t0 + Duration::from_millis(300), 150);
        let non_clean_rel =
            sm.on_trigger_released(pt, false, t0 + Duration::from_millis(360), true, 2500);
        assert_eq!(non_clean_rel, StateTransitionAction::None);
        assert!(!sm.card.visible);
    }
    #[test]
    fn oversized_word_anchor_rect_does_not_freeze_reextraction_when_sliding_cursor() {
        let mut sm = CoordinatorStateMachine::new();
        let t0 = Instant::now();
        let prompt_pt = ScreenPoint::new(205.0, 331.0);
        let input_pt = ScreenPoint::new(412.0, 331.0);

        sm.on_trigger_pressed(prompt_pt, false, t0, 100);
        let _ = sm.on_tick(t0 + Duration::from_millis(110), 2500);

        // Simulate a container-wide anchor rect (260pt wide across "Prompt API Multimodal Input")
        sm.present_card(
            PopupCardViewState {
                generation: 1,
                original_text: "Prompt".into(),
                scope: ExtractionScope::Word,
                ..Default::default()
            },
            Some(ScreenRect::new(180.0, 320.0, 260.0, 22.0)),
            ScreenRect::new(180.0, 350.0, 360.0, 140.0),
            false,
            t0 + Duration::from_millis(120),
            2500,
        );

        // Sliding pointer onto "Input" while keeping Option held must NOT be swallowed by the wide container rect!
        assert_eq!(
            sm.on_pointer_moved(input_pt, t0 + Duration::from_millis(200), 100, 2500, 8.0),
            StateTransitionAction::None
        );
        assert_eq!(
            sm.on_tick(t0 + Duration::from_millis(305), 2500),
            StateTransitionAction::StartHoverExtraction {
                pointer: input_pt,
                sentence_mode: false,
            }
        );
    }
}
