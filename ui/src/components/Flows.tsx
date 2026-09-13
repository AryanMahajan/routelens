import { useState } from "react";
import { api, CoreError } from "../api";

/**
 * Saved flows — one file each under `.routelens/flows/`, committed with the project so
 * whoever clones it gets the tests too.
 */
export function Flows({
  flows,
  onOpen,
  onNew,
  onChanged,
}: {
  flows: string[];
  onOpen: (name: string) => void;
  onNew: () => void;
  /** Something wrote to the workspace; reload what the sidebar shows. */
  onChanged: () => void;
}) {
  const [error, setError] = useState<string | null>(null);

  async function run(action: () => Promise<void>) {
    try {
      await action();
      setError(null);
      onChanged();
    } catch (e) {
      setError(e instanceof CoreError ? e.message : String(e));
    }
  }

  function rename(name: string) {
    const to = window.prompt("Rename flow", name)?.trim();
    if (!to || to === name) return;
    void run(() => api.renameFlow(name, to));
  }

  function remove(name: string) {
    if (!window.confirm(`Delete the flow "${name}"? This removes its file.`)) return;
    void run(() => api.deleteFlow(name));
  }

  return (
    <div className="flex flex-col gap-1">
      <button
        onClick={onNew}
        className="mx-1 mb-2 rounded bg-accent px-3 py-1.5 font-semibold text-ground transition hover:brightness-110"
      >
        New flow
      </button>

      {error && (
        <p onClick={() => setError(null)} className="mx-1 rounded border border-method-delete/30 bg-method-delete/5 px-2 py-1 text-[11px] text-method-delete">
          {error}
        </p>
      )}

      {flows.length === 0 && (
        <p className="px-2 py-3 text-muted">
          No flows yet. A flow chains requests from the project's API — log in, take the token,
          fetch a record, check it — and runs them as one test.
        </p>
      )}

      {flows.map((name) => (
        <div key={name} className="group flex items-center gap-1 rounded px-1 py-1 hover:bg-raised">
          <button onClick={() => onOpen(name)} className="flex min-w-0 flex-1 items-center gap-2 text-left">
            <span className="shrink-0 font-mono text-[10px] font-bold tracking-wider text-accent">FLOW</span>
            <span className="min-w-0 flex-1 truncate">{name}</span>
          </button>
          <button
            onClick={() => rename(name)}
            title="Rename"
            className="shrink-0 rounded px-1 text-[11px] text-muted opacity-0 transition hover:text-ink group-hover:opacity-100"
          >
            ✎
          </button>
          <button
            onClick={() => remove(name)}
            title="Delete"
            className="shrink-0 rounded px-1 text-[11px] text-muted opacity-0 transition hover:text-method-delete group-hover:opacity-100"
          >
            ✕
          </button>
        </div>
      ))}
    </div>
  );
}
