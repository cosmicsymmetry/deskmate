import { afterEach, beforeEach, expect, spyOn, test } from "bun:test";
import { act, useState } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { LoopRing } from "../src/components/LoopRing";

import type { AppConfig } from "../src/lib/types";

import { resetBackendMocks } from "./support/backendMock";
import { cardListConfig, pomodoroCard } from "./support/fixtures";
import { installDomLifecycle, buttonWithText } from "./support/dom";
import { installHttpLifecycle } from "./support/http";

beforeEach(resetBackendMocks);
const { mount, cleanupMountedRoots } = installDomLifecycle();
installHttpLifecycle(cleanupMountedRoots);
afterEach(resetBackendMocks);

function loopConfig(): AppConfig {
  // Three cards the loop can actually tell apart. Three clocks would all
  // read "Clock" now, which is true of the product and useless in a test
  // about which entry is which.
  return cardListConfig([
    { ...pomodoroCard("first", "Desk"), dwell_seconds: 45 },
    pomodoroCard("second", "Up next"),
    pomodoroCard("third", "Focus"),
  ]);
}

function renderLoopRing(config: AppConfig): string {
  return renderToStaticMarkup(
    <LoopRing
      config={config}
      issues={[]}
      selectedCardId="first"
      onSelect={() => {}}
      onChange={() => {}}
    />,
  );
}

test("the loop ring shows the loop length and every card in it", () => {
  // 45 + 20 + 20: the first card overrides the 20 s default, the other two take it.
  // Since schema v10 every card is in the loop, so there is no card to leave out.
  const html = renderLoopRing(loopConfig());
  expect(html).toMatch(/1 min 25 s/);
  expect(html).toContain("Desk");
  expect(html).toContain("Up next");
  expect(html).toContain("Focus");
});

test("the loop ring hides timings and the play control under manual advance", () => {
  const config = loopConfig();
  const html = renderLoopRing({
    ...config,
    advance: { kind: "manual" },
  });
  expect(html).not.toMatch(/\d+s</);
  expect(html).not.toContain("Play the loop");
  expect(html).toContain("Desk");
});

test("the loop ring owns pacing and writes the active loop advance mode", async () => {
  const initial = loopConfig();
  let latest = initial;

  function Harness() {
    const [config, setConfig] = useState(initial);
    latest = config;
    return (
      <LoopRing
        config={config}
        issues={[
          {
            path: "advance.default_dwell_seconds",
            code: "out-of-range",
            message: "Default dwell is out of range.",
          },
        ]}
        selectedCardId="first"
        onSelect={() => {}}
        onChange={setConfig}
      />
    );
  }

  const { container, root } = await mount();
  await act(async () => root.render(<Harness />));
  expect(container.querySelector("#loop-heading")?.textContent).toBe("Rotation");
  expect(container.textContent).not.toContain("Workday");
  const dwell = container.querySelector<HTMLInputElement>(".loop__dwell input");
  expect(dwell?.getAttribute("aria-invalid")).toBe("true");
  expect(dwell?.getAttribute("aria-describedby")).toBe("loop-pacing-issues");
  expect(container.querySelector("#loop-pacing-issues")?.tagName).toBe("UL");
  expect(container.textContent).toContain("Default dwell is out of range.");
  const manual = buttonWithText(container, "Manual");
  expect(manual?.getAttribute("aria-pressed")).toBe("false");
  await act(async () => manual?.click());
  expect(latest.advance).toEqual({ kind: "manual" });
  expect(buttonWithText(container, "Manual")?.getAttribute("aria-pressed")).toBe("true");
});

test("clicking the already-selected pacing mode does not emit a draft change", async () => {
  const config = loopConfig();
  let changeCount = 0;
  const { container, root } = await mount();

  await act(async () =>
    root.render(
      <LoopRing
        config={config}
        issues={[]}
        selectedCardId="first"
        onSelect={() => {}}
        onChange={() => {
          changeCount += 1;
        }}
      />,
    ),
  );
  await act(async () => buttonWithText(container, "Timed")?.click());
  expect(changeCount).toBe(0);
});

test("clearing the default dwell input keeps an empty edit without writing zero", async () => {
  const initial = loopConfig();
  let latest = initial;

  function Harness() {
    const [config, setConfig] = useState(initial);
    latest = config;
    return (
      <LoopRing
        config={config}
        issues={[]}
        selectedCardId="first"
        onSelect={() => {}}
        onChange={setConfig}
      />
    );
  }

  const { container, root } = await mount();
  await act(async () => root.render(<Harness />));
  const dwell = container.querySelector<HTMLInputElement>(".loop__dwell input");
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(dwell, "");
    dwell?.dispatchEvent(new Event("input", { bubbles: true }));
  });
  expect(dwell?.value).toBe("");
  expect(latest.advance).toEqual({
    kind: "timed",
    default_dwell_seconds: 20,
  });
});

