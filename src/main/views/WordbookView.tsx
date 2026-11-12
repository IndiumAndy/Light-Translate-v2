import React, { useMemo, useRef, useState } from "react";
import { Button } from "@astryxdesign/core/Button";
import { Divider } from "@astryxdesign/core/Divider";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { Heading } from "@astryxdesign/core/Heading";
import { HStack } from "@astryxdesign/core/HStack";
import { IconButton } from "@astryxdesign/core/IconButton";
import { Layout, LayoutContent } from "@astryxdesign/core/Layout";
import { List, ListItem } from "@astryxdesign/core/List";
import { Section } from "@astryxdesign/core/Section";
import { Text } from "@astryxdesign/core/Text";
import { TextInput } from "@astryxdesign/core/TextInput";
import { useToast } from "@astryxdesign/core/Toast";
import { VStack } from "@astryxdesign/core/VStack";
import { Bookmark, Copy, Download, Trash2, Upload } from "lucide-react";
import type { SavedEntry } from "../../shared/types";
import { appBridge } from "../bridge";

function formatShortDate(iso: string): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso.slice(0, 10);
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

export interface WordbookViewProps {
  entries: SavedEntry[];
  onRefreshEntries: () => Promise<void>;
  onNavigateTranslate: () => void;
}

export function WordbookView({
  entries,
  onRefreshEntries,
  onNavigateTranslate,
}: WordbookViewProps) {
  const showToast = useToast();
  const [searchQuery, setSearchQuery] = useState("");
  const fileInputRef = useRef<HTMLInputElement | null>(null);

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

  const handleImportFile = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    try {
      const content = await file.text();
      const count = await appBridge.importSavedEntries(content);
      await onRefreshEntries();
      showToast({
        body: `已导入 ${count} 条词条`,
        uniqueID: "wordbook-import",
      });
    } catch (err) {
      showToast({
        type: "error",
        body: `导入失败：${String(err)}`,
        uniqueID: "wordbook-import-err",
      });
    } finally {
      e.target.value = "";
    }
  };

  const handleExportJson = async () => {
    try {
      const json = await appBridge.exportSavedEntries();
      const blob = new Blob([json], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `LightTranslate-Wordbook-${new Date().toISOString().slice(0, 10)}.json`;
      a.click();
      URL.revokeObjectURL(url);
      showToast({
        body: "已导出为 JSON 文件",
        uniqueID: "wordbook-export",
        autoHideDuration: 2400,
      });
    } catch (err) {
      showToast({
        type: "error",
        body: `导出失败：${String(err)}`,
        uniqueID: "wordbook-export-err",
      });
    }
  };

  const handleDelete = async (item: SavedEntry) => {
    await appBridge.deleteSavedEntry(item.id);
    await onRefreshEntries();
    showToast({
      body: `已移除「${item.source}」`,
      uniqueID: "wordbook-delete",
      autoHideDuration: 4500,
      endContent: (
        <Button
          variant="ghost"
          size="sm"
          label="撤销"
          onClick={async () => {
            await appBridge.saveManualEntry({
              source: item.source,
              translation: item.translation,
              context: item.context,
            });
            await onRefreshEntries();
          }}
        />
      ),
    });
  };

  const handleCopyItem = async (item: SavedEntry) => {
    await appBridge.copyText(`${item.source} — ${item.translation}`);
    showToast({
      body: "已复制词条与释义",
      uniqueID: "wordbook-copy",
      autoHideDuration: 2000,
    });
  };

  return (
    <Layout
      height="fill"
      contentWidth={760}
      content={
        <LayoutContent padding={6}>
          <VStack gap={5}>
            {/* Borderless Page Title + Import/Export Actions */}
            <HStack justify="between" align="center" gap={3} wrap="wrap">
              <HStack align="center" gap={2}>
                <Heading level={2}>生词本</Heading>
                <Text type="supporting" hasTabularNumbers>
                  共 {entries.length.toLocaleString()} 词
                </Text>
              </HStack>

              <HStack align="center" gap={2}>
                <input
                  ref={fileInputRef}
                  type="file"
                  accept=".json,application/json"
                  onChange={handleImportFile}
                  className="hidden"
                />
                <Button
                  variant="secondary"
                  size="sm"
                  icon={<Upload className="w-3.5 h-3.5" strokeWidth={1.75} />}
                  label="导入"
                  onClick={() => fileInputRef.current?.click()}
                />
                <Button
                  variant="secondary"
                  size="sm"
                  icon={<Download className="w-3.5 h-3.5" strokeWidth={1.75} />}
                  label="导出"
                  isDisabled={entries.length === 0}
                  onClick={handleExportJson}
                />
              </HStack>
            </HStack>

            {entries.length > 0 && (
              <TextInput
                type="search"
                label="搜索生词本"
                isLabelHidden
                startIcon="search"
                hasClear
                width="100%"
                value={searchQuery}
                onChange={setSearchQuery}
                placeholder="搜索词汇、释义或例句…"
              />
            )}

            {filteredEntries.length === 0 ? (
              <Section variant="transparent" padding={8}>
                {entries.length === 0 ? (
                  <EmptyState
                    icon={<Bookmark className="w-6 h-6 text-secondary" strokeWidth={1.75} />}
                    title="暂无收藏词条"
                    description="查词或翻译时点击收藏，词条与例句会保存在这里。"
                    actions={
                      <Button
                        variant="secondary"
                        size="sm"
                        label="前往翻译"
                        onClick={onNavigateTranslate}
                      />
                    }
                  />
                ) : (
                  <EmptyState
                    title="未找到匹配词条"
                    description={`没有与「${searchQuery}」匹配的词汇或释义。`}
                    actions={
                      <Button
                        variant="secondary"
                        size="sm"
                        label="清除搜索"
                        onClick={() => setSearchQuery("")}
                      />
                    }
                  />
                )}
              </Section>
            ) : (
              /* Clean Lexicon Row List directly on cotton page surface (aligned to 24px content line) */
              <div className="min-w-0">
                <Divider />
                <List density="spacious" hasDividers>
                  {filteredEntries.map((item) => (
                    <ListItem
                      key={item.id}
                      itemKey={item.id}
                      align="start"
                      label={
                        <div className="flex items-baseline gap-3 flex-wrap select-text">
                          <span className="lt-serif text-[17.5px] font-semibold text-primary leading-snug">
                            {item.source}
                          </span>
                          <span className="text-[14px] text-primary/90 leading-relaxed">
                            {item.translation}
                          </span>
                        </div>
                      }
                      description={
                        item.context ? (
                          <span className="block mt-1 pl-3 border-l-2 border-border text-[13px] leading-relaxed text-secondary select-text">
                            {item.context}
                          </span>
                        ) : undefined
                      }
                      endContent={
                        <div className="flex items-center gap-1 pt-0.5">
                          {item.createdAt && (
                            <span className="mr-1.5 text-xs text-secondary tabular-nums hidden sm:inline">
                              {formatShortDate(item.createdAt)}
                            </span>
                          )}
                          <IconButton
                            variant="ghost"
                            size="sm"
                            label={`复制 ${item.source}`}
                            tooltip="复制词条"
                            icon={<Copy className="w-3.5 h-3.5" strokeWidth={1.75} />}
                            onClick={() => handleCopyItem(item)}
                          />
                          <IconButton
                            variant="ghost"
                            size="sm"
                            label={`移除 ${item.source}`}
                            tooltip="移除词条"
                            icon={<Trash2 className="w-3.5 h-3.5" strokeWidth={1.75} />}
                            onClick={() => handleDelete(item)}
                          />
                        </div>
                      }
                    />
                  ))}
                </List>
              </div>
            )}
          </VStack>
        </LayoutContent>
      }
    />
  );
}
