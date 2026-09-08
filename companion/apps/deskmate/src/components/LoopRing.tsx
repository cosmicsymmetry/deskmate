import { useEffect, useMemo, useRef, useState } from "react";

import {
  activePlaylist,
  loopAdvance,
  loopDeadline,
  loopSegments,
  formatDuration,
  issuesForPath,
  loopSeconds,
  setPlaylistAdvance,
} from "../lib/configDraft";
import type { AppConfig, PluginCatalog, ValidationIssue } from "../lib/types";
import { FieldIssues } from "./FieldIssues";

interface LoopRingProps {
  config: AppConfig;
  issues: ValidationIssue[];
  /** The server's plugin registry, so a plugin arc is named the same here as elsewhere. */
  catalog: PluginCatalog | null;
  selectedCardId: string | null;
  onSelect: (cardId: string) => void;
  onChange: (config: AppConfig) => void;
}

const DEFAULT_DWELL_SECONDS = 20;
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
 * riding it. It carries the one truth no other control shows: a card's share of
 * the loop.
 *
 * All the arithmetic still comes from the pure helpers in `configDraft`
 * (`loopSegments`, `loopSeconds`, `loopAdvance`, `loopDeadline`), so
 * the ring and any other consumer of loop length can never drift apart. This
 * component only maps those numbers onto a circle and wires the events.
 *
 * The ring and legend display the loop; the grid is the one place its order changes.
 */
export function LoopRing({
  config,
  issues,
  catalog,
  selectedCardId,
  onSelect,
  onChange,
}: LoopRingProps) {
  const playlist = activePlaylist(config);
  const playlistIndex = config.playlists.findIndex(
    (candidate) => candidate.id === config.active_playlist_id,
  );
  const isTimed = playlist?.advance.kind === "timed";
  const segments = useMemo(() => loopSegments(config, catalog), [config, catalog]);
  const total = playlist ? loopSeconds(config, playlist.id) : null;
  const [isPlaying, setIsPlaying] = useState(false);
  const [reducedMotion, setReducedMotion] = useState(false);
  const configuredDefaultDwell =
    playlist?.advance.kind === "timed" ? String(playlist.advance.default_dwell_seconds) : "";
  const [defaultDwellInput, setDefaultDwellInput] = useState(configuredDefaultDwell);

  useEffect(() => {
    setDefaultDwellInput(configuredDefaultDwell);
  }, [configuredDefaultDwell]);

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

  // Primitive deps only: `segments` and
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
    deadlineRef.current = loopDeadline(Date.now(), activeDwellSeconds);
    const interval = window.setInterval(() => {
      if (deadlineRef.current === null) {
        return;
      }
      const result = loopAdvance(
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

  const advanceIssues =
    playlistIndex < 0 ? [] : issuesForPath(issues, `playlists[${playlistIndex}].advance`);
  const defaultDwellPath = `playlists[${playlistIndex}].advance.default_dwell_seconds`;
  const pacing = playlist && (
    <>
      <fieldset className="loop__pacing">
        <legend className="sr-only">Pacing</legend>
        <button
          type="button"
          className="loop__pace"
          aria-pressed={isTimed}
          onClick={() => {
            if (!isTimed) {
              onChange(
                setPlaylistAdvance(config, playlist.id, {
                  kind: "timed",
                  default_dwell_seconds: DEFAULT_DWELL_SECONDS,
                }),
              );
            }
          }}
        >
          Timed
        </button>
        <button
          type="button"
          className="loop__pace"
          aria-pressed={!isTimed}
          onClick={() => {
            if (isTimed) {
              onChange(setPlaylistAdvance(config, playlist.id, { kind: "manual" }));
            }
          }}
        >
          Manual
        </button>
      </fieldset>
      {playlist.advance.kind === "timed" && (
        <label className="loop__dwell">
          <span className="sr-only">Default dwell in seconds</span>
          <input
            className="numeral"
            type="number"
            inputMode="numeric"
            min={5}
            max={3600}
            step={1}
            value={defaultDwellInput}
            onChange={(event) => {
              const raw = event.currentTarget.value;
              setDefaultDwellInput(raw);
              if (raw.trim() === "") {
                return;
              }
              const parsed = Number(raw);
              if (Number.isFinite(parsed)) {
                onChange(
                  setPlaylistAdvance(config, playlist.id, {
                    kind: "timed",
                    default_dwell_seconds: parsed,
                  }),
                );
              }
            }}
            aria-invalid={advanceIssues.some((issue) => issue.path === defaultDwellPath)}
            aria-describedby={advanceIssues.length > 0 ? "loop-pacing-issues" : undefined}
          />
          <span aria-hidden="true">s</span>
        </label>
      )}
    </>
  );

  const head = (
    <>
      <div className="loop__head">
        <p className="tile-label" id="loop-heading">
          The loop
        </p>
        <div className="loop__head-right">
          {pacing}
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
            </div>
          )}
        </div>
        {isTimed && segments.length > 1 && reducedMotion && (
          <span className="loop__hint">Playback is off while reduced motion is on.</span>
        )}
      </div>
      <FieldIssues issues={advanceIssues} className="loop__pacing-issues" id="loop-pacing-issues" />
    </>
  );

  if (segments.length === 0) {
    return (
      <section className="loop" aria-labelledby="loop-heading">
        {head}
        <p className="loop__empty">No cards yet.</p>
      </section>
    );
  }

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
      {head}

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
        <ol className="loop__legend" aria-label="Cards in the loop">
          {arcs.map((arc, index) => (
            <li
              key={arc.cardId}
              className={`loop__entry${arc.cardId === activeCardId ? " is-live" : ""}`}
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
              >
                <span className="loop__entry-text">
                  <span className="loop__entry-name">{arc.name}</span>
                  {arc.title && <span className="loop__entry-title">{arc.title}</span>}
                </span>
                {isTimed && <span className="loop__entry-dwell numeral">{arc.dwellSeconds}s</span>}
              </button>
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
