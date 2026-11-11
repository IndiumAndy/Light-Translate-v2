import React, { useEffect, useMemo, useRef, useState } from "react";
import ReactDOM from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  ArrowRight,
  BarChart3,
  Bookmark,
  Check,
  Copy,
  Crop,
  Download,
  Languages,
  Plus,
  RotateCcw,
  Search,
  Settings2,
  Trash2,
  Upload,
  X,
} from "lucide-react";
import type {
  AppSettings,
  ConnectionTestResult,
  HeatmapDayCell,
  LlmProviderKind,
  ManualStreamEvent,
  PermissionsStatus,
  QueryMode,
  SavedEntry,
  TriggerModifier,
  UsageStatsSummary,
  WordEntry,
} from "../shared/types";
import "../styles.css";

type ActiveTab = "home" | "translate" | "wordbook" | "settings";
type ShortcutTarget = "capture" | "selection";
type HeatmapRange = "26w" | "52w";

const WEEKDAY_NAMES_ZH = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"];

function heatmapCellColorClass(level: number): string {
  switch (level) {
    case 1:
      return "bg-zinc-300 dark:bg-zinc-700";
    case 2:
      return "bg-zinc-400 dark:bg-zinc-500";
    case 3:
      return "bg-zinc-600 dark:bg-zinc-300";
    case 4:
      return "bg-zinc-900 dark:bg-zinc-100";
    default:
      return "bg-zinc-100 dark:bg-zinc-800/80";
  }
}

function buildHeatmapWeeks(cells: HeatmapDayCell[]): (HeatmapDayCell | null)[][] {
  if (cells.length === 0) return [];
  const weeks: (HeatmapDayCell | null)[][] = [];
  let current: (HeatmapDayCell | null)[] = Array(7).fill(null);

  for (const cell of cells) {
    const rowIdx = Math.max(0, Math.min(6, cell.weekday - 1));
    if (rowIdx === 0 && current.some((c) => c !== null)) {
      weeks.push(current);
      current = Array(7).fill(null);
    }
    current[rowIdx] = cell;
  }

  if (current.some((c) => c !== null)) {
    weeks.push(current);
  }
  return weeks;
}

function triggerModifierLabel(mod: TriggerModifier | undefined, isMac: boolean): string {
  switch (mod) {
    case "leftOption":
      return isMac ? "左 ⌥ Option" : "左 Alt";
    case "rightOption":
      return isMac ? "右 ⌥ Option" : "右 Alt";
    case "control":
      return isMac ? "⌃ Control" : "Ctrl";
    default:
      return isMac ? "⌥ Option" : "Alt";
  }
}

const CODE_TO_KEY_TOKEN: Record<string, string> = {
  Space: "Space",
  Backquote: "`",
  Minus: "-",
  Equal: "=",
  BracketLeft: "[",
  BracketRight: "]",
  Backslash: "\\",
  Semicolon: ";",
  Quote: "'",
  Comma: ",",
  Period: ".",
  Slash: "/",
};

function parseShortcutKeycaps(raw: string, isMac: boolean): string[] {
  const trimmed = raw.trim();
  if (!trimmed) return [];
  let ctrl = false;
  let alt = false;
  let shift = false;
  let meta = false;
  let keyLabel = "";

  for (const part of trimmed.split("+").map((s) => s.trim()).filter(Boolean)) {
    const lower = part.toLowerCase();
    if (lower === "control" || lower === "ctrl" || lower === "⌃") ctrl = true;
    else if (lower === "alt" || lower === "option" || lower === "opt" || lower === "⌥") alt = true;
    else if (lower === "shift" || lower === "⇧") shift = true;
    else if (
      lower === "command" ||
      lower === "cmd" ||
      lower === "meta" ||
      lower === "super" ||
      lower === "win" ||
      lower === "⌘"
    )
      meta = true;
    else if (lower === "space") keyLabel = "Space";
    else if (lower === "grave" || lower === "backquote") keyLabel = "`";
    else if (lower === "minus") keyLabel = "-";
    else if (lower === "equal") keyLabel = "=";
    else if (lower === "bracketleft") keyLabel = "[";
    else if (lower === "bracketright") keyLabel = "]";
    else if (lower === "backslash") keyLabel = "\\";
    else if (lower === "semicolon") keyLabel = ";";
    else if (lower === "quote") keyLabel = "'";
    else if (lower === "comma") keyLabel = ",";
    else if (lower === "period") keyLabel = ".";
    else if (lower === "slash") keyLabel = "/";
    else keyLabel = part.toUpperCase();
  }

  if (!keyLabel) return [];
  const caps: string[] = [];
  if (isMac) {
    if (ctrl) caps.push("⌃");
    if (alt) caps.push("⌥");
    if (shift) caps.push("⇧");
    if (meta) caps.push("⌘");
  } else {
    if (ctrl) caps.push("Ctrl");
    if (alt) caps.push("Alt");
    if (shift) caps.push("Shift");
    if (meta) caps.push("Win");
  }
  caps.push(keyLabel);
  return caps;
}

function formatShortcutInline(raw: string, isMac: boolean): string {
  const caps = parseShortcutKeycaps(raw, isMac);
  if (caps.length === 0) return "";
  return isMac ? caps.join("") : caps.join("+");
}

function extractShortcutFromEvent(
  e: KeyboardEvent,
  isMac: boolean
): { canonical: string | null; previewMods: string[]; error?: string } {
  const previewMods: string[] = [];
  if (isMac) {
    if (e.ctrlKey) previewMods.push("⌃");
    if (e.altKey) previewMods.push("⌥");
    if (e.shiftKey) previewMods.push("⇧");
    if (e.metaKey) previewMods.push("⌘");
  } else {
    if (e.ctrlKey) previewMods.push("Ctrl");
    if (e.altKey) previewMods.push("Alt");
    if (e.shiftKey) previewMods.push("Shift");
    if (e.metaKey) previewMods.push("Win");
  }

  const code = e.code;
  if (
    code === "MetaLeft" ||
    code === "MetaRight" ||
    code === "ControlLeft" ||
    code === "ControlRight" ||
    code === "AltLeft" ||
    code === "AltRight" ||
    code === "ShiftLeft" ||
    code === "ShiftRight"
  ) {
    return { canonical: null, previewMods };
  }

  let keyToken: string | null = null;
  if (code.startsWith("Key") && code.length === 4) {
    keyToken = code.slice(3).toUpperCase();
  } else if (code.startsWith("Digit") && code.length === 6) {
    keyToken = code.slice(5);
  } else if (/^F([1-9]|1[0-2])$/.test(code)) {
    keyToken = code;
  } else if (CODE_TO_KEY_TOKEN[code]) {
    keyToken = CODE_TO_KEY_TOKEN[code];
  }

  if (!keyToken) {
    return {
      canonical: null,
      previewMods,
      error: "支持字母、数字、F1–F12、空格及常用标点键",
    };
  }

  const isFnKey = /^F([1-9]|1[0-2])$/.test(keyToken);
  if (!e.ctrlKey && !e.altKey && !e.metaKey && !isFnKey) {
    return {
      canonical: null,
      previewMods,
      error: isMac ? "请搭配 ⌘ / ⌃ / ⌥ 修饰键，或直接使用 F1–F12" : "请搭配 Ctrl / Alt / Win 修饰键，或直接使用 F1–F12",
    };
  }

  const parts: string[] = [];
  if (e.ctrlKey) parts.push("Control");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  if (e.metaKey) parts.push("Command");
  parts.push(keyToken);
  return { canonical: parts.join("+"), previewMods };
}

function Keycap({ children }: { children: React.ReactNode }) {
  return (
    <kbd className="inline-flex items-center justify-center min-w-5 h-5 px-1.5 rounded border border-zinc-200/90 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-800 text-xs font-sans text-zinc-700 dark:text-zinc-300 tabular-nums">
      {children}
    </kbd>
  );
}

