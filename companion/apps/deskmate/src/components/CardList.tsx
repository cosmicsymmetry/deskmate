import { useState, type DragEvent, type KeyboardEvent } from "react";

import {
  cardKindName,
  cardMoveFromKey,
  cardName,
  moveCard,
  nonRotationCards,
  rotationCards,
} from "../lib/configDraft";
import type { AppConfig, CardKind, CardSettings } from "../lib/types";

const MAX_CARDS = 8;

interface CardListProps {
  config: AppConfig;
  selectedCardId: string | null;
  onSelect: (cardId: string) => void;
  onAdd: (kind: CardKind) => void;
  onRemove: (cardId: string) => void;
  onReorder: (config: AppConfig) => void;
}

const addableKinds: { kind: CardKind; glyph: string; description: string }[] = [
  { kind: "clock", glyph: "09:41", description: "Time and date" },
  { kind: "pomodoro", glyph: "25", description: "Focus timer" },
  { kind: "calendar", glyph: "≡", description: "Upcoming events" },
  { kind: "weather", glyph: "☀", description: "Local conditions" },
  { kind: "json-feed", glyph: "{ }", description: "Custom JSON data" },
  { kind: "rss", glyph: "⟢", description: "Headlines feed" },
];

/// Dwell time shown beside a rotation row, in the numeral stack since it's a
/// duration. Falls back to an em dash for cards that inherit the carousel's
/// default rather than overriding it, so every row keeps the same columns.
function dwellLabel(card: CardSettings): string {
  if (card.presence.kind !== "in-rotation") {
    return "—";
  }
  return card.presence.dwell_seconds !== null ? `${card.presence.dwell_seconds}s` : "—";
}

function presenceLabel(card: CardSettings): string {
  return card.presence.kind === "alert-only" ? "Alert" : "Muted";
}

export function CardList({
  config,
  selectedCardId,
  onSelect,
  onAdd,
  onRemove,
  onReorder,
}: CardListProps) {
  const rotation = rotationCards(config);
  const alertsAndMuted = nonRotationCards(config);
  const [draggedId, setDraggedId] = useState<string | null>(null);
  const atCapacity = config.cards.length >= MAX_CARDS;

  const fullIndexOf = (cardId: string) => config.cards.findIndex((card) => card.id === cardId);
  const moveTo = (cardId: string, targetCardId: string) => {
    onReorder(moveCard(config, cardId, fullIndexOf(targetCardId)));
  };

  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    const delta = cardMoveFromKey(event.key, event.altKey);
    if (delta === 0) {
      return;
    }
    const neighbor = rotation[index + delta];
    if (!neighbor) {
      return;
    }
    event.preventDefault();
    moveTo(rotation[index].id, neighbor.id);
  };
  const onDragStart = (event: DragEvent<HTMLLIElement>, cardId: string) => {
    setDraggedId(cardId);
    event.dataTransfer.effectAllowed = "move";
    event.dataTransfer.setData("text/plain", cardId);
  };
  const onDrop = (event: DragEvent<HTMLLIElement>, targetCardId: string) => {
    event.preventDefault();
    const cardId = draggedId ?? event.dataTransfer.getData("text/plain");
    if (cardId && cardId !== targetCardId) {
      moveTo(cardId, targetCardId);
    }
    setDraggedId(null);
  };

  return (
    <section className="panel card-list-panel" aria-labelledby="card-list-heading">
      <div className="panel-heading">
        <div>
          <p className="step-label">Cards</p>
          <h2 id="card-list-heading">What should it show?</h2>
        </div>
        <span className="count-badge numeral">
          {config.cards.length}/{MAX_CARDS}
        </span>
      </div>

      <div className="card-list-section">
        <div className="card-list-section__heading">
          <p className="section-label">In rotation</p>
          <span className="keyboard-hint">⌥ ↑ ↓ to move</span>
        </div>
        {rotation.length === 0 ? (
          <div className="empty-state">
            <strong>Your rotation is empty</strong>
            <span>Add a card below to start the loop.</span>
          </div>
        ) : (
          <ol className="card-list" aria-label="In rotation">
            {rotation.map((card, index) => (
              <li
                key={card.id}
                draggable
                className={`card-row${selectedCardId === card.id ? " is-selected" : ""}${draggedId === card.id ? " is-dragging" : ""}`}
                onDragStart={(event) => onDragStart(event, card.id)}
                onDragEnd={() => setDraggedId(null)}
                onDragOver={(event) => event.preventDefault()}
                onDrop={(event) => onDrop(event, card.id)}
              >
                <span className="card-row__handle" aria-hidden="true">
                  ⠿
                </span>
                <span className="card-row__index numeral">{index + 1}</span>
                <button
                  type="button"
                  className="card-row__body"
                  aria-pressed={selectedCardId === card.id}
                  onClick={() => onSelect(card.id)}
                  onKeyDown={(event) => onKeyDown(event, index)}
                >
                  <strong>{cardName(card)}</strong>
                  <small>{cardKindName(card.kind)}</small>
                </button>
                <span className="card-row__dwell numeral">{dwellLabel(card)}</span>
                <span className="card-row__moves">
                  <button
                    type="button"
                    aria-label={`Move ${cardName(card)} up`}
                    disabled={index === 0}
                    onClick={(event) => {
                      event.stopPropagation();
                      moveTo(card.id, rotation[index - 1].id);
                    }}
                  >
                    ↑
                  </button>
                  <button
                    type="button"
                    aria-label={`Move ${cardName(card)} down`}
                    disabled={index === rotation.length - 1}
                    onClick={(event) => {
                      event.stopPropagation();
                      moveTo(card.id, rotation[index + 1].id);
                    }}
                  >
                    ↓
                  </button>
                </span>
                <button
                  type="button"
                  className="card-row__remove text-button text-button--danger"
                  aria-label={`Remove ${cardName(card)}`}
                  onClick={() => onRemove(card.id)}
                >
                  Remove
                </button>
              </li>
            ))}
          </ol>
        )}
      </div>

      <div className="card-list-section">
        <p className="section-label">Alerts and muted</p>
        {alertsAndMuted.length === 0 ? (
          <div className="empty-state">
            <span>Cards you mute or make alert-only appear here.</span>
          </div>
        ) : (
          <ul className="card-list" aria-label="Alerts and muted">
            {alertsAndMuted.map((card) => (
              <li
                key={card.id}
                className={`card-row card-row--unordered${selectedCardId === card.id ? " is-selected" : ""}`}
              >
                <button
                  type="button"
                  className="card-row__body"
                  aria-pressed={selectedCardId === card.id}
                  onClick={() => onSelect(card.id)}
                >
                  <strong>{cardName(card)}</strong>
                  <small>
                    {cardKindName(card.kind)} · {presenceLabel(card)}
                  </small>
                </button>
                <button
                  type="button"
                  className="card-row__remove text-button text-button--danger"
                  aria-label={`Remove ${cardName(card)}`}
                  onClick={() => onRemove(card.id)}
                >
                  Remove
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>

      <fieldset className="card-add">
        <legend className="sr-only">Add a card</legend>
        {addableKinds.map(({ kind, glyph, description }) => (
          <button
            className="add-card"
            type="button"
            key={kind}
            onClick={() => onAdd(kind)}
            disabled={atCapacity}
          >
            <span className="add-card__glyph numeral" aria-hidden="true">
              {glyph}
            </span>
            <span>
              <strong>Add {cardKindName(kind)}</strong>
              <small>{description}</small>
            </span>
            <span aria-hidden="true">+</span>
          </button>
        ))}
      </fieldset>
    </section>
  );
}
