import { afterEach, beforeEach, expect, spyOn, test } from "bun:test";
import { act, useState, type ComponentProps } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { CardList } from "../src/components/CardList";
import { addCard } from "../src/lib/configDraft";

import type { AppConfig, ValidationIssue } from "../src/lib/types";

import { resetBackendMocks } from "./support/backendMock";
import { clockCard, pictureCard, pomodoroCard, cardListConfig } from "./support/fixtures";
import { installDomLifecycle } from "./support/dom";
import { installHttpLifecycle } from "./support/http";

beforeEach(resetBackendMocks);
const { mount, cleanupMountedRoots } = installDomLifecycle();
installHttpLifecycle(cleanupMountedRoots);
afterEach(resetBackendMocks);

function cardListDefaults(): Omit<ComponentProps<typeof CardList>, "config"> {
  return {
    issues: [],
    pomodoros: [],
    selectedCardId: null,
    onSelect: () => {},
    onAdd: () => {},
    onChange: () => {},
    onRemove: () => {},
  };
}

test("an untitled card is not labelled with its template twice", () => {
  // The quiet line is the owner's words, so it is absent rather than a repeat of
  // the label sitting directly above it.
  const config = cardListConfig([clockCard("internal-uuid-0002", "")]);
  const library = renderToStaticMarkup(<CardList {...cardListDefaults()} config={config} />);
  expect(library).toContain("Digital clock");
  expect(library).not.toContain("card-tile__name");
});

test("a picture tile leads with its source and names every entry control", () => {
  const picture = pictureCard();
  const html = renderToStaticMarkup(
    <CardList
      {...cardListDefaults()}
      config={cardListConfig([picture])}
      selectedCardId={picture.id}
    />,
  );

  expect(html).not.toContain('class="tile-label">Picture<');
  // The tile's one fact is the source; generic format and kind labels would not
  // distinguish one picture from another.
  expect(html).toContain('<strong class="card-tile__value numeral">Claude limits</strong>');
  expect(html).not.toContain('numeral">PNG<');
  // No second line: a picture's title is its source name, said once.
  expect(html).not.toContain('class="card-tile__name"');
  expect(html).toContain('aria-label="Move Picture — Claude limits earlier"');
  expect(html).toContain('aria-label="Remove Picture — Claude limits');
});

test("picture tiles preserve missing and empty source identities", () => {
  const missing = { ...pictureCard("missing-picture"), source_id: "missing-source" };
  const missingHtml = renderToStaticMarkup(
    <CardList
      {...cardListDefaults()}
      config={{ ...cardListConfig([missing]), image_sources: [] }}
    />,
  );
  expect(missingHtml).toContain('<strong class="card-tile__value numeral">Missing source</strong>');

  const empty = pictureCard("empty-source-name");
  const emptyHtml = renderToStaticMarkup(
    <CardList
      {...cardListDefaults()}
      config={{
        ...cardListConfig([empty]),
        image_sources: [{ id: empty.source_id, name: "" }],
      }}
    />,
  );
  expect(emptyHtml).toContain('<strong class="card-tile__value numeral"></strong>');
  expect(emptyHtml).not.toContain('numeral">Missing source</strong>');
});

test("pomodoro tiles use duration fallback and floor live remaining time, including zero", () => {
  const fallback = pomodoroCard("fallback", "Focus");
  const live = pomodoroCard("live", "Live");
  const zero = pomodoroCard("zero", "Zero");
  const html = renderToStaticMarkup(
    <CardList
      {...cardListDefaults()}
      config={cardListConfig([fallback, live, zero])}
      pomodoros={[
        { card_id: live.id, state: "running", duration_seconds: 1500, remaining_seconds: 61.9 },
        { card_id: zero.id, state: "completed", duration_seconds: 1500, remaining_seconds: 0 },
      ]}
    />,
  );

  expect(html).toContain('numeral">25:00</strong>');
  expect(html).toContain('numeral">1:01</strong>');
  expect(html).toContain('numeral">0:00</strong>');
});

test("pomodoro tiles suppress blank labels and labels equal to the countdown", () => {
  const blank = pomodoroCard("blank", "");
  const repeated = pomodoroCard("repeated", "25:00");
  const html = renderToStaticMarkup(
    <CardList {...cardListDefaults()} config={cardListConfig([blank, repeated])} />,
  );

  expect(html.match(/class="card-tile__value numeral">25:00<\/strong>/g)).toHaveLength(2);
  expect(html).not.toContain("card-tile__name");
});

