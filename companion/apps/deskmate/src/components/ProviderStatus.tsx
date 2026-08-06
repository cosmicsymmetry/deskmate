import type { CardSettings, ProviderSnapshot } from "../lib/types";
import { cardName } from "../lib/configDraft";

interface ProviderStatusProps {
  providers: ProviderSnapshot[];
  cards: CardSettings[];
  refreshingId: string | null;
  onRefresh: (widgetId: string) => void;
}

export function formatProviderAge(ageSeconds: number | null): string {
  if (ageSeconds === null) {
    return "No successful refresh yet";
  }
  if (ageSeconds < 60) {
    return "Updated just now";
  }
  if (ageSeconds < 3600) {
    return `Updated ${Math.floor(ageSeconds / 60)} min ago`;
  }
  return `Updated ${Math.floor(ageSeconds / 3600)} hr ago`;
}

export function ProviderStatus({ providers, cards, refreshingId, onRefresh }: ProviderStatusProps) {
  if (providers.length === 0) {
    return null;
  }
  return (
    <section className="provider-panel" aria-labelledby="provider-heading">
      <div className="panel-heading panel-heading--compact">
        <div>
          <p className="step-label">Provider health</p>
          <h2 id="provider-heading">Data sources</h2>
        </div>
      </div>
      <div className="provider-list">
        {providers.map((provider) => {
          const widget = cards.find((candidate) => candidate.id === provider.widget_id);
          const state = provider.state;
          const problematic = state.kind === "stale" || state.kind === "error";
          return (
            <article
              key={provider.widget_id}
              className={`provider-row provider-row--${state.kind}`}
            >
              <span className="provider-state" aria-hidden="true" />
              <span>
                <strong>{widget ? cardName(widget) : "Unknown card"}</strong>
                <small>
                  {state.kind === "refreshing"
                    ? "Refreshing…"
                    : problematic
                      ? state.message
                      : formatProviderAge(provider.age_seconds)}
                </small>
                {problematic && provider.last_success_unix_ms !== null && (
                  <em>
                    Showing the last successful data · {formatProviderAge(provider.age_seconds)}
                  </em>
                )}
              </span>
              <button
                className="button button--quiet"
                type="button"
                disabled={refreshingId === provider.widget_id || state.kind === "refreshing"}
                onClick={() => onRefresh(provider.widget_id)}
              >
                {refreshingId === provider.widget_id || state.kind === "refreshing"
                  ? "Refreshing…"
                  : "Refresh now"}
              </button>
            </article>
          );
        })}
      </div>
    </section>
  );
}
