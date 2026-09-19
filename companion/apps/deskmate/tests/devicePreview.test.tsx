import { afterEach, beforeEach, expect, test } from "bun:test";
import { act } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { DevicePreview } from "../src/components/DevicePreview";

import type { CardSettings, ImageSource, PreviewFrame } from "../src/lib/types";

import { backendMocks, resetBackendMocks } from "./support/backendMock";
import { clockCard, pictureCard, pomodoroCard } from "./support/fixtures";
import { installDomLifecycle, renderPreviewInto, waitFor } from "./support/dom";
import { installHttpLifecycle } from "./support/http";

beforeEach(resetBackendMocks);
const { mount, cleanupMountedRoots } = installDomLifecycle();
installHttpLifecycle(cleanupMountedRoots);
afterEach(resetBackendMocks);

async function mountPreview(
  card: CardSettings,
  dataGeneration = 0,
  imageSources: ImageSource[] = [],
): Promise<Awaited<ReturnType<typeof mount>>> {
  const { container, root, cleanup } = await mount();
  await renderPreviewInto(
    root,
    <DevicePreview
      cards={[card]}
      selectedWidgetId={card.id}
      orientation="landscape"
      dataGeneration={dataGeneration}
      imageSources={imageSources}
    />,
  );
  return { container, root, cleanup };
}

test("renders the device's own pixels with the card identity while requesting its wire id", async () => {
  backendMocks.previewImpl = async (cardId) => {
    expect(cardId).toBe("upnext");
    return { png_base64: "Zmlyc3QtZnJhbWU=", sample: false, state: null };
  };
  const { container, cleanup } = await mountPreview(clockCard("upnext"));
  await waitFor(() => {
    const img = container.querySelector("img");
    expect(img).not.toBeNull();
    expect(img?.getAttribute("src")).toBe("data:image/png;base64,Zmlyc3QtZnJhbWU=");
    expect(img?.getAttribute("alt")).toBe("Device preview of Clock");
  });
  expect(container.querySelector(".stage__badge")).toBeNull();
  await cleanup();
});

test("shows the explicit unavailable state when the preview request rejects", async () => {
  backendMocks.previewImpl = async () => {
    throw new Error("simulator init failed");
  };
  const { container, cleanup } = await mountPreview(clockCard("upnext"));
  await waitFor(() => {
    expect(container.textContent).toContain("Preview unavailable");
  });
  expect(container.querySelector("img")).toBeNull();
  await cleanup();
});

test("badges the frame as sample when the card has never published data", async () => {
  backendMocks.previewImpl = async () => ({
    png_base64: "dW5jb25maWd1cmVk",
    sample: true,
    state: null,
  });
  const { container, cleanup } = await mountPreview(clockCard("upnext"));
  await waitFor(() => {
    expect(container.querySelector("img")).not.toBeNull();
    expect(container.textContent).toContain("No data yet");
  });
  await cleanup();
});

test("a frameless preview prints its state word, never the fault message", async () => {
  backendMocks.previewImpl = async () => ({
    png_base64: null,
    sample: false,
    state: "Waiting for the first refresh",
  });
  const { container, cleanup } = await mountPreview(pictureCard());
  await waitFor(() => {
    expect(container.textContent).toContain("Waiting for the first refresh");
  });
  expect(container.textContent).not.toContain("Preview unavailable");
  expect(container.querySelector("img")).toBeNull();
  // The badge means "a real frame from sample data". There is no frame here.
  expect(container.querySelector(".stage__badge")).toBeNull();
  await cleanup();
});

test("re-requests the preview when dataGeneration bumps", async () => {
  let calls = 0;
  backendMocks.previewImpl = async () => {
    calls += 1;
    return { png_base64: `frame-${calls}`, sample: false, state: null };
  };
  const card = clockCard("upnext");
  const { container, root, cleanup } = await mountPreview(card, 0);
  await waitFor(() => expect(calls).toBe(1));

  await renderPreviewInto(
    root,
    <DevicePreview
      cards={[card]}
      selectedWidgetId={card.id}
      orientation="landscape"
      dataGeneration={1}
      imageSources={[]}
    />,
  );
  await waitFor(() => expect(calls).toBe(2));
  await waitFor(() => {
    expect(container.querySelector("img")?.getAttribute("src")).toBe(
      "data:image/png;base64,frame-2",
    );
  });
  await cleanup();
});

