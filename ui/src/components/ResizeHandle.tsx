import { useCallback, useEffect, useState } from "react";

/**
 * A draggable edge between two panes.
 *
 * `grows` says which way the pane gets bigger: `"right"` for a pane on the left whose
 * handle sits on its right edge (the sidebar), `"left"` for a pane on the right whose
 * handle sits on its left edge (the inspector), `"up"` for a pane below the handle (the
 * response), `"down"` for one above it. `width` is the pane's size along that axis.
 * Double-click resets.
 */
export function ResizeHandle({
  width,
  min,
  max,
  grows,
  onChange,
  onReset,
}: {
  width: number;
  min: number;
  max: number;
  grows: "left" | "right" | "up" | "down";
  onChange: (width: number) => void;
  onReset?: () => void;
}) {
  const [dragging, setDragging] = useState(false);
  const vertical = grows === "up" || grows === "down";

  const onPointerDown = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      event.preventDefault();
      const start = vertical ? event.clientY : event.clientX;
      const startWidth = width;
      const sign = grows === "right" || grows === "down" ? 1 : -1;
      const target = event.currentTarget;
      target.setPointerCapture(event.pointerId);
      setDragging(true);

      const move = (e: PointerEvent) => {
        const at = vertical ? e.clientY : e.clientX;
        const next = Math.round(startWidth + sign * (at - start));
        onChange(Math.min(max, Math.max(min, next)));
      };
      const up = () => {
        setDragging(false);
        target.removeEventListener("pointermove", move);
        target.removeEventListener("pointerup", up);
        target.removeEventListener("pointercancel", up);
      };
      target.addEventListener("pointermove", move);
      target.addEventListener("pointerup", up);
      target.addEventListener("pointercancel", up);
    },
    [width, min, max, grows, vertical, onChange],
  );

  // Text would get selected all over the place while dragging otherwise.
  useEffect(() => {
    if (!dragging) return;
    const previous = document.body.style.userSelect;
    document.body.style.userSelect = "none";
    document.body.style.cursor = vertical ? "row-resize" : "col-resize";
    return () => {
      document.body.style.userSelect = previous;
      document.body.style.cursor = "";
    };
  }, [dragging, vertical]);

  if (vertical) {
    return (
      <div
        role="separator"
        aria-orientation="horizontal"
        title="Drag to resize · double-click to reset"
        onPointerDown={onPointerDown}
        onDoubleClick={onReset}
        className={`group relative z-10 h-1 shrink-0 cursor-row-resize select-none
          ${grows === "down" ? "-mb-1" : "-mt-1"}`}
      >
        <div
          className={`absolute inset-x-0 top-0 h-1 transition-colors
            ${dragging ? "bg-accent" : "bg-transparent group-hover:bg-accent/60"}`}
        />
      </div>
    );
  }

  return (
    <div
      role="separator"
      aria-orientation="vertical"
      title="Drag to resize · double-click to reset"
      onPointerDown={onPointerDown}
      onDoubleClick={onReset}
      className={`group relative z-10 w-1 shrink-0 cursor-col-resize select-none
        ${grows === "right" ? "-mr-1" : "-ml-1"}`}
    >
      <div
        className={`absolute inset-y-0 left-0 w-1 transition-colors
          ${dragging ? "bg-accent" : "bg-transparent group-hover:bg-accent/60"}`}
      />
    </div>
  );
}

/** A number remembered per machine, for pane widths. */
export function usePersistedNumber(key: string, fallback: number): [number, (n: number) => void] {
  const [value, setValue] = useState<number>(() => {
    try {
      const raw = localStorage.getItem(key);
      const n = raw === null ? NaN : Number(raw);
      return Number.isFinite(n) ? n : fallback;
    } catch {
      return fallback;
    }
  });
  const set = useCallback(
    (n: number) => {
      setValue(n);
      try {
        localStorage.setItem(key, String(n));
      } catch {
        // Blocked storage: the width just does not survive a restart.
      }
    },
    [key],
  );
  return [value, set];
}

export function usePersistedFlag(key: string, fallback: boolean): [boolean, (b: boolean) => void] {
  const [value, setValue] = useState<boolean>(() => {
    try {
      const raw = localStorage.getItem(key);
      return raw === null ? fallback : raw === "1";
    } catch {
      return fallback;
    }
  });
  const set = useCallback(
    (b: boolean) => {
      setValue(b);
      try {
        localStorage.setItem(key, b ? "1" : "0");
      } catch {
        // As above.
      }
    },
    [key],
  );
  return [value, set];
}
