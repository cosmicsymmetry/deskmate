import { useState, type DragEvent, type KeyboardEvent } from "react";

import {
  moveScreen,
  primaryScreenWidgetId,
  screenMoveFromKey,
  widgetName,
} from "../lib/configDraft";
import type { ScreenSettings, WidgetSettings } from "../lib/types";

interface ScreenArrangerProps {
  screens: ScreenSettings[];
  widgets: WidgetSettings[];
  selectedWidgetId: string | null;
  onSelect: (widgetId: string) => void;
  onReorder: (screens: ScreenSettings[]) => void;
}

export function ScreenArranger({
  screens,
  widgets,
  selectedWidgetId,
  onSelect,
  onReorder,
}: ScreenArrangerProps) {
  const [draggedId, setDraggedId] = useState<string | null>(null);
  const widgetById = new Map(widgets.map((widget) => [widget.id, widget]));

  const move = (screenId: string, targetIndex: number) => {
    onReorder(moveScreen(screens, screenId, targetIndex));
  };
  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>, screenId: string, index: number) => {
    const delta = screenMoveFromKey(event.key, event.altKey);
    if (delta === 0) {
      return;
    }
    event.preventDefault();
    move(screenId, index + delta);
  };
  const onDragStart = (event: DragEvent<HTMLLIElement>, screenId: string) => {
    setDraggedId(screenId);
    event.dataTransfer.effectAllowed = "move";
    event.dataTransfer.setData("text/plain", screenId);
  };
  const onDrop = (event: DragEvent<HTMLLIElement>, targetIndex: number) => {
    event.preventDefault();
    const screenId = draggedId ?? event.dataTransfer.getData("text/plain");
    if (screenId) {
      move(screenId, targetIndex);
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

      {screens.length === 0 ? (
        <div className="empty-state">
          <strong>No screens yet</strong>
          <span>Each new widget gets one screen automatically.</span>
        </div>
      ) : (
        <ol className="screen-list" aria-label="Screen order">
          {screens.map((screen, index) => {
            const widgetId = primaryScreenWidgetId(screen) ?? "";
            const widget = widgetById.get(widgetId);
            return (
              <li
                key={screen.id}
                draggable
                className={`${selectedWidgetId === widgetId ? "is-selected" : ""}${draggedId === screen.id ? " is-dragging" : ""}`}
                onDragStart={(event) => onDragStart(event, screen.id)}
                onDragEnd={() => setDraggedId(null)}
                onDragOver={(event) => event.preventDefault()}
                onDrop={(event) => onDrop(event, index)}
                aria-label={`Screen ${index + 1}: ${widget ? widgetName(widget) : widgetId}`}
              >
                <span className="drag-handle" aria-hidden="true">
                  ⠿
                </span>
                <span className="screen-number">{index + 1}</span>
                <button
                  type="button"
                  className="screen-name"
                  onClick={() => onSelect(widgetId)}
                  onKeyDown={(event) => onKeyDown(event, screen.id, index)}
                >
                  <strong>{widget ? widgetName(widget) : "Missing widget"}</strong>
                  <small>{screen.id}</small>
                </button>
                <span className="move-buttons">
                  <button
                    type="button"
                    aria-label={`Move ${widget ? widgetName(widget) : screen.id} up`}
                    disabled={index === 0}
                    onClick={(event) => {
                      event.stopPropagation();
                      move(screen.id, index - 1);
                    }}
                  >
                    ↑
                  </button>
                  <button
                    type="button"
                    aria-label={`Move ${widget ? widgetName(widget) : screen.id} down`}
                    disabled={index === screens.length - 1}
                    onClick={(event) => {
                      event.stopPropagation();
                      move(screen.id, index + 1);
                    }}
                  >
                    ↓
                  </button>
                </span>
              </li>
            );
          })}
        </ol>
      )}
    </section>
  );
}
