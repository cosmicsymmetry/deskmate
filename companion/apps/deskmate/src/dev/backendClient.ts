/**
 * Dev-only stand-in for `src/lib/backendClient.ts`.
 *
 * Enabled by `VITE_DESKMATE_MOCK=1`, which makes `vite.config.ts` alias
 * `./backendClient` at this module. No application code imports anything from
 * `src/dev/`, so a production build resolves the real HTTP client and never
 * reaches this file.
 *
 * Scenario state and behavior live in `mockBackend`; this client adds the
 * browser session guard shared by snapshot and account calls.
 */

import type { ApiError, AppSnapshot } from "../lib/types";
import {
  listenToAppState as mockListenToAppState,
  mockAccountRequest,
  mockGetAppSnapshot,
} from "./mockBackend";

import { DeskmateApiError, NO_PANELS_MESSAGE, SESSION_REQUIRED_MESSAGE } from "../lib/apiErrors";

export {
  DeskmateApiError,
  NO_PANELS_MESSAGE,
  SESSION_REQUIRED_MESSAGE,
  isNoPanels,
  isSessionMissing,
} from "../lib/apiErrors";

export {
  validateConfigDraft,
  saveConfig,
  getNetworkSettings,
  resumePushing,
  controlPomodoro,
  renderCardPreview,
  mintImageSource,
  listCreatableFaces,
  listImageSources,
  updateImageSourceFace,
} from "./mockBackend";

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

/** The harness can be asked to render the signed-out state with `?scenario=signedout`. */
function signedOutScenario(): boolean {
  return new URLSearchParams(window.location.search).get("scenario") === "signedout";
}

function noPanelsScenario(): boolean {
  return new URLSearchParams(window.location.search).get("scenario") === "nopanels";
}

let mockSignedIn = !signedOutScenario();

export async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const publicAccountCall =
    (method === "GET" && path === "/v1/app/instance") ||
    (method === "POST" &&
      ["/v1/app/setup", "/v1/app/auth/email", "/v1/app/auth/link"].includes(path));
  if (!mockSignedIn && path.startsWith("/v1/app/") && !publicAccountCall) {
    throw new DeskmateApiError({
      category: "runtime-unavailable",
      message: SESSION_REQUIRED_MESSAGE,
    });
  }
  const result = await mockAccountRequest(method, path, body);
  if (path === "/v1/app/setup" || path === "/v1/app/auth/link") mockSignedIn = true;
  if (
    (method === "DELETE" && path === "/v1/app/session") ||
    (method === "POST" && path === "/v1/app/sessions/revoke-all") ||
    (method === "DELETE" && path === "/v1/app/account")
  ) {
    mockSignedIn = false;
  }
  return result as T;
}

/**
 * The live snapshot stream is authenticated on the real server (`GET /v1/app/{id}/events`
 * answers 401 without a session), so the harness must not stream a signed-out page the
 * whole window either -- it did, which made `?scenario=signedout` render the loop.
 */
export function listenToAppState(handler: (payload: AppSnapshot) => void): Promise<() => void> {
  if (!mockSignedIn) {
    return Promise.reject(
      new DeskmateApiError({ category: "runtime-unavailable", message: SESSION_REQUIRED_MESSAGE }),
    );
  }
  if (noPanelsScenario()) {
    // With no panel there is no device id to subscribe to, so the real client never
    // opens a stream at all.
    return Promise.reject(
      new DeskmateApiError({ category: "not-found", message: NO_PANELS_MESSAGE }),
    );
  }
  return mockListenToAppState(handler);
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
  if (noPanelsScenario()) {
    return Promise.reject(
      new DeskmateApiError({ category: "not-found", message: NO_PANELS_MESSAGE }),
    );
  }
  return mockGetAppSnapshot();
}

// Deliberately absent, matching `src/lib/backendClient.ts`: provisioning,
// factory reset, the local-ownership switch and autostart. The harness must not
// expose affordances the shipped web app cannot perform.
