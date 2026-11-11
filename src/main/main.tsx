import React, { useEffect, useMemo, useRef, useState } from "react";
import ReactDOM from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  Activity,
  Bookmark,
  Check,
  Copy,
  Crop,
  Download,
  KeyRound,
  Languages,
  Plus,
  Search,
  Settings2,
  ShieldCheck,
  Sparkles,
  Trash2,
  Upload,
  Zap,
} from "lucide-react";
import type {
  AppSettings,
  ConnectionTestResult,
  LlmProviderKind,
  ManualStreamEvent,
  PermissionsStatus,
  QueryMode,
  SavedEntry,
  TriggerModifier,
  WordEntry,
} from "../shared/types";
import "../styles.css";

type ActiveTab = "translate" | "wordbook" | "settings";

function MainApp() {
  const [tab, setTab] = useState<ActiveTab>("translate");
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [permissions, setPermissions] = useState<PermissionsStatus | null>(null);
  const [keyStatus, setKeyStatus] = useState<Record<string, boolean>>({});
  const [entries, setEntries] = useState<SavedEntry[]>([]);

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

  const refreshAll = async () => {
    const [s, p, ks, list] = await Promise.all([
      invoke<AppSettings>("get_settings"),
      invoke<PermissionsStatus>("get_permissions_status", { prompt: false }),
      invoke<Record<string, boolean>>("get_api_key_status"),
      invoke<SavedEntry[]>("list_saved_entries"),
    ]);
    setSettings(s);
    setPermissions(p);
    setKeyStatus(ks);
    setEntries(list);
  };

  useEffect(() => {
    refreshAll().catch(() => {});

    const unlistenNav = listen<string>("main://navigate", (ev) => {
      if (ev.payload === "translate" || ev.payload === "wordbook" || ev.payload === "settings") {
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

    return () => {
      unlistenNav.then((f) => f());
      unlistenSettings.then((f) => f());
      unlistenWordbook.then((f) => f());
      unlistenManual.then((f) => f());
    };
  }, []);

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
      setWordbookNotice(`成功导入 ${count} 条新词条（自动兼容 v1 与 v2 格式）`);
    } catch (err) {
      setWordbookNotice(`导入失败: ${String(err)}`);
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

  return (
    <div className="h-screen w-screen flex bg-zinc-50 dark:bg-zinc-950 text-zinc-900 dark:text-zinc-100 text-[13.5px]">
      {/* Left Navigation Sidebar (HIG Two-Column Split View) */}
      <aside className="w-56 shrink-0 border-r border-zinc-200 dark:border-zinc-800/90 bg-zinc-100/80 dark:bg-zinc-900/70 flex flex-col justify-between p-3">
        <div className="space-y-4">
          <div className="px-2.5 pt-1 flex items-center justify-between">
            <div>
              <div className="text-[15px] font-semibold tracking-tight">Light Translate</div>
              <div className="text-[11px] text-zinc-500 dark:text-zinc-400">2.0 跨平台极速引擎</div>
            </div>
            {settings && (
              <button
                type="button"
                title={settings.enabled ? "全局取词已开启" : "全局取词已暂停"}
                onClick={() => persistSettings({ ...settings, enabled: !settings.enabled })}
                className={`w-3 h-3 rounded-full cursor-pointer transition-colors ${
                  settings.enabled ? "bg-emerald-500 shadow-[0_0_8px_rgba(16,185,129,0.5)]" : "bg-zinc-400"
                }`}
              />
            )}
          </div>

          <nav className="space-y-1">
            <button
              type="button"
              onClick={() => setTab("translate")}
              className={`w-full h-9 px-3 rounded-lg flex items-center gap-2.5 font-medium transition-colors cursor-pointer ${
                tab === "translate"
                  ? "bg-white dark:bg-zinc-800 text-sky-700 dark:text-sky-300 shadow-xs"
                  : "text-zinc-700 dark:text-zinc-300 hover:bg-black/[0.04] dark:hover:bg-white/[0.05]"
              }`}
            >
              <Languages className="w-4 h-4" />
              <span>对照翻译</span>
            </button>

            <button
              type="button"
              onClick={() => setTab("wordbook")}
              className={`w-full h-9 px-3 rounded-lg flex items-center justify-between font-medium transition-colors cursor-pointer ${
                tab === "wordbook"
                  ? "bg-white dark:bg-zinc-800 text-sky-700 dark:text-sky-300 shadow-xs"
                  : "text-zinc-700 dark:text-zinc-300 hover:bg-black/[0.04] dark:hover:bg-white/[0.05]"
              }`}
            >
              <span className="flex items-center gap-2.5">
                <Bookmark className="w-4 h-4" />
                <span>生词收藏本</span>
              </span>
              {entries.length > 0 && (
                <span className="text-[11px] tabular-nums px-1.5 py-0.2 rounded-full bg-zinc-200/80 dark:bg-zinc-700 text-zinc-700 dark:text-zinc-300">
                  {entries.length}
                </span>
              )}
            </button>

            <button
              type="button"
              onClick={() => setTab("settings")}
              className={`w-full h-9 px-3 rounded-lg flex items-center gap-2.5 font-medium transition-colors cursor-pointer ${
                tab === "settings"
                  ? "bg-white dark:bg-zinc-800 text-sky-700 dark:text-sky-300 shadow-xs"
                  : "text-zinc-700 dark:text-zinc-300 hover:bg-black/[0.04] dark:hover:bg-white/[0.05]"
              }`}
            >
              <Settings2 className="w-4 h-4" />
              <span>偏好设置</span>
            </button>
          </nav>
        </div>

        <div className="space-y-2">
          <button
            type="button"
            onClick={() => invoke("start_region_capture")}
            className="w-full h-9 px-3 rounded-lg border border-zinc-300/80 dark:border-zinc-700/80 bg-white/70 dark:bg-zinc-800/70 hover:bg-white dark:hover:bg-zinc-800 flex items-center justify-center gap-2 text-[12.5px] font-medium cursor-pointer transition-colors"
          >
            <Crop className="w-3.5 h-3.5 text-sky-600 dark:text-sky-400" />
            <span>框选截图翻译</span>
          </button>

          {needsPermWarning && (
            <button
              type="button"
              onClick={() => setTab("settings")}
              className="w-full text-left p-2.5 rounded-lg bg-amber-500/12 border border-amber-500/25 text-amber-800 dark:text-amber-300 text-[11.5px] leading-snug cursor-pointer"
            >
              部分系统取词/截图权限尚未开启，点击前往授权 →
            </button>
          )}
        </div>
      </aside>

      {/* Right Content Workspace */}
      <main className="flex-1 overflow-y-auto p-6">
        {tab === "translate" && (
          <div className="max-w-3xl mx-auto space-y-5">
            <div className="flex items-center justify-between">
              <div>
                <h1 className="text-[18px] font-semibold tracking-tight">手动对照翻译</h1>
                <p className="text-[12.5px] text-zinc-500 dark:text-zinc-400">
                  支持离线词典秒查、百毫秒极速机翻与大模型 SSE 实时流式译入
                </p>
              </div>
              <div className="inline-flex rounded-lg p-0.5 bg-zinc-200/75 dark:bg-zinc-800">
                <button
                  type="button"
                  onClick={() => setManualMode("selection")}
                  className={`px-3 py-1 rounded-md text-[12.5px] font-medium cursor-pointer ${
                    manualMode === "selection"
                      ? "bg-white dark:bg-zinc-900 shadow-xs text-zinc-900 dark:text-zinc-100"
                      : "text-zinc-600 dark:text-zinc-400"
                  }`}
                >
                  精准互译
                </button>
                <button
                  type="button"
                  onClick={() => setManualMode("explanation")}
                  className={`px-3 py-1 rounded-md text-[12.5px] font-medium cursor-pointer ${
                    manualMode === "explanation"
                      ? "bg-white dark:bg-zinc-900 shadow-xs text-zinc-900 dark:text-zinc-100"
                      : "text-zinc-600 dark:text-zinc-400"
                  }`}
                >
                  语境精讲
                </button>
              </div>
            </div>

            <div className="rounded-2xl border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900 shadow-xs overflow-hidden">
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
                placeholder="输入或粘贴英文 / 中文段落（按 ⌘/Ctrl + Enter 立即流式翻译）…"
                className="w-full p-4 bg-transparent resize-none focus:outline-none text-[14px] leading-relaxed select-text"
              />
              <div className="px-4 py-2.5 bg-zinc-50 dark:bg-zinc-900/60 border-t border-zinc-100 dark:border-zinc-800 flex items-center justify-between">
                <span className="text-[11.5px] text-zinc-500 tabular-nums">
                  {inputText.length} / 5000 字符
                </span>
                <div className="flex items-center gap-2">
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
                      className="h-8 px-3 rounded-lg text-[12.5px] text-zinc-600 dark:text-zinc-400 hover:bg-zinc-200/60 dark:hover:bg-zinc-800 cursor-pointer"
                    >
                      清空
                    </button>
                  )}
                  <button
                    type="button"
                    onClick={runManualTranslate}
                    disabled={!inputText.trim() || manualStreaming}
                    className="h-8 px-4 rounded-lg bg-sky-600 hover:bg-sky-500 disabled:opacity-50 text-white text-[12.5px] font-medium inline-flex items-center gap-1.5 cursor-pointer transition-colors"
                  >
                    {manualMode === "explanation" ? (
                      <Sparkles className="w-3.5 h-3.5" />
                    ) : (
                      <Languages className="w-3.5 h-3.5" />
                    )}
                    <span>{manualStreaming ? "生成中…" : manualMode === "explanation" ? "解析语境" : "立即翻译"}</span>
                  </button>
                </div>
              </div>
            </div>

            {(manualDict || manualFast || manualLlm || manualStreaming || manualError) && (
              <div className="rounded-2xl border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900 p-4 space-y-3 shadow-xs">
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-2 text-[12px] text-zinc-500 dark:text-zinc-400">
                    <span>译文结果</span>
                    {manualTtft !== null && (
                      <span className="px-1.5 py-0.5 rounded bg-emerald-500/10 text-emerald-700 dark:text-emerald-300 tabular-nums">
                        首字 {manualTtft}ms
                      </span>
                    )}
                  </div>
                  <div className="flex items-center gap-1.5">
                    <button
                      type="button"
                      onClick={async () => {
                        const out = manualLlm || manualFast || "";
                        if (!out) return;
                        await invoke("popup_copy_text", { text: out });
                        setManualCopied(true);
                        setTimeout(() => setManualCopied(false), 1400);
                      }}
                      className="h-7 px-2.5 rounded-lg border border-zinc-200 dark:border-zinc-700 text-[12px] inline-flex items-center gap-1 hover:bg-zinc-100 dark:hover:bg-zinc-800 cursor-pointer"
                    >
                      {manualCopied ? <Check className="w-3.5 h-3.5 text-emerald-500" /> : <Copy className="w-3.5 h-3.5" />}
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
                      className="h-7 px-2.5 rounded-lg border border-zinc-200 dark:border-zinc-700 text-[12px] inline-flex items-center gap-1 hover:bg-zinc-100 dark:hover:bg-zinc-800 cursor-pointer"
                    >
                      <Bookmark className="w-3.5 h-3.5" />
                      <span>收藏</span>
                    </button>
                  </div>
                </div>

                {manualDict && (
                  <div className="space-y-1 pb-2 border-b border-zinc-100 dark:border-zinc-800">
                    {manualDict.senses.map((s, i) => (
                      <div key={i} className="flex items-baseline gap-2 text-[13.5px]">
                        {s.partOfSpeech && (
                          <span className="font-mono text-[11.5px] px-1.5 py-0.2 rounded bg-zinc-100 dark:bg-zinc-800 text-zinc-600 dark:text-zinc-300">
                            {s.partOfSpeech}
                          </span>
                        )}
                        <span className="select-text">{s.glosses.join("；")}</span>
                      </div>
                    ))}
                  </div>
                )}

                {(manualLlm || manualFast) && (
                  <div className="select-text text-[14.5px] leading-[1.65] whitespace-pre-wrap">
                    {manualLlm || manualFast}
                    {manualStreaming && (
                      <span className="inline-block w-1.5 h-4 ml-0.5 align-middle bg-sky-500 animate-pulse" />
                    )}
                  </div>
                )}

                {manualError && (
                  <div className="text-[12.5px] text-rose-600 dark:text-rose-400">{manualError}</div>
                )}
              </div>
            )}
          </div>
        )}

        {tab === "wordbook" && (
          <div className="max-w-3xl mx-auto space-y-4">
            <div className="flex items-center justify-between gap-4">
              <div>
                <h1 className="text-[18px] font-semibold tracking-tight">生词收藏本</h1>
                <p className="text-[12.5px] text-zinc-500 dark:text-zinc-400">
                  保留查词时的真实原句语境，支持无缝导入旧版 SavedEntries.json
                </p>
              </div>
              <div className="flex items-center gap-2">
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
                  className="h-8 px-3 rounded-lg border border-zinc-300 dark:border-zinc-700 bg-white dark:bg-zinc-900 hover:bg-zinc-100 dark:hover:bg-zinc-800 inline-flex items-center gap-1.5 text-[12.5px] font-medium cursor-pointer"
                >
                  <Upload className="w-3.5 h-3.5" />
                  <span>导入 JSON</span>
                </button>
                <button
                  type="button"
                  onClick={handleExportJson}
                  disabled={entries.length === 0}
                  className="h-8 px-3 rounded-lg border border-zinc-300 dark:border-zinc-700 bg-white dark:bg-zinc-900 hover:bg-zinc-100 dark:hover:bg-zinc-800 disabled:opacity-50 inline-flex items-center gap-1.5 text-[12.5px] font-medium cursor-pointer"
                >
                  <Download className="w-3.5 h-3.5" />
                  <span>导出备份</span>
                </button>
              </div>
            </div>

            {wordbookNotice && (
              <div className="px-3.5 py-2 rounded-xl bg-sky-500/10 border border-sky-500/20 text-sky-800 dark:text-sky-300 text-[12.5px]">
                {wordbookNotice}
              </div>
            )}

            <div className="relative">
              <Search className="w-4 h-4 text-zinc-400 absolute left-3.5 top-1/2 -translate-y-1/2" />
              <input
                type="search"
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                placeholder="搜索单词、释义或原句语境…"
                className="w-full h-9 pl-9 pr-3 rounded-xl border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900 focus:outline-none focus:border-sky-500 text-[13px]"
              />
            </div>

            {filteredEntries.length === 0 ? (
              <div className="rounded-2xl border border-dashed border-zinc-300 dark:border-zinc-800 p-10 text-center text-zinc-500 dark:text-zinc-400">
                {entries.length === 0
                  ? "暂无收藏词条。悬停查词后点击浮窗上的书签图标即可收藏，或点击右上角导入旧版 JSON。"
                  : "没有匹配的搜索结果"}
              </div>
            ) : (
              <div className="space-y-2.5">
                {filteredEntries.map((item) => (
                  <div
                    key={item.id}
                    className="rounded-xl border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900 p-3.5 flex items-start justify-between gap-4"
                  >
                    <div className="space-y-1 min-w-0 select-text">
                      <div className="flex items-baseline gap-2.5 flex-wrap">
                        <span className="text-[15px] font-semibold text-zinc-900 dark:text-zinc-50">
                          {item.source}
                        </span>
                        <span className="text-[13.5px] text-sky-700 dark:text-sky-300">
                          {item.translation}
                        </span>
                      </div>
                      {item.context && (
                        <div className="text-[12.5px] text-zinc-500 dark:text-zinc-400 leading-relaxed">
                          {item.context}
                        </div>
                      )}
                    </div>
                    <button
                      type="button"
                      title="删除词条"
                      aria-label="删除词条"
                      onClick={async () => {
                        await invoke("delete_saved_entry", { id: item.id });
                        setEntries((prev) => prev.filter((x) => x.id !== item.id));
                      }}
                      className="w-7 h-7 shrink-0 rounded-lg inline-flex items-center justify-center text-zinc-400 hover:text-rose-600 hover:bg-rose-500/10 cursor-pointer"
                    >
                      <Trash2 className="w-3.5 h-3.5" />
                    </button>
                  </div>
                ))}
              </div>
            )}
          </div>
        )}

        {tab === "settings" && settings && activeEndpoint && (
          <div className="max-w-3xl mx-auto space-y-6 pb-8">
            <div className="flex items-center justify-between">
              <div>
                <h1 className="text-[18px] font-semibold tracking-tight">偏好设置</h1>
                <p className="text-[12.5px] text-zinc-500 dark:text-zinc-400">
                  所有配置原子落盘，API Key 独立加密存储于系统密钥环 (Keychain / Credential Manager)
                </p>
              </div>
              {settingsSavedBanner && (
                <span className="text-[12px] text-emerald-600 dark:text-emerald-400 font-medium">
                  ✓ 已自动保存
                </span>
              )}
            </div>

            {/* Section 1: OS Permissions */}
            {permissions && permissions.platform === "macos" && (
              <section className="rounded-2xl border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900 p-4 space-y-3">
                <div className="flex items-center gap-2 font-semibold text-[14px]">
                  <ShieldCheck className="w-4 h-4 text-sky-600 dark:text-sky-400" />
                  <span>macOS 系统权限状态</span>
                </div>
                <div className="grid grid-cols-2 gap-3">
                  <div className="p-3 rounded-xl bg-zinc-50 dark:bg-zinc-800/60 flex items-center justify-between">
                    <div>
                      <div className="font-medium text-[13px]">辅助功能 (Accessibility)</div>
                      <div className="text-[11.5px] text-zinc-500">用于全局修饰键监听与无障碍取词</div>
                    </div>
                    {permissions.accessibility ? (
                      <span className="text-[12px] text-emerald-600 dark:text-emerald-400 font-medium">已就绪</span>
                    ) : (
                      <button
                        type="button"
                        onClick={async () => {
                          const p = await invoke<PermissionsStatus>("get_permissions_status", { prompt: true });
                          setPermissions(p);
                        }}
                        className="h-7 px-2.5 rounded-lg bg-sky-600 text-white text-[12px] cursor-pointer"
                      >
                        请求授权
                      </button>
                    )}
                  </div>
                  <div className="p-3 rounded-xl bg-zinc-50 dark:bg-zinc-800/60 flex items-center justify-between">
                    <div>
                      <div className="font-medium text-[13px]">屏幕录制 (Screen Recording)</div>
                      <div className="text-[11.5px] text-zinc-500">用于 PDF/图片离线 Vision OCR 兜底</div>
                    </div>
                    <span
                      className={`text-[12px] font-medium ${
                        permissions.screenRecording
                          ? "text-emerald-600 dark:text-emerald-400"
                          : "text-amber-600 dark:text-amber-400"
                      }`}
                    >
                      {permissions.screenRecording ? "已就绪" : "未授权"}
                    </span>
                  </div>
                </div>
              </section>
            )}

            {/* Section 2: Trigger & Interaction */}
            <section className="rounded-2xl border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900 p-4 space-y-4">
              <div className="font-semibold text-[14px]">取词手势与交互</div>

              <div className="grid grid-cols-2 gap-4">
                <label className="space-y-1 block">
                  <span className="text-[12.5px] text-zinc-600 dark:text-zinc-400">悬停触发修饰键</span>
                  <select
                    value={settings.triggerModifier}
                    onChange={(e) =>
                      persistSettings({
                        ...settings,
                        triggerModifier: e.target.value as TriggerModifier,
                      })
                    }
                    className="w-full h-9 px-2.5 rounded-lg border border-zinc-300 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-800"
                  >
                    <option value="altOrOption">Option / Alt（左右皆可）</option>
                    <option value="leftOption">仅左 Option / Left Alt</option>
                    <option value="rightOption">仅右 Option / Right Alt</option>
                    <option value="control">Control (Ctrl)</option>
                  </select>
                </label>

                <label className="space-y-1 block">
                  <span className="text-[12.5px] text-zinc-600 dark:text-zinc-400">
                    悬停响应延迟: <strong className="tabular-nums">{settings.hoverDwellMs} ms</strong>
                  </span>
                  <input
                    type="range"
                    min={80}
                    max={500}
                    step={10}
                    value={settings.hoverDwellMs}
                    onChange={(e) =>
                      persistSettings({ ...settings, hoverDwellMs: Number(e.target.value) })
                    }
                    className="w-full accent-sky-600 mt-2"
                  />
                </label>

                <label className="space-y-1 block">
                  <span className="text-[12.5px] text-zinc-600 dark:text-zinc-400">
                    松键后卡片驻留时长:{" "}
                    <strong className="tabular-nums">{(settings.lingerDismissMs / 1000).toFixed(1)} s</strong>
                  </span>
                  <input
                    type="range"
                    min={1000}
                    max={6000}
                    step={250}
                    value={settings.lingerDismissMs}
                    onChange={(e) =>
                      persistSettings({ ...settings, lingerDismissMs: Number(e.target.value) })
                    }
                    className="w-full accent-sky-600 mt-2"
                  />
                </label>
              </div>

              <div className="space-y-2.5 pt-2 border-t border-zinc-100 dark:border-zinc-800">
                <label className="flex items-center justify-between cursor-pointer">
                  <div>
                    <div className="font-medium text-[13px]">双击修饰键直接翻译当前选区</div>
                    <div className="text-[11.5px] text-zinc-500">
                      选中文字后快速双击 Option/Alt，无论鼠标在何处直接弹出轻量浮窗
                    </div>
                  </div>
                  <input
                    type="checkbox"
                    checked={settings.doubleTapSelection}
                    onChange={(e) =>
                      persistSettings({ ...settings, doubleTapSelection: e.target.checked })
                    }
                    className="w-4 h-4 accent-sky-600"
                  />
                </label>

                <label className="flex items-center justify-between cursor-pointer">
                  <div>
                    <div className="font-medium text-[13px]">启用本地离线 OCR 自动兜底（推荐）</div>
                    <div className="text-[11.5px] text-zinc-500">
                      遇到 PDF、Canvas、图片或无映射控件时零延迟调用系统离线 OCR 并按列智能拼句
                    </div>
                  </div>
                  <input
                    type="checkbox"
                    checked={settings.enableOcrFallback}
                    onChange={(e) =>
                      persistSettings({ ...settings, enableOcrFallback: e.target.checked })
                    }
                    className="w-4 h-4 accent-sky-600"
                  />
                </label>

                <label className="flex items-center justify-between cursor-pointer">
                  <div>
                    <div className="font-medium text-[13px]">选区读取剪贴板备份还原兜底（可选）</div>
                    <div className="text-[11.5px] text-zinc-500">
                      仅当目标软件不支持无障碍选区时，备份原剪贴板 → 模拟复制 → 30ms 内无感还原（密码框严格跳过）
                    </div>
                  </div>
                  <input
                    type="checkbox"
                    checked={settings.enableClipboardFallback}
                    onChange={(e) =>
                      persistSettings({ ...settings, enableClipboardFallback: e.target.checked })
                    }
                    className="w-4 h-4 accent-sky-600"
                  />
                </label>
              </div>
            </section>

            {/* Section 3: Three-Tier Translation Pipeline & Streaming LLM */}
            <section className="rounded-2xl border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900 p-4 space-y-4">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2 font-semibold text-[14px]">
                  <Zap className="w-4 h-4 text-sky-600 dark:text-sky-400" />
                  <span>三级并发翻译引擎配置</span>
                </div>
              </div>

              <div className="space-y-2.5 pb-3 border-b border-zinc-100 dark:border-zinc-800">
                <label className="flex items-center justify-between cursor-pointer">
                  <div>
                    <div className="font-medium text-[13px]">开启百毫秒极速机器翻译预热层 (80~250ms)</div>
                    <div className="text-[11.5px] text-zinc-500">
                      在大模型流式生成前先秒出非推理机器直译，彻底消除空白等待
                    </div>
                  </div>
                  <input
                    type="checkbox"
                    checked={settings.enableFastMt}
                    onChange={(e) => persistSettings({ ...settings, enableFastMt: e.target.checked })}
                    className="w-4 h-4 accent-sky-600"
                  />
                </label>

                <label className="flex items-center justify-between cursor-pointer">
                  <div>
                    <div className="font-medium text-[13px]">纯查单词时仅用「离线词典 + 极速机翻」（零 Token 模式）</div>
                    <div className="text-[11.5px] text-zinc-500">
                      悬停单个词时不消耗大模型额度；点击浮窗 [译整句] 或 [解释] 时仍正常调用大模型
                    </div>
                  </div>
                  <input
                    type="checkbox"
                    checked={settings.wordFastOnly}
                    onChange={(e) => persistSettings({ ...settings, wordFastOnly: e.target.checked })}
                    className="w-4 h-4 accent-sky-600"
                  />
                </label>
              </div>

              <div className="grid grid-cols-3 gap-2">
                {(
                  [
                    { id: "gemini", label: "Google Gemini" },
                    { id: "deepseek", label: "DeepSeek" },
                    { id: "openaiCompatible", label: "OpenAI 兼容接口" },
                  ] as { id: LlmProviderKind; label: string }[]
                ).map((p) => (
                  <button
                    key={p.id}
                    type="button"
                    onClick={() => {
                      setTestResult(null);
                      persistSettings({ ...settings, activeProvider: p.id });
                    }}
                    className={`h-9 rounded-xl border text-[12.5px] font-medium cursor-pointer transition-colors ${
                      settings.activeProvider === p.id
                        ? "border-sky-500 bg-sky-500/10 text-sky-700 dark:text-sky-300"
                        : "border-zinc-200 dark:border-zinc-700 hover:bg-zinc-50 dark:hover:bg-zinc-800"
                    }`}
                  >
                    {p.label}
                  </button>
                ))}
              </div>

              <div className="grid grid-cols-2 gap-3">
                <label className="space-y-1 block">
                  <span className="text-[12px] text-zinc-500">API Base URL</span>
                  <input
                    type="url"
                    value={activeEndpoint.baseUrl}
                    onChange={(e) => updateActiveEndpoint({ baseUrl: e.target.value })}
                    placeholder="https://api.openai.com/v1"
                    className="w-full h-9 px-3 rounded-lg border border-zinc-300 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-800 text-[13px]"
                  />
                </label>

                <label className="space-y-1 block">
                  <span className="text-[12px] text-zinc-500">模型名称 (Model ID)</span>
                  <input
                    type="text"
                    value={activeEndpoint.model}
                    onChange={(e) => updateActiveEndpoint({ model: e.target.value })}
                    placeholder="gemini-2.5-flash-lite / deepseek-chat"
                    className="w-full h-9 px-3 rounded-lg border border-zinc-300 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-800 text-[13px]"
                  />
                </label>
              </div>

              <div className="space-y-1.5">
                <div className="flex items-center justify-between text-[12px]">
                  <span className="text-zinc-500 flex items-center gap-1">
                    <KeyRound className="w-3.5 h-3.5" />
                    <span>
                      API Key 状态:{" "}
                      {keyStatus[settings.activeProvider] ? (
                        <strong className="text-emerald-600 dark:text-emerald-400">已加密保存至系统密钥环</strong>
                      ) : (
                        <strong className="text-amber-600 dark:text-amber-400">尚未配置</strong>
                      )}
                    </span>
                  </span>
                </div>
                <div className="flex items-center gap-2">
                  <input
                    type="password"
                    value={apiKeyDraft}
                    onChange={(e) => setApiKeyDraft(e.target.value)}
                    placeholder={
                      keyStatus[settings.activeProvider]
                        ? "输入新密钥以覆盖更新（留空保持不变）…"
                        : "粘贴当前服务商的 API Key…"
                    }
                    className="flex-1 h-9 px-3 rounded-lg border border-zinc-300 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-800 text-[13px]"
                  />
                  <button
                    type="button"
                    onClick={handleSaveApiKey}
                    disabled={!apiKeyDraft.trim()}
                    className="h-9 px-3.5 rounded-lg bg-zinc-900 dark:bg-zinc-100 text-white dark:text-zinc-900 disabled:opacity-40 text-[12.5px] font-medium cursor-pointer"
                  >
                    保存密钥
                  </button>
                  <button
                    type="button"
                    onClick={handleTestConnection}
                    disabled={testingConn}
                    className="h-9 px-3.5 rounded-lg border border-sky-500/40 bg-sky-500/10 text-sky-700 dark:text-sky-300 text-[12.5px] font-medium inline-flex items-center gap-1.5 cursor-pointer"
                  >
                    <Activity className="w-3.5 h-3.5" />
                    <span>{testingConn ? "测速中…" : "流式连接自检"}</span>
                  </button>
                </div>
              </div>

              {testResult && (
                <div
                  className={`p-3 rounded-xl text-[12.5px] border ${
                    testResult.ok
                      ? "bg-emerald-500/10 border-emerald-500/25 text-emerald-800 dark:text-emerald-300"
                      : "bg-rose-500/10 border-rose-500/25 text-rose-700 dark:text-rose-300"
                  }`}
                >
                  {testResult.ok ? (
                    <span>
                      ✓ 流式连接正常 · 首字响应 (TTFT) <strong>{testResult.ttftMs} ms</strong> · 总耗时{" "}
                      <strong>{testResult.totalMs} ms</strong> · 示例输出：「{testResult.sampleOutput}」
                    </span>
                  ) : (
                    <span>✕ 连接失败：{testResult.error}</span>
                  )}
                </div>
              )}
            </section>

            {/* Section 4: Excluded Applications */}
            <section className="rounded-2xl border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900 p-4 space-y-3">
              <div>
                <div className="font-semibold text-[14px]">排除应用列表</div>
                <div className="text-[12px] text-zinc-500">
                  在这些应用中将自动跳过悬停与选区取词（macOS 填 Bundle ID，Windows 填进程名如 KeePassXC.exe）
                </div>
              </div>

              <div className="flex flex-wrap gap-2">
                {settings.excludedApps.map((appId) => (
                  <span
                    key={appId}
                    className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-lg bg-zinc-100 dark:bg-zinc-800 text-[12px] font-mono"
                  >
                    <span>{appId}</span>
                    <button
                      type="button"
                      onClick={() =>
                        persistSettings({
                          ...settings,
                          excludedApps: settings.excludedApps.filter((x) => x !== appId),
                        })
                      }
                      className="text-zinc-400 hover:text-rose-500 cursor-pointer"
                    >
                      ×
                    </button>
                  </span>
                ))}
              </div>

              <div className="flex items-center gap-2">
                <input
                  type="text"
                  value={newExcludedApp}
                  onChange={(e) => setNewExcludedApp(e.target.value)}
                  placeholder="例如 com.apple.Terminal 或 code.exe"
                  className="flex-1 h-8 px-3 rounded-lg border border-zinc-300 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-800 text-[12.5px] font-mono"
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
                  className="h-8 px-3 rounded-lg border border-zinc-300 dark:border-zinc-700 hover:bg-zinc-100 dark:hover:bg-zinc-800 inline-flex items-center gap-1 text-[12.5px] cursor-pointer"
                >
                  <Plus className="w-3.5 h-3.5" />
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
