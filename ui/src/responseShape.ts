/**
 * What a response looks like, for picking extract and assert targets without reading the
 * API's documentation again: every JSON path in the body with the value found there, and
 * the header names.
 *
 * Built from a real exchange — this step's last run, or failing that the most recent
 * history entry for the same endpoint — never from a guess. No React, no Tauri.
 */

import type { HistoryEntry, HttpResponse } from "./types";

export interface PathEntry {
  /** As the extractor reads it: `user.id`, `items[0].name`, `["odd key"]`. */
  path: string;
  /** A one-line rendering of what is there. */
  preview: string;
  /** Scalars can be compared and extracted as text; containers are for drilling into. */
  scalar: boolean;
  /** The raw value as text, for filling an assertion's expected value. */
  text: string;
}

export interface ResponseShape {
  status: number;
  headers: string[];
  paths: PathEntry[];
  /** Where it came from. */
  source: "run" | "history";
  /** For history: when that response was received. */
  at?: number;
}

/** No more than this many paths are listed; a huge body is not a shape, it is data. */
export const PATH_LIMIT = 400;
/** Arrays list their first few elements; the rest have the same shape. */
const ARRAY_SAMPLE = 3;

export function shapeOf(response: HttpResponse, source: "run" | "history", at?: number): ResponseShape {
  return {
    status: response.status,
    headers: [...new Set(response.headers.map(([name]) => name.toLowerCase()))].sort(),
    paths: pathsOf(response.body.bytes),
    source,
    at,
  };
}

/** Every path in a JSON document. Not JSON: no paths (body text is a different source). */
export function pathsOf(text: string): PathEntry[] {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return [];
  }
  const out: PathEntry[] = [];
  const visit = (node: unknown, path: string) => {
    if (out.length >= PATH_LIMIT) return;
    if (path) out.push(entry(path, node));
    if (Array.isArray(node)) {
      node.slice(0, ARRAY_SAMPLE).forEach((item, i) => visit(item, `${path}[${i}]`));
    } else if (node && typeof node === "object") {
      for (const [key, child] of Object.entries(node)) {
        visit(child, path ? `${path}${segment(key)}` : segment(key).replace(/^\./, ""));
      }
    }
  };
  visit(value, "");
  return out;
}

/** `.key` for a plain identifier, `["some key"]` otherwise, matching the extractor. */
function segment(key: string): string {
  return /^[A-Za-z_][\w-]*$/.test(key) ? `.${key}` : `[${JSON.stringify(key)}]`;
}

function entry(path: string, node: unknown): PathEntry {
  if (Array.isArray(node)) {
    return { path, preview: `[${node.length} item${node.length === 1 ? "" : "s"}]`, scalar: false, text: JSON.stringify(node) };
  }
  if (node && typeof node === "object") {
    const n = Object.keys(node).length;
    return { path, preview: `{${n} key${n === 1 ? "" : "s"}}`, scalar: false, text: JSON.stringify(node) };
  }
  // Scalars as the extractor renders them: strings bare, everything else as JSON.
  const text = typeof node === "string" ? node : JSON.stringify(node);
  const preview = typeof node === "string" ? JSON.stringify(node) : text;
  return { path, preview: preview.length > 60 ? `${preview.slice(0, 57)}…` : preview, scalar: true, text };
}

/**
 * The newest history entry that was this request: same method, and a URL that the
 * request's template — `{{base_url}}/users/{user_id}` — could have produced. Variables
 * match anything, path placeholders match one segment.
 */
export function matchingHistory(entries: HistoryEntry[], method: string, url: string): HistoryEntry | null {
  const pattern = templatePattern(url);
  const found = entries
    .filter((e) => e.method.toUpperCase() === method.toUpperCase() && e.response && pattern.test(e.url))
    .sort((a, b) => b.at - a.at);
  return found[0] ?? null;
}

export function templatePattern(url: string): RegExp {
  const escaped = url
    .split(/(\{\{[^}]*\}\}|\{[^{}]+\})/)
    .map((part) =>
      part.startsWith("{{") ? ".*" : part.startsWith("{") ? "[^/?#]+" : part.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"),
    )
    .join("");
  // A trailing query string on what was sent does not stop it being the same endpoint.
  return new RegExp(`^${escaped}(\\?.*)?$`);
}

/** A history row's response, which was stored as the response object itself. */
export function responseOf(entry: HistoryEntry): HttpResponse | null {
  const r = entry.response as Partial<HttpResponse> | null;
  if (!r || typeof r !== "object" || typeof r.status !== "number" || !r.body || typeof r.body.bytes !== "string") {
    return null;
  }
  return { ...r, headers: r.headers ?? [], redirects: r.redirects ?? [] } as HttpResponse;
}

/** `items[0].name` → `name`; `user.id` → `user_id` would be presumptuous, so the last part. */
export function suggestName(path: string): string {
  const last = path.replace(/\[\d+\]$/g, "").split(/\.|\[/).filter(Boolean).pop() ?? "";
  return last.replace(/^"|"\]?$/g, "").replace(/[^\w.-]/g, "_");
}
