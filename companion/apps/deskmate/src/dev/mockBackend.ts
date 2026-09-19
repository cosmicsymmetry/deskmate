/**
 * Dev-only in-browser stand-in for the Rust runtime.
 *
 * Reached through `src/dev/backendClient.ts`, which `vite.config.ts` aliases in
 * place of the real HTTP client under `VITE_DESKMATE_MOCK=1`. No application code
 * imports anything from `src/dev/`, so a production build resolves the real
 * client and never reaches this file.
 *
 * Scenarios let a whole device state be selected from the URL — `?scenario=offline`,
 * `?scenario=invalid`, `?scenario=firstrun`, `?scenario=empty` —
 * so every state the UI must handle can be opened, reviewed and screenshotted without
 * hardware. `?scenario=list` prints the set to the console.
 */

import type {
  AppConfig,
  AppSnapshot,
  ConfigApplyResult,
  DraftValidation,
  FaceDescriptor,
  ImageSourceDescriptor,
  NetworkSettings,
  MintedImageSource,
  PomodoroAction,
  PreviewFrame,
  ValidationIssue,
} from "../lib/types";
import { mockCardData, mockConfig, mockNetworkSettings, mockSnapshot } from "./fixture";
import { renderMockFrame } from "./mockPreview";
import { mockPictureConfig } from "./pictureFixture";

