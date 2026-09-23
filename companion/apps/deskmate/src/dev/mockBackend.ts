/**
 * Dev-only in-browser stand-in for the Rust runtime.
 *
 * Reached through `src/dev/backendClient.ts`, which `vite.config.ts` aliases in
 * place of the real HTTP client under `VITE_DESKMATE_MOCK=1`. No application code
 * imports anything from `src/dev/`, so a production build resolves the real
 * client and never reaches this file.
 *
 * Scenarios let a whole device state be selected from the URL — `?scenario=offline`,
 * `?scenario=invalid`, `?scenario=firstrun`, `?scenario=signedout` —
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
import type { PanelRow } from "../lib/account";
import { crc32c, MessageType, rawDecoded } from "../lib/serial/codec";
import type { PanelPort } from "../lib/serial/port";
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
  "signedout",
  "setup",
  "nopanels",
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
const CREATABLE_FACES: FaceDescriptor[] = [
  {
    kind: "weather",
    label: "Weather",
    fields: [
      {
        key: "location",
        label: "Location",
        type: "text",
        value: "",
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
  },
  {
    kind: "hackernews",
    label: "Hacker News",
    fields: [
      {
        key: "list",
        label: "Stories",
        type: "enum",
        value: "top",
        options: [
          { value: "top", label: "Front page" },
          { value: "best", label: "Best" },
          { value: "new", label: "Newest" },
          { value: "ask", label: "Ask HN" },
          { value: "show", label: "Show HN" },
        ],
      },
    ],
  },
  {
    kind: "rss",
    label: "RSS feed",
    fields: [
      {
        key: "url",
        label: "Feed URL",
        type: "url",
        value: "",
        placeholder: "https://example.com/feed.xml",
      },
      {
        key: "title",
        label: "Title",
        type: "text",
        value: "",
        placeholder: "News",
      },
    ],
  },
  {
    kind: "token",
    label: "Token price",
    fields: [
      {
        key: "coin_id",
        label: "Ticker",
        type: "text",
        value: "",
        placeholder: "SOL",
      },
      {
        key: "currency",
        label: "Currency",
        type: "text",
        value: "usd",
        placeholder: "usd",
      },
      {
        key: "chart",
        label: "Chart",
        type: "enum",
        value: "line",
        options: [
          { value: "line", label: "Line" },
          { value: "candles", label: "Candles" },
          { value: "none", label: "None" },
        ],
      },
    ],
  },
];
const listeners = new Set<(next: AppSnapshot) => void>();

function cloneFace(face: FaceDescriptor): FaceDescriptor {
  return JSON.parse(JSON.stringify(face)) as FaceDescriptor;
}

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

const imageSources = new Map(config.image_sources.map((source) => [source.id, { ...source }]));
const imageSourceFaces = new Map<string, FaceDescriptor>();
if (scenario === "picture") {
  const source = config.image_sources[0];
  const face = cloneFace(CREATABLE_FACES[0]);
  face.fields[0].value = "Dubai";
  if (source) imageSourceFaces.set(source.id, face);
}
const mintedTokens = new Set<string>();
let nextSourceNumber = 1;

function publish() {
  snapshot = { ...snapshot, config };
  for (const listener of listeners) listener(snapshot);
}

function reconcilePomodoros(previousConfig: AppConfig, nextConfig: AppConfig) {
  const previousSettings = new Map(
    previousConfig.cards.filter((card) => card.kind === "pomodoro").map((card) => [card.id, card]),
  );
  const previousSnapshots = new Map(snapshot.pomodoros.map((timer) => [timer.card_id, timer]));
  snapshot.pomodoros = nextConfig.cards.flatMap((card) => {
    if (card.kind !== "pomodoro") return [];
    const previous = previousSettings.get(card.id);
    const timer = previousSnapshots.get(card.id);
    if (
      previous?.kind === "pomodoro" &&
      previous.label === card.label &&
      previous.duration_seconds === card.duration_seconds &&
      timer
    ) {
      return [timer];
    }
    return [
      {
        card_id: card.id,
        state: "idle" as const,
        duration_seconds: card.duration_seconds,
        remaining_seconds: card.duration_seconds,
      },
    ];
  });
}

function imageSourceDescriptors(): ImageSourceDescriptor[] {
  return Array.from(imageSources.values(), (source) => {
    const face = imageSourceFaces.get(source.id);
    // The real server draws within seconds of the last field being filled in; the
    // mock says so at once, because it has nothing to draw.
    const complete = face?.fields.every((field) => field.type === "enum" || field.value.trim());
    return {
      ...source,
      face: face ? cloneFace(face) : null,
      face_status: face
        ? {
            state: complete ? ("drawn" as const) : ("needs-settings" as const),
            message: null,
            at_unix_seconds: complete ? Math.floor(Date.now() / 1000) : null,
          }
        : null,
    };
  });
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
  let changed = false;
  for (const timer of snapshot.pomodoros) {
    if (timer.state === "running" && timer.remaining_seconds > 0) {
      timer.remaining_seconds -= 1;
      if (timer.remaining_seconds === 0) timer.state = "completed";
      changed = true;
    }
  }
  if (changed) {
    snapshot.card_data = mockCardData();
    publish();
  }
}, 1000);

const delay = <T>(value: T, ms = 90): Promise<T> =>
  new Promise((resolve) => window.setTimeout(() => resolve(value), ms));

const mockAccount = {
  id: "account-owner",
  email: "owner@example.com",
  email_verified: true,
  is_instance_owner: true,
};
let setupRequired = scenario === "setup";
let signupsOpen = true;
let panels: PanelRow[] =
  scenario === "nopanels"
    ? []
    : [
        {
          id: network.device_id,
          connected: snapshot.device.connection.kind === "online",
          has_saved_config: snapshot.has_saved_config,
          configured_at: 1_795_000_000,
          state: "active" as const,
        },
      ];

function cobsEncode(decoded: Uint8Array): Uint8Array {
  const encoded: number[] = [0];
  let codeIndex = 0;
  let code = 1;
  for (const byte of decoded) {
    if (byte === 0) {
      encoded[codeIndex] = code;
      codeIndex = encoded.length;
      encoded.push(0);
      code = 1;
    } else {
      encoded.push(byte);
      code += 1;
      if (code === 0xff) {
        encoded[codeIndex] = code;
        codeIndex = encoded.length;
        encoded.push(0);
        code = 1;
      }
    }
  }
  encoded[codeIndex] = code;
  return Uint8Array.from([...encoded, 0]);
}

function responseFrame(messageType: number, requestId: number, payload: Uint8Array): Uint8Array {
  const decoded = new Uint8Array(10 + payload.byteLength + 4);
  const view = new DataView(decoded.buffer);
  decoded[0] = 2;
  decoded[1] = messageType;
  view.setUint32(4, requestId, true);
  view.setUint16(8, payload.byteLength, true);
  decoded.set(payload, 10);
  view.setUint32(decoded.byteLength - 4, crc32c(decoded.subarray(0, -4)), true);
  return cobsEncode(decoded);
}

function statusResponse(requestId: number, configured: boolean): Uint8Array {
  const firmware = new TextEncoder().encode("deskmate-mock");
  const ip = new TextEncoder().encode(configured ? "192.0.2.1" : "");
  const payload = Uint8Array.from([
    0xb4,
    0x00,
    0x02,
    0x01,
    0x60 + firmware.byteLength,
    ...firmware,
    0x02,
    0x01,
    0x03,
    0x01,
    0x04,
    0x00,
    0x05,
    0x18,
    0x18,
    0x06,
    0x00,
    0x07,
    0x18,
    0x5a,
    0x08,
    0x01,
    0x09,
    0x00,
    0x0a,
    0x00,
    0x0b,
    0x00,
    0x0c,
    0x00,
    0x0d,
    0x00,
    0x0e,
    0x00,
    0x0f,
    0x00,
    0x17,
    0x19,
    0x07,
    0xe0,
    0x18,
    0x18,
    configured ? 0x01 : 0x00,
    0x18,
    0x19,
    configured ? 0x02 : 0x00,
    0x18,
    0x1b,
    0x60 + ip.byteLength,
    ...ip,
  ]);
  return responseFrame(MessageType.StatusResponse, requestId, payload);
}

/**
 * The no-panels harness speaks at the same seam as Web Serial. It deliberately
 * answers only the two requests the real setup page sends and the real board
 * accepts: StatusRequest and NetworkConfig.
 */
