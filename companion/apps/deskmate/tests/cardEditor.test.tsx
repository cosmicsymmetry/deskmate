import { afterEach, beforeEach, expect, test } from "bun:test";
import { act, useState } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { CardEditor } from "../src/components/CardEditor";
import { CardList } from "../src/components/CardList";
import { LoopRing } from "../src/components/LoopRing";

import type {
  AppConfig,
  CardError,
  CardSettings,
  FaceDescriptor,
  MintedImageSource,
  ValidationIssue,
} from "../src/lib/types";

import { backendMocks, resetBackendMocks } from "./support/backendMock";
import { cards, clockCard, pictureCard, cardListConfig } from "./support/fixtures";
import { installDomLifecycle, waitFor, buttonWithText } from "./support/dom";
import { installHttpLifecycle } from "./support/http";

beforeEach(resetBackendMocks);
const { mount, cleanupMountedRoots } = installDomLifecycle();
installHttpLifecycle(cleanupMountedRoots);
afterEach(resetBackendMocks);

function renderCardEditor(
  card: CardSettings,
  issues: ValidationIssue[] = [],
  cardError: CardError | null = null,
  pictureAccess: MintedImageSource | null = null,
) {
  return renderToStaticMarkup(
    <CardEditor
      card={card}
      config={cardListConfig([card])}
      issues={issues}
      cardError={cardError}
      pomodoro={null}
      timerBusy={false}
      pictureAccess={pictureAccess}
      onChange={() => {}}
      onConfigChange={() => {}}
      onRemove={() => {}}
      onTimerAction={() => {}}
    />,
  );
}

test("no card offers a Name field, and the pomodoro keeps its timer label", () => {
  // A clock is a clock and a picture is named by its source. "Timer label" is
  // different: it is editable and drawn on the panel face.
  const clockHtml = renderCardEditor(clockCard("clock-1", "Desk"));
  expect(clockHtml).not.toContain("<span>Name</span>");
  expect(renderCardEditor(pictureCard())).not.toContain("<span>Name</span>");
  const pomodoro: CardSettings = {
    kind: "pomodoro",
    id: "pomodoro-1",
    label: "Focus",
    duration_seconds: 1500,
    template: { kind: "progress-ring" },
    tap_action: { kind: "start-pause" },
    refresh: { kind: "device-local" },
    alert: { kind: "none" },
    dwell_seconds: null,
  };
  expect(renderCardEditor(pomodoro)).toContain("<span>Timer label</span>");
});

test("cardIdentity names every visible surface without a template label", () => {
  // Control labels still carry template and title together deliberately: a screen reader hearing
  // "Remove Digital clock — Desk" is better served than by "Remove Desk".
  const config = cardListConfig([clockCard("internal-uuid-0001", "Desk")]);
  const library = renderToStaticMarkup(
    <CardList
      config={config}
      issues={[]}
      pomodoros={[]}
      selectedCardId="internal-uuid-0001"
      onSelect={() => {}}
      onAdd={() => {}}
      onChange={() => {}}
      onRemove={() => {}}
    />,
  );
  const loop = renderToStaticMarkup(
    <LoopRing
      config={config}
      issues={[]}
      selectedCardId={null}
      onSelect={() => {}}
      onChange={() => {}}
    />,
  );
  const editor = renderCardEditor(clockCard("internal-uuid-0001", "Desk"));

  // One name, the same on all three surfaces. "Desk" is a frozen creation
  // default rather than an editable identity, so it is not displayed.
  expect(library).not.toContain('class="tile-label">Digital clock<');
  expect(library).toContain('<strong class="card-tile__value numeral">Clock</strong>');
  expect(library).not.toContain('class="card-tile__name">Desk<');
  expect(loop).toContain('class="loop__entry-name">Clock<');
  expect(editor).toContain('id="editor-heading">Clock<');
  // The accessible name keeps the template: a listener gets no tile to look at.
  expect(library).toContain('aria-label="Move Digital clock — Clock earlier"');
  expect(library).toContain('aria-label="Remove Digital clock — Clock');
});

test("a picture card states its source instead of offering a menu of them", () => {
  // Rendering identity as a select would imply that changing it is a safe edit,
  // when a new source makes this a different card.
  const html = renderCardEditor(pictureCard());

  expect(html).toContain("Picture source");
  expect(html).toContain("Claude limits");
  expect(html).toContain("limits-source");
  expect(html).not.toContain("<select");
});

