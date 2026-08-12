// Light/dark theming. CSS custom properties (style.css) are the single
// source of truth for colors; this module only (a) applies the user's
// explicit override (or clears it to follow the OS/browser setting) via a
// `data-theme` attribute on <html>, persisted in localStorage, and (b)
// gives uPlot -- which draws its axes/grid directly on <canvas> and can't
// see CSS at all -- a way to read those same colors and to be told when
// they've changed, since a canvas repaint is never triggered by a CSS
// media-query flip on its own.

export type ThemePreference = "system" | "light" | "dark";

const STORAGE_KEY = "ici-web:theme";
const listeners = new Set<() => void>();
const media = window.matchMedia("(prefers-color-scheme: dark)");

export function getThemePreference(): ThemePreference {
  const stored = localStorage.getItem(STORAGE_KEY);
  return stored === "light" || stored === "dark" ? stored : "system";
}

export function setThemePreference(pref: ThemePreference): void {
  if (pref === "system") localStorage.removeItem(STORAGE_KEY);
  else localStorage.setItem(STORAGE_KEY, pref);
  applyAttribute(pref);
  notify();
}

/** Cycles system -> light -> dark -> system, for a single toggle button. */
export function cycleThemePreference(): ThemePreference {
  const order: ThemePreference[] = ["system", "light", "dark"];
  const next = order[(order.indexOf(getThemePreference()) + 1) % order.length];
  setThemePreference(next);
  return next;
}

function applyAttribute(pref: ThemePreference): void {
  if (pref === "system") document.documentElement.removeAttribute("data-theme");
  else document.documentElement.setAttribute("data-theme", pref);
}

/** Call once at startup, before anything reads chart colors. */
export function initTheme(): void {
  applyAttribute(getThemePreference());
  // A pure `prefers-color-scheme` change (no explicit override in play)
  // repaints the DOM for free via CSS, but every already-drawn uPlot
  // canvas needs an explicit nudge to re-read the new colors.
  media.addEventListener("change", () => {
    if (getThemePreference() === "system") notify();
  });
}

/** Subscribe to any theme change (explicit toggle or, while on "system", an OS-level flip). Charts should redraw(true, true) on this to re-run their color functions. */
export function onThemeChange(cb: () => void): () => void {
  listeners.add(cb);
  return () => listeners.delete(cb);
}

function notify(): void {
  for (const cb of listeners) cb();
}

function cssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

function hexToRgba(hex: string, alpha: number): string {
  const n = parseInt(hex.replace("#", ""), 16);
  return `rgba(${(n >> 16) & 255},${(n >> 8) & 255},${n & 255},${alpha})`;
}

// uPlot calls axis.stroke/grid.stroke/etc. as `(self, si) => color` on
// every redraw when given a function instead of a literal, so passing
// these directly as chart options keeps axes in sync with the theme
// without the caller needing to rebuild anything -- only redraw(true, true)
// on change (see onThemeChange above).
export function chartTextColor(): string {
  return cssVar("--text");
}

export function chartGridColor(): string {
  return cssVar("--border");
}

export function chartBgColor(): string {
  return cssVar("--bg");
}

export function chartMutedColor(alpha: number): string {
  return hexToRgba(cssVar("--text-muted"), alpha);
}
