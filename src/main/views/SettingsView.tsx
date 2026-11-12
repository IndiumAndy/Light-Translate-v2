import React, { useEffect, useState } from "react";
import { Banner } from "@astryxdesign/core/Banner";
import { Button } from "@astryxdesign/core/Button";
import { Card } from "@astryxdesign/core/Card";
import { Heading } from "@astryxdesign/core/Heading";
import { HStack } from "@astryxdesign/core/HStack";
import { IconButton } from "@astryxdesign/core/IconButton";
import { Kbd } from "@astryxdesign/core/Kbd";
import { Layout, LayoutContent } from "@astryxdesign/core/Layout";
import {
  SegmentedControl,
  SegmentedControlItem,
} from "@astryxdesign/core/SegmentedControl";
import { Selector } from "@astryxdesign/core/Selector";
import { Slider } from "@astryxdesign/core/Slider";
import { StatusDot } from "@astryxdesign/core/StatusDot";
import { Switch } from "@astryxdesign/core/Switch";
import { Tab, TabList } from "@astryxdesign/core/TabList";
import { Text } from "@astryxdesign/core/Text";
import { TextInput } from "@astryxdesign/core/TextInput";
import { useToast } from "@astryxdesign/core/Toast";
import { Token } from "@astryxdesign/core/Token";
import { VStack } from "@astryxdesign/core/VStack";
import { RotateCcw, X } from "lucide-react";
import type {
  AppSettings,
  ConnectionTestResult,
  LlmProviderKind,
  PermissionsStatus,
  TriggerModifier,
} from "../../shared/types";
import { appBridge } from "../bridge";
import {
  extractShortcutFromEvent,
  formatShortcutInline,
  toAstryxKbdKeys,
} from "../shortcuts";
import type { SettingsPane } from "./OverviewView";

type ShortcutTarget = "capture" | "selection";

function SettingsGroup({
  title,
  description,
  children,
}: {
  title: string;
  description: string;
  children: React.ReactNode;
}) {
  return (
    <section className="min-w-0">
      <VStack gap={2}>
        <div className="space-y-0.5">
          <Heading level={4}>{title}</Heading>
          <Text type="supporting" as="p">
            {description}
          </Text>
        </div>
        {children}
      </VStack>
    </section>
  );
}

function GroupedRowsCard({ children }: { children: React.ReactNode }) {
  return (
    <Card padding={0}>
      <div className="px-4 divide-y divide-border">{children}</div>
    </Card>
  );
}

function SettingSwitchRow({
  title,
  description,
  checked,
  onChange,
}: {
  title: string;
  description: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <div className="py-3.5 flex items-center justify-between gap-6">
      <div className="min-w-0 space-y-0.5">
        <div className="text-[14px] font-medium text-primary leading-snug">{title}</div>
        <Text type="supporting">{description}</Text>
      </div>
      <div className="shrink-0">
        <Switch size="sm" label={title} isLabelHidden value={checked} onChange={onChange} />
      </div>
    </div>
  );
}

export interface SettingsViewProps {
  settings: AppSettings;
  permissions: PermissionsStatus | null;
  keyStatus: Record<string, boolean>;
  isMac: boolean;
  defaultCaptureShortcut: string;
  defaultSelectionShortcut: string;
  pane: SettingsPane;
  onPaneChange: (pane: SettingsPane) => void;
  onPersistSettings: (next: AppSettings) => Promise<void>;
  onPermissionsUpdated: (p: PermissionsStatus) => void;
  onKeyStatusUpdated: (ks: Record<string, boolean>) => void;
}

