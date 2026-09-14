/**
 * Pure operations on a flow document and on the live state of a run.
 *
 * Everything the canvas does to a flow — add, connect, delete, duplicate, move — goes
 * through here and returns a new `Flow`, so the document in the tab is always the single
 * source of truth and React Flow is only ever a view of it. No React, no Tauri, so this file
 * is tested on its own.
 */

import {
  HANDLE_TRUE,
  operatorLabel,
  type Flow,
  type FlowEdge,
  type FlowEvent,
  type FlowNode,
  type NodeResult,
  type NodeStatus,
  type Position,
} from "./flowTypes";
import type { RequestDraft } from "./types";

export function emptyFlow(name: string): Flow {
  return { version: 1, name, description: null, nodes: [], edges: [] };
}

/** What the card and the tab call a node. */
export function nodeLabel(node: FlowNode): string {
  if (node.name?.trim()) return node.name.trim();
  if (node.type === "condition") {
    const right = node.op === "exists" || node.op === "not_exists" ? "" : ` ${node.right}`;
    return `if ${node.left} ${operatorLabel(node.op)}${right}`.trim();
  }
  if (node.type === "variables") {
    const names = node.variables.map((v) => v.name).filter(Boolean);
    return names.length ? `variables: ${names.join(", ")}` : "variables";
  }
  if (node.type === "display") {
    return node.text.trim() ? `display: ${node.text.trim()}` : "display";
  }
  return node.request.name?.trim() || `${node.request.method} ${pathOf(node.request.url)}`;
}

/** The part of a URL worth showing on a card: the path, without `{{base_url}}` or origin. */
export function pathOf(url: string): string {
  const withoutVariable = url.replace(/^\{\{[^}]*\}\}/, "");
  const withoutOrigin = withoutVariable.replace(/^[a-z][a-z0-9+.-]*:\/\/[^/]*/i, "");
  return withoutOrigin || url || "/";
}

/** A request node with the one check every new step starts with: it did not fail. */
export function requestNode(request: RequestDraft, position: Position): FlowNode {
  return {
    id: crypto.randomUUID(),
    name: null,
    position,
    type: "request",
    request: { ...request, id: crypto.randomUUID() },
    extract: [],
    assert: [{ from: "status", op: "less_than", expected: "400" }],
  };
}

export function conditionNode(position: Position): FlowNode {
  return {
    id: crypto.randomUUID(),
    name: null,
    position,
    type: "condition",
    left: "",
    op: "equals",
    right: "",
  };
}

export function variablesNode(position: Position): FlowNode {
  return {
    id: crypto.randomUUID(),
    name: null,
    position,
    type: "variables",
    variables: [{ name: "", value: "" }],
  };
}

export function displayNode(position: Position): FlowNode {
  return { id: crypto.randomUUID(), name: null, position, type: "display", text: "" };
}

/** An unconnected variables block: the flow's inputs, which run before everything else. */
export function isInputBlock(flow: Flow, node: FlowNode): boolean {
  return node.type === "variables" && !flow.edges.some((e) => e.to === node.id);
}

/** Roughly how wide a card is, for placing the next one beside it. */
export const NODE_WIDTH = 260;
export const NODE_GAP = 80;

/**
 * Where a new node should go: to the right of the selected node, otherwise below
 * everything that exists, otherwise at the origin. Deterministic, so the same click always
 * lands the card in the same place.
 */
export function placeNew(flow: Flow, after: string | null): Position {
  const anchor = after ? flow.nodes.find((n) => n.id === after) : undefined;
  if (anchor) {
    // Stack below any node that already sits to the anchor's right, so a fan-out reads as
    // a column rather than a pile.
    const x = anchor.position.x + NODE_WIDTH + NODE_GAP;
    const column = flow.nodes.filter((n) => Math.abs(n.position.x - x) < NODE_WIDTH / 2);
    const y = column.length
      ? Math.max(...column.map((n) => n.position.y)) + 140
      : anchor.position.y;
    return { x, y };
  }
  if (flow.nodes.length === 0) return { x: 80, y: 120 };
  const minX = Math.min(...flow.nodes.map((n) => n.position.x));
  const maxY = Math.max(...flow.nodes.map((n) => n.position.y));
  return { x: minX, y: maxY + 160 };
}

export function edgeId(edge: FlowEdge): string {
  return `${edge.from}|${edge.handle ?? ""}|${edge.to}`;
}

/** Add a node, wiring it after `from` when one is given. */
export function addNode(
  flow: Flow,
  node: FlowNode,
  from: { id: string; handle?: string | null } | null = null,
): Flow {
  const nodes = [...flow.nodes, node];
  let edges = flow.edges;
  if (from) {
    const source = flow.nodes.find((n) => n.id === from.id);
    if (source) {
      const handle = from.handle ?? (source.type === "condition" ? HANDLE_TRUE : null);
      edges = [...edges, { from: from.id, to: node.id, handle }];
    }
  }
  return { ...flow, nodes, edges };
}

