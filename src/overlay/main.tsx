import React, { useEffect, useLayoutEffect, useRef, useState } from "react";
import ReactDOM from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Check, Copy, Eye, GripVertical, Pin, X } from "lucide-react";
import type { CaptureOverlayViewState, OverlayBlockViewState } from "../shared/types";
import "../styles.css";

interface AutoFitBlockProps {
  block: OverlayBlockViewState;
  maxWidth: number;
  maxHeight: number;
  onCopyBlock: (text: string) => void;
}

function AutoFitBlock({ block, maxWidth, maxHeight, onCopyBlock }: AutoFitBlockProps) {
  const boxRef = useRef<HTMLDivElement | null>(null);
  const padX = 2.5;
  const padY = 1.5;

  const left = Math.max(0, block.rect.x - padX);
  const top = Math.max(0, block.rect.y - padY);
  const boxW = Math.min(maxWidth - left, block.rect.width + padX * 2);
  const boxH = Math.min(maxHeight - top, block.rect.height + padY * 2);

  const isSingleLine = block.lineCount <= 1 && boxH <= 44;
  const targetBasePx = Math.min(
    Math.max(10.5, block.estimatedFontPx * (isSingleLine ? 0.92 : 0.90)),
    isSingleLine ? Math.max(12, boxH * 0.78) : 34
  );

  const [fontPx, setFontPx] = useState<number>(targetBasePx);

  useLayoutEffect(() => {
    const el = boxRef.current;
    if (!el || !block.translation) {
      setFontPx(targetBasePx);
      return;
    }

    let candidate = targetBasePx;
    const minPx = 9.5;
    el.style.fontSize = `${candidate}px`;

    // Iteratively step down font size if translated Chinese overflows the original bounding box
    let guard = 0;
    while (
      candidate > minPx &&
      guard < 24 &&
      (el.scrollHeight > boxH + 3 || el.scrollWidth > boxW + 4)
    ) {
      candidate = Math.max(minPx, candidate - 0.75);
      el.style.fontSize = `${candidate}px`;
      guard += 1;
    }
    setFontPx(candidate);
  }, [block.translation, targetBasePx, boxW, boxH]);

  const isHeadingLike = block.estimatedFontPx >= 19.5 && block.lineCount <= 2;

  return (
    <>
      {/* Layer 1: Per-line tight background masks cleanly erase every underlying English stroke */}
      {block.lineRects.map((lr, idx) => {
        const lx = Math.max(0, lr.x - 3);
        const ly = Math.max(0, lr.y - 2);
        const lw = Math.min(maxWidth - lx, lr.width + 6);
        const lh = Math.min(maxHeight - ly, lr.height + 4);
        return (
          <div
            key={idx}
            style={{
              left: `${lx}px`,
              top: `${ly}px`,
              width: `${lw}px`,
              height: `${lh}px`,
              backgroundColor: block.bgColor,
            }}
            className="absolute rounded-[3px] pointer-events-none"
          />
        );
      })}

      {/* Layer 2: In-place translated Chinese typography */}
      <div
        ref={boxRef}
        title={`原文：${block.sourceText}\n（点击可单独复制此段译文）`}
        onClick={(e) => {
          e.stopPropagation();
          if (block.translation.trim()) {
            onCopyBlock(block.translation.trim());
          }
        }}
        style={{
          left: `${left}px`,
          top: `${top}px`,
          width: `${boxW}px`,
          maxHeight: `${boxH + 4}px`,
          fontSize: `${fontPx}px`,
          lineHeight: isSingleLine ? 1.2 : 1.36,
          fontWeight: isHeadingLike ? 600 : 450,
          color: block.textColor,
          backgroundColor: block.bgColor,
        }}
        className={`absolute rounded-[3px] px-[2px] overflow-hidden cursor-pointer transition-shadow hover:ring-1 ${
          block.isDarkBg ? "hover:ring-white/45" : "hover:ring-black/30"
        } ${
          isSingleLine ? "flex items-center" : ""
        }`}
      >
        <span className="break-words">{block.translation}</span>
      </div>
    </>
  );
}

