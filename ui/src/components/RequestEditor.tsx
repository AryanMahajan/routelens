import { useState } from "react";
import type { AuthConfig, BodyValue, RequestDraft } from "../types";
import { KeyValueEditor } from "./KeyValueEditor";
import { methodColour } from "./MethodBadge";
import { VariableInput } from "./VariableInput";

const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

type Tab = "params" | "headers" | "body" | "auth" | "settings";

export function RequestEditor({
  request,
  onChange,
  onSend,
  onCurl,
  sending,
}: {
  request: RequestDraft;
  onChange: (request: RequestDraft) => void;
  onSend: () => void;
  /** A cURL command landed in the URL bar; the parent turns it into a request. */
  onCurl: (text: string) => void;
  sending: boolean;
}) {
  const [tab, setTab] = useState<Tab>("params");

  function patch(changes: Partial<RequestDraft>) {
    onChange({ ...request, ...changes });
  }

  // Path parameters are discovered from the URL itself, so the editor always offers a field
  // for every `{placeholder}` present — including ones typed by hand just now.
  // `{{base_url}}` is a variable, not a parameter, so double braces are excluded.
  const pathParams = Array.from(request.url.matchAll(/(?<!\{)\{([^{}]+)\}(?!\})/g)).map(
    (m) => m[1]!,
  );

  const counts: Record<Tab, number> = {
    params: request.query.filter((q) => q.enabled).length,
    headers: request.headers.filter((h) => h.enabled).length,
    body: request.body.type === "none" ? 0 : 1,
    auth: request.auth.type === "none" ? 0 : 1,
    settings: 0,
  };

  return (
    <section className="flex min-h-0 flex-1 flex-col">
      {/* Method, URL, Send */}
      <div className="flex items-center gap-2 border-b border-edge p-3">
        <select
          value={request.method}
          onChange={(e) => patch({ method: e.target.value })}
          className={`shrink-0 rounded border border-edge bg-panel px-2 py-1.5 font-mono text-xs
            font-bold outline-none focus:border-accent ${methodColour(request.method)}`}
        >
          {METHODS.map((m) => (
            <option key={m} value={m} className="bg-panel text-ink">
              {m}
            </option>
          ))}
        </select>

        <VariableInput
          value={request.url}
          onChange={(url) => {
            // Paste a whole cURL command here and it becomes the request — no dialog.
            if (/^\s*curl\s/i.test(url)) {
              onCurl(url);
              return;
            }
            patch({ url });
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !sending) onSend();
          }}
          placeholder="{{base_url}}/api/v1/users — or paste a cURL command"
          className="rounded border border-edge bg-panel px-3 py-1.5 font-mono
            outline-none placeholder:text-muted/60 focus:border-accent"
        />

        <button
          onClick={onSend}
          disabled={sending || !request.url}
          title="Ctrl+Enter"
          className="shrink-0 rounded bg-accent px-4 py-1.5 font-semibold text-ground transition
            hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-40"
        >
          {sending ? "Sending…" : "Send"}
        </button>
      </div>

      {/* Tabs */}
      <div className="flex shrink-0 gap-1 border-b border-edge px-3">
        {(["params", "headers", "body", "auth", "settings"] as Tab[]).map((name) => (
          <button
            key={name}
            onClick={() => setTab(name)}
            className={`relative px-3 py-2 capitalize transition
              ${tab === name ? "text-ink" : "text-muted hover:text-ink"}`}
          >
            {name}
            {counts[name] > 0 && (
              <span className="ml-1.5 rounded-full bg-raised px-1.5 py-0.5 text-[10px] tabular-nums text-muted">
                {counts[name]}
              </span>
            )}
            {tab === name && (
              <span className="absolute inset-x-2 -bottom-px h-0.5 rounded-full bg-accent" />
            )}
          </button>
        ))}
      </div>

      <div className="min-h-0 flex-1 overflow-auto p-3">
        {tab === "params" && (
          <div className="flex flex-col gap-5">
            {pathParams.length > 0 && (
              <div>
                <h3 className="mb-2 text-[11px] font-semibold uppercase tracking-wider text-muted">
                  Path parameters
                </h3>
                <div className="flex flex-col gap-1">
                  {pathParams.map((name) => (
                    <label key={name} className="flex items-center gap-2">
                      <span className="w-2/5 shrink-0 truncate px-2 font-mono text-accent">
                        {name}
                      </span>
                      <input
                        value={request.path_values[name] ?? ""}
                        onChange={(e) =>
                          patch({
                            path_values: {
                              ...request.path_values,
                              [name]: e.target.value,
                            },
                          })
                        }
                        placeholder="required"
                        spellCheck={false}
                        className="min-w-0 flex-1 rounded border border-edge bg-panel px-2 py-1
                          font-mono outline-none placeholder:text-method-delete/60 focus:border-accent"
                      />
                    </label>
                  ))}
                </div>
              </div>
            )}

            <div>
              <h3 className="mb-2 text-[11px] font-semibold uppercase tracking-wider text-muted">
                Query
              </h3>
              <KeyValueEditor rows={request.query} onChange={(query) => patch({ query })} />
            </div>
          </div>
        )}

        {tab === "headers" && (
          <KeyValueEditor
            rows={request.headers}
            onChange={(headers) => patch({ headers })}
            keyPlaceholder="Header-Name"
          />
        )}

        {tab === "body" && <BodyEditor body={request.body} onChange={(body) => patch({ body })} />}

        {tab === "auth" && <AuthEditor auth={request.auth} onChange={(auth) => patch({ auth })} />}

        {tab === "settings" && (
          <SettingsEditor request={request} onChange={patch} />
        )}
      </div>
    </section>
  );
}

