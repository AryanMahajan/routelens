import { describe, expect, it } from "vitest";
import { matchingHistory, pathsOf, responseOf, shapeOf, suggestName, templatePattern } from "./responseShape";
import type { HistoryEntry, HttpResponse } from "./types";

function response(body: string, headers: [string, string][] = [["content-type", "application/json"]]): HttpResponse {
  return {
    status: 200,
    status_text: "OK",
    headers,
    body: { bytes: body, truncated: false },
    timing: { ttfb_ms: 1, total_ms: 2 },
    redirects: [],
    insecure: false,
  };
}

describe("response shape", () => {
  it("lists every path with a preview, sampling arrays and quoting odd keys", () => {
    const paths = pathsOf('{"user":{"id":42,"name":"Ann","odd key":true},"items":[{"id":"a"},{"id":"b"},{"id":"c"},{"id":"d"}],"none":null}');
    expect(paths.map((p) => p.path)).toEqual([
      "user",
      "user.id",
      "user.name",
      'user["odd key"]',
      "items",
      "items[0]",
      "items[0].id",
      "items[1]",
      "items[1].id",
      "items[2]",
      "items[2].id",
      "none",
    ]);
    const byPath = Object.fromEntries(paths.map((p) => [p.path, p]));
    expect(byPath["user"]).toMatchObject({ preview: "{3 keys}", scalar: false });
    expect(byPath["items"]).toMatchObject({ preview: "[4 items]", scalar: false });
    expect(byPath["user.id"]).toMatchObject({ preview: "42", text: "42", scalar: true });
    expect(byPath["user.name"]).toMatchObject({ preview: '"Ann"', text: "Ann" });
    expect(byPath["none"]).toMatchObject({ preview: "null", text: "null" });
    expect(pathsOf("not json")).toEqual([]);
    expect(pathsOf('"just a string"')).toEqual([]);
  });

  it("a shape carries the status and lower-cased header names once each", () => {
    const shape = shapeOf(response("[1]", [["Content-Type", "x"], ["content-type", "y"], ["X-Trace", "z"]]), "run");
    expect(shape).toMatchObject({ status: 200, headers: ["content-type", "x-trace"], source: "run" });
    expect(shape.paths.map((p) => p.path)).toEqual(["[0]"]);
  });

  it("finds the newest history entry the request template could have produced", () => {
    const entry = (id: number, method: string, url: string, at: number): HistoryEntry => ({
      id,
      at,
      method,
      url,
      status: 200,
      request: {},
      response: response('{"ok":true}'),
    });
    const entries = [
      entry(1, "GET", "http://localhost:9000/api/v1/users/2", 100),
      entry(2, "GET", "http://localhost:9000/api/v1/users/3?x=1", 300),
      entry(3, "GET", "http://localhost:9000/api/v1/users/", 200),
      entry(4, "DELETE", "http://localhost:9000/api/v1/users/3", 400),
      entry(5, "GET", "http://localhost:9000/api/v1/users/3/posts", 500),
    ];
    const found = matchingHistory(entries, "get", "{{base_url}}/api/v1/users/{user_id}");
    expect(found?.id).toBe(2);
    expect(matchingHistory(entries, "GET", "{{base_url}}/api/v1/users/")?.id).toBe(3);
    expect(matchingHistory(entries, "PUT", "{{base_url}}/api/v1/users/{user_id}")).toBeNull();
    expect(templatePattern("{{base_url}}/v1/items/{item_id}").test("http://x/v1/items/item-4")).toBe(true);
    expect(templatePattern("{{base_url}}/v1/items/{item_id}").test("http://x/v2/items/item-4")).toBe(false);
    expect(responseOf(found!)?.body.bytes).toBe('{"ok":true}');
    expect(responseOf({ ...found!, response: null })).toBeNull();
  });

  it("suggests a variable name from the last part of a path", () => {
    expect(suggestName("user.id")).toBe("id");
    expect(suggestName("items[0].name")).toBe("name");
    expect(suggestName("items[0]")).toBe("items");
    expect(suggestName('user["odd key"]')).toBe("odd_key");
    expect(suggestName("access_token")).toBe("access_token");
  });
});
