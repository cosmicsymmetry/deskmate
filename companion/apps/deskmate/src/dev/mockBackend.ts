/**
 * Dev-only in-browser stand-in for the Rust runtime.
 *
 * Enabled by `VITE_DESKMATE_MOCK=1`, which makes `vite.config.ts` alias
 * `@tauri-apps/api/core` and `@tauri-apps/api/event` at the two modules beside this
 * one. No application code imports anything from `src/dev/`, so a production build
 * resolves the real bridge and never reaches this file.
 *
 * Scenarios let a whole device state be selected from the URL — `?scenario=offline`,
 * `?scenario=local`, `?scenario=invalid`, `?scenario=firstrun`, `?scenario=empty` —
 * so every state the UI must handle can be opened, reviewed and screenshotted without
 * hardware. `?scenario=list` prints the set to the console.
 */
import { mockCardData, mockConfig, mockNetworkSettings, mockSnapshot } from "./fixture";
import { renderMockFrame } from "./mockPreview";
import type {
  AppConfig,
  AppSnapshot,
  DraftValidation,
  NetworkSettings,
  ValidationIssue,
} from "../lib/types";

export const SCENARIOS = [
  "default",
  "offline",
  "standalone",
  "local",
  "unowned",
  "invalid",
  "firstrun",
  "empty",
  "carderror",
] as const;
export type Scenario = (typeof SCENARIOS)[number];

function currentScenario(): Scenario {
  const raw = new URLSearchParams(window.location.search).get("scenario");
  return (SCENARIOS as readonly string[]).includes(raw ?? "") ? (raw as Scenario) : "default";
}

const scenario = currentScenario();

let config: AppConfig = mockConfig();
let network: NetworkSettings = mockNetworkSettings();
let snapshot: AppSnapshot = mockSnapshot(config);
let autostart = { enabled: true, preference_enabled: true };
const listeners = new Set<(next: AppSnapshot) => void>();

function applyScenario() {
  switch (scenario) {
    case "offline":
      snapshot.device.connection = { kind: "disconnected", reason: "No USB device found" };
      snapshot.device.port_name = null;
      snapshot.device.wifi_state = "down";
      snapshot.device.ip = null;
      snapshot.device.last_network_error = "WebSocket closed: 1006";
      break;
    case "standalone":
      snapshot.device.connection = { kind: "standalone" };
      break;
    case "local":
      snapshot.device.tier = "local";
      network = { server_url: "", device_id: "", tier: "local" };
      snapshot.device.wifi_state = null;
      snapshot.device.ip = null;
      break;
    case "unowned":
      snapshot.device.tier = null;
      snapshot.device.connection = { kind: "connecting" };
      network = { server_url: "", device_id: "", tier: null };
      break;
    case "invalid":
      if (config.cards[2]?.kind === "weather") config.cards[2].location = "";
      config.preferences.timezone = "Mars/Olympus";
      snapshot.persistence = {
        kind: "validation-failed",
        message: "The saved settings file did not pass validation",
        issues: [
          { path: "preferences.timezone", code: "invalid-timezone", message: "Unknown timezone." },
        ],
      };
      break;
    case "firstrun":
      config = {
        ...mockConfig(),
        cards: [mockConfig().cards[0]],
        playlists: [{ id: "day", name: "Workday", advance: { kind: "manual" }, entries: [] }],
      };
      snapshot = mockSnapshot(config);
      snapshot.has_saved_config = false;
      snapshot.providers = [];
      snapshot.pomodoros = [];
      snapshot.card_data = [];
      break;
    case "empty":
      config = {
        ...mockConfig(),
        cards: [],
        playlists: [{ id: "day", name: "Workday", advance: { kind: "manual" }, entries: [] }],
      };
      snapshot = mockSnapshot(config);
      snapshot.providers = [];
      snapshot.pomodoros = [];
      snapshot.card_data = [];
      break;
    case "carderror":
      snapshot.card_errors = [
        {
          kind: "scene-refused",
          card_id: "json-feed",
          message: "the display could not render this card's complete scene",
        },
      ];
      break;
    default:
      break;
  }
  snapshot.config = config;
}
applyScenario();

function publish() {
  snapshot = { ...snapshot, config };
  for (const listener of listeners) listener(snapshot);
}

