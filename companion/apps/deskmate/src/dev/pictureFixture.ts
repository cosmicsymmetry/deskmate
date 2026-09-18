/**
 * Dev-only picture-card fixtures. The card has a named reusable source but no
 * credential: the plaintext token is available only in the render immediately
 * after the mock mint operation, exactly as it is in the real app.
 */
import type { AppConfig, CardSettings } from "../lib/types";
import { mockConfig } from "./fixture";

function pictureCard(): CardSettings {
  return {
    kind: "picture",
    id: "usage-picture",
    title: "Claude limits",
    source_id: "claude-limits",
    tap_action: { kind: "none" },
    refresh: { kind: "manual" },
    alert: { kind: "none" },
    dwell_seconds: null,
  };
}

export function mockPictureConfig(): AppConfig {
  return {
    ...mockConfig(),
    cards: [pictureCard()],
    image_sources: [{ id: "claude-limits", name: "Claude usage" }],
  };
}
