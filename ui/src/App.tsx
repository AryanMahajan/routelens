import { useCallback, useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, CoreError } from "./api";
import {
  emptyRequest,
  type EndpointSpec,
  type Exchange,
  type RequestDraft,
  type ScanResult,
  type WorkspaceInfo,
} from "./types";
import { VariablesContext } from "./variables";
import { EnrichDialog } from "./components/EnrichDialog";
import { EnvironmentDialog } from "./components/EnvironmentDialog";
import { ImportDialog } from "./components/ImportDialog";
import { RequestEditor } from "./components/RequestEditor";
import { ResponseViewer } from "./components/ResponseViewer";
import { Sidebar } from "./components/Sidebar";
import { TabStrip } from "./components/TabStrip";

/** One open request. Everything a tab shows lives here, so switching tabs loses nothing. */
interface Tab {
  id: string;
  request: RequestDraft;
  exchange: Exchange | null;
  error: string | null;
  sending: boolean;
  /** The request as last opened or saved, for the unsaved-changes dot. */
  saved: string;
  /** The collection this was opened from or last saved into, so Save goes back there. */
  collection: string | null;
}

function newTab(request: RequestDraft, collection: string | null = null): Tab {
  return {
    id: crypto.randomUUID(),
    request,
    exchange: null,
    error: null,
    sending: false,
    saved: JSON.stringify(request),
    collection,
  };
}

function describe(e: unknown): string {
  return e instanceof CoreError ? e.message : String(e);
}

