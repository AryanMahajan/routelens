import {
  useEffect,
  useRef,
  useState,
  type ChangeEvent,
  type InputHTMLAttributes,
  type KeyboardEvent,
  type RefObject,
  type TextareaHTMLAttributes,
} from "react";
import { useVariableNames } from "../variables";

/**
 * `{{variable}}` completion for a text field.
 *
 * Type `{{` anywhere and the variables in scope appear; pick one with the arrow keys and
 * Enter or Tab. Everything else behaves like a plain field. `VariableInput` is the
 * single-line form for the URL bar, path parameters, headers and auth; `VariableTextarea`
 * is the same thing for a body.
 */

type Field = HTMLInputElement | HTMLTextAreaElement;

function useCompletion(
  field: RefObject<Field | null>,
  value: string,
  onChange: (value: string) => void,
) {
  const names = useVariableNames();
  const [open, setOpen] = useState(false);
  const [selected, setSelected] = useState(0);
  // Where the `{{` that opened the popup starts, so the completion replaces the right span.
  const [anchor, setAnchor] = useState<number | null>(null);
  const [partial, setPartial] = useState("");

  const matches = open
    ? names.filter((n) => n.toLowerCase().startsWith(partial.toLowerCase()))
    : [];

  useEffect(() => {
    if (selected >= matches.length) setSelected(0);
  }, [matches.length, selected]);

  function inspect(text: string, caret: number) {
    const before = text.slice(0, caret);
    const start = before.lastIndexOf("{{");
    const closed = before.lastIndexOf("}}");
    if (start === -1 || closed > start) {
      setOpen(false);
      return;
    }
    const typed = before.slice(start + 2);
    // A space means the user is writing something else, not a variable name.
    if (/\s/.test(typed)) {
      setOpen(false);
      return;
    }
    setAnchor(start);
    setPartial(typed);
    setOpen(true);
  }

  function handleChange(event: ChangeEvent<Field>) {
    onChange(event.target.value);
    inspect(event.target.value, event.target.selectionStart ?? event.target.value.length);
  }

  function complete(name: string) {
    if (anchor === null) return;
    const caret = field.current?.selectionStart ?? value.length;
    const after = value.slice(caret);
    // Do not double the closing braces when they are already typed.
    const tail = after.startsWith("}}") ? after.slice(2) : after;
    const next = `${value.slice(0, anchor)}{{${name}}}${tail}`;
    onChange(next);
    setOpen(false);
    const position = anchor + name.length + 4;
    requestAnimationFrame(() => field.current?.setSelectionRange(position, position));
  }

  /** Returns true when the key was consumed by the popup. */
  function handleKeyDown(event: KeyboardEvent<Field>): boolean {
    if (open && matches.length > 0) {
      if (event.key === "ArrowDown") {
        event.preventDefault();
        setSelected((i) => (i + 1) % matches.length);
        return true;
      }
      if (event.key === "ArrowUp") {
        event.preventDefault();
        setSelected((i) => (i - 1 + matches.length) % matches.length);
        return true;
      }
      if (event.key === "Enter" || event.key === "Tab") {
        event.preventDefault();
        complete(matches[selected] ?? matches[0]!);
        return true;
      }
    }
    if (event.key === "Escape" && open) {
      setOpen(false);
      return true;
    }
    return false;
  }

  const popup = open ? (
    <ul
      className="absolute left-0 top-full z-30 mt-1 max-h-56 min-w-48 overflow-auto rounded
        border border-edge bg-panel py-1 shadow-lg"
      role="listbox"
    >
      {matches.length === 0 && (
        <li className="px-3 py-1.5 text-muted italic">
          {names.length === 0 ? "No variables defined yet" : "No match"}
        </li>
      )}
      {matches.map((name, index) => (
        <li
          key={name}
          role="option"
          aria-selected={index === selected}
          onMouseDown={(e) => {
            e.preventDefault();
            complete(name);
          }}
          onMouseEnter={() => setSelected(index)}
          className={`cursor-pointer px-3 py-1 font-mono ${
            index === selected ? "bg-raised text-ink" : "text-muted"
          }`}
        >
          {name}
        </li>
      ))}
    </ul>
  ) : null;

  return {
    popup,
    handleChange,
    handleKeyDown,
    inspect,
    close: () => setTimeout(() => setOpen(false), 120),
  };
}

export function VariableInput({
  value,
  onChange,
  onKeyDown,
  className = "",
  ...rest
}: Omit<InputHTMLAttributes<HTMLInputElement>, "value" | "onChange"> & {
  value: string;
  onChange: (value: string) => void;
}) {
  const input = useRef<HTMLInputElement>(null);
  const c = useCompletion(input, value, onChange);

  return (
    <div className="relative min-w-0 flex-1">
      <input
        ref={input}
        value={value}
        onChange={c.handleChange}
        onKeyDown={(e) => {
          if (!c.handleKeyDown(e)) onKeyDown?.(e);
        }}
        onBlur={c.close}
        onClick={(e) => c.inspect(value, e.currentTarget.selectionStart ?? value.length)}
        spellCheck={false}
        autoComplete="off"
        className={`w-full ${className}`}
        {...rest}
      />
      {c.popup}
    </div>
  );
}

export function VariableTextarea({
  value,
  onChange,
  onKeyDown,
  className = "",
  ...rest
}: Omit<TextareaHTMLAttributes<HTMLTextAreaElement>, "value" | "onChange"> & {
  value: string;
  onChange: (value: string) => void;
}) {
  const area = useRef<HTMLTextAreaElement>(null);
  const c = useCompletion(area, value, onChange);

  return (
    <div className="relative flex min-h-0 min-w-0 flex-1 flex-col">
      <textarea
        ref={area}
        value={value}
        onChange={c.handleChange}
        onKeyDown={(e) => {
          if (!c.handleKeyDown(e)) onKeyDown?.(e);
        }}
        onBlur={c.close}
        onClick={(e) => c.inspect(value, e.currentTarget.selectionStart ?? value.length)}
        spellCheck={false}
        className={`min-h-0 w-full flex-1 ${className}`}
        {...rest}
      />
      {c.popup}
    </div>
  );
}
