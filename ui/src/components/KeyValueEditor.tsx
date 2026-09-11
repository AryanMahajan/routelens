import type { KeyValue } from "../types";
import { VariableInput } from "./VariableInput";

/**
 * The query / header / cookie table.
 *
 * A disabled row stays in the list rather than being deleted, which mirrors how the workspace
 * file stores it: toggling a parameter off should be a one-word change, not a removed line.
 */
export function KeyValueEditor({
  rows,
  onChange,
  keyPlaceholder = "name",
  valuePlaceholder = "value",
}: {
  rows: KeyValue[];
  onChange: (rows: KeyValue[]) => void;
  keyPlaceholder?: string;
  valuePlaceholder?: string;
}) {
  function update(index: number, patch: Partial<KeyValue>) {
    const next = rows.map((row, i) => (i === index ? { ...row, ...patch } : row));
    onChange(next);
  }

  function remove(index: number) {
    onChange(rows.filter((_, i) => i !== index));
  }

  function add() {
    onChange([...rows, { key: "", value: "", enabled: true }]);
  }

  return (
    <div className="flex flex-col gap-px">
      {rows.length === 0 && (
        <p className="px-1 py-3 text-muted italic">Nothing here yet.</p>
      )}

      {rows.map((row, index) => (
        <div
          key={index}
          className="group flex items-center gap-2 rounded px-1 py-1 hover:bg-raised/60"
        >
          <input
            type="checkbox"
            checked={row.enabled}
            onChange={(e) => update(index, { enabled: e.target.checked })}
            className="size-3.5 shrink-0 accent-accent"
            aria-label={row.key ? `Enable ${row.key}` : "Enable row"}
          />
          <input
            value={row.key}
            onChange={(e) => update(index, { key: e.target.value })}
            placeholder={keyPlaceholder}
            spellCheck={false}
            className={`w-2/5 min-w-0 rounded border border-transparent bg-transparent px-2 py-1 font-mono
              outline-none placeholder:text-muted/60 focus:border-edge focus:bg-ground
              ${row.enabled ? "" : "text-muted line-through"}`}
          />
          <VariableInput
            value={row.value}
            onChange={(value) => update(index, { value })}
            placeholder={valuePlaceholder}
            className={`rounded border border-transparent bg-transparent px-2 py-1 font-mono
              outline-none placeholder:text-muted/60 focus:border-edge focus:bg-ground
              ${row.enabled ? "" : "text-muted line-through"}`}
          />
          <button
            onClick={() => remove(index)}
            title="Remove row"
            className="shrink-0 rounded px-1.5 py-0.5 text-muted opacity-0 transition
              hover:bg-method-delete/15 hover:text-method-delete group-hover:opacity-100"
          >
            ✕
          </button>
        </div>
      ))}

      <button
        onClick={add}
        className="mt-1 self-start rounded px-2 py-1 text-muted transition hover:bg-raised hover:text-ink"
      >
        + Add
      </button>
    </div>
  );
}
