import { useRef, useState } from "react";
import { Badge } from "@astryxdesign/core/Badge";
import { Banner } from "@astryxdesign/core/Banner";
import { Button } from "@astryxdesign/core/Button";
import { Card } from "@astryxdesign/core/Card";
import { Heading } from "@astryxdesign/core/Heading";
import { HStack } from "@astryxdesign/core/HStack";
import { Kbd } from "@astryxdesign/core/Kbd";
import { Layout, LayoutContent } from "@astryxdesign/core/Layout";
import {
  SegmentedControl,
  SegmentedControlItem,
} from "@astryxdesign/core/SegmentedControl";
import { Selector } from "@astryxdesign/core/Selector";
import { Text } from "@astryxdesign/core/Text";
import { TextArea } from "@astryxdesign/core/TextArea";
import { useToast } from "@astryxdesign/core/Toast";
import { VStack } from "@astryxdesign/core/VStack";
import { ArrowRight, Bookmark, Check, Copy } from "lucide-react";
import type { QueryMode, WordEntry } from "../../shared/types";
import { appBridge } from "../bridge";

export interface TranslateViewProps {
  targetLanguage: string;
  onTargetLanguageChange: (lang: string) => void;
  inputText: string;
  setInputText: (val: string) => void;
  manualMode: QueryMode;
  setManualMode: (mode: QueryMode) => void;
  manualDict: WordEntry | null;
  manualFast: string | null;
  manualLlm: string;
  manualStreaming: boolean;
  manualTtft: number | null;
  manualError: string | null;
  onRunTranslate: () => void;
  onClearTranslate: () => void;
  onSavedEntry: () => Promise<void>;
}

