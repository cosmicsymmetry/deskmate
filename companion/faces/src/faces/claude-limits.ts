// Claude usage is another producer of an ordinary picture card. Two windows per
// page preserve the existing panel's large percentages, even when the feed grows.
import {
  ConfigurationError,
  type FaceDefinition,
  type RenderContext,
  type Settings,
  text,
  TransientError,
  truncateUtf8,
} from "../face";
import { relativeAge } from "../kit/age";
import { type FetchText, fetchText } from "../kit/http";
import { pageCount, pageForView, pageViews, recentTap } from "../kit/paging";
import { Canvas, fit, fixed, normalizeWhitespace, textWidth } from "../kit/svg";
import {
  baselineFromCenter,
  CANVAS_HEIGHT,
  CANVAS_WIDTH,
  CONTENT_WIDTH,
  GRID,
  GROUND,
  HAIRLINE,
  INK,
  INK_2,
  MARGIN,
  SIZE_BODY,
  SIZE_SUBHEAD,
  SURFACE,
  WEIGHT_SEMIBOLD,
} from "../kit/theme";

export interface UsageWindow {
  name: string;
  used_pct: number | null;
  resets_at_label: string;
  resets_in_label: string;
}

export interface ClaudeUsage {
  updated: string | null;
  plan: string;
  windows: UsageWindow[];
}

interface ClaudeLimitsState {
  url: string;
  usage: ClaudeUsage;
  page: number;
  tappedAt: string | null;
}

const PER_PAGE = 2;
// Six views fit the existing staging contract; bound cached state as well as SVG work.
const MAX_WINDOWS = 12;
const STALE_MS = 60 * 60_000;
// The *feed's* age is distinct from the server's last successful frame push.
const WARN = "#ff9f0a";

function record(value: unknown): Record<string, unknown> | undefined {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;
}

function label(value: unknown): string {
  return typeof value === "string" ? normalizeWhitespace(truncateUtf8(value, 160)) : "";
}

function usageFromDocument(document: unknown): ClaudeUsage {
  const fields = record(document);
  if (fields === undefined) {
    throw new TransientError("The Claude usage feed must contain a JSON object.");
  }
  if (!Array.isArray(fields.windows) || fields.windows.length === 0) {
    throw new TransientError("The Claude usage feed has no usage windows.");
  }
  if (fields.windows.length > MAX_WINDOWS) {
    throw new TransientError("The Claude usage feed has more than 12 windows; it cannot fit.");
  }
  const updated = label(fields.updated);
  // Require a timezone: interpreting a bare date in the server's zone hides stale data.
  const timestamp = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/i;
  return {
    updated: timestamp.test(updated) && Number.isFinite(Date.parse(updated)) ? updated : null,
    plan: label(fields.plan),
    windows: fields.windows.map((value, index) => {
      const window = record(value);
      if (window === undefined) {
        throw new TransientError("The Claude usage feed contains an invalid usage window.");
      }
      const used = window.used_pct;
      return {
        name: label(window.name) || `Window ${index + 1}`,
        used_pct:
          typeof used === "number" && Number.isFinite(used)
            ? Math.max(0, Math.min(100, used))
            : null,
        resets_at_label: label(window.resets_at_label),
        resets_in_label: label(window.resets_in_label),
      };
    }),
  };
}

export function parseClaudeUsage(body: string): ClaudeUsage {
  let document: unknown;
  try {
    document = JSON.parse(body);
  } catch {
    throw new TransientError("The Claude usage feed returned invalid JSON.");
  }
  return usageFromDocument(document);
}

function feedUrl(settings: Settings): string {
  const address = text(settings, "url");
  try {
    const url = new URL(address);
    if (
      !/^https:\/\//i.test(address) ||
      /[\s\\]/u.test(address) ||
      url.protocol !== "https:" ||
      url.username !== "" ||
      url.password !== ""
    ) {
      throw new Error("invalid usage URL");
    }
    return address;
  } catch {
    throw new ConfigurationError(
      "Set Usage feed to a valid https:// URL without a username or password.",
    );
  }
}

