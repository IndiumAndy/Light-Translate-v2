import { defineTheme } from "@astryxdesign/core/theme";
import { neutralTheme } from "@astryxdesign/theme-neutral";

export const editorialTheme = defineTheme({
  name: "editorial",
  extends: neutralTheme,

  typography: {
    scale: { base: 14, ratio: 1.2 },
    body: {
      family: "-apple-system",
      fallbacks:
        'BlinkMacSystemFont, "SF Pro Text", "PingFang SC", "Hiragino Sans GB", "Segoe UI Variable Text", system-ui, sans-serif',
    },
    heading: {
      family: "-apple-system",
      fallbacks:
        'BlinkMacSystemFont, "SF Pro Display", "PingFang SC", "Hiragino Sans GB", "Segoe UI Variable Display", system-ui, sans-serif',
      weights: { 1: "semibold", 2: "semibold", 3: "semibold", 4: "semibold" },
    },
    code: {
      family: "SF Mono",
      fallbacks: 'ui-monospace, Menlo, Monaco, Consolas, "Liberation Mono", monospace',
    },
  },

  motion: { fast: 120, medium: 240, slow: 480, ratio: 0.75 },

  // B1 · Crisp Cotton & Prussian Ink (清透棉白 + 现代钢笔藏青)
  tokens: {
    "--color-background-body": ["#F3F2EF", "#141517"],
    "--color-background-surface": ["#FCFCFA", "#1B1D20"],
    "--color-background-card": ["#FFFFFF", "#22252A"],
    "--color-background-popover": ["#FFFFFF", "#252930"],
    "--color-background-muted": ["#ECEBE7", "#252930"],

    "--color-neutral": ["rgba(21, 24, 30, 0.06)", "rgba(255, 255, 255, 0.08)"],
    "--color-overlay-hover": ["rgba(21, 24, 30, 0.045)", "rgba(255, 255, 255, 0.055)"],
    "--color-overlay-pressed": ["rgba(21, 24, 30, 0.085)", "rgba(255, 255, 255, 0.095)"],

    "--color-text-primary": ["#15181E", "#ECEEF2"],
    "--color-text-secondary": ["#616670", "#989EA8"],
    "--color-text-disabled": ["#989DA6", "#5C626E"],

    "--color-accent": ["#1F508A", "#78A9DC"],
    "--color-on-accent": ["#FFFFFF", "#0C1522"],
    "--color-accent-muted": ["#E6EFFA", "#1F3147"],
    "--color-text-accent": ["#1B467A", "#8BB8E8"],

    "--color-icon-accent": ["#1F508A", "#8BB8E8"],
    "--color-icon-primary": ["#181C23", "#ECEEF2"],
    "--color-icon-secondary": ["#646A75", "#949BA6"],
    "--color-icon-disabled": ["#9DA2AC", "#585E6A"],

    "--color-border": ["#E5E4DF", "rgba(255, 255, 255, 0.085)"],
    "--color-border-emphasized": ["#D2D1CB", "#383D47"],

    "--radius-none": "0px",
    "--radius-inner": "0.375rem",
    "--radius-element": "0.5rem",
    "--radius-container": "0.75rem",
    "--radius-page": "0.875rem",
    "--radius-full": "9999px",
  },

  components: {
    "app-shell-sidenav": {
      base: {
        backgroundColor: "var(--color-background-body)",
      },
    },
    card: {
      base: {
        padding: "var(--spacing-5)",
      },
    },
    section: {
      base: {
        padding: "var(--spacing-4)",
      },
    },
  },
});
