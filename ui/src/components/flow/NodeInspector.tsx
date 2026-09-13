import { useState } from "react";
import { api, CoreError } from "../../api";
import { nodeLabel, type NodeLive } from "../../flow";
import {
  describeFailure,
  describeSource,
  isUnary,
  OPERATORS,
  type Assertion,
  type AssertionResult,
  type Extraction,
  type FlowNode,
  type NodeResult,
  type Operator,
  type ValueSource,
} from "../../flowTypes";
import type { SourceView } from "../../types";
import { MethodBadge } from "../MethodBadge";
import { RequestEditor } from "../RequestEditor";
import { ResponseViewer } from "../ResponseViewer";
import { VariableInput } from "../VariableInput";

type Tab = "request" | "extract" | "assert" | "result";

/**
 * The panel beside the canvas for the selected card.
 *
 * The request itself is edited with the same editor a request tab uses — a flow step is an
 * ordinary request with two things bolted on: what to pull out of the response and what
 * must hold. After a run the same panel shows what came back, with the response viewer a
 * request tab would show.
 */
export function NodeInspector({
  node,
  live,
  source,
  culprit,
  onChange,
  onClose,
}: {
  node: FlowNode;
  live: NodeLive | null;
  source: SourceView | null;
  culprit: string | null;
  onChange: (changes: Partial<FlowNode>) => void;
  onClose: () => void;
}) {
  const [tab, setTab] = useState<Tab>(live?.result ? "result" : "request");
  const result = live?.result ?? null;

  const tabs: { id: Tab; label: string; count?: number }[] =
    node.type === "request"
      ? [
          { id: "request", label: "Request" },
          { id: "extract", label: "Extract", count: node.extract.length },
          { id: "assert", label: "Assert", count: node.assert.length },
          { id: "result", label: "Result" },
        ]
      : [
          { id: "request", label: "Condition" },
          { id: "result", label: "Result" },
        ];

  return (
    <aside className="flex w-[440px] shrink-0 flex-col border-l border-edge bg-panel">
      <div className="flex items-center gap-2 border-b border-edge px-3 py-2">
        {node.type === "request" ? (
          <MethodBadge method={node.request.method} className="shrink-0" />
        ) : (
          <span className="shrink-0 font-mono text-[10px] font-bold tracking-wider text-method-patch">IF</span>
        )}
        <input
          value={node.name ?? ""}
          onChange={(e) => onChange({ name: e.target.value || null })}
          placeholder={nodeLabel({ ...node, name: null })}
          className="min-w-0 flex-1 rounded border border-transparent bg-transparent px-2 py-1 font-semibold
            outline-none placeholder:text-muted/70 focus:border-edge focus:bg-ground"
        />
        {source && <RevealButton source={source} />}
        <button
          onClick={onClose}
          title="Close (Esc)"
          className="shrink-0 rounded px-1.5 text-muted transition hover:bg-raised hover:text-ink"
        >
          ✕
        </button>
      </div>

      <div className="flex shrink-0 gap-1 border-b border-edge px-2">
        {tabs.map((t) => (
          <button
            key={t.id}
            onClick={() => setTab(t.id)}
            className={`relative px-3 py-2 transition ${tab === t.id ? "text-ink" : "text-muted hover:text-ink"}`}
          >
            {t.label}
            {t.count !== undefined && t.count > 0 && (
              <span className="ml-1.5 rounded-full bg-raised px-1.5 py-0.5 text-[10px] tabular-nums text-muted">
                {t.count}
              </span>
            )}
            {t.id === "result" && result && (
              <span
                className={`ml-1.5 inline-block size-1.5 rounded-full align-middle ${
                  result.status === "passed"
                    ? "bg-method-get"
                    : result.status === "failed"
                      ? "bg-method-delete"
                      : "bg-muted"
                }`}
              />
            )}
            {tab === t.id && <span className="absolute inset-x-2 -bottom-px h-0.5 rounded-full bg-accent" />}
          </button>
        ))}
      </div>

      <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
        {tab === "request" && node.type === "request" && (
          <RequestEditor request={node.request} onChange={(request) => onChange({ request })} />
        )}
        {tab === "request" && node.type === "condition" && (
          <ConditionEditor node={node} onChange={onChange} />
        )}
        {tab === "extract" && node.type === "request" && (
          <ExtractEditor rows={node.extract} onChange={(extract) => onChange({ extract })} />
        )}
        {tab === "assert" && node.type === "request" && (
          <AssertEditor rows={node.assert} onChange={(assert) => onChange({ assert })} />
        )}
        {tab === "result" && <ResultView node={node} live={live} culprit={culprit} />}
      </div>
    </aside>
  );
}

