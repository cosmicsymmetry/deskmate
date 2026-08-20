import type { ProviderSnapshot } from "./types";

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

/**
 * A provider is only worth a word when it is *not* fine. A healthy feed refreshing on
 * its own schedule is not news, which is why nothing in the window reports one; this
 * returns null for that case and a sentence for the two cases that cost the user a
 * correct-looking card.
 */
export function providerTrouble(provider: ProviderSnapshot | null | undefined): string | null {
  if (!provider) {
    return null;
  }
  const state = provider.state;
  if (state.kind !== "stale" && state.kind !== "error") {
    return null;
  }
  const age =
    provider.last_success_unix_ms === null ? null : formatProviderAge(provider.age_seconds);
  return age
    ? `${state.message} · Showing the last good data, ${age.toLowerCase()}`
    : state.message;
}