export class MockPanelPort implements PanelPort {
  private readonly chunks: Uint8Array[] = [];
  private wakeReader: (() => void) | null = null;
  private opened = false;
  private configured = false;

  async open(): Promise<void> {
    this.opened = true;
  }

  async write(bytes: Uint8Array): Promise<void> {
    if (!this.opened) throw new Error("The mock panel is not open.");
    const request = rawDecoded(bytes);
    const view = new DataView(request.buffer, request.byteOffset, request.byteLength);
    const requestId = view.getUint32(4, true);
    if (request[1] === MessageType.StatusRequest) {
      this.enqueue(statusResponse(requestId, this.configured));
    } else if (request[1] === MessageType.NetworkConfig) {
      this.configured = true;
      panels = panels.map((panel) =>
        panel.state === "pending" ? { ...panel, connected: true, state: "active" } : panel,
      );
      this.enqueue(
        responseFrame(
          MessageType.Ack,
          requestId,
          Uint8Array.of(0xa1, 0x00, MessageType.NetworkConfig),
        ),
      );
    }
  }

  async *readable(): AsyncIterable<Uint8Array> {
    while (this.opened) {
      const chunk = this.chunks.shift();
      if (chunk) {
        yield chunk;
        continue;
      }
      await new Promise<void>((resolve) => {
        this.wakeReader = resolve;
      });
      this.wakeReader = null;
    }
  }

