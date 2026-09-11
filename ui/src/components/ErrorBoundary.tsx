import { Component, type ErrorInfo, type ReactNode } from "react";

/**
 * A render error must never blank the window. This shows what broke and offers a reload,
 * which is far more useful to someone testing the app than an empty pane.
 */
export class ErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state = { error: null as Error | null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("RouteLens UI crashed", error, info.componentStack);
  }

  render() {
    if (!this.state.error) return this.props.children;
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 p-8 text-center">
        <p className="font-semibold">Something in the UI broke.</p>
        <pre className="max-w-2xl overflow-auto rounded border border-edge bg-panel p-3 text-left font-mono text-xs text-method-delete">
          {this.state.error.message}
        </pre>
        <p className="text-muted">
          This is a bug worth reporting — the message above is what matters.
        </p>
        <button
          onClick={() => window.location.reload()}
          className="rounded bg-raised px-3 py-1.5 transition hover:brightness-125"
        >
          Reload
        </button>
      </div>
    );
  }
}
