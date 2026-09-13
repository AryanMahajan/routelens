import { describe, expect, it } from "vitest";
import {
  addNode,
  ancestors,
  applyEvent,
  conditionNode,
  connect,
  duplicateNodes,
  edgeId,
  edgeState,
  emptyFlow,
  hasCycle,
  nodeLabel,
  pathOf,
  placeNew,
  removeNodes,
  requestNode,
  upstreamVariables,
  type LiveState,
} from "./flow";
import type { Flow, FlowNode, NodeResult } from "./flowTypes";
import { emptyRequest } from "./types";

function get(url: string): FlowNode & { type: "request" } {
  return requestNode({ ...emptyRequest(), url }, { x: 0, y: 0 }) as FlowNode & { type: "request" };
}

function result(node: string, patch: Partial<NodeResult> = {}): NodeResult {
  return {
    node,
    status: "passed",
    failure: null,
    reason: null,
    duration_ms: 1,
    request: null,
    exchange: null,
    extracted: [],
    assertions: [],
    branch: null,
    compared: null,
    ...patch,
  };
}

describe("cards", () => {
  it("shows the path without the base url variable or the origin", () => {
    expect(pathOf("{{base_url}}/api/users/{id}")).toBe("/api/users/{id}");
    expect(pathOf("https://api.example.com:8443/health")).toBe("/health");
    expect(pathOf("{{base_url}}")).toBe("{{base_url}}");
  });

  it("labels a node by its name, then its request, then method and path", () => {
    const node = get("{{base_url}}/me");
    expect(nodeLabel(node)).toBe("GET /me");
    expect(nodeLabel({ ...node, request: { ...node.request, name: "Who am I" } })).toBe("Who am I");
    expect(nodeLabel({ ...node, name: "Step 2" })).toBe("Step 2");

    const cond: FlowNode = { ...conditionNode({ x: 0, y: 0 }), type: "condition", op: "equals", left: "{{role}}", right: "admin" };
    expect(nodeLabel(cond)).toBe("if {{role}} == admin");
    expect(nodeLabel({ ...cond, type: "condition", op: "exists" })).toBe("if {{role}} exists");
  });

  it("a new request node starts with the did-not-fail check", () => {
    const node = get("/x");
    expect(node.assert).toEqual([
      { from: "status", op: "less_than", expected: "400" },
    ]);
  });
});