test("a picture card whose source no longer exists says so rather than silently renaming", () => {
  const orphan = { ...pictureCard(), source_id: "deleted-source" };
  const html = renderToStaticMarkup(
    <CardEditor
      card={orphan}
      config={cardListConfig([pictureCard()])}
      issues={[]}
      cardError={null}
      pomodoro={null}
      timerBusy={false}
      pictureAccess={null}
      onChange={() => {}}
      onConfigChange={() => {}}
      onRemove={() => {}}
      onTimerAction={() => {}}
    />,
  );

  expect(html).toContain("deleted-source · Missing source");
});

test("a picture editor shows source access once, immediately after minting", () => {
  const picture = pictureCard();
  const access: MintedImageSource = {
    source_id: picture.source_id,
    token: "plaintext-once",
    push_url: "https://desk.example/v1/images/plaintext-once",
  };
  const firstRender = renderCardEditor(picture, [], null, access);

  expect(firstRender).toContain('id="editor-heading">Claude limits<');
  expect(firstRender).toContain("Claude limits");
  expect(firstRender).toContain(picture.source_id);
  expect(firstRender).toContain(access.push_url);
  expect(firstRender).toContain(access.token);
  expect(firstRender).toContain("Copy token");
  expect(firstRender).toContain("This plaintext token is shown once.");

  const laterRender = renderCardEditor(picture);
  expect(laterRender).not.toContain(access.token);
  expect(laterRender).not.toContain("Copy token");
  expect(laterRender).not.toContain("This plaintext token is shown once.");
});

test("a picture editor renders and saves server-described fields without face-specific logic", async () => {
  const picture = pictureCard();
  const descriptor: FaceDescriptor = {
    kind: "opaque-server-face",
    label: "Source settings",
    fields: [
      {
        key: "place",
        label: "Place",
        type: "text",
        value: "Dubai",
        placeholder: "Dubai",
      },
      {
        key: "units",
        label: "Units",
        type: "enum",
        value: "metric",
        options: [
          { value: "metric", label: "Metric" },
          { value: "imperial", label: "Imperial" },
        ],
      },
    ],
  };
  backendMocks.imageSourcesImpl = async () => [
    { id: picture.source_id, name: "Claude limits", face: descriptor },
  ];
  let savedFields: Record<string, string> | null = null;
  backendMocks.updateImageSourceFaceImpl = async (_sourceId, fields) => {
    savedFields = fields;
    return {
      ...descriptor,
      fields: descriptor.fields.map((field) => ({
        ...field,
        value: fields[field.key] ?? field.value,
      })),
    };
  };
  const { container, root, cleanup } = await mount();
  try {
    await act(async () =>
      root.render(
        <CardEditor
          card={picture}
          config={cardListConfig([picture])}
          issues={[]}
          cardError={null}
          pomodoro={null}
          timerBusy={false}
          onChange={() => {}}
          onConfigChange={() => {}}
          onRemove={() => {}}
          onTimerAction={() => {}}
        />,
      ),
    );
    await waitFor(() => expect(container.textContent).toContain("Source settings"));
    const place = container.querySelector<HTMLInputElement>('input[placeholder="Dubai"]');
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(
        place,
        "Berlin",
      );
      place?.dispatchEvent(new Event("input", { bubbles: true }));
    });
    const imperial = buttonWithText(container, "Imperial");
    await act(async () => imperial?.click());
    await act(async () => buttonWithText(container, "Save source settings")?.click());

    await waitFor(() => expect(savedFields).toEqual({ place: "Berlin", units: "imperial" }));
    expect(container.textContent).toContain("Saved on the server");
    expect(container.textContent).not.toContain(descriptor.kind);
  } finally {
    await cleanup();
    backendMocks.imageSourcesImpl = async () => [];
    backendMocks.updateImageSourceFaceImpl = async () => {
      throw new Error("updateImageSourceFace not configured for this test");
    };
  }
});

test("an external picture producer adds no settings form", async () => {
  const picture = pictureCard();
  backendMocks.imageSourcesImpl = async () => [
    { id: picture.source_id, name: "Claude limits", face: null },
  ];
  const { container, root, cleanup } = await mount();
  try {
    await act(async () =>
      root.render(
        <CardEditor
          card={picture}
          config={cardListConfig([picture])}
          issues={[]}
          cardError={null}
          pomodoro={null}
          timerBusy={false}
          onChange={() => {}}
          onConfigChange={() => {}}
          onRemove={() => {}}
          onTimerAction={() => {}}
        />,
      ),
    );
    await waitFor(() => expect(container.textContent).not.toContain("Loading source settings"));
    expect(container.textContent).not.toContain("Save source settings");
  } finally {
    await cleanup();
    backendMocks.imageSourcesImpl = async () => [];
  }
});

