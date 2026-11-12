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

  // B2 · Modern Porcelain & Academy Ink (温润瓷白 + 清澈学院蓝墨)
  tokens: {
    "--color-background-body": ["#F2F2F1", "#141518"],
    "--color-background-surface": ["#FDFDFC", "#1B1D21"],
    "--color-background-card": ["#FFFFFF", "#22252B"],
    "--color-background-popover": ["#FFFFFF", "#252930"],
    "--color-background-muted": ["#F5F5F4", "#252930"],

    "--color-neutral": ["rgba(20, 23, 31, 0.055)", "rgba(255, 255, 255, 0.08)"],
    "--color-overlay-hover": ["rgba(20, 23, 31, 0.04)", "rgba(255, 255, 255, 0.055)"],
    "--color-overlay-pressed": ["rgba(20, 23, 31, 0.08)", "rgba(255, 255, 255, 0.095)"],

    "--color-text-primary": ["#14171F", "#ECEEF2"],
    "--color-text-secondary": ["#5E6470", "#989EA8"],
    "--color-text-disabled": ["#969CA8", "#5C626E"],

    "--color-accent": ["#1E5494", "#7AABE0"],
    "--color-on-accent": ["#FFFFFF", "#0C1522"],
    "--color-accent-muted": ["#E5F0FC", "#1E3148"],
    "--color-text-accent": ["#194880", "#8BB8E8"],

    "--color-icon-accent": ["#1E5494", "#8BB8E8"],
    "--color-icon-primary": ["#171B24", "#ECEEF2"],
    "--color-icon-secondary": ["#616775", "#949BA6"],
    "--color-icon-disabled": ["#9AA0AD", "#585E6A"],

    "--color-border": ["#E4E5E7", "rgba(255, 255, 255, 0.085)"],
    "--color-border-emphasized": ["#CFD2D8", "#383D47"],

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
