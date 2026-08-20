/**
 * Dev-only stand-in for `render_card_preview`.
 *
 * The real command returns exact pixels from the device's own LVGL templates. Nothing
 * in a browser can do that, so this draws a deliberate approximation at the true
 * 448x368 and every frame is reported with `sample: true`, which is the same flag the
 * real backend sets when it renders from placeholder data. The UI already labels that
 * state, so a mock frame can never be mistaken for a device frame.
 *
 * Layout rules mirror the ones the firmware actually follows, because getting those
 * wrong would make the app's composition look correct against a lie:
 *  - clock faces carry no title chip and no eyebrow (docs/design + the 2026-08-17 change)
 *  - every other template keeps its title chip
 *  - the canvas is one clean 448x368 with no status strip
 */
import type { CardDataSnapshot, CardSettings } from "../lib/types";

export const PANEL_WIDTH = 448;
export const PANEL_HEIGHT = 368;

const INK = "#FFFFFF";
const DIM = "#8A8A8E";
const GROUND = "#000000";
const EMIT = "#FFB340";

function fieldValue(data: CardDataSnapshot | undefined, key: string): string | null {
  const field = data?.fields.find((candidate) => candidate.key === key);
  if (!field) return null;
  const { value } = field;
  return value.kind === "text" ? value.value : String(value.value);
}

function chip(ctx: CanvasRenderingContext2D, text: string) {
  ctx.font = "600 19px ui-sans-serif, system-ui, sans-serif";
  const width = ctx.measureText(text).width + 28;
  const x = (PANEL_WIDTH - width) / 2;
  ctx.fillStyle = "#1C1C1E";
  ctx.beginPath();
  ctx.roundRect(x, 26, width, 38, 19);
  ctx.fill();
  ctx.fillStyle = DIM;
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillText(text, PANEL_WIDTH / 2, 46);
}

function clockFace(ctx: CanvasRenderingContext2D, showSeconds: boolean, timezone: string) {
  let now = new Date();
  try {
    now = new Date(new Date().toLocaleString("en-US", { timeZone: timezone }));
  } catch {
    /* an invalid timezone is a validation issue the app shows; the frame still draws */
  }
  const hh = String(now.getHours()).padStart(2, "0");
  const mm = String(now.getMinutes()).padStart(2, "0");
  const ss = String(now.getSeconds()).padStart(2, "0");

  // Centred on its own canvas, hero at 8 * grid — no chip, no eyebrow.
  ctx.textAlign = "center";
  ctx.textBaseline = "alphabetic";
  ctx.fillStyle = INK;
  ctx.font = "300 136px ui-rounded, ui-sans-serif, system-ui, sans-serif";
  const time = showSeconds ? `${hh}:${mm}` : `${hh}:${mm}`;
  ctx.fillText(time, PANEL_WIDTH / 2, 210);
  if (showSeconds) {
    ctx.font = "400 44px ui-rounded, ui-sans-serif, system-ui, sans-serif";
    ctx.fillStyle = DIM;
    ctx.fillText(ss, PANEL_WIDTH / 2, 262);
  }
  ctx.font = "500 26px ui-sans-serif, system-ui, sans-serif";
  ctx.fillStyle = DIM;
  ctx.fillText(
    now.toLocaleDateString("en-GB", { weekday: "long", day: "numeric", month: "long" }),
    PANEL_WIDTH / 2,
    showSeconds ? 306 : 268,
  );
}

