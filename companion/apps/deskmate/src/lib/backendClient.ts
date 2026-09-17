/**
 * The companion's backend, over HTTP.
 *
 * This file replaces `lib/tauri.ts` and deliberately keeps its exact export
 * surface: the same function names, arguments and return types. Everything above
 * this module -- every component, `useAppState`, every test -- was written
 * against that surface and does not change because the transport did.
 *
 * `invoke(command, args)` became `fetch`, and the `app-state` Tauri event became
 * an `EventSource`. The error shape is unchanged too: the server answers with the
 * same `IpcError` union this module already knew how to raise, so `toIpcError`
 * and every `catch` that switches on `category` still mean what they meant.
 *
 * Authentication is a session cookie, obtained by trading the admin token
 * through `setServerEndpoint`. The cookie is `HttpOnly`, so this file can neither
 * read nor forge it; `credentials: "same-origin"` is what attaches it.
 */

import type {
  AppConfig,
  AppSnapshot,
  ConfigApplyResult,
  DraftValidation,
  FaceDescriptor,
  ImageSourceDescriptor,
  IpcError,
  MintedImageSource,
  NetworkSettings,
  PomodoroAction,
  PreviewFrame,
} from "./types";

export const APP_STATE_EVENT = "app-state";
export const MAX_DRAFT_BYTES = 64 * 1024;

const IPC_ERROR_CATEGORIES = new Set<IpcError["category"]>([
  "invalid-payload",
  "payload-too-large",
  "validation",
  "persistence",
  "runtime-unavailable",
  "not-found",
  "device",
  "internal",
]);

export class DeskmateCommandError extends Error {
  readonly details: IpcError;

  constructor(details: IpcError) {
    super(details.message);
    this.name = "DeskmateCommandError";
    this.details = details;
  }
}

export function toIpcError(error: unknown): IpcError {
  if (
    typeof error === "object" &&
    error !== null &&
    "category" in error &&
    "message" in error &&
    typeof error.category === "string" &&
    IPC_ERROR_CATEGORIES.has(error.category as IpcError["category"]) &&
    typeof error.message === "string"
  ) {
    return error as IpcError;
  }
  if (error instanceof DeskmateCommandError) {
    return error.details;
  }
  return {
    category: "internal",
    message: error instanceof Error ? error.message : String(error),
  };
}

function fail(details: IpcError): never {
  throw new DeskmateCommandError(details);
}

/**
 * The device this window is looking at.
 *
 * Every route below is device-scoped while the UI above still speaks about "the
 * display", so one id has to be chosen. The Mac app read it from its own
 * settings file; a browser has none, and the server already knows the answer.
 */
let selectedDeviceId: string | null = null;

/** Where an explicit choice is remembered between visits. */
const DEVICE_KEY = "deskmate.device_id";

interface DeviceRow {
  id: string;
  connected: boolean;
  has_saved_config: boolean;
}

function rememberedDeviceId(): string | null {
  try {
    return window.localStorage.getItem(DEVICE_KEY);
  } catch {
    // Storage can be denied outright (private windows, blocked site data). That
    // is not a failure: it just means the choice is not remembered.
    return null;
  }
}

function rememberDeviceId(id: string): void {
  try {
    window.localStorage.setItem(DEVICE_KEY, id);
  } catch {
    // See `rememberedDeviceId`.
  }
}

/**
 * Picks the display this window should open on.
 *
 * Registry order is mint order, not usefulness: a server that has minted spare
 * identities lists several that were never configured, and opening on the first
 * of those shows an empty loop for a display nobody owns. So an explicit choice
 * wins, then a display that is actually linked right now, then one that has been
 * configured, and only then the first id in the list.
 */
