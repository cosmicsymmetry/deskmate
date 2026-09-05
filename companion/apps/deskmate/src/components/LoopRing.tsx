import { useEffect, useMemo, useRef, useState, type DragEvent, type KeyboardEvent } from "react";

import {
  activePlaylist,
  cardMoveFromKey,
  filmstripAdvance,
  filmstripDeadline,
  filmstripSegments,
  formatDuration,
  loopSeconds,
  moveEntry,
} from "../lib/configDraft";
import { Icon } from "./Icon";
import type { AppConfig } from "../lib/types";

interface LoopRingProps {
  config: AppConfig;
  selectedCardId: string | null;
  onSelect: (cardId: string) => void;
  onReorder: (config: AppConfig) => void;
}

const TICK_MS = 1000;
const RADIUS = 82;
const CIRCUMFERENCE = 2 * Math.PI * RADIUS;
const GAP_DEGREES = 2.4;

/**
 * A segment's position along the `--arc-a` → `--arc-b` ramp, as 0–1.
 *
 * This used to be `index % 4`, which repeated every fifth entry — and a playlist holds
 * up to eight. Two arcs sharing a colour breaks the only mapping there is from an arc
 * back to its name in the legend.
 */
/** Two entries can share a template, so a move control has to say which one. */
function entryLabel(arc: { name: string; title: string | null }): string {
  return arc.title ? `${arc.name} — ${arc.title}` : arc.name;
}

function rampStep(index: number, count: number): number {
  return count < 2 ? 0 : index / (count - 1);
}

/**
 * The loop total as a clock reading rather than prose.
 *
 * `formatDuration` gives "1 min 50 s", which is the right form in a sentence and the
 * wrong one inside a ring: it wraps at the size this hero wants to be. The full
 * phrasing still reaches assistive technology through the ring's `aria-label`.
 */
function compactDuration(totalSeconds: number): string {
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = Math.floor(totalSeconds % 60);
  return `${minutes}:${String(seconds).padStart(2, "0")}`;
}

/**
 * The loop ring: one arc per active-playlist entry, its sweep proportional to that
 * entry's resolved dwell, with the on-panel entry at full luminance and a marker
 * riding it. It is the successor to the filmstrip and keeps the same truth —
 * a card's share of the loop, which no other control shows — in the form the rest
 * of this world is built from.
 *
 * All the arithmetic still comes from the pure helpers in `configDraft`
 * (`filmstripSegments`, `loopSeconds`, `filmstripAdvance`, `filmstripDeadline`), so
 * the ring and any other consumer of loop length can never drift apart. This
 * component only maps those numbers onto a circle and wires the events.
 *
 * The ring is the display; the legend beneath it carries selection and reordering.
 * Dragging arcs around a circle has no keyboard equivalent worth shipping, and the
 * reorder affordance has to stay reachable — so the legend keeps drag, the arrow
 * buttons, and the same Alt+Up/Down the playlist editor uses.
 */