function progressRing(
  ctx: CanvasRenderingContext2D,
  label: string,
  remaining: number,
  total: number,
) {
  const cx = PANEL_WIDTH / 2;
  const cy = 186;
  const r = 108;
  const fraction = total > 0 ? Math.max(0, Math.min(1, remaining / total)) : 0;

  ctx.lineWidth = 18;
  ctx.lineCap = "round";
  ctx.strokeStyle = "#242426";
  ctx.beginPath();
  ctx.arc(cx, cy, r, 0, Math.PI * 2);
  ctx.stroke();

  ctx.strokeStyle = EMIT;
  ctx.beginPath();
  ctx.arc(cx, cy, r, -Math.PI / 2, -Math.PI / 2 + Math.PI * 2 * fraction);
  ctx.stroke();

  const mins = Math.floor(remaining / 60);
  const secs = Math.floor(remaining % 60);
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillStyle = INK;
  ctx.font = "500 68px ui-rounded, ui-sans-serif, system-ui, sans-serif";
  ctx.fillText(`${mins}:${String(secs).padStart(2, "0")}`, cx, cy);
  ctx.fillStyle = DIM;
  ctx.font = "600 22px ui-sans-serif, system-ui, sans-serif";
  ctx.fillText(label.toUpperCase(), cx, 322);
}

function rowList(ctx: CanvasRenderingContext2D, title: string, rows: string[]) {
  chip(ctx, title);
  ctx.textAlign = "left";
  ctx.textBaseline = "middle";
  const visible = rows.length > 0 ? rows : ["--"];
  visible.slice(0, 4).forEach((row, index) => {
    const y = 118 + index * 62;
    ctx.fillStyle = index === 0 ? INK : DIM;
    ctx.font = `${index === 0 ? "500" : "400"} 27px ui-sans-serif, system-ui, sans-serif`;
    const text = row.length > 30 ? `${row.slice(0, 29)}…` : row;
    ctx.fillText(text, 44, y);
    if (index < Math.min(visible.length, 4) - 1) {
      ctx.strokeStyle = "#1C1C1E";
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(44, y + 31);
      ctx.lineTo(PANEL_WIDTH - 44, y + 31);
      ctx.stroke();
    }
  });
}

function heroCaption(ctx: CanvasRenderingContext2D, title: string, hero: string, caption: string) {
  chip(ctx, title);
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillStyle = INK;
  ctx.font = "300 124px ui-rounded, ui-sans-serif, system-ui, sans-serif";
  ctx.fillText(hero, PANEL_WIDTH / 2, 196);
  ctx.fillStyle = DIM;
  ctx.font = "500 26px ui-sans-serif, system-ui, sans-serif";
  ctx.fillText(caption, PANEL_WIDTH / 2, 288);
}

export function renderMockFrame(
  card: CardSettings,
  data: CardDataSnapshot | undefined,
  timezone: string,
  pomodoroRemaining: number | null,
): string {
  const canvas = document.createElement("canvas");
  canvas.width = PANEL_WIDTH;
  canvas.height = PANEL_HEIGHT;
  const ctx = canvas.getContext("2d");
  if (!ctx) return "";
  ctx.fillStyle = GROUND;
  ctx.fillRect(0, 0, PANEL_WIDTH, PANEL_HEIGHT);

  switch (card.kind) {
    case "clock":
      clockFace(ctx, card.show_seconds, timezone);
      break;
    case "pomodoro":
      progressRing(
        ctx,
        card.label || "Pomodoro",
        pomodoroRemaining ?? card.duration_seconds,
        card.duration_seconds,
      );
      break;
    case "weather":
      heroCaption(
        ctx,
        card.title || "Weather",
        fieldValue(data, "hero") ?? "--",
        fieldValue(data, "caption") ?? "No data yet",
      );
      break;
    case "json-feed":
      heroCaption(
        ctx,
        card.title || "JSON feed",
        fieldValue(data, "hero") ?? "--",
        fieldValue(data, "caption") ?? "No data yet",
      );
      break;
    case "calendar":
    case "rss": {
      const rows = (data?.fields ?? [])
        .filter((field) => field.key.startsWith("row"))
        .map((field) => (field.value.kind === "text" ? field.value.value : ""));
      rowList(ctx, card.title || (card.kind === "rss" ? "Headlines" : "Calendar"), rows);
      break;
    }
  }

  return canvas.toDataURL("image/png").replace(/^data:image\/png;base64,/, "");
}