test("the loop legend only displays and selects; it has no reorder affordance", () => {
  const html = renderLoopRing(loopConfig());
  expect(html).not.toContain("draggable");
  expect(html).not.toContain("loop__moves");
  expect(html).not.toContain("Move Digital clock");
});

test("playback keeps its deadline across equivalent configs and reads the latest loop order", async () => {
  const config = loopConfig();
  let now = 1_000;
  let tick: (() => void) | undefined;
  const selected: string[] = [];
  const onSelect = (id: string) => selected.push(id);
  const onChange = () => {};
  const browserWindow: Window = window;
  const dateNow = spyOn(Date, "now").mockImplementation(() => now);
  const interval = spyOn(browserWindow, "setInterval").mockImplementation((handler) => {
    if (typeof handler !== "function") throw new Error("Expected an interval callback");
    tick = () => handler();
    return 1;
  });
  const clearInterval = spyOn(browserWindow, "clearInterval").mockImplementation(() => {});
  try {
    const { container, root } = await mount();
    const render = async (draft: AppConfig) => {
      await act(async () =>
        root.render(
          <LoopRing
            config={draft}
            issues={[]}
            selectedCardId="first"
            onSelect={onSelect}
            onChange={onChange}
          />,
        ),
      );
    };
    await render(config);
    const play = buttonWithText(container, "Play the loop");
    if (!play) throw new Error("Missing playback control");
    await act(async () => play.click());
    if (!tick) throw new Error("Playback did not install its interval");
    const originalDeadline = now + 45_000;
    now += 100;
    await render(structuredClone(config));
    now = originalDeadline - 1;
    await act(async () => tick?.());
    expect(selected).toEqual([]);
    now = originalDeadline;
    await act(async () => tick?.());
    expect(selected).toEqual(["second"]);

    // Keep the selected prop and callbacks stable: the next tick must use the
    // new order without rearming the running session's next deadline.
    const nextDeadline = originalDeadline + 20_000;
    now += 100;
    const reordered = structuredClone(config);
    [reordered.cards[1], reordered.cards[2]] = [reordered.cards[2], reordered.cards[1]];
    await render(reordered);
    expect(interval).toHaveBeenCalledTimes(1);
    now = nextDeadline - 1;
    await act(async () => tick?.());
    expect(selected).toEqual(["second"]);
    now = nextDeadline;
    await act(async () => tick?.());
    expect(selected).toEqual(["second", "third"]);
  } finally {
    try {
      await cleanupMountedRoots();
    } finally {
      dateNow.mockRestore();
      interval.mockRestore();
      clearInterval.mockRestore();
    }
  }
});

test("playback keeps the original deadline and newest callback across parent rerenders", async () => {
  let now = 1_000;
  let tick: (() => void) | undefined;
  const selected: { revision: number; id: string }[] = [];
  function Harness({ revision }: { revision: number }) {
    return (
      <LoopRing
        config={loopConfig()}
        issues={[]}
        selectedCardId="first"
        onSelect={(id) => selected.push({ revision, id })}
        onChange={() => {}}
      />
    );
  }
  const browserWindow: Window = window;
  const dateNow = spyOn(Date, "now").mockImplementation(() => now);
  const interval = spyOn(browserWindow, "setInterval").mockImplementation((handler) => {
    if (typeof handler !== "function") throw new Error("Expected an interval callback");
    tick = () => handler();
    return 1;
  });
  const clearInterval = spyOn(browserWindow, "clearInterval").mockImplementation(() => {});
  try {
    const { container, root } = await mount(<Harness revision={0} />);
    await act(async () => buttonWithText(container, "Play the loop")?.click());
    for (let revision = 1; revision <= 3; revision += 1) {
      now += 10_000;
      await act(async () => root.render(<Harness revision={revision} />));
    }
    now = 45_999;
    await act(async () => tick?.());
    expect(selected).toEqual([]);
    now = 46_000;
    await act(async () => tick?.());
    expect(selected).toEqual([{ revision: 3, id: "second" }]);
    expect(interval).toHaveBeenCalledTimes(1);
    expect(clearInterval).not.toHaveBeenCalled();
  } finally {
    try {
      await cleanupMountedRoots();
    } finally {
      dateNow.mockRestore();
      interval.mockRestore();
      clearInterval.mockRestore();
    }
  }
});