function RevealButton({ source }: { source: SourceView }) {
  const [failed, setFailed] = useState(false);
  return (
    <button
      onClick={async () => {
        try {
          await api.revealInEditor(source.file, source.line);
        } catch (e) {
          setFailed(e instanceof CoreError);
        }
      }}
      title={failed ? "Could not open the file" : `${source.file}:${source.line}`}
      className="shrink-0 rounded px-2 py-1 font-mono text-[11px] text-muted transition hover:bg-raised hover:text-accent"
    >
      {failed ? "✕" : "↗"} {source.file.split("/").pop()}:{source.line}
    </button>
  );
}

// --- editors -----------------------------------------------------------------------------

const SOURCES: { value: ValueSource["from"]; label: string }[] = [
  { value: "body", label: "body path" },
  { value: "header", label: "header" },
  { value: "status", label: "status" },
  { value: "body_text", label: "body text" },
  { value: "duration", label: "duration" },
];

function withSource<T extends ValueSource>(row: T, from: ValueSource["from"]): T {
  const rest = { ...row } as Record<string, unknown>;
  delete rest["path"];
  delete rest["header"];
  const source: ValueSource =
    from === "body" ? { from, path: "" } : from === "header" ? { from, header: "" } : { from };
  return { ...rest, ...source } as T;
}

function SourceFields<T extends ValueSource>({ row, onChange }: { row: T; onChange: (row: T) => void }) {
  return (
    <>
      <select
        value={row.from}
        onChange={(e) => onChange(withSource(row, e.target.value as ValueSource["from"]))}
        className={selectClass}
      >
        {SOURCES.map((s) => (
          <option key={s.value} value={s.value}>
            {s.label}
          </option>
        ))}
      </select>
      {row.from === "body" && (
        <input
          value={row.path}
          onChange={(e) => onChange({ ...row, path: e.target.value })}
          placeholder="user.id"
          spellCheck={false}
          className={`${inputClass} font-mono`}
        />
      )}
      {row.from === "header" && (
        <input
          value={row.header}
          onChange={(e) => onChange({ ...row, header: e.target.value })}
          placeholder="Location"
          spellCheck={false}
          className={`${inputClass} font-mono`}
        />
      )}
    </>
  );
}

function ExtractEditor({ rows, onChange }: { rows: Extraction[]; onChange: (rows: Extraction[]) => void }) {
  const update = (i: number, row: Extraction) => onChange(rows.map((r, j) => (j === i ? row : r)));
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-auto p-3">
      <p className="text-muted">
        Pull values out of this response for the steps that follow. Each becomes{" "}
        <code className="rounded bg-raised px-1 font-mono text-accent">{"{{name}}"}</code> in
        any later URL, header or body.
      </p>
      {rows.map((row, i) => (
        <div key={i} className="flex flex-col gap-1.5 rounded border border-edge bg-ground p-2">
          <div className="flex items-center gap-2">
            <span className="text-muted">{"{{"}</span>
            <input
              value={row.name}
              onChange={(e) => update(i, { ...row, name: e.target.value.replace(/[^\w.-]/g, "") })}
              placeholder="auth_token"
              spellCheck={false}
              className={`${inputClass} font-mono text-accent`}
            />
            <span className="text-muted">{"}}"}</span>
            <button onClick={() => onChange(rows.filter((_, j) => j !== i))} title="Remove" className={removeClass}>
              ✕
            </button>
          </div>
          <div className="flex items-center gap-2">
            <span className="w-8 shrink-0 text-right text-muted">from</span>
            <SourceFields row={row} onChange={(r) => update(i, r)} />
          </div>
        </div>
      ))}
      <button
        onClick={() => onChange([...rows, { name: "", from: "body", path: "" }])}
        className="self-start rounded bg-raised px-3 py-1.5 transition hover:brightness-125"
      >
        + Extract a value
      </button>
    </div>
  );
}

