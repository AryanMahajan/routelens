import { describe, expect, it } from "vitest";
import {
  addNode,
  ancestors,
  applyEvent,
  conditionNode,
  connect,
  COALESCE_MS,
  connectedComponent,
  displayNode,
  duplicateNodes,
  edgeId,
  edgeState,
  emptyFlow,
  emptyHistory,
  hasCycle,
  HISTORY_LIMIT,
  moveNodes,
  nodeLabel,
  pathOf,
  placeNew,
  record,
  redo,
  removeNodes,
  requestNode,
  runScope,
  undo,
  upstreamVariables,
  variablesNode,
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
    output: null,
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

describe("blocks and scopes", () => {
  it("labels variables and display blocks by their content", () => {
    const vars = { ...variablesNode({ x: 0, y: 0 }), type: "variables" as const, variables: [{ name: "who", value: "ann" }, { name: "", value: "" }] };
    expect(nodeLabel(vars)).toBe("variables: who");
    expect(nodeLabel(variablesNode({ x: 0, y: 0 }))).toBe("variables");
    expect(nodeLabel({ ...displayNode({ x: 0, y: 0 }), type: "display", text: "hi {{who}}" })).toBe("display: hi {{who}}");
  });

  it("an unconnected variables block is in scope for every card; a wired one only downstream", () => {
    let flow = emptyFlow("f");
    const a = get("/a");
    const b = get("/b");
    flow = addNode(addNode(flow, a), b, { id: a.id });
    const inputs = { ...variablesNode({ x: 0, y: 0 }), type: "variables" as const, variables: [{ name: "who", value: "ann" }] };
    flow = addNode(flow, inputs);
    const mid = { ...variablesNode({ x: 0, y: 0 }), type: "variables" as const, variables: [{ name: "token", value: "t" }] };
    flow = addNode(flow, mid, { id: a.id });
    flow = connect(flow, mid.id, b.id, null);

    expect(upstreamVariables(flow, a.id)).toEqual(["who"]);
    expect(upstreamVariables(flow, b.id)).toEqual(["token", "who"]);
  });

  it("run scope: all, the wired group plus inputs, or one step plus inputs", () => {
    let flow = emptyFlow("f");
    const a = get("/a");
    const b = get("/b");
    const island = get("/island");
    const inputs = variablesNode({ x: 0, y: 0 });
    flow = addNode(addNode(addNode(addNode(flow, a), b, { id: a.id }), island), inputs);

    expect(runScope(flow, "all", b.id)).toBeNull();
    expect(runScope(flow, "connected", null)).toBeNull();
    expect(runScope(flow, "connected", b.id)).toEqual([a.id, b.id, inputs.id]);
    expect(runScope(flow, "step", b.id)).toEqual([b.id, inputs.id]);
    expect([...connectedComponent(flow, island.id)]).toEqual([island.id]);
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

  it("undo steps back through recorded edits, redo forward, and a new edit drops the redo stack", () => {
    const v0 = emptyFlow("f");
    const v1 = addNode(v0, get("{{base_url}}/a"));
    const v2 = addNode(v1, get("{{base_url}}/b"));

    let h = record(emptyHistory, v0, 1000);
    h = record(h, v1, 2000);
    expect(h.past).toEqual([v0, v1]);

    const back = undo(h, v2)!;
    expect(back.flow).toBe(v1);
    expect(back.history.future).toEqual([v2]);
    const further = undo(back.history, back.flow)!;
    expect(further.flow).toBe(v0);
    expect(undo(further.history, further.flow)).toBeNull();

    const forward = redo(further.history, further.flow)!;
    expect(forward.flow).toBe(v1);
    expect(forward.history.past).toEqual([v0]);
    expect(forward.history.future).toEqual([v2]);

    // Editing after an undo: what was undone is gone for good.
    const h2 = record(forward.history, forward.flow, 5000);
    expect(h2.future).toEqual([]);
    expect(redo(h2, v1)).toBeNull();
  });

  it("edits in quick succession are one undo step, and the history is bounded", () => {
    const v0 = emptyFlow("f");
    const v1 = { ...v0, name: "fl" };
    const v2 = { ...v0, name: "flo" };
    let h = record(emptyHistory, v0, 1000);
    h = record(h, v1, 1000 + COALESCE_MS - 1);
    h = record(h, v2, 1000 + COALESCE_MS * 2 - 2);
    expect(h.past, "typing three characters is one step").toEqual([v0]);

    h = record(h, v2, 10_000);
    expect(h.past).toEqual([v0, v2]);

    // Two cards added within the window are still two steps: the shape changed.
    const w1 = addNode(v2, get("{{base_url}}/a"));
    const w2 = addNode(w1, get("{{base_url}}/b"));
    h = record(h, w1, 10_050);
    h = record(h, w2, 10_100);
    expect(h.past).toEqual([v0, v2, w1, w2]);
    // A move ends once and is a step of its own even right after another edit.
    const moved = moveNodes(w2, { [w2.nodes[0]!.id]: { x: 9, y: 9 } });
    h = record(h, moved, 10_150);
    expect(h.past.length).toBe(5);

    for (let i = 0; i < HISTORY_LIMIT * 2; i++) h = record(h, { ...v0, name: `n${i}` }, 20_000 + i * 1000);
    expect(h.past.length).toBe(HISTORY_LIMIT);
    expect(h.past.at(-1)!.name).toBe(`n${HISTORY_LIMIT * 2 - 1}`);
  });
});
