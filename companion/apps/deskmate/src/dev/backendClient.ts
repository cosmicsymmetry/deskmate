/**
 * Dev-only stand-in for `src/lib/backendClient.ts`.
 *
 * Enabled by `VITE_DESKMATE_MOCK=1`, which makes `vite.config.ts` alias
 * `./backendClient` at this module. No application code imports anything from
 * `src/dev/`, so a production build resolves the real HTTP client and never
 * reaches this file.
 *
 * Scenario state and behavior live in `mockBackend`; this client adds the
 * browser session guard and sign-in-before-selection ordering.
 */

import type { ApiError, AppSnapshot, NetworkSettings } from "../lib/types";
import { mockGetAppSnapshot, selectMockDevice } from "./mockBackend";

import { DeskmateApiError, SESSION_REQUIRED_MESSAGE } from "../lib/apiErrors";

export { DeskmateApiError, SESSION_REQUIRED_MESSAGE, isSessionMissing } from "../lib/apiErrors";

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
  listenToAppState,
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
  return mockGetAppSnapshot();
}

export function signInAndSelectDevice(
  deviceId: string,
  adminToken: string,
): Promise<NetworkSettings> {
  return signIn(adminToken).then(() => selectMockDevice(deviceId));
}

// Deliberately absent, matching `src/lib/backendClient.ts`: provisioning,
// factory reset, the local-ownership switch and autostart. The harness must not
// expose affordances the shipped web app cannot perform.