export function SettingsView({
  settings,
  permissions,
  keyStatus,
  isMac,
  defaultCaptureShortcut,
  defaultSelectionShortcut,
  pane,
  onPaneChange,
  onPersistSettings,
  onPermissionsUpdated,
  onKeyStatusUpdated,
}: SettingsViewProps) {
  const showToast = useToast();

  // Shortcut recording state
  const [recordingTarget, setRecordingTarget] = useState<ShortcutTarget | null>(null);
  const [recordingPreviewKbd, setRecordingPreviewKbd] = useState<string | null>(null);
  const [shortcutNotice, setShortcutNotice] = useState<string | null>(null);

  // Provider & connection state
  const [apiKeyDraft, setApiKeyDraft] = useState("");
  const [testingConn, setTestingConn] = useState(false);
  const [testResult, setTestResult] = useState<ConnectionTestResult | null>(null);

  // Excluded apps state
  const [newExcludedApp, setNewExcludedApp] = useState("");

  useEffect(() => {
    if (pane !== "system") return;
    const timer = window.setInterval(() => {
      appBridge
        .getPermissionsStatus(false)
        .then(onPermissionsUpdated)
        .catch(() => {});
    }, 2500);
    return () => window.clearInterval(timer);
  }, [pane, onPermissionsUpdated]);

  useEffect(() => {
    if (pane !== "interaction") {
      setRecordingTarget(null);
      setShortcutNotice(null);
    }
  }, [pane]);

  useEffect(() => {
    appBridge.setShortcutRecording(recordingTarget !== null).catch(() => {});
    if (!recordingTarget) {
      setRecordingPreviewKbd(null);
      return;
    }

    const onKeyDown = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();

      if (e.code === "Escape") {
        setRecordingTarget(null);
        setRecordingPreviewKbd(null);
        setShortcutNotice(null);
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
        setRecordingPreviewKbd(null);
        setShortcutNotice(null);
        onPersistSettings(next).catch(() => {});
        return;
      }

      const parsed = extractShortcutFromEvent(e, isMac);
      setRecordingPreviewKbd(parsed.previewKbd);
      if (parsed.error) {
        setShortcutNotice(parsed.error);
        return;
      }
      if (!parsed.canonical) return;

      const otherBound =
        recordingTarget === "capture" ? settings.selectionShortcut : settings.captureShortcut;
      if (
        otherBound &&
        formatShortcutInline(otherBound, isMac) === formatShortcutInline(parsed.canonical, isMac)
      ) {
        setShortcutNotice(
          recordingTarget === "capture"
            ? "该组合键已绑定至「划词翻译」"
            : "该组合键已绑定至「区域截屏」"
        );
        return;
      }

      const next: AppSettings =
        recordingTarget === "capture"
          ? { ...settings, captureShortcut: parsed.canonical }
          : { ...settings, selectionShortcut: parsed.canonical };
      setRecordingTarget(null);
      setRecordingPreviewKbd(null);
      setShortcutNotice(null);
      onPersistSettings(next).catch(() => {});
      showToast({
        body: "快捷键已更新",
        uniqueID: "shortcut-saved",
        autoHideDuration: 1800,
      });
    };

    const onKeyUp = (e: KeyboardEvent) => {
      const parsed = extractShortcutFromEvent(e, isMac);
      setRecordingPreviewKbd(parsed.previewKbd);
    };

    const onBlur = () => {
      setRecordingTarget(null);
      setRecordingPreviewKbd(null);
    };

    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("keyup", onKeyUp, true);
    window.addEventListener("blur", onBlur);

    return () => {
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("keyup", onKeyUp, true);
      window.removeEventListener("blur", onBlur);
      appBridge.setShortcutRecording(false).catch(() => {});
    };
  }, [recordingTarget, settings, isMac, onPersistSettings, showToast]);

  const activeEndpoint =
    settings.activeProvider === "gemini"
      ? settings.gemini
      : settings.activeProvider === "deepseek"
        ? settings.deepseek
        : settings.openaiCompatible;

  const updateActiveEndpoint = (patch: Partial<{ baseUrl: string; model: string }>) => {
    const updated = { ...activeEndpoint, ...patch };
    const next: AppSettings =
      settings.activeProvider === "gemini"
        ? { ...settings, gemini: updated }
        : settings.activeProvider === "deepseek"
          ? { ...settings, deepseek: updated }
          : { ...settings, openaiCompatible: updated };
    onPersistSettings(next).catch(() => {});
  };

  const handleSaveApiKey = async () => {
    if (!apiKeyDraft.trim()) return;
    await appBridge.setApiKey(settings.activeProvider, apiKeyDraft.trim());
    setApiKeyDraft("");
    const ks = await appBridge.getApiKeyStatus();
    onKeyStatusUpdated(ks);
    showToast({
      body: "密钥已保存",
      uniqueID: "apikey-saved",
      autoHideDuration: 2200,
    });
  };

  const handleTestConnection = async () => {
    setTestingConn(true);
    setTestResult(null);
    try {
      const res = await appBridge.testProviderConnection({
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

  const handleAddExcludedApp = () => {
    const clean = newExcludedApp.trim();
    if (!clean) return;
    if (settings.excludedApps.includes(clean)) {
      setNewExcludedApp("");
      return;
    }
    setNewExcludedApp("");
    onPersistSettings({
      ...settings,
      excludedApps: [...settings.excludedApps, clean],
    }).catch(() => {});
  };

  const needsMacAuth =
    permissions &&
    permissions.platform === "macos" &&
    (!permissions.accessibility || !permissions.screenRecording);

  const hasSavedKey = Boolean(keyStatus[settings.activeProvider]);

  return (
    <Layout
      height="fill"
      contentWidth={680}
      content={
        <LayoutContent padding={6}>
          <div className="pb-6">
            <VStack gap={5}>
              {/* Single-Layer Title + TabList (Eliminates 100px stacked double header) */}
              <VStack gap={3}>
                <Heading level={2}>设置</Heading>

                <TabList
                  value={pane}
                  onChange={(val) => onPaneChange(val as SettingsPane)}
                  hasDivider
                  role="tablist"
                >
                  <Tab
                    value="interaction"
                    label="取词与快捷键"
                    panelId="settings-panel-interaction"
                  />
                  <Tab value="engine" label="翻译引擎" panelId="settings-panel-engine" />
                  <Tab
                    value="system"
                    label={needsMacAuth ? "权限与忽略 · 待开启" : "权限与忽略"}
                    panelId="settings-panel-system"
                  />
                </TabList>
              </VStack>

              {pane === "interaction" && (
                <div id="settings-panel-interaction" role="tabpanel">
                  <VStack gap={6}>
                    <SettingsGroup
                      title="全局快捷键"
                      description="在任意应用中唤起划词或区域取词。录制时按 Esc 取消，按 Delete 解绑。"
                    >
                      <VStack gap={3}>
                        <GroupedRowsCard>
                          {(
                            [
                              {
                                target: "selection" as ShortcutTarget,
                                title: "划词翻译",
                                subtitle: "翻译当前选中的文字",
                                value: settings.selectionShortcut,
                                defaultValue: defaultSelectionShortcut,
                                onChange: (val: string) =>
                                  onPersistSettings({ ...settings, selectionShortcut: val }),
                              },
                              {
                                target: "capture" as ShortcutTarget,
                                title: "区域截屏",
                                subtitle: "框选屏幕区域识别并翻译",
                                value: settings.captureShortcut,
                                defaultValue: defaultCaptureShortcut,
                                onChange: (val: string) =>
                                  onPersistSettings({ ...settings, captureShortcut: val }),
                              },
                            ] as const
                          ).map((item) => {
                            const isRec = recordingTarget === item.target;
                            const kbdKeys = toAstryxKbdKeys(item.value);
                            const isCustom = item.value !== item.defaultValue;

                            return (
                              <div
                                key={item.target}
                                className="py-3.5 flex items-center justify-between gap-6"
                              >
                                <div className="min-w-0 space-y-0.5">
                                  <div className="text-[14px] font-medium text-primary">
                                    {item.title}
                                  </div>
                                  <Text type="supporting">{item.subtitle}</Text>
                                </div>

                                <HStack align="center" gap={1.5}>
                                  <button
                                    type="button"
                                    onClick={() => {
                                      setShortcutNotice(null);
                                      setRecordingPreviewKbd(null);
                                      setRecordingTarget(isRec ? null : item.target);
                                    }}
                                    className={`h-8 px-3 rounded-[var(--radius-element)] border inline-flex items-center gap-1.5 text-[13px] font-medium cursor-pointer transition-colors ${
                                      isRec
                                        ? "border-[var(--color-accent)] bg-[var(--color-accent-muted)] text-[var(--color-text-accent)]"
                                        : "border-border bg-[var(--color-background-surface)] hover:border-[var(--color-border-emphasized)] text-primary"
                                    }`}
                                  >
                                    {isRec ? (
                                      recordingPreviewKbd ? (
                                        <>
                                          <Kbd keys={recordingPreviewKbd} />
                                          <span className="text-secondary">…</span>
                                        </>
                                      ) : (
                                        <span>按下组合键…</span>
                                      )
                                    ) : kbdKeys ? (
                                      <Kbd keys={kbdKeys} />
                                    ) : (
                                      <span className="text-secondary">点击绑定</span>
                                    )}
                                  </button>

                                  {item.value && !isRec && (
                                    <IconButton
                                      variant="ghost"
                                      size="sm"
                                      label={`解绑${item.title}快捷键`}
                                      tooltip="解绑快捷键"
                                      icon={<X className="w-3.5 h-3.5" strokeWidth={1.75} />}
                                      onClick={() => {
                                        setShortcutNotice(null);
                                        item.onChange("");
                                      }}
                                    />
                                  )}

                                  {isCustom && !isRec && (
                                    <IconButton
                                      variant="ghost"
                                      size="sm"
                                      label={`恢复默认 (${formatShortcutInline(
                                        item.defaultValue,
                                        isMac
                                      )})`}
                                      tooltip={`恢复默认 (${formatShortcutInline(
                                        item.defaultValue,
                                        isMac
                                      )})`}
                                      icon={<RotateCcw className="w-3.5 h-3.5" strokeWidth={1.75} />}
                                      onClick={() => {
                                        setShortcutNotice(null);
                                        item.onChange(item.defaultValue);
                                      }}
                                    />
                                  )}
                                </HStack>
                              </div>
                            );
                          })}
                        </GroupedRowsCard>

                        {shortcutNotice && (
                          <Banner
                            status="warning"
                            title={shortcutNotice}
                            isDismissable
                            onDismiss={() => setShortcutNotice(null)}
                            collapsible={false}
                          />
                        )}
                      </VStack>
                    </SettingsGroup>

                    <SettingsGroup
                      title="悬停与浮窗"
                      description="调整悬停触发键、响应延迟与浮窗收起方式。"
                    >
                      <GroupedRowsCard>
                        <div className="py-3.5 flex items-center justify-between gap-6">
                          <div className="min-w-0 space-y-0.5">
                            <div className="text-[14px] font-medium text-primary">悬停触发键</div>
                            <Text type="supporting">按住该修饰键将光标移至文字即可取词</Text>
                          </div>
                          <div className="w-48 shrink-0">
                            <Selector
                              label="悬停触发键"
                              isLabelHidden
                              size="sm"
                              width="100%"
                              value={settings.triggerModifier}
                              onChange={(val) =>
                                onPersistSettings({
                                  ...settings,
                                  triggerModifier: val as TriggerModifier,
                                })
                              }
                              options={[
                                {
                                  value: "altOrOption",
                                  label: isMac ? "⌥ Option（双侧）" : "Alt（双侧）",
                                },
                                {
                                  value: "leftOption",
                                  label: isMac ? "左 ⌥ Option" : "左 Alt",
                                },
                                {
                                  value: "rightOption",
                                  label: isMac ? "右 ⌥ Option" : "右 Alt",
                                },
                                {
                                  value: "control",
                                  label: isMac ? "⌃ Control" : "Ctrl",
                                },
                              ]}
                            />
                          </div>
                        </div>

                        <div className="py-3.5">
                          <Slider
                            label="悬停响应延迟"
                            description="光标停留达到该时长后弹出浮窗"
                            width="100%"
                            min={80}
                            max={500}
                            step={10}
                            value={settings.hoverDwellMs}
                            valueDisplay="text"
                            formatValue={(v: number) => `${v} ms`}
                            onChange={(v: number) => {
                              onPersistSettings({ ...settings, hoverDwellMs: v });
                            }}
                          />
                        </div>

                        <SettingSwitchRow
                          title="松开按键即收起浮窗"
                          description="关闭后，松开按键浮窗仍会短暂驻留"
                          checked={settings.dismissOnTriggerRelease}
                          onChange={(checked) =>
                            onPersistSettings({ ...settings, dismissOnTriggerRelease: checked })
                          }
                        />

                        {!settings.dismissOnTriggerRelease && (
                          <div className="py-3.5">
                            <Slider
                              label="浮窗驻留时长"
                              width="100%"
                              min={1000}
                              max={6000}
                              step={250}
                              value={settings.lingerDismissMs}
                              valueDisplay="text"
                              formatValue={(v: number) => `${(v / 1000).toFixed(1)} s`}
                              onChange={(v: number) => {
                                onPersistSettings({ ...settings, lingerDismissMs: v });
                              }}
                            />
                          </div>
                        )}

                        <SettingSwitchRow
                          title="双击修饰键翻译选区"
                          description="选中文本后快速连按两次修饰键触发翻译"
                          checked={settings.doubleTapSelection}
                          onChange={(checked) =>
                            onPersistSettings({ ...settings, doubleTapSelection: checked })
                          }
                        />
                      </GroupedRowsCard>
                    </SettingsGroup>

                    <SettingsGroup
                      title="识别辅助"
                      description="系统无法直接读取文本时，辅助提取光标或选区文字。"
                    >
                      <GroupedRowsCard>
                        <SettingSwitchRow
                          title="视觉取词（OCR）"
                          description="识别图片与扫描文档中的光标处文字"
                          checked={settings.enableOcrFallback}
                          onChange={(checked) =>
                            onPersistSettings({ ...settings, enableOcrFallback: checked })
                          }
                        />

                        <SettingSwitchRow
                          title="剪贴板兼容取词"
                          description="常规接口无法读取选区时，临时借助剪贴板获取文本"
                          checked={settings.enableClipboardFallback}
                          onChange={(checked) =>
                            onPersistSettings({ ...settings, enableClipboardFallback: checked })
                          }
                        />
                      </GroupedRowsCard>
                    </SettingsGroup>
                  </VStack>
                </div>
              )}

              {pane === "engine" && (
                <div id="settings-panel-engine" role="tabpanel">
                  <VStack gap={6}>
                    <SettingsGroup
                      title="响应策略"
                      description="设置默认目标语言、即时预译与词典直查行为。"
                    >
                      <GroupedRowsCard>
                        <div className="py-3.5 flex items-center justify-between gap-6">
                          <div className="min-w-0 space-y-0.5">
                            <div className="text-[14px] font-medium text-primary">默认目标语言</div>
                            <Text type="supporting">屏幕取词与手动翻译的目标语种</Text>
                          </div>
                          <div className="w-44 shrink-0">
                            <Selector
                              label="默认目标语言"
                              isLabelHidden
                              size="sm"
                              width="100%"
                              value={settings.targetLanguage}
                              onChange={(val) =>
                                onPersistSettings({ ...settings, targetLanguage: val })
                              }
                              options={[
                                { value: "简体中文", label: "简体中文" },
                                { value: "繁體中文", label: "繁體中文" },
                                { value: "English", label: "English" },
                                { value: "日本語", label: "日本語" },
                              ]}
                            />
                          </div>
                        </div>

                        <SettingSwitchRow
                          title="即时预译"
                          description="完整译文生成前，先显示快速预译结果"
                          checked={settings.enableFastMt}
                          onChange={(checked) =>
                            onPersistSettings({ ...settings, enableFastMt: checked })
                          }
                        />

                        <SettingSwitchRow
                          title="单词直查词典"
                          description="查询单个单词时仅显示内置词典释义"
                          checked={settings.wordFastOnly}
                          onChange={(checked) =>
                            onPersistSettings({ ...settings, wordFastOnly: checked })
                          }
                        />
                      </GroupedRowsCard>
                    </SettingsGroup>

                    <SettingsGroup
                      title="大模型服务"
                      description="配置模型服务商与接口密钥。"
                    >
                      <VStack gap={3}>
                        <Card>
                          <VStack gap={4}>
                            <SegmentedControl
                              label="模型服务商"
                              layout="fill"
                              size="sm"
                              value={settings.activeProvider}
                              onChange={(val) => {
                                setTestResult(null);
                                onPersistSettings({
                                  ...settings,
                                  activeProvider: val as LlmProviderKind,
                                });
                              }}
                            >
                              <SegmentedControlItem value="gemini" label="Gemini" />
                              <SegmentedControlItem value="deepseek" label="DeepSeek" />
                              <SegmentedControlItem value="openaiCompatible" label="OpenAI 兼容" />
                            </SegmentedControl>

                            <TextInput
                              type="url"
                              label="接口地址"
                              width="100%"
                              value={activeEndpoint.baseUrl}
                              onChange={(val) => updateActiveEndpoint({ baseUrl: val })}
                              placeholder="https://api.openai.com/v1"
                            />

                            <TextInput
                              type="text"
                              label="模型名称"
                              width="100%"
                              value={activeEndpoint.model}
                              onChange={(val) => updateActiveEndpoint({ model: val })}
                              placeholder="gemini-2.5-flash-lite"
                            />

                            <div className="pt-3 border-t border-border space-y-3">
                              <div className="flex items-end gap-2">
                                <div className="flex-1 min-w-0">
                                  <TextInput
                                    type="password"
                                    label="API Key"
                                    width="100%"
                                    value={apiKeyDraft}
                                    onChange={setApiKeyDraft}
                                    onEnter={handleSaveApiKey}
                                    placeholder={
                                      hasSavedKey ? "输入新密钥以替换现有密钥" : "sk-..."
                                    }
                                  />
                                </div>
                                {apiKeyDraft.trim().length > 0 && (
                                  <Button
                                    variant="primary"
                                    size="md"
                                    label="保存"
                                    onClick={handleSaveApiKey}
                                  />
                                )}
                              </div>

                              <HStack justify="between" align="center" gap={2}>
                                <HStack align="center" gap={1.5}>
                                  <StatusDot
                                    variant={hasSavedKey ? "success" : "neutral"}
                                    label={hasSavedKey ? "密钥已保存" : "未配置密钥"}
                                  />
                                  <Text type="supporting">
                                    {hasSavedKey ? "密钥已保存" : "未配置密钥"}
                                  </Text>
                                </HStack>

                                <Button
                                  variant="secondary"
                                  size="sm"
                                  label="测试连接"
                                  isLoading={testingConn}
                                  onClick={handleTestConnection}
                                />
                              </HStack>
                            </div>
                          </VStack>
                        </Card>

                        {testResult && (
                          <Banner
                            status={testResult.ok ? "success" : "error"}
                            title={
                              testResult.ok
                                ? `连接正常 · 首字 ${testResult.ttftMs} ms`
                                : "连接失败"
                            }
                            description={testResult.ok ? undefined : testResult.error ?? undefined}
                            isDismissable
                            onDismiss={() => setTestResult(null)}
                            collapsible={false}
                          />
                        )}
                      </VStack>
                    </SettingsGroup>
                  </VStack>
                </div>
              )}

              {pane === "system" && (
                <div id="settings-panel-system" role="tabpanel">
                  <VStack gap={6}>
                    {permissions && permissions.platform === "macos" && (
                      <SettingsGroup
                        title="macOS 系统授权"
                        description="跨应用取词与屏幕区域识别所需的系统访问许可。"
                      >
                        <GroupedRowsCard>
                          <div className="py-3.5 flex items-center justify-between gap-6">
                            <div className="space-y-0.5">
                              <div className="text-[14px] font-medium text-primary">辅助功能</div>
                              <Text type="supporting">读取光标指向与选中的屏幕文本</Text>
                            </div>
                            {permissions.accessibility ? (
                              <HStack align="center" gap={1.5}>
                                <StatusDot variant="success" label="辅助功能已开启" />
                                <Text type="supporting" color="primary" weight="medium">
                                  已开启
                                </Text>
                              </HStack>
                            ) : (
                              <Button
                                variant="primary"
                                size="sm"
                                label="前往开启"
                                onClick={async () => {
                                  const p = await appBridge.getPermissionsStatus(true);
                                  onPermissionsUpdated(p);
                                }}
                              />
                            )}
                          </div>

                          <div className="py-3.5 flex items-center justify-between gap-6">
                            <div className="space-y-0.5">
                              <div className="text-[14px] font-medium text-primary">屏幕录制</div>
                              <Text type="supporting">用于区域框选翻译与视觉文字识别</Text>
                            </div>
                            {permissions.screenRecording ? (
                              <HStack align="center" gap={1.5}>
                                <StatusDot variant="success" label="屏幕录制已开启" />
                                <Text type="supporting" color="primary" weight="medium">
                                  已开启
                                </Text>
                              </HStack>
                            ) : (
                              <Button
                                variant="primary"
                                size="sm"
                                label="前往开启"
                                onClick={async () => {
                                  const p = await appBridge.requestScreenRecording();
                                  onPermissionsUpdated(p);
                                }}
                              />
                            )}
                          </div>
                        </GroupedRowsCard>
                      </SettingsGroup>
                    )}

                    <SettingsGroup
                      title="忽略应用"
                      description="在这些应用中自动暂停悬停与双击取词。"
                    >
                      <Card>
                        <VStack gap={3}>
                          <div className="flex items-end gap-2">
                            <div className="flex-1 min-w-0">
                              <TextInput
                                label="应用标识"
                                isLabelHidden
                                width="100%"
                                value={newExcludedApp}
                                onChange={setNewExcludedApp}
                                onEnter={handleAddExcludedApp}
                                placeholder={
                                  isMac
                                    ? "输入 Bundle ID，例如 com.apple.Terminal"
                                    : "输入进程名，例如 code.exe"
                                }
                              />
                            </div>
                            <Button
                              variant="secondary"
                              size="md"
                              label="添加"
                              isDisabled={!newExcludedApp.trim()}
                              onClick={handleAddExcludedApp}
                            />
                          </div>

                          {settings.excludedApps.length > 0 ? (
                            <div className="flex flex-wrap gap-2 pt-0.5">
                              {settings.excludedApps.map((appId) => (
                                <Token
                                  key={appId}
                                  label={appId}
                                  onRemove={() =>
                                    onPersistSettings({
                                      ...settings,
                                      excludedApps: settings.excludedApps.filter(
                                        (x) => x !== appId
                                      ),
                                    })
                                  }
                                />
                              ))}
                            </div>
                          ) : (
                            <Text type="supporting">未添加忽略应用。</Text>
                          )}
                        </VStack>
                      </Card>
                    </SettingsGroup>
                  </VStack>
                </div>
              )}
            </VStack>
          </div>
        </LayoutContent>
      }
    />
  );
}
