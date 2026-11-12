import { invoke } from "@tauri-apps/api/core";
import { listen, type EventCallback, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppSettings,
  ConnectionTestResult,
  HeatmapDayCell,
  ManualStreamEvent,
  PermissionsStatus,
  QueryMode,
  SavedEntry,
  UsageStatsSummary,
} from "../shared/types";

export const isTauriRuntime = (): boolean =>
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

// Browser-only preview state so `pnpm dev` can be visually audited outside Tauri
// without ever affecting real Tauri IPC calls.
let previewSettings: AppSettings = {
  enabled: true,
  triggerModifier: "altOrOption",
  hoverDwellMs: 180,
  dismissOnTriggerRelease: true,
  lingerDismissMs: 2500,
  debounceRadiusPt: 8,
  doubleTapSelection: true,
  selectionShortcut: "Alt+Command+T",
  captureShortcut: "Control+Command+S",
  enableOcrFallback: true,
  enableClipboardFallback: true,
  enableFastMt: true,
  wordFastOnly: false,
  enableLlmTranslation: true,
  activeProvider: "gemini",
  gemini: {
    baseUrl: "https://generativelanguage.googleapis.com/v1beta/openai",
    model: "gemini-2.5-flash-lite",
  },
  deepseek: {
    baseUrl: "https://api.deepseek.com/v1",
    model: "deepseek-chat",
  },
  openaiCompatible: {
    baseUrl: "https://api.openai.com/v1",
    model: "gpt-4o-mini",
  },
  targetLanguage: "简体中文",
  excludedApps: ["com.apple.Terminal", "com.microsoft.VSCode"],
};

let previewKeyStatus: Record<string, boolean> = {
  gemini: true,
  deepseek: false,
  openaiCompatible: false,
};

let previewEntries: SavedEntry[] = [
  {
    id: "1",
    source: "ephemeral",
    translation: "短暂的；朝生暮死的；转瞬即逝的",
    context: "Liquid Glass creates an ephemeral layer of controls that defers to your content.",
    createdAt: "2026-11-11T09:12:00Z",
  },
  {
    id: "2",
    source: "vernacular",
    translation: "本土语言；日常用语；（特定领域的）行话与视觉语汇",
    context: "Draw the visual world from the subject's own materials, artifacts, and vernacular.",
    createdAt: "2026-11-10T18:45:00Z",
  },
  {
    id: "3",
    source: "perspicuous",
    translation: "清晰明了的；易于理解的；表达透彻的",
    context: "A perspicuous layout communicates hierarchy through proportion rather than borders.",
    createdAt: "2026-11-08T14:20:00Z",
  },
  {
    id: "4",
    source: "palimpsest",
    translation: "重写本；多层叠加而仍留旧迹的事物",
    context: "Every mature codebase is a palimpsest of earlier architectural assumptions.",
    createdAt: "2026-11-05T21:04:00Z",
  },
];

function buildPreviewStats(): UsageStatsSummary {
  const heatmap: HeatmapDayCell[] = [];
  const end = new Date("2026-11-11T12:00:00");
  for (let i = 363; i >= 0; i--) {
    const d = new Date(end);
    d.setDate(end.getDate() - i);
    const iso = d.toISOString().slice(0, 10);
    const jsDay = d.getDay();
    const weekday = jsDay === 0 ? 7 : jsDay;
    const seed = (i * 37 + weekday * 13) % 19;
    const recentBoost = i < 45 ? 1.35 : 1;
    const rawQueries = seed < 5 ? 0 : Math.round((seed - 3) * 3.2 * recentBoost);
    const wordCount = rawQueries === 0 ? 0 : rawQueries * 11 + ((i * 7) % 17);
    const level =
      wordCount === 0
        ? 0
        : wordCount < 120
          ? 1
          : wordCount < 260
            ? 2
            : wordCount < 420
              ? 3
              : 4;
    heatmap.push({
      date: iso,
      weekday,
      wordCount: i === 0 ? 468 : wordCount,
      charCount: (i === 0 ? 468 : wordCount) * 5,
      queryCount: i === 0 ? 38 : rawQueries,
      level: i === 0 ? 4 : level,
    });
  }
  return {
    todayDate: "2026-11-11",
    todayWords: 468,
    todayChars: 2410,
    todayQueries: 38,
    totalWords: 48920,
    totalChars: 256400,
    totalQueries: 4126,
    activeDays: 184,
    currentStreak: 12,
    longestStreak: 29,
    bestDayDate: "2026-10-24",
    bestDayWords: 920,
    thisWeekWords: 2840,
    lastWeekWords: 2310,
    heatmap,
  };
}