test("keeps a stale frame on screen while a superseding request is in flight, then replaces it", async () => {
  // Two cards, so switching `selectedWidgetId` (not `dataGeneration`) is what
  // triggers the re-request here — the effect depends on both.
  let resolveSecond!: (frame: PreviewFrame) => void;
  let requestCount = 0;
  backendMocks.previewImpl = async (cardId) => {
    requestCount += 1;
    if (cardId === "first") {
      return { png_base64: "first-frame", sample: false, state: null };
    }
    return new Promise((resolve) => {
      resolveSecond = resolve;
    });
  };
  const cardA = clockCard("first");
  const cardB = clockCard("second");
  const { container, root, cleanup } = await mountPreview(cardA);
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
      imageSources={[]}
    />,
  );
  await waitFor(() => expect(requestCount).toBe(2));
  // The old frame is still the last thing painted — no blank flash, no stale claim
  // beyond what was already shown, since the component never clears `frame` itself.
  expect(container.querySelector("img")?.getAttribute("src")).toBe(
    "data:image/png;base64,first-frame",
  );

  resolveSecond({ png_base64: "second-frame", sample: false, state: null });
  await waitFor(() => {
    expect(container.querySelector("img")?.getAttribute("src")).toBe(
      "data:image/png;base64,second-frame",
    );
  });
  await cleanup();
});

test("uses pomodoro labels and selected-card fallback in resolved image alt text", async () => {
  backendMocks.previewImpl = async (cardId) => {
    expect(cardId).toBe("wire-pomodoro-id");
    return { png_base64: "timer-frame", sample: false, state: null };
  };
  const card = pomodoroCard("wire-pomodoro-id", "Editorial sprint");
  const { container, root, cleanup } = await mountPreview(card);
  await waitFor(() => {
    expect(container.querySelector("img")?.getAttribute("alt")).toBe(
      "Device preview of Editorial sprint",
    );
  });

  await renderPreviewInto(
    root,
    <DevicePreview
      cards={[card]}
      selectedWidgetId="missing-selection"
      orientation="landscape"
      dataGeneration={0}
      imageSources={[]}
    />,
  );
  expect(container.querySelector("img")?.getAttribute("alt")).toBe(
    "Device preview of Editorial sprint",
  );
  await cleanup();
});

test("uses picture source names and the missing-source fallback in resolved image alt text", async () => {
  backendMocks.previewImpl = async () => ({
    png_base64: "picture-frame",
    sample: false,
    state: null,
  });
  const card = pictureCard("wire-picture-id");
  const named = await mountPreview(card, 0, [
    { id: "limits-source", name: "Quarterly launch board" },
  ]);
  await waitFor(() => {
    expect(named.container.querySelector("img")?.getAttribute("alt")).toBe(
      "Device preview of Quarterly launch board",
    );
  });
  await named.cleanup();

  const missing = await mountPreview(card);
  await waitFor(() => {
    expect(missing.container.querySelector("img")?.getAttribute("alt")).toBe(
      "Device preview of Missing source",
    );
  });
  await missing.cleanup();
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
// passes to `window.setInterval` and calls it directly.
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
    backendMocks.previewImpl = () => {
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
    const clock = clockCard("clock-preview");
    const { container, cleanup } = await mountPreview(clock);
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
      resolveSecond({ png_base64: "winning-frame", sample: false, state: null });
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

    await cleanup();
  } finally {
    try {
      await cleanupMountedRoots();
    } finally {
      window.setInterval = originalSetInterval;
      window.clearInterval = originalClearInterval;
    }
  }
});

test("shows 'No cards configured' when there are no cards, never a stale or invented frame", () => {
  // No live effect needed here: with no cards, `DevicePreview` never calls the
  // preview backend at all, so a static render is enough to check the empty state.
  const html = renderToStaticMarkup(
    <DevicePreview
      cards={[]}
      selectedWidgetId={null}
      orientation="landscape-flipped"
      dataGeneration={0}
      imageSources={[]}
    />,
  );
  expect(html).toContain("No cards configured");
  expect(html).not.toContain("<img");
});
