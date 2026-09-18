/**
 * Dev-only stand-in for `src/lib/backendClient.ts`.
 *
 * Enabled by `VITE_DESKMATE_MOCK=1`, which makes `vite.config.ts` alias
 * `./backendClient` at this module. No application code imports anything from
 * `src/dev/`, so a production build resolves the real HTTP client and never
 * reaches this file.
 *
 * Every function here forwards to `mockBackend`'s operation dispatcher. Keeping
 * scenario state and behavior in that one dispatcher prevents the mock client
 * from becoming a second implementation of the harness.
 */

import type {
  AppConfig,
  AppSnapshot,
  ConfigApplyResult,
  DraftValidation,
  FaceDescriptor,
  ImageSourceDescriptor,
  ApiError,
  MintedImageSource,
  NetworkSettings,
  PomodoroAction,
  PreviewFrame,
} from "../lib/types";
import { dispatchMockOperation, mockListen } from "./mockBackend";

export class DeskmateApiError extends Error {
  readonly details: ApiError;

  constructor(details: ApiError) {
    super(details.message);
    this.name = "DeskmateApiError";
    this.details = details;
  }
}

export function toApiError(error: unknown): ApiError {
  if (error instanceof DeskmateApiError) {
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
    return error as ApiError;
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

export function isSessionMissing(error: ApiError): boolean {
  return error.category === "runtime-unavailable" && error.message === SESSION_REQUIRED_MESSAGE;
}

let mockSignedIn = !signedOutScenario();

export function signIn(adminToken: string): Promise<void> {
  if (adminToken.trim() === "") {
    return Promise.reject(
      new DeskmateApiError({
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
      new DeskmateApiError({
        category: "runtime-unavailable",
        message: SESSION_REQUIRED_MESSAGE,
      }),
    );
  }
  return dispatchMockOperation("get_app_snapshot");
}

export function validateConfigDraft(config: AppConfig): Promise<DraftValidation> {
  return dispatchMockOperation("validate_config_draft", { draft: draftPayload(config) });
}

export function saveConfig(config: AppConfig): Promise<ConfigApplyResult> {
  return dispatchMockOperation("save_config", { draft: draftPayload(config) });
}

export function getNetworkSettings(): Promise<NetworkSettings> {
  return dispatchMockOperation("get_network_settings");
}

export function signInAndSelectDevice(
  deviceId: string,
  adminToken: string,
): Promise<NetworkSettings> {
  return signIn(adminToken).then(() =>
    dispatchMockOperation("sign_in_and_select_device", {
      request: { device_id: deviceId },
    }),
  );
}

// Deliberately absent, matching `src/lib/backendClient.ts`: provisioning,
// factory reset, the local-ownership switch and autostart. The harness must not
// expose affordances the shipped web app cannot perform.

export function resumePushing(): Promise<void> {
  return dispatchMockOperation("resume_pushing");
}

export function controlPomodoro(cardId: string, action: PomodoroAction): Promise<void> {
  return dispatchMockOperation("control_pomodoro", { target: { card_id: cardId }, action });
}

export function renderCardPreview(cardId: string): Promise<PreviewFrame> {
  return dispatchMockOperation("render_card_preview", { cardId });
}

export function mintImageSource(name: string, faceKind?: string): Promise<MintedImageSource> {
  return dispatchMockOperation("mint_image_source", {
    sourceName: name,
    faceKind: faceKind ?? null,
  });
}

export function listCreatableFaces(): Promise<FaceDescriptor[]> {
  return dispatchMockOperation("list_creatable_faces");
}

export function listImageSources(): Promise<ImageSourceDescriptor[]> {
  return dispatchMockOperation("list_image_sources");
}

export function updateImageSourceFace(
  sourceId: string,
  fields: Record<string, string>,
): Promise<FaceDescriptor> {
  return dispatchMockOperation("update_image_source_face", {
    request: { source_id: sourceId, fields },
  });
}

export function listenToAppState(onSnapshot: (snapshot: AppSnapshot) => void): Promise<() => void> {
  return Promise.resolve(mockListen(onSnapshot));
}