const manualListeners = new Set<(payload: ManualStreamEvent) => void>();

export async function safeListen<T>(
  event: string,
  handler: EventCallback<T>
): Promise<UnlistenFn> {
  if (isTauriRuntime()) {
    return listen<T>(event, handler);
  }
  if (event === "manual://progress") {
    const fn = (payload: ManualStreamEvent) => {
      handler({
        event,
        id: 0,
        payload: payload as unknown as T,
      });
    };
    manualListeners.add(fn);
    return () => {
      manualListeners.delete(fn);
    };
  }
  return () => {};
}

export const appBridge = {
  async getSettings(): Promise<AppSettings> {
    if (isTauriRuntime()) return invoke<AppSettings>("get_settings");
    return previewSettings;
  },

  async saveSettings(settings: AppSettings): Promise<AppSettings> {
    if (isTauriRuntime()) return invoke<AppSettings>("save_settings", { settings });
    previewSettings = { ...settings };
    return previewSettings;
  },

  async getPermissionsStatus(prompt: boolean): Promise<PermissionsStatus> {
    if (isTauriRuntime()) {
      return invoke<PermissionsStatus>("get_permissions_status", { prompt });
    }
    return { platform: "macos", accessibility: true, screenRecording: true };
  },

  async requestScreenRecording(): Promise<PermissionsStatus> {
    if (isTauriRuntime()) return invoke<PermissionsStatus>("request_screen_recording");
    return { platform: "macos", accessibility: true, screenRecording: true };
  },

  async getApiKeyStatus(): Promise<Record<string, boolean>> {
    if (isTauriRuntime()) return invoke<Record<string, boolean>>("get_api_key_status");
    return { ...previewKeyStatus };
  },

  async setApiKey(provider: string, apiKey: string): Promise<void> {
    if (isTauriRuntime()) {
      await invoke("set_api_key", { provider, apiKey });
      return;
    }
    previewKeyStatus = { ...previewKeyStatus, [provider]: Boolean(apiKey.trim()) };
  },

  async testProviderConnection(args: {
    provider: string;
    baseUrl: string;
    model: string;
    apiKeyOverride: string | null;
  }): Promise<ConnectionTestResult> {
    if (isTauriRuntime()) {
      return invoke<ConnectionTestResult>("test_provider_connection", args);
    }
    await new Promise((r) => setTimeout(r, 320));
    return {
      ok: true,
      ttftMs: 138,
      totalMs: 284,
      sampleOutput: "你好",
      error: null,
    };
  },

  async listSavedEntries(): Promise<SavedEntry[]> {
    if (isTauriRuntime()) return invoke<SavedEntry[]>("list_saved_entries");
    return [...previewEntries];
  },

  async saveManualEntry(args: {
    source: string;
    translation: string;
    context: string;
  }): Promise<void> {
    if (isTauriRuntime()) {
      await invoke("save_manual_entry", args);
      return;
    }
    previewEntries = [
      {
        id: String(Date.now()),
        source: args.source,
        translation: args.translation,
        context: args.context,
        createdAt: new Date().toISOString(),
      },
      ...previewEntries,
    ];
  },

  async deleteSavedEntry(id: string): Promise<void> {
    if (isTauriRuntime()) {
      await invoke("delete_saved_entry", { id });
      return;
    }
    previewEntries = previewEntries.filter((x) => x.id !== id);
  },

  async importSavedEntries(jsonContent: string): Promise<number> {
    if (isTauriRuntime()) {
      return invoke<number>("import_saved_entries", { jsonContent });
    }
    const parsed = JSON.parse(jsonContent);
    if (Array.isArray(parsed)) return parsed.length;
    return 0;
  },

  async exportSavedEntries(): Promise<string> {
    if (isTauriRuntime()) return invoke<string>("export_saved_entries");
    return JSON.stringify(previewEntries, null, 2);
  },

  async getUsageStats(): Promise<UsageStatsSummary> {
    if (isTauriRuntime()) return invoke<UsageStatsSummary>("get_usage_stats");
    return buildPreviewStats();
  },

  async setShortcutRecording(active: boolean): Promise<void> {
    if (isTauriRuntime()) {
      await invoke("set_shortcut_recording", { active });
    }
  },

  async startRegionCapture(): Promise<void> {
    if (isTauriRuntime()) {
      await invoke("start_region_capture");
    }
  },

  async copyText(text: string): Promise<void> {
    if (isTauriRuntime()) {
      await invoke("popup_copy_text", { text });
      return;
    }
    await navigator.clipboard?.writeText(text);
  },

  async manualTranslateStream(args: {
    requestId: string;
    text: string;
    mode: QueryMode;
  }): Promise<void> {
    if (isTauriRuntime()) {
      await invoke("manual_translate_stream", args);
      return;
    }
    const normalized = args.text.trim().toLowerCase();
    const isSingleWord = !/\s/.test(normalized) && normalized.length <= 28;
    const knownDict: Record<string, { partOfSpeech: string; glosses: string[] }[]> = {
      perspicuous: [
        { partOfSpeech: "adj.", glosses: ["清晰明了的", "易于理解的", "表达透彻的"] },
      ],
      ephemeral: [
        { partOfSpeech: "adj.", glosses: ["短暂的", "朝生暮死的", "转瞬即逝的"] },
      ],
      vernacular: [
        { partOfSpeech: "n.", glosses: ["本土语言", "日常用语", "特定领域的语汇"] },
        { partOfSpeech: "adj.", glosses: ["用方言写成的", "地方性的"] },
      ],
      palimpsest: [
        { partOfSpeech: "n.", glosses: ["重写本", "多层叠加而仍留旧迹的事物"] },
      ],
    };
    const dict = isSingleWord
      ? {
          word: args.text.trim(),
          truncated: false,
          senses: knownDict[normalized] ?? [
            { partOfSpeech: "adj.", glosses: ["精炼的", "明晰的"] },
          ],
        }
      : null;
    const sample =
      args.mode === "explanation"
        ? isSingleWord
          ? `词源与用法：「${args.text.trim()}」源自拉丁语词根，常见于学术写作与批评文本，用于形容论述、结构或界面表达清晰透彻、一目了然。`
          : "句法解析：主句以核心名词短语领起，通过对比结构突出“以比例与秩序替代冗余装饰”的论述重点。"
        : isSingleWord
          ? knownDict[normalized]?.[0]?.glosses.join("；") ?? args.text.trim()
          : "清晰透彻的界面布局，依靠比例与对齐而非繁复边框来传达层级秩序。";

    manualListeners.forEach((fn) =>
      fn({
        requestId: args.requestId,
        dictionary: dict,
        fastTranslation: null,
        llmTranslation: sample.slice(0, 14),
        done: false,
        ttftMs: 126,
        error: null,
      })
    );
    await new Promise((r) => setTimeout(r, 180));
    manualListeners.forEach((fn) =>
      fn({
        requestId: args.requestId,
        dictionary: dict,
        fastTranslation: null,
        llmTranslation: sample,
        done: true,
        ttftMs: 126,
        error: null,
      })
    );
  },
};
