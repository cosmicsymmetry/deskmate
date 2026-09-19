import { describe, expect, test } from "bun:test";

import * as shared from "../src/lib/apiErrors";
import * as production from "../src/lib/backendClient";
import * as mock from "../src/dev/backendClient";
import type { ApiError } from "../src/lib/types";

describe("API error primitives", () => {
  test("both clients re-export the same constructor and session primitives", () => {
    expect(production.DeskmateApiError).toBe(shared.DeskmateApiError);
    expect(mock.DeskmateApiError).toBe(shared.DeskmateApiError);
    expect(production.isSessionMissing).toBe(shared.isSessionMissing);
    expect(mock.isSessionMissing).toBe(shared.isSessionMissing);
    const details: ApiError = { category: "not-found", message: "Missing" };
    expect(production.toApiError(new mock.DeskmateApiError(details))).toBe(details);
    expect(mock.toApiError(new production.DeskmateApiError(details))).toBe(details);
  });

  test("keeps constructor details, message, and name", () => {
    const details: ApiError = {
      category: "payload-too-large",
      message: "Too large",
      maximum_bytes: 65536,
    };
    for (const client of [production, mock]) {
      const error = new client.DeskmateApiError(details);
      expect(error).toBeInstanceOf(Error);
      expect(error.name).toBe("DeskmateApiError");
      expect(error.message).toBe(details.message);
      expect(error.details).toBe(details);
      expect(client.toApiError(error)).toBe(details);
    }
  });

  test("recognizes only the exact missing-session category and message", () => {
    const message = "This browser is not signed in. Enter the admin token to continue.";
    for (const client of [production, mock]) {
      expect(client.SESSION_REQUIRED_MESSAGE).toBe(message);
      expect(client.isSessionMissing({ category: "runtime-unavailable", message })).toBe(true);
      expect(client.isSessionMissing({ category: "internal", message })).toBe(false);
      expect(
        client.isSessionMissing({ category: "runtime-unavailable", message: `${message} ` }),
      ).toBe(false);
    }
  });

  test("production rejects unknown categories while the mock passes them through", () => {
    const unknown = { category: "future-category", message: "Future failure", extra: true };
    expect(production.toApiError(unknown)).toEqual({
      category: "internal",
      message: "[object Object]",
    });
    expect(mock.toApiError(unknown) === unknown).toBe(true);
  });

  test("preserves structural errors and normalizes unstructured failures", () => {
    const known: ApiError = { category: "validation", message: "Invalid", issues: [] };
    for (const client of [production, mock]) {
      expect(client.toApiError(known)).toBe(known);
      expect(client.toApiError(new Error("Failed"))).toEqual({
        category: "internal",
        message: "Failed",
      });
      expect(client.toApiError(null)).toEqual({ category: "internal", message: "null" });
      expect(client.toApiError({ category: 42, message: "Bad category" })).toEqual({
        category: "internal",
        message: "[object Object]",
      });
      expect(client.toApiError("Oops")).toEqual({ category: "internal", message: "Oops" });
    }
  });
});
