import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";

import { App } from "../src/App";
import { DeviceHeader } from "../src/components/DeviceHeader";
import { DevicePreview } from "../src/components/DevicePreview";
import { ProviderStatus, formatProviderAge } from "../src/components/ProviderStatus";
import { ScreenArranger } from "../src/components/ScreenArranger";
import { WidgetEditor } from "../src/components/WidgetEditor";
import { ipcContractFixtures } from "../src/lib/types.contract";

const snapshot = ipcContractFixtures.snapshot;
const widgets = snapshot.config.widgets;

describe("settings accessibility and states", () => {
  test("renders a non-blocking loading state before the first backend snapshot", () => {
    const html = renderToStaticMarkup(<App />);
    expect(html).toContain("Opening your display settings");
    expect(html).toContain("background service keeps running");
  });

  test("labels the editor controls and explains the clean canvas", () => {
    const clock = widgets.find((widget) => widget.kind === "clock");
    if (!clock) {
      throw new Error("contract fixture is missing its clock widget");
    }
    const html = renderToStaticMarkup(
      <WidgetEditor
        widget={clock}
        widgetIndex={0}
        issues={[]}
        pomodoro={null}
        timerBusy={false}
        filePickerBusy={false}
        onChange={() => {}}
        onRemove={() => {}}
        onTimerAction={() => {}}
        onChooseCalendarFile={() => {}}
      />,
    );
    expect(html).toContain("Widget ID");
    expect(html).toContain("Clean 448 × 368 canvas");
    expect(html).toContain("Show seconds");
  });

  test("offers a bounded native file chooser for local calendars", () => {
    const calendar = widgets.find((widget) => widget.kind === "calendar");
    if (calendar?.kind !== "calendar") {
      throw new Error("contract fixture is missing its calendar widget");
    }
    const html = renderToStaticMarkup(
      <WidgetEditor
        widget={{ ...calendar, source: { kind: "file", value: "" } }}
        widgetIndex={2}
        issues={[]}
        pomodoro={null}
        timerBusy={false}
        filePickerBusy={false}
        onChange={() => {}}
        onRemove={() => {}}
        onTimerAction={() => {}}
        onChooseCalendarFile={() => {}}
      />,
    );
    expect(html).toContain("Choose file…");
    expect(html).toContain("up to 1 MB");
  });

  test("exposes explicit move buttons and keyboard instructions", () => {
    const html = renderToStaticMarkup(
      <ScreenArranger
        screens={snapshot.config.screens}
        widgets={widgets}
        selectedWidgetId="clock"
        onSelect={() => {}}
        onReorder={() => {}}
      />,
    );
    expect(html).toContain("Screen order");
    expect(html).toContain("⌥ ↑ ↓ to move");
    expect(html).toContain("Move Focus up");
    expect(html).toContain("Move Focus down");
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
        widgets={widgets}
        refreshingId={null}
        onRefresh={() => {}}
      />,
    );
    expect(providers).toContain("Showing the last successful data");
    expect(providers).toContain("Refresh now");
  });

  test("renders deterministic preview and a useful empty state", () => {
    const populated = renderToStaticMarkup(
      <DevicePreview
        widgets={widgets}
        screens={snapshot.config.screens}
        selectedWidgetId="clock"
        pomodoros={snapshot.pomodoros}
        orientation="landscape"
        onSelect={() => {}}
      />,
    );
    expect(populated).toContain("09:41");
    expect(populated).toContain("Layout preview · not pixel-identical");
    expect(populated).not.toContain("USB · 09:41");

    const empty = renderToStaticMarkup(
      <DevicePreview
        widgets={[]}
        screens={[]}
        selectedWidgetId={null}
        pomodoros={[]}
        orientation="landscape-flipped"
        onSelect={() => {}}
      />,
    );
    expect(empty).toContain("The standalone clock stays available.");
    expect(empty).toContain("is-flipped");
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