function OverlayApp() {
  const [view, setView] = useState<CaptureOverlayViewState | null>(null);
  const [cropUrl, setCropUrl] = useState<string | null>(null);
  const [toggleOriginal, setToggleOriginal] = useState(false);
  const [holdOriginal, setHoldOriginal] = useState(false);
  const [copyFeedback, setCopyFeedback] = useState<string | null>(null);

  const activeCropUrlRef = useRef<string | null>(null);
  const dragLastPt = useRef<{ x: number; y: number } | null>(null);
  const copyTimerRef = useRef<number | null>(null);

  const setManagedCropUrl = (next: string | null) => {
    if (activeCropUrlRef.current) {
      URL.revokeObjectURL(activeCropUrlRef.current);
    }
    activeCropUrlRef.current = next;
    setCropUrl(next);
  };

  const loadCropImage = async () => {
    try {
      const buf = await invoke<ArrayBuffer>("get_overlay_crop_image");
      if (buf && buf.byteLength > 8) {
        const url = URL.createObjectURL(new Blob([buf], { type: "image/png" }));
        setManagedCropUrl(url);
      } else {
        setManagedCropUrl(null);
      }
    } catch {
      setManagedCropUrl(null);
    }
  };

  useEffect(() => {
    invoke<CaptureOverlayViewState>("get_overlay_state")
      .then((s) => {
        setView(s);
        if (s.visible) {
          loadCropImage();
        }
      })
      .catch(() => {});

    const unlistenState = listen<CaptureOverlayViewState>("overlay://state", (ev) => {
      setView(ev.payload);
      if (!ev.payload.visible) {
        setToggleOriginal(false);
        setHoldOriginal(false);
      }
    });

    const unlistenNewImg = listen<number>("overlay://new-image", () => {
      setToggleOriginal(false);
      setHoldOriginal(false);
      setCopyFeedback(null);
      loadCropImage();
    });

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        invoke("overlay_dismiss").catch(() => {});
      }
    };
    window.addEventListener("keydown", onKeyDown);

    return () => {
      window.removeEventListener("keydown", onKeyDown);
      if (activeCropUrlRef.current) {
        URL.revokeObjectURL(activeCropUrlRef.current);
      }
      unlistenState.then((fn) => fn());
      unlistenNewImg.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    const onPointerMove = (e: PointerEvent) => {
      if (!dragLastPt.current) return;
      const dx = e.screenX - dragLastPt.current.x;
      const dy = e.screenY - dragLastPt.current.y;
      if (Math.abs(dx) >= 1 || Math.abs(dy) >= 1) {
        dragLastPt.current = { x: e.screenX, y: e.screenY };
        invoke("overlay_move_by", { dx, dy }).catch(() => {});
      }
    };
    const onPointerUp = () => {
      dragLastPt.current = null;
      setHoldOriginal(false);
    };
    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", onPointerUp);
    return () => {
      window.removeEventListener("pointermove", onPointerMove);
      window.removeEventListener("pointerup", onPointerUp);
    };
  }, []);

  const flashCopyLabel = (label: string) => {
    setCopyFeedback(label);
    if (copyTimerRef.current) {
      window.clearTimeout(copyTimerRef.current);
    }
    copyTimerRef.current = window.setTimeout(() => {
      setCopyFeedback(null);
    }, 1400);
  };

  if (!view || !view.visible) {
    return null;
  }

  const showingOriginal = toggleOriginal || holdOriginal;
  const anyLoading = view.blocks.some((b) => b.isLoading && !b.translation);
  const allTranslatedText = view.blocks
    .map((b) => b.translation.trim())
    .filter(Boolean)
    .join("\n\n");

  const toolbarTop =
    view.toolbarPlacement === "above"
      ? Math.max(2, view.imageOffsetY - 38)
      : view.toolbarPlacement === "below"
        ? view.imageOffsetY + view.imageHeight + 5
        : view.imageOffsetY + 8;

  const toolbarLeft =
    view.toolbarPlacement === "insideTop"
      ? Math.max(6, view.imageOffsetX + 8)
      : Math.max(2, view.imageOffsetX);

  const statusText = view.error
    ? view.error
    : anyLoading
      ? "识别贴合中…"
      : view.isRefining
        ? "AI 语境精修中…"
        : showingOriginal
          ? "正在查看原文"
          : `已贴合 ${view.blocks.length} 处`;

  return (
    <div
      onMouseDown={(e) => {
        // Clicking transparent margin outside narrow selection dismisses when unpinned
        if (e.target === e.currentTarget && !view.pinned) {
          invoke("overlay_dismiss").catch(() => {});
        }
      }}
      className="relative w-screen h-screen bg-transparent select-none overflow-hidden"
    >
      {/* Floating Control Pill Toolbar */}
      <div
        style={{
          left: `${toolbarLeft}px`,
          top: `${toolbarTop}px`,
        }}
        onPointerDown={(e) => {
          if ((e.target as HTMLElement).closest("button")) return;
          if (e.button === 0) {
            dragLastPt.current = { x: e.screenX, y: e.screenY };
          }
        }}
        className="absolute z-30 h-[34px] px-1.5 rounded-full backdrop-blur-xl bg-zinc-950/92 text-zinc-100 border border-white/15 shadow-lg inline-flex items-center gap-1 text-[11.5px] cursor-grab active:cursor-grabbing max-w-[calc(100vw-8px)]"
      >
        <GripVertical className="w-3.5 h-3.5 text-zinc-400 shrink-0 ml-0.5" />

        <span className="inline-flex items-center gap-1.5 px-1 text-zinc-200 truncate max-w-[140px]">
          <span
            className={`w-1.5 h-1.5 rounded-full shrink-0 ${
              view.error
                ? "bg-rose-400"
                : anyLoading || view.isRefining
                  ? "bg-white animate-pulse motion-reduce:animate-none"
                  : "bg-emerald-400"
            }`}
          />
          <span className="truncate tabular-nums">
            {copyFeedback || statusText}
          </span>
        </span>

        <span className="w-px h-3.5 bg-white/15 mx-0.5 shrink-0" />

        <button
          type="button"
          title="点击切换原文/译文，或按住临时透视原文"
          onPointerDown={(e) => {
            e.stopPropagation();
            if (!toggleOriginal) setHoldOriginal(true);
          }}
          onPointerUp={(e) => {
            e.stopPropagation();
            setHoldOriginal(false);
          }}
          onClick={(e) => {
            e.stopPropagation();
            setHoldOriginal(false);
            setToggleOriginal((v) => !v);
          }}
          className={`h-7 px-2.5 rounded-full inline-flex items-center gap-1 whitespace-nowrap shrink-0 cursor-pointer active:scale-[0.97] transition-all ${
            showingOriginal
              ? "bg-white text-zinc-950 font-medium"
              : "hover:bg-white/12 text-zinc-200"
          }`}
        >
          <Eye className="w-3 h-3 shrink-0" />
          <span>{showingOriginal ? "译文" : "原文"}</span>
        </button>

        <button
          type="button"
          title="复制全部译文"
          disabled={!allTranslatedText}
          onClick={async (e) => {
            e.stopPropagation();
            if (!allTranslatedText) return;
            await invoke("popup_copy_text", { text: allTranslatedText }).catch(() => {});
            flashCopyLabel("已复制全部");
          }}
          className="h-7 px-2.5 rounded-full inline-flex items-center gap-1 whitespace-nowrap shrink-0 hover:bg-white/12 active:scale-[0.97] disabled:opacity-40 text-zinc-200 cursor-pointer transition-all"
        >
          {copyFeedback ? (
            <Check className="w-3 h-3 text-white shrink-0" />
          ) : (
            <Copy className="w-3 h-3 shrink-0" />
          )}
          <span>复制</span>
        </button>

        <button
          type="button"
          title={view.pinned ? "取消钉住" : "钉住在屏幕上（钉住后可任意拖动对照阅读）"}
          onClick={(e) => {
            e.stopPropagation();
            invoke("overlay_toggle_pin", { forcePinned: null }).catch(() => {});
          }}
          className={`h-7 px-2.5 rounded-full inline-flex items-center gap-1 whitespace-nowrap shrink-0 cursor-pointer active:scale-[0.97] transition-all ${
            view.pinned
              ? "bg-white text-zinc-950 font-medium"
              : "hover:bg-white/12 text-zinc-200"
          }`}
        >
          <Pin className="w-3 h-3 shrink-0" />
          <span>{view.pinned ? "已钉住" : "钉住"}</span>
        </button>

        <button
          type="button"
          title="关闭 (Esc)"
          aria-label="关闭原位翻译层"
          onClick={(e) => {
            e.stopPropagation();
            invoke("overlay_dismiss").catch(() => {});
          }}
          className="w-7 h-7 rounded-full inline-flex items-center justify-center shrink-0 hover:bg-white/15 active:scale-[0.96] text-zinc-300 hover:text-white cursor-pointer transition-all"
        >
          <X className="w-3.5 h-3.5" />
        </button>
      </div>

      {/* Exact 1:1 Captured Region Canvas + In-Place Overlaid Chinese Blocks */}
      <div
        style={{
          left: `${view.imageOffsetX}px`,
          top: `${view.imageOffsetY}px`,
          width: `${view.imageWidth}px`,
          height: `${view.imageHeight}px`,
        }}
        onPointerDown={(e) => {
          if (!view.pinned) return;
          if ((e.target as HTMLElement).closest("[data-block-interactive]")) return;
          if (e.button === 0) {
            dragLastPt.current = { x: e.screenX, y: e.screenY };
          }
        }}
        className={`absolute overflow-hidden rounded-[4px] ${
          view.pinned
            ? "ring-1 ring-zinc-900/55 dark:ring-white/65 cursor-grab active:cursor-grabbing"
            : "ring-1 ring-black/15 dark:ring-white/20"
        }`}
      >
        {cropUrl && (
          <img
            src={cropUrl}
            alt=""
            draggable={false}
            className="w-full h-full object-fill pointer-events-none select-none block"
          />
        )}

        {!showingOriginal &&
          view.blocks.map((block) => {
            if (!block.translation) {
              if (!block.isLoading) return null;
              // Shimmer skeleton over detected OCR lines while waiting ~150ms for Fast MT
              return (
                <React.Fragment key={block.id}>
                  {block.lineRects.map((lr, idx) => (
                    <div
                      key={idx}
                      style={{
                        left: `${Math.max(0, lr.x - 2)}px`,
                        top: `${Math.max(0, lr.y - 1)}px`,
                        width: `${Math.min(view.imageWidth, lr.width + 4)}px`,
                        height: `${Math.min(view.imageHeight, lr.height + 2)}px`,
                        backgroundColor: block.bgColor,
                      }}
                      className="absolute rounded-[3px] opacity-80 animate-pulse pointer-events-none"
                    />
                  ))}
                </React.Fragment>
              );
            }

            return (
              <div key={block.id} data-block-interactive="true">
                <AutoFitBlock
                  block={block}
                  maxWidth={view.imageWidth}
                  maxHeight={view.imageHeight}
                  onCopyBlock={async (txt) => {
                    await invoke("popup_copy_text", { text: txt }).catch(() => {});
                    flashCopyLabel("已复制该段");
                  }}
                />
              </div>
            );
          })}
      </div>
    </div>
  );
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <OverlayApp />
  </React.StrictMode>
);
