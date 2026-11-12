import { useMemo, useState } from "react";
import { Button } from "@astryxdesign/core/Button";
import { Card } from "@astryxdesign/core/Card";
import { Divider } from "@astryxdesign/core/Divider";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { Grid } from "@astryxdesign/core/Grid";
import { Heading } from "@astryxdesign/core/Heading";
import { HStack } from "@astryxdesign/core/HStack";
import { Kbd } from "@astryxdesign/core/Kbd";
import { Layout, LayoutContent } from "@astryxdesign/core/Layout";
import { List, ListItem } from "@astryxdesign/core/List";
import { MetadataList, MetadataListItem } from "@astryxdesign/core/MetadataList";
import {
  SegmentedControl,
  SegmentedControlItem,
} from "@astryxdesign/core/SegmentedControl";
import { Text } from "@astryxdesign/core/Text";
import { VStack } from "@astryxdesign/core/VStack";
import { Bookmark } from "lucide-react";
import type {
  AppSettings,
  HeatmapDayCell,
  SavedEntry,
  UsageStatsSummary,
} from "../../shared/types";
import {
  toAstryxKbdKeys,
  triggerModifierKbdKeys,
  triggerModifierLabel,
} from "../shortcuts";

export type SettingsPane = "interaction" | "engine" | "system";
type HeatmapRange = "26w" | "52w";

const WEEKDAY_NAMES_ZH = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"];

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

function heatLevelClass(level: number): string {
  switch (level) {
    case 1:
      return "lt-heat-1";
    case 2:
      return "lt-heat-2";
    case 3:
      return "lt-heat-3";
    case 4:
      return "lt-heat-4";
    default:
      return "lt-heat-0";
  }
}

export interface OverviewViewProps {
  settings: AppSettings | null;
  usageStats: UsageStatsSummary | null;
  entries: SavedEntry[];
  isMac: boolean;
  defaultCaptureShortcut: string;
  defaultSelectionShortcut: string;
  onNavigateTranslate: () => void;
  onNavigateWordbook: () => void;
  onNavigateSettings: (pane?: SettingsPane) => void;
}

