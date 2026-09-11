/**
 * Dev-only picture-card fixtures. The card has a named reusable source but no
 * credential: the plaintext token is available only in the render immediately
 * after the mock mint command, exactly as it is in the real app.
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
  };
}

export function mockPictureConfig(): AppConfig {
  return {
    ...mockConfig(),
    cards: [pictureCard()],
    image_sources: [{ id: "claude-limits", name: "Claude usage" }],
    playlists: [
      {
        id: "day",
        name: "Workday",
        advance: { kind: "timed", default_dwell_seconds: 20 },
        entries: [{ card_id: "usage-picture", dwell_seconds: null }],
      },
    ],
    active_playlist_id: "day",
  };
}