function pickDevice(devices: DeviceRow[]): DeviceRow | undefined {
  const remembered = rememberedDeviceId();
  return (
    devices.find((device) => device.id === remembered) ??
    devices.find((device) => device.connected) ??
    devices.find((device) => device.has_saved_config) ??
    devices.at(0)
  );
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  let response: Response;
  try {
    response = await fetch(path, {
      method,
      credentials: "same-origin",
      headers: body === undefined ? undefined : { "content-type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch (error) {
    // A failed fetch is the network, not the server: there is no body to read
    // and no category to trust, so this is reported as what it is.
    fail({
      category: "runtime-unavailable",
      message: error instanceof Error ? error.message : "the server could not be reached",
    });
  }

  if (response.status === 401) {
    // The session is the thing that is missing, and saying so is what lets the
    // UI offer the one action that fixes it rather than a generic failure.
    fail({
      category: "runtime-unavailable",
      message: "This browser is not signed in. Enter the admin token to continue.",
    });
  }

  if (!response.ok) {
    let parsed: unknown = null;
    try {
      parsed = await response.json();
    } catch {
      parsed = null;
    }
    const details = toIpcError(parsed);
    if (parsed === null) {
      details.message = `${response.status} ${response.statusText}`.trim();
    }
    throw new DeskmateCommandError(details);
  }

  if (response.status === 204) {
    return undefined as T;
  }
  return (await response.json()) as T;
}

async function deviceId(): Promise<string> {
  if (selectedDeviceId !== null) {
    return selectedDeviceId;
  }
  const devices = await request<DeviceRow[]>("GET", "/v1/app/devices");
  const chosen = pickDevice(devices);
  if (!chosen) {
    fail({
      category: "not-found",
      message: "This server owns no display yet. Mint one before configuring it.",
    });
  }
  selectedDeviceId = chosen.id;
  return chosen.id;
}

async function devicePath(suffix: string): Promise<string> {
  return `/v1/app/${encodeURIComponent(await deviceId())}${suffix}`;
}

function draftPayload(config: AppConfig): { json: string } {
  let json: string;
  try {
    json = JSON.stringify(config);
  } catch (error) {
    fail({
      category: "invalid-payload",
      message: error instanceof Error ? error.message : "configuration cannot be serialized",
    });
  }
  if (new TextEncoder().encode(json).byteLength > MAX_DRAFT_BYTES) {
    fail({
      category: "payload-too-large",
      message: `Configuration exceeds the ${MAX_DRAFT_BYTES}-byte limit.`,
      maximum_bytes: MAX_DRAFT_BYTES,
    });
  }
  return { json };
}

export function getAppSnapshot(): Promise<AppSnapshot> {
  return devicePath("/snapshot").then((path) => request<AppSnapshot>("GET", path));
}

export function validateConfigDraft(config: AppConfig): Promise<DraftValidation> {
  const draft = draftPayload(config);
  return devicePath("/config/validate").then((path) =>
    request<DraftValidation>("POST", path, draft),
  );
}

/**
 * Both save paths are the same request now.
 *
 * The Mac app had two because it had two owners: a local tier where it wrote the
 * file itself, and a networked tier where it PUT to the server. The browser only
 * ever has the second, so `saveApplyConfig` and `saveServerConfig` converge --
 * kept as two names so the call sites above did not have to change.
 */
export function saveApplyConfig(config: AppConfig): Promise<ConfigApplyResult> {
  const draft = draftPayload(config);
  return devicePath("/config").then((path) => request<ConfigApplyResult>("PUT", path, draft));
}

export const saveServerConfig = saveApplyConfig;

/**
 * What this window is talking to.
 *
 * The server URL is this page's own origin by construction -- the app is served
 * by the server it configures -- and the tier is always networked, because a
 * browser cannot be a device's local owner.
 */
export function getNetworkSettings(): Promise<NetworkSettings> {
  return deviceId()
    .then((id) => ({
      server_url: window.location.origin,
      device_id: id,
      tier: "networked" as const,
    }))
    .catch(() => ({
      server_url: window.location.origin,
      device_id: "",
      tier: "networked" as const,
    }));
}

/**
 * Signs this browser in.
 *
 * Named for what it did on the Mac -- where it stored a server endpoint and an
 * admin token -- because the form that calls it is unchanged. Here the endpoint
 * is already known and the admin token is traded for a session cookie, so the
 * same three fields still do the same job: say which server, which display, and
 * prove you may configure it.
 */
export async function setServerEndpoint(
  _serverUrl: string,
  requestedDeviceId: string,
  adminToken: string,
): Promise<NetworkSettings> {
  await request<void>("POST", "/v1/app/session", { token: adminToken });
  const requested = requestedDeviceId.trim();
  if (requested === "") {
    selectedDeviceId = null;
  } else {
    // An id typed into the panel is an explicit choice and outlives the tab,
    // which is the only way to reach a second display on a multi-display server.
    selectedDeviceId = requested;
    rememberDeviceId(requested);
  }
  return getNetworkSettings();
}

// Provisioning, factory reset and the local-ownership switch are not here.
// They write the display's own Wi-Fi and server settings over the cable, which a
// browser does not have, and `deskmate-cli` already performs all of them. An
// exported stub that always threw would have put the same absence behind a
// button that looks like it works.

/**
 * Clears `preferences.paused`, which is what "Resume sending" means.
 *
 * On the Mac this was a runtime command; here it is a configuration write,
 * because the server's runtime reads the same preference. The escape hatch
 * `CLAUDE.md` requires -- a config that arrives already paused must be
 * resumable -- therefore still works.
 */
export async function resumePushing(): Promise<void> {
  const snapshot = await getAppSnapshot();
  if (!snapshot.config.preferences.paused) {
    return;
  }
  await saveApplyConfig({
    ...snapshot.config,
    preferences: { ...snapshot.config.preferences, paused: false },
  });
}

export function controlPomodoro(cardId: string, action: PomodoroAction): Promise<void> {
  return devicePath("/pomodoro").then((path) =>
    request<void>("POST", path, { card_id: cardId, action }),
  );
}

// Autostart is not here either: it is a property of an application that starts,
// and this one is a page. `preferences.autostart` stays in the schema because
// removing a field would be a schema bump, and it keeps whatever value the owner
// last saved -- the browser simply does not offer to change something it cannot
// honour.

export function renderCardPreview(cardId: string): Promise<PreviewFrame> {
  return devicePath("/preview").then((path) =>
    request<PreviewFrame>("POST", path, { card_id: cardId }),
  );
}

export function mintImageSource(name: string, faceKind?: string): Promise<MintedImageSource> {
  return request<MintedImageSource>("POST", "/v1/images", {
    name,
    face_kind: faceKind ?? null,
  });
}

/** The faces the server can draw. The add menu is built from this, which is why
 *  "Weather" can appear in the window without the app knowing what weather is. */
export function listCreatableFaces(): Promise<FaceDescriptor[]> {
  return request<FaceDescriptor[]>("GET", "/v1/faces");
}

export function listImageSources(): Promise<ImageSourceDescriptor[]> {
  return request<ImageSourceDescriptor[]>("GET", "/v1/images");
}

export function updateImageSourceFace(
  sourceId: string,
  fields: Record<string, string>,
): Promise<FaceDescriptor> {
  return request<FaceDescriptor>("PUT", `/v1/images/${encodeURIComponent(sourceId)}/face`, {
    fields,
  });
}

/**
 * The snapshot stream.
 *
 * `EventSource` reconnects on its own, which is the behaviour the Tauri event
 * had for free: a server restart or a dropped tunnel resumes without the window
 * needing to know it happened. A parse failure is skipped rather than thrown,
 * because a malformed frame must not tear down a subscription that will
 * otherwise keep delivering good ones.
 */
export function listenToAppState(onSnapshot: (snapshot: AppSnapshot) => void): Promise<() => void> {
  return devicePath("/events").then((path) => {
    const source = new EventSource(path, { withCredentials: true });
    source.addEventListener(APP_STATE_EVENT, (event) => {
      try {
        onSnapshot(JSON.parse((event as MessageEvent<string>).data) as AppSnapshot);
      } catch {
        // A frame this build cannot parse is dropped, not fatal.
      }
    });
    return () => source.close();
  });
}
