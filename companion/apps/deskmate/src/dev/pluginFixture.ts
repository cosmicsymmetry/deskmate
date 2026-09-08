/**
 * Dev-only plugin fixtures. Never ships: only `VITE_DESKMATE_MOCK=1` aliases the
 * Tauri bridge at `src/dev/`, so a production build resolves the real bridge and
 * tree-shakes this away.
 *
 * The catalog mirrors the four curated plugins in `companion/plugins/`, with the
 * display names, descriptions and cadences their v2 manifests declare and the real
 * byte lengths of their committed assets. `node_count` is the registry's own sum
 * (`[[nodes]]` plus every `[[repeats]] nodes`), and digests are plainly fake
 * constants: nothing in the window renders a digest, and inventing a plausible
 * SHA-256 would be a lie a reader might trust.
 */
import type {
  AppConfig,
  CardDataSnapshot,
  CardSettings,
  PluginCatalog,
  ProviderSnapshot,
  ServerCardState,
} from "../lib/types";
import { mockConfig } from "./fixture";

export const MOCK_PLUGIN_CATALOG: PluginCatalog = {
  plugins: [
    {
      id: "aqi",
      name: "aqi",
      version: "1.0.0",
      node_count: 7,
      assets: [
        { file: "icons.ttf", kind: "icon-font", byte_length: 4320, digest: "dev-digest-aqi" },
      ],
      display_name: "Air quality",
      description: "EPA index for a location",
      manifest_version: 2,
      template: "display-list",
      refresh_minutes: 15,
    },
    {
      id: "agenda",
      name: "agenda",
      version: "1.0.0",
      node_count: 7,
      assets: [
        { file: "badge.rgb565", kind: "image", byte_length: 812, digest: "dev-digest-agenda" },
      ],
      display_name: "Agenda",
      description: "Your next few events",
      manifest_version: 2,
      template: "display-list",
      refresh_minutes: 10,
    },
    {
      id: "claude-limits",
      name: "claude-limits",
      version: "1.0.0",
      node_count: 12,
      assets: [],
      display_name: "Claude usage",
      description: "Session and weekly subscription limits",
      manifest_version: 2,
      template: "display-list",
      refresh_minutes: 10,
    },
    {
      id: "svg-aqi",
      name: "svg-aqi",
      version: "1.0.0",
      node_count: 0,
      assets: [],
      display_name: "Air quality, drawn",
      description: "The same index, drawn as SVG",
      manifest_version: 2,
      template: "svg",
      refresh_minutes: 15,
    },
  ],
  load_failures: [],
};

function pluginCard(id: string, title: string, pluginId: string, minutes: number): CardSettings {
  return {
    kind: "plugin",
    id,
    title,
    plugin_id: pluginId,
    tap_action: { kind: "none" },
    refresh: { kind: "interval", minutes },
    alert: { kind: "none" },
  };
}

/**
 * Two plugin cards in the loop — one display-list, one SVG — and two outside it, so
 * one scenario reaches every outcome the preview route can produce: a frame, a
 * stale frame, "waiting for the first refresh", and a plugin the registry lost.
 */
export function mockPluginConfig(): AppConfig {
  return {
    ...mockConfig(),
    cards: [
      pluginCard("air", "Office air", "aqi", 15),
      pluginCard("air-svg", "", "svg-aqi", 15),
      pluginCard("usage", "", "claude-limits", 10),
      pluginCard("retired", "Old panel", "com.example.retired", 30),
    ],
    playlists: [
      {
        id: "day",
        name: "Workday",
        advance: { kind: "timed", default_dwell_seconds: 20 },
        entries: [
          { card_id: "air", dwell_seconds: 30 },
          { card_id: "air-svg", dwell_seconds: null },
        ],
      },
    ],
    active_playlist_id: "day",
  };
}

export function mockPluginCardState(): ServerCardState[] {
  return [
    { card_id: "air", provider: { kind: "fresh" }, hero: "42", errors: [] },
    {
      card_id: "air-svg",
      provider: { kind: "stale", message: "Feed timed out after 10s" },
      hero: "51",
      errors: [],
    },
    { card_id: "usage", provider: { kind: "idle" }, hero: null, errors: [] },
    {
      card_id: "retired",
      provider: { kind: "error", message: "Plugin “com.example.retired” is not loaded" },
      hero: null,
      errors: [],
    },
  ];
}

/** What the Mac's projection puts into `providers` for those same cards. */
export function mockPluginProviders(): ProviderSnapshot[] {
  return mockPluginCardState().map((state) => ({
    widget_id: state.card_id,
    state: state.provider,
    last_success_unix_ms: state.hero === null ? null : 1,
    age_seconds: state.hero === null ? null : 300,
  }));
}

/** And into `card_data`, as the `hero` field the tile reads. */
export function mockPluginCardData(): CardDataSnapshot[] {
  return mockPluginCardState()
    .filter((state) => state.hero !== null)
    .map((state) => ({
      card_id: state.card_id,
      fields: [{ key: "hero", value: { kind: "text" as const, value: state.hero ?? "" } }],
    }));
}