test("a tile does not print the source name twice when the title repeats it", () => {
  const named = { ...pictureCard(), title: "Claude limits" };
  const html = renderToStaticMarkup(
    <CardList {...cardListDefaults()} config={cardListConfig([named])} selectedCardId={named.id} />,
  );

  expect(html).toContain('numeral">Claude limits</strong>');
  expect(html).not.toContain("card-tile__name");
});

test("the add menu creates a picture through its own server action", async () => {
  let pictureAdds = 0;
  const { container, root, cleanup } = await mount();
  try {
    await act(async () =>
      root.render(
        <CardList
          {...cardListDefaults()}
          config={cardListConfig([clockCard("clock", "Desk")])}
          onAddPicture={() => {
            pictureAdds += 1;
          }}
        />,
      ),
    );
    await act(async () => container.querySelector<HTMLButtonElement>(".card-tile__add")?.click());
    expect(container.textContent).toContain("Pictures");
    const items = [...container.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')];
    const picture = items.find((button) => button.textContent?.includes("New picture source"));
    expect(picture).toBeDefined();
    const beforePicture = items.at(-2);

    beforePicture?.focus();
    await act(async () =>
      beforePicture?.dispatchEvent(
        new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }),
      ),
    );
    expect<Element | null | undefined>(document.activeElement).toBe(picture);

    await act(async () => picture?.click());
    expect(pictureAdds).toBe(1);
    expect(container.querySelector('[role="menu"]')).toBeNull();
  } finally {
    await cleanup();
  }
});

test("a server-drawn face is not filed under Built in", async () => {
  // Server-drawn faces produce picture cards, so filing them beside device-local
  // kinds would contradict the tile, editor heading, and `cardLabel` result.
  const { container, root, cleanup } = await mount();
  try {
    await act(async () =>
      root.render(
        <CardList
          {...cardListDefaults()}
          config={cardListConfig([])}
          creatableFaces={[{ kind: "weather", label: "Weather", fields: [] }]}
          onAddFace={() => {}}
        />,
      ),
    );
    await act(async () => container.querySelector<HTMLButtonElement>(".card-tile__add")?.click());

    const groups = [...container.querySelectorAll(".menu__group")].map((group) => ({
      legend: group.querySelector("legend")?.textContent?.trim() ?? "",
      items: [...group.querySelectorAll('[role="menuitem"]')].map(
        (item) => item.querySelector("strong")?.textContent?.trim() ?? "",
      ),
    }));

    const builtIn = groups.find((group) => group.legend === "Built in");
    const serverSide = groups.find((group) => group.legend === "Server-side");
    expect(builtIn).toBeDefined();
    expect(serverSide).toBeDefined();
    expect(builtIn?.items).toContain("Digital clock");
    expect(builtIn?.items).not.toContain("Weather");
    expect(serverSide?.items).toEqual(["Weather"]);
  } finally {
    await cleanup();
  }
});

test("the add menu offers the faces the server says it can draw", async () => {
  // The app does not know what weather is: it renders whatever the server
  // listed, so a fourth face appears here with no app change at all.
  let chosen: { kind: string; label: string } | null = null;
  const { container, root, cleanup } = await mount();
  try {
    await act(async () =>
      root.render(
        <CardList
          {...cardListDefaults()}
          config={cardListConfig([])}
          creatableFaces={[
            { kind: "weather", label: "Weather", fields: [] },
            { kind: "sunrise", label: "Sunrise", fields: [] },
          ]}
          onAddFace={(kind, label) => {
            chosen = { kind, label };
          }}
        />,
      ),
    );
    await act(async () => container.querySelector<HTMLButtonElement>(".card-tile__add")?.click());
    const menu = container.querySelector('[role="menu"]');
    expect(menu?.textContent).toContain("Weather");
    // A face this app has never heard of still reaches the menu.
    expect(menu?.textContent).toContain("Sunrise");

    const weatherItem = [
      ...(menu?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]') ?? []),
    ].find((button) => button.textContent?.includes("Weather"));
    await act(async () => weatherItem?.click());
    expect<{ kind: string; label: string } | null>(chosen).toEqual({
      kind: "weather",
      label: "Weather",
    });
    expect(container.querySelector('[role="menu"]') === null).toBe(true);
  } finally {
    await cleanup();
  }
});