export function TranslateView({
  targetLanguage,
  onTargetLanguageChange,
  inputText,
  setInputText,
  manualMode,
  setManualMode,
  manualDict,
  manualFast,
  manualLlm,
  manualStreaming,
  manualTtft,
  manualError,
  onRunTranslate,
  onClearTranslate,
  onSavedEntry,
}: TranslateViewProps) {
  const showToast = useToast();
  const [copied, setCopied] = useState(false);
  const [savedJustNow, setSavedJustNow] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);

  const outputText = manualLlm || manualFast || "";
  const hasOutput = Boolean(manualDict || outputText || manualStreaming || manualError);

  const handleCopy = async () => {
    const fallbackDict =
      manualDict?.senses
        .map((s) => `${s.partOfSpeech ? `${s.partOfSpeech} ` : ""}${s.glosses.join("；")}`)
        .join("\n") ?? "";
    const toCopy = outputText || fallbackDict;
    if (!toCopy) return;
    await appBridge.copyText(toCopy);
    setCopied(true);
    showToast({ body: "已复制到剪贴板", uniqueID: "translate-copy", autoHideDuration: 2200 });
    setTimeout(() => setCopied(false), 1500);
  };

  const handleSaveWordbook = async () => {
    const fallbackDict =
      manualDict?.senses
        .map((s) => `${s.partOfSpeech ? `${s.partOfSpeech} ` : ""}${s.glosses.join("；")}`)
        .join("；") ?? "";
    const gloss = (outputText || fallbackDict).trim();
    if (!inputText.trim() || !gloss) return;
    await appBridge.saveManualEntry({
      source: inputText.trim(),
      translation: gloss,
      context: "",
    });
    await onSavedEntry();
    setSavedJustNow(true);
    showToast({ body: "已收入生词本", uniqueID: "translate-save", autoHideDuration: 2200 });
    setTimeout(() => setSavedJustNow(false), 1600);
  };

  return (
    <Layout
      height="fill"
      contentWidth={760}
      content={
        <LayoutContent padding={6}>
          <VStack gap={5}>
            {/* Borderless Contextual Header Row */}
            <HStack justify="between" align="center" gap={3} wrap="wrap">
              <HStack align="center" gap={3}>
                <Heading level={2}>翻译</Heading>
                <div className="flex items-center gap-1.5 text-secondary">
                  <Text type="supporting">自动检测</Text>
                  <ArrowRight className="w-3.5 h-3.5 text-secondary" strokeWidth={1.75} />
                  <Selector
                    label="目标语言"
                    isLabelHidden
                    variant="secondary"
                    size="sm"
                    value={targetLanguage}
                    onChange={onTargetLanguageChange}
                    options={[
                      { value: "简体中文", label: "简体中文" },
                      { value: "繁體中文", label: "繁體中文" },
                      { value: "English", label: "English" },
                      { value: "日本語", label: "日本語" },
                    ]}
                  />
                </div>
              </HStack>

              <SegmentedControl
                size="sm"
                label="输出模式"
                value={manualMode === "explanation" ? "explanation" : "selection"}
                onChange={(val) => setManualMode(val as QueryMode)}
              >
                <SegmentedControlItem value="selection" label="译文" />
                <SegmentedControlItem value="explanation" label="语境解析" />
              </SegmentedControl>
            </HStack>

            {/* Source Input Composer using Native Astryx TextArea */}
            <Card>
              <VStack gap={3}>
                <div
                  onKeyDown={(e) => {
                    if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
                      e.preventDefault();
                      onRunTranslate();
                    }
                  }}
                >
                  <TextArea
                    ref={textareaRef}
                    label="待翻译原文"
                    isLabelHidden
                    rows={5}
                    maxLength={5000}
                    width="100%"
                    value={inputText}
                    onChange={(val) => setInputText(val)}
                    placeholder="输入单词、长句或段落…"
                  />
                </div>

                <HStack justify="between" align="center" gap={3}>
                  <HStack align="center" gap={1.5}>
                    <Text type="supporting">快捷键</Text>
                    <Kbd keys="mod+enter" />
                  </HStack>

                  <HStack align="center" gap={2}>
                    {inputText.length > 0 && (
                      <Button
                        variant="ghost"
                        size="sm"
                        label="清空"
                        onClick={() => {
                          onClearTranslate();
                          textareaRef.current?.focus();
                        }}
                      />
                    )}
                    <Button
                      variant="primary"
                      size="sm"
                      label={manualMode === "explanation" ? "解析" : "翻译"}
                      isLoading={manualStreaming}
                      isDisabled={!inputText.trim()}
                      onClick={onRunTranslate}
                    />
                  </HStack>
                </HStack>
              </VStack>
            </Card>

            {/* Clean Result Surface: Appears only when active, keeping empty state calm & airy */}
            {hasOutput && (
              <Card>
                <VStack gap={3}>
                  <HStack justify="between" align="center" gap={2}>
                    <HStack align="center" gap={2}>
                      <Text type="supporting" weight="medium">
                        {manualMode === "explanation" ? "语境解析" : "译文"}
                      </Text>
                      {manualTtft !== null && (
                        <Text type="supporting" hasTabularNumbers>
                          · 首字响应 {manualTtft} ms
                        </Text>
                      )}
                    </HStack>

                    <HStack align="center" gap={1.5}>
                      <Button
                        variant="ghost"
                        size="sm"
                        icon={
                          copied ? (
                            <Check className="w-3.5 h-3.5" strokeWidth={1.75} />
                          ) : (
                            <Copy className="w-3.5 h-3.5" strokeWidth={1.75} />
                          )
                        }
                        label={copied ? "已复制" : "复制"}
                        isDisabled={!outputText && !manualDict}
                        onClick={handleCopy}
                      />
                      <Button
                        variant="secondary"
                        size="sm"
                        icon={
                          savedJustNow ? (
                            <Check className="w-3.5 h-3.5" strokeWidth={1.75} />
                          ) : (
                            <Bookmark className="w-3.5 h-3.5" strokeWidth={1.75} />
                          )
                        }
                        label={savedJustNow ? "已收藏" : "收藏"}
                        isDisabled={!inputText.trim() || (!outputText.trim() && !manualDict)}
                        onClick={handleSaveWordbook}
                      />
                    </HStack>
                  </HStack>

                  {/* Dictionary Headword & Senses */}
                  {manualDict &&
                    (() => {
                      const firstGloss = manualDict.senses[0]?.glosses.join("；") ?? "";
                      const hasDistinctProse =
                        (Boolean(outputText.trim()) &&
                          outputText.trim() !== firstGloss.trim()) ||
                        manualStreaming;
                      return (
                        <div
                          className={`space-y-2.5 select-text ${
                            hasDistinctProse ? "pb-3.5 border-b border-border" : ""
                          }`}
                        >
                          <div className="lt-serif text-[22px] font-semibold text-primary leading-snug">
                            {manualDict.word}
                          </div>
                          <div className="space-y-1.5">
                            {manualDict.senses.map((s, idx) => (
                              <div key={idx} className="flex items-baseline gap-2.5">
                                {s.partOfSpeech && (
                                  <Badge variant="blue" label={s.partOfSpeech} />
                                )}
                                <span className="text-[14.5px] leading-relaxed text-primary">
                                  {s.glosses.join("；")}
                                </span>
                              </div>
                            ))}
                          </div>
                        </div>
                      );
                    })()}

                  {/* Streaming LLM / Fast Output */}
                  {(() => {
                    const firstGloss = manualDict?.senses[0]?.glosses.join("；") ?? "";
                    const hasDistinctProse =
                      (Boolean(outputText.trim()) && outputText.trim() !== firstGloss.trim()) ||
                      manualStreaming;
                    if (!hasDistinctProse) return null;
                    return (
                      <div className="select-text text-[15px] leading-[1.75] text-primary whitespace-pre-wrap">
                        {outputText}
                        {manualStreaming && (
                          <span
                            aria-hidden="true"
                            className="inline-block w-1.5 h-4 ml-1 align-middle rounded-[2px] bg-[var(--color-accent)] animate-pulse motion-reduce:animate-none"
                          />
                        )}
                      </div>
                    );
                  })()}

                  {manualError && (
                    <Banner
                      status="error"
                      title="翻译失败"
                      description={manualError}
                      collapsible={false}
                    />
                  )}
                </VStack>
              </Card>
            )}
          </VStack>
        </LayoutContent>
      }
    />
  );
}
