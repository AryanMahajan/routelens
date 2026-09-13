import { useMemo, useState } from "react";
import { hasCycle, nodeLabel, updateNode, upstreamVariables, type LiveState } from "../../flow";
import type { Flow, FlowNode, FlowRun, Position } from "../../flowTypes";
import type { ScanResult } from "../../types";
import { useVariableNames, VariablesContext } from "../../variables";
import { ResizeHandle, usePersistedNumber } from "../ResizeHandle";
import { EndpointPicker, type Pick } from "./EndpointPicker";
import { FlowCanvas, sourceFor, type FlowUpdate } from "./FlowCanvas";
import { NodeInspector } from "./NodeInspector";

const INSPECTOR_WIDTH = 440;

/**
 * A flow tab: toolbar, canvas, and the inspector for whichever card is selected.
 *
 * Everything that touches the core — adding a discovered endpoint, running, saving — is
 * the parent's job, because those need the workspace and the tab list. This component owns
 * only what is visible.
 */
export function FlowEditor({
  flow,
  live,
  run,
  running,
  selected,
  scan,
  error,
  onChange,
  onSelect,
  onAdd,
  onDropEndpoint,
  onRun,
  onSave,
}: {
  flow: Flow;
  live: LiveState;
  run: FlowRun | null;
  running: boolean;
  selected: string | null;
  scan: ScanResult | null;
  error: string | null;
  onChange: (update: FlowUpdate) => void;
  onSelect: (id: string | null) => void;
  /** Add a node after the selected one: a discovered endpoint, a blank request, a condition. */
  onAdd: (pick: Pick) => void;
  onDropEndpoint: (endpoint: string, position: Position) => void;
  onRun: () => void;
  onSave: () => void;
}) {
  const [picking, setPicking] = useState(false);
  const [inspectorWidth, setInspectorWidth] = usePersistedNumber("routelens.inspector.width", INSPECTOR_WIDTH);
  const cyclic = useMemo(() => hasCycle(flow), [flow]);
  const node = selected ? (flow.nodes.find((n) => n.id === selected) ?? null) : null;

  const labels = useMemo(() => {
    const out: Record<string, string> = {};
    for (const n of flow.nodes) out[n.id] = nodeLabel(n);
    return out;
  }, [flow.nodes]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 items-center gap-2 border-b border-edge px-3 py-2">
        <input
          value={flow.name}
          onChange={(e) => onChange((f) => ({ ...f, name: e.target.value }))}
          placeholder="Untitled flow"
          spellCheck={false}
          className="min-w-0 flex-1 rounded border border-transparent bg-transparent px-2 py-1 font-semibold
            outline-none placeholder:text-muted/60 focus:border-edge focus:bg-panel"
        />

        <div className="relative">
          <button
            onClick={() => setPicking((p) => !p)}
            className="shrink-0 rounded bg-raised px-3 py-1 transition hover:brightness-125"
            title="Add a step — or click / drag an endpoint from the API panel"
          >
            + Add
          </button>
          {picking && (
            <EndpointPicker
              scan={scan}
              onClose={() => setPicking(false)}
              onPick={(pick) => {
                setPicking(false);
                onAdd(pick);
              }}
            />
          )}
        </div>

        <button
          onClick={onRun}
          disabled={running || flow.nodes.length === 0 || cyclic}
          title={cyclic ? "The flow has a cycle" : "Run (Ctrl+Enter)"}
          className="shrink-0 rounded bg-accent px-4 py-1 font-semibold text-ground transition hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-40"
        >
          {running ? "Running…" : "▶ Run"}
        </button>

        <button
          onClick={onSave}
          disabled={!flow.name.trim()}
          title="Ctrl+S"
          className="shrink-0 rounded bg-raised px-3 py-1 transition hover:brightness-125 disabled:opacity-40"
        >
          Save
        </button>
      </div>

      {(run || cyclic || error) && (
        <div className="flex shrink-0 flex-wrap items-center gap-x-4 gap-y-1 border-b border-edge bg-panel/60 px-3 py-1.5 text-[11px]">
          {cyclic && <span className="text-method-post">The flow has a cycle — remove an edge to run it.</span>}
          {error && <span className="text-method-delete">{error}</span>}
          {run && !running && (
            <>
              <span className={run.summary.failed === 0 ? "font-semibold text-method-get" : "font-semibold text-method-delete"}>
                {run.summary.failed === 0 ? "Passed" : "Failed"}
              </span>
              <span className="text-method-get tabular-nums">{run.summary.passed} passed</span>
              {run.summary.failed > 0 && <span className="text-method-delete tabular-nums">{run.summary.failed} failed</span>}
              {run.summary.skipped > 0 && <span className="text-muted tabular-nums">{run.summary.skipped} skipped</span>}
              <span className="text-muted tabular-nums">{run.duration_ms} ms</span>
              {Object.keys(run.variables).length > 0 && (
                <span className="min-w-0 truncate font-mono text-muted" title={Object.entries(run.variables).map(([k, v]) => `${k} = ${v}`).join("\n")}>
                  {Object.keys(run.variables).map((k) => `{{${k}}}`).join(" ")}
                </span>
              )}
            </>
          )}
        </div>
      )}

      <div className="flex min-h-0 flex-1">
        <FlowCanvas
          flow={flow}
          live={live}
          selected={selected}
          scan={scan}
          onChange={onChange}
          onSelect={onSelect}
          onDropEndpoint={onDropEndpoint}
        />
        {node && (
          <>
            <ResizeHandle
              width={inspectorWidth}
              min={320}
              max={900}
              grows="left"
              onChange={setInspectorWidth}
              onReset={() => setInspectorWidth(INSPECTOR_WIDTH)}
            />
            <WithUpstreamVariables flow={flow} node={node}>
              <NodeInspector
                key={node.id}
                node={node}
                width={inspectorWidth}
                live={live[node.id] ?? null}
                source={sourceFor(node, scan)}
                culprit={culpritFor(node, live, labels)}
                onChange={(changes) => onChange((f) => updateNode(f, node.id, changes))}
                onClose={() => onSelect(null)}
              />
            </WithUpstreamVariables>
          </>
        )}
      </div>
    </div>
  );
}

/** The autocomplete inside a node offers what its ancestors extract, on top of the environment. */
function WithUpstreamVariables({ flow, node, children }: { flow: Flow; node: FlowNode; children: React.ReactNode }) {
  const environment = useVariableNames();
  const names = useMemo(() => {
    const merged = new Set([...upstreamVariables(flow, node.id), ...environment]);
    return [...merged].sort();
  }, [flow, node.id, environment]);
  return <VariablesContext.Provider value={names}>{children}</VariablesContext.Provider>;
}

function culpritFor(node: FlowNode, live: LiveState, labels: Record<string, string>): string | null {
  const reason = live[node.id]?.result?.reason;
  return reason ? (labels[reason.node] ?? null) : null;
}