export default function App() {
  const [workspace, setWorkspace] = useState<WorkspaceInfo | null>(null);
  const [tabs, setTabs] = useState<Tab[]>(() => [newTab(emptyRequest())]);
  const [activeId, setActiveId] = useState<string>(() => "");
  const [importing, setImporting] = useState(false);
  const [managingEnvironments, setManagingEnvironments] = useState(false);
  const [enriching, setEnriching] = useState(false);
  const [saveTarget, setSaveTarget] = useState("Saved");
  const [scan, setScan] = useState<ScanResult | null>(null);
  const [scanning, setScanning] = useState(false);
  const [variableNames, setVariableNames] = useState<string[]>([]);
  // Bumped to make the sidebar reload after something writes to the workspace.
  const [refreshKey, setRefreshKey] = useState(0);

  const refresh = useCallback(() => setRefreshKey((n) => n + 1), []);

  // The first tab is created before state exists, so adopt it once.
  useEffect(() => {
    if (!activeId && tabs[0]) setActiveId(tabs[0].id);
  }, [activeId, tabs]);

  const active = tabs.find((t) => t.id === activeId) ?? tabs[0] ?? null;

  function updateTab(id: string, changes: Partial<Tab>) {
    setTabs((current) => current.map((t) => (t.id === id ? { ...t, ...changes } : t)));
  }

  function openTab(
    request: RequestDraft,
    matchOn?: (tab: Tab) => boolean,
    collection: string | null = null,
  ) {
    // Re-use an untouched tab that already shows the same thing, otherwise open a new one.
    const existing = matchOn ? tabs.find((t) => matchOn(t) && t.saved === JSON.stringify(t.request)) : undefined;
    if (existing) {
      setActiveId(existing.id);
      return;
    }
    // A pristine blank tab is replaced rather than left behind.
    const blank = active && !active.request.url && active.saved === JSON.stringify(active.request);
    const tab = newTab(request, collection);
    setTabs((current) => (blank ? current.map((t) => (t.id === active.id ? tab : t)) : [...current, tab]));
    setActiveId(tab.id);
  }

  function closeTab(id: string) {
    const tab = tabs.find((t) => t.id === id);
    if (!tab) return;
    if (tab.saved !== JSON.stringify(tab.request) && !window.confirm("Close this tab and discard its unsaved changes?")) {
      return;
    }
    const index = tabs.findIndex((t) => t.id === id);
    const remaining = tabs.filter((t) => t.id !== id);
    if (remaining.length === 0) {
      const blank = newTab(emptyRequest());
      setTabs([blank]);
      setActiveId(blank.id);
      return;
    }
    setTabs(remaining);
    if (id === activeId) {
      const neighbour = remaining[Math.min(index, remaining.length - 1)]!;
      setActiveId(neighbour.id);
    }
  }

  const reloadVariables = useCallback(() => {
    void api
      .variableNames()
      .then(setVariableNames)
      .catch(() => setVariableNames([]));
  }, []);

  // A workspace may already be open if the window was reloaded during development.
  useEffect(() => {
    void api
      .workspaceInfo()
      .then(setWorkspace)
      .catch(() => {});
  }, []);

  useEffect(() => {
    if (workspace) reloadVariables();
    else setVariableNames([]);
  }, [workspace, reloadVariables]);

  async function openWorkspace() {
    const picked = await open({ directory: true, multiple: false });
    if (typeof picked !== "string") return;

    try {
      const name = picked.split(/[/\\]/).filter(Boolean).pop() ?? "workspace";
      const info = await api.openOrCreateWorkspace(picked, name);
      setWorkspace(info);
      setScan(null);
      refresh();
      // Discovery is the point of opening a project, so do it without being asked.
      if (info.kind === "project") void runScan();
    } catch (e) {
      if (active) updateTab(active.id, { error: describe(e) });
    }
  }

  async function runScan() {
    setScanning(true);
    try {
      const result = await api.scanProject();
      setScan(result);
      // A scan may have seeded the environment's base_url.
      setWorkspace(await api.workspaceInfo());
    } catch (e) {
      if (active) updateTab(active.id, { error: describe(e) });
    } finally {
      setScanning(false);
    }
  }

  async function openEndpoint(endpoint: EndpointSpec) {
    try {
      const request = await api.openEndpoint(endpoint.id);
      openTab(request, (t) => t.request.spec_ref === request.spec_ref);
    } catch (e) {
      if (active) updateTab(active.id, { error: describe(e) });
    }
  }

  async function send(tab: Tab) {
    updateTab(tab.id, { sending: true, error: null });
    try {
      const exchange = await api.send(tab.request);
      updateTab(tab.id, { exchange, sending: false });
    } catch (e) {
      updateTab(tab.id, { exchange: null, error: describe(e), sending: false });
    } finally {
      // The send was recorded in history whether it succeeded or not.
      refresh();
    }
  }

  async function save(tab: Tab) {
    if (!workspace) return;
    const named: RequestDraft = {
      ...tab.request,
      name: tab.request.name ?? `${tab.request.method} ${tab.request.url}`,
    };
    const target = tab.collection ?? (saveTarget.trim() || "Saved");
    try {
      await api.saveRequest(target, named);
      updateTab(tab.id, { request: named, saved: JSON.stringify(named), collection: target });
      setWorkspace(await api.workspaceInfo());
      refresh();
    } catch (e) {
      updateTab(tab.id, { error: describe(e) });
    }
  }

  async function importCurl(tab: Tab, text: string) {
    try {
      const result = await api.importCurl(text);
      const request = { ...result.value, id: tab.request.id };
      updateTab(tab.id, {
        request,
        exchange: null,
        error: result.warnings.length > 0 ? `Imported with warnings: ${result.warnings.join("; ")}` : null,
      });
    } catch (e) {
      updateTab(tab.id, { error: describe(e) });
    }
  }

  // Ctrl/Cmd+Enter sends, Ctrl+S saves, Ctrl+T opens a tab, Ctrl+W closes one — from
  // anywhere, including inside an input.
  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (!(event.ctrlKey || event.metaKey)) return;
      if (event.key === "Enter" && active && !active.sending && active.request.url) {
        event.preventDefault();
        void send(active);
      } else if (event.key.toLowerCase() === "s") {
        event.preventDefault();
        if (active && workspace && active.request.url) void save(active);
      } else if (event.key.toLowerCase() === "t") {
        event.preventDefault();
        openTab(emptyRequest());
      } else if (event.key.toLowerCase() === "w" && active) {
        event.preventDefault();
        closeTab(active.id);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  return (
    <VariablesContext.Provider value={variableNames}>
      <div className="flex h-full">
        <Sidebar
          workspace={workspace}
          refreshKey={refreshKey}
          onOpenWorkspace={openWorkspace}
          onImport={() => setImporting(true)}
          onWorkspaceChange={setWorkspace}
          onManageEnvironments={() => setManagingEnvironments(true)}
          scan={scan}
          scanning={scanning}
          onScan={runScan}
          onEnrich={() => setEnriching(true)}
          onOpenEndpoint={openEndpoint}
          onChanged={refresh}
          onOpenRequest={(saved, collection) =>
            openTab(saved, (t) => t.request.id === saved.id, collection)
          }
        />

        <main className="flex min-w-0 flex-1 flex-col">
          <TabStrip
            tabs={tabs.map((t) => ({
              id: t.id,
              request: t.request,
              dirty: t.saved !== JSON.stringify(t.request),
            }))}
            activeId={active?.id ?? null}
            onActivate={setActiveId}
            onClose={closeTab}
            onNew={() => openTab(emptyRequest())}
          />

          {active && (
            <>
              <div className="flex shrink-0 items-center gap-2 border-b border-edge px-3 py-2">
                <input
                  value={active.request.name ?? ""}
                  onChange={(e) =>
                    updateTab(active.id, {
                      request: { ...active.request, name: e.target.value || null },
                    })
                  }
                  placeholder="Untitled request"
                  className="min-w-0 flex-1 rounded border border-transparent bg-transparent px-2 py-1
                    font-semibold outline-none placeholder:text-muted/60 focus:border-edge focus:bg-panel"
                />

                <input
                  value={active.collection ?? saveTarget}
                  onChange={(e) => {
                    setSaveTarget(e.target.value);
                    if (active.collection !== null) updateTab(active.id, { collection: null });
                  }}
                  list="collection-names"
                  title="Collection to save into"
                  className="w-32 shrink-0 rounded border border-edge bg-panel px-2 py-1 outline-none focus:border-accent"
                />
                <datalist id="collection-names">
                  {workspace?.collections.map((name) => (
                    <option key={name} value={name} />
                  ))}
                </datalist>
                <button
                  onClick={() => save(active)}
                  disabled={!workspace || !active.request.url}
                  title="Ctrl+S"
                  className="shrink-0 rounded bg-raised px-3 py-1 transition hover:brightness-125 disabled:opacity-40"
                >
                  Save
                </button>
              </div>

              <RequestEditor
                key={active.id}
                request={active.request}
                onChange={(request) => updateTab(active.id, { request })}
                onSend={() => send(active)}
                onCurl={(text) => importCurl(active, text)}
                sending={active.sending}
              />

              <div className="flex min-h-0 flex-[1.2] flex-col">
                <ResponseViewer
                  exchange={active.exchange}
                  error={active.error}
                  sending={active.sending}
                />
              </div>
            </>
          )}
        </main>

        {importing && (
          <ImportDialog
            onClose={() => setImporting(false)}
            onCollectionsChanged={async () => {
              setWorkspace(await api.workspaceInfo());
              refresh();
            }}
            onImported={(imported) => openTab(imported)}
          />
        )}

        {enriching && (
          <EnrichDialog
            onClose={() => setEnriching(false)}
            onEnriched={(result) => {
              setScan(result);
              // Endpoints may have changed identity; tabs opened from the old scan still work.
            }}
          />
        )}

        {managingEnvironments && workspace && (
          <EnvironmentDialog
            workspace={workspace}
            onClose={() => {
              setManagingEnvironments(false);
              reloadVariables();
            }}
            onWorkspaceChange={(info) => {
              setWorkspace(info);
              reloadVariables();
            }}
          />
        )}
      </div>
    </VariablesContext.Provider>
  );
}
