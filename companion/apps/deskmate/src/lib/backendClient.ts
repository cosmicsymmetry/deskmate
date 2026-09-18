/**
 * The companion's typed HTTP client.
 *
 * Components depend on task-level functions from this module rather than on
 * routes or response parsing. Requests normalize failures into the shared
 * `IpcError` union, while snapshots arrive through an `EventSource` subscription.
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

const APP_STATE_EVENT = "app-state";
const MAX_DRAFT_BYTES = 64 * 1024;

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
 * What a missing session reads as.
 *
 * A 401 arrives with no body -- the server answers bare, so it never echoes what
 * was presented -- so the client is what turns it into a message.
 */
export const SESSION_REQUIRED_MESSAGE =
  "This browser is not signed in. Enter the admin token to continue.";

/**
 * Whether this failure is "no session" rather than anything else.
 *
 * Exported because the window has to tell the two apart: every other failure is
 * answered by retrying, and this one is answered by signing in. Asking the
 * module that constructed the error keeps that knowledge in one place instead of
 * spreading a string comparison through the UI.
 */
export function isSessionMissing(error: IpcError): boolean {
  return error.category === "runtime-unavailable" && error.message === SESSION_REQUIRED_MESSAGE;
}

/** Signs this browser in with the admin token, without selecting a display. */
export async function signIn(adminToken: string): Promise<void> {
  await request<void>("POST", "/v1/app/session", { token: adminToken });
}

/**
 * The device this window is looking at.
 *
 * Every route below is device-scoped while the UI speaks about "the display",
 * so one id has to be chosen from an explicit remembered selection or the
 * server's registry.
 */
let selectedDeviceId: string | null = null;

/** Where an explicit choice is remembered between visits. */
const DEVICE_KEY = "deskmate.device_id";

interface DeviceRow {
  id: string;
  connected: boolean;
  has_saved_config: boolean;
  configured_at: number | null;
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
 * of those shows an empty loop for a display nobody owns. Worse, "has ever been
 * configured" is not enough on its own -- the live server has three configured
 * identities and one panel.
 *
 * So: an id the operator typed wins outright, then a display that is linked
 * right now, then the one configured most recently, then the first id. The
 * recency step is what actually resolves a retired identity from the working
 * one, because the working one is the one being saved to.
 */
function pickDevice(devices: DeviceRow[]): DeviceRow | undefined {
  const remembered = rememberedDeviceId();
  const mostRecentlyConfigured = devices
    .filter((device) => device.has_saved_config)
    .sort((left, right) => (right.configured_at ?? 0) - (left.configured_at ?? 0))
    .at(0);
  return (
    devices.find((device) => device.id === remembered) ??
    devices.find((device) => device.connected) ??
    mostRecentlyConfigured ??
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
      message: SESSION_REQUIRED_MESSAGE,
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

/** Saves through the server, which applies the configuration if the display is linked. */
export function saveConfig(config: AppConfig): Promise<ConfigApplyResult> {
  const draft = draftPayload(config);
  return devicePath("/config").then((path) => request<ConfigApplyResult>("PUT", path, draft));
}

/**
 * What this window is talking to.
 *
 * The server URL is this page's own origin by construction -- the app is served
 * by the server it configures -- and the tier is always networked because this
 * web companion only uses server-owned routes. `deskmate-cli` handles cable
 * ownership.
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
 * Signs this browser in and optionally selects a display.
 *
 * The server endpoint is already fixed by the page's origin; the token is traded
 * for a session cookie, and a non-empty device id selects one display from a
 * multi-display server.
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
// web companion does not implement. `deskmate-cli` is the supported path for
// all of them. An exported stub that always threw would have put the same absence
// behind a button that looks like it works.

/**
 * Clears `preferences.paused`, which is what "Resume sending" means.
 *
 * The server runtime reads this preference from configuration, so recovering
 * from an already-paused document is a normal configuration write.
 */
export async function resumePushing(): Promise<void> {
  const snapshot = await getAppSnapshot();
  if (!snapshot.config.preferences.paused) {
    return;
  }
  await saveConfig({
    ...snapshot.config,
    preferences: { ...snapshot.config.preferences, paused: false },
  });
}

export function controlPomodoro(cardId: string, action: PomodoroAction): Promise<void> {
  return devicePath("/pomodoro").then((path) =>
    request<void>("POST", path, { card_id: cardId, action }),
  );
}

// Autostart is not exposed: it is a property of an application that starts, and
// this one is a page. `preferences.autostart` stays in the frozen schema and
// retains its saved value.

export function renderCardPreview(cardId: string): Promise<PreviewFrame> {
  return devicePath("/preview").then((path) =>
    request<PreviewFrame>("POST", path, { card_id: cardId }),
  );
}

/**
 * Mints a picture source and returns it in the window's own vocabulary.
 *
 * The route answers `{id, token}`; the UI contract requires `{source_id, token,
 * push_url}`. Validate and translate that response here so a malformed route
 * response cannot produce an undefined source id after the server has already
 * minted the source.
 *
 * `push_url` is this page's own origin, for the same reason `getNetworkSettings`
 * reports it: the app is served by the server a producer would push to.
 */
export async function mintImageSource(name: string, faceKind?: string): Promise<MintedImageSource> {
  const minted = await request<{ id: string; token: string }>("POST", "/v1/images", {
    name,
    face_kind: faceKind ?? null,
  });
  if (typeof minted.id !== "string" || typeof minted.token !== "string") {
    fail({
      category: "internal",
      message: "the server returned a picture source this app could not read",
    });
  }
  return {
    source_id: minted.id,
    token: minted.token,
    push_url: `${window.location.origin}/v1/images/${encodeURIComponent(minted.token)}`,
  };
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
 * `EventSource` reconnects on its own, so a server restart or dropped tunnel
 * resumes without the page coordinating retries. A parse failure is skipped
 * rather than thrown, because one malformed frame must not tear down a stream
 * that can keep delivering good ones.
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
