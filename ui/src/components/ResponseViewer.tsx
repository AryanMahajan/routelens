import { useMemo, useState } from "react";
import type { Exchange } from "../types";

type Tab = "body" | "headers" | "timing" | "sent";

export function ResponseViewer({
  exchange,
  error,
  sending,
}: {
  exchange: Exchange | null;
  error: string | null;
  sending: boolean;
}) {
  const [tab, setTab] = useState<Tab>("body");
  const [raw, setRaw] = useState(false);

  const pretty = useMemo(() => {
    if (!exchange) return null;
    try {
      return JSON.stringify(JSON.parse(exchange.response.body.bytes), null, 2);
    } catch {
      return null;
    }
  }, [exchange]);

  if (sending) {
    return <Placeholder>Sending…</Placeholder>;
  }

  if (error) {
    return (
      <div className="flex min-h-0 flex-1 flex-col overflow-auto p-4">
        <h3 className="mb-2 font-semibold text-method-delete">Request failed</h3>
        <pre className="whitespace-pre-wrap rounded border border-method-delete/30 bg-method-delete/5 p-3 font-mono text-method-delete">
          {error}
        </pre>
      </div>
    );
  }

  if (!exchange) {
    return <Placeholder>Send a request to see the response.</Placeholder>;
  }

  const { response } = exchange;
  const statusColour =
    response.status < 300
      ? "text-method-get"
      : response.status < 400
        ? "text-method-put"
        : "text-method-delete";

  const size = response.body.reported_length ?? response.body.bytes.length;

  return (
    <section className="flex min-h-0 flex-1 flex-col border-t border-edge">
      {/* Status line */}
      <div className="flex shrink-0 flex-wrap items-center gap-x-4 gap-y-1 border-b border-edge px-3 py-2">
        <span className={`font-mono font-bold ${statusColour}`}>
          {response.status} {response.status_text}
        </span>
        <span className="text-muted tabular-nums">{response.timing.total_ms} ms</span>
        <span className="text-muted tabular-nums">{formatBytes(size)}</span>

        {response.insecure && (
          <span
            className="rounded bg-method-delete/15 px-1.5 py-0.5 text-[11px] text-method-delete"
            title="Certificate verification was disabled for this request"
          >
            insecure
          </span>
        )}
        {response.body.truncated && (
          <span className="rounded bg-method-post/15 px-1.5 py-0.5 text-[11px] text-method-post">
            truncated
          </span>
        )}
        {response.body.content_encoding && (
          <span
            className="rounded bg-raised px-1.5 py-0.5 text-[11px] text-muted"
            title="Decoded automatically; the raw bytes on the wire were compressed"
          >
            {response.body.content_encoding}
          </span>
        )}
      </div>

      {/* Redirect chain — worth showing, because credentials may have been dropped mid-way. */}
      {response.redirects.length > 0 && (
        <div className="shrink-0 border-b border-edge bg-panel/60 px-3 py-2">
          <h4 className="mb-1 text-[11px] font-semibold uppercase tracking-wider text-muted">
            Redirects
          </h4>
          <ol className="flex flex-col gap-0.5">
            {response.redirects.map((hop, i) => (
              <li key={i} className="font-mono text-[11px] text-muted">
                <span className="text-method-put">{hop.status}</span> → {hop.to}
                {hop.credentials_stripped && (
                  <span className="ml-2 text-method-post">credentials dropped (cross-origin)</span>
                )}
              </li>
            ))}
          </ol>
        </div>
      )}

      {/* Tabs */}
      <div className="flex shrink-0 items-center gap-1 border-b border-edge px-3">
        {(["body", "headers", "timing", "sent"] as Tab[]).map((name) => (
          <button
            key={name}
            onClick={() => setTab(name)}
            className={`relative px-3 py-2 capitalize transition
              ${tab === name ? "text-ink" : "text-muted hover:text-ink"}`}
          >
            {name === "sent" ? "sent request" : name}
            {tab === name && (
              <span className="absolute inset-x-2 -bottom-px h-0.5 rounded-full bg-accent" />
            )}
          </button>
        ))}

        {tab === "body" && pretty && (
          <button
            onClick={() => setRaw(!raw)}
            className="ml-auto rounded px-2 py-1 text-muted transition hover:bg-raised hover:text-ink"
          >
            {raw ? "Pretty" : "Raw"}
          </button>
        )}
      </div>

      <div className="min-h-0 flex-1 overflow-auto">
        {tab === "body" && (
          <pre className="whitespace-pre-wrap break-all p-3 font-mono leading-relaxed">
            {(raw ? null : pretty) ?? response.body.bytes}
          </pre>
        )}

        {tab === "headers" && <HeaderTable headers={response.headers} />}

        {tab === "timing" && (
          <dl className="grid max-w-sm grid-cols-2 gap-x-6 gap-y-2 p-4 font-mono">
            <dt className="text-muted">Time to first byte</dt>
            <dd className="tabular-nums">{response.timing.ttfb_ms} ms</dd>
            <dt className="text-muted">Total</dt>
            <dd className="tabular-nums">{response.timing.total_ms} ms</dd>
            <dt className="text-muted">Body size</dt>
            <dd className="tabular-nums">{formatBytes(size)}</dd>
            <dd className="col-span-2 mt-2 font-sans text-muted">
              The DNS / TCP / TLS breakdown needs a custom connector and is not wired up yet.
            </dd>
          </dl>
        )}

        {tab === "sent" && (
          <div className="p-3">
            <p className="mb-3 text-muted">
              What actually went out — variables resolved, auth applied, disabled rows gone.
            </p>
            <p className="mb-3 font-mono">
              <span className="font-bold text-accent">{exchange.request.method}</span>{" "}
              {exchange.request.url}
            </p>
            <HeaderTable headers={exchange.request.headers} />
            {exchange.request.body_preview && (
              <pre className="mt-3 whitespace-pre-wrap break-all rounded border border-edge bg-panel p-3 font-mono">
                {exchange.request.body_preview}
              </pre>
            )}
          </div>
        )}
      </div>
    </section>
  );
}

function HeaderTable({ headers }: { headers: [string, string][] }) {
  if (headers.length === 0) {
    return <p className="p-4 text-muted italic">No headers.</p>;
  }
  return (
    <table className="w-full border-collapse font-mono">
      <tbody>
        {headers.map(([name, value], i) => (
          <tr key={i} className="border-b border-edge/50 align-top last:border-0">
            <td className="w-1/3 py-1.5 pl-3 pr-4 text-muted">{name}</td>
            <td className="break-all py-1.5 pr-3">{value}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function Placeholder({ children }: { children: React.ReactNode }) {
  return (
    <section className="flex min-h-0 flex-1 items-center justify-center border-t border-edge text-muted">
      {children}
    </section>
  );
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}
