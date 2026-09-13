import {
  Background,
  BackgroundVariant,
  Controls,
  MiniMap,
  ReactFlow,
  ReactFlowProvider,
  applyEdgeChanges,
  applyNodeChanges,
  useReactFlow,
  type Connection,
  type Edge,
  type EdgeChange,
  type NodeChange,
} from "@xyflow/react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  connect,
  duplicateNodes,
  edgeId,
  edgeState,
  moveNodes,
  nodeLabel,
  removeEdges,
  removeNodes,
  type LiveState,
} from "../../flow";
import type { Flow, FlowNode, Position as FlowPosition } from "../../flowTypes";
import type { ScanResult, SourceView } from "../../types";
import { ConditionNode, RequestNode, type RfNode } from "./nodes";

/** The MIME type an endpoint dragged out of the API panel carries. */
export const ENDPOINT_DRAG_TYPE = "application/x-routelens-endpoint";

const nodeTypes = { request: RequestNode, condition: ConditionNode };

export type FlowUpdate = (flow: Flow) => Flow;

/**
 * The canvas. React Flow draws it; the `Flow` in the tab is the truth.
 *
 * Every edit — connect, delete, move, duplicate — is expressed as an update to the flow
 * document and handed up. React Flow's own node and edge state is rebuilt from the document
 * whenever it changes, keeping only what React Flow alone knows: measured sizes, an
 * in-progress drag, the selection.
 */
export function FlowCanvas(props: FlowCanvasProps) {
  return (
    <ReactFlowProvider>
      <Canvas {...props} />
    </ReactFlowProvider>
  );
}

export interface FlowCanvasProps {
  flow: Flow;
  live: LiveState;
  /** The node whose inspector is open. */
  selected: string | null;
  scan: ScanResult | null;
  onChange: (update: FlowUpdate) => void;
  onSelect: (id: string | null) => void;
  /** An endpoint from the API panel was dropped here. */
  onDropEndpoint: (endpointId: string, position: FlowPosition) => void;
}

