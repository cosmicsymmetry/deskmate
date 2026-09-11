/**
 * Where the add-card menu goes.
 *
 * Kept pure and apart from the component because the decision is arithmetic on
 * rectangles: those numbers are testable, whereas the DOM measurement that feeds
 * them is not — a jsdom test has no layout to measure.
 *
 * The menu is a popover, so it is placed against the **viewport**, not against
 * the scrolling work column it happens to live in. It used to be positioned
 * absolutely inside `.face__work`, which has `overflow-y: auto` and therefore
 * clipped it, and a flat 300px cap then pushed the final group past the menu's
 * own scroll fold. Its heading stayed on screen with nothing underneath it: the
 * entries were in the DOM, but painted nowhere a click could reach.
 */

/** Matches the `.menu` width in `styles.css`; the two must agree. */
export const MENU_WIDTH = 272;

/** Space between the trigger and the menu. */
export const MENU_GAP = 6;

/** Breathing room kept against the viewport edge. */
export const MENU_MARGIN = 12;

export interface MenuTriggerRect {
  top: number;
  bottom: number;
  left: number;
  right: number;
}

export interface MenuViewport {
  width: number;
  height: number;
}

export interface MenuPlacement {
  /** Anchor the menu's right edge to the trigger's, so it does not overhang. */
  alignEnd: boolean;
  /** Distance from the top of the viewport, in CSS pixels. */
  top: number;
  /** The tallest the menu may be where it has been placed. */
  maxHeight: number;
}

/**
 * `contentHeight` is the menu's unconstrained height, measured from the DOM. It
 * is what lets this decide whether the menu can be shown whole rather than
 * capping it at a constant and hoping.
 */
export function placeAddMenu(
  trigger: MenuTriggerRect,
  viewport: MenuViewport,
  contentHeight: number,
): MenuPlacement {
  const alignEnd = trigger.left + MENU_WIDTH > viewport.width - MENU_MARGIN;
  const spaceBelow = viewport.height - trigger.bottom - MENU_GAP - MENU_MARGIN;
  const spaceAbove = trigger.top - MENU_GAP - MENU_MARGIN;

  if (contentHeight <= spaceBelow) {
    return { alignEnd, top: trigger.bottom + MENU_GAP, maxHeight: spaceBelow };
  }
  if (contentHeight <= spaceAbove) {
    return { alignEnd, top: trigger.top - MENU_GAP - contentHeight, maxHeight: spaceAbove };
  }

  // Neither side of the trigger can hold the menu. Anchoring to it anyway is
  // what produced the defect this module exists for: the menu keeps its tidy
  // relationship to the button and hides most of itself behind its own scroll
  // fold. A window shorter than the menu is common — the app's own window is —
  // so the menu is clamped into the viewport instead and stays whole, giving up
  // only its exact alignment with the button.
  const usable = viewport.height - MENU_MARGIN * 2;
  const height = Math.min(contentHeight, usable);
  const top = Math.min(
    Math.max(MENU_MARGIN, trigger.bottom + MENU_GAP),
    viewport.height - MENU_MARGIN - height,
  );
  return { alignEnd, top: Math.max(MENU_MARGIN, top), maxHeight: usable };
}