// A deliberately partial re-implementation of the backend's rules: enough that every
// issue-rendering path in the UI can be exercised, never a second source of truth.
function validate(draft: AppConfig): DraftValidation {
  const issues: ValidationIssue[] = [];
  const push = (path: string, code: ValidationIssue["code"], message: string) =>
    issues.push({ path, code, message });

  try {
    new Intl.DateTimeFormat("en", { timeZone: draft.preferences.timezone });
  } catch {
    push("preferences.timezone", "invalid-timezone", "That is not a timezone name.");
  }

  draft.cards.forEach((card, index) => {
    const at = `cards[${index}]`;
    switch (card.kind) {
      case "pomodoro":
        if (card.duration_seconds < 60 || card.duration_seconds > 86_400) {
          push(`${at}.duration_seconds`, "out-of-range", "Use between 1 and 1440 minutes.");
        }
        break;
      case "weather":
        if (!card.location.trim()) push(`${at}.location`, "empty", "Enter a location.");
        break;
      case "calendar":
        if (!card.source.value.trim()) {
          push(`${at}.source`, "invalid-source", "Choose a file or enter a web address.");
        }
        break;
      case "rss":
        if (!card.url.trim()) push(`${at}.url`, "empty", "Enter a feed address.");
        if (card.max_items < 1 || card.max_items > 5) {
          push(`${at}.max_items`, "out-of-range", "Show between 1 and 5 headlines.");
        }
        break;
      case "json-feed":
        if (!card.url.trim()) push(`${at}.url`, "empty", "Enter a feed address.");
        card.mappings.forEach((mapping, m) => {
          if (!mapping.field.trim()) push(`${at}.mappings[${m}].field`, "empty", "Name the field.");
          if (!mapping.path.trim())
            push(`${at}.mappings[${m}].path`, "empty", "Enter a JSON path.");
        });
        break;
      default:
        break;
    }
    if (card.alert.kind !== "none" && card.alert.hold.kind === "seconds") {
      const held = card.alert.hold.value;
      if (held < 5 || held > 600) {
        push(`${at}.alert.hold.value`, "out-of-range", "Hold for between 5 and 600 seconds.");
      }
    }
    if (card.alert.kind === "before-event") {
      if (card.alert.lead_minutes < 1 || card.alert.lead_minutes > 60) {
        push(`${at}.alert.lead_minutes`, "out-of-range", "Lead by 1 to 60 minutes.");
      }
    }
  });

  draft.playlists.forEach((playlist, index) => {
    const at = `playlists[${index}]`;
    if (!playlist.name.trim()) push(`${at}.name`, "empty", "Name this playlist.");
    if (playlist.advance.kind === "timed") {
      const dwell = playlist.advance.default_dwell_seconds;
      if (dwell < 5 || dwell > 3600) {
        push(`${at}.advance.default_dwell_seconds`, "out-of-range", "Use 5 to 3600 seconds.");
      }
    }
    playlist.entries.forEach((entry, e) => {
      if (!draft.cards.some((card) => card.id === entry.card_id)) {
        push(`${at}.entries[${e}]`, "missing-reference", "This card is no longer in the library.");
      }
      if (entry.dwell_seconds !== null && (entry.dwell_seconds < 5 || entry.dwell_seconds > 3600)) {
        push(`${at}.entries[${e}].dwell_seconds`, "out-of-range", "Use 5 to 3600 seconds.");
      }
    });
  });

  if (!draft.playlists.some((playlist) => playlist.id === draft.active_playlist_id)) {
    push("active_playlist_id", "missing-reference", "No playlist is active.");
  }

  return { valid: issues.length === 0, issues };
}

// A running pomodoro ticks so the preview and the timer complication move, which is
// the only way to review motion and tabular-numeral behaviour without hardware.
window.setInterval(() => {
  const timer = snapshot.pomodoros[0];
  if (timer && timer.state === "running" && timer.remaining_seconds > 0) {
    timer.remaining_seconds -= 1;
    snapshot.card_data = mockCardData();
    publish();
  }
}, 1000);

/**
 * The mock's argument bag. Every command that reads arguments is always called with
 * them by `src/lib/tauri.ts`, so a missing bag is a harness bug — surfaced as the
 * same typed IPC error shape the real backend would return, not a TypeError.
 */
function requireArgs(args: Record<string, unknown> | undefined): Record<string, unknown> {
  if (!args) {
    throw { category: "invalid-payload", message: "mock backend received no arguments" };
  }
  return args;
}

const delay = <T>(value: T, ms = 90): Promise<T> =>
  new Promise((resolve) => window.setTimeout(() => resolve(value), ms));

