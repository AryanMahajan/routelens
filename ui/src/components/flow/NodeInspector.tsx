import { useEffect, useState } from "react";
import { api, CoreError } from "../../api";
import { nodeLabel, type NodeLive } from "../../flow";
import {
  matchingHistory,
  responseOf,
  shapeOf,
  suggestName,
  type PathEntry,
  type ResponseShape,
} from "../../responseShape";
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
  type Variable,
} from "../../flowTypes";
import type { SourceView } from "../../types";
import { MethodBadge } from "../MethodBadge";
import { RequestEditor } from "../RequestEditor";
import { ResponseViewer } from "../ResponseViewer";
import { VariableInput, VariableTextarea } from "../VariableInput";
import { PathPicker } from "./PathPicker";

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
  width,
  live,
  source,
  culprit,
  onChange,
  onClose,
  onRunStep,
  hasPriorRun,
}: {
  node: FlowNode;
  width: number;
  live: NodeLive | null;
  source: SourceView | null;
  culprit: string | null;
  onChange: (changes: Partial<FlowNode>) => void;
  onClose: () => void;
  /** Run this card alone, with the last run's variables. Null while a run is going. */
  onRunStep: (() => void) | null;
  hasPriorRun: boolean;
}) {
  const [tab, setTab] = useState<Tab>(live?.result ? "result" : "request");
  const result = live?.result ?? null;
  const shape = useResponseShape(node, live);

  const tabs: { id: Tab; label: string; count?: number }[] =
    node.type === "request"
      ? [
          { id: "request", label: "Request" },
          { id: "extract", label: "Extract", count: node.extract.length },
          { id: "assert", label: "Assert", count: node.assert.length },
          { id: "result", label: "Result" },
        ]
      : [
          {
            id: "request",
            label: node.type === "condition" ? "Condition" : node.type === "variables" ? "Variables" : "Display",
          },
          { id: "result", label: "Result" },
        ];

  return (
    <aside className="flex shrink-0 flex-col border-l border-edge bg-panel" style={{ width }}>
      <div className="flex items-center gap-2 border-b border-edge px-3 py-2">
        {node.type === "request" ? (
          <MethodBadge method={node.request.method} className="shrink-0" />
        ) : node.type === "condition" ? (
          <span className="shrink-0 font-mono text-[10px] font-bold tracking-wider text-method-patch">IF</span>
        ) : node.type === "variables" ? (
          <span className="shrink-0 font-mono text-[10px] font-bold tracking-wider text-accent">{"{{ }}"}</span>
        ) : (
          <span className="shrink-0 font-mono text-[10px] font-bold tracking-wider text-method-put">▤</span>
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
          onClick={() => onRunStep?.()}
          disabled={!onRunStep}
          title={
            hasPriorRun
              ? "Run only this card, with the variables from the last run (Ctrl+Shift+Enter)"
              : "Run only this card (Ctrl+Shift+Enter). Nothing has run yet, so only environment variables are in scope."
          }
          className="shrink-0 rounded bg-raised px-2 py-1 text-[11px] transition hover:brightness-125 disabled:opacity-40"
        >
          ▶ Step
        </button>
        <button
          onClick={onClose}
          title="Hide the inspector (Esc) — select a card to bring it back"
          className="shrink-0 rounded px-1.5 text-muted transition hover:bg-raised hover:text-ink"
        >
          »
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
        {tab === "request" && node.type === "variables" && (
          <VariablesEditor rows={node.variables} onChange={(variables) => onChange({ variables })} />
        )}
        {tab === "request" && node.type === "display" && (
          <DisplayEditor text={node.text} onChange={(text) => onChange({ text })} />
        )}
        {tab === "extract" && node.type === "request" && (
          <ExtractEditor rows={node.extract} onChange={(extract) => onChange({ extract })} shape={shape} />
        )}
        {tab === "assert" && node.type === "request" && (
          <AssertEditor rows={node.assert} onChange={(assert) => onChange({ assert })} shape={shape} />
        )}
        {tab === "result" && <ResultView node={node} live={live} culprit={culprit} />}
      </div>
    </aside>
  );
}

/**
 * The shape of this step's response, for the extract and assert pickers: what the last
 * run got back, or — before any run — the newest history entry for the same endpoint,
 * which a request tab or an earlier flow may have produced.
 */
function useResponseShape(node: FlowNode, live: NodeLive | null): ResponseShape | null {
  const exchange = live?.result?.exchange ?? null;
  const method = node.type === "request" ? node.request.method : "";
  const url = node.type === "request" ? node.request.url : "";
  const [fromHistory, setFromHistory] = useState<ResponseShape | null>(null);

  useEffect(() => {
    if (exchange || !url) return;
    let cancelled = false;
    void api
      .history(200)
      .then((entries) => {
        if (cancelled) return;
        const entry = matchingHistory(entries, method, url);
        const response = entry ? responseOf(entry) : null;
        setFromHistory(response && entry ? shapeOf(response, "history", entry.at) : null);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [exchange, method, url]);

  if (exchange) return shapeOf(exchange.response, "run");
  return fromHistory;
}

function ShapeNote({ shape }: { shape: ResponseShape | null }) {
  if (!shape) {
    return (
      <p className="text-[11px] text-muted">
        Paths can be picked from a real response: run this card once (▶ Step), or send the
        request from a tab, and the list appears here.
      </p>
    );
  }
  const when =
    shape.source === "run"
      ? "the last run"
      : `history, ${shape.at ? new Date(shape.at * (shape.at < 1e12 ? 1000 : 1)).toLocaleString() : "earlier"}`;
  return (
    <p className="text-[11px] text-muted">
      Picking from {when}: <span className="font-mono">{shape.status}</span>, {shape.paths.length} path
      {shape.paths.length === 1 ? "" : "s"}, {shape.headers.length} header{shape.headers.length === 1 ? "" : "s"}.
    </p>
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

function SourceFields<T extends ValueSource>({
  row,
  onChange,
  shape,
  onPick,
}: {
  row: T;
  onChange: (row: T) => void;
  shape: ResponseShape | null;
  /** A path or header was chosen from the response, with what was found there. */
  onPick?: (entry: PathEntry) => void;
}) {
  const headerOptions: PathEntry[] | null = shape
    ? shape.headers.map((h) => ({ path: h, preview: "", scalar: true, text: "" }))
    : null;
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
        <PathPicker
          value={row.path}
          onChange={(path) => onChange({ ...row, path })}
          onPick={onPick}
          options={shape?.paths ?? null}
          placeholder="user.id"
          emptyHint="No response to pick from yet — run this card once."
          className={`${inputClass} font-mono`}
        />
      )}
      {row.from === "header" && (
        <PathPicker
          value={row.header}
          onChange={(header) => onChange({ ...row, header })}
          onPick={onPick}
          options={headerOptions}
          placeholder="Location"
          emptyHint="No response to pick from yet — run this card once."
          className={`${inputClass} font-mono`}
        />
      )}
    </>
  );
}

function ExtractEditor({
  rows,
  onChange,
  shape,
}: {
  rows: Extraction[];
  onChange: (rows: Extraction[]) => void;
  shape: ResponseShape | null;
}) {
  const update = (i: number, row: Extraction) => onChange(rows.map((r, j) => (j === i ? row : r)));
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-auto p-3">
      <p className="text-muted">
        Pull values out of this response for the steps that follow. Each becomes{" "}
        <code className="rounded bg-raised px-1 font-mono text-accent">{"{{name}}"}</code> in
        any later URL, header or body.
      </p>
      <ShapeNote shape={shape} />
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
            <SourceFields
              row={row}
              onChange={(r) => update(i, r)}
              shape={shape}
              // A picked path names the variable too, unless one was typed already.
              onPick={(entry) => {
                if (!row.name.trim()) {
                  const name = suggestName(entry.path);
                  onChange(rows.map((r, j) => (j === i ? { ...r, name, ...(r.from === "body" ? { path: entry.path } : { header: entry.path }) } : r)));
                }
              }}
            />
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

function AssertEditor({
  rows,
  onChange,
  shape,
}: {
  rows: Assertion[];
  onChange: (rows: Assertion[]) => void;
  shape: ResponseShape | null;
}) {
  const update = (i: number, row: Assertion) => onChange(rows.map((r, j) => (j === i ? row : r)));
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-auto p-3">
      <p className="text-muted">
        What must hold for this step to pass. Any failing check fails the step, and every
        step after it is skipped.
      </p>
      <ShapeNote shape={shape} />
      {rows.map((row, i) => (
        <div key={i} className="flex flex-col gap-1.5 rounded border border-edge bg-ground p-2">
          <div className="flex items-center gap-2">
            <SourceFields
              row={row}
              onChange={(r) => update(i, r)}
              shape={shape}
              // Picking a value from the response asserts it stays that way: equals, with
              // what was found. A container can only be checked for presence.
              onPick={(entry) =>
                onChange(
                  rows.map((r, j) =>
                    j === i
                      ? {
                          ...r,
                          ...(r.from === "body" ? { path: entry.path } : { header: entry.path }),
                          ...(entry.scalar && entry.text
                            ? { op: "equals" as Operator, expected: entry.text }
                            : { op: "exists" as Operator, expected: "" }),
                        }
                      : r,
                  ),
                )
              }
            />
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

function VariablesEditor({ rows, onChange }: { rows: Variable[]; onChange: (rows: Variable[]) => void }) {
  const update = (i: number, row: Variable) => onChange(rows.map((r, j) => (j === i ? row : r)));
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-auto p-3">
      <p className="text-muted">
        The flow's own inputs. Change a value here and every step that uses{" "}
        <code className="rounded bg-raised px-1 font-mono text-accent">{"{{name}}"}</code> follows —
        no environment edit, no touching the steps. A value may use other variables. With
        nothing wired into this block it runs before everything else.
      </p>
      {rows.map((row, i) => (
        <div key={i} className="flex items-center gap-2">
          <input
            value={row.name}
            onChange={(e) => update(i, { ...row, name: e.target.value.replace(/[^\w.-]/g, "") })}
            placeholder="who"
            spellCheck={false}
            className={`${inputClass} w-2/5 flex-none font-mono text-accent`}
          />
          <span className="text-muted">=</span>
          <VariableInput
            value={row.value}
            onChange={(value) => update(i, { ...row, value })}
            placeholder="ann"
            className={`${inputClass} font-mono`}
          />
          <button onClick={() => onChange(rows.filter((_, j) => j !== i))} title="Remove" className={removeClass}>
            ✕
          </button>
        </div>
      ))}
      <button
        onClick={() => onChange([...rows, { name: "", value: "" }])}
        className="self-start rounded bg-raised px-3 py-1.5 transition hover:brightness-125"
      >
        + Variable
      </button>
    </div>
  );
}

function DisplayEditor({ text, onChange }: { text: string; onChange: (text: string) => void }) {
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3 p-3">
      <p className="shrink-0 text-muted">
        A sentence built from variables, shown on the card once the run reaches it — a
        readable summary of what the flow found. Type <code className="rounded bg-raised px-1 font-mono text-accent">{"{{"}</code>{" "}
        for what is in scope.
      </p>
      <VariableTextarea
        value={text}
        onChange={onChange}
        placeholder={"{{who}} is user {{found_id}}"}
        className="resize-none rounded border border-edge bg-ground p-3 font-mono leading-relaxed outline-none placeholder:text-muted/50 focus:border-accent"
      />
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

      {result.output !== null && (
        <Section title="Output">
          <p className="whitespace-pre-wrap break-words text-[13px]">{result.output}</p>
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
        <Section title={node.type === "variables" ? "Declared" : "Extracted"}>
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