export async function fetchClaudeUsage(
  settings: Settings,
  get: FetchText = fetchText,
): Promise<ClaudeUsage> {
  const url = feedUrl(settings);
  try {
    return parseClaudeUsage(await get(url));
  } catch (error) {
    // kit/http owns egress refusals and HTTP/deadline/body-cap sentences. Preserve
    // them for face_status, including configuration errors for private addresses.
    if (error instanceof ConfigurationError || error instanceof TransientError) throw error;
    throw new TransientError("The Claude usage feed could not be fetched. It will be retried.");
  }
}

export function usageFreshness(usage: ClaudeUsage, now: Date): { label: string; warning: boolean } {
  if (usage.updated === null) return { label: "Update time unknown", warning: true };
  const updated = new Date(usage.updated);
  const age = now.getTime() - updated.getTime();
  if (age < -5 * 60_000) return { label: "Update time is ahead", warning: true };
  const relative = relativeAge(updated, now);
  const when = relative === "now" ? "now" : `${relative} ago`;
  return age > STALE_MS
    ? { label: `Stale · updated ${when}`, warning: true }
    : { label: `Updated ${when}`, warning: false };
}

export function renderClaudeLimits(usage: ClaudeUsage, now: Date, page = 0): string {
  const canvas = new Canvas(CANVAS_WIDTH, CANVAS_HEIGHT);
  canvas.rect(0, 0, CANVAS_WIDTH, CANVAS_HEIGHT, GROUND);
  const right = CANVAS_WIDTH - MARGIN;
  canvas.text({
    x: MARGIN,
    baseline: 6 * GRID,
    content: "Claude",
    size: SIZE_SUBHEAD,
    fill: INK,
    weight: WEIGHT_SEMIBOLD,
  });
  if (usage.plan !== "") {
    const plan = fit(usage.plan, SIZE_BODY, WEIGHT_SEMIBOLD, 20 * GRID);
    const width = textWidth(plan, SIZE_BODY, WEIGHT_SEMIBOLD) + 3 * GRID;
    canvas.roundedRect(right - width, 3 * GRID, width, 4 * GRID, GRID, SURFACE);
    canvas.text({
      x: right - width / 2,
      baseline: baselineFromCenter(5 * GRID, SIZE_BODY),
      content: plan,
      size: SIZE_BODY,
      fill: INK_2,
      weight: WEIGHT_SEMIBOLD,
      anchor: "middle",
    });
  }

  const pages = pageCount(usage.windows.length, PER_PAGE);
  const selected = Math.max(0, Math.min(pages - 1, Math.trunc(page)));
  const windows = usage.windows.slice(selected * PER_PAGE, (selected + 1) * PER_PAGE);
  windows.forEach((window, index) => {
    const top = (windows.length === 1 ? 15 : 10) * GRID + index * 15 * GRID;
    const percent = window.used_pct === null ? "—" : `${fixed(window.used_pct, 0)}%`;
    const size = 60;
    const nameWidth = CONTENT_WIDTH - textWidth(percent, size, WEIGHT_SEMIBOLD) - 2 * GRID;
    canvas.text({
      x: MARGIN,
      baseline: top + 4 * GRID,
      content: fit(window.name, SIZE_SUBHEAD, WEIGHT_SEMIBOLD, nameWidth),
      size: SIZE_SUBHEAD,
      fill: INK,
      weight: WEIGHT_SEMIBOLD,
    });
    canvas.text({
      x: right,
      baseline: top + 5 * GRID,
      content: percent,
      size,
      fill: INK,
      weight: WEIGHT_SEMIBOLD,
      anchor: "end",
    });
    const barY = top + 7 * GRID;
    const height = 1.5 * GRID;
    canvas.roundedRect(MARGIN, barY, CONTENT_WIDTH, height, height / 2, HAIRLINE);
    if (window.used_pct !== null && window.used_pct > 0) {
      // Exact length: 1% really is four pixels, not a minimum-width 14px pill.
      canvas.roundedRect(
        MARGIN,
        barY,
        (CONTENT_WIDTH * window.used_pct) / 100,
        height,
        height / 2,
        INK,
      );
    }
    const reset =
      window.resets_at_label !== ""
        ? `Resets ${window.resets_at_label}`
        : window.resets_in_label !== ""
          ? `Reset in ${window.resets_in_label} at update`
          : "Reset time unavailable";
    canvas.text({
      x: MARGIN,
      baseline: top + 12 * GRID,
      content: fit(reset, SIZE_BODY, 400, CONTENT_WIDTH),
      size: SIZE_BODY,
      fill: INK_2,
    });
  });

  const freshness = usageFreshness(usage, now);
  const counter = pages > 1 ? `${selected + 1}/${pages}` : "";
  const counterWidth = counter === "" ? 0 : textWidth(counter, SIZE_BODY, 400) + 2 * GRID;
  canvas.text({
    x: MARGIN,
    baseline: CANVAS_HEIGHT - MARGIN,
    content: fit(freshness.label, SIZE_BODY, 400, CONTENT_WIDTH - counterWidth),
    size: SIZE_BODY,
    fill: freshness.warning ? WARN : INK_2,
  });
  if (counter !== "") {
    canvas.text({
      x: right,
      baseline: CANVAS_HEIGHT - MARGIN,
      content: counter,
      size: SIZE_BODY,
      fill: INK_2,
      anchor: "end",
    });
  }
  return canvas.finish();
}