export function LoopRing({ config, selectedCardId, onSelect, onReorder }: LoopRingProps) {
  const playlist = activePlaylist(config);
  const isTimed = playlist?.advance.kind === "timed";
  const segments = useMemo(() => filmstripSegments(config), [config]);
  const total = playlist ? loopSeconds(config, playlist.id) : null;
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

  const segmentsRef = useRef(segments);
  segmentsRef.current = segments;
  const deadlineRef = useRef<number | null>(null);

  // Primitive deps only, for the same reason the filmstrip had them: `segments` and
  // `activeSegment` are freshly allocated every render, so depending on them tore
  // down and re-armed the interval on every unrelated re-render — and a running
  // pomodoro re-renders this component once a second, which meant a 20-45s dwell
  // could never survive long enough to fire.
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
      <section className="loop" aria-labelledby="loop-heading">
        <p className="tile-label" id="loop-heading">
          The loop
        </p>
        <p className="loop__empty">
          {playlist ? `${playlist.name} has no cards yet.` : "No playlist is active."}
        </p>
      </section>
    );
  }

  const moveTo = (cardId: string, targetSegmentIndex: number) => {
    if (!playlist) {
      return;
    }
    const targetCardId = segments[targetSegmentIndex]?.cardId;
    if (!targetCardId) {
      return;
    }
    const sourceIndex = playlist.entries.findIndex((entry) => entry.card_id === cardId);
    const targetEntryIndex = playlist.entries.findIndex((entry) => entry.card_id === targetCardId);
    if (sourceIndex < 0 || targetEntryIndex < 0) {
      return;
    }
    onReorder(moveEntry(config, playlist.id, sourceIndex, targetEntryIndex));
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

  // Equal arcs when the playlist advances manually: with no dwell there is no
  // proportion to encode, and a ring that implied one would be inventing it.
  const share = (widthPercent: number) => (isTimed ? widthPercent / 100 : 1 / segments.length);

  let sweptDegrees = 0;
  const arcs = segments.map((segment) => {
    const startDegrees = sweptDegrees;
    const spanDegrees = share(segment.widthPercent) * 360;
    sweptDegrees += spanDegrees;
    const paintedDegrees = Math.max(spanDegrees - GAP_DEGREES, 1.5);
    return {
      ...segment,
      startDegrees,
      spanDegrees,
      dash: (paintedDegrees / 360) * CIRCUMFERENCE,
      offset: -(startDegrees / 360) * CIRCUMFERENCE,
      midDegrees: startDegrees + spanDegrees / 2,
    };
  });
  const activeArc = arcs.find((arc) => arc.cardId === activeCardId) ?? arcs[0];
  // Computed in the SVG's own rotated space: the element is already turned -90deg
  // so that arcs begin at twelve o'clock, and subtracting it again here put the
  // marker a quarter-turn behind the arc it marks.
  const markerAngle = (activeArc.startDegrees * Math.PI) / 180;

  return (
    <section className="loop" aria-labelledby="loop-heading">
      <div className="loop__head">
        <p className="tile-label" id="loop-heading">
          {playlist?.name}
        </p>
        <div className="loop__head-right">
          <span className="tile-label">{isTimed ? "Timed loop" : "Manual order"}</span>
          {isTimed && segments.length > 1 && (
            <div className="loop__transport">
              <button
                type="button"
                className={`loop__play${isPlaying ? " is-playing" : ""}`}
                aria-pressed={isPlaying}
                disabled={reducedMotion}
                onClick={() => setIsPlaying((value) => !value)}
              >
                {isPlaying ? "Pause preview" : "Play the loop"}
              </button>
              {reducedMotion && (
                <span className="loop__hint">Playback is off while reduced motion is on.</span>
              )}
            </div>
          )}
        </div>
      </div>

      <div className="loop__body">
        <div className="loop__ring">
          <svg
            viewBox="0 0 200 200"
            role="img"
            aria-label={ringSummary(segments.length, total, isTimed)}
          >
            <circle className="loop__track" cx="100" cy="100" r={RADIUS} />
            {arcs.map((arc, index) => (
              <circle
                key={arc.cardId}
                className={`loop__arc${arc.cardId === activeCardId ? " is-live" : ""}`}
                cx="100"
                cy="100"
                r={RADIUS}
                style={{
                  strokeDasharray: `${arc.dash} ${CIRCUMFERENCE - arc.dash}`,
                  strokeDashoffset: arc.offset,
                  // A shallow ramp inside one hue family, so neighbouring arcs stay
                  // tellable apart without colour ever becoming the state channel —
                  // luminance is what says "on the panel now".
                  ["--arc-step" as string]: String(rampStep(index, segments.length)),
                }}
              />
            ))}
            <circle
              className="loop__marker"
              cx={100 + RADIUS * Math.cos(markerAngle)}
              cy={100 + RADIUS * Math.sin(markerAngle)}
              r="7"
            />
          </svg>
          <div className="loop__centre">
            <strong className="loop__total numeral">
              {isTimed && total !== null ? compactDuration(total) : String(segments.length)}
            </strong>
            <span className="tile-label">
              {isTimed ? "Loop length" : segments.length === 1 ? "Card" : "Cards"}
            </span>
          </div>
        </div>
        <ol className="loop__legend" aria-label="Cards in the active playlist">
          {arcs.map((arc, index) => (
            <li
              key={arc.cardId}
              draggable
              className={`loop__entry${draggedId === arc.cardId ? " is-dragging" : ""}${
                arc.cardId === activeCardId ? " is-live" : ""
              }`}
              onDragStart={(event) => onDragStart(event, arc.cardId)}
              onDragEnd={() => setDraggedId(null)}
              onDragOver={(event) => event.preventDefault()}
              onDrop={(event) => onDrop(event, index)}
            >
              <span
                className="loop__swatch"
                aria-hidden="true"
                style={{ ["--arc-step" as string]: String(rampStep(index, arcs.length)) }}
              />
              <button
                type="button"
                className="loop__entry-body"
                aria-pressed={arc.cardId === selectedCardId}
                onClick={() => onSelect(arc.cardId)}
                onKeyDown={(event) => onKeyDown(event, index)}
              >
                <span className="loop__entry-text">
                  <span className="loop__entry-name">{arc.name}</span>
                  {arc.title && <span className="loop__entry-title">{arc.title}</span>}
                </span>
                {isTimed && <span className="loop__entry-dwell numeral">{arc.dwellSeconds}s</span>}
              </button>
              <span className="loop__moves">
                <button
                  type="button"
                  aria-label={`Move ${entryLabel(arc)} earlier`}
                  disabled={index === 0}
                  onClick={() => moveTo(arc.cardId, index - 1)}
                >
                  <Icon name="up" />
                </button>
                <button
                  type="button"
                  aria-label={`Move ${entryLabel(arc)} later`}
                  disabled={index === arcs.length - 1}
                  onClick={() => moveTo(arc.cardId, index + 1)}
                >
                  <Icon name="down" />
                </button>
              </span>
            </li>
          ))}
        </ol>
      </div>
    </section>
  );
}

function ringSummary(count: number, total: number | null, isTimed: boolean): string {
  const cards = `${count} card${count === 1 ? "" : "s"}`;
  return isTimed && total !== null
    ? `Loop ring: ${cards}, ${formatDuration(total)} total`
    : `Loop ring: ${cards} in manual order`;
}