function BodyEditor({
  body,
  onChange,
}: {
  body: BodyValue;
  onChange: (body: BodyValue) => void;
}) {
  const content = body.type === "json" || body.type === "text" ? body.content : "";

  return (
    <div className="flex h-full flex-col gap-3">
      <div className="flex shrink-0 gap-1">
        {(["none", "json", "text", "form"] as const).map((kind) => (
          <button
            key={kind}
            onClick={() => {
              if (kind === "none") onChange({ type: "none" });
              else if (kind === "json") onChange({ type: "json", content });
              else if (kind === "text")
                onChange({ type: "text", content, content_type: "text/plain" });
              else onChange({ type: "form", fields: [] });
            }}
            className={`rounded px-2.5 py-1 uppercase tracking-wide transition
              ${body.type === kind ? "bg-accent text-ground" : "bg-raised text-muted hover:text-ink"}`}
          >
            {kind}
          </button>
        ))}
      </div>

      {body.type === "none" && (
        <p className="text-muted italic">
          No body. A body on <code className="font-mono">GET</code> is unusual but permitted —
          the engine sends whatever you describe.
        </p>
      )}

      {(body.type === "json" || body.type === "text") && (
        <textarea
          value={body.content}
          onChange={(e) => onChange({ ...body, content: e.target.value })}
          spellCheck={false}
          placeholder={body.type === "json" ? '{\n  "name": "Aryan"\n}' : ""}
          className="min-h-0 flex-1 resize-none rounded border border-edge bg-panel p-3 font-mono
            leading-relaxed outline-none placeholder:text-muted/50 focus:border-accent"
        />
      )}

      {body.type === "form" && (
        <KeyValueEditor
          rows={body.fields}
          onChange={(fields) => onChange({ type: "form", fields })}
        />
      )}
    </div>
  );
}