test("the add menu offers an unused picture source and omits one already in the loop", async () => {
  const used = { id: "used-source", name: "Already shown" };
  const unused = { id: "unused-source", name: "Bring this back" };
  const config: AppConfig = {
    ...cardListConfig([{ ...pictureCard(), source_id: used.id }]),
    image_sources: [used, unused],
  };
  let chosenSource: string | null = null;
  const { container, root, cleanup } = await mount();
  try {
    await act(async () =>
      root.render(
        <CardList
          {...cardListDefaults()}
          config={config}
          onAddPicture={(source) => {
            chosenSource = source?.id ?? null;
          }}
        />,
      ),
    );
    await act(async () => container.querySelector<HTMLButtonElement>(".card-tile__add")?.click());
    const menu = container.querySelector('[role="menu"]');
    expect(menu?.textContent).toContain(unused.name);
    expect(menu?.textContent).not.toContain(used.name);
    expect(menu?.textContent).toContain("New picture source");

    const unusedItem = [
      ...(menu?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]') ?? []),
    ].find((button) => button.textContent?.includes(unused.name));
    await act(async () => unusedItem?.click());
    expect<string | null>(chosenSource).toBe(unused.id);
    expect(container.querySelector('[role="menu"]')).toBeNull();
  } finally {
    await cleanup();
  }
});

test("add-menu keyboard navigation crosses groups, wraps, and skips a disabled source action", async () => {
  const config: AppConfig = {
    ...cardListConfig([]),
    image_sources: Array.from({ length: 8 }, (_, index) => ({
      id: `source-${index}`,
      name: `Source ${index}`,
    })),
  };
  const { container, root, cleanup } = await mount();
  try {
    await act(async () =>
      root.render(
        <CardList
          {...cardListDefaults()}
          config={config}
          creatableFaces={[{ kind: "weather", label: "Weather", fields: [] }]}
          onAddFace={() => {}}
        />,
      ),
    );
    await act(async () => container.querySelector<HTMLButtonElement>(".card-tile__add")?.click());
    const items = [...container.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')];
    const item = (label: string) =>
      items.find((button) => button.querySelector("strong")?.textContent === label);
    const press = async (button: HTMLButtonElement | undefined, key: string) => {
      await act(async () =>
        button?.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true })),
      );
    };

    expect(items.map((button) => button.querySelector("strong")?.textContent)).toEqual([
      "Digital clock",
      "Pomodoro",
      "Weather",
      "Source 0",
      "Source 1",
      "Source 2",
      "Source 3",
      "Source 4",
      "Source 5",
      "Source 6",
      "Source 7",
      "New picture source",
    ]);
    expect(item("New picture source")?.disabled).toBe(true);

    item("Digital clock")?.focus();
    await press(item("Digital clock"), "ArrowDown");
    expect<Element | null | undefined>(document.activeElement).toBe(item("Pomodoro"));
    await press(item("Pomodoro"), "ArrowDown");
    expect<Element | null | undefined>(document.activeElement).toBe(item("Weather"));
    await press(item("Weather"), "ArrowDown");
    expect<Element | null | undefined>(document.activeElement).toBe(item("Source 0"));

    item("Source 7")?.focus();
    await press(item("Source 7"), "ArrowDown");
    expect<Element | null | undefined>(document.activeElement).toBe(item("Digital clock"));
    await press(item("Digital clock"), "ArrowUp");
    expect<Element | null | undefined>(document.activeElement).toBe(item("Source 7"));
  } finally {
    await cleanup();
  }
});

