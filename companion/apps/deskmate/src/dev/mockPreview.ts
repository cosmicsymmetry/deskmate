/**
 * Dev-only stand-in for `render_card_preview`.
 *
 * The preview endpoint returns exact pixels from the device renderer. Nothing in
 * a browser can do that, so this draws a deliberate approximation at the true
 * 448x368 and every frame is reported with `sample: true`, which is the same flag the
 * real backend sets when it renders from placeholder data. The UI already labels that
 * state, so a mock frame can never be mistaken for a device frame.
 *
 * Layout rules mirror the device: clocks carry no title or eyebrow, pomodoros put
 * their label under the ring, pictures own their full frame, and the canvas is one
 * clean 448x368 surface with no status strip.
 */
import type { CardSettings } from "../lib/types";

export const PANEL_WIDTH = 448;
export const PANEL_HEIGHT = 368;

const INK = "#FFFFFF";
const DIM = "#8A8A8E";
const GROUND = "#000000";
const EMIT = "#FFB340";

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

  // Centered on its own canvas, with the time as the only hero.
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

function pictureFrame(ctx: CanvasRenderingContext2D, title: string) {
  ctx.fillStyle = "#16162A";
  ctx.fillRect(0, 0, PANEL_WIDTH, PANEL_HEIGHT);
  ctx.fillStyle = "#8D8DFF";
  ctx.fillRect(24, 86, 400, 74);
  ctx.fillStyle = "#5B5BC8";
  ctx.fillRect(24, 218, 288, 74);
  ctx.textAlign = "left";
  ctx.textBaseline = "middle";
  ctx.fillStyle = INK;
  ctx.font = "600 25px ui-sans-serif, system-ui, sans-serif";
  ctx.fillText(title || "Pushed picture", 38, 48);
  ctx.font = "500 34px ui-rounded, ui-sans-serif, system-ui, sans-serif";
  ctx.fillText("72%", 42, 123);
  ctx.fillText("48%", 42, 255);
}

export function renderMockFrame(
  card: CardSettings,
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
    case "picture":
      pictureFrame(ctx, card.title);
      break;
  }

  return canvas.toDataURL("image/png").replace(/^data:image\/png;base64,/, "");
}