function AuthEditor({
  auth,
  onChange,
}: {
  auth: AuthConfig;
  onChange: (auth: AuthConfig) => void;
}) {
  return (
    <div className="flex max-w-xl flex-col gap-3">
      <select
        value={auth.type}
        onChange={(e) => {
          const kind = e.target.value;
          if (kind === "none") onChange({ type: "none" });
          else if (kind === "bearer") onChange({ type: "bearer", token: "" });
          else if (kind === "basic") onChange({ type: "basic", username: "", password: "" });
          else onChange({ type: "api_key", key: "", value: "", location: "header" });
        }}
        className="self-start rounded border border-edge bg-panel px-2 py-1.5 outline-none focus:border-accent"
      >
        <option value="none">No auth</option>
        <option value="bearer">Bearer token</option>
        <option value="basic">Basic</option>
        <option value="api_key">API key</option>
      </select>

      {auth.type === "bearer" && (
        <Field label="Token">
          <input
            value={auth.token}
            onChange={(e) => onChange({ ...auth, token: e.target.value })}
            placeholder="{{secret:api_token}}"
            spellCheck={false}
            className={inputClass}
          />
        </Field>
      )}

      {auth.type === "basic" && (
        <>
          <Field label="Username">
            <input
              value={auth.username}
              onChange={(e) => onChange({ ...auth, username: e.target.value })}
              spellCheck={false}
              className={inputClass}
            />
          </Field>
          <Field label="Password">
            <input
              value={auth.password}
              onChange={(e) => onChange({ ...auth, password: e.target.value })}
              placeholder="{{secret:password}}"
              spellCheck={false}
              className={inputClass}
            />
          </Field>
        </>
      )}

      {auth.type === "api_key" && (
        <>
          <Field label="Key">
            <input
              value={auth.key}
              onChange={(e) => onChange({ ...auth, key: e.target.value })}
              placeholder="X-API-Key"
              spellCheck={false}
              className={inputClass}
            />
          </Field>
          <Field label="Value">
            <input
              value={auth.value}
              onChange={(e) => onChange({ ...auth, value: e.target.value })}
              placeholder="{{secret:api_key}}"
              spellCheck={false}
              className={inputClass}
            />
          </Field>
          <Field label="Send in">
            <select
              value={auth.location}
              onChange={(e) =>
                onChange({ ...auth, location: e.target.value as typeof auth.location })
              }
              className={inputClass}
            >
              <option value="header">Header</option>
              <option value="query">Query</option>
              <option value="cookie">Cookie</option>
            </select>
          </Field>
        </>
      )}

      {auth.type !== "none" && (
        <p className="text-muted">
          Reference a secret as{" "}
          <code className="rounded bg-raised px-1 font-mono text-accent">
            {"{{secret:name}}"}
          </code>
          . The value stays in the private tier and is substituted moments before sending, so
          it never reaches a file you could commit.
        </p>
      )}
    </div>
  );
}

function SettingsEditor({
  request,
  onChange,
}: {
  request: RequestDraft;
  onChange: (changes: Partial<RequestDraft>) => void;
}) {
  const s = request.settings;
  const set = (patch: Partial<typeof s>) => onChange({ settings: { ...s, ...patch } });

  return (
    <div className="flex max-w-xl flex-col gap-4">
      <label className="flex items-start gap-2">
        <input
          type="checkbox"
          checked={s.follow_redirects}
          onChange={(e) => set({ follow_redirects: e.target.checked })}
          className="mt-0.5 size-3.5 accent-accent"
        />
        <span>
          Follow redirects
          <span className="block text-muted">
            Off by default so you see what the server actually returned. Credentials are
            dropped automatically if a redirect crosses origins.
          </span>
        </span>
      </label>

      <label className="flex items-start gap-2">
        <input
          type="checkbox"
          checked={s.accept_invalid_certs}
          onChange={(e) => set({ accept_invalid_certs: e.target.checked })}
          className="mt-0.5 size-3.5 accent-method-delete"
        />
        <span>
          Skip certificate verification
          <span className="block text-muted">
            For local development against self-signed certificates. Applies to this request
            only, and is never saved — reopening the workspace clears it.
          </span>
        </span>
      </label>

      <Field label="Timeout (ms)">
        <input
          type="number"
          value={s.timeout_ms}
          min={100}
          step={500}
          onChange={(e) => set({ timeout_ms: Number(e.target.value) || 30000 })}
          className={inputClass}
        />
      </Field>
    </div>
  );
}

const inputClass =
  "w-full rounded border border-edge bg-panel px-2 py-1.5 font-mono outline-none " +
  "placeholder:text-muted/60 focus:border-accent";

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="flex flex-col gap-1">
      <span className="text-[11px] font-semibold uppercase tracking-wider text-muted">
        {label}
      </span>
      {children}
    </label>
  );
}
