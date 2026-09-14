import { describe, expect, it } from "vitest";
import { tokenize } from "./CodeView";

describe("json tokenizer", () => {
  it("tells keys from strings and colours every scalar kind", () => {
    const kinds = tokenize('  "name": "Ann", "n": -1.5e3, "ok": true, "x": null,').map((t) => t.kind);
    expect(kinds).toEqual([
      "plain",
      "key", "punct", "plain", "string", "punct", "plain",
      "key", "punct", "plain", "number", "punct", "plain",
      "key", "punct", "plain", "boolean", "punct", "plain",
      "key", "punct", "plain", "null", "punct",
    ]);
  });

  it("keeps escaped quotes inside a string and reassembles the line exactly", () => {
    const line = '{"say": "he said \\"hi\\"", "list": [1, 2]}';
    const tokens = tokenize(line);
    expect(tokens.map((t) => t.text).join("")).toBe(line);
    expect(tokens.find((t) => t.kind === "string")?.text).toBe('"he said \\"hi\\""');
    expect(tokenize("").length).toBe(0);
  });
});
