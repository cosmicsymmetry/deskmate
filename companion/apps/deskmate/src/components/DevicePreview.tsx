import { useEffect, useRef, useState } from "react";

import { renderCardPreview } from "../lib/backend";
import { cardIdentity } from "../lib/configDraft";
import type { CardSettings, DisplayOrientation, ImageSource } from "../lib/types";

interface DevicePreviewProps {
  cards: CardSettings[];
  selectedWidgetId: string | null;
  orientation: DisplayOrientation;
  dataGeneration: number; // bump when card_data changes; triggers re-render
  imageSources: ImageSource[];
}

/** Re-render cadence per card kind, in milliseconds. */
const REFRESH_MS: Record<string, number> = {
  // Live templates re-render at 1 Hz so the clock ticks (spec §2.5).
  clock: 1_000,
  pomodoro: 1_000,
  // A picture's preview is the frame its source last drew, and nothing tells the
  // window when a source draws. Without this a face created a moment ago says "No
  // frame yet" until the owner clicks away and back.
  picture: 10_000,
};

export function DevicePreview({
  cards,
  selectedWidgetId,
  orientation,
  dataGeneration,
  imageSources,
}: DevicePreviewProps) {
  const widget = cards.find((card) => card.id === selectedWidgetId) ?? cards[0];
  const [frame, setFrame] = useState<{
    pngBase64: string | null;
    sample: boolean;
    state: string | null;
  } | null>(null);
  const [unavailable, setUnavailable] = useState(false);
  const generation = useRef(0);

  // The dependency list below is deliberately narrower than `widget` itself —
  // `widget?.id`/`widget?.kind` are the only parts of it a re-render should react to
  // (its object identity changes on every snapshot poll even when the selected card
  // doesn't). `orientation` and `dataGeneration` aren't read in the effect body at
  // all; they're intentional re-fetch triggers, per `dataGeneration`'s own doc
  // comment on `DevicePreviewProps` above.
  // biome-ignore lint/correctness/useExhaustiveDependencies: see comment above.
  useEffect(() => {
    if (!widget) return;
    let cancelled = false;
    const tick = async () => {
      const requested = ++generation.current;
      try {
        const rendered = await renderCardPreview(widget.id);
        if (!cancelled && requested === generation.current) {
          setFrame({
            pngBase64: rendered.png_base64,
            sample: rendered.sample,
            state: rendered.state,
          });
          setUnavailable(false);
        }
      } catch {
        // Guarded the same way the success branch above is: `preview.rs`'s
        // latest-wins coalescing replies to a superseded job with `Err("superseded")`
        // *before* the winning job even starts rendering, so an older request's
        // rejection can land after a newer request has already succeeded. Without
        // this check that stale rejection would flash "Preview unavailable" over a
        // correct, already-rendered frame before self-correcting — a stale UI state
        // the spec's "never stale" rule forbids just as much as a stale frame.
        if (!cancelled && requested === generation.current) setUnavailable(true);
      }
    };
    void tick();
    const cadence = REFRESH_MS[widget.kind];
    const interval = cadence === undefined ? undefined : window.setInterval(tick, cadence);
    return () => {
      cancelled = true;
      if (interval) window.clearInterval(interval);
    };
  }, [widget?.id, widget?.kind, orientation, dataGeneration]);

  return (
    /* The recognizable panel surface identifies itself visually; the section's
       accessible name supplies the equivalent context without extra chrome. */
    <section className="stage" aria-label="What the display is showing">
      <div className="stage__frame">
        <div className="stage__screen">
          {unavailable || !widget ? (
            <p className="stage__unavailable">
              {widget ? "Preview unavailable" : "No cards configured"}
            </p>
          ) : frame?.pngBase64 ? (
            <img
              alt={`Device preview of ${cardIdentity(widget, imageSources)}`}
              width={448}
              height={368}
              src={`data:image/png;base64,${frame.pngBase64}`}
            />
          ) : frame?.state ? (
            /* A state, not a fault: "Preview unavailable" stays reserved for a
               transport failure, which is the one case the reader can do nothing
               about. */
            <p className="stage__unavailable">{frame.state}</p>
          ) : null}
          {/* The badge means "a real frame, drawn from sample data". A frame with no
              pixels has nothing to badge; Step 14 gives that case its own word. */}
          {frame?.sample && frame.pngBase64 && <span className="stage__badge">No data yet</span>}
        </div>
      </div>
    </section>
  );
}