  async close(): Promise<void> {
    this.opened = false;
    this.wakeReader?.();
  }

  onDisconnect(_listener: () => void): void {}

  private enqueue(chunk: Uint8Array): void {
    this.chunks.push(chunk);
    this.wakeReader?.();
  }
}

if (scenario === "nopanels") {
  (
    globalThis as typeof globalThis & {
      __DESKMATE_MOCK_PANEL_PORT__?: () => PanelPort;
    }
  ).__DESKMATE_MOCK_PANEL_PORT__ = () => new MockPanelPort();
}

export async function mockAccountRequest(
  method: string,
  path: string,
  body?: unknown,
): Promise<unknown> {
  const payload = body as Record<string, unknown> | undefined;
  switch (`${method} ${path}`) {
    case "GET /v1/app/instance":
      return delay({
        setup_required: setupRequired,
        google_enabled: true,
        signups_open: signupsOpen,
        edition: "self-hosted" as const,
      });
    case "POST /v1/app/setup":
      setupRequired = false;
      return delay({ account: { ...mockAccount, email: String(payload?.email ?? "") } });
    case "POST /v1/app/auth/email":
      return delay({});
    case "POST /v1/app/auth/link":
      if (payload?.token === "expired") throw new Error("This sign-in link has expired.");
      return delay({ account: mockAccount });
    case "GET /v1/app/account":
      return delay(mockAccount);
    case "DELETE /v1/app/session":
    case "POST /v1/app/sessions/revoke-all":
    case "DELETE /v1/app/account":
      return delay(undefined);
    case "PUT /v1/app/instance/signups":
      signupsOpen = payload?.open === true;
      return delay({ signups_open: signupsOpen });
    case "GET /v1/app/devices":
      return delay(panels.map((panel) => ({ ...panel })));
    case "POST /v1/app/devices/claim": {
      const panel = {
        id: "desk-claimed",
        connected: false,
        has_saved_config: false,
        configured_at: null,
        state: "pending" as const,
      };
      panels = [...panels, panel];
      return delay({
        device_id: panel.id,
        token: "claim-token-shown-once",
        link_url: "wss://deskmate.rodi.one/v1/device/link",
      });
    }
    default:
      if (method === "DELETE" && path.startsWith("/v1/app/devices/")) {
        const id = decodeURIComponent(path.slice("/v1/app/devices/".length));
        panels = panels.filter((panel) => panel.id !== id);
        return delay(undefined);
      }
      throw new Error(`The mock backend does not implement ${method} ${path}.`);
  }
}

