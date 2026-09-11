import { useEffect, useState } from "react";
import { api } from "../api";
import { applyTheme, currentTheme, type Theme } from "../theme";
import type {
  Collection,
  EndpointSpec,
  HistoryEntry,
  RequestDraft,
  ScanResult,
  WorkspaceInfo,
} from "../types";
import { Collections } from "./Collections";
import { Explorer } from "./Explorer";
import { MethodBadge } from "./MethodBadge";

type Panel = "api" | "collections" | "history";

export function Sidebar({
  workspace,
  onOpenRequest,
  onOpenWorkspace,
  onChanged,
  onImport,
  onWorkspaceChange,
  onManageEnvironments,
  refreshKey,
  scan,
  scanning,
  onScan,
  onEnrich,
  onOpenEndpoint,
}: {
  workspace: WorkspaceInfo | null;
  onOpenRequest: (request: RequestDraft, collection: string) => void;
  onOpenWorkspace: () => void;
  /** Something wrote to the workspace; reload what the sidebar shows. */
  onChanged: () => void;
  onImport: () => void;
  onWorkspaceChange: (info: WorkspaceInfo) => void;
  onManageEnvironments: () => void;
  refreshKey: number;
  scan: ScanResult | null;
  scanning: boolean;
  onScan: () => void;
  onEnrich: () => void;
  onOpenEndpoint: (endpoint: EndpointSpec) => void;
}) {
  // The API tree is the reason RouteLens exists, so it opens first for a project workspace.
  const [panel, setPanel] = useState<Panel>("api");
  const [collections, setCollections] = useState<Collection[]>([]);
  const [history, setHistory] = useState<HistoryEntry[]>([]);
  const [theme, setTheme] = useState<Theme>(currentTheme);

  function toggleTheme() {
    const next: Theme = theme === "dark" ? "light" : "dark";
    applyTheme(next);
    setTheme(next);
  }

  useEffect(() => {
    if (!workspace) {
      setCollections([]);
      setHistory([]);
      return;
    }

    let cancelled = false;

    void (async () => {
      const loaded = await Promise.all(
        workspace.collections.map((name) => api.loadCollection(name).catch(() => null)),
      );
      if (!cancelled) {
        setCollections(loaded.filter((c): c is Collection => c !== null));
      }
    })();

    void api
      .history(50)
      .then((entries) => {
        if (!cancelled) setHistory(entries);
      })
      .catch(() => {});

    return () => {
      cancelled = true;
    };
  }, [workspace, refreshKey]);

  return (
    <aside className="flex w-72 shrink-0 flex-col border-r border-edge bg-panel">
      {/* Workspace header */}
      <div className="border-b border-edge p-3">
        {workspace ? (
          <>
            <div className="flex items-baseline justify-between gap-2">
              <h1 className="truncate font-semibold" title={workspace.root}>
                {workspace.name}
              </h1>
              <button
                onClick={onOpenWorkspace}
                className="shrink-0 text-muted transition hover:text-ink"
                title="Open another workspace"
              >
                Open…
              </button>
            </div>

            <div className="mt-2 flex gap-1">
              <select
                value={workspace.active_environment ?? ""}
                onChange={async (e) => {
                  const name = e.target.value || null;
                  onWorkspaceChange(await api.setEnvironment(name));
                }}
                className="min-w-0 flex-1 rounded border border-edge bg-ground px-2 py-1 outline-none focus:border-accent"
              >
                <option value="">No environment</option>
                {workspace.environments.map((name) => (
                  <option key={name} value={name}>
                    {name}
                  </option>
                ))}
              </select>
              <button
                onClick={onManageEnvironments}
                title="Variables and secrets"
                className="shrink-0 rounded border border-edge bg-ground px-2 font-mono text-[11px] text-muted transition hover:text-ink"
              >
                {"{{ }}"}
              </button>
            </div>

            {workspace.missing_secrets.length > 0 && (
              <p className="mt-2 rounded border border-method-post/30 bg-method-post/5 px-2 py-1 text-[11px] text-method-post">
                Unset {workspace.missing_secrets.length === 1 ? "secret" : "secrets"}:{" "}
                <span className="font-mono">{workspace.missing_secrets.join(", ")}</span>
              </p>
            )}
          </>
        ) : (
          <button
            onClick={onOpenWorkspace}
            className="w-full rounded bg-accent px-3 py-2 font-semibold text-ground transition hover:brightness-110"
          >
            Open a project
          </button>
        )}
      </div>

      {/* Panel switch */}
      <div className="flex shrink-0 border-b border-edge">
        {(["api", "collections", "history"] as Panel[]).map((name) => (
          <button
            key={name}
            onClick={() => setPanel(name)}
            className={`relative flex-1 py-2 capitalize transition
              ${panel === name ? "text-ink" : "text-muted hover:text-ink"}`}
          >
            {name}
            {panel === name && (
              <span className="absolute inset-x-4 bottom-0 h-0.5 rounded-full bg-accent" />
            )}
          </button>
        ))}
      </div>

      <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
        {panel === "api" &&
          (workspace?.kind === "project" ? (
            <Explorer
              scan={scan}
              scanning={scanning}
              onScan={onScan}
              onEnrich={onEnrich}
              onOpenEndpoint={onOpenEndpoint}
            />
          ) : (
            <p className="px-3 py-4 text-muted">
              {workspace
                ? "This workspace has no project attached, so there is nothing to scan."
                : "Open a project to discover its API."}
            </p>
          ))}

        {panel !== "api" && <div className="min-h-0 flex-1 overflow-auto p-2">
        {panel === "collections" &&
          (workspace ? (
            <Collections
              collections={collections}
              onOpenRequest={onOpenRequest}
              onChanged={onChanged}
            />
          ) : (
            <p className="px-2 py-4 text-muted">Open a project to see its collections.</p>
          ))}

        {panel === "history" && (
          <>
            {history.length === 0 && (
              <p className="px-2 py-4 text-muted">Nothing sent yet.</p>
            )}
            {history.map((entry) => (
              <div
                key={entry.id}
                className="flex items-center gap-2 rounded px-2 py-1.5"
                title={entry.error ?? undefined}
              >
                <MethodBadge method={entry.method} className="w-12 shrink-0 text-right" />
                <span className="min-w-0 flex-1 truncate text-muted">{entry.url}</span>
                <span
                  className={`shrink-0 font-mono text-[11px] tabular-nums ${
                    entry.error
                      ? "text-method-delete"
                      : (entry.status ?? 0) < 400
                        ? "text-method-get"
                        : "text-method-delete"
                  }`}
                >
                  {entry.error ? "err" : entry.status}
                </span>
              </div>
            ))}
          </>
        )}
        </div>}
      </div>

      <div className="flex shrink-0 gap-2 border-t border-edge p-2">
        <button
          onClick={onImport}
          disabled={!workspace}
          className="flex-1 rounded bg-raised px-3 py-1.5 transition hover:brightness-125 disabled:opacity-40"
        >
          Import…
        </button>
        <button
          onClick={toggleTheme}
          title={theme === "dark" ? "Switch to light theme" : "Switch to dark theme"}
          className="shrink-0 rounded bg-raised px-3 py-1.5 text-muted transition hover:brightness-125 hover:text-ink"
        >
          {theme === "dark" ? "☀" : "☾"}
        </button>
      </div>
    </aside>
  );
}
