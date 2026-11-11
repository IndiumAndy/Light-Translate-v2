import React, { useEffect, useState } from "react";
import ReactDOM from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { ScreenRect } from "../shared/types";
import "../styles.css";

function CaptureApp() {
  const [monitorOrigin, setMonitorOrigin] = useState({ x: 0, y: 0 });
  const [dragStart, setDragStart] = useState<{ x: number; y: number } | null>(null);
  const [dragCurrent, setDragCurrent] = useState<{ x: number; y: number } | null>(null);

  useEffect(() => {
    const unlisten = listen<ScreenRect>("capture://activate", (ev) => {
      setMonitorOrigin({ x: ev.payload.x, y: ev.payload.y });
      setDragStart(null);
      setDragCurrent(null);
    });

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setDragStart(null);
        setDragCurrent(null);
        invoke("cancel_region_capture").catch(() => {});
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
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
    await invoke("submit_region_capture", { rect: screenRect }).catch(() => {});
  };

  return (
    <div
      onContextMenu={(e) => {
        e.preventDefault();
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
      className="fixed inset-0 w-screen h-screen cursor-crosshair bg-black/28 select-none"
    >
      {/* Top hint pill */}
      <div className="fixed top-6 left-1/2 -translate-x-1/2 px-4 py-2 rounded-full backdrop-blur-xl bg-zinc-900/85 text-zinc-100 text-[13px] border border-white/15 shadow-lg pointer-events-none whitespace-nowrap">
        拖拽框选需要识别翻译的区域 · 按 Esc 或右键取消
      </div>

      {selectionRect && (
        <div
          style={{
            left: `${selectionRect.left}px`,
            top: `${selectionRect.top}px`,
            width: `${selectionRect.width}px`,
            height: `${selectionRect.height}px`,
          }}
          className="fixed border-2 border-sky-400 bg-sky-400/10 shadow-[0_0_0_9999px_rgba(0,0,0,0.28)] pointer-events-none"
        >
          <div className="absolute -top-7 left-0 px-2 py-0.5 rounded bg-zinc-900/90 text-white text-[11px] font-mono tabular-nums">
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
