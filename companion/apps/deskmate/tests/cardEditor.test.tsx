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
  FaceStatus,
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

const opaqueFace: FaceDescriptor = {
  kind: "opaque-server-face",
  label: "Source settings",
  fields: [
    { key: "place", label: "Place", type: "text", value: "Dubai", placeholder: "Dubai" },
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

/** Mounts the editor on a picture card whose source answers with `face` and `status`. */
async function mountFaceSettings(face: FaceDescriptor, status: FaceStatus | null) {
  const picture = pictureCard();
  let current = face;
  let currentStatus = status;
  const saves: Record<string, string>[] = [];
  backendMocks.imageSourcesImpl = async () => [
    { id: picture.source_id, name: "Claude limits", face: current, face_status: currentStatus },
  ];
  backendMocks.updateImageSourceFaceImpl = async (_sourceId, fields) => {
    saves.push(fields);
    current = {
      ...current,
      fields: current.fields.map((field) => ({
        ...field,
        value: fields[field.key] ?? field.value,
      })),
    };
    return current;
  };
  const mounted = await mount();
  await act(async () =>
    mounted.root.render(
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
  await waitFor(() => expect(mounted.container.textContent).toContain(face.label));
  const type = async (placeholder: string, value: string) => {
    const input = mounted.container.querySelector<HTMLInputElement>(
      `input[placeholder="${placeholder}"]`,
    );
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, value);
      input?.dispatchEvent(new Event("input", { bubbles: true }));
    });
    return input;
  };
  return {
    ...mounted,
    saves,
    type,
    setStatus: (next: FaceStatus | null) => {
      currentStatus = next;
    },
    cleanup: async () => {
      await mounted.cleanup();
      backendMocks.imageSourcesImpl = async () => [];
      backendMocks.updateImageSourceFaceImpl = async () => {
        throw new Error("updateImageSourceFace not configured for this test");
      };
    },
  };
}

test("face settings save themselves: there is no second save button to forget", async () => {
  // The defect this pins: a separate "Save source settings" button beside the window's
  // "Save to server" meant the prominent one silently discarded a typed coin ID. The
  // face stayed blank, the panel said "Waiting for the first picture", the window said
  // "Saved to the server".
  const editor = await mountFaceSettings(opaqueFace, null);
  try {
    expect(buttonWithText(editor.container, "Save source settings")).toBeUndefined();

    const place = await editor.type("Dubai", "Berlin");
    expect(editor.saves).toEqual([]);
    // Leaving the field is what clicking the window's own save does first.
    await act(async () => place?.dispatchEvent(new FocusEvent("focusout", { bubbles: true })));
    await waitFor(() => expect(editor.saves).toEqual([{ place: "Berlin" }]));

    // A choice has no "leaving": it saves at once, and only what changed.
    await act(async () => buttonWithText(editor.container, "Imperial")?.click());
    await waitFor(() => expect(editor.saves).toEqual([{ place: "Berlin" }, { units: "imperial" }]));
    expect(editor.container.textContent).not.toContain(opaqueFace.kind);
  } finally {
    await editor.cleanup();
  }
});

test("an emptied required field is an unfinished form, not a request", async () => {
  const editor = await mountFaceSettings(opaqueFace, null);
  try {
    const place = await editor.type("Dubai", "   ");
    await act(async () => place?.dispatchEvent(new FocusEvent("focusout", { bubbles: true })));
    await act(async () => {});
    expect(editor.saves).toEqual([]);
  } finally {
    await editor.cleanup();
  }
});

test("the window says why a face is not drawing, in the server's own words", async () => {
  const blank: FaceDescriptor = {
    kind: "token",
    label: "Token price",
    fields: [
      { key: "coin_id", label: "Coin ID", type: "text", value: "", placeholder: "solana" },
      { key: "currency", label: "Currency", type: "text", value: "usd", placeholder: "usd" },
    ],
  };
  const editor = await mountFaceSettings(blank, {
    state: "needs-settings",
    message: null,
    at_unix_seconds: null,
  });
  try {
    expect(editor.container.textContent).toContain("Fill in Coin ID to start this face.");

    // A ticker where an id belongs: the server reports it, and saving re-reads the status.
    editor.setStatus({
      state: "needs-attention",
      message: "the token was not found; check the coin ID",
      at_unix_seconds: 1_790_000_000,
    });
    const coin = await editor.type("solana", "SOL");
    await act(async () => coin?.dispatchEvent(new FocusEvent("focusout", { bubbles: true })));
    await waitFor(() =>
      expect(editor.container.querySelector('[role="alert"]')?.textContent).toBe(
        "The token was not found; check the coin ID.",
      ),
    );
  } finally {
    await editor.cleanup();
  }
});

test("a drawn face and a retrying one are statuses, not alarms", async () => {
  const editor = await mountFaceSettings(opaqueFace, {
    state: "retrying",
    message: "api.coingecko.com returned HTTP 429",
    at_unix_seconds: 1_790_000_000,
  });
  try {
    const status = editor.container.querySelector('.source-settings [role="status"]');
    expect(status?.textContent).toContain("api.coingecko.com returned HTTP 429.");
    expect(status?.textContent).toContain("keeps the last frame");
    expect(editor.container.querySelector('.source-settings [role="alert"]')).toBeNull();
  } finally {
    await editor.cleanup();
  }
});

test("an external picture producer adds no settings form", async () => {
  const picture = pictureCard();
  backendMocks.imageSourcesImpl = async () => [
    { id: picture.source_id, name: "Claude limits", face: null, face_status: null },
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

test("a tappable face tells the owner what a tap does, and others say nothing", async () => {
  // The panel has three states for a picture -- waiting, stale, drawn -- and none of
  // them can say "this one answers a tap". The window is where that is learnable.
  const tappable = await mountFaceSettings(
    { ...opaqueFace, tap: "Tap the panel for the next stories." },
    null,
  );
  try {
    await waitFor(() =>
      expect(tappable.container.textContent).toContain("Tap the panel for the next stories."),
    );
    // And it must REPLACE the gesture note's claim, not sit beside it: a picture
    // card's document action is always `none`, so the editor used to say a tap did
    // nothing while the panel paged the front page.
    expect(tappable.container.textContent).not.toContain("Tapping this card does nothing.");
  } finally {
    await tappable.cleanup();
  }

  const quiet = await mountFaceSettings(opaqueFace, null);
  try {
    expect(quiet.container.textContent).not.toContain("Tap the panel");
    await waitFor(() =>
      expect(quiet.container.textContent).toContain("Tapping this card does nothing."),
    );
  } finally {
    await quiet.cleanup();
  }
});
