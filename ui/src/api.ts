/**
 * The only place this UI talks to the Rust core.
 *
 * Every call goes through Tauri's `invoke`. Keeping them in one module means the command
 * names — which are strings, and so invisible to the type checker — are written once.
 */

import { invoke } from "@tauri-apps/api/core";
import {
  normalizeRequest,
  type Collection,
  type ScanResult,
  type Environment,
  type Exchange,
  type HistoryEntry,
  type ImportResult,
  type OpenApiSummary,
  type RequestDraft,
  type WireCollection,
  type WireRequestDraft,
  type WorkspaceInfo,
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
  loadEnvironment: async (name: string): Promise<Environment> => {
    const wire = await call<Environment>("load_environment", { name });
    return { ...wire, secrets: wire.secrets ?? [] };
  },
  saveEnvironment: (environment: Environment) =>
    call<void>("save_environment", { environment }),
  deleteEnvironment: (name: string) => call<WorkspaceInfo>("delete_environment", { name }),
  /** Every `{{name}}` that would resolve right now; secrets appear as `secret:NAME`. */
  variableNames: () => call<string[]>("variable_names"),

  // --- secrets (names only ever cross this boundary) ---
  secretNames: () => call<string[]>("secret_names"),
  setSecret: (name: string, value: string) => call<void>("set_secret", { name, value }),
  deleteSecret: (name: string) => call<void>("delete_secret", { name }),

  // --- collections ---
  loadCollection: async (name: string): Promise<Collection> => {
    const wire = await call<WireCollection>("load_collection", { name });
    return { ...wire, requests: (wire.requests ?? []).map(normalizeRequest) };
  },
  saveRequest: (collection: string, request: RequestDraft) =>
    call<void>("save_request", { collection, request }),

  // --- discovery (reads source; never executes it) ---
  scanProject: () => call<ScanResult>("scan_project"),
  openEndpoint: async (id: string, baseUrl?: string) =>
    normalizeRequest(
      await call<WireRequestDraft>("open_endpoint", { id, baseUrl: baseUrl ?? null }),
    ),
  revealInEditor: (file: string, line: number) =>
    call<void>("reveal_in_editor", { file, line }),

  // --- sending ---
  send: async (request: RequestDraft): Promise<Exchange> => {
    const wire = await call<Exchange>("send_request", { request });
    // `redirects` is omitted on the wire when empty — which is every direct response.
    return { ...wire, response: { ...wire.response, redirects: wire.response.redirects ?? [] } };
  },

  // --- history ---
  history: (limit = 50) => call<HistoryEntry[]>("history", { limit }),
  clearHistory: () => call<void>("clear_history"),

  // --- import ---
  importCurl: async (text: string): Promise<ImportResult<RequestDraft>> => {
    const result = await call<ImportResult<WireRequestDraft>>("import_curl", { text });
    return { ...result, value: normalizeRequest(result.value) };
  },
  importRawHttp: async (text: string): Promise<ImportResult<RequestDraft>> => {
    const result = await call<ImportResult<WireRequestDraft>>("import_raw_http", { text });
    return { ...result, value: normalizeRequest(result.value) };
  },
  importOpenApi: (text: string, collection: string) =>
    call<ImportResult<OpenApiSummary>>("import_openapi", { text, collection }),
};
