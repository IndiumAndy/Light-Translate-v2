import React, { useEffect, useRef, useState } from "react";
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
  Zap,
} from "lucide-react";
import type { ExtractionScope, ExtractionSource, PopupCardViewState } from "../shared/types";
import "../styles.css";

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

function scopeLabel(scope: ExtractionScope, source: ExtractionSource): string {
  const s =
    scope === "word"
      ? "单词"
      : scope === "sentence"
        ? "整句"
        : scope === "selection"
          ? "选区"
          : scope === "regionOcr"
            ? "截图取词"
            : "短语";
  const src =
    source === "ocr"
      ? " · OCR"
      : source === "clipboardFallback"
        ? " · 剪贴板"
        : "";
  return `${s}${src}`;
}

function PopupApp() {
  const [card, setCard] = useState<PopupCardViewState>(EMPTY_STATE);
  const [copied, setCopied] = useState(false);
  const [toast, setToast] = useState<string | null>(null);
  const containerRef = useRef<HTMLDivElement | null>(null);

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

  // Automatically sync native overlay height to fit content tightly
  useEffect(() => {
    if (!containerRef.current || !card.visible) return;
    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        const h = Math.ceil(entry.contentRect.height + 16);
        invoke("popup_resize", { height: h }).catch(() => {});
      }
    });
    observer.observe(containerRef.current);
    return () => observer.disconnect();
  }, [card.visible, card.generation]);

  if (!card.visible) {
    return <div className="w-full h-full bg-transparent" />;
  }

  // Status-only banner (e.g. permission reminder)
  if (card.statusBanner && !card.originalText) {
    return (
      <div className="p-2">
        <div
          ref={containerRef}
          className="rounded-xl px-3.5 py-2.5 backdrop-blur-2xl bg-white/92 dark:bg-zinc-900/92 border border-black/[0.08] dark:border-white/[0.12] shadow-lg text-[13px] text-zinc-800 dark:text-zinc-100 flex items-center justify-between gap-2"
        >
          <span>{card.statusBanner}</span>
          <button
            type="button"
            aria-label="关闭提示"
            onClick={() => invoke("popup_dismiss")}
            className="w-6 h-6 rounded-md flex items-center justify-center text-zinc-500 hover:bg-black/5 dark:hover:bg-white/10"
          >
            <X className="w-3.5 h-3.5" />
          </button>
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
  const showFastFallbackBadge = !card.llmTranslation && !!card.fastTranslation;
  const canExpandSentence =
    card.scope === "word" &&
    (!!card.contextSentence || card.source === "accessibility" || card.source === "ocr");

  return (
    <div className="p-2">
      <div
        ref={containerRef}
        className="rounded-2xl backdrop-blur-2xl bg-white/92 dark:bg-zinc-900/92 border border-black/[0.09] dark:border-white/[0.13] shadow-[0_12px_36px_rgba(0,0,0,0.18)] text-zinc-900 dark:text-zinc-100 overflow-hidden transition-colors"
      >
        {/* Top bar: Scope metadata + Non-activating quick controls */}
        <div className="px-3.5 pt-2.5 pb-1.5 flex items-center justify-between gap-2 border-b border-black/[0.05] dark:border-white/[0.06]">
          <div className="flex items-center gap-1.5 min-w-0">
            <span className="inline-flex items-center px-1.5 py-0.5 rounded-md text-[11px] font-medium bg-zinc-900/[0.06] dark:bg-white/[0.10] text-zinc-700 dark:text-zinc-300">
              {scopeLabel(card.scope, card.source)}
            </span>
            {card.completeness === "partial" && card.scope === "sentence" && (
              <span className="text-[11px] text-amber-700 dark:text-amber-400">
                片段
              </span>
            )}
            {card.ttftMs !== null && card.ttftMs > 0 && (
              <span className="text-[11px] tabular-nums text-zinc-500 dark:text-zinc-400">
                {card.ttftMs}ms
              </span>
            )}
            {!card.interactive && (
              <span className="text-[11px] text-zinc-500 dark:text-zinc-400">
                松开修饰键可操作
              </span>
            )}
          </div>

          {/* Interactive toolbar (28x28pt minimum desktop hit targets per HIG) */}
          <div
            className={`flex items-center gap-1 transition-opacity ${
              card.interactive ? "opacity-100" : "opacity-55 pointer-events-none"
            }`}
          >
            {canExpandSentence && (
              <button
                type="button"
                title="一键翻译所在整句"
                aria-label="翻译整句"
                onClick={() => invoke("popup_expand_sentence")}
                className="h-7 px-2 rounded-lg text-[11.5px] font-medium inline-flex items-center gap-1 bg-sky-600/12 dark:bg-sky-400/15 text-sky-700 dark:text-sky-300 hover:bg-sky-600/20 dark:hover:bg-sky-400/25 transition-colors cursor-pointer"
              >
                <TextQuote className="w-3.5 h-3.5" />
                <span>译整句</span>
              </button>
            )}

            <button
              type="button"
              title="AI 语境解析"
              aria-label="语境解释"
              onClick={() => invoke("popup_explain")}
              className="w-7 h-7 rounded-lg inline-flex items-center justify-center text-zinc-600 dark:text-zinc-300 hover:bg-black/[0.06] dark:hover:bg-white/[0.10] transition-colors cursor-pointer"
            >
              <Sparkles className="w-3.5 h-3.5" />
            </button>

            <button
              type="button"
              title="复制译文"
              aria-label="复制译文"
              onClick={handleCopy}
              className="w-7 h-7 rounded-lg inline-flex items-center justify-center text-zinc-600 dark:text-zinc-300 hover:bg-black/[0.06] dark:hover:bg-white/[0.10] transition-colors cursor-pointer"
            >
              {copied ? (
                <Check className="w-3.5 h-3.5 text-emerald-600 dark:text-emerald-400" />
              ) : (
                <Copy className="w-3.5 h-3.5" />
              )}
            </button>

            <button
              type="button"
              title="加入生词收藏本"
              aria-label="收藏词条"
              onClick={handleSave}
              className={`w-7 h-7 rounded-lg inline-flex items-center justify-center transition-colors cursor-pointer ${
                card.saved
                  ? "text-amber-600 dark:text-amber-400 bg-amber-500/12"
                  : "text-zinc-600 dark:text-zinc-300 hover:bg-black/[0.06] dark:hover:bg-white/[0.10]"
              }`}
            >
              <Bookmark className="w-3.5 h-3.5" fill={card.saved ? "currentColor" : "none"} />
            </button>

            <button
              type="button"
              title={card.pinned ? "取消固定" : "固定浮窗"}
              aria-label="固定浮窗"
              onClick={() => invoke("popup_toggle_pin")}
              className={`w-7 h-7 rounded-lg inline-flex items-center justify-center transition-colors cursor-pointer ${
                card.pinned
                  ? "text-sky-600 dark:text-sky-400 bg-sky-500/15"
                  : "text-zinc-600 dark:text-zinc-300 hover:bg-black/[0.06] dark:hover:bg-white/[0.10]"
              }`}
            >
              <Pin className="w-3.5 h-3.5" />
            </button>

            <button
              type="button"
              title="关闭 (Esc)"
              aria-label="关闭浮窗"
              onClick={() => invoke("popup_dismiss")}
              className="w-7 h-7 rounded-lg inline-flex items-center justify-center text-zinc-500 dark:text-zinc-400 hover:bg-black/[0.06] dark:hover:bg-white/[0.10] transition-colors cursor-pointer"
            >
              <X className="w-3.5 h-3.5" />
            </button>
          </div>
        </div>

        {/* Main Content Body */}
        <div className="px-3.5 py-2.5 space-y-2">
          {/* Source Text */}
          <div
            className={`select-text break-words ${
              card.scope === "word"
                ? "text-[16px] font-semibold tracking-tight text-zinc-900 dark:text-zinc-50"
                : "text-[13px] leading-[1.48] text-zinc-600 dark:text-zinc-300 line-clamp-4"
            }`}
          >
            {card.originalText}
          </div>

          {/* Tier 1: Instant Offline WordBook (< 5ms) */}
          {card.dictionary && card.dictionary.senses.length > 0 && (
            <div className="space-y-1 pt-0.5">
              {card.dictionary.senses.map((sense, idx) => (
                <div key={idx} className="flex items-baseline gap-2 text-[13px] leading-[1.45]">
                  {sense.partOfSpeech && (
                    <span className="shrink-0 font-mono text-[11.5px] px-1.5 py-0.2 rounded bg-zinc-900/[0.05] dark:bg-white/[0.08] text-zinc-600 dark:text-zinc-300">
                      {sense.partOfSpeech}
                    </span>
                  )}
                  <span className="select-text text-zinc-800 dark:text-zinc-100">
                    {sense.glosses.join("；")}
                  </span>
                </div>
              ))}
            </div>
          )}

          {/* Tier 2 & Tier 3: Fast MT + SSE Streaming LLM Translation */}
          {(primaryTranslation || card.isStreaming || card.translationError) && (
            <div
              className={`${
                card.dictionary ? "pt-1.5 border-t border-black/[0.06] dark:border-white/[0.07]" : ""
              }`}
            >
              {showFastFallbackBadge && (
                <div className="inline-flex items-center gap-1 text-[11px] text-sky-700 dark:text-sky-300 mb-0.5">
                  <Zap className="w-3 h-3" />
                  <span>极速直译</span>
                </div>
              )}

              {primaryTranslation ? (
                <div className="select-text text-[14px] leading-[1.58] font-normal text-zinc-900 dark:text-zinc-50 break-words whitespace-pre-wrap">
                  {primaryTranslation}
                  {card.isStreaming && (
                    <span className="inline-block w-1.5 h-3.5 ml-0.5 align-middle bg-sky-500 animate-pulse rounded-xs" />
                  )}
                </div>
              ) : card.isStreaming ? (
                <div className="flex items-center gap-2 text-[13px] text-zinc-500 dark:text-zinc-400 py-0.5">
                  <span className="w-2 h-2 rounded-full bg-sky-500 animate-ping" />
                  <span>正在流式译入…</span>
                </div>
              ) : null}

              {card.translationError && (
                <div className="mt-1 text-[12px] text-rose-600 dark:text-rose-400">
                  {card.translationError}
                </div>
              )}
            </div>
          )}

          {/* On-Demand Contextual AI Explanation */}
          {(card.isExplaining || card.explanation || card.explanationError) && (
            <div className="mt-2 rounded-xl px-3 py-2 bg-zinc-900/[0.04] dark:bg-white/[0.05] border border-black/[0.05] dark:border-white/[0.06] space-y-1">
              <div className="text-[11px] font-medium text-zinc-500 dark:text-zinc-400 flex items-center gap-1">
                <Sparkles className="w-3 h-3 text-sky-500" />
                <span>AI 语境解析</span>
              </div>
              {card.explanation ? (
                <div className="select-text text-[12.5px] leading-[1.55] text-zinc-800 dark:text-zinc-200 whitespace-pre-wrap">
                  {card.explanation}
                  {card.isExplaining && (
                    <span className="inline-block w-1.5 h-3 ml-0.5 align-middle bg-sky-500 animate-pulse" />
                  )}
                </div>
              ) : card.isExplaining ? (
                <div className="text-[12px] text-zinc-500 dark:text-zinc-400">正在结合上下文分析…</div>
              ) : null}
              {card.explanationError && (
                <div className="text-[12px] text-rose-600 dark:text-rose-400">
                  {card.explanationError}
                </div>
              )}
            </div>
          )}

          {toast && (
            <div className="text-[11.5px] text-sky-700 dark:text-sky-300 pt-0.5">{toast}</div>
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
