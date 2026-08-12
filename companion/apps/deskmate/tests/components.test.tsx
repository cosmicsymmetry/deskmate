import { describe, expect, mock, test } from "bun:test";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";

import { App } from "../src/App";
import { CardEditor } from "../src/components/CardEditor";
import { CardList } from "../src/components/CardList";
import { DeviceHeader } from "../src/components/DeviceHeader";
import { DevicePreview } from "../src/components/DevicePreview";
import { Filmstrip } from "../src/components/Filmstrip";
import { ProviderStatus, formatProviderAge } from "../src/components/ProviderStatus";
import {
  cardsContainerIssues,
  issuesForCard,
  issuesForPath,
  unclaimedIssues,
} from "../src/lib/configDraft";
import * as tauriModule from "../src/lib/tauri";
import type { AppConfig, CardSettings, PreviewFrame, ValidationIssue } from "../src/lib/types";
import { ipcContractFixtures } from "../src/lib/types.contract";

const snapshot = ipcContractFixtures.snapshot;
const cards = snapshot.config.cards;

// `DevicePreview` calls `renderCardPreview` (typed IPC over `tauri.ts`) directly, so
// its behaviour tests mock that one export at the module boundary — the same
// boundary every other command is mocked at when a test needs to control an IPC
// result — while every other export of `tauri.ts` (used transitively by `App` and
// `useAppState`) stays real.
let previewImpl: (cardId: string) => Promise<PreviewFrame> = () =>
  Promise.reject(new Error("renderCardPreview not configured for this test"));

mock.module("../src/lib/tauri", () => ({
  ...tauriModule,
  renderCardPreview: (cardId: string) => previewImpl(cardId),
}));

/// Renders into a live DOM root (unlike this file's other `renderToStaticMarkup`
/// tests) because `DevicePreview` fetches its frame in a `useEffect` — SSR never
/// runs effects, so these are the only tests here that need one. The extra
/// microtask turn after `render` lets the mocked `renderCardPreview` promise (and
/// the `setState` it drives) settle inside this `act` call rather than after it.
async function renderPreviewInto(root: Root, element: Parameters<Root["render"]>[0]) {
  await act(async () => {
    root.render(element);
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

/// Polls `assertion` inside `act`, so the passive-effect state updates the polling
/// itself waits out (rather than the initial `renderPreviewInto` settle) are also
/// attributed to an `act` scope.
async function waitFor(assertion: () => void, timeoutMs = 500): Promise<void> {
  const start = Date.now();
  for (;;) {
    try {
      assertion();
      return;
    } catch (error) {
      if (Date.now() - start > timeoutMs) {
        throw error;
      }
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 5));
      });
    }
  }
}

function clockCard(
  id: string,
  presence: CardSettings["presence"],
  title = `Card ${id}`,
): CardSettings {
  return {
    kind: "clock",
    id,
    title,
    show_seconds: true,
    template: { kind: "digital-clock" },
    tap_action: { kind: "none" },
    refresh: { kind: "device-local" },
    presence,
    alert:
      presence.kind === "alert-only"
        ? { kind: "on-timer-finish", hold: { kind: "until-dismissed" } }
        : { kind: "none" },
  };
}

function weatherCard(id: string, alert: CardSettings["alert"] = { kind: "none" }): CardSettings {
  return {
    kind: "weather",
    id,
    title: "Weather",
    location: "",
    units: "metric",
    template: { kind: "big-number-label" },
    tap_action: { kind: "none" },
    refresh: { kind: "interval", minutes: 30 },
    presence: { kind: "in-rotation", dwell_seconds: null },
    alert,
  };
}

/// Finds the self-closing `<input ...>` tag carrying both `name` and
/// `value`, regardless of where React's server renderer places `checked` /
/// `disabled` relative to the other attributes — that ordering is an
/// implementation detail (observed to differ from JSX source order), not
/// something a test should assume.
function radioTag(html: string, name: string, value: string): string | undefined {
  const inputs = html.match(/<input[^>]*>/g) ?? [];
  return inputs.find((tag) => tag.includes(`name="${name}"`) && tag.includes(`value="${value}"`));
}

function cardListConfig(cardList: CardSettings[]): AppConfig {
  return {
    schema_version: 3,
    preferences: { timezone: "UTC", autostart: false, paused: false, orientation: "landscape" },
    cards: cardList,
    assets: [],
    carousel: { advance: { kind: "timed", default_dwell_seconds: 20 } },
    updater: { channel: "stable", checks: "notify" },
  };
}