/**
 * Connect two nodes. Refuses a self-loop, a duplicate, a handle the source does not have,
 * and anything that would close a cycle — a flow with a cycle has no first step, and it is
 * clearer to refuse the drag than to accept it and fail at run time.
 */
export function connect(flow: Flow, from: string, to: string, handle: string | null): Flow {
  if (from === to) return flow;
  const source = flow.nodes.find((n) => n.id === from);
  const target = flow.nodes.find((n) => n.id === to);
  if (!source || !target) return flow;
  const wanted = source.type === "condition" ? (handle ?? HANDLE_TRUE) : null;
  const edge: FlowEdge = { from, to, handle: wanted };
  if (flow.edges.some((e) => edgeId(e) === edgeId(edge))) return flow;
  const next = { ...flow, edges: [...flow.edges, edge] };
  return hasCycle(next) ? flow : next;
}

export function removeNodes(flow: Flow, ids: Iterable<string>): Flow {
  const gone = new Set(ids);
  if (gone.size === 0) return flow;
  return {
    ...flow,
    nodes: flow.nodes.filter((n) => !gone.has(n.id)),
    edges: flow.edges.filter((e) => !gone.has(e.from) && !gone.has(e.to)),
  };
}

export function removeEdges(flow: Flow, ids: Iterable<string>): Flow {
  const gone = new Set(ids);
  if (gone.size === 0) return flow;
  return { ...flow, edges: flow.edges.filter((e) => !gone.has(edgeId(e))) };
}

export function moveNodes(flow: Flow, positions: Record<string, Position>): Flow {
  return {
    ...flow,
    nodes: flow.nodes.map((n) => (positions[n.id] ? { ...n, position: positions[n.id]! } : n)),
  };
}

export function updateNode(flow: Flow, id: string, changes: Partial<FlowNode>): Flow {
  return {
    ...flow,
    nodes: flow.nodes.map((n) => (n.id === id ? ({ ...n, ...changes } as FlowNode) : n)),
  };
}

/**
 * Copy nodes, keeping the edges *between* them and offsetting the copies so they are
 * visibly new. Returns the ids of the copies so they can be selected.
 */
export function duplicateNodes(flow: Flow, ids: Iterable<string>): { flow: Flow; ids: string[] } {
  const wanted = new Set(ids);
  const mapping = new Map<string, string>();
  const copies: FlowNode[] = [];
  for (const node of flow.nodes) {
    if (!wanted.has(node.id)) continue;
    const id = crypto.randomUUID();
    mapping.set(node.id, id);
    const position = { x: node.position.x + 40, y: node.position.y + 40 };
    copies.push(
      node.type === "request"
        ? { ...node, id, position, request: { ...node.request, id: crypto.randomUUID() } }
        : { ...node, id, position },
    );
  }
  if (copies.length === 0) return { flow, ids: [] };
  const edges = flow.edges
    .filter((e) => mapping.has(e.from) && mapping.has(e.to))
    .map((e) => ({ ...e, from: mapping.get(e.from)!, to: mapping.get(e.to)! }));
  return {
    flow: { ...flow, nodes: [...flow.nodes, ...copies], edges: [...flow.edges, ...edges] },
    ids: [...mapping.values()],
  };
}

export function hasCycle(flow: Flow): boolean {
  const out = new Map<string, string[]>();
  for (const e of flow.edges) out.set(e.from, [...(out.get(e.from) ?? []), e.to]);
  const state = new Map<string, "visiting" | "done">();
  const visit = (id: string): boolean => {
    const seen = state.get(id);
    if (seen === "visiting") return true;
    if (seen === "done") return false;
    state.set(id, "visiting");
    for (const next of out.get(id) ?? []) if (visit(next)) return true;
    state.set(id, "done");
    return false;
  };
  return flow.nodes.some((n) => visit(n.id));
}

/** Every node wired to `id`, directly or through others, in either direction. */
export function connectedComponent(flow: Flow, id: string): Set<string> {
  const found = new Set<string>([id]);
  const queue = [id];
  while (queue.length) {
    const current = queue.pop()!;
    for (const e of flow.edges) {
      const other = e.from === current ? e.to : e.to === current ? e.from : null;
      if (other && !found.has(other)) {
        found.add(other);
        queue.push(other);
      }
    }
  }
  return found;
}

export type RunScope = "all" | "connected" | "step";

/**
 * Which nodes a run covers. `null` means the whole flow. A narrowed run always includes
 * the input blocks, so the flow's own variables are in scope whatever is being run.
 */
export function runScope(flow: Flow, scope: RunScope, selected: string | null): string[] | null {
  if (scope === "all" || !selected || !flow.nodes.some((n) => n.id === selected)) return null;
  const ids = scope === "step" ? new Set([selected]) : connectedComponent(flow, selected);
  for (const node of flow.nodes) if (isInputBlock(flow, node)) ids.add(node.id);
  return flow.nodes.filter((n) => ids.has(n.id)).map((n) => n.id);
}

