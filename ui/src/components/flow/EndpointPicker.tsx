import { useEffect, useMemo, useRef, useState } from "react";
import type { EndpointSpec, ScanResult } from "../../types";
import { MethodBadge } from "../MethodBadge";

export type Pick =
  | { kind: "endpoint"; endpoint: EndpointSpec }
  | { kind: "blank" }
  | { kind: "condition" }
  | { kind: "variables" }
  | { kind: "display" };

/**
 * The "+ Add" menu: the project's discovered API first, because that is the point, plus a
 * blank request and a condition. Type to filter; Enter takes the first match.
 */
export function EndpointPicker({
  scan,
  onPick,
  onClose,
}: {
  scan: ScanResult | null;
  onPick: (pick: Pick) => void;
  onClose: () => void;
}) {
  const [filter, setFilter] = useState("");
  const input = useRef<HTMLInputElement>(null);
  const box = useRef<HTMLDivElement>(null);

  useEffect(() => {
    // `preventScroll`: the menu hangs off the toolbar's right edge, and a plain focus()
    // would scroll the whole window sideways to reveal it.
    input.current?.focus({ preventScroll: true });
    function onDown(event: MouseEvent) {
      if (box.current && !box.current.contains(event.target as Node)) onClose();
    }
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") onClose();
    }
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [onClose]);

  const endpoints = useMemo(() => {
    const needle = filter.trim().toLowerCase();
    const all = (scan?.endpoints ?? []).filter((e) => !e.unresolved);
    if (!needle) return all;
    return all.filter(
      (e) =>
        e.display.toLowerCase().includes(needle) ||
        (e.summary ?? "").toLowerCase().includes(needle) ||
        (e.group ?? "").toLowerCase().includes(needle),
    );
  }, [scan, filter]);

  return (
    <div
      ref={box}
      className="absolute right-0 top-full z-20 mt-1 flex max-h-[420px] w-96 max-w-[calc(100vw-2rem)] flex-col overflow-hidden rounded-md border border-edge bg-panel shadow-xl"
    >
      <input
        ref={input}
        value={filter}
        onChange={(e) => setFilter(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && endpoints[0]) onPick({ kind: "endpoint", endpoint: endpoints[0] });
        }}
        placeholder={scan ? "Filter the project's endpoints…" : "No scan yet"}
        spellCheck={false}
        className="m-2 rounded border border-edge bg-ground px-2 py-1.5 outline-none placeholder:text-muted/60 focus:border-accent"
      />

      <div className="flex flex-wrap gap-1 border-b border-edge px-2 pb-2">
        <button onClick={() => onPick({ kind: "blank" })} className={chip}>
          Blank request
        </button>
        <button onClick={() => onPick({ kind: "condition" })} className={chip}>
          <span className="font-mono text-method-patch">IF</span> Condition
        </button>
        <button
          onClick={() => onPick({ kind: "variables" })}
          className={chip}
          title="Declare the flow's own variables — change a value here, not in every step"
        >
          <span className="font-mono text-accent">{"{{ }}"}</span> Variables
        </button>
        <button
          onClick={() => onPick({ kind: "display" })}
          className={chip}
          title="Show a value or sentence built from variables after the run"
        >
          <span className="font-mono text-method-put">▤</span> Display
        </button>
      </div>

      <div className="min-h-0 flex-1 overflow-auto p-1">
        {!scan && (
          <p className="px-2 py-3 text-muted">
            Scan the project from the API panel and its endpoints will be listed here.
          </p>
        )}
        {scan && endpoints.length === 0 && (
          <p className="px-2 py-3 text-muted">Nothing matches.</p>
        )}
        {endpoints.map((endpoint) => (
          <button
            key={endpoint.id}
            onClick={() => onPick({ kind: "endpoint", endpoint })}
            title={endpoint.summary ?? endpoint.display}
            className="flex w-full items-center gap-2 rounded px-2 py-1 text-left hover:bg-raised"
          >
            <MethodBadge method={endpoint.method} className="w-12 shrink-0 text-right" />
            <span className="min-w-0 flex-1 truncate font-mono">{endpoint.path}</span>
            {endpoint.group && <span className="shrink-0 text-[10px] text-muted">{endpoint.group}</span>}
          </button>
        ))}
      </div>
    </div>
  );
}

const chip = "rounded bg-raised px-2.5 py-1 transition hover:brightness-125";