function MainApp() {
  const [tab, setTab] = useState<ActiveTab>("home");
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [permissions, setPermissions] = useState<PermissionsStatus | null>(null);
  const [keyStatus, setKeyStatus] = useState<Record<string, boolean>>({});
  const [entries, setEntries] = useState<SavedEntry[]>([]);
  const [usageStats, setUsageStats] = useState<UsageStatsSummary | null>(null);
  const [heatmapRange, setHeatmapRange] = useState<HeatmapRange>("26w");
  const [hoveredDay, setHoveredDay] = useState<HeatmapDayCell | null>(null);
  const [pinnedDay, setPinnedDay] = useState<HeatmapDayCell | null>(null);

  // Manual Translate state
  const [inputText, setInputText] = useState("");
  const [manualMode, setManualMode] = useState<QueryMode>("selection");
  const [manualDict, setManualDict] = useState<WordEntry | null>(null);
  const [manualFast, setManualFast] = useState<string | null>(null);
  const [manualLlm, setManualLlm] = useState("");
  const [manualStreaming, setManualStreaming] = useState(false);
  const [manualTtft, setManualTtft] = useState<number | null>(null);
  const [manualError, setManualError] = useState<string | null>(null);
  const [manualCopied, setManualCopied] = useState(false);
  const activeReqId = useRef<string>("");

  // Wordbook state
  const [searchQuery, setSearchQuery] = useState("");
  const [wordbookNotice, setWordbookNotice] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);

  // Settings state
  const [apiKeyDraft, setApiKeyDraft] = useState("");
  const [testResult, setTestResult] = useState<ConnectionTestResult | null>(null);
  const [testingConn, setTestingConn] = useState(false);
  const [newExcludedApp, setNewExcludedApp] = useState("");
  const [settingsSavedBanner, setSettingsSavedBanner] = useState(false);
  const [recordingTarget, setRecordingTarget] = useState<ShortcutTarget | null>(null);
  const [recordingModsPreview, setRecordingModsPreview] = useState<string[]>([]);
  const [shortcutHint, setShortcutHint] = useState<string | null>(null);

  const isMac =
    permissions?.platform === "macos" ||
    (typeof navigator !== "undefined" && navigator.userAgent.includes("Mac"));
  const defaultCaptureShortcut = isMac ? "Control+Command+S" : "Alt+Shift+S";
  const defaultSelectionShortcut = isMac ? "Alt+Command+T" : "Alt+Shift+T";

  const refreshAll = async () => {
    const [s, p, ks, list, stats] = await Promise.all([
      invoke<AppSettings>("get_settings"),
      invoke<PermissionsStatus>("get_permissions_status", { prompt: false }),
      invoke<Record<string, boolean>>("get_api_key_status"),
      invoke<SavedEntry[]>("list_saved_entries"),
      invoke<UsageStatsSummary>("get_usage_stats"),
    ]);
    setSettings(s);
    setPermissions(p);
    setKeyStatus(ks);
    setEntries(list);
    setUsageStats(stats);
  };

  useEffect(() => {
    refreshAll().catch(() => {});

    const unlistenNav = listen<string>("main://navigate", (ev) => {
      if (
        ev.payload === "home" ||
        ev.payload === "translate" ||
        ev.payload === "wordbook" ||
        ev.payload === "settings"
      ) {
        setTab(ev.payload);
      }
      refreshAll().catch(() => {});
    });

    const unlistenSettings = listen<AppSettings>("settings://changed", (ev) => {
      setSettings(ev.payload);
    });

    const unlistenWordbook = listen("wordbook://changed", () => {
      invoke<SavedEntry[]>("list_saved_entries")
        .then(setEntries)
        .catch(() => {});
    });

    const unlistenStats = listen<UsageStatsSummary>("stats://updated", (ev) => {
      setUsageStats(ev.payload);
      setPinnedDay((prev) =>
        prev ? ev.payload.heatmap.find((c) => c.date === prev.date) ?? prev : null
      );
    });

    const unlistenManual = listen<ManualStreamEvent>("manual://progress", (ev) => {
      const p = ev.payload;
      if (p.requestId !== activeReqId.current) return;
      setManualDict(p.dictionary);
      setManualFast(p.fastTranslation);
      setManualLlm(p.llmTranslation);
      setManualStreaming(!p.done);
      if (p.ttftMs !== null) setManualTtft(p.ttftMs);
      setManualError(p.error);
    });

    const onFocus = () => {
      invoke<PermissionsStatus>("get_permissions_status", { prompt: false })
        .then(setPermissions)
        .catch(() => {});
      invoke<UsageStatsSummary>("get_usage_stats")
        .then(setUsageStats)
        .catch(() => {});
    };
    window.addEventListener("focus", onFocus);

    return () => {
      window.removeEventListener("focus", onFocus);
      unlistenNav.then((f) => f());
      unlistenSettings.then((f) => f());
      unlistenWordbook.then((f) => f());
      unlistenStats.then((f) => f());
      unlistenManual.then((f) => f());
    };
  }, []);

  useEffect(() => {
    if (tab !== "settings") {
      setRecordingTarget(null);
      return;
    }
    const timer = window.setInterval(() => {
      invoke<PermissionsStatus>("get_permissions_status", { prompt: false })
        .then(setPermissions)
        .catch(() => {});
    }, 2500);
    return () => window.clearInterval(timer);
  }, [tab]);

  useEffect(() => {
    invoke("set_shortcut_recording", { active: recordingTarget !== null }).catch(() => {});
    if (!recordingTarget || !settings) {
      setRecordingModsPreview([]);
      return;
    }

    const onKeyDown = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();

      if (e.code === "Escape") {
        setRecordingTarget(null);
        setRecordingModsPreview([]);
        setShortcutHint(null);
        return;
      }

      if (
        (e.code === "Backspace" || e.code === "Delete") &&
        !e.metaKey &&
        !e.ctrlKey &&
        !e.altKey &&
        !e.shiftKey
      ) {
        const next: AppSettings =
          recordingTarget === "capture"
            ? { ...settings, captureShortcut: "" }
            : { ...settings, selectionShortcut: "" };
        setRecordingTarget(null);
        setRecordingModsPreview([]);
        setShortcutHint(null);
        persistSettings(next).catch(() => {});
        return;
      }

      const parsed = extractShortcutFromEvent(e, isMac);
      setRecordingModsPreview(parsed.previewMods);
      if (parsed.error) {
        setShortcutHint(parsed.error);
        return;
      }
      if (!parsed.canonical) {
        return;
      }

      const otherBound =
        recordingTarget === "capture" ? settings.selectionShortcut : settings.captureShortcut;
      if (
        otherBound &&
        formatShortcutInline(otherBound, isMac) === formatShortcutInline(parsed.canonical, isMac)
      ) {
        setShortcutHint(
          recordingTarget === "capture"
            ? "该快捷键已用于「划词翻译」，请换一个组合"
            : "该快捷键已用于「截屏翻译」，请换一个组合"
        );
        return;
      }

      const next: AppSettings =
        recordingTarget === "capture"
          ? { ...settings, captureShortcut: parsed.canonical }
          : { ...settings, selectionShortcut: parsed.canonical };
      setRecordingTarget(null);
      setRecordingModsPreview([]);
      setShortcutHint(null);
      persistSettings(next).catch(() => {});
    };

    const onKeyUp = (e: KeyboardEvent) => {
      const parsed = extractShortcutFromEvent(e, isMac);
      setRecordingModsPreview(parsed.previewMods);
    };

    const onBlur = () => {
      setRecordingTarget(null);
      setRecordingModsPreview([]);
    };

    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("keyup", onKeyUp, true);
    window.addEventListener("blur", onBlur);

    return () => {
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("keyup", onKeyUp, true);
      window.removeEventListener("blur", onBlur);
      invoke("set_shortcut_recording", { active: false }).catch(() => {});
    };
  }, [recordingTarget, settings, isMac]);

  const runManualTranslate = async () => {
    const trimmed = inputText.trim();
    if (!trimmed) return;
    const reqId = `${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
    activeReqId.current = reqId;
    setManualDict(null);
    setManualFast(null);
    setManualLlm("");
    setManualTtft(null);
    setManualError(null);
    setManualStreaming(true);

    try {
      await invoke("manual_translate_stream", {
        requestId: reqId,
        text: trimmed,
        mode: manualMode,
      });
    } catch (err) {
      setManualStreaming(false);
      setManualError(String(err));
    }
  };

  const filteredEntries = useMemo(() => {
    const q = searchQuery.trim().toLowerCase();
    if (!q) return entries;
    return entries.filter(
      (e) =>
        e.source.toLowerCase().includes(q) ||
        e.translation.toLowerCase().includes(q) ||
        e.context.toLowerCase().includes(q)
    );
  }, [entries, searchQuery]);

  const persistSettings = async (next: AppSettings) => {
    setSettings(next);
    const saved = await invoke<AppSettings>("save_settings", { settings: next });
    setSettings(saved);
    setSettingsSavedBanner(true);
    setTimeout(() => setSettingsSavedBanner(false), 1400);
  };

  const handleImportFile = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    try {
      const content = await file.text();
      const count = await invoke<number>("import_saved_entries", { jsonContent: content });
      const list = await invoke<SavedEntry[]>("list_saved_entries");
      setEntries(list);
      setWordbookNotice(`已导入 ${count} 条词条`);
    } catch (err) {
      setWordbookNotice(`导入失败：${String(err)}`);
    } finally {
      e.target.value = "";
    }
  };

  const handleExportJson = async () => {
    try {
      const json = await invoke<string>("export_saved_entries");
      const blob = new Blob([json], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `LightTranslate-Wordbook-${new Date().toISOString().slice(0, 10)}.json`;
      a.click();
      URL.revokeObjectURL(url);
    } catch (err) {
      setWordbookNotice(String(err));
    }
  };

  const activeEndpoint = settings
    ? settings.activeProvider === "gemini"
      ? settings.gemini
      : settings.activeProvider === "deepseek"
        ? settings.deepseek
        : settings.openaiCompatible
    : null;

  const updateActiveEndpoint = (patch: Partial<{ baseUrl: string; model: string }>) => {
    if (!settings || !activeEndpoint) return;
    const updatedEndpoint = { ...activeEndpoint, ...patch };
    const next: AppSettings =
      settings.activeProvider === "gemini"
        ? { ...settings, gemini: updatedEndpoint }
        : settings.activeProvider === "deepseek"
          ? { ...settings, deepseek: updatedEndpoint }
          : { ...settings, openaiCompatible: updatedEndpoint };
    persistSettings(next).catch(() => {});
  };

  const handleSaveApiKey = async () => {
    if (!settings) return;
    await invoke("set_api_key", {
      provider: settings.activeProvider,
      apiKey: apiKeyDraft,
    });
    setApiKeyDraft("");
    const ks = await invoke<Record<string, boolean>>("get_api_key_status");
    setKeyStatus(ks);
  };

  const handleTestConnection = async () => {
    if (!settings || !activeEndpoint) return;
    setTestingConn(true);
    setTestResult(null);
    try {
      const res = await invoke<ConnectionTestResult>("test_provider_connection", {
        provider: settings.activeProvider,
        baseUrl: activeEndpoint.baseUrl,
        model: activeEndpoint.model,
        apiKeyOverride: apiKeyDraft.trim() ? apiKeyDraft.trim() : null,
      });
      setTestResult(res);
    } catch (err) {
      setTestResult({
        ok: false,
        ttftMs: 0,
        totalMs: 0,
        sampleOutput: "",
        error: String(err),
      });
    } finally {
      setTestingConn(false);
    }
  };

  const needsPermWarning =
    permissions &&
    permissions.platform === "macos" &&
    (!permissions.accessibility || !permissions.screenRecording);

  const visibleHeatmapWeeks = useMemo(() => {
    const all = buildHeatmapWeeks(usageStats?.heatmap ?? []);
    return heatmapRange === "26w" ? all.slice(-26) : all;
  }, [usageStats, heatmapRange]);

  const heatmapMonthMarkers = useMemo(() => {
    const markers: { colIdx: number; label: string }[] = [];
    const minColGap = heatmapRange === "26w" ? 2 : 3;

    visibleHeatmapWeeks.forEach((week, colIdx) => {
      const firstOfMonth = week.find((c) => c?.date.endsWith("-01"));
      if (firstOfMonth) {
        const monthNum = parseInt(firstOfMonth.date.slice(5, 7), 10);
        if (!Number.isNaN(monthNum)) {
          if (markers.length > 0 && colIdx - markers[markers.length - 1].colIdx < minColGap) {
            if (markers[markers.length - 1].colIdx === 0) {
              markers.pop();
            } else {
              return;
            }
          }
          markers.push({ colIdx, label: `${monthNum}月` });
        }
      } else if (colIdx === 0) {
        const firstNonNull = week.find((c) => c !== null);
        if (firstNonNull) {
          const monthNum = parseInt(firstNonNull.date.slice(5, 7), 10);
          if (!Number.isNaN(monthNum)) {
            markers.push({ colIdx: 0, label: `${monthNum}月` });
          }
        }
      }
    });

    return markers;
  }, [visibleHeatmapWeeks, heatmapRange]);

  const todayCell =
    usageStats && usageStats.heatmap.length > 0
      ? usageStats.heatmap[usageStats.heatmap.length - 1]
      : null;
  const inspectedDay = hoveredDay ?? pinnedDay ?? todayCell;

  const weekMetaLabel = useMemo(() => {
    if (!usageStats) return "—";
    if (usageStats.lastWeekWords > 0) {
      const pct = Math.round(
        ((usageStats.thisWeekWords - usageStats.lastWeekWords) / usageStats.lastWeekWords) * 100
      );
      const sign = pct > 0 ? `+${pct}%` : `${pct}%`;
      return `较前 7 日 ${sign}`;
    }
    return `活跃 ${usageStats.activeDays} 天`;
  }, [usageStats]);

  const navItems: { id: ActiveTab; label: string; icon: React.ComponentType<{ className?: string; strokeWidth?: number }>; badge?: number }[] = [
    { id: "home", label: "概览", icon: BarChart3 },
    { id: "translate", label: "翻译", icon: Languages },
    { id: "wordbook", label: "生词本", icon: Bookmark, badge: entries.length > 0 ? entries.length : undefined },
    { id: "settings", label: "设置", icon: Settings2 },
  ];

  const captureCaps = parseShortcutKeycaps(
    settings?.captureShortcut ?? defaultCaptureShortcut,
    isMac
  );
  const selectionCaps = parseShortcutKeycaps(
    settings?.selectionShortcut ?? defaultSelectionShortcut,
    isMac
  );

  return (
    <div className="h-screen w-screen flex bg-zinc-50 dark:bg-zinc-950 text-zinc-900 dark:text-zinc-100 text-[13px] antialiased">
      {/* Left Navigation Sidebar */}
      <aside className="w-52 shrink-0 border-r border-zinc-200/80 dark:border-zinc-800/90 bg-zinc-100/70 dark:bg-zinc-900/60 flex flex-col justify-between p-3">
        <div className="space-y-4">
          <div className="px-2.5 pt-1 flex items-center justify-between">
            <span className="text-sm font-semibold tracking-[-0.01em] text-zinc-950 dark:text-zinc-50">
              Light Translate
            </span>
            {settings && (
              <button
                type="button"
                aria-label={settings.enabled ? "全局取词已开启，点击暂停" : "全局取词已暂停，点击开启"}
                title={settings.enabled ? "全局取词已开启" : "全局取词已暂停"}
                onClick={() => persistSettings({ ...settings, enabled: !settings.enabled })}
                className="w-6 h-6 -mr-1 rounded-md inline-flex items-center justify-center hover:bg-black/[0.05] dark:hover:bg-white/[0.06] cursor-pointer transition-colors"
              >
                <span
                  className={`w-2 h-2 rounded-full transition-colors ${
                    settings.enabled ? "bg-emerald-500" : "bg-zinc-400"
                  }`}
                />
              </button>
            )}
          </div>

          <nav className="space-y-1">
            {navItems.map((item) => {
              const Icon = item.icon;
              const active = tab === item.id;
              return (
                <button
                  key={item.id}
                  type="button"
                  onClick={() => setTab(item.id)}
                  className={`w-full h-9 px-3 rounded-lg flex items-center justify-between font-medium transition-colors cursor-pointer ${
                    active
                      ? "bg-white dark:bg-zinc-800 text-zinc-950 dark:text-zinc-50 shadow-2xs ring-1 ring-black/[0.05] dark:ring-white/[0.06]"
                      : "text-zinc-600 dark:text-zinc-400 hover:text-zinc-900 dark:hover:text-zinc-100 hover:bg-black/[0.04] dark:hover:bg-white/[0.05]"
                  }`}
                >
                  <span className="flex items-center gap-2.5">
                    <Icon className="w-4 h-4 shrink-0" strokeWidth={1.75} />
                    <span>{item.label}</span>
                  </span>
                  {item.badge !== undefined && (
                    <span className="text-xs tabular-nums px-1.5 py-0.5 leading-none rounded-md bg-zinc-200/80 dark:bg-zinc-700 text-zinc-700 dark:text-zinc-200">
                      {item.badge}
                    </span>
                  )}
                </button>
              );
            })}
          </nav>
        </div>

        <div className="space-y-2">
          <button
            type="button"
            onClick={() => invoke("start_region_capture")}
            className="w-full h-9 px-3 rounded-lg border border-zinc-200/90 dark:border-zinc-700/80 bg-white dark:bg-zinc-800/80 hover:bg-zinc-50 dark:hover:bg-zinc-800 flex items-center justify-between gap-2 text-[13px] font-medium cursor-pointer transition-colors whitespace-nowrap"
          >
            <span className="inline-flex items-center gap-2 shrink-0">
              <Crop className="w-3.5 h-3.5 shrink-0 text-zinc-600 dark:text-zinc-300" strokeWidth={1.75} />
              <span>截屏翻译</span>
            </span>
            {settings?.captureShortcut && formatShortcutInline(settings.captureShortcut, isMac) && (
              <kbd className="px-1.5 py-0.5 rounded border border-zinc-200 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-900 text-xs font-sans text-zinc-600 dark:text-zinc-300 shrink-0 tabular-nums leading-none">
                {formatShortcutInline(settings.captureShortcut, isMac)}
              </kbd>
            )}
          </button>

          {needsPermWarning && (
            <button
              type="button"
              onClick={() => setTab("settings")}
              className="w-full text-left px-3 py-2 rounded-lg bg-amber-500/10 border border-amber-500/25 text-amber-800 dark:text-amber-300 text-xs leading-snug cursor-pointer transition-colors hover:bg-amber-500/15"
            >
              系统权限未完成授权，前往开启 →
            </button>
          )}
        </div>
      </aside>

      {/* Right Content Workspace */}
      <main className="flex-1 overflow-y-auto px-8 py-6">
        {tab === "home" && (
          <div className="max-w-3xl mx-auto space-y-5">
            <div className="flex items-center justify-between">
              <h1 className="text-lg font-semibold tracking-[-0.02em] text-zinc-950 dark:text-zinc-50">
                概览
              </h1>
            </div>

            {/* 4 Stat Cards */}
            <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
              <div className="rounded-xl border border-zinc-200/80 dark:border-zinc-800 bg-white dark:bg-zinc-900 p-4">
                <div className="text-xs text-zinc-500 dark:text-zinc-400">今日词数</div>
                <div className="mt-2 text-[28px] leading-none font-medium tracking-[-0.02em] tabular-nums text-zinc-950 dark:text-zinc-50">
                  {(usageStats?.todayWords ?? 0).toLocaleString()}
                </div>
                <div className="mt-2 text-xs text-zinc-500 dark:text-zinc-400 tabular-nums">
                  {usageStats?.todayQueries ?? 0} 次查询
                </div>
              </div>

              <div className="rounded-xl border border-zinc-200/80 dark:border-zinc-800 bg-white dark:bg-zinc-900 p-4">
                <div className="text-xs text-zinc-500 dark:text-zinc-400">连续天数</div>
                <div className="mt-2 text-[28px] leading-none font-medium tracking-[-0.02em] tabular-nums text-zinc-950 dark:text-zinc-50">
                  {usageStats?.currentStreak ?? 0}
                </div>
                <div className="mt-2 text-xs text-zinc-500 dark:text-zinc-400 tabular-nums">
                  最长 {usageStats?.longestStreak ?? 0} 天
                </div>
              </div>

              <div className="rounded-xl border border-zinc-200/80 dark:border-zinc-800 bg-white dark:bg-zinc-900 p-4">
                <div className="text-xs text-zinc-500 dark:text-zinc-400">近 7 日</div>
                <div className="mt-2 text-[28px] leading-none font-medium tracking-[-0.02em] tabular-nums text-zinc-950 dark:text-zinc-50">
                  {(usageStats?.thisWeekWords ?? 0).toLocaleString()}
                </div>
                <div className="mt-2 text-xs text-zinc-500 dark:text-zinc-400 tabular-nums">
                  {weekMetaLabel}
                </div>
              </div>

              <div className="rounded-xl border border-zinc-200/80 dark:border-zinc-800 bg-white dark:bg-zinc-900 p-4">
                <div className="text-xs text-zinc-500 dark:text-zinc-400">累计词数</div>
                <div className="mt-2 text-[28px] leading-none font-medium tracking-[-0.02em] tabular-nums text-zinc-950 dark:text-zinc-50">
                  {(usageStats?.totalWords ?? 0).toLocaleString()}
                </div>
                <div className="mt-2 text-xs text-zinc-500 dark:text-zinc-400 tabular-nums">
                  {(usageStats?.totalQueries ?? 0).toLocaleString()} 次查询
                </div>
              </div>
            </div>

            {/* Daily Reading Heatmap */}
            <section className="rounded-xl border border-zinc-200/80 dark:border-zinc-800 bg-white dark:bg-zinc-900 p-4 space-y-3">
              <div className="flex items-center justify-between gap-3">
                <h2 className="text-sm font-medium text-zinc-950 dark:text-zinc-50">每日记录</h2>
                <div className="inline-flex shrink-0 rounded-lg p-0.5 bg-zinc-100 dark:bg-zinc-800">
                  <button
                    type="button"
                    onClick={() => setHeatmapRange("26w")}
                    className={`px-2.5 py-1 rounded-md text-xs font-medium cursor-pointer transition-colors ${
                      heatmapRange === "26w"
                        ? "bg-white dark:bg-zinc-900 text-zinc-900 dark:text-zinc-100 shadow-2xs"
                        : "text-zinc-600 dark:text-zinc-400 hover:text-zinc-900 dark:hover:text-zinc-100"
                    }`}
                  >
                    近半年
                  </button>
                  <button
                    type="button"
                    onClick={() => setHeatmapRange("52w")}
                    className={`px-2.5 py-1 rounded-md text-xs font-medium cursor-pointer transition-colors ${
                      heatmapRange === "52w"
                        ? "bg-white dark:bg-zinc-900 text-zinc-900 dark:text-zinc-100 shadow-2xs"
                        : "text-zinc-600 dark:text-zinc-400 hover:text-zinc-900 dark:hover:text-zinc-100"
                    }`}
                  >
                    全年
                  </button>
                </div>
              </div>

              {visibleHeatmapWeeks.length > 0 ? (
                <div className="flex items-stretch gap-2 select-none">
                  {/* Weekday Y-Axis */}
                  <div className="w-4 shrink-0 flex flex-col pt-5" aria-hidden="true">
                    <div
                      className={`flex-1 grid grid-rows-7 ${
                        heatmapRange === "26w" ? "gap-1" : "gap-0.5"
                      } text-xs text-zinc-400 dark:text-zinc-500`}
                    >
                      <span className="flex items-center leading-none">一</span>
                      <span />
                      <span className="flex items-center leading-none">三</span>
                      <span />
                      <span className="flex items-center leading-none">五</span>
                      <span />
                      <span className="flex items-center leading-none">日</span>
                    </div>
                  </div>

                  {/* Month Header + Week Columns */}
                  <div className="flex-1 min-w-0" onMouseLeave={() => setHoveredDay(null)}>
                    <div className="relative h-4 mb-1" aria-hidden="true">
                      {heatmapMonthMarkers.map((m) => (
                        <span
                          key={`${m.colIdx}-${m.label}`}
                          style={{
                            left: `${(m.colIdx / Math.max(visibleHeatmapWeeks.length, 1)) * 100}%`,
                          }}
                          className="absolute top-0 text-xs leading-none text-zinc-400 dark:text-zinc-500 whitespace-nowrap"
                        >
                          {m.label}
                        </span>
                      ))}
                    </div>

                    <div
                      className={`grid ${heatmapRange === "26w" ? "gap-1" : "gap-0.5"}`}
                      style={{
                        gridTemplateColumns: `repeat(${Math.max(visibleHeatmapWeeks.length, 1)}, minmax(0, 1fr))`,
                      }}
                    >
                      {visibleHeatmapWeeks.map((week, colIdx) => (
                        <div
                          key={colIdx}
                          className={`grid grid-rows-7 ${
                            heatmapRange === "26w" ? "gap-1" : "gap-0.5"
                          }`}
                        >
                          {week.map((cell, rowIdx) => {
                            if (!cell) {
                              return (
                                <div
                                  key={rowIdx}
                                  className="aspect-square w-full rounded-xs opacity-0 pointer-events-none"
                                />
                              );
                            }
                            const isToday = cell.date === usageStats?.todayDate;
                            const isPinned = pinnedDay?.date === cell.date;
                            const weekdayText =
                              WEEKDAY_NAMES_ZH[Math.max(0, Math.min(6, cell.weekday - 1))];
                            return (
                              <button
                                key={cell.date}
                                type="button"
                                aria-label={`${cell.date} ${weekdayText}：${cell.wordCount} 词，${cell.queryCount} 次查询`}
                                title={`${cell.date} ${weekdayText} · ${cell.wordCount.toLocaleString()} 词`}
                                onMouseEnter={() => setHoveredDay(cell)}
                                onFocus={() => setHoveredDay(cell)}
                                onBlur={() => setHoveredDay(null)}
                                onClick={() =>
                                  setPinnedDay((prev) =>
                                    prev?.date === cell.date ? null : cell
                                  )
                                }
                                className={`aspect-square w-full rounded-xs cursor-pointer transition-colors hover:ring-1 hover:ring-zinc-900/50 dark:hover:ring-zinc-100/60 ${heatmapCellColorClass(
                                  cell.level
                                )} ${
                                  isPinned
                                    ? "ring-2 ring-zinc-950 dark:ring-white ring-offset-1 ring-offset-white dark:ring-offset-zinc-900 z-10"
                                    : isToday
                                      ? "ring-1 ring-zinc-900/50 dark:ring-zinc-100/60"
                                      : ""
                                }`}
                              />
                            );
                          })}
                        </div>
                      ))}
                    </div>
                  </div>
                </div>
              ) : (
                <div className="h-32 flex items-center justify-center text-xs text-zinc-500 dark:text-zinc-400">
                  暂无记录
                </div>
              )}

              {/* Heatmap Readout & Intensity Legend */}
              <div className="pt-3 border-t border-zinc-100 dark:border-zinc-800/80 flex flex-wrap items-center justify-between gap-2 text-xs">
                <div className="flex items-center gap-2 min-w-0 tabular-nums text-zinc-500 dark:text-zinc-400">
                  {inspectedDay ? (
                    <>
                      <span className="font-medium text-zinc-900 dark:text-zinc-100">
                        {inspectedDay.date}{" "}
                        {WEEKDAY_NAMES_ZH[Math.max(0, Math.min(6, inspectedDay.weekday - 1))]}
                      </span>
                      <span className="text-zinc-300 dark:text-zinc-700">·</span>
                      <span>
                        <strong className="font-medium text-zinc-900 dark:text-zinc-100">
                          {inspectedDay.wordCount.toLocaleString()}
                        </strong>{" "}
                        词
                      </span>
                      <span className="text-zinc-300 dark:text-zinc-700">·</span>
                      <span>{inspectedDay.queryCount.toLocaleString()} 次查询</span>
                      {pinnedDay && !hoveredDay && (
                        <button
                          type="button"
                          onClick={() => setPinnedDay(null)}
                          className="ml-1 text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100 underline underline-offset-2 cursor-pointer transition-colors"
                        >
                          取消固定
                        </button>
                      )}
                    </>
                  ) : (
                    <span>悬停方格查看单日记录</span>
                  )}
                </div>

                <div className="flex items-center gap-1 text-xs text-zinc-400 dark:text-zinc-500 shrink-0 select-none">
                  <span>少</span>
                  {[0, 1, 2, 3, 4].map((lv) => (
                    <span
                      key={lv}
                      className={`w-2.5 h-2.5 rounded-xs ${heatmapCellColorClass(lv)}`}
                    />
                  ))}
                  <span>多</span>
                </div>
              </div>
            </section>

            {/* Bottom 2-Column Shortcuts & Recent Wordbook */}
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
              {/* Shortcuts Summary */}
              <section className="rounded-xl border border-zinc-200/80 dark:border-zinc-800 bg-white dark:bg-zinc-900 p-4 space-y-3">
                <div className="flex items-center justify-between">
                  <h2 className="text-sm font-medium text-zinc-950 dark:text-zinc-50">快捷键</h2>
                  <button
                    type="button"
                    onClick={() => setTab("settings")}
                    className="text-xs text-zinc-500 dark:text-zinc-400 hover:text-zinc-900 dark:hover:text-zinc-100 inline-flex items-center gap-0.5 cursor-pointer transition-colors"
                  >
                    <span>修改</span>
                    <ArrowRight className="w-3 h-3" />
                  </button>
                </div>

                <div className="divide-y divide-zinc-100 dark:divide-zinc-800/80 text-[13px]">
                  <div className="py-2 first:pt-0 flex items-center justify-between gap-3">
                    <span className="text-zinc-600 dark:text-zinc-400">悬停取词</span>
                    <Keycap>{triggerModifierLabel(settings?.triggerModifier, isMac)}</Keycap>
                  </div>
                  <div className="py-2 flex items-center justify-between gap-3">
                    <span className="text-zinc-600 dark:text-zinc-400">截屏翻译</span>
                    {captureCaps.length > 0 ? (
                      <span className="inline-flex items-center gap-1">
                        {captureCaps.map((c, idx) => (
                          <Keycap key={idx}>{c}</Keycap>
                        ))}
                      </span>
                    ) : (
                      <span className="text-xs text-zinc-400">未设置</span>
                    )}
                  </div>
                  <div className="py-2 last:pb-0 flex items-center justify-between gap-3">
                    <span className="text-zinc-600 dark:text-zinc-400">划词翻译</span>
                    {selectionCaps.length > 0 ? (
                      <span className="inline-flex items-center gap-1">
                        {selectionCaps.map((c, idx) => (
                          <Keycap key={idx}>{c}</Keycap>
                        ))}
                      </span>
                    ) : settings?.doubleTapSelection ? (
                      <Keycap>双击修饰键</Keycap>
                    ) : (
                      <span className="text-xs text-zinc-400">未设置</span>
                    )}
                  </div>
                </div>
              </section>

              {/* Recent Saved Words */}
              <section className="rounded-xl border border-zinc-200/80 dark:border-zinc-800 bg-white dark:bg-zinc-900 p-4 flex flex-col gap-3">
                <div className="flex items-center justify-between">
                  <h2 className="text-sm font-medium text-zinc-950 dark:text-zinc-50">最近收藏</h2>
                  <button
                    type="button"
                    onClick={() => setTab("wordbook")}
                    className="text-xs text-zinc-500 dark:text-zinc-400 hover:text-zinc-900 dark:hover:text-zinc-100 inline-flex items-center gap-0.5 cursor-pointer transition-colors"
                  >
                    <span>全部</span>
                    <ArrowRight className="w-3 h-3" />
                  </button>
                </div>

                {entries.length > 0 ? (
                  <div className="divide-y divide-zinc-100 dark:divide-zinc-800/80">
                    {entries.slice(0, 3).map((item) => (
                      <button
                        key={item.id}
                        type="button"
                        onClick={() => setTab("wordbook")}
                        className="w-full py-2 first:pt-0 last:pb-0 flex items-baseline justify-between gap-3 text-left hover:opacity-80 cursor-pointer transition-opacity"
                      >
                        <span className="font-medium text-[13px] text-zinc-900 dark:text-zinc-100 truncate">
                          {item.source}
                        </span>
                        <span className="text-xs text-zinc-500 dark:text-zinc-400 truncate max-w-[58%]">
                          {item.translation}
                        </span>
                      </button>
                    ))}
                  </div>
                ) : (
                  <div className="flex-1 flex items-center justify-center text-xs text-zinc-400 dark:text-zinc-500 py-6">
                    暂无收藏
                  </div>
                )}
              </section>
            </div>
          </div>
        )}

        {tab === "translate" && (
          <div className="max-w-3xl mx-auto space-y-4">
            <div className="flex items-center justify-between gap-4">
              <h1 className="text-lg font-semibold tracking-[-0.02em] text-zinc-950 dark:text-zinc-50">
                翻译
              </h1>
              <div className="inline-flex shrink-0 rounded-lg p-0.5 bg-zinc-200/70 dark:bg-zinc-800">
                <button
                  type="button"
                  onClick={() => setManualMode("selection")}
                  className={`px-3 py-1 rounded-md text-xs font-medium whitespace-nowrap cursor-pointer transition-colors ${
                    manualMode === "selection"
                      ? "bg-white dark:bg-zinc-900 shadow-2xs text-zinc-900 dark:text-zinc-100"
                      : "text-zinc-600 dark:text-zinc-400 hover:text-zinc-900 dark:hover:text-zinc-100"
                  }`}
                >
                  译文
                </button>
                <button
                  type="button"
                  onClick={() => setManualMode("explanation")}
                  className={`px-3 py-1 rounded-md text-xs font-medium whitespace-nowrap cursor-pointer transition-colors ${
                    manualMode === "explanation"
                      ? "bg-white dark:bg-zinc-900 shadow-2xs text-zinc-900 dark:text-zinc-100"
                      : "text-zinc-600 dark:text-zinc-400 hover:text-zinc-900 dark:hover:text-zinc-100"
                  }`}
                >
                  解析
                </button>
              </div>
            </div>

            <div className="rounded-xl border border-zinc-200/80 dark:border-zinc-800 bg-white dark:bg-zinc-900 overflow-hidden">
              <textarea
                value={inputText}
                onChange={(e) => setInputText(e.target.value)}
                onKeyDown={(e) => {
                  if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
                    e.preventDefault();
                    runManualTranslate();
                  }
                }}
                rows={5}
                placeholder="输入或粘贴文本"
                className="w-full p-4 bg-transparent resize-none focus:outline-none text-sm leading-relaxed select-text placeholder:text-zinc-400"
              />
              <div className="px-4 py-2.5 bg-zinc-50/80 dark:bg-zinc-900/60 border-t border-zinc-100 dark:border-zinc-800 flex items-center justify-between gap-3">
                <span className="text-xs text-zinc-400 dark:text-zinc-500 tabular-nums whitespace-nowrap">
                  {inputText.length.toLocaleString()} / 5,000
                </span>
                <div className="flex items-center gap-2 shrink-0">
                  {inputText && (
                    <button
                      type="button"
                      onClick={() => {
                        setInputText("");
                        setManualDict(null);
                        setManualFast(null);
                        setManualLlm("");
                        setManualError(null);
                      }}
                      className="h-8 px-3 shrink-0 whitespace-nowrap rounded-lg text-xs font-medium text-zinc-600 dark:text-zinc-400 hover:bg-zinc-200/60 dark:hover:bg-zinc-800 cursor-pointer transition-colors"
                    >
                      清空
                    </button>
                  )}
                  <button
                    type="button"
                    onClick={runManualTranslate}
                    disabled={!inputText.trim() || manualStreaming}
                    className="h-8 px-4 shrink-0 whitespace-nowrap rounded-lg bg-zinc-900 hover:bg-zinc-800 dark:bg-zinc-100 dark:hover:bg-white disabled:opacity-40 text-white dark:text-zinc-900 text-xs font-medium inline-flex items-center gap-1.5 cursor-pointer transition-colors"
                  >
                    <span>
                      {manualStreaming ? "生成中…" : manualMode === "explanation" ? "开始解析" : "翻译"}
                    </span>
                  </button>
                </div>
              </div>
            </div>

            {(manualDict || manualFast || manualLlm || manualStreaming || manualError) && (
              <div className="rounded-xl border border-zinc-200/80 dark:border-zinc-800 bg-white dark:bg-zinc-900 p-4 space-y-3">
                <div className="flex items-center justify-between gap-3">
                  <div className="flex items-center gap-2 text-xs text-zinc-500 dark:text-zinc-400 min-w-0">
                    <span className="whitespace-nowrap">结果</span>
                    {manualTtft !== null && (
                      <span className="px-1.5 py-0.5 rounded bg-zinc-100 dark:bg-zinc-800 text-zinc-600 dark:text-zinc-300 tabular-nums whitespace-nowrap">
                        {manualTtft} ms
                      </span>
                    )}
                  </div>
                  <div className="flex items-center gap-1.5 shrink-0">
                    <button
                      type="button"
                      onClick={async () => {
                        const out = manualLlm || manualFast || "";
                        if (!out) return;
                        await invoke("popup_copy_text", { text: out });
                        setManualCopied(true);
                        setTimeout(() => setManualCopied(false), 1400);
                      }}
                      className="h-7 px-2.5 shrink-0 whitespace-nowrap rounded-md border border-zinc-200 dark:border-zinc-700 text-xs inline-flex items-center gap-1 hover:bg-zinc-100 dark:hover:bg-zinc-800 cursor-pointer transition-colors"
                    >
                      {manualCopied ? (
                        <Check className="w-3.5 h-3.5 shrink-0 text-zinc-900 dark:text-zinc-100" />
                      ) : (
                        <Copy className="w-3.5 h-3.5 shrink-0" />
                      )}
                      <span>复制</span>
                    </button>
                    <button
                      type="button"
                      onClick={async () => {
                        const out = manualLlm || manualFast || "";
                        if (!inputText.trim() || !out.trim()) return;
                        await invoke("save_manual_entry", {
                          source: inputText.trim(),
                          translation: out.trim(),
                          context: "",
                        });
                        const list = await invoke<SavedEntry[]>("list_saved_entries");
                        setEntries(list);
                      }}
                      className="h-7 px-2.5 shrink-0 whitespace-nowrap rounded-md border border-zinc-200 dark:border-zinc-700 text-xs inline-flex items-center gap-1 hover:bg-zinc-100 dark:hover:bg-zinc-800 cursor-pointer transition-colors"
                    >
                      <Bookmark className="w-3.5 h-3.5 shrink-0" />
                      <span>收藏</span>
                    </button>
                  </div>
                </div>

                {manualDict && (
                  <div className="space-y-1 pb-2 border-b border-zinc-100 dark:border-zinc-800">
                    {manualDict.senses.map((s, i) => (
                      <div key={i} className="flex items-baseline gap-2 text-[13px]">
                        {s.partOfSpeech && (
                          <span className="font-mono text-xs px-1.5 py-0.5 rounded bg-zinc-100 dark:bg-zinc-800 text-zinc-600 dark:text-zinc-300">
                            {s.partOfSpeech}
                          </span>
                        )}
                        <span className="select-text">{s.glosses.join("；")}</span>
                      </div>
                    ))}
                  </div>
                )}

                {(manualLlm || manualFast) && (
                  <div className="select-text text-sm leading-relaxed whitespace-pre-wrap">
                    {manualLlm || manualFast}
                    {manualStreaming && (
                      <span className="inline-block w-1.5 h-4 ml-0.5 align-middle rounded-xs bg-zinc-900 dark:bg-zinc-100 animate-pulse motion-reduce:animate-none" />
                    )}
                  </div>
                )}

                {manualError && (
                  <div className="text-xs text-rose-600 dark:text-rose-400">{manualError}</div>
                )}
              </div>
            )}
          </div>
        )}

        {tab === "wordbook" && (
          <div className="max-w-3xl mx-auto space-y-4">
            <div className="flex items-center justify-between gap-4">
              <h1 className="text-lg font-semibold tracking-[-0.02em] text-zinc-950 dark:text-zinc-50">
                生词本
              </h1>
              <div className="flex items-center gap-2 shrink-0">
                <input
                  ref={fileInputRef}
                  type="file"
                  accept=".json,application/json"
                  onChange={handleImportFile}
                  className="hidden"
                />
                <button
                  type="button"
                  onClick={() => fileInputRef.current?.click()}
                  className="h-8 px-3 shrink-0 whitespace-nowrap rounded-lg border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-900 hover:bg-zinc-100 dark:hover:bg-zinc-800 inline-flex items-center gap-1.5 text-xs font-medium cursor-pointer transition-colors"
                >
                  <Upload className="w-3.5 h-3.5 shrink-0" strokeWidth={1.75} />
                  <span>导入</span>
                </button>
                <button
                  type="button"
                  onClick={handleExportJson}
                  disabled={entries.length === 0}
                  className="h-8 px-3 shrink-0 whitespace-nowrap rounded-lg border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-900 hover:bg-zinc-100 dark:hover:bg-zinc-800 disabled:opacity-40 inline-flex items-center gap-1.5 text-xs font-medium cursor-pointer transition-colors"
                >
                  <Download className="w-3.5 h-3.5 shrink-0" strokeWidth={1.75} />
                  <span>导出</span>
                </button>
              </div>
            </div>

            {wordbookNotice && (
              <div className="px-3 py-2 rounded-lg bg-zinc-100 dark:bg-zinc-800/90 border border-zinc-200 dark:border-zinc-700 text-zinc-800 dark:text-zinc-200 text-xs">
                {wordbookNotice}
              </div>
            )}

            <div className="relative">
              <Search className="w-4 h-4 text-zinc-400 absolute left-3 top-1/2 -translate-y-1/2" strokeWidth={1.75} />
              <input
                type="search"
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                placeholder="搜索生词或释义"
                className="w-full h-9 pl-9 pr-3 rounded-lg border border-zinc-200/90 dark:border-zinc-800 bg-white dark:bg-zinc-900 focus:outline-none focus:border-zinc-900 dark:focus:border-zinc-100 text-[13px] placeholder:text-zinc-400"
              />
            </div>

            {filteredEntries.length === 0 ? (
              <div className="rounded-xl border border-zinc-200/80 dark:border-zinc-800 bg-white dark:bg-zinc-900 py-12 text-center text-xs text-zinc-400 dark:text-zinc-500">
                {entries.length === 0 ? "暂无生词" : "无匹配结果"}
              </div>
            ) : (
              <div className="space-y-2">
                {filteredEntries.map((item) => (
                  <div
                    key={item.id}
                    className="rounded-xl border border-zinc-200/80 dark:border-zinc-800 bg-white dark:bg-zinc-900 p-4 flex items-start justify-between gap-4"
                  >
                    <div className="space-y-1 min-w-0 select-text">
                      <div className="flex items-baseline gap-2.5 flex-wrap">
                        <span className="text-sm font-semibold text-zinc-950 dark:text-zinc-50">
                          {item.source}
                        </span>
                        <span className="text-[13px] text-zinc-600 dark:text-zinc-300">
                          {item.translation}
                        </span>
                      </div>
                      {item.context && (
                        <div className="text-xs text-zinc-500 dark:text-zinc-400 leading-relaxed">
                          {item.context}
                        </div>
                      )}
                    </div>
                    <button
                      type="button"
                      title="删除"
                      aria-label="删除"
                      onClick={async () => {
                        await invoke("delete_saved_entry", { id: item.id });
                        setEntries((prev) => prev.filter((x) => x.id !== item.id));
                      }}
                      className="w-7 h-7 shrink-0 rounded-md inline-flex items-center justify-center text-zinc-400 hover:text-rose-600 hover:bg-rose-500/10 cursor-pointer transition-colors"
                    >
                      <Trash2 className="w-3.5 h-3.5" strokeWidth={1.75} />
                    </button>
                  </div>
                ))}
              </div>
            )}
          </div>
        )}

        {tab === "settings" && settings && activeEndpoint && (
          <div className="max-w-3xl mx-auto space-y-5 pb-8">
            <div className="flex items-center justify-between gap-4">
              <h1 className="text-lg font-semibold tracking-[-0.02em] text-zinc-950 dark:text-zinc-50">
                设置
              </h1>
              {settingsSavedBanner && (
                <span className="shrink-0 whitespace-nowrap text-xs text-zinc-500 dark:text-zinc-400">
                  已保存
                </span>
              )}
            </div>

            {/* Section 1: OS Permissions */}
            {permissions && permissions.platform === "macos" && (
              <section className="rounded-xl border border-zinc-200/80 dark:border-zinc-800 bg-white dark:bg-zinc-900 p-4 space-y-3">
                <h2 className="text-sm font-medium text-zinc-950 dark:text-zinc-50">系统权限</h2>
                <div className="divide-y divide-zinc-100 dark:divide-zinc-800/80">
                  <div className="py-2 first:pt-0 flex items-center justify-between gap-4">
                    <span className="text-[13px] text-zinc-800 dark:text-zinc-200">辅助功能</span>
                    {permissions.accessibility ? (
                      <span className="text-xs text-emerald-600 dark:text-emerald-400 font-medium">
                        已授权
                      </span>
                    ) : (
                      <button
                        type="button"
                        onClick={async () => {
                          const p = await invoke<PermissionsStatus>("get_permissions_status", {
                            prompt: true,
                          });
                          setPermissions(p);
                        }}
                        className="h-7 px-2.5 rounded-md bg-zinc-900 hover:bg-zinc-800 dark:bg-zinc-100 dark:hover:bg-white text-white dark:text-zinc-900 text-xs font-medium cursor-pointer transition-colors"
                      >
                        授权
                      </button>
                    )}
                  </div>
                  <div className="py-2 last:pb-0 flex items-center justify-between gap-4">
                    <span className="text-[13px] text-zinc-800 dark:text-zinc-200">屏幕录制</span>
                    {permissions.screenRecording ? (
                      <span className="text-xs text-emerald-600 dark:text-emerald-400 font-medium">
                        已授权
                      </span>
                    ) : (
                      <button
                        type="button"
                        onClick={async () => {
                          const p = await invoke<PermissionsStatus>("request_screen_recording");
                          setPermissions(p);
                        }}
                        className="h-7 px-2.5 rounded-md bg-zinc-900 hover:bg-zinc-800 dark:bg-zinc-100 dark:hover:bg-white text-white dark:text-zinc-900 text-xs font-medium cursor-pointer transition-colors"
                      >
                        授权
                      </button>
                    )}
                  </div>
                </div>
              </section>
            )}

            {/* Section 2: Global Shortcuts */}
            <section className="rounded-xl border border-zinc-200/80 dark:border-zinc-800 bg-white dark:bg-zinc-900 p-4 space-y-3">
              <div className="flex items-center justify-between gap-3">
                <h2 className="text-sm font-medium text-zinc-950 dark:text-zinc-50">快捷键</h2>
                {recordingTarget !== null && (
                  <span className="text-xs text-zinc-500 dark:text-zinc-400 whitespace-nowrap">
                    Esc 取消 · Delete 清空
                  </span>
                )}
              </div>

              <div className="divide-y divide-zinc-100 dark:divide-zinc-800/80">
                {(
                  [
                    {
                      target: "capture" as ShortcutTarget,
                      title: "截屏翻译",
                      value: settings.captureShortcut,
                      defaultValue: defaultCaptureShortcut,
                      onChange: (val: string) =>
                        persistSettings({ ...settings, captureShortcut: val }),
                    },
                    {
                      target: "selection" as ShortcutTarget,
                      title: "划词翻译",
                      value: settings.selectionShortcut,
                      defaultValue: defaultSelectionShortcut,
                      onChange: (val: string) =>
                        persistSettings({ ...settings, selectionShortcut: val }),
                    },
                  ]
                ).map((item) => {
                  const isRec = recordingTarget === item.target;
                  const caps = parseShortcutKeycaps(item.value, isMac);
                  const isCustom = item.value !== item.defaultValue;

                  return (
                    <div
                      key={item.target}
                      className="py-2.5 first:pt-0 last:pb-0 flex items-center justify-between gap-4"
                    >
                      <span className="text-[13px] text-zinc-800 dark:text-zinc-200">
                        {item.title}
                      </span>

                      <div className="flex items-center gap-1.5 shrink-0">
                        <button
                          type="button"
                          onClick={() => {
                            setShortcutHint(null);
                            setRecordingModsPreview([]);
                            setRecordingTarget(isRec ? null : item.target);
                          }}
                          className={`h-8 px-2.5 rounded-lg border inline-flex items-center gap-1 text-xs font-medium whitespace-nowrap shrink-0 cursor-pointer transition-colors ${
                            isRec
                              ? "border-zinc-900 dark:border-zinc-100 bg-zinc-900/[0.05] dark:bg-white/[0.08] text-zinc-950 dark:text-zinc-50"
                              : "border-zinc-200 dark:border-zinc-700 bg-zinc-50/80 dark:bg-zinc-800/60 hover:border-zinc-400 dark:hover:border-zinc-600 text-zinc-800 dark:text-zinc-100"
                          }`}
                        >
                          {isRec ? (
                            recordingModsPreview.length > 0 ? (
                              <span className="inline-flex items-center gap-1">
                                {recordingModsPreview.map((m, idx) => (
                                  <Keycap key={idx}>{m}</Keycap>
                                ))}
                                <span className="text-zinc-400">…</span>
                              </span>
                            ) : (
                              <span>按下组合键…</span>
                            )
                          ) : caps.length > 0 ? (
                            caps.map((cap, idx) => <Keycap key={idx}>{cap}</Keycap>)
                          ) : (
                            <span className="text-zinc-400">点击设置</span>
                          )}
                        </button>

                        {item.value && !isRec && (
                          <button
                            type="button"
                            title="清空"
                            aria-label="清空"
                            onClick={() => {
                              setShortcutHint(null);
                              item.onChange("");
                            }}
                            className="w-7 h-8 shrink-0 rounded-md inline-flex items-center justify-center text-zinc-400 hover:text-rose-500 hover:bg-rose-500/10 cursor-pointer transition-colors"
                          >
                            <X className="w-3.5 h-3.5 shrink-0" />
                          </button>
                        )}

                        {isCustom && !isRec && (
                          <button
                            type="button"
                            title={`恢复默认 (${formatShortcutInline(item.defaultValue, isMac)})`}
                            onClick={() => {
                              setShortcutHint(null);
                              item.onChange(item.defaultValue);
                            }}
                            className="h-8 px-2 shrink-0 whitespace-nowrap rounded-md text-xs text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100 hover:bg-zinc-100 dark:hover:bg-zinc-800 inline-flex items-center gap-1 cursor-pointer transition-colors"
                          >
                            <RotateCcw className="w-3 h-3 shrink-0" />
                            <span>重置</span>
                          </button>
                        )}
                      </div>
                    </div>
                  );
                })}
              </div>

              {shortcutHint && (
                <div className="px-3 py-2 rounded-lg bg-amber-500/10 border border-amber-500/25 text-amber-800 dark:text-amber-300 text-xs">
                  {shortcutHint}
                </div>
              )}
            </section>

            {/* Section 3: Trigger & Interaction */}
            <section className="rounded-xl border border-zinc-200/80 dark:border-zinc-800 bg-white dark:bg-zinc-900 p-4 space-y-4">
              <h2 className="text-sm font-medium text-zinc-950 dark:text-zinc-50">取词与交互</h2>

              <div className="grid grid-cols-2 gap-4">
                <label className="space-y-1.5 block">
                  <span className="text-xs text-zinc-500 dark:text-zinc-400 block">悬停修饰键</span>
                  <select
                    value={settings.triggerModifier}
                    onChange={(e) =>
                      persistSettings({
                        ...settings,
                        triggerModifier: e.target.value as TriggerModifier,
                      })
                    }
                    className="w-full h-9 px-2.5 rounded-lg border border-zinc-200 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-800 text-[13px]"
                  >
                    <option value="altOrOption">{isMac ? "Option（左右皆可）" : "Alt（左右皆可）"}</option>
                    <option value="leftOption">{isMac ? "左 Option" : "左 Alt"}</option>
                    <option value="rightOption">{isMac ? "右 Option" : "右 Alt"}</option>
                    <option value="control">{isMac ? "Control" : "Ctrl"}</option>
                  </select>
                </label>

                <label className="space-y-1.5 block">
                  <div className="flex items-center justify-between text-xs">
                    <span className="text-zinc-500 dark:text-zinc-400">悬停延迟</span>
                    <span className="font-medium text-zinc-900 dark:text-zinc-100 tabular-nums">
                      {settings.hoverDwellMs} ms
                    </span>
                  </div>
                  <input
                    type="range"
                    min={80}
                    max={500}
                    step={10}
                    value={settings.hoverDwellMs}
                    onChange={(e) =>
                      persistSettings({ ...settings, hoverDwellMs: Number(e.target.value) })
                    }
                    className="w-full accent-zinc-900 dark:accent-zinc-100 mt-2"
                  />
                </label>

                {!settings.dismissOnTriggerRelease && (
                  <label className="space-y-1.5 block col-span-2">
                    <div className="flex items-center justify-between text-xs">
                      <span className="text-zinc-500 dark:text-zinc-400">浮窗驻留时长</span>
                      <span className="font-medium text-zinc-900 dark:text-zinc-100 tabular-nums">
                        {(settings.lingerDismissMs / 1000).toFixed(1)} s
                      </span>
                    </div>
                    <input
                      type="range"
                      min={1000}
                      max={6000}
                      step={250}
                      value={settings.lingerDismissMs}
                      onChange={(e) =>
                        persistSettings({ ...settings, lingerDismissMs: Number(e.target.value) })
                      }
                      className="w-full accent-zinc-900 dark:accent-zinc-100 mt-1"
                    />
                  </label>
                )}
              </div>

              <div className="divide-y divide-zinc-100 dark:divide-zinc-800/80 border-t border-zinc-100 dark:border-zinc-800/80 pt-1">
                <label className="py-2.5 flex items-center justify-between gap-4 cursor-pointer">
                  <span className="text-[13px] text-zinc-800 dark:text-zinc-200">
                    松开修饰键后立即收起浮窗
                  </span>
                  <input
                    type="checkbox"
                    checked={settings.dismissOnTriggerRelease}
                    onChange={(e) =>
                      persistSettings({ ...settings, dismissOnTriggerRelease: e.target.checked })
                    }
                    className="w-4 h-4 shrink-0 accent-zinc-900 dark:accent-zinc-100"
                  />
                </label>

                <label className="py-2.5 flex items-center justify-between gap-4 cursor-pointer">
                  <span className="text-[13px] text-zinc-800 dark:text-zinc-200">
                    双击修饰键翻译选中文本
                  </span>
                  <input
                    type="checkbox"
                    checked={settings.doubleTapSelection}
                    onChange={(e) =>
                      persistSettings({ ...settings, doubleTapSelection: e.target.checked })
                    }
                    className="w-4 h-4 shrink-0 accent-zinc-900 dark:accent-zinc-100"
                  />
                </label>

                <label className="py-2.5 flex items-center justify-between gap-4 cursor-pointer">
                  <span className="text-[13px] text-zinc-800 dark:text-zinc-200">
                    无障碍受限时使用离线 OCR 识别
                  </span>
                  <input
                    type="checkbox"
                    checked={settings.enableOcrFallback}
                    onChange={(e) =>
                      persistSettings({ ...settings, enableOcrFallback: e.target.checked })
                    }
                    className="w-4 h-4 shrink-0 accent-zinc-900 dark:accent-zinc-100"
                  />
                </label>

                <label className="py-2.5 last:pb-0 flex items-center justify-between gap-4 cursor-pointer">
                  <span className="text-[13px] text-zinc-800 dark:text-zinc-200">
                    无障碍受限时通过剪贴板读取选区
                  </span>
                  <input
                    type="checkbox"
                    checked={settings.enableClipboardFallback}
                    onChange={(e) =>
                      persistSettings({ ...settings, enableClipboardFallback: e.target.checked })
                    }
                    className="w-4 h-4 shrink-0 accent-zinc-900 dark:accent-zinc-100"
                  />
                </label>
              </div>
            </section>

            {/* Section 4: Translation Providers */}
            <section className="rounded-xl border border-zinc-200/80 dark:border-zinc-800 bg-white dark:bg-zinc-900 p-4 space-y-4">
              <h2 className="text-sm font-medium text-zinc-950 dark:text-zinc-50">翻译服务</h2>

              <div className="divide-y divide-zinc-100 dark:divide-zinc-800/80 border-b border-zinc-100 dark:border-zinc-800/80 pb-1">
                <label className="py-2.5 first:pt-0 flex items-center justify-between gap-4 cursor-pointer">
                  <span className="text-[13px] text-zinc-800 dark:text-zinc-200">
                    流式翻译前先显示快速机翻
                  </span>
                  <input
                    type="checkbox"
                    checked={settings.enableFastMt}
                    onChange={(e) => persistSettings({ ...settings, enableFastMt: e.target.checked })}
                    className="w-4 h-4 shrink-0 accent-zinc-900 dark:accent-zinc-100"
                  />
                </label>

                <label className="py-2.5 flex items-center justify-between gap-4 cursor-pointer">
                  <span className="text-[13px] text-zinc-800 dark:text-zinc-200">
                    查单个词时不调用大模型
                  </span>
                  <input
                    type="checkbox"
                    checked={settings.wordFastOnly}
                    onChange={(e) => persistSettings({ ...settings, wordFastOnly: e.target.checked })}
                    className="w-4 h-4 shrink-0 accent-zinc-900 dark:accent-zinc-100"
                  />
                </label>
              </div>

              <div className="grid grid-cols-3 gap-2">
                {(
                  [
                    { id: "gemini", label: "Gemini" },
                    { id: "deepseek", label: "DeepSeek" },
                    { id: "openaiCompatible", label: "OpenAI 兼容" },
                  ] as { id: LlmProviderKind; label: string }[]
                ).map((p) => (
                  <button
                    key={p.id}
                    type="button"
                    onClick={() => {
                      setTestResult(null);
                      persistSettings({ ...settings, activeProvider: p.id });
                    }}
                    className={`h-9 px-3 whitespace-nowrap rounded-lg border text-[13px] font-medium cursor-pointer transition-colors ${
                      settings.activeProvider === p.id
                        ? "border-zinc-900 dark:border-zinc-100 bg-zinc-900 text-white dark:bg-zinc-100 dark:text-zinc-900"
                        : "border-zinc-200 dark:border-zinc-700 hover:bg-zinc-50 dark:hover:bg-zinc-800 text-zinc-700 dark:text-zinc-300"
                    }`}
                  >
                    {p.label}
                  </button>
                ))}
              </div>

              <div className="grid grid-cols-2 gap-3">
                <label className="space-y-1.5 block">
                  <span className="text-xs text-zinc-500 dark:text-zinc-400 block">接口地址</span>
                  <input
                    type="url"
                    value={activeEndpoint.baseUrl}
                    onChange={(e) => updateActiveEndpoint({ baseUrl: e.target.value })}
                    placeholder="https://api.openai.com/v1"
                    className="w-full h-9 px-3 rounded-lg border border-zinc-200 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-800 text-[13px]"
                  />
                </label>

                <label className="space-y-1.5 block">
                  <span className="text-xs text-zinc-500 dark:text-zinc-400 block">模型</span>
                  <input
                    type="text"
                    value={activeEndpoint.model}
                    onChange={(e) => updateActiveEndpoint({ model: e.target.value })}
                    placeholder="gemini-2.5-flash-lite"
                    className="w-full h-9 px-3 rounded-lg border border-zinc-200 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-800 text-[13px]"
                  />
                </label>
              </div>

              <div className="space-y-1.5">
                <div className="flex items-center justify-between text-xs">
                  <span className="text-zinc-500 dark:text-zinc-400">API Key</span>
                  <span
                    className={
                      keyStatus[settings.activeProvider]
                        ? "text-zinc-600 dark:text-zinc-300"
                        : "text-amber-600 dark:text-amber-400"
                    }
                  >
                    {keyStatus[settings.activeProvider] ? "已配置" : "未配置"}
                  </span>
                </div>
                <div className="flex items-center gap-2">
                  <input
                    type="password"
                    value={apiKeyDraft}
                    onChange={(e) => setApiKeyDraft(e.target.value)}
                    placeholder={
                      keyStatus[settings.activeProvider] ? "输入新密钥以替换" : "输入 API Key"
                    }
                    className="min-w-0 flex-1 h-9 px-3 rounded-lg border border-zinc-200 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-800 text-[13px] placeholder:text-zinc-400"
                  />
                  <button
                    type="button"
                    onClick={handleSaveApiKey}
                    disabled={!apiKeyDraft.trim()}
                    className="h-9 px-3.5 shrink-0 whitespace-nowrap rounded-lg bg-zinc-900 hover:bg-zinc-800 dark:bg-zinc-100 dark:hover:bg-white text-white dark:text-zinc-900 disabled:opacity-40 text-xs font-medium cursor-pointer transition-colors"
                  >
                    保存
                  </button>
                  <button
                    type="button"
                    onClick={handleTestConnection}
                    disabled={testingConn}
                    className="h-9 px-3.5 shrink-0 whitespace-nowrap rounded-lg border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-900 hover:bg-zinc-100 dark:hover:bg-zinc-800 text-zinc-800 dark:text-zinc-200 text-xs font-medium cursor-pointer transition-colors"
                  >
                    {testingConn ? "测试中…" : "测试连接"}
                  </button>
                </div>
              </div>

              {testResult && (
                <div
                  className={`px-3 py-2 rounded-lg text-xs border tabular-nums ${
                    testResult.ok
                      ? "bg-emerald-500/10 border-emerald-500/25 text-emerald-800 dark:text-emerald-300"
                      : "bg-rose-500/10 border-rose-500/25 text-rose-700 dark:text-rose-300"
                  }`}
                >
                  {testResult.ok
                    ? `连接正常 · 首字 ${testResult.ttftMs} ms · 总计 ${testResult.totalMs} ms`
                    : `连接失败：${testResult.error}`}
                </div>
              )}
            </section>

            {/* Section 5: Excluded Applications */}
            <section className="rounded-xl border border-zinc-200/80 dark:border-zinc-800 bg-white dark:bg-zinc-900 p-4 space-y-3">
              <h2 className="text-sm font-medium text-zinc-950 dark:text-zinc-50">排除应用</h2>

              {settings.excludedApps.length > 0 && (
                <div className="flex flex-wrap gap-1.5">
                  {settings.excludedApps.map((appId) => (
                    <span
                      key={appId}
                      className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-md bg-zinc-100 dark:bg-zinc-800 text-xs font-mono text-zinc-700 dark:text-zinc-300"
                    >
                      <span>{appId}</span>
                      <button
                        type="button"
                        aria-label={`移除 ${appId}`}
                        onClick={() =>
                          persistSettings({
                            ...settings,
                            excludedApps: settings.excludedApps.filter((x) => x !== appId),
                          })
                        }
                        className="text-zinc-400 hover:text-rose-500 cursor-pointer transition-colors"
                      >
                        ×
                      </button>
                    </span>
                  ))}
                </div>
              )}

              <div className="flex items-center gap-2">
                <input
                  type="text"
                  value={newExcludedApp}
                  onChange={(e) => setNewExcludedApp(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      const clean = newExcludedApp.trim();
                      if (!clean || settings.excludedApps.includes(clean)) return;
                      setNewExcludedApp("");
                      persistSettings({
                        ...settings,
                        excludedApps: [...settings.excludedApps, clean],
                      });
                    }
                  }}
                  placeholder={isMac ? "输入 Bundle ID，如 com.apple.Terminal" : "输入进程名，如 code.exe"}
                  className="min-w-0 flex-1 h-8 px-3 rounded-lg border border-zinc-200 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-800 text-xs font-mono placeholder:font-sans placeholder:text-zinc-400"
                />
                <button
                  type="button"
                  onClick={() => {
                    const clean = newExcludedApp.trim();
                    if (!clean || settings.excludedApps.includes(clean)) return;
                    setNewExcludedApp("");
                    persistSettings({
                      ...settings,
                      excludedApps: [...settings.excludedApps, clean],
                    });
                  }}
                  className="h-8 px-3 shrink-0 whitespace-nowrap rounded-lg border border-zinc-200 dark:border-zinc-700 hover:bg-zinc-100 dark:hover:bg-zinc-800 inline-flex items-center gap-1 text-xs font-medium cursor-pointer transition-colors"
                >
                  <Plus className="w-3.5 h-3.5 shrink-0" />
                  <span>添加</span>
                </button>
              </div>
            </section>
          </div>
        )}
      </main>
    </div>
  );
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <MainApp />
  </React.StrictMode>
);
