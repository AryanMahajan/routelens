import type { HttpMethod } from "../types";

/**
 * Method chips are how the endpoint tree stays scannable, so the colours have to be
 * distinguishable at a glance and consistent everywhere a method appears.
 */
const COLOURS: Record<string, string> = {
  GET: "text-method-get",
  POST: "text-method-post",
  PUT: "text-method-put",
  PATCH: "text-method-patch",
  DELETE: "text-method-delete",
  HEAD: "text-muted",
  OPTIONS: "text-muted",
  TRACE: "text-muted",
};

export function methodColour(method: HttpMethod): string {
  return COLOURS[method] ?? "text-accent";
}

export function MethodBadge({
  method,
  className = "",
}: {
  method: HttpMethod;
  className?: string;
}) {
  return (
    <span
      className={`font-mono text-[10px] font-bold tracking-wider tabular-nums ${methodColour(
        method,
      )} ${className}`}
    >
      {method}
    </span>
  );
}
