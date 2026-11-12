import React, { useEffect, useLayoutEffect, useRef, useState } from "react";
import ReactDOM from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  Bookmark,
  Check,
  Copy,
  Pin,
  Sparkles,
  TextQuote,
  X,
} from "lucide-react";
import type { PopupCardViewState } from "../shared/types";
import "../styles.css";

const IS_MAC_NATIVE_GLASS =
  typeof navigator !== "undefined" && /Mac/i.test(navigator.userAgent);

const EMPTY_STATE: PopupCardViewState = {
  visible: false,
  generation: 0,
  originalText: "",
  contextSentence: null,
  sentenceCompleteness: null,
  scope: "word",
  completeness: "complete",
  source: "accessibility",
  dictionary: null,
  fastTranslation: null,
  llmTranslation: "",
  isStreaming: false,
  translationError: null,
  explanation: null,
  isExplaining: false,
  explanationError: null,
  statusBanner: null,
  interactive: false,
  pinned: false,
  saved: false,
  ttftMs: null,
};

function PopupApp() {
  const [card, setCard] = useState<PopupCardViewState>(EMPTY_STATE);
  const [copied, setCopied] = useState(false);
  const [toast, setToast] = useState<string | null>(null);
  const contentRef = useRef<HTMLDivElement | null>(null);
  const lastHeightRef = useRef<number>(0);

  const isStatusOnly = Boolean(card.statusBanner && !card.originalText);

  useEffect(() => {
    invoke<PopupCardViewState>("get_popup_state")
      .then((initial) => setCard(initial))
      .catch(() => {});

    const unlistenPromise = listen<PopupCardViewState>("popup://state", (ev) => {
      setCard(ev.payload);
      setCopied(false);
      setToast(null);
    });

    return () => {
      unlistenPromise.then((fn) => fn());
    };
  }, []);

  // Measure intrinsic content height and sync native window height tightly without jitter
  useLayoutEffect(() => {
    if (!card.visible) {
      lastHeightRef.current = 0;
      return;
    }
    const el = contentRef.current;
    if (!el) return;

    const outerPadding = IS_MAC_NATIVE_GLASS ? 0 : 16;
    const syncHeight = () => {
      const measured = Math.ceil(el.getBoundingClientRect().height + outerPadding);
      if (measured > 0 && Math.abs(measured - lastHeightRef.current) >= 1) {
        lastHeightRef.current = measured;
        invoke("popup_resize", { height: measured }).catch(() => {});
      }
    };

    syncHeight();
    const observer = new ResizeObserver(() => {
      syncHeight();
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, [card.visible, isStatusOnly]);

  if (!card.visible) {
    return <div className="w-full h-full bg-transparent" />;
  }

  const shellClassName = IS_MAC_NATIVE_GLASS
    ? "min-h-screen max-h-screen w-full rounded-[16px] lt-popup-glass-mac overflow-y-auto overflow-x-hidden"
    : "p-2";

  const innerSurfaceClassName = IS_MAC_NATIVE_GLASS
    ? "w-full"
    : "rounded-[16px] lt-popup-glass-fallback overflow-hidden";

  // Status-only banner (e.g. permission reminder)
  if (isStatusOnly) {
    return (
      <div className={shellClassName}>
        <div className={innerSurfaceClassName}>
          <div
            ref={contentRef}
            className="px-3.5 py-2.5 text-[13px] leading-[1.45] text-[var(--lt-ink-primary)] flex items-center justify-between gap-2.5"
          >
            <span className="min-w-0 break-words">{card.statusBanner}</span>
            <button
              type="button"
              title="关闭 (Esc)"
              aria-label="关闭提示"
              onClick={() => invoke("popup_dismiss")}
              className="w-7 h-7 shrink-0 rounded-[8px] inline-flex items-center justify-center text-[var(--lt-ink-secondary)] hover:bg-[var(--lt-control-hover)] transition-colors cursor-pointer"
            >
              <X className="w-3.5 h-3.5" />
            </button>
          </div>
        </div>
      </div>
    );
  }

  const handleCopy = async () => {
    try {
      await invoke("popup_copy_text", { text: null });
      setCopied(true);
      setTimeout(() => setCopied(false), 1400);
    } catch (err) {
      setToast(String(err));
    }
  };

  const handleSave = async () => {
    try {
      const msg = await invoke<string>("popup_save_entry");
      setToast(msg);
      setTimeout(() => setToast(null), 1600);
    } catch (err) {
      setToast(String(err));
    }
  };

  const primaryTranslation = card.llmTranslation || card.fastTranslation || "";
  const hasDictionary = Boolean(card.dictionary && card.dictionary.senses.length > 0);
  const showTranslationSection = Boolean(
    primaryTranslation || card.isStreaming || card.translationError
  );
  const showTranslationDivider =
    showTranslationSection && (hasDictionary || card.scope !== "word");
  const canExpandSentence =
    card.scope === "word" &&
    (!!card.contextSentence || card.source === "accessibility" || card.source === "ocr");

  return (
    <div className={shellClassName}>
      <div className={innerSurfaceClassName}>
        <div ref={contentRef} className="px-4 py-3">
          {/* Source Text — immediate focus, zero top header clutter */}
          <div
            className={`select-text break-words ${
              card.scope === "word"
                ? "text-[17px] leading-[1.3] font-semibold tracking-[-0.015em] text-[var(--lt-ink-primary)]"
                : "text-[13px] leading-[1.5] font-normal text-[var(--lt-ink-secondary)] line-clamp-3"
            }`}
          >
            {card.originalText}
          </div>

          {/* Tier 1: Instant Offline WordBook (< 5ms) */}
          {hasDictionary && card.dictionary && (
            <div className="mt-2 space-y-1">
              {card.dictionary.senses.map((sense, idx) => (
                <div
                  key={idx}
                  className="flex items-baseline gap-2 text-[13.5px] leading-[1.48]"
                >
                  {sense.partOfSpeech && (
                    <span className="shrink-0 font-mono text-[11px] font-medium px-1.5 py-[1px] rounded-[5px] bg-[var(--lt-surface-subtle)] text-[var(--lt-ink-secondary)]">
                      {sense.partOfSpeech}
                    </span>
                  )}
                  <span className="select-text text-[var(--lt-ink-primary)]">
                    {sense.glosses.join("；")}
                  </span>
                </div>
              ))}
            </div>
          )}

          {/* Tier 2 & Tier 3: Fast MT + SSE Streaming LLM Translation */}
          {showTranslationSection && (
            <div
              className={
                showTranslationDivider
                  ? "mt-2 pt-2 border-t border-[var(--lt-hairline)]"
                  : "mt-1.5"
              }
            >
              {primaryTranslation ? (
                <div className="select-text text-[14.5px] leading-[1.58] font-normal text-[var(--lt-ink-primary)] break-words whitespace-pre-wrap">
                  {primaryTranslation}
                  {card.isStreaming && (
                    <span className="inline-block w-[5px] h-[14px] ml-1 align-[-2px] rounded-[1.5px] bg-[var(--lt-ink-primary)] animate-pulse motion-reduce:animate-none" />
                  )}
                </div>
              ) : card.isStreaming ? (
                <div className="flex items-center gap-2 text-[13px] text-[var(--lt-ink-muted)] py-0.5 whitespace-nowrap">
                  <span className="w-1.5 h-1.5 rounded-full bg-[var(--lt-ink-primary)] animate-pulse motion-reduce:animate-none" />
                  <span>翻译中…</span>
                </div>
              ) : null}

              {card.translationError && (
                <div className="mt-1 text-[12px] leading-[1.45] text-rose-600 dark:text-rose-400">
                  {card.translationError}
                </div>
              )}
            </div>
          )}

          {/* On-Demand Contextual AI Explanation (tonal inset, never glass-on-glass) */}
          {(card.isExplaining || card.explanation || card.explanationError) && (
            <div className="mt-2.5 rounded-[11px] px-3 py-2 bg-[var(--lt-surface-subtle)] space-y-1">
              <div className="text-[11px] font-medium text-[var(--lt-ink-secondary)] flex items-center gap-1 whitespace-nowrap">
                <Sparkles className="w-3 h-3 text-[var(--lt-ink-primary)]" />
                <span>AI 语境解析</span>
              </div>
              {card.explanation ? (
                <div className="select-text text-[13px] leading-[1.56] text-[var(--lt-ink-primary)] whitespace-pre-wrap break-words">
                  {card.explanation}
                  {card.isExplaining && (
                    <span className="inline-block w-[5px] h-[12px] ml-1 align-[-1px] rounded-[1.5px] bg-[var(--lt-ink-primary)] animate-pulse motion-reduce:animate-none" />
                  )}
                </div>
              ) : card.isExplaining ? (
                <div className="text-[12px] text-[var(--lt-ink-muted)] whitespace-nowrap">
                  正在结合上下文分析…
                </div>
              ) : null}
              {card.explanationError && (
                <div className="text-[12px] text-rose-600 dark:text-rose-400">
                  {card.explanationError}
                </div>
              )}
            </div>
          )}

          {/* On-Demand Bottom Action Bar — hidden while holding modifier, aligned single row on release */}
          {card.interactive && (
            <div className="mt-2.5 pt-2 border-t border-[var(--lt-hairline)] flex items-center justify-between gap-2 whitespace-nowrap select-none">
              <div className="flex items-center gap-2 min-w-0">
                {canExpandSentence && (
                  <button
                    type="button"
                    title="翻译所在整句"
                    aria-label="翻译所在整句"
                    onClick={() => invoke("popup_expand_sentence")}
                    className="h-7 px-2.5 rounded-[8px] text-[12px] font-medium inline-flex items-center gap-1.5 shrink-0 bg-[var(--lt-accent-bg)] text-[var(--lt-accent-fg)] hover:opacity-90 active:scale-[0.98] transition-all cursor-pointer"
                  >
                    <TextQuote className="w-3.5 h-3.5 shrink-0" />
                    <span>译整句</span>
                  </button>
                )}
                {toast && (
                  <span className="text-[11.5px] text-[var(--lt-ink-muted)] truncate">
                    {toast}
                  </span>
                )}
              </div>

              {/* 28x28pt desktop hit targets per Apple HIG */}
              <div className="flex items-center gap-0.5 shrink-0 ml-auto">
                <button
                  type="button"
                  title="AI 语境解析"
                  aria-label="AI 语境解析"
                  onClick={() => invoke("popup_explain")}
                  className={`w-7 h-7 rounded-[8px] inline-flex items-center justify-center active:scale-[0.96] transition-all cursor-pointer ${
                    card.isExplaining || card.explanation
                      ? "text-[var(--lt-ink-primary)] bg-[var(--lt-surface-active)]"
                      : "text-[var(--lt-ink-secondary)] hover:bg-[var(--lt-control-hover)]"
                  }`}
                >
                  <Sparkles className="w-3.5 h-3.5" />
                </button>

                <button
                  type="button"
                  title="复制译文"
                  aria-label="复制译文"
                  onClick={handleCopy}
                  className="w-7 h-7 rounded-[8px] inline-flex items-center justify-center text-[var(--lt-ink-secondary)] hover:bg-[var(--lt-control-hover)] active:scale-[0.96] transition-all cursor-pointer"
                >
                  {copied ? (
                    <Check className="w-3.5 h-3.5 text-[var(--lt-ink-primary)]" />
                  ) : (
                    <Copy className="w-3.5 h-3.5" />
                  )}
                </button>

                <button
                  type="button"
                  title="收藏词条"
                  aria-label="收藏词条"
                  onClick={handleSave}
                  className={`w-7 h-7 rounded-[8px] inline-flex items-center justify-center active:scale-[0.96] transition-all cursor-pointer ${
                    card.saved
                      ? "text-[var(--lt-ink-primary)] bg-[var(--lt-surface-active)]"
                      : "text-[var(--lt-ink-secondary)] hover:bg-[var(--lt-control-hover)]"
                  }`}
                >
                  <Bookmark
                    className="w-3.5 h-3.5"
                    fill={card.saved ? "currentColor" : "none"}
                  />
                </button>

                <button
                  type="button"
                  title={card.pinned ? "取消固定" : "固定浮窗"}
                  aria-label={card.pinned ? "取消固定" : "固定浮窗"}
                  onClick={() => invoke("popup_toggle_pin")}
                  className={`w-7 h-7 rounded-[8px] inline-flex items-center justify-center active:scale-[0.96] transition-all cursor-pointer ${
                    card.pinned
                      ? "text-[var(--lt-accent-fg)] bg-[var(--lt-accent-bg)]"
                      : "text-[var(--lt-ink-secondary)] hover:bg-[var(--lt-control-hover)]"
                  }`}
                >
                  <Pin className="w-3.5 h-3.5" />
                </button>

                {card.pinned && (
                  <button
                    type="button"
                    title="关闭 (Esc)"
                    aria-label="关闭浮窗"
                    onClick={() => invoke("popup_dismiss")}
                    className="w-7 h-7 rounded-[8px] inline-flex items-center justify-center text-[var(--lt-ink-muted)] hover:bg-[var(--lt-control-hover)] transition-colors cursor-pointer"
                  >
                    <X className="w-3.5 h-3.5" />
                  </button>
                )}
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <PopupApp />
  </React.StrictMode>
);
