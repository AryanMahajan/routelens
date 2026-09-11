import { createContext, useContext } from "react";

/**
 * The variable names the active environment can resolve, for autocomplete.
 *
 * Held at the top of the app and refreshed whenever the workspace, environment or secrets
 * change, so every `{{` popup in every input agrees on what exists.
 */
export const VariablesContext = createContext<string[]>([]);

export function useVariableNames(): string[] {
  return useContext(VariablesContext);
}
