import "./main.css";

import React, { useCallback, useEffect, useRef, useState, type SVGProps } from "react";
import ReactDOM from "react-dom/client";
import { AppShell } from "@astryxdesign/core/AppShell";
import {
  SideNav,
  SideNavCollapseButton,
  SideNavHeading,
  SideNavItem,
  SideNavSection,
} from "@astryxdesign/core/SideNav";
import { StatusDot } from "@astryxdesign/core/StatusDot";
import { Text } from "@astryxdesign/core/Text";
import { Theme } from "@astryxdesign/core/theme";
import { ToastViewport } from "@astryxdesign/core/Toast";
import { Tooltip } from "@astryxdesign/core/Tooltip";
import {
  BookOpen,
  Bookmark,
  Languages,
  SlidersHorizontal,
  type LucideIcon,
} from "lucide-react";
import type {
  AppSettings,
  ManualStreamEvent,
  PermissionsStatus,
  QueryMode,
  SavedEntry,
  UsageStatsSummary,
  WordEntry,
} from "../shared/types";
import { appBridge, safeListen } from "./bridge";
import { linuxSessionKind } from "./shortcuts";
import { editorialTheme } from "./theme/built/editorial";
import { OverviewView, type SettingsPane } from "./views/OverviewView";
import { SettingsView } from "./views/SettingsView";
import { TranslateView } from "./views/TranslateView";
import { WordbookView } from "./views/WordbookView";

type ActiveTab = "home" | "translate" | "wordbook" | "settings";

function createStrokeMatchedIcon(IconComp: LucideIcon) {
  const Wrapped = (props: SVGProps<SVGSVGElement>) => (
    <IconComp {...props} strokeWidth={1.75} />
  );
  Wrapped.displayName = `Matched(${IconComp.displayName ?? "Icon"})`;
  return Wrapped;
}

const NavOverviewIcon = createStrokeMatchedIcon(BookOpen);
const NavTranslateIcon = createStrokeMatchedIcon(Languages);
const NavWordbookIcon = createStrokeMatchedIcon(Bookmark);
const NavSettingsIcon = createStrokeMatchedIcon(SlidersHorizontal);