function Canvas({ flow, live, selected, scan, onChange, onSelect, onDropEndpoint }: FlowCanvasProps) {
  const { screenToFlowPosition, fitView } = useReactFlow();
  const [nodes, setNodes] = useState<RfNode[]>([]);
  const nodesRef = useRef(nodes);
  nodesRef.current = nodes;
  const [edges, setEdges] = useState<Edge[]>([]);
  const [notice, setNotice] = useState<string | null>(null);
  const noticeTimer = useRef<number | null>(null);

  const say = useCallback((text: string) => {
    setNotice(text);
    if (noticeTimer.current) window.clearTimeout(noticeTimer.current);
    noticeTimer.current = window.setTimeout(() => setNotice(null), 2200);
  }, []);

  const labels = useMemo(() => {
    const out: Record<string, string> = {};
    for (const n of flow.nodes) out[n.id] = nodeLabel(n);
    return out;
  }, [flow.nodes]);

  // Rebuild React Flow's nodes from the document, keeping what only React Flow knows.
  useEffect(() => {
    setNodes((previous) => {
      const old = new Map(previous.map((n) => [n.id, n]));
      return flow.nodes.map((node): RfNode => {
        const was = old.get(node.id);
        const state = live[node.id] ?? null;
        const culprit = culpritOf(state, labels);
        const common = {
          id: node.id,
          // A drag in progress owns the position until it ends.
          position: was?.dragging ? was.position : node.position,
          // The tab's `selected` is the single selection and wins whenever it names a
          // node; a multi-selection reports as null and is React Flow's to keep.
          selected: selected !== null ? node.id === selected : (was?.selected ?? false),
          dragging: was?.dragging,
          measured: was?.measured,
        };
        if (node.type === "condition") {
          return { ...common, type: "condition", data: { node, live: state, culprit } };
        }
        return {
          ...common,
          type: "request",
          data: { node, live: state, culprit, source: sourceFor(node, scan) },
        };
      });
    });
  }, [flow.nodes, live, labels, scan, selected]);

  useEffect(() => {
    setEdges((previous) => {
      const old = new Map(previous.map((e) => [e.id, e]));
      return flow.edges.map((edge): Edge => {
        const id = edgeId(edge);
        const state = edgeState(edge, live);
        const targetRunning = live[edge.to]?.status === "running";
        return {
          id,
          source: edge.from,
          target: edge.to,
          sourceHandle: edge.handle ?? undefined,
          label: edge.handle ?? undefined,
          className: `rl-edge rl-edge-${state}`,
          animated: targetRunning,
          selected: old.get(id)?.selected ?? false,
        };
      });
    });
  }, [flow.edges, live]);

  // Keep everything in view as the flow grows: the first card, and every card added after
  // it — a step added beside the selected one would otherwise land behind the inspector.
  const seen = useRef(0);
  useEffect(() => {
    const grew = flow.nodes.length > seen.current;
    seen.current = flow.nodes.length;
    if (!grew) return;
    const id = window.setTimeout(
      () => void fitView({ padding: 0.2, maxZoom: 1, duration: 200 }),
      20,
    );
    return () => window.clearTimeout(id);
  }, [flow.nodes.length, fitView]);

  // Selection is reported from here — the changes React Flow emits for clicks and box
  // selection — and never from `onSelectionChange`, which also fires when the nodes are
  // rebuilt from the document and would feed a stale selection straight back up.
  const onNodesChange = useCallback(
    (changes: NodeChange<RfNode>[]) => {
      const next = applyNodeChanges(changes, nodesRef.current);
      setNodes(next);
      if (changes.some((c) => c.type === "select")) {
        const picked = next.filter((n) => n.selected);
        onSelect(picked.length === 1 ? picked[0]!.id : null);
      }
      const removed = changes.filter((c) => c.type === "remove").map((c) => c.id);
      if (removed.length > 0) {
        onChange((f) => removeNodes(f, removed));
        if (selected && removed.includes(selected)) onSelect(null);
      }
    },
    [onChange, onSelect, selected],
  );

  const onEdgesChange = useCallback(
    (changes: EdgeChange[]) => {
      setEdges((current) => applyEdgeChanges(changes, current));
      const removed = changes.filter((c) => c.type === "remove").map((c) => c.id);
      if (removed.length > 0) onChange((f) => removeEdges(f, removed));
    },
    [onChange],
  );

  const onConnect = useCallback(
    (connection: Connection) => {
      if (!connection.source || !connection.target) return;
      const { source, target } = connection;
      const handle = connection.sourceHandle ?? null;
      // Decide against the document as rendered, so the refusal can be shown; the update
      // itself is applied to whatever the document is by then.
      if (connect(flow, source, target, handle) === flow) {
        say("Not connected — that would loop, or already exists.");
        return;
      }
      onChange((f) => connect(f, source, target, handle));
    },
    [flow, onChange, say],
  );

  const onNodeDragStop = useCallback(
    (_event: MouseEvent | TouchEvent, _node: RfNode, dragged: RfNode[]) => {
      const positions: Record<string, FlowPosition> = {};
      for (const n of dragged) positions[n.id] = { x: Math.round(n.position.x), y: Math.round(n.position.y) };
      onChange((f) => moveNodes(f, positions));
    },
    [onChange],
  );

  // Ctrl+D duplicates, Ctrl+A selects all, Escape clears — when the canvas has focus, not
  // an input.
  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      const target = event.target as HTMLElement | null;
      if (target && (target.closest("input, textarea, select, [contenteditable]") || !target.closest(".react-flow"))) {
        return;
      }
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "d") {
        event.preventDefault();
        const ids = nodesRef.current.filter((n) => n.selected).map((n) => n.id);
        if (ids.length === 0) return;
        onChange((f) => {
          const { flow: next, ids: copies } = duplicateNodes(f, ids);
          window.setTimeout(() => {
            setNodes((current) => current.map((n) => ({ ...n, selected: copies.includes(n.id) })));
            onSelect(copies.length === 1 ? (copies[0] ?? null) : null);
          }, 0);
          return next;
        });
      } else if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "a") {
        event.preventDefault();
        setNodes((current) => current.map((n) => ({ ...n, selected: true })));
      } else if (event.key === "Escape") {
        setNodes((current) => current.map((n) => (n.selected ? { ...n, selected: false } : n)));
        setEdges((current) => current.map((e) => (e.selected ? { ...e, selected: false } : e)));
        onSelect(null);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onChange, onSelect]);

  return (
    <div
      className="relative min-h-0 min-w-0 flex-1"
      onDragOver={(event) => {
        if (event.dataTransfer.types.includes(ENDPOINT_DRAG_TYPE)) {
          event.preventDefault();
          event.dataTransfer.dropEffect = "copy";
        }
      }}
      onDrop={(event) => {
        const id = event.dataTransfer.getData(ENDPOINT_DRAG_TYPE);
        if (!id) return;
        event.preventDefault();
        const position = screenToFlowPosition({ x: event.clientX, y: event.clientY });
        onDropEndpoint(id, { x: Math.round(position.x) - 130, y: Math.round(position.y) - 30 });
      }}
    >
      <ReactFlow
        nodes={nodes}
        edges={edges}
        nodeTypes={nodeTypes}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        onConnect={onConnect}
        onNodeDragStop={onNodeDragStop}
        deleteKeyCode={["Delete", "Backspace"]}
        multiSelectionKeyCode={["Control", "Meta"]}
        selectionKeyCode="Shift"
        minZoom={0.2}
        maxZoom={2}
        defaultEdgeOptions={{ type: "default" }}
        fitView={false}
        nodesFocusable
        edgesFocusable
        proOptions={{ hideAttribution: false }}
      >
        <Background variant={BackgroundVariant.Dots} gap={20} size={1} />
        <Controls showInteractive={false} position="bottom-left" />
        <MiniMap
          position="bottom-right"
          pannable
          zoomable
          nodeColor={(n) => {
            const status = (n.data as { live?: { status?: string } | null }).live?.status;
            if (status === "passed") return "var(--t-get)";
            if (status === "failed") return "var(--t-delete)";
            if (status === "running") return "var(--t-accent)";
            return "var(--t-raised)";
          }}
        />
      </ReactFlow>

      {flow.nodes.length === 0 && (
        <div className="pointer-events-none absolute inset-0 flex items-center justify-center">
          <div className="max-w-sm rounded-lg border border-dashed border-edge bg-panel/80 p-5 text-center text-muted">
            <p className="mb-1 font-semibold text-ink">An empty flow</p>
            <p>
              Drag an endpoint in from the <span className="text-ink">API</span> panel, click one
              there, or use <span className="text-ink">+ Add</span> above. Connect cards by
              dragging from a right-hand handle to a left-hand one.
            </p>
          </div>
        </div>
      )}

      {notice && (
        <div className="pointer-events-none absolute inset-x-0 bottom-4 flex justify-center">
          <span className="rounded border border-method-post/40 bg-panel px-3 py-1.5 text-[11px] text-method-post shadow">
            {notice}
          </span>
        </div>
      )}
    </div>
  );
}

function culpritOf(state: LiveState[string] | null, labels: Record<string, string>): string | null {
  const reason = state?.result?.reason;
  if (!reason) return null;
  return labels[reason.node] ?? null;
}

/** The source location of the endpoint a node was built from, if the current scan has it. */
export function sourceFor(node: FlowNode, scan: ScanResult | null): SourceView | null {
  if (node.type !== "request" || !node.request.spec_ref || !scan) return null;
  return scan.endpoints.find((e) => e.id === node.request.spec_ref)?.source ?? null;
}
