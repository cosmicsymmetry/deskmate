/**
 * Dev-only stand-in for `src/lib/backendClient.ts`.
 *
 * Enabled by `VITE_DESKMATE_MOCK=1`, which makes `vite.config.ts` alias
 * `./backendClient` at this module. No application code imports anything from
 * `src/dev/`, so a production build resolves the real HTTP client and never
 * reaches this file.
 *
 * Every function here forwards to `mockBackend`'s command dispatcher with the
 * arguments the Tauri bridge used to send. Keeping that indirection -- rather
 * than re-implementing the scenarios against the new function surface -- is what
 * lets the ten `?scenario=` states keep working unchanged through the transport
 * change they know nothing about.
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
} from "../lib/types";
import { mockInvoke, mockListen } from "./mockBackend";

export const APP_STATE_EVENT = "app-state";
export const MAX_DRAFT_BYTES = 64 * 1024;

export class DeskmateCommandError extends Error {
  readonly details: IpcError;

  constructor(details: IpcError) {
    super(details.message);
    this.name = "DeskmateCommandError";
    this.details = details;
  }
}

export function toIpcError(error: unknown): IpcError {
  if (error instanceof DeskmateCommandError) {
    return error.details;
  }
  if (
    typeof error === "object" &&
    error !== null &&
    "category" in error &&
    "message" in error &&
    typeof error.category === "string" &&
    typeof error.message === "string"
  ) {
    return error as IpcError;
  }
  return {
    category: "internal",
    message: error instanceof Error ? error.message : String(error),
  };
}

function draftPayload(config: AppConfig): { json: string } {
  return { json: JSON.stringify(config) };
}

export const SESSION_REQUIRED_MESSAGE =
  "This browser is not signed in. Enter the admin token to continue.";

/** The harness can be asked to render the signed-out state with `?scenario=signedout`. */
function signedOutScenario(): boolean {
  return new URLSearchParams(window.location.search).get("scenario") === "signedout";
}

export function isSessionMissing(error: IpcError): boolean {
  return error.category === "runtime-unavailable" && error.message === SESSION_REQUIRED_MESSAGE;
}

let mockSignedIn = !signedOutScenario();

export function signIn(adminToken: string): Promise<void> {
  if (adminToken.trim() === "") {
    return Promise.reject(
      new DeskmateCommandError({
        category: "runtime-unavailable",
        message: SESSION_REQUIRED_MESSAGE,
      }),
    );
  }
  mockSignedIn = true;
  return Promise.resolve();
}

export function getAppSnapshot(): Promise<AppSnapshot> {
  if (!mockSignedIn) {
    return Promise.reject(
      new DeskmateCommandError({
        category: "runtime-unavailable",
        message: SESSION_REQUIRED_MESSAGE,
      }),
    );
  }
  return mockInvoke("get_app_snapshot");
}

export function validateConfigDraft(config: AppConfig): Promise<DraftValidation> {
  return mockInvoke("validate_config_draft", { draft: draftPayload(config) });
}

export function saveApplyConfig(config: AppConfig): Promise<ConfigApplyResult> {
  return mockInvoke("save_apply_config", { draft: draftPayload(config) });
}

export function saveServerConfig(config: AppConfig): Promise<ConfigApplyResult> {
  return mockInvoke("save_server_config", { request: { draft: draftPayload(config) } });
}

export function getNetworkSettings(): Promise<NetworkSettings> {
  return mockInvoke("get_network_settings");
}

export function setServerEndpoint(
  serverUrl: string,
  deviceId: string,
  adminToken: string,
): Promise<NetworkSettings> {
  return mockInvoke("set_server_endpoint", {
    request: { server_url: serverUrl, device_id: deviceId, admin_token: adminToken },
  });
}

// Deliberately absent, matching `src/lib/backendClient.ts`: provisioning,
// factory reset, the local-ownership switch and autostart. A harness that offered
// them would let the UI be developed against affordances the shipped app does not
// have -- the same class of divergence that made the WKWebView focus defect
// invisible in Chrome.

export function resumePushing(): Promise<void> {
  return mockInvoke("resume_pushing");
}

export function controlPomodoro(cardId: string, action: PomodoroAction): Promise<void> {
  return mockInvoke("control_pomodoro", { target: { card_id: cardId }, action });
}

export function renderCardPreview(cardId: string): Promise<PreviewFrame> {
  return mockInvoke("render_card_preview", { cardId });
}

export function mintImageSource(name: string, faceKind?: string): Promise<MintedImageSource> {
  return mockInvoke("mint_image_source", { sourceName: name, faceKind: faceKind ?? null });
}

export function listCreatableFaces(): Promise<FaceDescriptor[]> {
  return mockInvoke("list_creatable_faces");
}

export function listImageSources(): Promise<ImageSourceDescriptor[]> {
  return mockInvoke("list_image_sources");
}

export function updateImageSourceFace(
  sourceId: string,
  fields: Record<string, string>,
): Promise<FaceDescriptor> {
  return mockInvoke("update_image_source_face", {
    request: { source_id: sourceId, fields },
  });
}

export function listenToAppState(onSnapshot: (snapshot: AppSnapshot) => void): Promise<() => void> {
  return Promise.resolve(mockListen(onSnapshot));
}
