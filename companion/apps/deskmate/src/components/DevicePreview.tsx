import { useEffect, useState, type CSSProperties } from "react";

import { cardFields, cardName } from "../lib/configDraft";
import type {
  CardDataSnapshot,
  CardFieldValue,
  CardSettings,
  DisplayOrientation,
  PomodoroSnapshot,
} from "../lib/types";

interface DevicePreviewProps {
  cards: CardSettings[];
  selectedWidgetId: string | null;
  cardData: CardDataSnapshot[];
  pomodoros: PomodoroSnapshot[];
  orientation: DisplayOrientation;
}

function pad(value: number): string {
  return String(value).padStart(2, "0");
}

function textField(fields: Map<string, CardFieldValue>, key: string): string {
  const field = fields.get(key);
  return field?.kind === "text" ? field.value : "";
}

function boolField(fields: Map<string, CardFieldValue>, key: string): boolean {
  const field = fields.get(key);
  return field?.kind === "boolean" ? field.value : false;
}

/// A visible, non-tooltip marker worn whenever a face has nothing published
/// yet to show — i.e. `cardFields` returned an empty map because the
/// runtime has no snapshot for this card (typically: added but never
/// saved). Never applied just because one row or field happens to be blank;
/// see `cardFields`'s own contract for the empty-map rule.
function SampleBadge() {
  return <span className="preview-sample-badge">Sample</span>;
}

/// The device's own wall clock, ticking locally the same way the physical
/// panel's standalone clock does — this is real, live data, not a
/// published field, because clock cards never emit one (their refresh
/// policy is `device-local`). SSR never runs the effect, so the server-
/// rendered markup is simply whatever moment the render happened to run at.
function useLiveClock(): Date {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const id = window.setInterval(() => setNow(new Date()), 1000);
    return () => window.clearInterval(id);
  }, []);
  return now;
}

function ClockFace({ widget }: { widget: Extract<CardSettings, { kind: "clock" }> }) {
  const now = useLiveClock();
  const weekday = now.toLocaleDateString(undefined, { weekday: "long" });
  const monthDay = now.toLocaleDateString(undefined, { month: "long", day: "numeric" });
  return (
    <div className="preview-clock">
      <span>{widget.title || "Desk"}</span>
      <strong className="numeral">
        {pad(now.getHours())}:{pad(now.getMinutes())}
        {widget.show_seconds && <small>:{pad(now.getSeconds())}</small>}
      </strong>
      <span>
        {weekday} · {monthDay}
      </span>
    </div>
  );
}

/// The empty-config fallback face: no cards exist at all, so there is
/// nothing to select — but the physical panel still shows its standalone
/// clock in that state, and so does this preview, using the same live time
/// as `ClockFace` rather than a frozen placeholder string.
function StandaloneClockFace() {
  const now = useLiveClock();
  return (
    <div className="preview-empty">
      <strong className="numeral">
        {pad(now.getHours())}:{pad(now.getMinutes())}
      </strong>
      <span>The standalone clock stays available.</span>
    </div>
  );
}

function PomodoroFace({
  widget,
  pomodoro,
}: {
  widget: Extract<CardSettings, { kind: "pomodoro" }>;
  pomodoro: PomodoroSnapshot | undefined;
}) {
  const remaining = pomodoro?.remaining_seconds ?? widget.duration_seconds;
  const minutes = Math.floor(remaining / 60);
  const seconds = remaining % 60;
  const total = Math.max(1, pomodoro?.duration_seconds ?? widget.duration_seconds);
  const progress = Math.round((remaining / total) * 360);
  return (
    <div className="preview-pomodoro">
      <div className="preview-ring" style={{ "--progress": `${progress}deg` } as CSSProperties}>
        <span className="numeral">
          {pad(minutes)}:{pad(seconds)}
        </span>
      </div>
      <strong>{widget.label || "Focus"}</strong>
      <small>{pomodoro?.state ?? "Tap to start"}</small>
    </div>
  );
}

/// The up-to-five `rowN_title` / `rowN_time` pairs a calendar or RSS card
/// publishes, in order, skipping rows with no title — that is how the
/// backend represents "fewer than five items right now", not a special
/// field of its own.
function rowsFromFields(fields: Map<string, CardFieldValue>): { title: string; time: string }[] {
  const rows: { title: string; time: string }[] = [];
  for (let index = 0; index < 5; index += 1) {
    const title = textField(fields, `row${index}_title`);
    if (!title) {
      continue;
    }
    rows.push({ title, time: textField(fields, `row${index}_time`) });
  }
  return rows;
}

/// Shared face for every provider-backed card (calendar, RSS, weather,
/// JSON feed): a title plus whatever rows the runtime last published. This
/// is the face the invented "Design review 10:30" data used to live on —
/// it now shows exactly what the device shows, including truncation and a
/// genuinely empty state, instead of plausible-looking fiction.
function RowsFace({
  widget,
  fields,
  sample,
}: {
  widget: CardSettings;
  fields: Map<string, CardFieldValue>;
  sample: boolean;
}) {
  const title = (sample ? "" : textField(fields, "title")) || cardName(widget);
  const rows = sample ? [] : rowsFromFields(fields);
  const stale = boolField(fields, "stale");
  const error = textField(fields, "error");
  return (
    <div className="preview-calendar">
      <strong>
        {title}
        {sample && <SampleBadge />}
      </strong>
      {rows.length > 0 ? (
        rows.map((row) => (
          <div key={`${row.title}-${row.time}`}>
            <span>{row.title}</span>
            {row.time && <time className="numeral">{row.time}</time>}
          </div>
        ))
      ) : (
        <div className="preview-calendar__placeholder">
          <span>
            {sample
              ? "Add a source and save to preview real items."
              : stale && error
                ? error
                : "No items yet."}
          </span>
        </div>
      )}
    </div>
  );
}

function WidgetFace({
  widget,
  cardData,
  pomodoro,
}: {
  widget: CardSettings;
  cardData: CardDataSnapshot[];
  pomodoro: PomodoroSnapshot | undefined;
}) {
  if (widget.kind === "clock") {
    return <ClockFace widget={widget} />;
  }
  if (widget.kind === "pomodoro") {
    return <PomodoroFace widget={widget} pomodoro={pomodoro} />;
  }
  const fields = cardFields(cardData, widget.id);
  return <RowsFace widget={widget} fields={fields} sample={fields.size === 0} />;
}

export function DevicePreview({
  cards,
  selectedWidgetId,
  cardData,
  pomodoros,
  orientation,
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
        <span>Same data as your display · approximate pixels</span>
      </div>
      <div className={`device-shell${orientation === "landscape-flipped" ? " is-flipped" : ""}`}>
        <div className="device-screen">
          {widget ? (
            <WidgetFace widget={widget} cardData={cardData} pomodoro={pomodoro} />
          ) : (
            <StandaloneClockFace />
          )}
        </div>
        <span className="device-port" aria-hidden="true" />
      </div>
    </section>
  );
}