export const SCENARIOS = [
  "default",
  "offline",
  "standalone",
  "unowned",
  "invalid",
  "firstrun",
  "empty",
  "carderror",
  "picture",
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
let mockFace: FaceDescriptor = {
  kind: "server-face",
  label: "Source settings",
  fields: [
    {
      key: "place",
      label: "Place",
      type: "text",
      value: "Dubai",
      placeholder: "Dubai",
    },
    {
      key: "units",
      label: "Units",
      type: "enum",
      value: "metric",
      options: [
        { value: "metric", label: "Metric" },
        { value: "imperial", label: "Imperial" },
      ],
    },
  ],
};
const listeners = new Set<(next: AppSnapshot) => void>();

function applyScenario() {
  switch (scenario) {
    case "offline":
      snapshot.device.connection = {
        kind: "disconnected",
        reason: "The display is not linked to the server",
      };
      snapshot.device.port_name = null;
      snapshot.device.wifi_state = "down";
      snapshot.device.ip = null;
      snapshot.device.last_network_error = "WebSocket closed: 1006";
      break;
    case "standalone":
      snapshot.device.connection = { kind: "standalone" };
      break;
    case "unowned":
      snapshot.device.tier = null;
      snapshot.device.connection = { kind: "connecting" };
      network = { ...network, device_id: "", tier: null };
      break;
    case "invalid":
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
      // A real first run is `AppConfig::default()`: one clock, already in the loop. The
      // only step left is saving.
      config = {
        ...mockConfig(),
        cards: [mockConfig().cards[0]],
      };
      snapshot = mockSnapshot(config);
      snapshot.has_saved_config = false;
      snapshot.pomodoros = [];
      snapshot.card_data = [];
      break;
    case "empty":
      config = {
        ...mockConfig(),
        cards: [],
      };
      snapshot = mockSnapshot(config);
      snapshot.pomodoros = [];
      snapshot.card_data = [];
      break;
    case "carderror":
      snapshot.card_errors = [
        {
          kind: "scene-refused",
          card_id: "air-quality",
          message: "the display could not render this card's complete scene",
        },
      ];
      break;
    case "picture":
      config = mockPictureConfig();
      snapshot = mockSnapshot(config);
      snapshot.card_data = [];
      snapshot.pomodoros = [];
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

function imageSourceDescriptors(): ImageSourceDescriptor[] {
  return config.image_sources.map((source, index) => ({
    ...source,
    face: scenario === "picture" && index === 0 ? mockFace : null,
  }));
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
      case "clock":
        break;
      case "picture":
        if (!draft.image_sources.some((source) => source.id === card.source_id)) {
          push(`${at}.source_id`, "missing-reference", "Choose an existing picture source.");
        }
        break;
    }
    if (card.alert.kind !== "none" && card.alert.hold.kind === "seconds") {
      const held = card.alert.hold.value;
      if (held < 5 || held > 600) {
        push(`${at}.alert.hold.value`, "out-of-range", "Hold for between 5 and 600 seconds.");
      }
    }
  });

  if (draft.advance.kind === "timed") {
    const dwell = draft.advance.default_dwell_seconds;
    if (dwell < 5 || dwell > 3600) {
      push("advance.default_dwell_seconds", "out-of-range", "Use 5 to 3600 seconds.");
    }
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

const delay = <T>(value: T, ms = 90): Promise<T> =>
  new Promise((resolve) => window.setTimeout(() => resolve(value), ms));

export function mockGetAppSnapshot(): Promise<AppSnapshot> {
  return delay(snapshot);
}

export function getNetworkSettings(): Promise<NetworkSettings> {
  return delay(network);
}

export function mintImageSource(_name: string, _faceKind?: string): Promise<MintedImageSource> {
  const sourceNumber = config.image_sources.length + 1;
  const token = `dev-picture-token-${sourceNumber}`;
  return delay({
    source_id: `picture-source-${sourceNumber}`,
    token,
    push_url: `${network.server_url.replace(/\/$/, "")}/v1/images/${token}`,
  });
}

export async function listCreatableFaces(): Promise<FaceDescriptor[]> {
  throw { category: "not-found", message: "mock backend has no operation `list_creatable_faces`" };
}

export function listImageSources(): Promise<ImageSourceDescriptor[]> {
  return delay(imageSourceDescriptors());
}

export async function updateImageSourceFace(
  sourceId: string,
  fields: Record<string, string>,
): Promise<FaceDescriptor> {
  if (!config.image_sources.some((source) => source.id === sourceId)) {
    throw { category: "not-found", message: "This picture source no longer exists." };
  }
  mockFace = {
    ...mockFace,
    fields: mockFace.fields.map((field) => ({
      ...field,
      value: fields[field.key] ?? field.value,
    })),
  };
  return delay(mockFace);
}

export function validateConfigDraft(draft: AppConfig): Promise<DraftValidation> {
  return delay(validate(JSON.parse(JSON.stringify(draft)) as AppConfig), 40);
}

export function saveConfig(draft: AppConfig): Promise<ConfigApplyResult> {
  config = JSON.parse(JSON.stringify(draft)) as AppConfig;
  try {
    publish();
  } catch (error) {
    // Serialization fails synchronously; subscriber failures reject the save promise.
    return Promise.reject(error);
  }
  return delay({ save: { generation: 1, warning: null } }, 350);
}

export async function resumePushing(): Promise<void> {
  config = {
    ...config,
    preferences: { ...config.preferences, paused: false },
  };
  snapshot.runtime = { kind: "running" };
  publish();
  return delay(undefined);
}

export async function controlPomodoro(_cardId: string, action: PomodoroAction): Promise<void> {
  const timer = snapshot.pomodoros[0];
  if (timer) {
    if (action === "start") timer.state = "running";
    if (action === "pause") timer.state = "paused";
    if (action === "reset") {
      timer.state = "idle";
      timer.remaining_seconds = timer.duration_seconds;
    }
  }
  publish();
  return delay(undefined);
}

export async function renderCardPreview(cardId: string): Promise<PreviewFrame> {
  const card = config.cards.find((candidate) => candidate.id === cardId);
  if (!card) throw { category: "not-found", message: "No such card." };
  const timer = snapshot.pomodoros.find((candidate) => candidate.card_id === cardId);
  return {
    png_base64: renderMockFrame(
      card,
      config.preferences.timezone,
      timer?.remaining_seconds ?? null,
    ),
    sample: true,
    state: null,
  };
}

export function selectMockDevice(deviceId: string): Promise<NetworkSettings> {
  network = {
    ...network,
    // A blank selection keeps the harness's automatically selected display.
    device_id: deviceId.trim() === "" ? network.device_id : deviceId,
  };
  return delay(network);
}

export function listenToAppState(handler: (payload: AppSnapshot) => void): Promise<() => void> {
  listeners.add(handler);
  return Promise.resolve(() => listeners.delete(handler));
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
