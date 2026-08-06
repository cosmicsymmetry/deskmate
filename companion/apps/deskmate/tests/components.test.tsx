import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";

import { App } from "../src/App";
import { CardEditor } from "../src/components/CardEditor";
import { CardList } from "../src/components/CardList";
import { DeviceHeader } from "../src/components/DeviceHeader";
import { DevicePreview } from "../src/components/DevicePreview";
import { Filmstrip } from "../src/components/Filmstrip";
import { ProviderStatus, formatProviderAge } from "../src/components/ProviderStatus";
import type {
  AppConfig,
  CardDataSnapshot,
  CardField,
  CardSettings,
  DisplayTemplate,
  ValidationIssue,
} from "../src/lib/types";
import { ipcContractFixtures } from "../src/lib/types.contract";

const snapshot = ipcContractFixtures.snapshot;
const cards = snapshot.config.cards;

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

  function renderPreview({
    cardId,
    cardData,
  }: {
    cardId: string;
    cardData: CardDataSnapshot[];
  }): string {
    return renderToStaticMarkup(
      <DevicePreview
        cards={[calendarCard(cardId, { kind: "in-rotation", dwell_seconds: null })]}
        selectedWidgetId={cardId}
        cardData={cardData}
        pomodoros={[]}
        orientation="landscape"
      />,
    );
  }

  test("renders a real pushed value on the preview face", () => {
    const cardData: CardDataSnapshot[] = [
      {
        card_id: "upnext",
        fields: [{ key: "row0_title", value: { kind: "text", value: "Q3 Planning Sync" } }],
      },
    ];
    const html = renderPreview({ cardId: "upnext", cardData });
    expect(html).toContain("Q3 Planning Sync");
    expect(html).not.toMatch(/sample/i);
  });

  test("a card with no published data yet is clearly labelled as a sample", () => {
    const html = renderPreview({ cardId: "upnext", cardData: [] });
    expect(html).toMatch(/sample/i);
  });

  // `renderPreview` above is pinned to a calendar/row-list card, so the extended
  // templates (which are not calendar-shaped) get their own helper rather than
  // overloading that one's signature. It reuses the same `weatherCard`/`clockCard`
  // fixtures already defined in this file and drives `DevicePreview` the same way:
  // one card, selected, with `cardData` standing in for the runtime's last-published
  // snapshot for that card id.
  function renderTemplatePreview({
    template,
    fields,
    base,
  }: {
    template: DisplayTemplate;
    fields: CardField[];
    base?: CardSettings;
  }): string {
    const card: CardSettings = { ...(base ?? weatherCard("template-preview")), template };
    return renderToStaticMarkup(
      <DevicePreview
        cards={[card]}
        selectedWidgetId={card.id}
        cardData={fields.length > 0 ? [{ card_id: card.id, fields }] : []}
        pomodoros={[]}
        orientation="landscape"
      />,
    );
  }

  test("big-number-label preview shows the value as the hero", () => {
    const html = renderTemplatePreview({
      template: { kind: "big-number-label" },
      fields: [
        { key: "title", value: { kind: "text", value: "Downloads" } },
        { key: "value", value: { kind: "text", value: "1,204" } },
        { key: "label", value: { kind: "text", value: "this week" } },
      ],
    });
    expect(html).toContain("1,204");
    expect(html).toContain("this week");
    expect(html).toContain("Downloads");
  });

  test("big-number-label falls back to a placeholder when value is absent", () => {
    const html = renderTemplatePreview({ template: { kind: "big-number-label" }, fields: [] });
    expect(html).toContain("--");
  });

  test("icon-badge-text preview shows icon, badge, value and label", () => {
    const html = renderTemplatePreview({
      template: { kind: "icon-badge-text", icon_asset_id: null },
      fields: [
        { key: "icon", value: { kind: "text", value: "cloud-sun" } },
        { key: "badge", value: { kind: "text", value: "Berlin" } },
        { key: "value", value: { kind: "text", value: "21°" } },
        { key: "label", value: { kind: "text", value: "Partly cloudy" } },
      ],
    });
    expect(html).toContain("Berlin");
    expect(html).toContain("21°");
    expect(html).toContain("Partly cloudy");
    expect(html).toContain("preview-icon--cloud-sun");
  });

  test("an unrecognised icon name renders the unknown icon", () => {
    const html = renderTemplatePreview({
      template: { kind: "icon-badge-text", icon_asset_id: null },
      fields: [{ key: "icon", value: { kind: "text", value: "meteor" } }],
    });
    expect(html).toContain("preview-icon--unknown");
  });

  test("analog-clock preview renders a face with hands", () => {
    const html = renderTemplatePreview({
      template: { kind: "analog-clock" },
      fields: [],
      base: clockCard("analog-preview", { kind: "in-rotation", dwell_seconds: null }),
    });
    expect(html).toContain("preview-analog-face");
    expect(html).toContain("preview-analog-hand--hour");
    expect(html).toContain("preview-analog-hand--minute");
  });

  test("renders a useful empty state and keeps the panel's narrower claim", () => {
    const populated = renderToStaticMarkup(
      <DevicePreview
        cards={cards}
        selectedWidgetId="clock"
        cardData={snapshot.card_data}
        pomodoros={snapshot.pomodoros}
        orientation="landscape"
      />,
    );
    expect(populated).toContain("Same data as your display · approximate pixels");
    // Not pixel-perfect, and no longer claims to be — just an honest label.
    expect(populated).not.toContain("Layout preview · not pixel-identical");

    const empty = renderToStaticMarkup(
      <DevicePreview
        cards={[]}
        selectedWidgetId={null}
        cardData={[]}
        pomodoros={[]}
        orientation="landscape-flipped"
      />,
    );
    expect(empty).toContain("The standalone clock stays available.");
    expect(empty).toContain("is-flipped");
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

  test("includes narrow-window and reduced-motion fallbacks", async () => {
    const css = await Bun.file(new URL("../src/styles.css", import.meta.url)).text();
    expect(css).toContain("@media (max-width: 430px)");
    expect(css).toContain("@media (prefers-reduced-motion: reduce)");
    expect(css).toContain("animation-duration: 0.001ms");
  });
});
