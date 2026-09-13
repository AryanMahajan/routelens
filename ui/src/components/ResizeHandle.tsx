import { useCallback, useEffect, useState } from "react";

/**
 * A draggable edge between two panes.
 *
 * `grows` says which way the pane gets bigger: `"right"` for a pane on the left whose
 * handle sits on its right edge (the sidebar), `"left"` for a pane on the right whose
 * handle sits on its left edge (the inspector). Double-click resets.
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
  grows: "left" | "right";
  onChange: (width: number) => void;
  onReset?: () => void;
}) {
  const [dragging, setDragging] = useState(false);

  const onPointerDown = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      event.preventDefault();
      const startX = event.clientX;
      const startWidth = width;
      const sign = grows === "right" ? 1 : -1;
      const target = event.currentTarget;
      target.setPointerCapture(event.pointerId);
      setDragging(true);

      const move = (e: PointerEvent) => {
        const next = Math.round(startWidth + sign * (e.clientX - startX));
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
    [width, min, max, grows, onChange],
  );

  // Text would get selected all over the place while dragging otherwise.
  useEffect(() => {
    if (!dragging) return;
    const previous = document.body.style.userSelect;
    document.body.style.userSelect = "none";
    document.body.style.cursor = "col-resize";
    return () => {
      document.body.style.userSelect = previous;
      document.body.style.cursor = "";
    };
  }, [dragging]);

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