describe("editing", () => {
  it("adds after the selected node and wires it, through the true output of a condition", () => {
    let flow = emptyFlow("f");
    const a = get("/a");
    flow = addNode(flow, a);
    const cond = conditionNode(placeNew(flow, a.id));
    flow = addNode(flow, cond, { id: a.id });
    const b = get("/b");
    flow = addNode(flow, b, { id: cond.id });

    expect(flow.edges).toEqual([
      { from: a.id, to: cond.id, handle: null },
      { from: cond.id, to: b.id, handle: "true" },
    ]);
    expect(cond.position.x).toBeGreaterThan(a.position.x);
    expect(cond.position.y).toBe(a.position.y);
  });

  it("places a fan-out as a column and a new root below everything", () => {
    let flow = emptyFlow("f");
    const root = get("/root");
    flow = addNode(flow, root);
    const first = requestNode(emptyRequest(), placeNew(flow, root.id));
    flow = addNode(flow, first, { id: root.id });
    const second = requestNode(emptyRequest(), placeNew(flow, root.id));
    expect(second.position.x).toBe(first.position.x);
    expect(second.position.y).toBeGreaterThan(first.position.y);

    const orphan = placeNew(flow, null);
    expect(orphan.y).toBeGreaterThan(Math.max(root.position.y, first.position.y));
    expect(placeNew(emptyFlow("e"), null)).toEqual({ x: 80, y: 120 });
  });

  it("refuses a self-loop, a duplicate, and a connection that would close a cycle", () => {
    let flow = emptyFlow("f");
    const a = get("/a");
    const b = get("/b");
    flow = addNode(addNode(flow, a), b);
    flow = connect(flow, a.id, b.id, null);
    expect(flow.edges).toHaveLength(1);
    expect(connect(flow, a.id, b.id, null)).toBe(flow);
    expect(connect(flow, a.id, a.id, null)).toBe(flow);
    expect(connect(flow, b.id, a.id, null)).toBe(flow);
    expect(hasCycle({ ...flow, edges: [...flow.edges, { from: b.id, to: a.id, handle: null }] })).toBe(true);
  });

  it("removing a node takes its edges with it", () => {
    let flow = emptyFlow("f");
    const a = get("/a");
    const b = get("/b");
    const c = get("/c");
    flow = addNode(addNode(addNode(flow, a), b, { id: a.id }), c, { id: b.id });
    flow = removeNodes(flow, [b.id]);
    expect(flow.nodes.map((n) => n.id)).toEqual([a.id, c.id]);
    expect(flow.edges).toEqual([]);
  });

  it("duplicating keeps the edges between the copies and offsets them", () => {
    let flow = emptyFlow("f");
    const a = get("/a");
    const b = get("/b");
    flow = addNode(addNode(flow, a), b, { id: a.id });
    const { flow: next, ids } = duplicateNodes(flow, [a.id, b.id]);
    expect(ids).toHaveLength(2);
    expect(next.nodes).toHaveLength(4);
    expect(next.edges).toHaveLength(2);
    const copyA = next.nodes.find((n) => n.id === ids[0])!;
    expect(copyA.position).toEqual({ x: 40, y: 40 });
    expect(next.edges.some((e) => e.from === ids[0] && e.to === ids[1])).toBe(true);
    // Copies carry fresh request ids too, so two cards never share one.
    const requestIds = next.nodes.map((n) => (n.type === "request" ? n.request.id : ""));
    expect(new Set(requestIds).size).toBe(4);
  });

  it("knows which variables a node can rely on from its ancestors", () => {
    let flow = emptyFlow("f");
    const login = get("/login");
    login.extract.push({ name: "token", from: "body", path: "t" });
    const me = get("/me");
    me.extract.push({ name: "user_id", from: "body", path: "id" });
    const other = get("/other");
    other.extract.push({ name: "unrelated", from: "status" });
    const user = get("/user");
    flow = addNode(addNode(addNode(addNode(flow, login), me, { id: login.id }), user, { id: me.id }), other);

    expect([...ancestors(flow, user.id)].sort()).toEqual([login.id, me.id].sort());
    expect(upstreamVariables(flow, user.id)).toEqual(["token", "user_id"]);
    expect(upstreamVariables(flow, login.id)).toEqual([]);
  });
});

describe("run state", () => {
  it("folds events into per-node status", () => {
    let live: LiveState = {};
    live = applyEvent(live, { event: "started", order: ["a", "b"] });
    expect(live).toEqual({ a: { status: "pending", result: null }, b: { status: "pending", result: null } });
    live = applyEvent(live, { event: "node_started", node: "a" });
    expect(live["a"]?.status).toBe("running");
    const done = result("a", { status: "failed", failure: { kind: "assertions", failed: 1 } });
    live = applyEvent(live, { event: "node_finished", result: done });
    expect(live["a"]).toEqual({ status: "failed", result: done });
    expect(live["b"]?.status).toBe("pending");
  });

  it("colours edges by what happened at their source", () => {
    const flow: Flow = emptyFlow("f");
    const edge = { from: "a", to: "b", handle: null };
    expect(edgeState(edge, {})).toBe("idle");
    expect(edgeState(edge, { a: { status: "passed", result: result("a") } })).toBe("taken");
    expect(
      edgeState(edge, {
        a: { status: "failed", result: result("a", { status: "failed", failure: { kind: "assertions", failed: 1 } }) },
      }),
    ).toBe("dead");
    expect(
      edgeState(edge, {
        a: { status: "skipped", result: result("a", { status: "skipped", reason: { kind: "upstream_failed", node: "z" } }) },
      }),
    ).toBe("dead");
    expect(
      edgeState(edge, {
        a: { status: "skipped", result: result("a", { status: "skipped", reason: { kind: "branch_not_taken", node: "c", handle: "true" } }) },
      }),
    ).toBe("inactive");

    const taken = { from: "c", to: "b", handle: "true" };
    const notTaken = { from: "c", to: "d", handle: "false" };
    const live: LiveState = { c: { status: "passed", result: result("c", { branch: "true" }) } };
    expect(edgeState(taken, live)).toBe("taken");
    expect(edgeState(notTaken, live)).toBe("inactive");
    expect(edgeId(taken)).toBe("c|true|b");
    expect(flow.nodes).toEqual([]);
  });
});