export async function mockInvoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  switch (command) {
    case "get_app_snapshot":
      return delay(snapshot) as Promise<T>;
    case "get_network_settings":
      return delay(network) as Promise<T>;
    case "get_autostart_status":
      return delay(autostart) as Promise<T>;
    case "set_autostart_enabled": {
      autostart = { enabled: Boolean(args?.enabled), preference_enabled: Boolean(args?.enabled) };
      return delay(autostart) as Promise<T>;
    }
    case "validate_config_draft": {
      const draft = JSON.parse((requireArgs(args).draft as { json: string }).json) as AppConfig;
      return delay(validate(draft), 40) as Promise<T>;
    }
    case "save_apply_config":
    case "save_server_config": {
      const fields = requireArgs(args);
      const raw = (fields.draft ?? (fields.request as { draft: unknown }).draft) as {
        json: string;
      };
      config = JSON.parse(raw.json) as AppConfig;
      publish();
      return delay({ save: { generation: 1, warning: null } }, 350) as Promise<T>;
    }
    case "set_pushing_paused":
      config = {
        ...config,
        preferences: { ...config.preferences, paused: Boolean(args?.paused) },
      };
      snapshot.runtime = { kind: args?.paused ? "paused" : "running" };
      publish();
      return delay(undefined as T);
    case "control_pomodoro": {
      const timer = snapshot.pomodoros[0];
      const action = args?.action as string;
      if (timer) {
        if (action === "start") timer.state = "running";
        if (action === "pause") timer.state = "paused";
        if (action === "reset") {
          timer.state = "idle";
          timer.remaining_seconds = timer.duration_seconds;
        }
      }
      publish();
      return delay(undefined as T);
    }
    case "refresh_provider": {
      const id = (requireArgs(args).target as { widget_id: string }).widget_id;
      const provider = snapshot.providers.find((candidate) => candidate.widget_id === id);
      if (provider) {
        provider.state = { kind: "refreshing" };
        publish();
        window.setTimeout(() => {
          provider.state = { kind: "fresh" };
          provider.age_seconds = 0;
          publish();
        }, 900);
      }
      return delay(undefined as T);
    }
    case "render_card_preview": {
      const cardId = args?.cardId as string;
      const card = config.cards.find((candidate) => candidate.id === cardId);
      if (!card) throw { category: "not-found", message: "No such card." };
      const timer = snapshot.pomodoros.find((candidate) => candidate.widget_id === cardId);
      return {
        png_base64: renderMockFrame(
          card,
          snapshot.card_data.find((candidate) => candidate.card_id === cardId),
          config.preferences.timezone,
          timer?.remaining_seconds ?? null,
        ),
        sample: true,
      } as T;
    }
    case "choose_ics_file":
      return delay("/Users/you/Calendars/work.ics") as Promise<T>;
    case "set_server_endpoint": {
      const request = requireArgs(args).request as { server_url: string; device_id: string };
      network = {
        ...network,
        server_url: request.server_url,
        // Blank means "leave the stored id alone", matching set_server_endpoint.
        device_id: request.device_id.trim() === "" ? network.device_id : request.device_id,
      };
      return delay(network) as Promise<T>;
    }
    case "provision_device": {
      const request = args?.request as {
        server_url: string;
        device_id: string;
        tier: "local" | "networked";
      };
      network = {
        server_url: request.server_url,
        device_id: request.device_id,
        tier: request.tier,
      };
      snapshot.device.tier = request.tier;
      publish();
      return delay(network, 600) as Promise<T>;
    }
    case "factory_reset_device":
      network = { server_url: "", device_id: "", tier: null };
      snapshot.device.tier = null;
      publish();
      return delay(undefined as T, 600);
    case "use_local_ownership":
      network = { ...network, tier: "local" };
      snapshot.device.tier = "local";
      publish();
      return delay(network) as Promise<T>;
    case "set_settings_window_visible":
      return delay(snapshot) as Promise<T>;
    default:
      throw { category: "not-found", message: `mock backend has no command \`${command}\`` };
  }
}

export function mockListen(handler: (payload: AppSnapshot) => void): () => void {
  listeners.add(handler);
  return () => listeners.delete(handler);
}

if (scenario === "default") {
  // eslint-disable-next-line no-console
  console.info(`[deskmate mock] scenarios: ${SCENARIOS.join(", ")} — add ?scenario=<name>`);
}

// Review hook: `?theme=dark` / `?theme=light` pins the palette so both schemes can be
// inspected from one machine. The shipped app never sets this attribute.
const themeOverride = new URLSearchParams(window.location.search).get("theme");
if (themeOverride === "dark" || themeOverride === "light") {
  document.documentElement.dataset.theme = themeOverride;
}
