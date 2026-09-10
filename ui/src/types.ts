/**
 * The wire format between the Rust core and this UI.
 *
 * These mirror the serde representation of `rl-model` and `rl-http` types. Where a Rust enum
 * is tagged, the tag name here matches the `#[serde(tag = "...")]` attribute exactly — get
 * that wrong and the value silently fails to deserialize on the way back.
 */

export type HttpMethod =
  | "GET"
  | "HEAD"
  | "POST"
  | "PUT"
  | "PATCH"
  | "DELETE"
  | "OPTIONS"
  | "TRACE"
  | string; // Unknown methods round-trip as themselves.

export interface KeyValue {
  key: string;
  value: string;
  enabled: boolean;
  description?: string | null;
}

export type ApiKeyLocation = "header" | "query" | "cookie";

export type AuthConfig =
  | { type: "none" }
  | { type: "inherit" }
  | { type: "bearer"; token: string }
  | { type: "basic"; username: string; password: string }
  | { type: "api_key"; key: string; value: string; location: ApiKeyLocation };

export type BodyValue =
  | { type: "none" }
  | { type: "json"; content: string }
  | { type: "text"; content: string; content_type: string }
  | { type: "form"; fields: KeyValue[] }
  | { type: "multipart"; parts: unknown[] }
  | { type: "binary"; path: string; content_type?: string | null };

export interface RequestSettings {
  follow_redirects: boolean;
  max_redirects: number;
  /** Session-only. Never serialized by the core, so it is never persisted. */
  accept_invalid_certs: boolean;
  timeout_ms: number;
}

export interface RequestDraft {
  id: string;
  name?: string | null;
  spec_ref?: string | null;
  method: HttpMethod;
  url: string;
  path_values: Record<string, string>;
  query: KeyValue[];
  headers: KeyValue[];
  cookies: KeyValue[];
  auth: AuthConfig;
  body: BodyValue;
  settings: RequestSettings;
}

export interface Timing {
  ttfb_ms: number;
  total_ms: number;
}

export interface Hop {
  status: number;
  from: string;
  to: string;
  credentials_stripped: boolean;
}

export interface ResponseBody {
  bytes: string;
  truncated: boolean;
  reported_length?: number | null;
  content_type?: string | null;
  content_encoding?: string | null;
}

export interface HttpResponse {
  status: number;
  status_text: string;
  headers: [string, string][];
  body: ResponseBody;
  timing: Timing;
  redirects: Hop[];
  insecure: boolean;
}

export interface SentRequest {
  method: string;
  url: string;
  headers: [string, string][];
  body_size: number;
  body_preview?: string | null;
}

export interface Exchange {
  request: SentRequest;
  response: HttpResponse;
}

export interface WorkspaceInfo {
  name: string;
  root: string;
  kind: "project" | "standalone";
  collections: string[];
  environments: string[];
  active_environment?: string | null;
  /** Secret names the active environment expects but the store does not hold. */
  missing_secrets: string[];
}

export interface Collection {
  version: number;
  name: string;
  description?: string | null;
  requests: RequestDraft[];
}

export interface Environment {
  version: number;
  name: string;
  variables: Record<string, string>;
  secrets: string[];
}

export interface HistoryEntry {
  id: number;
  at: number;
  method: string;
  url: string;
  status?: number | null;
  duration_ms?: number | null;
  error?: string | null;
  request: unknown;
  response: unknown;
}

/** Every import carries what it could not honour, so nothing is silently dropped. */
export interface ImportResult<T> {
  value: T;
  warnings: string[];
}

export interface OpenApiSummary {
  title: string;
  version: string;
  servers: string[];
  endpoint_count: number;
}

/** Create a blank request. Mirrors `RequestDraft::new` on the Rust side. */
export function emptyRequest(): RequestDraft {
  return {
    id: crypto.randomUUID(),
    name: null,
    spec_ref: null,
    method: "GET",
    url: "",
    path_values: {},
    query: [],
    headers: [],
    cookies: [],
    auth: { type: "none" },
    body: { type: "none" },
    settings: {
      follow_redirects: false,
      max_redirects: 10,
      accept_invalid_certs: false,
      timeout_ms: 30000,
    },
  };
}