function MainWorkspace() {
  const [tab, setTab] = useState<ActiveTab>("home");
  const [navCollapsed, setNavCollapsed] = useState<boolean>(false);
  const [settingsPane, setSettingsPane] = useState<SettingsPane>("interaction");
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [permissions, setPermissions] = useState<PermissionsStatus | null>(null);
  const [keyStatus, setKeyStatus] = useState<Record<string, boolean>>({});
  const [entries, setEntries] = useState<SavedEntry[]>([]);
  const [usageStats, setUsageStats] = useState<UsageStatsSummary | null>(null);

  // Manual Translate state kept at shell level so switching tabs preserves active work
  const [inputText, setInputText] = useState("");
  const [manualMode, setManualMode] = useState<QueryMode>("selection");
  const [manualDict, setManualDict] = useState<WordEntry | null>(null);
  const [manualFast, setManualFast] = useState<string | null>(null);
  const [manualLlm, setManualLlm] = useState("");
  const [manualStreaming, setManualStreaming] = useState(false);
  const [manualTtft, setManualTtft] = useState<number | null>(null);
  const [manualError, setManualError] = useState<string | null>(null);
  const activeReqId = useRef<string>("");

  const isMac =
    permissions?.platform === "macos" ||
    (typeof navigator !== "undefined" && navigator.userAgent.includes("Mac"));
  const platformId = permissions?.platform ?? "";
  const isLinux =
    platformId.startsWith("linux") ||
    (platformId === "" &&
      typeof navigator !== "undefined" &&
      /Linux/i.test(navigator.userAgent) &&
      !isMac);
  const linuxSession = linuxSessionKind(permissions?.platform);
  const defaultCaptureShortcut = isMac ? "Control+Command+S" : "Alt+Shift+S";
  const defaultSelectionShortcut = isMac ? "Alt+Command+T" : "Alt+Shift+T";

  const refreshEntries = useCallback(async () => {
    const list = await appBridge.listSavedEntries();
    setEntries(list);
  }, []);

  const refreshAll = useCallback(async () => {
    const [s, p, ks, list, stats] = await Promise.all([
      appBridge.getSettings(),
      appBridge.getPermissionsStatus(false),
      appBridge.getApiKeyStatus(),
      appBridge.listSavedEntries(),
      appBridge.getUsageStats(),
    ]);
    setSettings(s);
    setPermissions(p);
    setKeyStatus(ks);
    setEntries(list);
    setUsageStats(stats);
  }, []);

  useEffect(() => {
    refreshAll().catch(() => {});

    const unlistenNav = safeListen<string>("main://navigate", (ev) => {
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

    const unlistenSettings = safeListen<AppSettings>("settings://changed", (ev) => {
      setSettings(ev.payload);
    });

    const unlistenWordbook = safeListen("wordbook://changed", () => {
      refreshEntries().catch(() => {});
    });

    const unlistenStats = safeListen<UsageStatsSummary>("stats://updated", (ev) => {
      setUsageStats(ev.payload);
    });

    const unlistenManual = safeListen<ManualStreamEvent>("manual://progress", (ev) => {
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
      appBridge
        .getPermissionsStatus(false)
        .then(setPermissions)
        .catch(() => {});
      appBridge
        .getUsageStats()
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
  }, [refreshAll, refreshEntries]);

  // Standard macOS window navigation shortcuts: ⌘1 / ⌘2 / ⌘3 / ⌘, / ⌘\
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const mod = isMac ? e.metaKey : e.ctrlKey;
      if (!mod || e.altKey || e.shiftKey) return;
      if (e.key === ",") {
        e.preventDefault();
        setTab("settings");
      } else if (e.key === "1") {
        e.preventDefault();
        setTab("home");
      } else if (e.key === "2") {
        e.preventDefault();
        setTab("translate");
      } else if (e.key === "3") {
        e.preventDefault();
        setTab("wordbook");
      } else if (e.key === "\\") {
        e.preventDefault();
        setNavCollapsed((prev) => !prev);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [isMac]);

  const persistSettings = useCallback(async (next: AppSettings) => {
    setSettings(next);
    const saved = await appBridge.saveSettings(next);
    setSettings(saved);
  }, []);

  const runManualTranslate = useCallback(async () => {
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
      await appBridge.manualTranslateStream({
        requestId: reqId,
        text: trimmed,
        mode: manualMode,
      });
    } catch (err) {
      setManualStreaming(false);
      setManualError(String(err));
    }
  }, [inputText, manualMode]);

  const translateCurrentSelection = useCallback(async () => {
    await appBridge.translateCurrentSelection();
  }, []);

  const clearManualTranslate = useCallback(() => {
    activeReqId.current = "";
    setInputText("");
    setManualDict(null);
    setManualFast(null);
    setManualLlm("");
    setManualTtft(null);
    setManualError(null);
    setManualStreaming(false);
  }, []);

  const openSettingsPane = useCallback((pane?: SettingsPane) => {
    if (pane) setSettingsPane(pane);
    setTab("settings");
  }, []);

  const needsMacAuth = Boolean(
    permissions &&
      permissions.platform === "macos" &&
      (!permissions.accessibility || !permissions.screenRecording)
  );

  return (
    <AppShell
      variant="wash"
      height="fill"
      contentPadding={0}
      mobileNav={{ breakpoint: "none" }}
      sideNav={
        <SideNav
          aria-label="主导航"
          collapsible={{
            isCollapsed: navCollapsed,
            onCollapsedChange: setNavCollapsed,
            hasButton: false,
            buttonLabel: navCollapsed ? "展开侧边栏" : "收起侧边栏",
          }}
          style={navCollapsed ? undefined : { width: 232 }}
          header={
            <SideNavHeading
              heading="Light Translate"
              icon={
                <Languages
                  className="w-4 h-4 text-[var(--color-icon-accent)]"
                  strokeWidth={1.75}
                />
              }
              headerEndContent={
                <div className="flex items-center gap-0.5 -mr-1.5">
                  {settings && (
                    <Tooltip
                      placement="below"
                      content={
                        isLinux
                          ? settings.enabled
                            ? "选区翻译已启用（点击暂停）"
                            : "选区翻译已暂停（点击开启）"
                          : settings.enabled
                            ? "全局取词运行中（点击暂停）"
                            : "全局取词已暂停（点击开启）"
                      }
                    >
                      <button
                        type="button"
                        aria-label={
                          isLinux
                            ? settings.enabled
                              ? "暂停选区翻译"
                              : "开启选区翻译"
                            : settings.enabled
                              ? "暂停全局取词"
                              : "开启全局取词"
                        }
                        aria-pressed={settings.enabled}
                        onClick={() =>
                          persistSettings({ ...settings, enabled: !settings.enabled })
                        }
                        className="w-5 h-6 rounded-[var(--radius-element)] inline-flex items-center justify-center hover:bg-[var(--color-overlay-hover)] cursor-pointer transition-colors"
                      >
                        <StatusDot
                          variant={settings.enabled ? "success" : "neutral"}
                          label={
                            isLinux
                              ? settings.enabled
                                ? "选区翻译已启用"
                                : "选区翻译已暂停"
                              : settings.enabled
                                ? "全局取词运行中"
                                : "全局取词已暂停"
                          }
                        />
                      </button>
                    </Tooltip>
                  )}
                  <SideNavCollapseButton size="sm" />
                </div>
              }
            />
          }
          footer={
            <SideNavItem
              label="设置"
              icon={NavSettingsIcon}
              isSelected={tab === "settings"}
              onClick={() => {
                if (needsMacAuth) setSettingsPane("system");
                setTab("settings");
              }}
              endContent={
                needsMacAuth ? (
                  <StatusDot variant="warning" label="待开启系统授权" />
                ) : undefined
              }
            />
          }
          footerIcons={navCollapsed ? <SideNavCollapseButton size="sm" /> : undefined}
        >
          <SideNavSection title="主菜单" isHeaderHidden>
            <SideNavItem
              label="概览"
              icon={NavOverviewIcon}
              isSelected={tab === "home"}
              onClick={() => setTab("home")}
            />
            <SideNavItem
              label="翻译"
              icon={NavTranslateIcon}
              isSelected={tab === "translate"}
              onClick={() => setTab("translate")}
            />
            <SideNavItem
              label="生词本"
              icon={NavWordbookIcon}
              isSelected={tab === "wordbook"}
              onClick={() => setTab("wordbook")}
              endContent={
                entries.length > 0 ? (
                  <Text type="supporting" hasTabularNumbers>
                    {entries.length}
                  </Text>
                ) : undefined
              }
            />
          </SideNavSection>
        </SideNav>
      }
    >
      {/* Variant A: Elevated rounded cotton sheet resting over soft warm-cotton shell */}
      <div className="h-full p-2 pl-0">
        <div className="h-full rounded-[var(--radius-page)] border border-border bg-[var(--color-background-surface)] shadow-2xs overflow-hidden">
          {tab === "home" && (
            <OverviewView
              settings={settings}
              usageStats={usageStats}
              entries={entries}
              isMac={isMac}
              isLinux={isLinux}
              linuxSession={linuxSession}
              defaultCaptureShortcut={defaultCaptureShortcut}
              onNavigateTranslate={() => setTab("translate")}
              onNavigateWordbook={() => setTab("wordbook")}
              onNavigateSettings={openSettingsPane}
            />
          )}

          {tab === "translate" && (
            <TranslateView
              targetLanguage={settings?.targetLanguage ?? "简体中文"}
              onTargetLanguageChange={(lang) => {
                if (settings) {
                  persistSettings({ ...settings, targetLanguage: lang });
                }
              }}
              inputText={inputText}
              setInputText={setInputText}
              manualMode={manualMode}
              setManualMode={setManualMode}
              manualDict={manualDict}
              manualFast={manualFast}
              manualLlm={manualLlm}
              manualStreaming={manualStreaming}
              manualTtft={manualTtft}
              manualError={manualError}
              onRunTranslate={runManualTranslate}
              onClearTranslate={clearManualTranslate}
              onSavedEntry={refreshEntries}
              isLinux={isLinux}
              linuxSession={linuxSession}
              onTranslateSelection={translateCurrentSelection}
            />
          )}

          {tab === "wordbook" && (
            <WordbookView
              entries={entries}
              onRefreshEntries={refreshEntries}
              onNavigateTranslate={() => setTab("translate")}
            />
          )}

          {tab === "settings" && settings && (
            <SettingsView
              settings={settings}
              permissions={permissions}
              keyStatus={keyStatus}
              isMac={isMac}
              isLinux={isLinux}
              linuxSession={linuxSession}
              defaultCaptureShortcut={defaultCaptureShortcut}
              defaultSelectionShortcut={defaultSelectionShortcut}
              pane={settingsPane}
              onPaneChange={setSettingsPane}
              onPersistSettings={persistSettings}
              onPermissionsUpdated={setPermissions}
              onKeyStatusUpdated={setKeyStatus}
            />
          )}
        </div>
      </div>
    </AppShell>
  );
}

function AppRoot() {
  const [systemDark, setSystemDark] = useState<boolean>(() =>
    typeof window !== "undefined" && typeof window.matchMedia === "function"
      ? window.matchMedia("(prefers-color-scheme: dark)").matches
      : false
  );

  useEffect(() => {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") return;
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = (e: MediaQueryListEvent) => setSystemDark(e.matches);
    setSystemDark(mq.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);

  return (
    <Theme theme={editorialTheme} mode={systemDark ? "dark" : "light"}>
      <ToastViewport />
      <MainWorkspace />
    </Theme>
  );
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <AppRoot />
  </React.StrictMode>
);
