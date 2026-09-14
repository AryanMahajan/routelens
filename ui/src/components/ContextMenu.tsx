import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

/**
 * A right-click menu, placed at the pointer and kept inside the window.
 *
 * Closes on any click elsewhere, Escape, a scroll or a resize — the same rules a native
 * menu follows, so it never hangs around after the thing it referred to has moved.
 */

export type ContextMenuItem =
  | { separator: true }
  | {
      label: string;
      /** Shown on the right, dimmed: the keyboard route to the same thing. */
      shortcut?: string;
      danger?: boolean;
      disabled?: boolean;
      onClick: () => void;
    };

export function ContextMenu({
  x,
  y,
  items,
  onClose,
}: {
  x: number;
  y: number;
  items: ContextMenuItem[];
  onClose: () => void;
}) {
  const menu = useRef<HTMLUListElement>(null);
  const [place, setPlace] = useState({ left: x, top: y });

  useLayoutEffect(() => {
    const box = menu.current?.getBoundingClientRect();
    const width = box?.width ?? 200;
    const height = box?.height ?? 200;
    setPlace({
      left: Math.max(4, Math.min(x, window.innerWidth - width - 4)),
      top: Math.max(4, Math.min(y, window.innerHeight - height - 4)),
    });
  }, [x, y, items.length]);

  useEffect(() => {
    const down = (event: MouseEvent) => {
      if (!menu.current?.contains(event.target as Node)) onClose();
    };
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("mousedown", down, true);
    window.addEventListener("keydown", key, true);
    window.addEventListener("scroll", onClose, true);
    window.addEventListener("resize", onClose);
    return () => {
      window.removeEventListener("mousedown", down, true);
      window.removeEventListener("keydown", key, true);
      window.removeEventListener("scroll", onClose, true);
      window.removeEventListener("resize", onClose);
    };
  }, [onClose]);

  return createPortal(
    <ul
      ref={menu}
      role="menu"
      style={{ position: "fixed", left: place.left, top: place.top }}
      className="z-50 min-w-44 rounded border border-edge bg-panel py-1 shadow-xl"
      onContextMenu={(e) => e.preventDefault()}
    >
      {items.map((item, index) =>
        "separator" in item ? (
          <li key={index} role="separator" className="my-1 border-t border-edge" />
        ) : (
          <li key={item.label} role="menuitem">
            <button
              disabled={item.disabled}
              onClick={() => {
                onClose();
                item.onClick();
              }}
              className={`flex w-full items-center gap-6 whitespace-nowrap px-3 py-1 text-left transition
                hover:bg-raised disabled:cursor-default disabled:opacity-40 disabled:hover:bg-transparent ${
                  item.danger ? "text-method-delete" : ""
                }`}
            >
              <span className="flex-1">{item.label}</span>
              {item.shortcut && (
                <span className="text-[10px] tracking-wide text-muted">{item.shortcut}</span>
              )}
            </button>
          </li>
        ),
      )}
    </ul>,
    document.body,
  );
}
