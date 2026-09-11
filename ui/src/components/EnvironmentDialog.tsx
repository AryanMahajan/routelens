import { useEffect, useState } from "react";
import { api, CoreError } from "../api";
import type { Environment, WorkspaceInfo } from "../types";

/**
 * Where `base_url`, API keys and everything else `{{…}}` can name gets defined.
 *
 * Variables are committed with the workspace and shown in full. Secrets are the opposite:
 * stored outside git, and only their *names* ever come back from the core — a value can be
 * set or replaced here, never read back. Reference one as `{{secret:NAME}}`.
 */
export function EnvironmentDialog({
  workspace,
  onClose,
  onWorkspaceChange,
}: {
  workspace: WorkspaceInfo;
  onClose: () => void;
  onWorkspaceChange: (info: WorkspaceInfo) => void;
}) {
  const [selected, setSelected] = useState<string | null>(
    workspace.active_environment ?? workspace.environments[0] ?? null,
  );
  const [environment, setEnvironment] = useState<Environment | null>(null);
  const [rows, setRows] = useState<{ key: string; value: string }[]>([]);
  const [secrets, setSecrets] = useState<string[]>([]);
  const [newSecret, setNewSecret] = useState({ name: "", value: "" });
  const [newName, setNewName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [dirty, setDirty] = useState(false);

  useEffect(() => {
    void api
      .secretNames()
      .then(setSecrets)
      .catch(() => {});
  }, []);

  useEffect(() => {
    if (!selected) {
      setEnvironment(null);
      setRows([]);
      return;
    }
    let cancelled = false;
    void api
      .loadEnvironment(selected)
      .then((env) => {
        if (cancelled) return;
        setEnvironment(env);
        setRows(Object.entries(env.variables).map(([key, value]) => ({ key, value })));
        setDirty(false);
      })
      .catch((e) => setError(e instanceof CoreError ? e.message : String(e)));
    return () => {
      cancelled = true;
    };
  }, [selected]);

  function report(e: unknown) {
    setError(e instanceof CoreError ? e.message : String(e));
  }

  async function save() {
    if (!environment) return;
    const variables: Record<string, string> = {};
    for (const row of rows) {
      if (row.key.trim()) variables[row.key.trim()] = row.value;
    }
    try {
      await api.saveEnvironment({ ...environment, variables });
      setDirty(false);
      onWorkspaceChange(await api.workspaceInfo());
    } catch (e) {
      report(e);
    }
  }

  async function create() {
    const name = newName.trim();
    if (!name) return;
    try {
      await api.saveEnvironment({ version: 1, name, variables: {}, secrets: [] });
      const info = await api.setEnvironment(name);
      onWorkspaceChange(info);
      setNewName("");
      setSelected(name);
    } catch (e) {
      report(e);
    }
  }

  async function remove(name: string) {
    if (!window.confirm(`Delete environment "${name}"? Its variables are lost.`)) return;
    try {
      const info = await api.deleteEnvironment(name);
      onWorkspaceChange(info);
      setSelected(info.active_environment ?? info.environments[0] ?? null);
    } catch (e) {
      report(e);
    }
  }

  async function activate(name: string) {
    try {
      onWorkspaceChange(await api.setEnvironment(name));
    } catch (e) {
      report(e);
    }
  }

  async function addSecret() {
    const name = newSecret.name.trim();
    if (!name || !newSecret.value) return;
    try {
      await api.setSecret(name, newSecret.value);
      setSecrets(await api.secretNames());
      setNewSecret({ name: "", value: "" });
      onWorkspaceChange(await api.workspaceInfo());
    } catch (e) {
      report(e);
    }
  }

  async function removeSecret(name: string) {
    try {
      await api.deleteSecret(name);
      setSecrets(await api.secretNames());
      onWorkspaceChange(await api.workspaceInfo());
    } catch (e) {
      report(e);
    }
  }

  function updateRow(index: number, patch: Partial<{ key: string; value: string }>) {
    setRows(rows.map((r, i) => (i === index ? { ...r, ...patch } : r)));
    setDirty(true);
  }

  const field =
    "min-w-0 rounded border border-edge bg-ground px-2 py-1 font-mono outline-none " +
    "placeholder:text-muted/60 focus:border-accent";

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="flex h-[80vh] w-full max-w-4xl overflow-hidden rounded-lg border border-edge bg-panel shadow-2xl">
        {/* Environment list */}
        <aside className="flex w-56 shrink-0 flex-col border-r border-edge">
          <h2 className="border-b border-edge px-3 py-2 text-[11px] font-semibold uppercase tracking-wider text-muted">
            Environments
          </h2>
          <ul className="min-h-0 flex-1 overflow-auto p-1">
            {workspace.environments.map((name) => (
              <li key={name}>
                <button
                  onClick={() => setSelected(name)}
                  className={`flex w-full items-center gap-2 rounded px-2 py-1.5 text-left transition
                    ${selected === name ? "bg-raised text-ink" : "text-muted hover:text-ink"}`}
                >
                  <span className="min-w-0 flex-1 truncate">{name}</span>
                  {workspace.active_environment === name && (
                    <span className="rounded bg-accent/15 px-1.5 text-[10px] font-semibold text-accent">
                      active
                    </span>
                  )}
                </button>
              </li>
            ))}
          </ul>
          <form
            className="flex gap-1 border-t border-edge p-2"
            onSubmit={(e) => {
              e.preventDefault();
              void create();
            }}
          >
            <input
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              placeholder="new environment"
              className={`${field} flex-1`}
            />
            <button
              type="submit"
              disabled={!newName.trim()}
              className="rounded bg-raised px-2 transition hover:brightness-125 disabled:opacity-40"
              title="Create"
            >
              +
            </button>
          </form>
        </aside>

        {/* Editor */}
        <div className="flex min-w-0 flex-1 flex-col">
          <header className="flex items-center gap-2 border-b border-edge px-4 py-2">
            <h2 className="min-w-0 flex-1 truncate font-semibold">
              {selected ?? "No environment selected"}
            </h2>
            {selected && workspace.active_environment !== selected && (
              <button
                onClick={() => activate(selected)}
                className="rounded bg-raised px-3 py-1 transition hover:brightness-125"
              >
                Use this one
              </button>
            )}
            {selected && (
              <button
                onClick={() => remove(selected)}
                className="rounded px-3 py-1 text-muted transition hover:bg-method-delete/15 hover:text-method-delete"
              >
                Delete
              </button>
            )}
            <button
              onClick={onClose}
              className="rounded px-2 py-1 text-muted transition hover:text-ink"
              title="Close"
            >
              ✕
            </button>
          </header>

          <div className="min-h-0 flex-1 overflow-auto p-4">
            {environment && (
              <section>
                <div className="mb-2 flex items-baseline justify-between">
                  <h3 className="text-[11px] font-semibold uppercase tracking-wider text-muted">
                    Variables
                  </h3>
                  <p className="text-[11px] text-muted">
                    Use as <span className="font-mono">{"{{name}}"}</span> · committed with the
                    workspace
                  </p>
                </div>
                <div className="flex flex-col gap-px">
                  {rows.map((row, index) => (
                    <div key={index} className="group flex items-center gap-2 rounded px-1 py-0.5 hover:bg-raised/60">
                      <input
                        value={row.key}
                        onChange={(e) => updateRow(index, { key: e.target.value })}
                        placeholder="base_url"
                        className={`${field} w-2/5`}
                      />
                      <input
                        value={row.value}
                        onChange={(e) => updateRow(index, { value: e.target.value })}
                        placeholder="http://localhost:8000"
                        className={`${field} flex-1`}
                      />
                      <button
                        onClick={() => {
                          setRows(rows.filter((_, i) => i !== index));
                          setDirty(true);
                        }}
                        className="shrink-0 rounded px-1.5 py-0.5 text-muted opacity-0 transition hover:bg-method-delete/15 hover:text-method-delete group-hover:opacity-100"
                        title="Remove"
                      >
                        ✕
                      </button>
                    </div>
                  ))}
                  <div className="mt-1 flex items-center gap-3">
                    <button
                      onClick={() => {
                        setRows([...rows, { key: "", value: "" }]);
                        setDirty(true);
                      }}
                      className="rounded px-2 py-1 text-muted transition hover:bg-raised hover:text-ink"
                    >
                      + Add variable
                    </button>
                    <button
                      onClick={save}
                      disabled={!dirty}
                      className="rounded bg-accent px-3 py-1 font-semibold text-ground transition hover:brightness-110 disabled:opacity-40"
                    >
                      Save
                    </button>
                    {dirty && <span className="text-[11px] text-muted">unsaved changes</span>}
                  </div>
                </div>
              </section>
            )}

            <section className="mt-8">
              <div className="mb-2 flex items-baseline justify-between">
                <h3 className="text-[11px] font-semibold uppercase tracking-wider text-muted">
                  Secrets
                </h3>
                <p className="text-[11px] text-muted">
                  Use as <span className="font-mono">{"{{secret:name}}"}</span> · stored
                  outside git, never shown again
                </p>
              </div>
              <ul className="flex flex-col gap-px">
                {secrets.length === 0 && (
                  <li className="px-1 py-2 text-muted italic">No secrets stored on this machine.</li>
                )}
                {secrets.map((name) => (
                  <li key={name} className="group flex items-center gap-2 rounded px-1 py-1 hover:bg-raised/60">
                    <span className="w-2/5 truncate px-2 font-mono">{name}</span>
                    <span className="flex-1 px-2 font-mono text-muted">••••••••</span>
                    <button
                      onClick={() => removeSecret(name)}
                      className="shrink-0 rounded px-1.5 py-0.5 text-muted opacity-0 transition hover:bg-method-delete/15 hover:text-method-delete group-hover:opacity-100"
                      title="Delete secret"
                    >
                      ✕
                    </button>
                  </li>
                ))}
              </ul>
              <form
                className="mt-2 flex items-center gap-2"
                onSubmit={(e) => {
                  e.preventDefault();
                  void addSecret();
                }}
              >
                <input
                  value={newSecret.name}
                  onChange={(e) => setNewSecret({ ...newSecret, name: e.target.value })}
                  placeholder="api_key"
                  className={`${field} w-2/5`}
                />
                <input
                  type="password"
                  value={newSecret.value}
                  onChange={(e) => setNewSecret({ ...newSecret, value: e.target.value })}
                  placeholder="value"
                  className={`${field} flex-1`}
                />
                <button
                  type="submit"
                  disabled={!newSecret.name.trim() || !newSecret.value}
                  className="rounded bg-raised px-3 py-1 transition hover:brightness-125 disabled:opacity-40"
                >
                  {secrets.includes(newSecret.name.trim()) ? "Replace" : "Add"}
                </button>
              </form>
            </section>

            {error && (
              <p className="mt-6 rounded border border-method-delete/40 bg-method-delete/10 px-3 py-2 font-mono text-method-delete">
                {error}
              </p>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