test("never shows the wire id", () => {
  // A distinctive id with no overlap with any visible label (unlike the
  // fixture's plain "clock", which is also a substring of the visible
  // "Digital clock" kind name and would make this assertion meaningless).
  const clock = clockCard("internal-uuid-0001", "Desk");
  const html = renderCardEditor(clock);
  expect(html).toContain("Show seconds");
  // IDs are wire identifiers, not something a person should see or edit.
  expect(html).not.toContain("Widget ID");
  expect(html).not.toContain("internal-uuid-0001");
});

test("the card editor shows no fixed canvas dimension caption", () => {
  const html = renderCardEditor(clockCard("clock-1"));
  expect(html).not.toContain("448");
});

test("pomodoro cards offer their timer-finish alert controls", () => {
  const pomodoro = cards.find((card) => card.kind === "pomodoro");
  if (!pomodoro) {
    throw new Error("contract fixture is missing its pomodoro card");
  }
  expect(renderCardEditor(pomodoro)).toContain("Take over the screen when the timer ends");
});

test("states the card's tap gesture and the shared alert-dismiss behaviour", () => {
  const clock = cards.find((card) => card.kind === "clock");
  if (!clock) {
    throw new Error("contract fixture is missing its clock widget");
  }
  const html = renderCardEditor(clock);
  expect(html).toContain("Tapping this card does nothing.");
  expect(html).toContain("Swiping the screen moves through the loop.");
  expect(html).toContain("a tap dismisses it");
});

test("the selected card's timed-loop dwell field writes the card's own dwell", async () => {
  const card = clockCard("clock-1", "Desk");
  const initial = cardListConfig([card]);
  let latest = initial;

  function Harness() {
    const [config, setConfig] = useState(initial);
    latest = config;
    return (
      <CardEditor
        card={card}
        config={config}
        issues={[
          {
            path: "cards[0].dwell_seconds",
            code: "out-of-range",
            message: "Entry dwell is out of range.",
          },
        ]}
        cardError={null}
        pomodoro={null}
        timerBusy={false}
        onChange={() => {}}
        onConfigChange={setConfig}
        onRemove={() => {}}
        onTimerAction={() => {}}
      />
    );
  }

  const { container, root, cleanup } = await mount();
  try {
    await act(async () => root.render(<Harness />));
    const dwell = container.querySelector<HTMLInputElement>(
      'input[aria-label="Stays on the panel for Clock"]',
    );
    expect(dwell?.placeholder).toBe("20 s, the loop's default");
    expect(dwell?.getAttribute("aria-invalid")).toBe("true");
    expect(container.textContent).toContain("Entry dwell is out of range.");
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(dwell, "45");
      dwell?.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(latest.cards[0].dwell_seconds).toBe(45);
  } finally {
    await cleanup();
  }
});

test("the dwell field is shown when the loop is timed or the value must be repaired", () => {
  const card = clockCard("clock-1", "Desk");
  const issue: ValidationIssue = {
    path: "cards[0].dwell_seconds",
    code: "out-of-range",
    message: "Entry dwell is out of range.",
  };
  const renderDwell = (config: AppConfig, issues: ValidationIssue[] = []) =>
    renderToStaticMarkup(
      <CardEditor
        card={config.cards[0]}
        config={config}
        issues={issues}
        cardError={null}
        pomodoro={null}
        timerBusy={false}
        onChange={() => {}}
        onConfigChange={() => {}}
        onRemove={() => {}}
        onTimerAction={() => {}}
      />,
    );

  // Manual advance with no value set: nothing to show.
  const manualNullConfig = cardListConfig([card]);
  manualNullConfig.advance = { kind: "manual" };
  expect(renderDwell(manualNullConfig)).not.toContain("Stays on the panel for");

  // Manual advance with an out-of-range value: shown so it can be repaired.
  const manualInvalidConfig = structuredClone(manualNullConfig);
  manualInvalidConfig.cards[0].dwell_seconds = 2;
  const manualInvalid = renderDwell(manualInvalidConfig, [issue]);
  expect(manualInvalid).toContain("Stays on the panel for");
  expect(manualInvalid).toContain('aria-invalid="true"');
  expect(manualInvalid).toContain("Dwell applies when the loop is timed.");

  const timed = renderDwell(cardListConfig([card]));
  expect(timed).toContain("Stays on the panel for");
});
