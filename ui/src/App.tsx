import { useCallback, useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, CoreError } from "./api";
import { emptyRequest, type Exchange, type RequestDraft, type WorkspaceInfo } from "./types";
import { ImportDialog } from "./components/ImportDialog";
import { RequestEditor } from "./components/RequestEditor";
import { ResponseViewer } from "./components/ResponseViewer";
import { Sidebar } from "./components/Sidebar";

export default function App() {
  const [workspace, setWorkspace] = useState<WorkspaceInfo | null>(null);
  const [request, setRequest] = useState<RequestDraft>(emptyRequest);
  const [exchange, setExchange] = useState<Exchange | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const [importing, setImporting] = useState(false);
  const [saveTarget, setSaveTarget] = useState("Saved");
  // Bumped to make the sidebar reload after something writes to the workspace.
  const [refreshKey, setRefreshKey] = useState(0);

  const refresh = useCallback(() => setRefreshKey((n) => n + 1), []);

  // A workspace may already be open if the window was reloaded during development.
  useEffect(() => {
    void api
      .workspaceInfo()
      .then(setWorkspace)
      .catch(() => {});
  }, []);

  async function openWorkspace() {
    const picked = await open({ directory: true, multiple: false });
    if (typeof picked !== "string") return;

    try {
      const name = picked.split(/[/\\]/).filter(Boolean).pop() ?? "workspace";
      setWorkspace(await api.openOrCreateWorkspace(picked, name));
      refresh();
    } catch (e) {
      setError(e instanceof CoreError ? e.message : String(e));
    }
  }

  async function send() {
    setSending(true);
    setError(null);
    try {
      setExchange(await api.send(request));
    } catch (e) {
      setExchange(null);
      setError(e instanceof CoreError ? e.message : String(e));
    } finally {
      setSending(false);
      // The send was recorded in history whether it succeeded or not.
      refresh();
    }
  }

  async function save() {
    if (!workspace) return;
    const named: RequestDraft = {
      ...request,
      name: request.name ?? `${request.method} ${request.url}`,
    };
    try {
      await api.saveRequest(saveTarget, named);
      setRequest(named);
      setWorkspace(await api.workspaceInfo());
      refresh();
    } catch (e) {
      setError(e instanceof CoreError ? e.message : String(e));
    }
  }

  // Ctrl/Cmd+Enter sends from anywhere, including inside the body editor.
  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if ((event.ctrlKey || event.metaKey) && event.key === "Enter") {
        event.preventDefault();
        if (!sending && request.url) void send();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  return (
    <div className="flex h-full">
      <Sidebar
        workspace={workspace}
        refreshKey={refreshKey}
        onOpenWorkspace={openWorkspace}
        onImport={() => setImporting(true)}
        onWorkspaceChange={setWorkspace}
        onOpenRequest={(saved) => {
          setRequest(saved);
          setExchange(null);
          setError(null);
        }}
      />

      <main className="flex min-w-0 flex-1 flex-col">
        <div className="flex shrink-0 items-center gap-2 border-b border-edge px-3 py-2">
          <input
            value={request.name ?? ""}
            onChange={(e) => setRequest({ ...request, name: e.target.value || null })}
            placeholder="Untitled request"
            className="min-w-0 flex-1 rounded border border-transparent bg-transparent px-2 py-1
              font-semibold outline-none placeholder:text-muted/60 focus:border-edge focus:bg-panel"
          />

          <input
            value={saveTarget}
            onChange={(e) => setSaveTarget(e.target.value)}
            title="Collection to save into"
            className="w-32 shrink-0 rounded border border-edge bg-panel px-2 py-1 outline-none focus:border-accent"
          />
          <button
            onClick={save}
            disabled={!workspace || !request.url}
            className="shrink-0 rounded bg-raised px-3 py-1 transition hover:brightness-125 disabled:opacity-40"
          >
            Save
          </button>
        </div>

        <RequestEditor
          request={request}
          onChange={setRequest}
          onSend={send}
          sending={sending}
        />

        <div className="flex min-h-0 flex-[1.2] flex-col">
          <ResponseViewer exchange={exchange} error={error} sending={sending} />
        </div>
      </main>

      {importing && (
        <ImportDialog
          onClose={() => setImporting(false)}
          onCollectionsChanged={async () => {
            setWorkspace(await api.workspaceInfo());
            refresh();
          }}
          onImported={(imported) => {
            setRequest(imported);
            setExchange(null);
            setError(null);
          }}
        />
      )}
    </div>
  );
}
