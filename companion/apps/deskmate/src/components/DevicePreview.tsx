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
  const [frame, setFrame] = useState<{ pngBase64: string; sample: boolean } | null>(null);
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
        if (!cancelled) setUnavailable(true); // explicit, never stale (spec §3.3)
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
    <section className="preview-panel" aria-labelledby="preview-heading">
      <div className="preview-heading">
        <div>
          <p className="step-label">Device preview</p>
          <h2 id="preview-heading">448 × 368 landscape</h2>
        </div>
        <span>Rendered by the device's own templates · exact pixels</span>
      </div>
      <div className="device-shell">
        <div className="device-screen">
          {unavailable || !widget ? (
            <p className="preview-unavailable">
              {widget ? "Preview unavailable" : "No cards configured"}
            </p>
          ) : frame ? (
            <img
              alt={`Device preview of ${widget.id}`}
              width={448}
              height={368}
              src={`data:image/png;base64,${frame.pngBase64}`}
            />
          ) : null}
          {frame?.sample && <span className="preview-sample-badge">No data yet</span>}
        </div>
        <span className="device-port" aria-hidden="true" />
      </div>
    </section>
  );
}
