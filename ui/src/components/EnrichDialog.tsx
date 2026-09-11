import { useEffect, useState } from "react";
import { api, CoreError } from "../api";
import type { EnrichProposal, ScanResult } from "../types";

/**
 * The consent step for runtime enrich.
 *
 * Everything else RouteLens does reads the project. This runs it — imports the application
 * to ask for its own route table — so the exact command is shown, with where each part of it
 * came from, and nothing happens until "Run" is pressed. The target is remembered per project
 * in `workspace.yaml`; the interpreter is not, because it is machine-specific.
 */
export function EnrichDialog({
  onClose,
  onEnriched,
}: {
  onClose: () => void;
  onEnriched: (scan: ScanResult) => void;
}) {
  const [proposal, setProposal] = useState<EnrichProposal | null>(null);
  const [target, setTarget] = useState("");
  const [interpreter, setInterpreter] = useState<string | null>(null);
  const [command, setCommand] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const [done, setDone] = useState<ScanResult | null>(null);

  useEffect(() => {
    void api
      .enrichProposal()
      .then((p) => {
        setProposal(p);
        setTarget(p.remembered_target ?? p.targets[0]?.target ?? "");
        setInterpreter(p.interpreters[0]?.path ?? null);
        setCommand(p.command ?? null);
      })
      .catch((e) => setError(describe(e)));
  }, []);

  // The command line tracks every change, so what is approved is what runs.
  useEffect(() => {
    if (!proposal || !target.trim()) {
      setCommand(null);
      return;
    }
    let cancelled = false;
    void api
      .enrichCommand(target.trim(), interpreter)
      .then((line) => {
        if (!cancelled) setCommand(line);
      })
      .catch((e) => {
        if (!cancelled) {
          setCommand(null);
          setError(describe(e));
        }
      });
    return () => {
      cancelled = true;
    };
  }, [proposal, target, interpreter]);

  async function run() {
    setRunning(true);
    setError(null);
    try {
      const scan = await api.runEnrich(target.trim(), interpreter);
      setDone(scan);
      onEnriched(scan);
    } catch (e) {
      setError(describe(e));
    } finally {
      setRunning(false);
    }
  }

  async function forget() {
    try {
      await api.revokeEnrich();
      setProposal((p) => (p ? { ...p, remembered_target: null } : p));
    } catch (e) {
      setError(describe(e));
    }
  }

  const knownTarget = proposal?.targets.find((t) => t.target === target);
  const report = done?.enrich ?? null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6"
      onClick={onClose}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        className="flex max-h-[85vh] w-full max-w-2xl flex-col overflow-hidden rounded-lg border border-edge bg-panel shadow-2xl"
      >
        <header className="flex items-center justify-between border-b border-edge px-4 py-3">
          <h2 className="font-semibold">Ask the application</h2>
          <button onClick={onClose} className="rounded px-2 text-muted transition hover:text-ink">
            ✕
          </button>
        </header>

        <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-auto px-4 py-4">
          {report ? (
            <Result scan={done!} />
          ) : (
            <>
              <p className="text-muted">
                Runtime enrich <strong className="text-ink">imports and executes your project's code</strong>{" "}
                — module-level code runs, including anything it does at import time — and asks the
                application for its own route table. It starts no server. The result is merged onto
                the static scan: exact paths and schemas, with source locations kept.
              </p>

              {proposal && (
                <>
                  <Field label="Application object" hint="module:attribute, or module:factory() to call a factory">
                    <input
                      value={target}
                      onChange={(e) => setTarget(e.target.value)}
                      list="enrich-targets"
                      spellCheck={false}
                      placeholder="app.main:app"
                      className="w-full rounded border border-edge bg-ground px-2 py-1 font-mono outline-none focus:border-accent"
                    />
                    <datalist id="enrich-targets">
                      {proposal.targets.map((t) => (
                        <option key={t.target} value={t.target}>
                          {t.source}
                        </option>
                      ))}
                    </datalist>
                    <p className="mt-1 text-[11px] text-muted">
                      {knownTarget
                        ? `Suggested from ${knownTarget.source}.`
                        : proposal.targets.length > 0
                          ? `Suggestions: ${proposal.targets.map((t) => t.target).join(", ")}`
                          : "Nothing in the project names the application; enter it by hand."}
                      {proposal.remembered_target && (
                        <>
                          {" "}
                          Remembered for this project.{" "}
                          <button onClick={forget} className="underline hover:text-ink">
                            Forget
                          </button>
                        </>
                      )}
                    </p>
                  </Field>

                  <Field label="Interpreter" hint="must have the project's dependencies installed">
                    {proposal.interpreters.length > 0 ? (
                      <select
                        value={interpreter ?? ""}
                        onChange={(e) => setInterpreter(e.target.value || null)}
                        className="w-full rounded border border-edge bg-ground px-2 py-1 font-mono outline-none focus:border-accent"
                      >
                        {proposal.interpreters.map((i) => (
                          <option key={i.path} value={i.path}>
                            {i.path} — {i.source}
                          </option>
                        ))}
                      </select>
                    ) : (
                      <p className="text-method-post">
                        No Python interpreter found. Create a virtual environment in the project
                        (<code>.venv</code>) and install its dependencies.
                      </p>
                    )}
                  </Field>

                  <Field label="This will run">
                    <pre className="overflow-x-auto whitespace-pre-wrap break-all rounded border border-edge bg-ground px-2 py-1.5 font-mono text-[12px]">
                      {command ?? "—"}
                    </pre>
                    <p className="mt-1 text-[11px] text-muted">
                      The helper script is written to <code>{proposal.helper_path}</code> before it
                      runs, so you can read it. It introspects only.
                    </p>
                  </Field>
                </>
              )}
            </>
          )}

          {error && (
            <pre className="whitespace-pre-wrap break-words rounded border border-method-delete/40 bg-method-delete/10 px-2 py-1.5 font-mono text-[12px] text-method-delete">
              {error}
            </pre>
          )}
        </div>

        <footer className="flex items-center justify-end gap-2 border-t border-edge px-4 py-3">
          {report ? (
            <button
              onClick={onClose}
              className="rounded bg-accent px-4 py-1.5 font-semibold text-ground transition hover:brightness-110"
            >
              Done
            </button>
          ) : (
            <>
              <button onClick={onClose} className="rounded px-3 py-1.5 text-muted transition hover:text-ink">
                Cancel
              </button>
              <button
                onClick={run}
                disabled={running || !command || !target.trim()}
                className="rounded bg-accent px-4 py-1.5 font-semibold text-ground transition
                  hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-40"
              >
                {running ? "Running…" : "Run"}
              </button>
            </>
          )}
        </footer>
      </div>
    </div>
  );
}