function AssertEditor({ rows, onChange }: { rows: Assertion[]; onChange: (rows: Assertion[]) => void }) {
  const update = (i: number, row: Assertion) => onChange(rows.map((r, j) => (j === i ? row : r)));
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-auto p-3">
      <p className="text-muted">
        What must hold for this step to pass. Any failing check fails the step, and every
        step after it is skipped.
      </p>
      {rows.map((row, i) => (
        <div key={i} className="flex flex-col gap-1.5 rounded border border-edge bg-ground p-2">
          <div className="flex items-center gap-2">
            <SourceFields row={row} onChange={(r) => update(i, r)} />
            <button onClick={() => onChange(rows.filter((_, j) => j !== i))} title="Remove" className={removeClass}>
              ✕
            </button>
          </div>
          <div className="flex items-center gap-2">
            <select
              value={row.op}
              onChange={(e) => update(i, { ...row, op: e.target.value as Operator })}
              className={selectClass}
            >
              {OPERATORS.map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
            </select>
            {!isUnary(row.op) && (
              <VariableInput
                value={row.expected}
                onChange={(expected) => update(i, { ...row, expected })}
                placeholder="expected"
                className={`${inputClass} font-mono`}
              />
            )}
          </div>
        </div>
      ))}
      <div className="flex flex-wrap gap-2">
        <button
          onClick={() => onChange([...rows, { from: "status", op: "equals", expected: "200" }])}
          className="rounded bg-raised px-3 py-1.5 transition hover:brightness-125"
        >
          + Status
        </button>
        <button
          onClick={() => onChange([...rows, { from: "body", path: "", op: "exists", expected: "" }])}
          className="rounded bg-raised px-3 py-1.5 transition hover:brightness-125"
        >
          + Body field
        </button>
        <button
          onClick={() => onChange([...rows, { from: "header", header: "", op: "exists", expected: "" }])}
          className="rounded bg-raised px-3 py-1.5 transition hover:brightness-125"
        >
          + Header
        </button>
      </div>
    </div>
  );
}

