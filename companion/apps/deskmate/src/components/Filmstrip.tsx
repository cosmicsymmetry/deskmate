import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type DragEvent,
  type KeyboardEvent,
} from "react";

import {
  cardMoveFromKey,
  filmstripAdvance,
  filmstripDeadline,
  filmstripSegments,
  formatDuration,
  moveCardWithinRotation,
} from "../lib/configDraft";
import type { AppConfig } from "../lib/types";

interface FilmstripProps {
  config: AppConfig;
  selectedCardId: string | null;
  onSelect: (cardId: string) => void;
  onReorder: (config: AppConfig) => void;
}

const TICK_MS = 1000;

/// The loop ribbon: one segment per in-rotation card, its width proportional
/// to its resolved dwell — the one place in the app that shows a card's
/// share of the loop, which no other control does. All the math (segment
/// widths, playback advance decisions) lives in the pure helpers this
/// component calls; it only wires them to drag/click/keyboard events and a
/// deadline-driven timer.
export function Filmstrip({ config, selectedCardId, onSelect, onReorder }: FilmstripProps) {
  const isTimed = config.carousel.advance.kind === "timed";
  // Memoized on `config` alone (not recomputed on every render) so its
  // identity — and therefore every value derived from it below — stays
  // stable across re-renders that App triggers for unrelated reasons, most
  // notably a running pomodoro publishing a new snapshot roughly once a
  // second. Without this, a fresh array/objects every render would make
  // any effect keyed on them look "changed" on every tick.
  const segments = useMemo(() => filmstripSegments(config), [config]);
  const total = isTimed ? segments.reduce((sum, segment) => sum + segment.dwellSeconds, 0) : null;
  const [isPlaying, setIsPlaying] = useState(false);
  const [draggedId, setDraggedId] = useState<string | null>(null);
  const [reducedMotion, setReducedMotion] = useState(false);

  useEffect(() => {
    const query = window.matchMedia("(prefers-reduced-motion: reduce)");
    setReducedMotion(query.matches);
    const onChange = () => {
      setReducedMotion(query.matches);
      if (query.matches) {
        setIsPlaying(false);
      }
    };
    query.addEventListener("change", onChange);
    return () => query.removeEventListener("change", onChange);
  }, []);

  const activeCardId: string | null =
    segments.find((segment) => segment.cardId === selectedCardId)?.cardId ??
    segments[0]?.cardId ??
    null;
  const activeSegment = segments.find((segment) => segment.cardId === activeCardId);
  const activeDwellSeconds = activeSegment?.dwellSeconds ?? 0;

  // A "latest" ref, updated unconditionally on every render, so the ticking
  // callback below always resolves "what's next" against the current
  // rotation order even if a drag-reorder happens mid-play without itself
  // changing which card is currently active (which is the only thing the
  // effect below re-arms on).
  const segmentsRef = useRef(segments);
  segmentsRef.current = segments;

  // Holds an absolute deadline (`Date.now()`-epoch), not a relative
  // duration, and lives in a ref so it survives re-renders untouched. The
  // effect below only ever compares "now" against this deadline via the
  // pure `filmstripAdvance` — it never recomputes the deadline just
  // because the component happened to re-render.
  const deadlineRef = useRef<number | null>(null);

  // Steps the selection through the rotation at real dwell timing. The
  // dependency array is deliberately just the PRIMITIVES that should
  // actually restart playback — whether we're playing, which card is
  // active, and that card's own dwell — never `segments` or `activeSegment`
  // themselves. Those are freshly allocated on every render (see the
  // `useMemo` comment above for why that's still true even after
  // memoizing), and depending on them was the bug: the interval got torn
  // down and its deadline recomputed from "now" on every unrelated
  // re-render, so a 20-45s dwell could never survive long enough to fire
  // while, say, a pomodoro was ticking once a second. With primitive deps,
  // once armed, this effect is left alone across renders that don't
  // actually change what's playing.
  useEffect(() => {
    if (!isTimed || !isPlaying || activeCardId === null || segments.length < 2) {
      deadlineRef.current = null;
      return;
    }
    if (reducedMotion) {
      return;
    }
    deadlineRef.current = filmstripDeadline(Date.now(), activeDwellSeconds);
    const interval = window.setInterval(() => {
      if (deadlineRef.current === null) {
        return;
      }
      const result = filmstripAdvance(
        segmentsRef.current,
        activeCardId,
        deadlineRef.current,
        Date.now(),
      );
      if (result) {
        deadlineRef.current = result.deadlineMs;
        onSelect(result.cardId);
      }
    }, TICK_MS);
    return () => {
      window.clearInterval(interval);
    };
  }, [
    isTimed,
    isPlaying,
    activeCardId,
    activeDwellSeconds,
    segments.length,
    reducedMotion,
    onSelect,
  ]);

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
        <div className="filmstrip-play-row">
          <button
            type="button"
            className="filmstrip-play"
            aria-pressed={isPlaying}
            disabled={reducedMotion}
            onClick={() => setIsPlaying((value) => !value)}
          >
            {isPlaying ? "Pause" : "Play"}
          </button>
          {reducedMotion && (
            <span className="filmstrip-play-hint">
              Automatic playback is off because reduced motion is on.
            </span>
          )}
        </div>
      )}
    </section>
  );
}
