export interface ScreenPoint {
  x: number;
  y: number;
}

export interface ScreenRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export type ExtractionScope = "word" | "sentence" | "selection" | "label" | "regionOcr";
export type ExtractionCompleteness = "complete" | "partial";
export type ExtractionSource =
  | "accessibility"
  | "ocr"
  | "clipboardFallback"
  | "primarySelection"
  | "manual";

export interface WordSense {
  partOfSpeech: string;
  glosses: string[];
}

export interface WordEntry {
  word: string;
  senses: WordSense[];
  truncated: boolean;
}

export interface PopupCardViewState {
  visible: boolean;
  generation: number;
  originalText: string;
  contextSentence: string | null;
  sentenceCompleteness: ExtractionCompleteness | null;
  scope: ExtractionScope;
  completeness: ExtractionCompleteness;
  source: ExtractionSource;
  dictionary: WordEntry | null;
  fastTranslation: string | null;
  llmTranslation: string;
  isStreaming: boolean;
  translationError: string | null;
  explanation: string | null;
  isExplaining: boolean;
  explanationError: string | null;
  statusBanner: string | null;
  interactive: boolean;
  pinned: boolean;
  saved: boolean;
  ttftMs: number | null;
}

export type TriggerModifier = "altOrOption" | "leftOption" | "rightOption" | "control";
export type LlmProviderKind = "gemini" | "deepseek" | "openaiCompatible";
export type FastMtEngine = "googleFree" | "microsoftEdge";
export type QueryMode = "word" | "sentence" | "selection" | "explanation";

export interface ProviderEndpointSettings {
  baseUrl: string;
  model: string;
}

export interface AppSettings {
  enabled: boolean;
  triggerModifier: TriggerModifier;
  hoverDwellMs: number;
  lingerDismissMs: number;
  debounceRadiusPt: number;
  doubleTapSelection: boolean;
  selectionShortcut: string;
  captureShortcut: string;
  enableOcrFallback: boolean;
  enableClipboardFallback: boolean;
  enableFastMt: boolean;
  fastMtEngine: FastMtEngine;
  wordFastOnly: boolean;
  enableLlmTranslation: boolean;
  activeProvider: LlmProviderKind;
  gemini: ProviderEndpointSettings;
  deepseek: ProviderEndpointSettings;
  openaiCompatible: ProviderEndpointSettings;
  targetLanguage: string;
  excludedApps: string[];
}

export interface ConnectionTestResult {
  ok: boolean;
  ttftMs: number;
  totalMs: number;
  sampleOutput: string;
  error: string | null;
}

export interface SavedEntry {
  id: string;
  source: string;
  translation: string;
  context: string;
  createdAt: string;
}

export interface PermissionsStatus {
  platform: string;
  accessibility: boolean;
  screenRecording: boolean;
}

export interface ManualStreamEvent {
  requestId: string;
  dictionary: WordEntry | null;
  fastTranslation: string | null;
  llmTranslation: string;
  done: boolean;
  ttftMs: number | null;
  error: string | null;
}
