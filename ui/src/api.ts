/**
 * The only place this UI talks to the Rust core.
 *
 * Every call goes through Tauri's `invoke`. Keeping them in one module means the command
 * names — which are strings, and so invisible to the type checker — are written once.
 */

import { invoke } from "@tauri-apps/api/core";
import type {
  Collection,
  Environment,
  Exchange,
  HistoryEntry,
  ImportResult,
  OpenApiSummary,
  RequestDraft,
  WorkspaceInfo,
} from "./types";

/** An error raised by the core, already carrying its flattened source chain. */
export class CoreError extends Error {}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    // Commands reject with `{ message }`; anything else is a genuine surprise.
    if (typeof error === "object" && error !== null && "message" in error) {
      throw new CoreError(String((error as { message: unknown }).message));
    }
    throw new CoreError(String(error));
  }
}

export const api = {
  // --- workspace ---
  openWorkspace: (path: string) => call<WorkspaceInfo>("open_workspace", { path }),
  openOrCreateWorkspace: (path: string, name: string) =>
    call<WorkspaceInfo>("open_or_create_workspace", { path, name }),
  createWorkspace: (path: string, name: string, standalone = false) =>
    call<WorkspaceInfo>("create_workspace", { path, name, standalone }),
  workspaceInfo: () => call<WorkspaceInfo>("workspace_info"),
  closeWorkspace: () => call<void>("close_workspace"),

  // --- environments ---
  setEnvironment: (name: string | null) => call<WorkspaceInfo>("set_environment", { name }),
  loadEnvironment: (name: string) => call<Environment>("load_environment", { name }),
  saveEnvironment: (environment: Environment) =>
    call<void>("save_environment", { environment }),

  // --- secrets (names only ever cross this boundary) ---
  secretNames: () => call<string[]>("secret_names"),
  setSecret: (name: string, value: string) => call<void>("set_secret", { name, value }),
  deleteSecret: (name: string) => call<void>("delete_secret", { name }),

  // --- collections ---
  loadCollection: (name: string) => call<Collection>("load_collection", { name }),
  saveRequest: (collection: string, request: RequestDraft) =>
    call<void>("save_request", { collection, request }),

  // --- sending ---
  send: (request: RequestDraft) => call<Exchange>("send_request", { request }),

  // --- history ---
  history: (limit = 50) => call<HistoryEntry[]>("history", { limit }),
  clearHistory: () => call<void>("clear_history"),

  // --- import ---
  importCurl: (text: string) => call<ImportResult<RequestDraft>>("import_curl", { text }),
  importRawHttp: (text: string) =>
    call<ImportResult<RequestDraft>>("import_raw_http", { text }),
  importOpenApi: (text: string, collection: string) =>
    call<ImportResult<OpenApiSummary>>("import_openapi", { text, collection }),
};
