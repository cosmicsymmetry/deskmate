import type { ApiError } from "./types";

export class DeskmateApiError extends Error {
  readonly details: ApiError;

  constructor(details: ApiError) {
    super(details.message);
    this.name = "DeskmateApiError";
    this.details = details;
  }
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
 * Exported because the page has to tell the two apart: every other failure is
 * answered by retrying, and this one is answered by signing in. Asking the
 * module that constructed the error keeps that knowledge in one place instead of
 * spreading a string comparison through the UI.
 */
export function isSessionMissing(error: ApiError): boolean {
  return error.category === "runtime-unavailable" && error.message === SESSION_REQUIRED_MESSAGE;
}
