/**
 * The wire format for flows and flow runs.
 *
 * Mirrors `rl_model::flow` and `rl_flow::run`. A node is its request plus what to extract
 * from the response and what to assert on it; edges are dependencies. Everything the core
 * marks `skip_serializing_if` arrives absent and is filled in by `normalizeFlow` /
 * `normalizeFlowRun`, the same way `normalizeRequest` does for requests.
 */

import { normalizeRequest, type Exchange, type RequestDraft, type WireRequestDraft } from "./types";

export const HANDLE_TRUE = "true";
export const HANDLE_FALSE = "false";

export type Operator =
  | "equals"
  | "not_equals"
  | "contains"
  | "not_contains"
  | "exists"
  | "not_exists"
  | "greater_than"
  | "less_than";

export const OPERATORS: { value: Operator; label: string }[] = [
  { value: "equals", label: "==" },
  { value: "not_equals", label: "!=" },
  { value: "contains", label: "contains" },
  { value: "not_contains", label: "not contains" },
  { value: "exists", label: "exists" },
  { value: "not_exists", label: "not exists" },
  { value: "greater_than", label: ">" },
  { value: "less_than", label: "<" },
];

export function isUnary(op: Operator): boolean {
  return op === "exists" || op === "not_exists";
}

export function operatorLabel(op: Operator): string {
  return OPERATORS.find((o) => o.value === op)?.label ?? op;
}

/** Where a value comes from. Flattened into extractions and assertions on the wire. */
export type ValueSource =
  | { from: "status" }
  | { from: "header"; header: string }
  | { from: "body"; path: string }
  | { from: "body_text" }
  | { from: "duration" };

export type Extraction = { name: string } & ValueSource;

export type Assertion = { op: Operator; expected: string } & ValueSource;

export interface Position {
  x: number;
  y: number;
}

export type RequestNodeKind = {
  type: "request";
  request: RequestDraft;
  extract: Extraction[];
  assert: Assertion[];
};

export type ConditionNodeKind = { type: "condition"; left: string; op: Operator; right: string };

export type NodeKind = RequestNodeKind | ConditionNodeKind;

export type FlowNode = {
  id: string;
  name: string | null;
  position: Position;
} & NodeKind;

export interface FlowEdge {
  from: string;
  to: string;
  /** `true` / `false` out of a condition; null out of a request. */
  handle: string | null;
}

export interface Flow {
  version: number;
  name: string;
  description: string | null;
  nodes: FlowNode[];
  edges: FlowEdge[];
}

type WireFlowNode = {
  id: string;
  name?: string | null;
  position?: Partial<Position>;
} & (
  | {
      type: "request";
      request: WireRequestDraft;
      extract?: Extraction[];
      assert?: ({ op: Operator; expected?: string } & ValueSource)[];
    }
  | { type: "condition"; left: string; op: Operator; right?: string }
);

export interface WireFlow {
  version: number;
  name: string;
  description?: string | null;
  nodes?: WireFlowNode[];
  edges?: { from: string; to: string; handle?: string | null }[];
}

export function normalizeFlow(wire: WireFlow): Flow {
  return {
    version: wire.version,
    name: wire.name,
    description: wire.description ?? null,
    nodes: (wire.nodes ?? []).map((node): FlowNode => {
      const base = {
        id: node.id,
        name: node.name ?? null,
        position: { x: node.position?.x ?? 0, y: node.position?.y ?? 0 },
      };
      if (node.type === "condition") {
        return { ...base, type: "condition", left: node.left, op: node.op, right: node.right ?? "" };
      }
      return {
        ...base,
        type: "request",
        request: normalizeRequest(node.request),
        extract: node.extract ?? [],
        assert: (node.assert ?? []).map((a) => ({ ...a, expected: a.expected ?? "" })),
      };
    }),
    edges: (wire.edges ?? []).map((e) => ({ from: e.from, to: e.to, handle: e.handle ?? null })),
  };
}

// --- runs --------------------------------------------------------------------------------

export type Failure =
  | { kind: "undefined_variables"; names: string[] }
  | { kind: "resolve"; message: string }
  | { kind: "transport"; message: string }
  | { kind: "extraction"; name: string; message: string }
  | { kind: "assertions"; failed: number };

export type SkipReason =
  | { kind: "upstream_failed"; node: string }
  | { kind: "branch_not_taken"; node: string; handle: string };

export interface AssertionResult {
  assertion: Assertion;
  /** The right-hand side with variables resolved. */
  expected: string;
  actual: string | null;
  passed: boolean;
  error: string | null;
}

export type NodeStatus = "passed" | "failed" | "skipped";

export interface NodeResult {
  node: string;
  status: NodeStatus;
  failure: Failure | null;
  reason: SkipReason | null;
  duration_ms: number;
  /** The request as resolved and attempted; null when it never got that far. */
  request: RequestDraft | null;
  exchange: Exchange | null;
  extracted: { name: string; value: string }[];
  assertions: AssertionResult[];
  /** Which output a condition took. */
  branch: string | null;
  compared: [string, string] | null;
}

export interface FlowRun {
  flow: string;
  started_at: number;
  duration_ms: number;
  results: NodeResult[];
  variables: Record<string, string>;
  summary: { passed: number; failed: number; skipped: number };
}

export type FlowEvent =
  | { event: "started"; order: string[] }
  | { event: "node_started"; node: string }
  | { event: "node_finished"; result: NodeResult }
  | { event: "finished"; run: FlowRun };

/** Everything the core omits on the wire, filled in. */
export function normalizeNodeResult(wire: NodeResult): NodeResult {
  const exchange = wire.exchange
    ? {
        ...wire.exchange,
        response: { ...wire.exchange.response, redirects: wire.exchange.response.redirects ?? [] },
      }
    : null;
  return {
    ...wire,
    failure: wire.failure ?? null,
    reason: wire.reason ?? null,
    request: wire.request ? normalizeRequest(wire.request as unknown as WireRequestDraft) : null,
    exchange,
    extracted: wire.extracted ?? [],
    assertions: (wire.assertions ?? []).map((a) => ({
      ...a,
      actual: a.actual ?? null,
      error: a.error ?? null,
      assertion: { ...a.assertion, expected: a.assertion.expected ?? "" },
    })),
    branch: wire.branch ?? null,
    compared: wire.compared ?? null,
  };
}

export function normalizeFlowRun(wire: FlowRun): FlowRun {
  return {
    ...wire,
    results: wire.results.map(normalizeNodeResult),
    variables: wire.variables ?? {},
  };
}

/** Why a node failed, in a sentence. */
export function describeFailure(failure: Failure): string {
  switch (failure.kind) {
    case "undefined_variables":
      return `undefined variable${failure.names.length === 1 ? "" : "s"}: ${failure.names.join(", ")}`;
    case "resolve":
    case "transport":
      return failure.message;
    case "extraction":
      return `could not extract {{${failure.name}}}: ${failure.message}`;
    case "assertions":
      return `${failure.failed} assertion${failure.failed === 1 ? "" : "s"} failed`;
  }
}

/** A value source as a short label: `status`, `header Location`, `body.user.id`. */
export function describeSource(source: ValueSource): string {
  switch (source.from) {
    case "status":
      return "status";
    case "duration":
      return "duration";
    case "body_text":
      return "body";
    case "header":
      return `header ${source.header}`;
    case "body": {
      const path = source.path.replace(/^\$\.?/, "");
      return path ? `body.${path}` : "body";
    }
  }
}
