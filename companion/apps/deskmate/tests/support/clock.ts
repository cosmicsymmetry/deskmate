import { afterEach, beforeEach, spyOn } from "bun:test";
import { act } from "react";

// Like the mock-backend suite, control browser timeouts, leaving the short
// mount/waitFor scheduling ticks real. Flush React between deadlines so validation can
// finish and schedule the following autosave at its actual virtual time.
export function installWindowClock(cleanup: () => Promise<void>) {
  let now = 0;
  let nextId = 0;
  const timers = new Map<number, { at: number; run: () => void }>();
  let restore: () => void;

  beforeEach(() => {
    now = 0;
    nextId = 0;
    timers.clear();
    const browserWindow: Window = window;
    const realTimeout = browserWindow.setTimeout.bind(browserWindow);
    const realClear = browserWindow.clearTimeout.bind(browserWindow);
    const timeout = spyOn(browserWindow, "setTimeout").mockImplementation((handler, ms = 0) => {
      if (ms < 10) return realTimeout(handler, ms);
      const id = --nextId;
      timers.set(id, { at: now + ms, run: () => (handler as () => void)() });
      return id;
    });
    const clear = spyOn(browserWindow, "clearTimeout").mockImplementation((id) => {
      if (id === undefined || !timers.delete(id)) realClear(id);
    });
    restore = () => {
      timeout.mockRestore();
      clear.mockRestore();
    };
  });

  afterEach(async () => {
    try {
      await cleanup();
    } finally {
      restore();
      timers.clear();
    }
  });

  function nextDue(target: number) {
    return [...timers.entries()]
      .filter(([, timer]) => timer.at <= target)
      .sort((a, b) => a[1].at - b[1].at)[0];
  }

  // Synchronous ticking lets a test dispatch a competing click before React
  // paints the state set by an autosave timeout. Call it inside act().
  function tick(ms: number) {
    const target = now + ms;
    for (let next = nextDue(target); next; next = nextDue(target)) {
      const [id, timer] = next;
      timers.delete(id);
      now = timer.at;
      timer.run();
    }
    now = target;
  }

  async function advance(ms: number) {
    const target = now + ms;
    for (let next = nextDue(target); next; next = nextDue(target)) {
      await act(async () => tick(next[1].at - now));
    }
    now = target;
  }

  return { advance, tick };
}