function ConditionEditor({
  node,
  onChange,
}: {
  node: FlowNode & { type: "condition" };
  onChange: (changes: Partial<FlowNode>) => void;
}) {
  return (
    <div className="flex flex-col gap-3 overflow-auto p-3">
      <p className="text-muted">
        Compare two values — usually a variable extracted upstream against a literal — and
        send the run down <span className="text-method-get">true</span> or{" "}
        <span className="text-method-delete">false</span>. Whatever hangs off the other output is
        skipped, not failed.
      </p>
      <label className="flex flex-col gap-1">
        <span className={labelClass}>Left</span>
        <VariableInput
          value={node.left}
          onChange={(left) => onChange({ left })}
          placeholder="{{role}}"
          className={`${inputClass} font-mono`}
        />
      </label>
      <label className="flex flex-col gap-1">
        <span className={labelClass}>Operator</span>
        <select
          value={node.op}
          onChange={(e) => onChange({ op: e.target.value as Operator })}
          className={`${selectClass} self-start`}
        >
          {OPERATORS.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </select>
      </label>
      {!isUnary(node.op) && (
        <label className="flex flex-col gap-1">
          <span className={labelClass}>Right</span>
          <VariableInput
            value={node.right}
            onChange={(right) => onChange({ right })}
            placeholder="admin"
            className={`${inputClass} font-mono`}
          />
        </label>
      )}
    </div>
  );
}

// --- result ------------------------------------------------------------------------------

function ResultView({ node, live, culprit }: { node: FlowNode; live: NodeLive | null; culprit: string | null }) {
  const result = live?.result ?? null;
  if (live?.status === "running") return <Empty>Running…</Empty>;
  if (live?.status === "pending") return <Empty>Waiting for the steps before it.</Empty>;
  if (!result) return <Empty>Run the flow to see what this step did.</Empty>;

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-auto">
      <Verdict result={result} culprit={culprit} />

      {node.type === "request" && result.request && (
        <Section title="Sent">
          <p className="font-mono">
            <span className="font-bold text-accent">{result.request.method}</span>{" "}
            {result.exchange?.request.url ?? result.request.url}
          </p>
        </Section>
      )}

      {result.compared && (
        <Section title="Compared">
          <p className="font-mono">
            <span className="text-accent">{result.compared[0] || "∅"}</span>{" "}
            <span className="text-muted">{node.type === "condition" ? describeOp(node.op) : ""}</span>{" "}
            {result.compared[1]}
            {result.branch && (
              <>
                {" "}
                → <span className={result.branch === "true" ? "text-method-get" : "text-method-delete"}>{result.branch}</span>
              </>
            )}
          </p>
        </Section>
      )}

      {result.extracted.length > 0 && (
        <Section title="Extracted">
          <table className="w-full font-mono">
            <tbody>
              {result.extracted.map((e) => (
                <tr key={e.name} className="align-top">
                  <td className="w-1/3 py-0.5 pr-3 text-accent">{`{{${e.name}}}`}</td>
                  <td className="break-all py-0.5">{e.value}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </Section>
      )}

      {result.assertions.length > 0 && (
        <Section title="Checks">
          <ul className="flex flex-col gap-1">
            {result.assertions.map((a, i) => (
              <AssertionRow key={i} result={a} />
            ))}
          </ul>
        </Section>
      )}

      {result.exchange ? (
        <div className="flex min-h-[280px] flex-col">
          <ResponseViewer exchange={result.exchange} error={null} sending={false} />
        </div>
      ) : (
        result.failure?.kind === "transport" && (
          <ResponseViewer exchange={null} error={result.failure.message} sending={false} />
        )
      )}
    </div>
  );
}

function Verdict({ result, culprit }: { result: NodeResult; culprit: string | null }) {
  if (result.status === "passed") {
    return (
      <div className="border-b border-edge px-3 py-2 text-method-get">
        <span className="font-semibold">Passed</span>
        <span className="text-muted"> · {result.duration_ms} ms</span>
      </div>
    );
  }
  if (result.status === "failed" && result.failure) {
    return (
      <div className="border-b border-edge bg-method-delete/5 px-3 py-2 text-method-delete">
        <span className="font-semibold">Failed</span>
        <span className="text-muted"> · {result.duration_ms} ms</span>
        <p className="mt-1 whitespace-pre-wrap break-words font-mono text-[11px]">
          {describeFailure(result.failure)}
        </p>
      </div>
    );
  }
  const why =
    result.reason?.kind === "upstream_failed"
      ? `${culprit ?? "an earlier step"} failed, so this step did not run.`
      : result.reason?.kind === "branch_not_taken"
        ? `${culprit ?? "The condition"} sent the run down its ${result.reason.handle === "true" ? "false" : "true"} output.`
        : "This step did not run.";
  return (
    <div className="border-b border-edge px-3 py-2 text-muted">
      <span className="font-semibold">Skipped</span>
      <p className="mt-1">{why}</p>
    </div>
  );
}

function AssertionRow({ result }: { result: AssertionResult }) {
  const { assertion } = result;
  const expected = isUnary(assertion.op) ? "" : ` ${result.expected}`;
  return (
    <li className="flex items-start gap-2 font-mono text-[12px]">
      <span className={`shrink-0 ${result.passed ? "text-method-get" : "text-method-delete"}`}>
        {result.passed ? "✓" : "✗"}
      </span>
      <span className="min-w-0 flex-1 break-all">
        {describeSource(assertion)} <span className="text-muted">{describeOp(assertion.op)}</span>
        {expected}
        {!result.passed && (
          <span className="block text-muted">
            {result.error ? result.error : result.actual === null ? "nothing there" : `got ${result.actual}`}
          </span>
        )}
      </span>
    </li>
  );
}

function describeOp(op: Operator): string {
  return OPERATORS.find((o) => o.value === op)?.label ?? op;
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="border-b border-edge px-3 py-2">
      <h4 className={`mb-1 ${labelClass}`}>{title}</h4>
      {children}
    </div>
  );
}

function Empty({ children }: { children: React.ReactNode }) {
  return <div className="flex flex-1 items-center justify-center p-6 text-center text-muted">{children}</div>;
}

const inputClass =
  "min-w-0 flex-1 rounded border border-edge bg-panel px-2 py-1 outline-none placeholder:text-muted/60 focus:border-accent";
const selectClass = "shrink-0 rounded border border-edge bg-panel px-2 py-1 outline-none focus:border-accent";
const removeClass = "shrink-0 rounded px-1.5 text-muted transition hover:bg-raised hover:text-method-delete";
const labelClass = "text-[11px] font-semibold uppercase tracking-wider text-muted";

