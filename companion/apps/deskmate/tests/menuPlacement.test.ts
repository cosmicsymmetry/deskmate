import { describe, expect, test } from "bun:test";

import { MENU_GAP, MENU_MARGIN, MENU_WIDTH, placeAddMenu } from "../src/lib/menuPlacement";

/**
 * The add slot as it sits in a real viewport: low in a tall work column, which is
 * the position that produced the defect these tests exist for.
 */
const lowTrigger = { top: 241, bottom: 348, left: 880, right: 1028 };

/** The browser viewport, which is shorter than the page content. */
const viewport = { width: 1096, height: 700 };

/** A deliberately tall menu, independent of whichever card kinds are available. */
const fullMenu = 479;

describe("add-card menu placement", () => {
  test("a menu that fits below the trigger hangs from it", () => {
    const placement = placeAddMenu(lowTrigger, viewport, 200);

    expect(placement.top).toBe(lowTrigger.bottom + MENU_GAP);
  });

  test("a menu that only fits above the trigger sits on top of it", () => {
    const trigger = { ...lowTrigger, top: 420, bottom: 527 };

    const placement = placeAddMenu(trigger, { ...viewport, height: 600 }, 380);

    expect(placement.top).toBe(trigger.top - MENU_GAP - 380);
  });

  /**
   * The viewport has room for the whole menu, just not beneath the button, so the
   * menu must be shown whole rather than anchored and clipped.
   */
  test("a menu too tall for either side is shown whole rather than anchored", () => {
    const placement = placeAddMenu(lowTrigger, viewport, fullMenu);

    expect(placement.maxHeight).toBeGreaterThanOrEqual(fullMenu);
    expect(placement.top).toBeGreaterThanOrEqual(MENU_MARGIN);
    expect(placement.top + fullMenu).toBeLessThanOrEqual(viewport.height - MENU_MARGIN);
  });

  test("a menu taller than the viewport fills it and scrolls without overflowing", () => {
    const placement = placeAddMenu(lowTrigger, viewport, 5_000);

    expect(placement.top).toBe(MENU_MARGIN);
    expect(placement.maxHeight).toBe(viewport.height - MENU_MARGIN * 2);
  });

  test("a trigger near the right edge anchors the menu's right edge to it", () => {
    expect(placeAddMenu(lowTrigger, viewport, fullMenu).alignEnd).toBe(true);
    expect(placeAddMenu({ ...lowTrigger, left: 40 }, viewport, fullMenu).alignEnd).toBe(false);
  });

  test("a trigger with exactly enough room to the right does not flip", () => {
    const left = viewport.width - MENU_MARGIN - MENU_WIDTH;

    expect(placeAddMenu({ ...lowTrigger, left }, viewport, fullMenu).alignEnd).toBe(false);
    expect(placeAddMenu({ ...lowTrigger, left: left + 1 }, viewport, fullMenu).alignEnd).toBe(true);
  });
});
