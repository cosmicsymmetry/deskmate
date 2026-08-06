import {
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type DragEvent,
  type KeyboardEvent,
} from "react";

import {
  cardMoveFromKey,
  filmstripSegments,
  formatDuration,
  moveCardWithinRotation,
  nextFilmstripCardId,
} from "../lib/configDraft";
import type { AppConfig } from "../lib/types";

interface FilmstripProps {
  config: AppConfig;
  selectedCardId: string | null;
  onSelect: (cardId: string) => void;
  onReorder: (config: AppConfig) => void;
}

/// The loop ribbon: one segment per in-rotation card, its width proportional
/// to its resolved dwell — the one place in the app that shows a card's
/// share of the loop, which no other control does. All the math (segment
/// widths, playback order) lives in the pure helpers this component calls;
/// it only wires them to drag/click/keyboard events and a real-time timer.
export function Filmstrip({ config, selectedCardId, onSelect, onReorder }: FilmstripProps) {
  const isTimed = config.carousel.advance.kind === "timed";
  const segments = filmstripSegments(config);
  const total = isTimed ? segments.reduce((sum, segment) => sum + segment.dwellSeconds, 0) : null;
  const [isPlaying, setIsPlaying] = useState(false);
  const [draggedId, setDraggedId] = useState<string | null>(null);
  const reducedMotionRef = useRef(false);

  useEffect(() => {
    const query = window.matchMedia("(prefers-reduced-motion: reduce)");
    reducedMotionRef.current = query.matches;
    const onChange = () => {
      reducedMotionRef.current = query.matches;
      if (query.matches) {
        setIsPlaying(false);
      }
    };
    query.addEventListener("change", onChange);
    return () => query.removeEventListener("change", onChange);
  }, []);

  const activeCardId =
    segments.find((segment) => segment.cardId === selectedCardId)?.cardId ?? segments[0]?.cardId;
  const activeSegment = segments.find((segment) => segment.cardId === activeCardId);

  // Steps the selection to the next card after the active segment's real
  // dwell — this is what makes the play control move "at real dwell
  // timing" rather than some fixed tick, and why it re-arms every time the
  // active segment (or the dwell that owns it) changes.
  useEffect(() => {
    if (!isTimed || !isPlaying || !activeSegment || segments.length < 2) {
      return;
    }
    if (reducedMotionRef.current) {
      return;
    }
    const timeout = window.setTimeout(() => {
      const next = nextFilmstripCardId(segments, activeSegment.cardId);
      if (next) {
        onSelect(next);
      }
    }, Math.max(1, activeSegment.dwellSeconds) * 1000);
    return () => window.clearTimeout(timeout);
  }, [isTimed, isPlaying, activeSegment, segments, onSelect]);

  if (segments.length === 0) {
    return (
      <section className="filmstrip-panel" aria-labelledby="filmstrip-heading">
        <p className="step-label" id="filmstrip-heading">
          Rotation loop
        </p>
        <p className="filmstrip-empty">Nothing is in rotation yet.</p>
      </section>
    );
  }

  const moveTo = (cardId: string, targetIndex: number) => {
    onReorder(moveCardWithinRotation(config, cardId, targetIndex));
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
      moveTo(cardId, targetIndex);
    }
    setDraggedId(null);
  };
  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    const delta = cardMoveFromKey(event.key, event.altKey);
    if (delta === 0 || !segments[index + delta]) {
      return;
    }
    event.preventDefault();
    moveTo(segments[index].cardId, index + delta);
  };

  return (
    <section className="filmstrip-panel" aria-labelledby="filmstrip-heading">
      <div className="filmstrip-heading">
        <p className="step-label" id="filmstrip-heading">
          {isTimed ? "Rotation loop" : "Rotation order"}
        </p>
        {isTimed && total !== null && (
          <span className="filmstrip-total numeral">{formatDuration(total)}</span>
        )}
      </div>
      <div className="filmstrip-scroll">
        <ol className="filmstrip-track" aria-label="Cards in rotation">
          {segments.map((segment, index) => (
            <li
              key={segment.cardId}
              draggable
              className={`filmstrip-segment${draggedId === segment.cardId ? " is-dragging" : ""}`}
              style={{ flexGrow: segment.widthPercent } as CSSProperties}
              onDragStart={(event) => onDragStart(event, segment.cardId)}
              onDragEnd={() => setDraggedId(null)}
              onDragOver={(event) => event.preventDefault()}
              onDrop={(event) => onDrop(event, index)}
            >
              <button
                type="button"
                className={`filmstrip-segment__button${segment.cardId === selectedCardId ? " is-selected" : ""}`}
                aria-pressed={segment.cardId === selectedCardId}
                onClick={() => onSelect(segment.cardId)}
                onKeyDown={(event) => onKeyDown(event, index)}
              >
                <span className="filmstrip-segment__name">{segment.name}</span>
                {isTimed && (
                  <span className="filmstrip-segment__dwell numeral">{segment.dwellSeconds}s</span>
                )}
              </button>
            </li>
          ))}
        </ol>
        {isTimed && activeSegment && (
          <span
            className="filmstrip-playhead"
            aria-hidden="true"
            style={{
              left: `${activeSegment.offsetPercent}%`,
              transitionDuration: isPlaying ? `${Math.max(1, activeSegment.dwellSeconds)}s` : "0s",
            }}
          />
        )}
      </div>
      {isTimed && segments.length > 1 && (
        <button
          type="button"
          className="filmstrip-play"
          aria-pressed={isPlaying}
          onClick={() => setIsPlaying((value) => !value)}
        >
          {isPlaying ? "Pause" : "Play"}
        </button>
      )}
    </section>
  );
}