test("the grid renders container issues and scopes a card's own issues to its tile", () => {
  const config = cardListConfig([pomodoroCard("first", "Desk"), pomodoroCard("second", "Up next")]);
  const containerIssue: ValidationIssue = {
    path: "cards",
    code: "out-of-range",
    message: "The loop entries need attention.",
  };
  const secondDwellIssue: ValidationIssue = {
    path: "cards[1].dwell_seconds",
    code: "out-of-range",
    message: "The second dwell is out of range.",
  };
  const container = document.createElement("div");
  container.innerHTML = renderToStaticMarkup(
    <CardList
      {...cardListDefaults()}
      config={config}
      issues={[containerIssue, secondDwellIssue]}
    />,
  );

  expect(container.textContent).toContain("The loop entries need attention.");
  const firstTile = [...container.querySelectorAll<HTMLLIElement>(".card-tile")].find((tile) =>
    tile.textContent?.includes("Desk"),
  );
  const secondTile = [...container.querySelectorAll<HTMLLIElement>(".card-tile")].find((tile) =>
    tile.textContent?.includes("Up next"),
  );
  expect(firstTile?.classList.contains("has-issue")).toBe(false);
  expect(firstTile?.textContent).not.toContain("The second dwell is out of range.");
  expect(secondTile?.classList.contains("has-issue")).toBe(true);
  expect(secondTile?.textContent).toContain("The second dwell is out of range.");
});

test("the add slot is disabled at capacity and names the limiting contract", () => {
  const config = cardListConfig(
    Array.from({ length: 8 }, (_, index) => clockCard(`card-${index}`)),
  );
  const html = renderToStaticMarkup(<CardList {...cardListDefaults()} config={config} />);
  expect(html).toContain('class="card-tile__add"');
  expect(html).toContain('aria-describedby="add-card-capacity"');
  expect(html).toContain('disabled=""');
  expect(html).toContain("The limit is 8 cards.");
});

test("the add menu aligns to the slot end only when it would overflow the viewport", async () => {
  const config = cardListConfig([clockCard("clock", "Desk")]);
  const { container, root, cleanup } = await mount();
  container.className = "face__work";
  // Measured against the viewport, not this column: the menu is `position:
  // fixed` precisely so the column's `overflow-y` cannot clip it.
  const originalClientWidth = Object.getOwnPropertyDescriptor(
    document.documentElement,
    "clientWidth",
  );
  Object.defineProperty(document.documentElement, "clientWidth", {
    configurable: true,
    value: 1000,
  });

  try {
    await act(async () => root.render(<CardList {...cardListDefaults()} config={config} />));
    const slotRoot = container.querySelector<HTMLLIElement>(".card-tile--add");
    const slot = container.querySelector<HTMLButtonElement>(".card-tile__add");
    if (!slotRoot) {
      throw new Error("add-card slot root was not rendered");
    }

    const slotAt = (left: number) =>
      ({ top: 300, bottom: 407, left, right: left + 148 }) as DOMRect;

    // 800 + 272 overflows 1000, so the menu hangs off its right edge instead.
    slotRoot.getBoundingClientRect = () => slotAt(800);
    await act(async () => slot?.click());
    expect(container.querySelector(".menu")?.classList.contains("menu--end")).toBe(true);

    await act(async () => slot?.click());
    slotRoot.getBoundingClientRect = () => slotAt(100);
    await act(async () => slot?.click());
    expect(container.querySelector(".menu")?.classList.contains("menu--end")).toBe(false);
  } finally {
    try {
      await cleanup();
    } finally {
      if (originalClientWidth)
        Object.defineProperty(document.documentElement, "clientWidth", originalClientWidth);
      else Reflect.deleteProperty(document.documentElement, "clientWidth");
    }
  }
});

/**
 * Opening the menu focuses its first item, and focusing something inside a
 * scrolling column makes the browser scroll it into view. A menu that closed
 * on scroll therefore closed itself the instant it opened — visible only as
 * "I picked a card and nothing happened", because by the time the pointer
 * arrived there was nothing under it. It repositions instead.
 */
/**
 * WebKit does not move focus to a `<button>` on mousedown — a macOS
 * convention Chrome does not share. Opening the menu focuses its first item,
 * so pressing the mouse on any entry blurred that item with a **null**
 * `relatedTarget`. Reading that as "focus left the menu" unmounted the menu
 * between mousedown and click, so the click never landed and no card of any
 * kind could be added. Focus going nowhere is not focus leaving; a click
 * genuinely outside is caught by the document mousedown listener instead.
 *
 * This is invisible to the browser harness, which runs in Chrome.
 */