describe("settings accessibility and states", () => {
  test("renders a non-blocking loading state before the first backend snapshot", () => {
    const html = renderToStaticMarkup(<App />);
    expect(html).toContain("Opening your display settings");
    expect(html).toContain("background service keeps running");
  });

  function renderCardEditor(
    card: CardSettings,
    issues: ValidationIssue[] = [],
    isOnlyRotationCard = false,
  ) {
    return renderToStaticMarkup(
      <CardEditor
        card={card}
        issues={issues}
        pomodoro={null}
        defaultDwellSeconds={20}
        isOnlyRotationCard={isOnlyRotationCard}
        timerBusy={false}
        filePickerBusy={false}
        onChange={() => {}}
        onRemove={() => {}}
        onTimerAction={() => {}}
        onChooseCalendarFile={() => {}}
      />,
    );
  }

  test("explains the clean canvas and never shows the wire id", () => {
    // A distinctive id with no overlap with any visible label (unlike the
    // fixture's plain "clock", which is also a substring of the visible
    // "Digital clock" kind name and would make this assertion meaningless).
    const clock = clockCard(
      "internal-uuid-0001",
      { kind: "in-rotation", dwell_seconds: null },
      "Desk",
    );
    const html = renderCardEditor(clock);
    expect(html).toContain("Clean 448 × 368 canvas");
    expect(html).toContain("Show seconds");
    // IDs are wire identifiers, not something a person should see or edit.
    expect(html).not.toContain("Widget ID");
    expect(html).not.toContain("internal-uuid-0001");
  });

  test("offers a bounded native file chooser for local calendars", () => {
    const calendar = cards.find((card) => card.kind === "calendar");
    if (calendar?.kind !== "calendar") {
      throw new Error("contract fixture is missing its calendar widget");
    }
    const html = renderCardEditor({ ...calendar, source: { kind: "file", value: "" } });
    expect(html).toContain("Choose file…");
    expect(html).toContain("up to 1 MB");
  });

  test("alert-only is unavailable until an alert is configured", () => {
    const pomodoro = cards.find((card) => card.kind === "pomodoro");
    if (pomodoro?.kind !== "pomodoro") {
      throw new Error("contract fixture is missing its pomodoro widget");
    }
    const card: CardSettings = { ...pomodoro, alert: { kind: "none" } };
    const html = renderCardEditor(card);
    const radio = radioTag(html, "presence", "alert-only");
    expect(radio).not.toBeUndefined();
    expect(radio).toContain('disabled=""');
    expect(html).toContain("Turn on an alert below");
  });

  test("alert-only becomes available once an alert is configured", () => {
    const pomodoro = cards.find((card) => card.kind === "pomodoro");
    if (pomodoro?.kind !== "pomodoro") {
      throw new Error("contract fixture is missing its pomodoro widget");
    }
    const card: CardSettings = {
      ...pomodoro,
      alert: { kind: "on-timer-finish", hold: { kind: "until-dismissed" } },
    };
    const html = renderCardEditor(card);
    const radio = radioTag(html, "presence", "alert-only");
    expect(radio).not.toBeUndefined();
    expect(radio).not.toContain("disabled");
  });

  test("weather cards offer no alert controls", () => {
    const html = renderCardEditor(weatherCard("weather-1"));
    // The other four kinds have no trigger that could ever fire, so a
    // disabled alert control there would be noise — nothing renders at all.
    expect(html).not.toContain("alert-fieldset");
    expect(html).not.toMatch(/<legend>Alert<\/legend>/);
  });

  test("pomodoro and calendar cards do offer alert controls", () => {
    const pomodoro = cards.find((card) => card.kind === "pomodoro");
    const calendar = cards.find((card) => card.kind === "calendar");
    if (!pomodoro || !calendar) {
      throw new Error("contract fixture is missing pomodoro or calendar widgets");
    }
    expect(renderCardEditor(pomodoro)).toContain("Take over the screen when the timer ends");
    expect(renderCardEditor(calendar)).toContain("Take over the screen before an event");
  });

  test("states the card's tap gesture and the shared alert-dismiss behaviour", () => {
    const clock = cards.find((card) => card.kind === "clock");
    if (!clock) {
      throw new Error("contract fixture is missing its clock widget");
    }
    const html = renderCardEditor(clock);
    expect(html).toContain("Tapping this card does nothing.");
    expect(html).toContain("a tap dismisses it");
  });

  test("exposes explicit move buttons and keyboard instructions", () => {
    const config = cardListConfig([
      clockCard("first-clock-id", { kind: "in-rotation", dwell_seconds: 20 }),
      clockCard("second-clock-id", { kind: "in-rotation", dwell_seconds: null }),
    ]);
    const html = renderToStaticMarkup(
      <CardList
        config={config}
        issues={[]}
        selectedCardId="first-clock-id"
        onSelect={() => {}}
        onAdd={() => {}}
        onRemove={() => {}}
        onReorder={() => {}}
      />,
    );
    expect(html).toContain("⌥ ↑ ↓ to move");
    expect(html).toContain("Move Card second-clock-id up");
    expect(html).toContain("Move Card first-clock-id down");
  });

  test("the card list separates rotation from alerts and never shows ids", () => {
    const config = cardListConfig([
      clockCard("internal-uuid-0001", { kind: "in-rotation", dwell_seconds: 20 }, "Desk"),
      clockCard("internal-uuid-0002", { kind: "alert-only" }, "Focus"),
      clockCard("internal-uuid-0003", { kind: "off" }, "Spare"),
    ]);
    const html = renderToStaticMarkup(
      <CardList
        config={config}
        issues={[]}
        selectedCardId={null}
        onSelect={() => {}}
        onAdd={() => {}}
        onRemove={() => {}}
        onReorder={() => {}}
      />,
    );
    expect(html).toContain('aria-label="In rotation"');
    expect(html).toContain('aria-label="Alerts and muted"');
    expect(html).toContain("Desk");
    expect(html).toContain("Focus");
    expect(html).toContain("Spare");
    // Internal identifiers never reach the user — only their titles do.
    expect(html).not.toContain("internal-uuid-0001");
    expect(html).not.toContain("internal-uuid-0002");
    expect(html).not.toContain("internal-uuid-0003");
  });

  test("adding is disabled at the eight-card contract limit", () => {
    const config = cardListConfig(
      Array.from({ length: 8 }, (_, index) =>
        clockCard(`card-${index}`, { kind: "in-rotation", dwell_seconds: null }),
      ),
    );
    const html = renderToStaticMarkup(
      <CardList
        config={config}
        issues={[]}
        selectedCardId={null}
        onSelect={() => {}}
        onAdd={() => {}}
        onRemove={() => {}}
        onReorder={() => {}}
      />,
    );
    const addButtons = html.match(/<button class="add-card"[^>]*>[\s\S]*?<\/button>/g) ?? [];
    expect(addButtons.length).toBe(6);
    for (const button of addButtons) {
      expect(button).toContain('disabled=""');
      expect(button).toContain(">Add ");
    }
  });

  test("renders disconnected, protocol mismatch, stale, and last-good copy", () => {
    const mismatch = {
      ...snapshot,
      device: { ...snapshot.device, protocol_version: 2 as number },
    };
    const header = renderToStaticMarkup(
      <DeviceHeader
        snapshot={mismatch}
        commandError={null}
        busyAction={null}
        onTogglePause={() => {}}
      />,
    );
    expect(header).toContain("Display not connected");
    expect(header).toContain("supports protocol 1");

    const providers = renderToStaticMarkup(
      <ProviderStatus
        providers={snapshot.providers}
        cards={cards}
        refreshingId={null}
        onRefresh={() => {}}
      />,
    );
    expect(providers).toContain("Showing the last successful data");
    expect(providers).toContain("Refresh now");
  });

  function calendarCard(
    id: string,
    presence: CardSettings["presence"],
    title = "Up next",
  ): CardSettings {
    return {
      kind: "calendar",
      id,
      title,
      source: { kind: "url", value: "https://example.com/cal.ics" },
      template: { kind: "row-list" },
      tap_action: { kind: "none" },
      refresh: { kind: "interval", minutes: 15 },
      presence,
      alert: { kind: "none" },
    };
  }

  /// A live-mounted `DevicePreview` for one selected card, wired to the shared
  /// `previewImpl` mock above. `dataGeneration` defaults to 0 since most of these
  /// tests aren't exercising re-fetch-on-change.
  async function mountPreview(
    card: CardSettings,
    dataGeneration = 0,
  ): Promise<{ container: HTMLDivElement; root: Root }> {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    await renderPreviewInto(
      root,
      <DevicePreview
        cards={[card]}
        selectedWidgetId={card.id}
        orientation="landscape"
        dataGeneration={dataGeneration}
      />,
    );
    return { container, root };
  }

  test("renders the device's own pixels as an img with a data URL once the IPC resolves", async () => {
    previewImpl = async (cardId) => {
      expect(cardId).toBe("upnext");
      return { png_base64: "Zmlyc3QtZnJhbWU=", sample: false };
    };
    const { container, root } = await mountPreview(
      calendarCard("upnext", { kind: "in-rotation", dwell_seconds: null }),
    );
    await waitFor(() => {
      const img = container.querySelector("img");
      expect(img).not.toBeNull();
      expect(img?.getAttribute("src")).toBe("data:image/png;base64,Zmlyc3QtZnJhbWU=");
    });
    expect(container.querySelector(".preview-sample-badge")).toBeNull();
    root.unmount();
  });

  test("shows the explicit unavailable state when the IPC rejects, never a stale or invented frame", async () => {
    previewImpl = async () => {
      throw new Error("simulator init failed");
    };
    const { container, root } = await mountPreview(
      calendarCard("upnext", { kind: "in-rotation", dwell_seconds: null }),
    );
    await waitFor(() => {
      expect(container.textContent).toContain("Preview unavailable");
    });
    expect(container.querySelector("img")).toBeNull();
    root.unmount();
  });

  test("badges the frame as sample when the card has never published data", async () => {
    previewImpl = async () => ({ png_base64: "dW5jb25maWd1cmVk", sample: true });
    const { container, root } = await mountPreview(
      calendarCard("upnext", { kind: "in-rotation", dwell_seconds: null }),
    );
    await waitFor(() => {
      expect(container.querySelector("img")).not.toBeNull();
      expect(container.textContent).toContain("No data yet");
    });
    root.unmount();
  });

  test("re-requests the preview when dataGeneration bumps", async () => {
    let calls = 0;
    previewImpl = async () => {
      calls += 1;
      return { png_base64: `frame-${calls}`, sample: false };
    };
    const card = calendarCard("upnext", { kind: "in-rotation", dwell_seconds: null });
    const { container, root } = await mountPreview(card, 0);
    await waitFor(() => expect(calls).toBe(1));

    await renderPreviewInto(
      root,
      <DevicePreview
        cards={[card]}
        selectedWidgetId={card.id}
        orientation="landscape"
        dataGeneration={1}
      />,
    );
    await waitFor(() => expect(calls).toBe(2));
    await waitFor(() => {
      expect(container.querySelector("img")?.getAttribute("src")).toBe(
        "data:image/png;base64,frame-2",
      );
    });
    root.unmount();
  });

  test("keeps a stale frame on screen while a superseding request is in flight, then replaces it", async () => {
    // Two cards, so switching `selectedWidgetId` (not `dataGeneration`) is what
    // triggers the re-request here — the effect depends on both.
    let resolveSecond!: (frame: PreviewFrame) => void;
    let requestCount = 0;
    previewImpl = async (cardId) => {
      requestCount += 1;
      if (cardId === "first") {
        return { png_base64: "first-frame", sample: false };
      }
      return new Promise((resolve) => {
        resolveSecond = resolve;
      });
    };
    const cardA = calendarCard("first", { kind: "in-rotation", dwell_seconds: null });
    const cardB = calendarCard("second", { kind: "in-rotation", dwell_seconds: null });
    const { container, root } = await mountPreview(cardA);
    await waitFor(() => {
      expect(container.querySelector("img")?.getAttribute("src")).toBe(
        "data:image/png;base64,first-frame",
      );
    });

    await renderPreviewInto(
      root,
      <DevicePreview
        cards={[cardA, cardB]}
        selectedWidgetId="second"
        orientation="landscape"
        dataGeneration={0}
      />,
    );
    await waitFor(() => expect(requestCount).toBe(2));
    // The old frame is still the last thing painted — no blank flash, no stale claim
    // beyond what was already shown, since the component never clears `frame` itself.
    expect(container.querySelector("img")?.getAttribute("src")).toBe(
      "data:image/png;base64,first-frame",
    );

    resolveSecond({ png_base64: "second-frame", sample: false });
    await waitFor(() => {
      expect(container.querySelector("img")?.getAttribute("src")).toBe(
        "data:image/png;base64,second-frame",
      );
    });
    root.unmount();
  });

  // Regression test: `preview.rs`'s latest-wins coalescing replies to a superseded
  // job with `Err("superseded")` *before* the winning job even starts rendering, so
  // an older request's rejection can land in the browser after a newer request from
  // the *same* effect invocation has already succeeded. This only happens within one
  // effect invocation (a prop-driven re-render already fully guards the old promise
  // via `cancelled`, set synchronously during React's cleanup) — the live-templates
  // 1 Hz interval is the one in-component path that issues a second request before
  // the first has settled, so this test drives that interval deterministically
  // instead of waiting on a real 1 s timer: it captures the handler `DevicePreview`
  // passes to `window.setInterval` and invokes it itself.
  test("an older superseded rejection landing after a newer success does not flip the panel to unavailable", async () => {
    let capturedTick: (() => void) | undefined;
    const originalSetInterval = window.setInterval;
    const originalClearInterval = window.clearInterval;
    window.setInterval = ((handler: () => void) => {
      capturedTick = handler;
      return 0;
    }) as unknown as typeof window.setInterval;
    window.clearInterval = (() => {}) as unknown as typeof window.clearInterval;

    try {
      let rejectFirst!: (error: unknown) => void;
      let resolveSecond!: (frame: PreviewFrame) => void;
      let callCount = 0;
      previewImpl = () => {
        callCount += 1;
        if (callCount === 1) {
          return new Promise((_resolve, reject) => {
            rejectFirst = reject;
          });
        }
        return new Promise((resolve) => {
          resolveSecond = resolve;
        });
      };

      // `clock` is a live template, so `DevicePreview` registers the 1 Hz interval
      // this test drives by hand.
      const clock = clockCard("clock-preview", { kind: "in-rotation", dwell_seconds: null });
      const { container, root } = await mountPreview(clock);
      await waitFor(() => expect(callCount).toBe(1));
      expect(capturedTick).not.toBeUndefined();

      // Fire the "interval tick" ourselves: the second, superseding request.
      await act(async () => {
        capturedTick?.();
        await new Promise((resolve) => setTimeout(resolve, 0));
      });
      await waitFor(() => expect(callCount).toBe(2));

      // The newer request succeeds first...
      await act(async () => {
        resolveSecond({ png_base64: "winning-frame", sample: false });
        await new Promise((resolve) => setTimeout(resolve, 0));
      });
      await waitFor(() => {
        expect(container.querySelector("img")?.getAttribute("src")).toBe(
          "data:image/png;base64,winning-frame",
        );
      });

      // ...then the older, now-superseded request's rejection lands. It must not
      // flip the panel to "Preview unavailable" over the already-correct frame.
      await act(async () => {
        rejectFirst(new Error("superseded"));
        await new Promise((resolve) => setTimeout(resolve, 20));
      });
      expect(container.textContent).not.toContain("Preview unavailable");
      expect(container.querySelector("img")?.getAttribute("src")).toBe(
        "data:image/png;base64,winning-frame",
      );

      root.unmount();
    } finally {
      window.setInterval = originalSetInterval;
      window.clearInterval = originalClearInterval;
    }
  });

  test("shows 'No cards configured' when there are no cards, never a stale or invented frame", () => {
    // No live effect needed here: with no cards, `DevicePreview` never calls the
    // preview IPC at all, so a static render is enough to check the empty state.
    const html = renderToStaticMarkup(
      <DevicePreview
        cards={[]}
        selectedWidgetId={null}
        orientation="landscape-flipped"
        dataGeneration={0}
      />,
    );
    expect(html).toContain("No cards configured");
    expect(html).not.toContain("<img");
  });

  function filmstripConfig(): AppConfig {
    return {
      schema_version: 3,
      preferences: { timezone: "UTC", autostart: false, paused: false, orientation: "landscape" },
      cards: [
        calendarCard("first", { kind: "in-rotation", dwell_seconds: 45 }, "Desk"),
        calendarCard("second", { kind: "in-rotation", dwell_seconds: 20 }, "Up next"),
        calendarCard("third", { kind: "alert-only" }, "Focus"),
      ],
      assets: [],
      carousel: { advance: { kind: "timed", default_dwell_seconds: 20 } },
      updater: { channel: "stable", checks: "notify" },
    };
  }

  function renderFilmstrip(config: AppConfig): string {
    return renderToStaticMarkup(
      <Filmstrip
        config={config}
        issues={[]}
        selectedCardId="first"
        onSelect={() => {}}
        onReorder={() => {}}
        onChangeAdvance={() => {}}
      />,
    );
  }

  test("the filmstrip shows the loop length and only in-rotation cards", () => {
    const html = renderFilmstrip(filmstripConfig());
    expect(html).toMatch(/1 min 5 s/);
    expect(html).toContain("Desk");
    expect(html).toContain("Up next");
    expect(html).not.toContain("Focus");
  });

  test("the filmstrip hides timings and the play control under manual advance", () => {
    const config = filmstripConfig();
    const html = renderFilmstrip({
      ...config,
      carousel: { advance: { kind: "manual" } },
    });
    expect(html).not.toMatch(/\d+s</);
    expect(html).not.toContain(">Play<");
    expect(html).toContain("Desk");
  });

  test("formats provider staleness without exposing raw timestamps", () => {
    expect(formatProviderAge(null)).toBe("No successful refresh yet");
    expect(formatProviderAge(30)).toBe("Updated just now");
    expect(formatProviderAge(120)).toBe("Updated 2 min ago");
    expect(formatProviderAge(7200)).toBe("Updated 2 hr ago");
  });

  test("an issue on a path no card, preference, or carousel surface claims (e.g. a missing-capability issue) is not silently dropped", () => {
    const config = cardListConfig([
      clockCard("only-card", { kind: "in-rotation", dwell_seconds: null }),
    ]);
    const capabilityIssue: ValidationIssue = {
      path: "device.capabilities",
      code: "requires-capability",
      message:
        "the connected firmware does not support extended templates. Update the firmware, or remove the cards and settings that need it.",
    };
    expect(unclaimedIssues([capabilityIssue], config)).toEqual([capabilityIssue]);
  });

  test("every issue is claimed by exactly one surface — the cards container, a card, a preference field, the carousel, or the unclaimed fallback", () => {
    const config = cardListConfig([
      clockCard("first", { kind: "in-rotation", dwell_seconds: null }),
      clockCard("second", { kind: "in-rotation", dwell_seconds: null }),
    ]);
    const issues: ValidationIssue[] = [
      { path: "cards", code: "empty", message: "At least one card must be in the rotation." },
      { path: "cards[0].title", code: "too-long", message: "Title is too long." },
      { path: "cards[1]", code: "invalid-composition", message: "This card is misconfigured." },
      { path: "preferences.timezone", code: "invalid-timezone", message: "Unknown timezone." },
      {
        path: "carousel.advance.default_dwell_seconds",
        code: "out-of-range",
        message: "Dwell time is out of range.",
      },
      {
        path: "device.capabilities",
        code: "requires-capability",
        message: "the connected firmware does not support extended templates.",
      },
    ];

    // Reconstructed independently from the same public helpers each real surface calls
    // (App scopes CardList/CardEditor/the timezone field/the Filmstrip this exact way —
    // see App.tsx), plus the fallback under test. This is the invariant that actually
    // guards against the class of bug this fix addresses: if a surface's claim and
    // `unclaimedIssues`'s notion of "claimed" ever drift apart, an issue either goes
    // missing from every surface (as `device.capabilities` originally did) or gets
    // double-rendered — either way this union stops matching `issues` one-to-one.
    const union = [
      ...cardsContainerIssues(issues),
      ...config.cards.flatMap((card) => issuesForCard(issues, config, card.id)),
      ...issuesForPath(issues, "preferences.timezone"),
      ...issuesForPath(issues, "carousel.advance.default_dwell_seconds"),
      ...unclaimedIssues(issues, config),
    ];

    expect(union).toHaveLength(issues.length);
    expect(new Set(union)).toEqual(new Set(issues));
  });

  test("includes narrow-window and reduced-motion fallbacks", async () => {
    const css = await Bun.file(new URL("../src/styles.css", import.meta.url)).text();
    expect(css).toContain("@media (max-width: 430px)");
    expect(css).toContain("@media (prefers-reduced-motion: reduce)");
    expect(css).toContain("animation-duration: 0.001ms");
  });
});
