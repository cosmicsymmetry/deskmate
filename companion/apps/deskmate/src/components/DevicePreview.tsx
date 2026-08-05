import type { CSSProperties } from "react";

import { primaryScreenWidgetId, widgetName } from "../lib/configDraft";
import type {
  DisplayOrientation,
  PomodoroSnapshot,
  ScreenSettings,
  WidgetSettings,
} from "../lib/types";

interface DevicePreviewProps {
  widgets: WidgetSettings[];
  screens: ScreenSettings[];
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
  widget: WidgetSettings;
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
  widgets,
  screens,
  selectedWidgetId,
  pomodoros,
  orientation,
  onSelect,
}: DevicePreviewProps) {
  const activeScreen =
    screens.find((screen) => primaryScreenWidgetId(screen) === selectedWidgetId) ??
    screens[0] ??
    null;
  const activeWidgetId = activeScreen ? primaryScreenWidgetId(activeScreen) : null;
  const widget = activeScreen
    ? widgets.find((candidate) => candidate.id === activeWidgetId)
    : undefined;
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
      {screens.length > 1 && (
        <fieldset className="preview-dots">
          <legend className="sr-only">Preview screen</legend>
          {screens.map((screen, index) => {
            const widgetId = primaryScreenWidgetId(screen) ?? "";
            const candidate = widgets.find((item) => item.id === widgetId);
            return (
              <button
                key={screen.id}
                type="button"
                className={screen.id === activeScreen?.id ? "is-active" : ""}
                aria-label={`Preview screen ${index + 1}${candidate ? `: ${widgetName(candidate)}` : ""}`}
                aria-pressed={screen.id === activeScreen?.id}
                onClick={() => onSelect(widgetId)}
              />
            );
          })}
        </fieldset>
      )}
    </section>
  );
}
