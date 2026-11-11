import React, { useEffect, useRef, useState } from "react";
import ReactDOM from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { CaptureActivatePayload, ScreenRect } from "../shared/types";
import "../styles.css";

function bytesToBlobUrl(buf: ArrayBuffer): string | null {
  if (!buf || buf.byteLength < 8) return null;
  const head = new Uint8Array(buf, 0, 4);
  const mime = head[0] === 0xff && head[1] === 0xd8 ? "image/jpeg" : "image/png";
  return URL.createObjectURL(new Blob([buf], { type: mime }));
}

function CaptureApp() {
  const [monitorOrigin, setMonitorOrigin] = useState({ x: 0, y: 0 });
  const [backdropUrl, setBackdropUrl] = useState<string | null>(null);
  const [dragStart, setDragStart] = useState<{ x: number; y: number } | null>(null);
  const [dragCurrent, setDragCurrent] = useState<{ x: number; y: number } | null>(null);
  const activeUrlRef = useRef<string | null>(null);

  const updateBackdropUrl = (next: string | null) => {
    if (activeUrlRef.current) {
      URL.revokeObjectURL(activeUrlRef.current);
    }
    activeUrlRef.current = next;
    setBackdropUrl(next);
  };

  useEffect(() => {
    const unlisten = listen<CaptureActivatePayload>("capture://activate", async (ev) => {
      const { monitor, hasBackdrop } = ev.payload;
      setMonitorOrigin({ x: monitor.x, y: monitor.y });
      setDragStart(null);
      setDragCurrent(null);

      if (hasBackdrop) {
        try {
          const buf = await invoke<ArrayBuffer>("get_capture_backdrop");
          updateBackdropUrl(bytesToBlobUrl(buf));
        } catch {
          updateBackdropUrl(null);
        }
      } else {
        updateBackdropUrl(null);
      }
    });

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setDragStart(null);
        setDragCurrent(null);
        updateBackdropUrl(null);
        invoke("cancel_region_capture").catch(() => {});
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      if (activeUrlRef.current) {
        URL.revokeObjectURL(activeUrlRef.current);
      }
      unlisten.then((fn) => fn());
    };
  }, []);

  const selectionRect =
    dragStart && dragCurrent
      ? {
          left: Math.min(dragStart.x, dragCurrent.x),
          top: Math.min(dragStart.y, dragCurrent.y),
          width: Math.abs(dragCurrent.x - dragStart.x),
          height: Math.abs(dragCurrent.y - dragStart.y),
        }
      : null;

  const handleMouseUp = async () => {
    if (!selectionRect || selectionRect.width < 10 || selectionRect.height < 10) {
      setDragStart(null);
      setDragCurrent(null);
      if (selectionRect && selectionRect.width < 4 && selectionRect.height < 4) {
        updateBackdropUrl(null);
        invoke("cancel_region_capture").catch(() => {});
      }
      return;
    }

    const screenRect: ScreenRect = {
      x: monitorOrigin.x + selectionRect.left,
      y: monitorOrigin.y + selectionRect.top,
      width: selectionRect.width,
      height: selectionRect.height,
    };
    setDragStart(null);
    setDragCurrent(null);
    updateBackdropUrl(null);
    await invoke("submit_region_capture", { rect: screenRect }).catch(() => {});
  };

  return (
    <div
      onContextMenu={(e) => {
        e.preventDefault();
        updateBackdropUrl(null);
        invoke("cancel_region_capture").catch(() => {});
      }}
      onMouseDown={(e) => {
        if (e.button !== 0) return;
        setDragStart({ x: e.clientX, y: e.clientY });
        setDragCurrent({ x: e.clientX, y: e.clientY });
      }}
      onMouseMove={(e) => {
        if (!dragStart) return;
        setDragCurrent({ x: e.clientX, y: e.clientY });
      }}
      onMouseUp={handleMouseUp}
      className="fixed inset-0 w-screen h-screen cursor-crosshair select-none overflow-hidden"
    >
      {/* Crisp freeze-frame screen snapshot so transient menus / video subtitles stay frozen */}
      {backdropUrl && (
        <img
          src={backdropUrl}
          alt=""
          draggable={false}
          className="fixed inset-0 w-screen h-screen object-fill pointer-events-none select-none"
        />
      )}

      {/* Dimmed backdrop when no selection box is being dragged */}
      {!selectionRect && <div className="fixed inset-0 bg-black/30 pointer-events-none" />}

      {/* Top instruction pill */}
      <div className="fixed top-6 left-1/2 -translate-x-1/2 z-20 px-4 py-2 rounded-full backdrop-blur-xl bg-zinc-950/88 text-zinc-100 text-[13px] border border-white/15 shadow-lg pointer-events-none whitespace-nowrap">
        拖拽框选英文区域，松手即原位贴合译文 · 按 Esc 或右键取消
      </div>

      {/* Spotlight cut-out selection box */}
      {selectionRect && (
        <div
          style={{
            left: `${selectionRect.left}px`,
            top: `${selectionRect.top}px`,
            width: `${selectionRect.width}px`,
            height: `${selectionRect.height}px`,
          }}
          className="fixed z-10 border-[1.5px] border-white outline outline-1 outline-black/45 bg-transparent shadow-[0_0_0_9999px_rgba(0,0,0,0.36)] pointer-events-none"
        >
          <div className="absolute -top-6 left-0 px-2 py-0.5 rounded bg-zinc-950/92 text-zinc-100 border border-white/15 text-[11px] font-mono tabular-nums whitespace-nowrap">
            {Math.round(selectionRect.width)} × {Math.round(selectionRect.height)}
          </div>
        </div>
      )}
    </div>
  );
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <CaptureApp />
  </React.StrictMode>
);
