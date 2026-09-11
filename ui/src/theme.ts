/**
 * Light or dark, chosen by the user and remembered per machine.
 *
 * Applied as `data-theme` on <html>; the palettes live in `styles.css`. Dark is the default
 * because the tool sits beside an editor, and most editors are dark.
 */

export type Theme = "dark" | "light";

const KEY = "routelens.theme";

export function currentTheme(): Theme {
  try {
    return localStorage.getItem(KEY) === "light" ? "light" : "dark";
  } catch {
    return "dark";
  }
}

export function applyTheme(theme: Theme) {
  document.documentElement.dataset.theme = theme;
  try {
    localStorage.setItem(KEY, theme);
  } catch {
    // Private mode or blocked storage: the choice just does not survive a restart.
  }
}
