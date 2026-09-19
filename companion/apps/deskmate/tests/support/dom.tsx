import { afterEach } from "bun:test";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";

// Each suite owns its registry so a failed assertion cannot leave a live root
// calling another test's backend or a detached container in the document.
export function installDomLifecycle() {
  const cleanups = new Set<() => Promise<void>>();
  async function cleanupMountedRoots() {
    for (const cleanup of [...cleanups]) await cleanup();
  }
  afterEach(cleanupMountedRoots);
  async function mount(element: Parameters<Root["render"]>[0] = null) {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    const cleanup = async () => {
      if (!cleanups.delete(cleanup)) return;
      try {
        await act(async () => root.unmount());
      } finally {
        container.remove();
      }
    };
    cleanups.add(cleanup);
    await renderPreviewInto(root, element);
    return { container, root, cleanup };
  }
  return { mount, cleanupMountedRoots };
}

export async function renderPreviewInto(root: Root, element: Parameters<Root["render"]>[0]) {
  await act(async () => {
    root.render(element);
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

/// Polls `assertion` inside `act`, so the passive-effect state updates the polling
/// itself waits out (rather than the initial `renderPreviewInto` settle) are also
/// attributed to an `act` scope.
export async function waitFor(assertion: () => void, timeoutMs = 500): Promise<void> {
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

export function buttonWithText(container: ParentNode, text: string): HTMLButtonElement | undefined {
  return [...container.querySelectorAll("button")].find(
    (button) => button.textContent?.trim() === text,
  );
}
