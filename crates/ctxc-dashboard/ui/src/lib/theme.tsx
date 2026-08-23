/**
 * Light, dark, or whatever the desktop says.
 *
 * The choice is remembered locally and applied as a class on `<html>`, which is
 * what every token in `index.css` keys off. "System" is a real third option
 * rather than the absence of a choice: a developer tool sits next to a terminal
 * and an editor, and following them is usually right — but not always, which is
 * why the other two exist.
 */

import * as React from "react";

export type Theme = "light" | "dark" | "system";

const STORAGE_KEY = "ctxc-theme";

/** The theme chosen last time, or "system" for a first visit. */
export function stored(): Theme {
  const saved = localStorage.getItem(STORAGE_KEY);
  return saved === "light" || saved === "dark" ? saved : "system";
}

/** What "system" currently resolves to. */
function preferred(): "light" | "dark" {
  return window.matchMedia("(prefers-color-scheme: dark)").matches
    ? "dark"
    : "light";
}

/** Put the resolved theme on `<html>`. */
function apply(theme: Theme) {
  const resolved = theme === "system" ? preferred() : theme;
  document.documentElement.classList.toggle("dark", resolved === "dark");
}

/**
 * Apply the saved theme before React renders.
 *
 * Called from the entry point so the first paint is already the right colour;
 * doing it in an effect would show a flash of the wrong one.
 */
export function applyStoredTheme() {
  apply(stored());
}

export interface ThemeControl {
  theme: Theme;
  resolved: "light" | "dark";
  set: (theme: Theme) => void;
}

export function useTheme(): ThemeControl {
  const [theme, setTheme] = React.useState<Theme>(stored);
  const [resolved, setResolved] = React.useState<"light" | "dark">(() =>
    theme === "system" ? preferred() : theme,
  );

  React.useEffect(() => {
    apply(theme);
    setResolved(theme === "system" ? preferred() : theme);

    if (theme !== "system") return;

    // Following the system means following it as it changes, not only as it was
    // when the page loaded.
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const update = () => {
      apply("system");
      setResolved(preferred());
    };
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, [theme]);

  const set = React.useCallback((next: Theme) => {
    if (next === "system") {
      localStorage.removeItem(STORAGE_KEY);
    } else {
      localStorage.setItem(STORAGE_KEY, next);
    }
    setTheme(next);
  }, []);

  return { theme, resolved, set };
}