function storedState(value: unknown, url: string): ClaudeLimitsState | undefined {
  const state = record(value);
  if (state === undefined || state.url !== url) return undefined;
  try {
    const usage = usageFromDocument(state.usage);
    const pages = pageCount(usage.windows.length, PER_PAGE);
    return {
      url,
      usage,
      page:
        typeof state.page === "number" && Number.isInteger(state.page) && state.page >= 0
          ? state.page % pages
          : 0,
      tappedAt: typeof state.tappedAt === "string" ? state.tappedAt : null,
    };
  } catch {
    // Invalid cached state is disposable; fetching below yields the real status.
    return undefined;
  }
}

function advance(state: ClaudeLimitsState, taps: number, now: Date): ClaudeLimitsState {
  return {
    ...state,
    page: (state.page + taps) % pageCount(state.usage.windows.length, PER_PAGE),
    tappedAt: now.toISOString(),
  };
}

export async function renderClaudeLimitsRequest(
  settings: Settings,
  now: Date,
  context: RenderContext = {},
  get: FetchText = fetchText,
): Promise<{ svg: string; state: ClaudeLimitsState }> {
  const url = feedUrl(settings);
  const previous = storedState(context.state, url);
  if (previous !== undefined && (context.view !== undefined || context.event !== undefined)) {
    const state =
      context.view === undefined && context.event !== undefined
        ? advance(previous, context.event.taps, now)
        : previous;
    const page =
      context.view === undefined
        ? state.page
        : pageForView(context.view, pageCount(state.usage.windows.length, PER_PAGE));
    return { svg: renderClaudeLimits(state.usage, now, page), state };
  }
  const usage = await fetchClaudeUsage(settings, get);
  const keepPage = previous !== undefined && recentTap(previous.tappedAt, now);
  const state: ClaudeLimitsState = {
    url,
    usage,
    page: keepPage ? previous.page % pageCount(usage.windows.length, PER_PAGE) : 0,
    tappedAt: keepPage ? previous.tappedAt : null,
  };
  const page =
    context.view === undefined
      ? state.page
      : pageForView(context.view, pageCount(usage.windows.length, PER_PAGE));
  return { svg: renderClaudeLimits(usage, now, page), state };
}

export const claudeLimits: FaceDefinition = {
  kind: "claude-limits",
  label: "Claude limits",
  fields: [
    {
      type: "text",
      key: "url",
      label: "Usage feed",
      placeholder: "e.g. https://example.com/claude-usage.json",
    },
  ],
  refreshSeconds: 180,
  tap: "Tap the panel for more usage windows, when available.",
  views(settings, value) {
    const state = storedState(value, text(settings, "url"));
    return pageViews(pageCount(state?.usage.windows.length ?? 0, PER_PAGE));
  },
  onTap(settings, value, event, now) {
    const previous = storedState(value, text(settings, "url"));
    if (previous === undefined) return { view: "" };
    const state = advance(previous, event.taps, now);
    return {
      view: pageViews(pageCount(state.usage.windows.length, PER_PAGE))[state.page] ?? "",
      state,
    };
  },
  render: renderClaudeLimitsRequest,
};