/** Every node upstream of `id`, however far. */
export function ancestors(flow: Flow, id: string): Set<string> {
  const found = new Set<string>();
  const queue = [id];
  while (queue.length) {
    const current = queue.pop()!;
    for (const e of flow.edges) {
      if (e.to === current && !found.has(e.from)) {
        found.add(e.from);
        queue.push(e.from);
      }
    }
  }
  return found;
}

/**
 * The variable names a node can rely on: everything its ancestors extract or declare, plus
 * every input block — those run first whatever the edges say.
 */
export function upstreamVariables(flow: Flow, id: string): string[] {
  const names = new Set<string>();
  const declare = (node: FlowNode | undefined) => {
    if (node?.type === "request") for (const e of node.extract) if (e.name) names.add(e.name);
    if (node?.type === "variables") for (const v of node.variables) if (v.name) names.add(v.name);
  };
  for (const up of ancestors(flow, id)) declare(flow.nodes.find((n) => n.id === up));
  for (const node of flow.nodes) if (node.id !== id && isInputBlock(flow, node)) declare(node);
  return [...names].sort();
}

// --- undo ------------------------------------------------------------------------------

/**
 * Undo history for one flow: the documents before each edit, and the ones undone.
 *
 * Edits arriving within a short window of each other are one step — typing a URL is one
 * undo, not one per character — which is what `stamp` is for.
 */
export interface History {
  past: Flow[];
  future: Flow[];
  /** When the last step was recorded, in ms. */
  stamp: number;
}

export const HISTORY_LIMIT = 100;
export const COALESCE_MS = 400;

export const emptyHistory: History = { past: [], future: [], stamp: 0 };

/**
 * The document is about to change from `before`: remember it, and forget what was undone.
 *
 * Only edits that keep the same cards and connections merge with the previous step —
 * keystrokes in a field, a value changed twice. Adding, removing or wiring a card is always
 * its own step, however quickly it followed the last one.
 */
export function record(history: History, before: Flow, now: number): History {
  const last = history.past.at(-1);
  const merge = last !== undefined && now - history.stamp < COALESCE_MS && sameShape(last, before);
  const past = merge ? history.past : [...history.past, before].slice(-HISTORY_LIMIT);
  return { past, future: [], stamp: now };
}

function sameShape(a: Flow, b: Flow): boolean {
  return (
    a.nodes.length === b.nodes.length &&
    a.edges.length === b.edges.length &&
    a.nodes.every((n, i) => n.id === b.nodes[i]?.id && n.position === b.nodes[i]?.position) &&
    a.edges.every((e, i) => edgeId(e) === edgeId(b.edges[i]!))
  );
}

export function undo(history: History, current: Flow): { history: History; flow: Flow } | null {
  const flow = history.past.at(-1);
  if (!flow) return null;
  return {
    flow,
    history: { past: history.past.slice(0, -1), future: [current, ...history.future], stamp: 0 },
  };
}

export function redo(history: History, current: Flow): { history: History; flow: Flow } | null {
  const [flow, ...future] = history.future;
  if (!flow) return null;
  return { flow, history: { past: [...history.past, current], future, stamp: 0 } };
}

// --- live run state ----------------------------------------------------------------------

export type LiveStatus = "pending" | "running" | NodeStatus;

export interface NodeLive {
  status: LiveStatus;
  result: NodeResult | null;
}

export type LiveState = Record<string, NodeLive>;

/** Fold one run event into the per-node state the cards render from. */
export function applyEvent(live: LiveState, event: FlowEvent): LiveState {
  switch (event.event) {
    case "started": {
      const next: LiveState = {};
      for (const id of event.order) next[id] = { status: "pending", result: null };
      return next;
    }
    case "node_started":
      return { ...live, [event.node]: { status: "running", result: null } };
    case "node_finished":
      return {
        ...live,
        [event.result.node]: { status: event.result.status, result: event.result },
      };
    case "finished": {
      const next: LiveState = {};
      for (const result of event.run.results) next[result.node] = { status: result.status, result };
      return next;
    }
  }
}

export type EdgeState = "idle" | "taken" | "dead" | "inactive";

/**
 * How an edge fared, from its source's outcome: `taken` when the run went down it, `dead`
 * when a failure stopped it, `inactive` when a condition sent the run the other way.
 */
export function edgeState(edge: FlowEdge, live: LiveState): EdgeState {
  const source = live[edge.from];
  if (!source?.result) return "idle";
  const { result } = source;
  if (result.status === "failed") return "dead";
  if (result.status === "skipped") {
    return result.reason?.kind === "upstream_failed" ? "dead" : "inactive";
  }
  if (result.branch && edge.handle && result.branch !== edge.handle) return "inactive";
  return "taken";
}
