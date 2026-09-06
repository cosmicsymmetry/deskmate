import type { ReactElement } from "react";

/**
 * The icon set: authored on one 16px grid, one 1.75 stroke, round caps and joins.
 *
 * Deskmate ships no icon library — the Tauri CSP blocks every external host and
 * nothing is bundled — and the alternative habit, borrowing Unicode glyphs like ×
 * and ↑ as icons, hands their weight and alignment to whatever font resolves. These
 * are drawn instead, so a control's mark carries the same stroke as every other.
 */
type IconName = "left" | "right" | "close" | "plus" | "check" | "refresh" | "settings";

const PATHS: Record<IconName, ReactElement> = {
  left: <path d="M10 4 6 8l4 4" />,
  right: <path d="M6 4l4 4-4 4" />,
  close: <path d="M4.5 4.5l7 7M11.5 4.5l-7 7" />,
  plus: <path d="M8 3.5v9M3.5 8h9" />,
  check: <path d="M3.5 8.5l3 3 6-6.5" />,
  refresh: <path d="M13 8a5 5 0 1 1-1.6-3.7M13 2.5V5h-2.5" />,
  /* Sliders, not a cogwheel: eight teeth turn to mush at 1.75 stroke on a 16 grid,
     and the line breaks below keep each knob legible instead of overprinting it. */
  settings: (
    <>
      <path d="M2.5 5.5h1.6M7.9 5.5h5.6M2.5 10.5h6.1M12.4 10.5h1.1" />
      <circle cx="6" cy="5.5" r="1.9" />
      <circle cx="10.5" cy="10.5" r="1.9" />
    </>
  ),
};

export function Icon({ name, className }: { name: IconName; className?: string }) {
  return (
    <svg
      className={className ? `icon ${className}` : "icon"}
      viewBox="0 0 16 16"
      width="16"
      height="16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.75"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      {PATHS[name]}
    </svg>
  );
}
