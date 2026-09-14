import type { HTMLAttributes, ReactNode } from "react";

/**
 * The pieces every tree in the sidebar is drawn from — the API explorer's groups and the
 * collections' folders — so they look like one thing: a chevron that turns, a folder that
 * opens, rows that indent by depth and light up under the pointer.
 */

/** How far each level steps in. */
export const INDENT = 14;

export function Chevron({ open, className = "" }: { open: boolean; className?: string }) {
  return (
    <svg
      viewBox="0 0 16 16"
      width="14"
      height="14"
      aria-hidden
      className={`shrink-0 text-muted transition-transform duration-150 ${open ? "rotate-90" : ""} ${className}`}
    >
      <path d="M6 4l4 4-4 4" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

export function FolderIcon({ open, className = "" }: { open: boolean; className?: string }) {
  return (
    <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden className={`shrink-0 text-muted ${className}`}>
      {open ? (
        <path
          d="M1.5 4.5A1.5 1.5 0 0 1 3 3h3.2l1.4 1.5H13a1.5 1.5 0 0 1 1.5 1.5v.5H4.4a1.5 1.5 0 0 0-1.42 1.02L1.5 12V4.5zm1.2 8.5 1.4-4.4A.5.5 0 0 1 4.6 8.3H15l-1.5 4.3a.5.5 0 0 1-.47.4H2.7z"
          fill="currentColor"
          fillRule="evenodd"
        />
      ) : (
        <path
          d="M1.5 4.5A1.5 1.5 0 0 1 3 3h3.2l1.4 1.5H13a1.5 1.5 0 0 1 1.5 1.5v6A1.5 1.5 0 0 1 13 13.5H3A1.5 1.5 0 0 1 1.5 12v-7.5z"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.2"
        />
      )}
    </svg>
  );
}

/**
 * One line of a tree. `depth` sets the indent; the row is a flex line so callers put the
 * chevron, icon, label and trailing marks in directly.
 */
export function TreeRow({
  depth = 0,
  active = false,
  className = "",
  style,
  children,
  ...rest
}: HTMLAttributes<HTMLDivElement> & { depth?: number; active?: boolean; children: ReactNode }) {
  return (
    <div
      {...rest}
      style={{ paddingLeft: 6 + depth * INDENT, ...style }}
      className={`group flex h-7 items-center gap-1.5 rounded pr-1 text-[12.5px] transition ${
        active ? "bg-accent/15 text-ink" : "hover:bg-raised"
      } ${className}`}
    >
      {children}
    </div>
  );
}

/** The heading row for a group or folder: chevron, folder, name, count. */
export function FolderRow({
  depth = 0,
  open,
  name,
  count,
  onToggle,
  trailing,
  ...rest
}: Omit<HTMLAttributes<HTMLDivElement>, "children"> & {
  depth?: number;
  open: boolean;
  name: string;
  count?: number;
  onToggle: () => void;
  /** Something after the count — a menu button, say. */
  trailing?: ReactNode;
}) {
  return (
    <TreeRow depth={depth} {...rest}>
      <button
        onClick={onToggle}
        className="flex h-full min-w-0 flex-1 items-center gap-1.5 text-left"
        aria-expanded={open}
      >
        <Chevron open={open} />
        <FolderIcon open={open} />
        <span className="min-w-0 flex-1 truncate">{name}</span>
        {count !== undefined && (
          <span className="shrink-0 pr-1 text-[11px] tabular-nums text-muted">{count}</span>
        )}
      </button>
      {trailing}
    </TreeRow>
  );
}

/** Group names arrive as tags and router names — `admin`, `users` — and read better capitalised. */
export function displayName(group: string): string {
  return group.charAt(0).toUpperCase() + group.slice(1);
}
