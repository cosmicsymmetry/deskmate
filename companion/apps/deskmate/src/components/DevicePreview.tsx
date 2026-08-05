import type { CSSProperties } from "react";

import { widgetName } from "../lib/configDraft";
import type { CardSettings, DisplayOrientation, PomodoroSnapshot } from "../lib/types";

interface DevicePreviewProps {
  cards: CardSettings[];
  selectedWidgetId: string | null;
  pomodoros: PomodoroSnapshot[];
  orientation: DisplayOrientation;
  onSelect: (widgetId: string) => void;
}

function pad(value: number): string {
  return String(value).padStart(2, "0");
}

function WidgetFace({
  widget,
  pomodoro,
}: {
  widget: CardSettings;
  pomodoro: PomodoroSnapshot | undefined;
}) {
  if (widget.kind === "clock") {
    return (
      <div className="preview-clock">
        <span>{widget.title || "Desk"}</span>
        <strong>09:41{widget.show_seconds && <small>:26</small>}</strong>
        <span>Wednesday · August 5</span>
      </div>
    );
  }
  if (widget.kind === "pomodoro") {
    const remaining = pomodoro?.remaining_seconds ?? widget.duration_seconds;
    const minutes = Math.floor(remaining / 60);
    const seconds = remaining % 60;
    const total = Math.max(1, pomodoro?.duration_seconds ?? widget.duration_seconds);
    const progress = Math.round((remaining / total) * 360);
    return (
      <div className="preview-pomodoro">
        <div className="preview-ring" style={{ "--progress": `${progress}deg` } as CSSProperties}>
          <span>
            {pad(minutes)}:{pad(seconds)}
          </span>
        </div>
        <strong>{widget.label || "Focus"}</strong>
        <small>{pomodoro?.state ?? "Tap to start"}</small>
      </div>
    );
  }
  return (
    <div className="preview-calendar">
      <strong>{widget.title || "Up next"}</strong>
      <div>
        <span>Design review</span>
        <time>10:30</time>
      </div>
      <div>
        <span>Lunch</span>
        <time>12:00</time>
      </div>
      <div>
        <span>Project sync</span>
        <time>14:15</time>
      </div>
    </div>
  );
}

export function DevicePreview({
  cards,
  selectedWidgetId,
  pomodoros,
  orientation,
  onSelect,
}: DevicePreviewProps) {
  // Falls back to the first card when the selection doesn't resolve, exactly as the
  // old screen model fell back to `screens[0]`. The empty state below therefore still
  // renders in precisely the same reachable case as before: zero cards configured.
  // (The old model could also show it via a screen referencing a deleted widget id —
  // a dangling reference — but a card IS its own widget now, so that case no longer
  // exists to lose.)
  const widget = cards.find((card) => card.id === selectedWidgetId) ?? cards[0] ?? undefined;
  const pomodoro = widget
    ? pomodoros.find((candidate) => candidate.widget_id === widget.id)
    : undefined;

  return (
    <section className="preview-panel" aria-labelledby="preview-heading">
      <div className="preview-heading">
        <div>
          <p className="step-label">Device preview</p>
          <h2 id="preview-heading">448 × 368 landscape</h2>
        </div>
        <span>Layout preview · not pixel-identical</span>
      </div>
      <div className={`device-shell${orientation === "landscape-flipped" ? " is-flipped" : ""}`}>
        <div className="device-screen">
          {widget ? (
            <WidgetFace widget={widget} pomodoro={pomodoro} />
          ) : (
            <div className="preview-empty">
              <strong>09:41</strong>
              <span>The standalone clock stays available.</span>
            </div>
          )}
        </div>
        <span className="device-port" aria-hidden="true" />
      </div>
      {cards.length > 1 && (
        <fieldset className="preview-dots">
          <legend className="sr-only">Preview screen</legend>
          {cards.map((card, index) => (
            <button
              key={card.id}
              type="button"
              className={card.id === widget?.id ? "is-active" : ""}
              aria-label={`Preview screen ${index + 1}: ${widgetName(card)}`}
              aria-pressed={card.id === widget?.id}
              onClick={() => onSelect(card.id)}
            />
          ))}
        </fieldset>
      )}
    </section>
  );
}
