import { widgetKindName, widgetName, type WidgetKind } from "../lib/configDraft";
import type { CardSettings } from "../lib/types";

interface WidgetGalleryProps {
  cards: CardSettings[];
  selectedWidgetId: string | null;
  onSelect: (widgetId: string) => void;
  onAdd: (kind: WidgetKind) => void;
}

const kinds: { kind: WidgetKind; glyph: string; description: string }[] = [
  { kind: "clock", glyph: "09:41", description: "Time and date" },
  { kind: "pomodoro", glyph: "25", description: "Focus timer" },
  { kind: "calendar", glyph: "≡", description: "Upcoming events" },
];

export function WidgetGallery({ cards, selectedWidgetId, onSelect, onAdd }: WidgetGalleryProps) {
  return (
    <section className="panel gallery-panel" aria-labelledby="widgets-heading">
      <div className="panel-heading">
        <div>
          <p className="step-label">1 · Widgets</p>
          <h2 id="widgets-heading">What should it show?</h2>
        </div>
        <span className="count-badge">{cards.length}/16</span>
      </div>

      {cards.length === 0 ? (
        <div className="empty-state">
          <strong>Your display is empty</strong>
          <span>Add a widget to create its first screen.</span>
        </div>
      ) : (
        <ul className="widget-list" aria-label="Configured widgets">
          {cards.map((widget) => (
            <li key={widget.id}>
              <button
                className={`widget-card${selectedWidgetId === widget.id ? " is-selected" : ""}`}
                type="button"
                aria-pressed={selectedWidgetId === widget.id}
                onClick={() => onSelect(widget.id)}
              >
                <span className={`widget-glyph widget-glyph--${widget.kind}`} aria-hidden="true">
                  {widget.kind === "clock" ? "09:41" : widget.kind === "pomodoro" ? "25" : "≡"}
                </span>
                <span>
                  <strong>{widgetName(widget)}</strong>
                  <small>{widgetKindName(widget.kind)}</small>
                </span>
                <span className="chevron" aria-hidden="true">
                  ›
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}

      <fieldset className="gallery-add">
        <legend className="sr-only">Add a widget</legend>
        {kinds.map(({ kind, glyph, description }) => (
          <button
            className="add-card"
            type="button"
            key={kind}
            onClick={() => onAdd(kind)}
            disabled={cards.length >= 16}
          >
            <span className="add-card__glyph" aria-hidden="true">
              {glyph}
            </span>
            <span>
              <strong>Add {widgetKindName(kind)}</strong>
              <small>{description}</small>
            </span>
            <span aria-hidden="true">+</span>
          </button>
        ))}
      </fieldset>
    </section>
  );
}
