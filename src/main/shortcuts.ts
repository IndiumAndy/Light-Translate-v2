import type { TriggerModifier } from "../shared/types";

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

export function triggerModifierLabel(
  mod: TriggerModifier | undefined,
  isMac: boolean
): string {
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

export function triggerModifierKbdKeys(mod: TriggerModifier | undefined): string {
  switch (mod) {
    case "control":
      return "ctrl";
    default:
      return "alt";
  }
}

/**
 * Converts stored canonical shortcut strings (e.g. "Control+Command+S")
 * into Astryx `<Kbd keys="..." />` format (e.g. "ctrl+meta+S").
 */
export function toAstryxKbdKeys(raw: string): string | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;

  let ctrl = false;
  let alt = false;
  let shift = false;
  let meta = false;
  let keyLabel = "";

  for (const part of trimmed.split("+").map((s) => s.trim()).filter(Boolean)) {
    const lower = part.toLowerCase();
    if (lower === "control" || lower === "ctrl" || lower === "⌃") ctrl = true;
    else if (lower === "alt" || lower === "option" || lower === "opt" || lower === "⌥")
      alt = true;
    else if (lower === "shift" || lower === "⇧") shift = true;
    else if (
      lower === "command" ||
      lower === "cmd" ||
      lower === "meta" ||
      lower === "super" ||
      lower === "win" ||
      lower === "⌘"
    ) {
      meta = true;
    } else if (lower === "space") keyLabel = "Space";
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
    else if (lower === "+") keyLabel = "plus";
    else keyLabel = part.toUpperCase();
  }

  if (!keyLabel) return null;

  const tokens: string[] = [];
  if (ctrl) tokens.push("ctrl");
  if (alt) tokens.push("alt");
  if (shift) tokens.push("shift");
  if (meta) tokens.push("meta");
  tokens.push(keyLabel);
  return tokens.join("+");
}

export function formatShortcutInline(raw: string, isMac: boolean): string {
  const kbd = toAstryxKbdKeys(raw);
  if (!kbd) return "";
  const parts = kbd.split("+");
  if (isMac) {
    return parts
      .map((p) => {
        if (p === "ctrl") return "⌃";
        if (p === "alt") return "⌥";
        if (p === "shift") return "⇧";
        if (p === "meta") return "⌘";
        if (p === "plus") return "+";
        return p;
      })
      .join("");
  }
  return parts
    .map((p) => {
      if (p === "ctrl") return "Ctrl";
      if (p === "alt") return "Alt";
      if (p === "shift") return "Shift";
      if (p === "meta") return "Win";
      if (p === "plus") return "+";
      return p;
    })
    .join("+");
}

export function extractShortcutFromEvent(
  e: KeyboardEvent,
  isMac: boolean
): { canonical: string | null; previewKbd: string | null; error?: string } {
  const previewTokens: string[] = [];
  if (e.ctrlKey) previewTokens.push("ctrl");
  if (e.altKey) previewTokens.push("alt");
  if (e.shiftKey) previewTokens.push("shift");
  if (e.metaKey) previewTokens.push("meta");
  const previewKbd = previewTokens.length > 0 ? previewTokens.join("+") : null;

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
    return { canonical: null, previewKbd };
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
      previewKbd,
      error: "支持字母、数字、F1–F12、空格及常用标点",
    };
  }

  const isFnKey = /^F([1-9]|1[0-2])$/.test(keyToken);
  if (!e.ctrlKey && !e.altKey && !e.metaKey && !isFnKey) {
    return {
      canonical: null,
      previewKbd,
      error: isMac ? "需包含 ⌘、⌃ 或 ⌥，或直接使用 F1–F12" : "需包含 Ctrl、Alt 或 Win，或直接使用 F1–F12",
    };
  }

  const parts: string[] = [];
  if (e.ctrlKey) parts.push("Control");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  if (e.metaKey) parts.push("Command");
  parts.push(keyToken);
  return { canonical: parts.join("+"), previewKbd };
}
