import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import { createPortal } from "react-dom";
import type { PathEntry } from "../../responseShape";

/**
 * A text field for a body path or a header name, with the real response's paths one
 * click away. Typing filters the list; picking fills the field and tells the caller what
 * was found there, so an assertion can start from the value as it is.
 *
 * With no response to draw on the field is an ordinary input and the button says why.
 */
export function PathPicker({
  value,
  onChange,
  onPick,
  options,
  placeholder,
  emptyHint,
  className = "",
}: {
  value: string;
  onChange: (value: string) => void;
  /** The chosen entry, after `onChange` has been called with its path. */
  onPick?: (entry: PathEntry) => void;
  options: PathEntry[] | null;
  placeholder: string;
  /** Shown in the list when there is nothing to pick from. */
  emptyHint: string;
  className?: string;
}) {
  const input = useRef<HTMLInputElement>(null);
  const [open, setOpen] = useState(false);
  const [selected, setSelected] = useState(0);
  const [place, setPlace] = useState<CSSProperties>({});

  const needle = value.trim().toLowerCase();
  const matches = (options ?? []).filter((o) => !needle || o.path.toLowerCase().includes(needle));

  useEffect(() => {
    if (selected >= matches.length) setSelected(0);
  }, [matches.length, selected]);

  useLayoutEffect(() => {
    if (!open || !input.current) return;
    const rect = input.current.getBoundingClientRect();
    const height = 260;
    const below = rect.bottom + 4 + height <= window.innerHeight;
    setPlace({
      position: "fixed",
      left: Math.min(rect.left, Math.max(0, window.innerWidth - 360)),
      top: below ? rect.bottom + 4 : undefined,
      bottom: below ? undefined : window.innerHeight - rect.top + 4,
      width: Math.min(Math.max(320, rect.width), window.innerWidth - 16),
    });
    const close = (event: Event) => {
      if (event.target !== input.current) setOpen(false);
    };
    window.addEventListener("scroll", close, true);
    window.addEventListener("resize", close);
    return () => {
      window.removeEventListener("scroll", close, true);
      window.removeEventListener("resize", close);
    };
  }, [open, matches.length]);

  function pick(entry: PathEntry) {
    onChange(entry.path);
    onPick?.(entry);
    setOpen(false);
  }

  return (
    <div className="relative flex min-w-0 flex-1">
      <input
        ref={input}
        value={value}
        onChange={(e) => {
          onChange(e.target.value);
          if (options) setOpen(true);
        }}
        onFocus={() => options && setOpen(true)}
        onBlur={() => setTimeout(() => setOpen(false), 120)}
        onKeyDown={(e) => {
          if (!open) return;
          if (e.key === "ArrowDown") {
            e.preventDefault();
            setSelected((i) => (i + 1) % Math.max(matches.length, 1));
          } else if (e.key === "ArrowUp") {
            e.preventDefault();
            setSelected((i) => (i - 1 + matches.length) % Math.max(matches.length, 1));
          } else if ((e.key === "Enter" || e.key === "Tab") && matches[selected]) {
            e.preventDefault();
            pick(matches[selected]!);
          } else if (e.key === "Escape") {
            setOpen(false);
          }
        }}
        placeholder={placeholder}
        spellCheck={false}
        autoComplete="off"
        className={`w-full rounded-r-none ${className}`}
      />
      <button
        type="button"
        onMouseDown={(e) => {
          e.preventDefault();
          // Focusing opens the list; a plain set afterwards decides, so a click on an
          // open list closes it rather than the two cancelling out.
          const wasOpen = open;
          input.current?.focus();
          setOpen(!wasOpen && options !== null);
        }}
        title={options ? "Pick from the response" : emptyHint}
        className={`shrink-0 rounded-r border border-l-0 border-edge px-1.5 text-[11px] transition ${
          options ? "bg-raised text-ink hover:brightness-125" : "bg-panel text-muted"
        }`}
        aria-label="Pick from the response"
      >
        ⌄
      </button>
      {open &&
        createPortal(
          <ul
            style={place}
            role="listbox"
            className="z-50 max-h-64 overflow-auto rounded border border-edge bg-panel py-1 shadow-lg"
          >
            {!options && <li className="px-3 py-2 text-muted italic">{emptyHint}</li>}
            {options && matches.length === 0 && (
              <li className="px-3 py-2 text-muted italic">
                {options.length === 0 ? "The body is not JSON." : "No path matches."}
              </li>
            )}
            {matches.map((entry, index) => (
              <li
                key={entry.path}
                role="option"
                aria-selected={index === selected}
                onMouseDown={(e) => {
                  e.preventDefault();
                  pick(entry);
                }}
                onMouseEnter={() => setSelected(index)}
                className={`flex cursor-pointer items-baseline gap-3 px-3 py-1 font-mono ${
                  index === selected ? "bg-raised text-ink" : ""
                }`}
              >
                <span className={`min-w-0 truncate ${entry.scalar ? "text-accent" : "text-muted"}`}>
                  {entry.path}
                </span>
                <span className="ml-auto shrink-0 max-w-[45%] truncate text-[11px] text-muted">
                  {entry.preview}
                </span>
              </li>
            ))}
          </ul>,
          document.body,
        )}
    </div>
  );
}
