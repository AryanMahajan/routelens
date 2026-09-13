import type { HttpMethod } from "../types";
import { MethodBadge } from "./MethodBadge";

export interface TabSummary {
  id: string;
  label: string;
  /** A request tab shows its method; a flow tab shows a flow mark instead. */
  method: HttpMethod | null;
  dirty: boolean;
}

/**
 * One tab per open request or flow, the way every API client and browser does it.
 *
 * A dot marks unsaved edits. Middle-click closes; so does the ✕ that appears on hover.
 */
export function TabStrip({
  tabs,
  activeId,
  onActivate,
  onClose,
  onNew,
}: {
  tabs: TabSummary[];
  activeId: string | null;
  onActivate: (id: string) => void;
  onClose: (id: string) => void;
  onNew: () => void;
}) {
  return (
    <div className="flex shrink-0 items-stretch overflow-x-auto border-b border-edge bg-panel">
      {tabs.map((tab) => {
        const active = tab.id === activeId;
        const label = tab.label;
        return (
          <div
            key={tab.id}
            role="tab"
            aria-selected={active}
            onClick={() => onActivate(tab.id)}
            onAuxClick={(e) => {
              if (e.button === 1) onClose(tab.id);
            }}
            className={`group relative flex max-w-56 shrink-0 cursor-default items-center gap-2 border-r
              border-edge px-3 py-2 ${active ? "bg-ground text-ink" : "text-muted hover:text-ink"}`}
          >
            {tab.method !== null ? (
              <MethodBadge method={tab.method} className="shrink-0" />
            ) : (
              <span
                className="shrink-0 font-mono text-[10px] font-bold tracking-wider text-accent"
                title="Flow"
              >
                FLOW
              </span>
            )}
            <span className="min-w-0 flex-1 truncate" title={label}>
              {label}
            </span>
            <button
              onClick={(e) => {
                e.stopPropagation();
                onClose(tab.id);
              }}
              title={tab.dirty ? "Unsaved changes — close" : "Close"}
              className={`grid size-4 shrink-0 place-items-center rounded text-[10px] hover:bg-raised
                ${tab.dirty ? "" : "opacity-0 group-hover:opacity-100"}`}
            >
              {tab.dirty && <span className="size-2 rounded-full bg-method-post group-hover:hidden" />}
              <span className={tab.dirty ? "hidden group-hover:inline" : ""}>✕</span>
            </button>
            {active && <span className="absolute inset-x-0 -bottom-px h-0.5 bg-accent" />}
          </div>
        );
      })}
      <button
        onClick={onNew}
        title="New request (Ctrl+T)"
        className="shrink-0 px-3 text-muted transition hover:text-ink"
      >
        +
      </button>
    </div>
  );
}