function Field({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
  return (
    <label className="block">
      <span className="mb-1 block text-[11px] font-semibold uppercase tracking-wider text-muted">
        {label}
        {hint && <span className="ml-2 font-normal normal-case tracking-normal">{hint}</span>}
      </span>
      {children}
    </label>
  );
}

function Result({ scan }: { scan: ScanResult }) {
  const report = scan.enrich!;
  return (
    <div className="flex flex-col gap-3">
      <p>
        Asked the <strong>{report.framework}</strong> application at{" "}
        <code className="font-mono">{report.target}</code> in {report.duration_ms} ms.
      </p>
      <ul className="grid grid-cols-2 gap-x-6 gap-y-1 tabular-nums">
        <Stat n={report.matched} label="found by both" />
        <Stat n={report.gaps_filled} label="gaps resolved by runtime" />
        <Stat n={report.runtime_only} label="only at runtime — no source location" />
        <Stat n={report.static_only} label="only in source — probably not served" />
      </ul>
      {report.warnings.length > 0 && (
        <p className="text-[12px] text-muted">{report.warnings.join(" · ")}</p>
      )}
      {report.stderr.trim() && (
        <details>
          <summary className="cursor-pointer text-[12px] text-muted">
            The application printed {report.stderr.trim().split("\n").length} line(s) while importing
          </summary>
          <pre className="mt-1 max-h-48 overflow-auto rounded border border-edge bg-ground px-2 py-1.5 font-mono text-[11px] text-muted">
            {report.stderr}
          </pre>
        </details>
      )}
      <p className="text-[11px] text-muted">
        Rescanning returns to the static result. The target is remembered in{" "}
        <code>.routelens/workspace.yaml</code>.
      </p>
    </div>
  );
}

function Stat({ n, label }: { n: number; label: string }) {
  return (
    <li className="flex items-baseline gap-2">
      <span className="w-8 text-right font-semibold">{n}</span>
      <span className="text-muted">{label}</span>
    </li>
  );
}

function describe(e: unknown): string {
  return e instanceof CoreError ? e.message : String(e);
}