test("a click inside the menu is not mistaken for focus leaving it", async () => {
  const config = cardListConfig([clockCard("clock", "Desk")]);
  const added: string[] = [];
  const { container, root, cleanup } = await mount();
  container.className = "face__work";

  try {
    await act(async () =>
      root.render(
        <CardList {...cardListDefaults()} config={config} onAdd={(kind) => added.push(kind)} />,
      ),
    );
    const slot = container.querySelector<HTMLButtonElement>(".card-tile__add");
    await act(async () => slot?.click());
    const firstItem = container.querySelector<HTMLButtonElement>('[role="menuitem"]');
    if (!firstItem) {
      throw new Error("the add menu rendered no items");
    }

    // What WebKit does when the mouse goes down on a menu item.
    await act(async () => {
      firstItem.dispatchEvent(new FocusEvent("focusout", { bubbles: true, relatedTarget: null }));
    });

    expect(container.querySelector(".menu")).not.toBeNull();

    await act(async () => firstItem.click());
    expect(added).toEqual(["clock"]);
  } finally {
    await cleanup();
  }
});

test("the add menu survives the scroll that opening it causes", async () => {
  const config = cardListConfig([clockCard("clock", "Desk")]);
  const { container, root, cleanup } = await mount();
  container.className = "face__work";

  try {
    await act(async () => root.render(<CardList {...cardListDefaults()} config={config} />));
    const slot = container.querySelector<HTMLButtonElement>(".card-tile__add");
    await act(async () => slot?.click());
    expect(container.querySelector(".menu")).not.toBeNull();

    await act(async () => {
      container.dispatchEvent(new Event("scroll", { bubbles: false }));
      window.dispatchEvent(new Event("scroll"));
    });

    expect(container.querySelector(".menu")).not.toBeNull();
  } finally {
    await cleanup();
  }
});

