import { useState, type DragEvent, type KeyboardEvent } from "react";

import { widgetName } from "../lib/configDraft";
import type { CardSettings } from "../lib/types";

interface ScreenArrangerProps {
  cards: CardSettings[];
  selectedWidgetId: string | null;
  onSelect: (widgetId: string) => void;
  onReorder: (cards: CardSettings[]) => void;
}

function moveCard(cards: CardSettings[], cardId: string, targetIndex: number): CardSettings[] {
  const sourceIndex = cards.findIndex((card) => card.id === cardId);
  if (sourceIndex < 0 || cards.length < 2) {
    return cards;
  }
  const boundedTarget = Math.max(0, Math.min(targetIndex, cards.length - 1));
  if (sourceIndex === boundedTarget) {
    return cards;
  }
  const reordered = [...cards];
  const [moved] = reordered.splice(sourceIndex, 1);
  reordered.splice(boundedTarget, 0, moved);
  return reordered;
}

function moveFromKey(key: string, altKey: boolean): -1 | 0 | 1 {
  if (!altKey) {
    return 0;
  }
  if (key === "ArrowUp") {
    return -1;
  }
  if (key === "ArrowDown") {
    return 1;
  }
  return 0;
}

export function ScreenArranger({
  cards,
  selectedWidgetId,
  onSelect,
  onReorder,
}: ScreenArrangerProps) {
  const [draggedId, setDraggedId] = useState<string | null>(null);

  const move = (cardId: string, targetIndex: number) => {
    onReorder(moveCard(cards, cardId, targetIndex));
  };
  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>, cardId: string, index: number) => {
    const delta = moveFromKey(event.key, event.altKey);
    if (delta === 0) {
      return;
    }
    event.preventDefault();
    move(cardId, index + delta);
  };
  const onDragStart = (event: DragEvent<HTMLLIElement>, cardId: string) => {
    setDraggedId(cardId);
    event.dataTransfer.effectAllowed = "move";
    event.dataTransfer.setData("text/plain", cardId);
  };
  const onDrop = (event: DragEvent<HTMLLIElement>, targetIndex: number) => {
    event.preventDefault();
    const cardId = draggedId ?? event.dataTransfer.getData("text/plain");
    if (cardId) {
      move(cardId, targetIndex);
    }
    setDraggedId(null);
  };

  return (
    <section className="panel arranger-panel" aria-labelledby="screens-heading">
      <div className="panel-heading panel-heading--compact">
        <div>
          <p className="step-label">3 · Screens</p>
          <h2 id="screens-heading">Choose the swipe order</h2>
        </div>
        <span className="keyboard-hint">⌥ ↑ ↓ to move</span>
      </div>

      {cards.length === 0 ? (
        <div className="empty-state">
          <strong>No screens yet</strong>
          <span>Each new widget gets one screen automatically.</span>
        </div>
      ) : (
        <ol className="screen-list" aria-label="Screen order">
          {cards.map((card, index) => (
            <li
              key={card.id}
              draggable
              className={`${selectedWidgetId === card.id ? "is-selected" : ""}${draggedId === card.id ? " is-dragging" : ""}`}
              onDragStart={(event) => onDragStart(event, card.id)}
              onDragEnd={() => setDraggedId(null)}
              onDragOver={(event) => event.preventDefault()}
              onDrop={(event) => onDrop(event, index)}
              aria-label={`Screen ${index + 1}: ${widgetName(card)}`}
            >
              <span className="drag-handle" aria-hidden="true">
                ⠿
              </span>
              <span className="screen-number">{index + 1}</span>
              <button
                type="button"
                className="screen-name"
                onClick={() => onSelect(card.id)}
                onKeyDown={(event) => onKeyDown(event, card.id, index)}
              >
                <strong>{widgetName(card)}</strong>
                <small>{card.id}</small>
              </button>
              <span className="move-buttons">
                <button
                  type="button"
                  aria-label={`Move ${widgetName(card)} up`}
                  disabled={index === 0}
                  onClick={(event) => {
                    event.stopPropagation();
                    move(card.id, index - 1);
                  }}
                >
                  ↑
                </button>
                <button
                  type="button"
                  aria-label={`Move ${widgetName(card)} down`}
                  disabled={index === cards.length - 1}
                  onClick={(event) => {
                    event.stopPropagation();
                    move(card.id, index + 1);
                  }}
                >
                  ↓
                </button>
              </span>
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}