export function OverviewView({
  settings,
  usageStats,
  entries,
  isMac,
  defaultCaptureShortcut,
  defaultSelectionShortcut,
  onNavigateTranslate,
  onNavigateWordbook,
  onNavigateSettings,
}: OverviewViewProps) {
  const [heatmapRange, setHeatmapRange] = useState<HeatmapRange>("26w");
  const [hoveredDay, setHoveredDay] = useState<HeatmapDayCell | null>(null);
  const [pinnedDay, setPinnedDay] = useState<HeatmapDayCell | null>(null);

  const visibleHeatmapWeeks = useMemo(() => {
    const all = buildHeatmapWeeks(usageStats?.heatmap ?? []);
    return heatmapRange === "26w" ? all.slice(-26) : all;
  }, [usageStats, heatmapRange]);

  const heatmapMonthMarkers = useMemo(() => {
    const markers: { colIdx: number; label: string }[] = [];
    const minColGap = heatmapRange === "26w" ? 3 : 4;

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
      return `较上周 ${sign}`;
    }
    return `活跃 ${usageStats.activeDays} 天`;
  }, [usageStats]);

  const captureKbd = toAstryxKbdKeys(settings?.captureShortcut ?? defaultCaptureShortcut);
  const selectionKbd = toAstryxKbdKeys(settings?.selectionShortcut ?? defaultSelectionShortcut);
  const hoverModKbd = triggerModifierKbdKeys(settings?.triggerModifier);
  const hoverModText = triggerModifierLabel(settings?.triggerModifier, isMac);

  const cellGapCls = heatmapRange === "26w" ? "gap-[4px]" : "gap-[2.5px]";
  const cellRadiusCls = heatmapRange === "26w" ? "rounded-[3px]" : "rounded-[2px]";

  return (
    <Layout
      height="fill"
      contentWidth={760}
      content={
        <LayoutContent padding={6}>
          <VStack gap={6}>
            {/* Borderless Page Title + Active Summary */}
            <HStack justify="between" align="center" gap={3}>
              <Heading level={2}>概览</Heading>
              {usageStats?.todayDate && (
                <Text type="supporting" hasTabularNumbers>
                  {usageStats.todayDate} · 累计活跃 {usageStats.activeDays} 天
                </Text>
              )}
            </HStack>

            {/* Lightweight Borderless Metric Strip via Astryx MetadataList */}
            <MetadataList columns={4}>
              <MetadataListItem label="今日词量">
                <VStack gap={0.5}>
                  <span className="text-[24px] leading-tight font-semibold tracking-[-0.025em] tabular-nums text-[var(--color-text-accent)]">
                    {(usageStats?.todayWords ?? 0).toLocaleString()}
                  </span>
                  <Text type="supporting" hasTabularNumbers>
                    {(usageStats?.todayQueries ?? 0).toLocaleString()} 次查词
                  </Text>
                </VStack>
              </MetadataListItem>

              <MetadataListItem label="连续活跃">
                <VStack gap={0.5}>
                  <div className="flex items-baseline gap-1">
                    <span className="text-[24px] leading-tight font-semibold tracking-[-0.025em] tabular-nums text-primary">
                      {usageStats?.currentStreak ?? 0}
                    </span>
                    <Text type="supporting" weight="medium">
                      天
                    </Text>
                  </div>
                  <Text type="supporting" hasTabularNumbers>
                    最长 {usageStats?.longestStreak ?? 0} 天
                  </Text>
                </VStack>
              </MetadataListItem>

              <MetadataListItem label="近 7 日词量">
                <VStack gap={0.5}>
                  <span className="text-[24px] leading-tight font-semibold tracking-[-0.025em] tabular-nums text-primary">
                    {(usageStats?.thisWeekWords ?? 0).toLocaleString()}
                  </span>
                  <Text type="supporting" hasTabularNumbers>
                    {weekMetaLabel}
                  </Text>
                </VStack>
              </MetadataListItem>

              <MetadataListItem label="累计词量">
                <VStack gap={0.5}>
                  <span className="text-[24px] leading-tight font-semibold tracking-[-0.025em] tabular-nums text-primary">
                    {(usageStats?.totalWords ?? 0).toLocaleString()}
                  </span>
                  <Text type="supporting" hasTabularNumbers>
                    {(usageStats?.totalQueries ?? 0).toLocaleString()} 次查词
                  </Text>
                </VStack>
              </MetadataListItem>
            </MetadataList>

            {/* Single Hero Card: Reading Cadence Heatmap with 26W / 52W SegmentedControl */}
            <Card>
              <VStack gap={3}>
                <HStack justify="between" align="center" gap={3}>
                  <Heading level={3}>阅读足迹</Heading>
                  <SegmentedControl
                    size="sm"
                    label="热力图时间跨度"
                    value={heatmapRange}
                    onChange={(val) => setHeatmapRange(val as HeatmapRange)}
                  >
                    <SegmentedControlItem value="26w" label="近 26 周" />
                    <SegmentedControlItem value="52w" label="近 52 周" />
                  </SegmentedControl>
                </HStack>

                {visibleHeatmapWeeks.length > 0 ? (
                  <div className="flex items-stretch gap-2.5 select-none pt-0.5">
                    {/* Weekday Y-Axis */}
                    <div className="w-4 shrink-0 flex flex-col pt-5" aria-hidden="true">
                      <div className={`flex-1 grid grid-rows-7 ${cellGapCls} text-xs text-secondary`}>
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
                              left: `${
                                (m.colIdx / Math.max(visibleHeatmapWeeks.length, 1)) * 100
                              }%`,
                            }}
                            className="absolute top-0 text-xs leading-none text-secondary whitespace-nowrap"
                          >
                            {m.label}
                          </span>
                        ))}
                      </div>

                      <div
                        className={`grid ${cellGapCls}`}
                        style={{
                          gridTemplateColumns: `repeat(${Math.max(
                            visibleHeatmapWeeks.length,
                            1
                          )}, minmax(0, 1fr))`,
                        }}
                      >
                        {visibleHeatmapWeeks.map((week, colIdx) => (
                          <div key={colIdx} className={`grid grid-rows-7 ${cellGapCls}`}>
                            {week.map((cell, rowIdx) => {
                              if (!cell) {
                                return (
                                  <div
                                    key={rowIdx}
                                    className={`aspect-square w-full ${cellRadiusCls} opacity-0 pointer-events-none`}
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
                                  aria-label={`${cell.date} ${weekdayText}：${cell.wordCount} 词，${cell.queryCount} 次查词`}
                                  title={`${cell.date} ${weekdayText} · ${cell.wordCount.toLocaleString()} 词`}
                                  onMouseEnter={() => setHoveredDay(cell)}
                                  onFocus={() => setHoveredDay(cell)}
                                  onBlur={() => setHoveredDay(null)}
                                  onClick={() =>
                                    setPinnedDay((prev) =>
                                      prev?.date === cell.date ? null : cell
                                    )
                                  }
                                  className={`aspect-square w-full ${cellRadiusCls} cursor-pointer transition-transform duration-100 motion-reduce:transition-none hover:ring-1 hover:ring-[var(--color-text-primary)]/45 ${heatLevelClass(
                                    cell.level
                                  )} ${
                                    isPinned
                                      ? "ring-2 ring-[var(--color-accent)] ring-offset-1 ring-offset-[var(--color-background-card)] z-10"
                                      : isToday
                                        ? "ring-1 ring-[var(--color-text-primary)]/40"
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
                  <EmptyState
                    isCompact
                    title="暂无阅读记录"
                    description="按住修饰键屏幕取词，或在「翻译」中输入文本。"
                    actions={
                      <Button
                        variant="secondary"
                        size="sm"
                        label="前往翻译"
                        onClick={onNavigateTranslate}
                      />
                    }
                  />
                )}

                <Divider />

                {/* Readout & Prussian Ink Legend */}
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <div className="flex items-center gap-2 min-w-0">
                    {inspectedDay && (
                      <>
                        <Text
                          type="supporting"
                          color="primary"
                          weight="medium"
                          hasTabularNumbers
                        >
                          {inspectedDay.date}{" "}
                          {WEEKDAY_NAMES_ZH[Math.max(0, Math.min(6, inspectedDay.weekday - 1))]}
                        </Text>
                        <Text type="supporting">·</Text>
                        <Text type="supporting" hasTabularNumbers>
                          <strong className="font-semibold text-primary">
                            {inspectedDay.wordCount.toLocaleString()}
                          </strong>{" "}
                          词
                        </Text>
                        <Text type="supporting">·</Text>
                        <Text type="supporting" hasTabularNumbers>
                          {inspectedDay.queryCount.toLocaleString()} 次查词
                        </Text>
                        {pinnedDay && !hoveredDay && (
                          <Button
                            variant="ghost"
                            size="sm"
                            label="取消固定"
                            onClick={() => setPinnedDay(null)}
                          />
                        )}
                      </>
                    )}
                  </div>

                  <div
                    className="flex items-center gap-1.5 select-none"
                    aria-label="阅读量深浅图例"
                  >
                    <Text type="supporting">少</Text>
                    {[0, 1, 2, 3, 4].map((lv) => (
                      <span
                        key={lv}
                        className={`w-2.5 h-2.5 rounded-[2px] ${heatLevelClass(lv)}`}
                      />
                    ))}
                    <Text type="supporting">多</Text>
                  </div>
                </div>
              </VStack>
            </Card>

            {/* Clean Two-Column Sections aligned to the 24px content line (no Card soup, no negative bleed) */}
            <Grid columns={{ minWidth: 260, repeat: "fit" }} gap={6}>
              <section className="min-w-0">
                <VStack gap={2}>
                  <HStack justify="between" align="center">
                    <Heading level={4}>快捷触发</Heading>
                    <Button
                      variant="ghost"
                      size="sm"
                      label="设置"
                      onClick={() => onNavigateSettings("interaction")}
                    />
                  </HStack>

                  <Divider />

                  <List density="compact" hasDividers>
                    <ListItem
                      label="悬停取词"
                      description="按住修饰键悬停于文字上方"
                      endContent={
                        settings?.triggerModifier === "leftOption" ||
                        settings?.triggerModifier === "rightOption" ? (
                          <Text type="supporting" weight="medium">
                            {hoverModText}
                          </Text>
                        ) : (
                          <Kbd keys={hoverModKbd} />
                        )
                      }
                    />
                    <ListItem
                      label="划词翻译"
                      description="选中文本后唤起译文浮窗"
                      endContent={
                        selectionKbd ? (
                          <Kbd keys={selectionKbd} />
                        ) : settings?.doubleTapSelection ? (
                          <Text type="supporting" weight="medium">
                            双击 {hoverModText}
                          </Text>
                        ) : (
                          <Text type="supporting">未绑定</Text>
                        )
                      }
                    />
                    <ListItem
                      label="区域截屏"
                      description="框选屏幕区域识别并翻译"
                      endContent={
                        captureKbd ? (
                          <Kbd keys={captureKbd} />
                        ) : (
                          <Text type="supporting">未绑定</Text>
                        )
                      }
                    />
                  </List>
                </VStack>
              </section>

              <section className="min-w-0">
                <VStack gap={2}>
                  <HStack justify="between" align="center">
                    <Heading level={4}>近期收录</Heading>
                    {entries.length > 0 && (
                      <Button
                        variant="ghost"
                        size="sm"
                        label="查看全部"
                        onClick={onNavigateWordbook}
                      />
                    )}
                  </HStack>

                  <Divider />

                  {entries.length > 0 ? (
                    <List density="compact" hasDividers>
                      {entries.slice(0, 3).map((item) => (
                        <ListItem
                          key={item.id}
                          onClick={onNavigateWordbook}
                          label={
                            <span className="lt-serif text-[15.5px] font-semibold text-primary">
                              {item.source}
                            </span>
                          }
                          description={
                            <Text type="supporting" maxLines={1}>
                              {item.translation}
                            </Text>
                          }
                        />
                      ))}
                    </List>
                  ) : (
                    <div className="pt-2">
                      <EmptyState
                        isCompact
                        icon={<Bookmark className="w-5 h-5 text-secondary" strokeWidth={1.75} />}
                        title="暂无收藏词条"
                        description="查词时点击收藏即可在此回顾。"
                      />
                    </div>
                  )}
                </VStack>
              </section>
            </Grid>
          </VStack>
        </LayoutContent>
      }
    />
  );
}