test("the add slot menu restores focus on Escape and a picked kind enrols and selects", async () => {
  const initial = cardListConfig([clockCard("clock", "Desk")]);
  let latest = initial;
  let selected: string | null = null;

  function Harness() {
    const [config, setConfig] = useState(initial);
    const [selectedCardId, setSelectedCardId] = useState<string | null>(null);
    latest = config;
    selected = selectedCardId;
    return (
      <CardList
        {...cardListDefaults()}
        config={config}
        selectedCardId={selectedCardId}
        onSelect={setSelectedCardId}
        onAdd={(kind) => {
          const result = addCard(config, kind);
          setConfig(result.config);
          setSelectedCardId(result.cardId);
        }}
        onChange={setConfig}
      />
    );
  }

  const { container, root, cleanup } = await mount();
  try {
    await act(async () => root.render(<Harness />));
    const slot = container.querySelector<HTMLButtonElement>(".card-tile__add");
    await act(async () => slot?.click());
    expect(slot?.getAttribute("aria-expanded")).toBe("true");
    expect(container.querySelector('[role="menu"]')).not.toBeNull();
    const menuText = container.querySelector('[role="menu"]')?.textContent;
    expect(menuText).toContain("Digital clock");
    expect(menuText).toContain("Pomodoro");
    expect(menuText).not.toContain("Weather");
    expect(menuText).not.toContain("Calendar");
    expect(menuText).not.toContain("RSS");

    const firstItem = container.querySelector<HTMLButtonElement>('[role="menuitem"]');
    await act(async () => {
      firstItem?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(container.querySelector('[role="menu"]')).toBeNull();
    expect(document.activeElement === slot).toBe(true);

    await act(async () => slot?.click());
    slot?.focus();
    await act(async () => {
      slot?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(container.querySelector('[role="menu"]')).toBeNull();
    expect(slot?.getAttribute("aria-expanded")).toBe("false");
    expect(document.activeElement === slot).toBe(true);

    await act(async () => slot?.click());
    const pomodoro = [...container.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(
      (button) => button.textContent?.includes("Pomodoro"),
    );
    await act(async () => pomodoro?.click());
    expect(latest.cards.at(-1)?.kind).toBe("pomodoro");
    expect(latest.cards.at(-1)?.id).toBe("pomodoro");
    expect<string | null>(selected).toBe("pomodoro");
    expect(container.querySelector('[role="menu"]')).toBeNull();
    expect(document.activeElement !== document.body).toBe(true);
    expect(document.activeElement?.classList.contains("card-tile__body")).toBe(true);
    // The tile is the countdown and the label now; the template is not printed.
    expect(document.activeElement?.textContent).toContain("Focus");
  } finally {
    await cleanup();
  }
});

test("tabbing focus outside the add slot closes its menu", async () => {
  const config = cardListConfig([clockCard("clock", "Desk")]);
  const { container, root, cleanup } = await mount();
  const outside = document.createElement("button");
  outside.textContent = "Outside the menu";
  document.body.append(container, outside);

  try {
    await act(async () => root.render(<CardList {...cardListDefaults()} config={config} />));
    const slot = container.querySelector<HTMLButtonElement>(".card-tile__add");
    await act(async () => slot?.click());
    container.querySelector<HTMLButtonElement>('[role="menuitem"]')?.focus();
    await act(async () => outside.focus());

    expect(container.querySelector('[role="menu"]')).toBeNull();
    expect(slot?.getAttribute("aria-expanded")).toBe("false");
  } finally {
    await cleanup();
    outside.remove();
  }
});

test("grid move buttons and Alt arrows update active-loop order", async () => {
  const config = cardListConfig([pomodoroCard("first", "Desk"), pomodoroCard("second", "Up next")]);
  let latest = config;
  const { container, root, cleanup } = await mount();

  function Harness() {
    const [value, setValue] = useState(config);
    latest = value;
    return <CardList {...cardListDefaults()} config={value} onChange={setValue} />;
  }

  try {
    await act(async () => root.render(<Harness />));
    expect(
      container.querySelector<HTMLButtonElement>(
        'button[aria-label="Move Pomodoro — Desk earlier"]',
      )?.disabled,
    ).toBe(true);
    expect(
      container.querySelector<HTMLButtonElement>(
        'button[aria-label="Move Pomodoro — Up next later"]',
      )?.disabled,
    ).toBe(true);
    const earlier = container.querySelector<HTMLButtonElement>(
      'button[aria-label="Move Pomodoro — Up next earlier"]',
    );
    expect(earlier).not.toBeNull();
    await act(async () => earlier?.click());
    expect(latest.cards.map((card) => card.id)).toEqual(["second", "first"]);

    const later = container.querySelector<HTMLButtonElement>(
      'button[aria-label="Move Pomodoro — Up next later"]',
    );
    await act(async () => later?.click());
    expect(latest.cards.map((card) => card.id)).toEqual(["first", "second"]);

    let selectedTile = [
      ...container.querySelectorAll<HTMLButtonElement>(".card-tile .card-tile__body"),
    ].find((button) => button.textContent?.includes("Desk"));
    expect(selectedTile).not.toBeNull();
    await act(async () => {
      selectedTile?.dispatchEvent(
        new KeyboardEvent("keydown", { key: "ArrowRight", altKey: true, bubbles: true }),
      );
    });
    expect(latest.cards.map((card) => card.id)).toEqual(["second", "first"]);

    selectedTile = [
      ...container.querySelectorAll<HTMLButtonElement>(".card-tile .card-tile__body"),
    ].find((button) => button.textContent?.includes("Desk"));
    await act(async () => {
      selectedTile?.dispatchEvent(
        new KeyboardEvent("keydown", { key: "ArrowLeft", altKey: true, bubbles: true }),
      );
    });
    expect(latest.cards.map((card) => card.id)).toEqual(["first", "second"]);
  } finally {
    await cleanup();
  }
});

test("keyboard reorder restores focus to the moved tile body", async () => {
  const config = cardListConfig([pomodoroCard("first", "Desk"), pomodoroCard("second", "Up next")]);
  const { container, root, cleanup } = await mount();

  function Harness() {
    const [value, setValue] = useState(config);
    return (
      <CardList
        {...cardListDefaults()}
        config={value}
        onChange={(next) => {
          (document.activeElement as HTMLElement | null)?.blur();
          setValue(next);
        }}
      />
    );
  }

  try {
    await act(async () => root.render(<Harness />));
    const firstTile = [...container.querySelectorAll<HTMLButtonElement>(".card-tile__body")].find(
      (button) => button.textContent?.includes("Desk"),
    );
    firstTile?.focus();
    await act(async () =>
      firstTile?.dispatchEvent(
        new KeyboardEvent("keydown", { key: "ArrowRight", altKey: true, bubbles: true }),
      ),
    );
    const movedTile = [...container.querySelectorAll<HTMLButtonElement>(".card-tile__body")].find(
      (button) => button.textContent?.includes("Desk"),
    );
    expect(movedTile?.textContent).toContain("Desk");
    expect(document.activeElement === movedTile).toBe(true);
  } finally {
    await cleanup();
  }
});

test("dropping a loop tile on itself does not emit a draft change", async () => {
  const config = cardListConfig([pomodoroCard("first", "Desk"), pomodoroCard("second", "Up next")]);
  const changes: AppConfig[] = [];
  const { container, root, cleanup } = await mount();

  try {
    await act(async () =>
      root.render(
        <CardList
          {...cardListDefaults()}
          config={config}
          onChange={(next) => changes.push(next)}
        />,
      ),
    );
    const firstTile = [
      ...container.querySelectorAll<HTMLLIElement>('.card-tile[draggable="true"]'),
    ].find((tile) => tile.textContent?.includes("Desk"));
    const secondTile = [
      ...container.querySelectorAll<HTMLLIElement>('.card-tile[draggable="true"]'),
    ].find((tile) => tile.textContent?.includes("Up next"));
    if (!firstTile || !secondTile) throw new Error("Both draggable tiles must be rendered");
    const transfer = new DataTransfer();
    transfer.setData("text/plain", "first");
    const drop = new Event("drop", { bubbles: true }) as DragEvent;
    Object.defineProperty(drop, "dataTransfer", { value: transfer });
    await act(async () => firstTile.dispatchEvent(drop));
    expect(changes).toHaveLength(0);

    const secondTransfer = new DataTransfer();
    secondTransfer.setData("text/plain", "first");
    const secondDrop = new Event("drop", { bubbles: true });
    Object.defineProperty(secondDrop, "dataTransfer", { value: secondTransfer });
    await act(async () => secondTile.dispatchEvent(secondDrop));
    expect(changes).toHaveLength(1);
    expect(changes[0].cards.map((card) => card.id)).toEqual(["second", "first"]);
  } finally {
    await cleanup();
  }
});

for (const activation of ["click", "Enter", " "]) {
  test(`server face ${JSON.stringify(activation)} closes the menu and restores focus once`, async () => {
    const choices: string[] = [];
    const { container } = await mount(
      <CardList
        {...cardListDefaults()}
        config={cardListConfig([])}
        creatableFaces={[{ kind: "sunrise", label: "Sunrise", fields: [] }]}
        onAddFace={(kind) => choices.push(kind)}
      />,
    );
    const add = container.querySelector<HTMLButtonElement>(".card-tile__add");
    if (!add) throw new Error("Missing add control");
    await act(async () => add.click());
    const focus = spyOn(add, "focus");
    try {
      const face = [...container.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(
        (item) => item.textContent?.includes("Sunrise"),
      );
      if (!face) throw new Error("Missing server face");
      await act(async () => {
        if (activation === "click") face.click();
        else {
          const event = new KeyboardEvent("keydown", {
            key: activation,
            bubbles: true,
            cancelable: true,
          });
          face.dispatchEvent(event);
          expect(event.defaultPrevented).toBe(true);
        }
      });
      expect(choices).toEqual(["sunrise"]);
      expect(container.querySelector('[role="menu"]') === null).toBe(true);
      expect(add.getAttribute("aria-expanded")).toBe("false");
      expect(document.activeElement).toBe(add);
      expect(focus).toHaveBeenCalledTimes(1);
    } finally {
      focus.mockRestore();
    }
  });
}

for (const operation of ["mint", "save"]) {
  test(`server faces are disabled and skipped by arrow navigation during ${operation}`, async () => {
    const choices: string[] = [];
    const renderList = (busy: boolean) => (
      <CardList
        {...cardListDefaults()}
        config={cardListConfig([])}
        pictureBusy={busy && operation === "mint"}
        saving={busy && operation === "save"}
        creatableFaces={[{ kind: "weather", label: "Weather", fields: [] }]}
        onAddFace={(kind) => choices.push(kind)}
      />
    );
    const { container, root } = await mount(renderList(false));
    await act(async () => container.querySelector<HTMLButtonElement>(".card-tile__add")?.click());
    await act(async () => root.render(renderList(true)));
    const items = [...container.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')];
    const face = items.find((item) => item.textContent?.includes("Weather"));
    expect(face?.disabled).toBe(true);
    await act(async () => face?.click());
    expect(choices).toEqual([]);
    items[1].focus();
    await act(async () =>
      items[1].dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true })),
    );
    expect(document.activeElement === items[0]).toBe(true);
    await act(async () =>
      items[0].dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true })),
    );
    expect(document.activeElement === items[1]).toBe(true);
  });
}
