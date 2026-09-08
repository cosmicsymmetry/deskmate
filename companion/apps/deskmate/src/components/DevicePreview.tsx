import { useEffect, useRef, useState } from "react";

import { renderCardPreview } from "../lib/tauri";
import type { CardSettings, DisplayOrientation } from "../lib/types";

interface DevicePreviewProps {
  cards: CardSettings[];
  selectedWidgetId: string | null;
  orientation: DisplayOrientation;
  dataGeneration: number; // bump when card_data changes; triggers re-render
}

const LIVE_TEMPLATES = new Set(["clock", "pomodoro"]);

export function DevicePreview({
  cards,
  selectedWidgetId,
  orientation,
  dataGeneration,
}: DevicePreviewProps) {
  const widget = cards.find((card) => card.id === selectedWidgetId) ?? cards[0];
  const [frame, setFrame] = useState<{ pngBase64: string | null; sample: boolean } | null>(null);
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
          setFrame({ pngBase64: rendered.png_base64, sample: rendered.sample });
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
    // Live templates re-render at 1 Hz so the clock ticks (spec §2.5).
    const interval = LIVE_TEMPLATES.has(widget.kind) ? window.setInterval(tick, 1000) : undefined;
    return () => {
      cancelled = true;
      if (interval) window.clearInterval(interval);
    };
  }, [widget?.id, widget?.kind, orientation, dataGeneration]);

  return (
    /* No caption, no resolution, no label. The panel is the one object in the
       window that looks exactly like the thing it represents, so it identifies
       itself; anything printed around it was describing what you can already see. */
    <section className="stage" aria-label="What the display is showing">
      <div className="stage__frame">
        <div className="stage__screen">
          {unavailable || !widget ? (
            <p className="stage__unavailable">
              {widget ? "Preview unavailable" : "No cards configured"}
            </p>
          ) : frame?.pngBase64 ? (
            <img
              alt={`Device preview of ${widget.id}`}
              width={448}
              height={368}
              src={`data:image/png;base64,${frame.pngBase64}`}
            />
          ) : null}
          {/* The badge means "a real frame, drawn from sample data". A frame with no
              pixels has nothing to badge; Step 14 gives that case its own word. */}
          {frame?.sample && frame.pngBase64 && <span className="stage__badge">No data yet</span>}
        </div>
      </div>
    </section>
  );
}
