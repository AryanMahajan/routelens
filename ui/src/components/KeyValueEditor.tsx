import type { KeyValue } from "../types";
import { VariableInput } from "./VariableInput";

/**
 * The query / header / form-field table.
 *
 * A disabled row stays in the list rather than being deleted, which mirrors how the workspace
 * file stores it: toggling a parameter off should be a one-word change, not a removed line.
 *
 * Drawn as a table with a ruled key and value column — the shape every API client uses for
 * this — so a request reads the same way here as it does anywhere else.
 */
export function KeyValueEditor({
  rows,
  onChange,
  keyPlaceholder = "name",
  valuePlaceholder = "value",
  emptyText = "Nothing here yet.",
}: {
  rows: KeyValue[];
  onChange: (rows: KeyValue[]) => void;
  keyPlaceholder?: string;
  valuePlaceholder?: string;
  emptyText?: string;
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
    <div className="flex flex-col gap-2">
      {rows.length === 0 ? (
        <p className="px-1 py-2 text-muted italic">{emptyText}</p>
      ) : (
        <Table>
          {rows.map((row, index) => (
            <Row key={index} muted={!row.enabled}>
              <input
                type="checkbox"
                checked={row.enabled}
                onChange={(e) => update(index, { enabled: e.target.checked })}
                className="mr-2 size-3.5 shrink-0 accent-accent"
                aria-label={row.key ? `Enable ${row.key}` : "Enable row"}
              />
              <input
                value={row.key}
                onChange={(e) => update(index, { key: e.target.value })}
                placeholder={keyPlaceholder}
                spellCheck={false}
                className={`${cellClass} ${keyCellClass} ${row.enabled ? "" : "line-through"}`}
              />
              <VariableInput
                value={row.value}
                onChange={(value) => update(index, { value })}
                placeholder={valuePlaceholder}
                className={`${cellClass} ${row.enabled ? "" : "line-through"}`}
              />
              <button
                onClick={() => remove(index)}
                title="Remove row"
                className="mx-1 shrink-0 rounded px-1 py-0.5 text-muted opacity-0 transition
                  hover:bg-method-delete/15 hover:text-method-delete group-hover:opacity-100"
              >
                ✕
              </button>
            </Row>
          ))}
        </Table>
      )}

      <button
        onClick={add}
        className="self-start rounded px-2 py-1 text-muted transition hover:bg-raised hover:text-ink"
      >
        + Add
      </button>
    </div>
  );
}

/** The ruled frame: a header row, then one [toggle | key | value | remove] row per entry. */
export function Table({
  children,
  keyHeading = "Key",
  valueHeading = "Value",
}: {
  children: React.ReactNode;
  keyHeading?: string;
  valueHeading?: string;
}) {
  return (
    <div className="overflow-hidden rounded border border-edge">
      <div className={`${rowClass} bg-panel text-[10px] font-semibold uppercase tracking-wider text-muted`}>
        <span className="mr-2 size-3.5" />
        <span className={`px-2 py-1 ${keyCellClass}`}>{keyHeading}</span>
        <span className="px-2 py-1">{valueHeading}</span>
        <span className="w-7" />
      </div>
      <div className="divide-y divide-edge">{children}</div>
    </div>
  );
}

export function Row({ children, muted = false }: { children: React.ReactNode; muted?: boolean }) {
  return (
    <div className={`${rowClass} group hover:bg-raised/40 ${muted ? "text-muted" : ""}`}>
      {children}
    </div>
  );
}

const rowClass = "grid grid-cols-[auto_minmax(0,2fr)_minmax(0,3fr)_auto] items-center pl-2";

/** A cell input: no chrome of its own, the table's rules do the framing. */
export const cellClass =
  "min-w-0 w-full bg-transparent px-2 py-1.5 font-mono outline-none placeholder:text-muted/50 focus:bg-ground";
/** The key column carries the rule between the two columns. */
export const keyCellClass = "border-r border-edge";
