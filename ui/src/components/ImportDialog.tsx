import { useState } from "react";
import { api, CoreError } from "../api";
import type { RequestDraft } from "../types";

type Kind = "curl" | "raw" | "openapi";

const PLACEHOLDERS: Record<Kind, string> = {
  curl: `curl 'https://api.example.com/users?page=2' \\
  -H 'Authorization: Bearer token' \\
  -H 'Content-Type: application/json' \\
  --data-raw '{"name":"Aryan"}'`,
  raw: `POST /api/v1/users HTTP/1.1
Host: api.example.com
Content-Type: application/json

{"name": "Aryan"}`,
  openapi: `{
  "openapi": "3.0.0",
  "info": { "title": "Petstore", "version": "1.0" },
  "servers": [{ "url": "https://api.example.com" }],
  "paths": { "/pets": { "get": { "summary": "List pets" } } }
}`,
};

const BLURBS: Record<Kind, string> = {
  curl: "Paste a command copied from browser devtools. Quoting, line continuations and $'…' escapes are all handled.",
  raw: "Paste a raw request from a log, a proxy, or a .http file.",
  openapi: "OpenAPI 3.1, 3.0, or Swagger 2.0 — as JSON or YAML. Every operation becomes a saved request.",
};

export function ImportDialog({
  onClose,
  onImported,
  onCollectionsChanged,
}: {
  onClose: () => void;
  onImported: (request: RequestDraft) => void;
  onCollectionsChanged: () => void;
}) {
  const [kind, setKind] = useState<Kind>("curl");
  const [text, setText] = useState("");
  const [collection, setCollection] = useState("Imported");
  const [warnings, setWarnings] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function run() {
    setBusy(true);
    setError(null);
    setWarnings([]);

    try {
      if (kind === "openapi") {
        const result = await api.importOpenApi(text, collection);
        setWarnings(result.warnings);
        onCollectionsChanged();
        // Warnings are worth reading, so the dialog stays open when there are any.
        if (result.warnings.length === 0) onClose();
      } else {
        const result =
          kind === "curl" ? await api.importCurl(text) : await api.importRawHttp(text);
        setWarnings(result.warnings);
        onImported(result.value);
        if (result.warnings.length === 0) onClose();
      }
    } catch (e) {
      setError(e instanceof CoreError ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6"
      onClick={onClose}
    >
      <div
        className="flex max-h-full w-full max-w-3xl flex-col overflow-hidden rounded-lg border border-edge bg-panel shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="flex shrink-0 items-center justify-between border-b border-edge px-4 py-3">
          <h2 className="font-semibold">Import</h2>
          <button onClick={onClose} className="text-muted transition hover:text-ink">
            ✕
          </button>
        </header>

        <div className="flex shrink-0 gap-1 border-b border-edge px-4 py-2">
          {(["curl", "raw", "openapi"] as Kind[]).map((name) => (
            <button
              key={name}
              onClick={() => {
                setKind(name);
                setWarnings([]);
                setError(null);
              }}
              className={`rounded px-3 py-1 transition
                ${kind === name ? "bg-accent text-ground" : "bg-raised text-muted hover:text-ink"}`}
            >
              {name === "raw" ? "Raw HTTP" : name === "curl" ? "cURL" : "OpenAPI"}
            </button>
          ))}
        </div>

        <div className="min-h-0 flex-1 overflow-auto p-4">
          <p className="mb-3 text-muted">{BLURBS[kind]}</p>

          <textarea
            value={text}
            onChange={(e) => setText(e.target.value)}
            placeholder={PLACEHOLDERS[kind]}
            spellCheck={false}
            rows={12}
            className="w-full resize-y rounded border border-edge bg-ground p-3 font-mono
              leading-relaxed outline-none placeholder:text-muted/40 focus:border-accent"
          />

          {kind === "openapi" && (
            <label className="mt-3 flex items-center gap-2">
              <span className="text-muted">Save as collection</span>
              <input
                value={collection}
                onChange={(e) => setCollection(e.target.value)}
                className="rounded border border-edge bg-ground px-2 py-1 font-mono outline-none focus:border-accent"
              />
            </label>
          )}

          {error && (
            <p className="mt-3 rounded border border-method-delete/30 bg-method-delete/5 px-3 py-2 text-method-delete">
              {error}
            </p>
          )}

          {warnings.length > 0 && (
            <div className="mt-3 rounded border border-method-post/30 bg-method-post/5 px-3 py-2">
              <h3 className="mb-1 font-semibold text-method-post">
                Imported, with {warnings.length}{" "}
                {warnings.length === 1 ? "note" : "notes"}
              </h3>
              <ul className="flex list-disc flex-col gap-1 pl-4 text-method-post/90">
                {warnings.map((warning, i) => (
                  <li key={i}>{warning}</li>
                ))}
              </ul>
            </div>
          )}
        </div>

        <footer className="flex shrink-0 justify-end gap-2 border-t border-edge px-4 py-3">
          <button
            onClick={onClose}
            className="rounded px-3 py-1.5 text-muted transition hover:bg-raised hover:text-ink"
          >
            {warnings.length > 0 ? "Done" : "Cancel"}
          </button>
          <button
            onClick={run}
            disabled={busy || !text.trim()}
            className="rounded bg-accent px-4 py-1.5 font-semibold text-ground transition
              hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-40"
          >
            {busy ? "Importing…" : "Import"}
          </button>
        </footer>
      </div>
    </div>
  );
}