export function mockGetAppSnapshot(): Promise<AppSnapshot> {
  return delay(snapshot);
}

export function getNetworkSettings(): Promise<NetworkSettings> {
  return delay(network);
}

export function mintImageSource(name: string, faceKind?: string): Promise<MintedImageSource> {
  const face =
    faceKind === undefined
      ? undefined
      : CREATABLE_FACES.find((candidate) => candidate.kind === faceKind);
  if (faceKind !== undefined && !face) {
    return Promise.reject({
      category: "invalid-payload",
      message: `Unknown face kind "${faceKind}".`,
    });
  }

  let sourceId: string;
  let token: string;
  do {
    sourceId = `picture-source-${nextSourceNumber}`;
    token = `dev-picture-token-${nextSourceNumber}`;
    nextSourceNumber += 1;
  } while (imageSources.has(sourceId) || mintedTokens.has(token));

  imageSources.set(sourceId, { id: sourceId, name });
  mintedTokens.add(token);
  if (face) imageSourceFaces.set(sourceId, cloneFace(face));
  return delay({
    source_id: sourceId,
    token,
    push_url: `${network.server_url.replace(/\/$/, "")}/v1/images/${token}`,
  });
}

export function listCreatableFaces(): Promise<FaceDescriptor[]> {
  return delay(CREATABLE_FACES.map(cloneFace));
}

export function listImageSources(): Promise<ImageSourceDescriptor[]> {
  return delay(imageSourceDescriptors());
}

export async function updateImageSourceFace(
  sourceId: string,
  fields: Record<string, string>,
): Promise<FaceDescriptor> {
  if (!imageSources.has(sourceId)) {
    throw { category: "not-found", message: "This picture source no longer exists." };
  }
  const face = imageSourceFaces.get(sourceId);
  if (!face) {
    throw {
      category: "invalid-payload",
      message: "This picture source has no configurable face.",
    };
  }
  const updated = {
    ...face,
    fields: face.fields.map((field) => ({
      ...field,
      value: fields[field.key] ?? field.value,
    })),
  };
  imageSourceFaces.set(sourceId, updated);
  return delay(cloneFace(updated));
}

export function validateConfigDraft(draft: AppConfig): Promise<DraftValidation> {
  return delay(validate(JSON.parse(JSON.stringify(draft)) as AppConfig), 40);
}

export function saveConfig(draft: AppConfig): Promise<ConfigApplyResult> {
  const nextConfig = JSON.parse(JSON.stringify(draft)) as AppConfig;
  reconcilePomodoros(config, nextConfig);
  config = nextConfig;
  const retained = new Set(nextConfig.image_sources.map((source) => source.id));
  for (const sourceId of imageSources.keys()) {
    if (!retained.has(sourceId)) {
      imageSources.delete(sourceId);
      imageSourceFaces.delete(sourceId);
    }
  }
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

export async function controlPomodoro(cardId: string, action: PomodoroAction): Promise<void> {
  const timer = snapshot.pomodoros.find((candidate) => candidate.card_id === cardId);
  if (!timer) {
    throw { category: "device", message: `UnknownCard { card_id: "${cardId}" }` };
  }
  switch (action) {
    case "start":
      if (timer.state === "idle" || timer.state === "paused") timer.state = "running";
      break;
    case "pause":
      if (timer.state === "running") timer.state = "paused";
      break;
    case "toggle":
      if (timer.state === "running") timer.state = "paused";
      else if (timer.state === "idle" || timer.state === "paused") timer.state = "running";
      break;
    case "reset":
      timer.state = "idle";
      timer.remaining_seconds = timer.duration_seconds;
      break;
    default: {
      const exhaustive: never = action;
      throw new Error(`Unhandled pomodoro action: ${exhaustive}`);
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
