// The weather face: the owner approved this design on 2026-09-17. Treat its
// geometry as settled -- `test/golden/weather--*.svg` pins it byte for byte.
//
// The constraints that are load-bearing, so a change does not break them by accident:
// - Colour lives in a rounded hero module on black, NEVER full-bleed. The panel is
//   emissive; a 448x368 field of saturated colour is a desk lamp.
// - The face is coloured BY its condition, rather than one neutral template with a
//   swapped icon.
// - Glyphs are drawn, not shipped as assets: circles, lines and paths parameterized
//   by one scale, so the same code draws the hero glyph and the strip glyph.
// - The hourly forecast is a strip of six equal columns, not a chart with axes.
// - Conditions are coarser than the WMO code list on purpose.

import {
  ConfigurationError,
  type FaceDefinition,
  type RenderContext,
  type RenderResult,
  type Settings,
  text,
  TransientError,
  truncateUtf8,
} from "../face";
import { type FetchText, fetchText } from "../kit/http";
import {
  Canvas,
  fit,
  fitSize,
  fitTracked,
  fixed,
  normalizeWhitespace,
  textWidth,
} from "../kit/svg";
import {
  baselineFromCapTop,
  baselineFromCenter,
  CANVAS_HEIGHT,
  CANVAS_WIDTH,
  CAP_HEIGHT,
  CONTENT_WIDTH,
  GRID,
  GROUND,
  HAIRLINE,
  HERO_STEPS,
  INK,
  INK_3,
  MARGIN,
  RADIUS_MODULE,
  SIZE_BODY,
  SIZE_CAPTION,
  SIZE_EYEBROW,
  SIZE_SUBHEAD,
  SURFACE,
  TRACKING_EYEBROW,
  WEIGHT_SEMIBOLD,
} from "../kit/theme";

// ---------------------------------------------------------------------------
// The view model.
// ---------------------------------------------------------------------------

export type Condition =
  | "clear-day"
  | "clear-night"
  | "partly-cloudy-day"
  | "partly-cloudy-night"
  | "cloudy"
  | "fog"
  | "drizzle"
  | "rain"
  | "sleet"
  | "snow"
  | "thunderstorm";

export interface HourlyStep {
  /** The local hour, two digits: "14". */
  label: string;
  temperature: number;
  condition: Condition;
}

export interface DailyStep {
  /** Open-Meteo's local calendar date, kept intact rather than converted by the host. */
  date: string;
  /** A fixed English weekday derived from `date`, independent of the host's locale. */
  label: string;
  high: number;
  low: number;
  condition: Condition;
  /** The same words the current conditions use, so a day can fill the hero. */
  summary: string;
}

export interface WeatherFace {
  /** The geocoder's own display name, so the panel says what the forecast is for. */
  place: string;
  temperature: number;
  summary: string;
  condition: Condition;
  high: number;
  low: number;
  hourly: HourlyStep[];
  daily: DailyStep[];
}

export interface WeatherState {
  view: "now" | "days";
  tappedAt: string | null;
  /**
   * The forecast the last fetch returned, so a tap can redraw without going back
   * to the network.
   *
   * Both views are the same reading said about a different moment, so a tap that
   * only switches between them has no reason to fetch. Measured at about 1 KB of
   * JSON against the 16 KB the server allows per source.
   */
  forecast?: WeatherFace;
  /** When `forecast` was fetched. Absent or stale means fetch again. */
  fetchedAt?: string;
}

/**
 * How long a cached forecast may answer a tap.
 *
 * Shorter than the card's own refresh, so a tap never shows a reading the
 * schedule would already have replaced.
 */
const FORECAST_CACHE_MS = 10 * 60 * 1_000;

function cachedForecast(state: WeatherState, now: Date): WeatherFace | undefined {
  if (state.forecast === undefined || state.fetchedAt === undefined) {
    return undefined;
  }
  const at = Date.parse(state.fetchedAt);
  if (!Number.isFinite(at) || now.getTime() - at > FORECAST_CACHE_MS) {
    return undefined;
  }
  return state.forecast;
}

/**
 * A tap redraws from the forecast already in hand when there is one, so the
 * answer costs a draw instead of a draw plus an Open-Meteo round trip.
 */
export async function renderWeatherRequest(
  settings: Settings,
  now: Date,
  context: RenderContext = {},
  get: FetchText = fetchText,
): Promise<RenderResult> {
  const previous = weatherState(context.state);
  // Being told a view is a tap, and a tap must not visit Open-Meteo: both views
  // say the same reading about a different moment, so the forecast already in
  // hand answers it. The event is honoured beside the view because the server
  // and this package deploy independently.
  const answeringATap = context.view !== undefined || context.event !== undefined;
  const cached = answeringATap ? cachedForecast(previous, now) : undefined;
  const face = cached ?? (await fetchWeather(settings, get, context.timezone));
  const fetchedAt = cached === undefined ? now.toISOString() : previous.fetchedAt;
  const result = renderWeatherResult(face, now, context);
  const drawn = typeof result === "string" ? { svg: result } : result;
  return {
    svg: drawn.svg,
    state: { ...(drawn.state as WeatherState), forecast: face, fetchedAt },
  };
}

interface Palette {
  ground: string;
  accent: string;
  muted: string;
}

const PALETTES: Record<Condition, Palette> = {
  "clear-day": { ground: "#0e3a5c", accent: "#ffc24d", muted: "#a9c6dc" },
  "clear-night": { ground: "#0c1330", accent: "#dce3f5", muted: "#9aa4c4" },
  "partly-cloudy-day": { ground: "#123249", accent: "#ffc24d", muted: "#a9c0d0" },
  "partly-cloudy-night": { ground: "#10182f", accent: "#dce3f5", muted: "#9aa4c4" },
  cloudy: { ground: "#24282f", accent: "#c6cbd6", muted: "#9ba1ac" },
  fog: { ground: "#22272b", accent: "#aeb6bd", muted: "#97a0a7" },
  drizzle: { ground: "#13293d", accent: "#8fc4ea", muted: "#9db6c9" },
  rain: { ground: "#102a3d", accent: "#6fb3e0", muted: "#9ab3c6" },
  sleet: { ground: "#17293a", accent: "#9fc7e6", muted: "#a2b4c2" },
  snow: { ground: "#1a2833", accent: "#e8f2ff", muted: "#a6b5c1" },
  thunderstorm: { ground: "#1a1526", accent: "#ffd34d", muted: "#a79fb8" },
};

export function conditionFromWmo(code: number, isDay: boolean): Condition {
  switch (code) {
    case 0:
    case 1:
      return isDay ? "clear-day" : "clear-night";
    case 2:
      return isDay ? "partly-cloudy-day" : "partly-cloudy-night";
    case 45:
    case 48:
      return "fog";
    case 51:
    case 53:
    case 55:
      return "drizzle";
    case 56:
    case 57:
    case 66:
    case 67:
      return "sleet";
    case 61:
    case 63:
    case 65:
    case 80:
    case 81:
    case 82:
      return "rain";
    case 71:
    case 73:
    case 75:
    case 77:
    case 85:
    case 86:
      return "snow";
    case 95:
    case 96:
    case 99:
      return "thunderstorm";
    default:
      return "cloudy";
  }
}

function summaryFromWmo(code: number): string {
  if (code === 0) return "Clear";
  if (code === 1 || code === 2) return "Partly cloudy";
  if (code === 3) return "Overcast";
  if (code === 45 || code === 48) return "Fog";
  if ([51, 53, 55, 56, 57].includes(code)) return "Drizzle";
  if ([61, 63, 65, 66, 67, 80, 81, 82].includes(code)) return "Rain";
  if ([71, 73, 75, 77, 85, 86].includes(code)) return "Snow";
  if ([95, 96, 99].includes(code)) return "Thunderstorm";
  return "Unknown";
}

// ---------------------------------------------------------------------------
// Data: Open-Meteo, no key. One geocoding request, one forecast request.
// ---------------------------------------------------------------------------

const GEOCODING_ENDPOINT = "https://geocoding-api.open-meteo.com/v1/search";
const FORECAST_ENDPOINT = "https://api.open-meteo.com/v1/forecast";
const MAX_TEMPERATURE = 200;
export const STRIP_COLUMNS = 6;

type Json = Record<string, unknown>;

function isObject(value: unknown): value is Json {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function finite(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function temperature(value: unknown): number | undefined {
  const degrees = finite(value);
  return degrees !== undefined && Math.abs(degrees) <= MAX_TEMPERATURE ? degrees : undefined;
}

/** Whole degrees, halves away from zero in both directions: -18.5 is -19, not -18. */
function wholeDegrees(degrees: number): number {
  const tenths = Math.sign(degrees) * Math.round(Math.abs(degrees) * 10);
  // `|| 0` folds negative zero: -0.4 degrees is "0", never "-0".
  return Math.trunc((tenths + Math.sign(tenths) * 5) / 10) || 0;
}

function isDay(value: unknown, missing: boolean): boolean {
  if (value === undefined) {
    return missing;
  }
  return value === true || value === 1;
}

function withQuery(endpoint: string, pairs: [string, string][]): string {
  const url = new URL(endpoint);
  for (const [key, value] of pairs) {
    url.searchParams.append(key, value);
  }
  return url.toString();
}

function parseJson(body: string, what: string): Json {
  try {
    const document: unknown = JSON.parse(body);
    if (isObject(document)) {
      return document;
    }
  } catch {
    // Fall through to the shared refusal.
  }
  throw new TransientError(`the ${what} response is not JSON`);
}

export function parseForecast(body: string, place: string): WeatherFace {
  const document = parseJson(body, "forecast");
  const current = document.current;
  if (!isObject(current)) {
    throw new TransientError("the forecast has no current conditions");
  }
  const now = temperature(current.temperature_2m);
  const code = finite(current.weather_code);
  if (now === undefined || code === undefined) {
    throw new TransientError("the forecast's current conditions are incomplete");
  }

  const daily = isObject(document.daily) ? document.daily : {};
  const max = temperature(
    Array.isArray(daily.temperature_2m_max) ? daily.temperature_2m_max[0] : undefined,
  );
  const min = temperature(
    Array.isArray(daily.temperature_2m_min) ? daily.temperature_2m_min[0] : undefined,
  );
  const high = max !== undefined && min !== undefined ? Math.max(max, min) : now;
  const low = max !== undefined && min !== undefined ? Math.min(max, min) : now;

  return {
    place: truncateUtf8(place, 64),
    temperature: wholeDegrees(now),
    summary: summaryFromWmo(code),
    condition: conditionFromWmo(code, isDay(current.is_day, false)),
    high: wholeDegrees(high),
    low: wholeDegrees(low),
    hourly: hourlySteps(document, typeof current.time === "string" ? current.time : ""),
    daily: dailySteps(document),
  };
}

/**
 * The next hours, starting at the current one. The requested timezone makes every
 * timestamp local to the place, so the hour is read straight off the string
 * ("2026-09-12T14:00") and never converted.
 */
function hourlySteps(document: Json, currentTime: string): HourlyStep[] {
  const hourly = document.hourly;
  if (!isObject(hourly)) {
    return [];
  }
  const { time, temperature_2m: temperatures, weather_code: codes, is_day: days } = hourly;
  if (!Array.isArray(time) || !Array.isArray(temperatures) || !Array.isArray(codes)) {
    return [];
  }
  const thisHour = currentTime.slice(0, 13);
  if (thisHour.length !== 13) {
    return [];
  }
  const start = time.findIndex((stamp) => typeof stamp === "string" && stamp.startsWith(thisHour));
  if (start < 0) {
    return [];
  }

  const steps: HourlyStep[] = [];
  for (let index = start; index < time.length && steps.length < STRIP_COLUMNS; index += 1) {
    const stamp: unknown = time[index];
    const degrees = temperature(temperatures[index]);
    const code = finite(codes[index]);
    const hour = typeof stamp === "string" ? Number(stamp.slice(11, 13)) : Number.NaN;
    if (degrees === undefined || code === undefined || !(hour >= 0 && hour < 24)) {
      continue;
    }
    steps.push({
      label: String(hour).padStart(2, "0"),
      temperature: wholeDegrees(degrees),
      condition: conditionFromWmo(code, isDay(Array.isArray(days) ? days[index] : undefined, true)),
    });
  }
  return steps;
}

const WEEKDAYS = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"] as const;

/** Open-Meteo returns local dates with no offset; read the calendar, not the host clock. */
function weekdayOf(date: string): string | undefined {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(date);
  if (match === null) {
    return undefined;
  }
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  const instant = new Date(Date.UTC(year, month - 1, day));
  if (
    instant.getUTCFullYear() !== year ||
    instant.getUTCMonth() !== month - 1 ||
    instant.getUTCDate() !== day
  ) {
    return undefined;
  }
  return WEEKDAYS[instant.getUTCDay()];
}

function dailySteps(document: Json): DailyStep[] {
  const daily = document.daily;
  if (!isObject(daily)) {
    return [];
  }
  const {
    time,
    weather_code: codes,
    temperature_2m_max: maximums,
    temperature_2m_min: minimums,
  } = daily;
  if (
    !Array.isArray(time) ||
    !Array.isArray(codes) ||
    !Array.isArray(maximums) ||
    !Array.isArray(minimums)
  ) {
    return [];
  }

  const steps: DailyStep[] = [];
  for (let index = 0; index < time.length && steps.length < 6; index += 1) {
    const date = time[index];
    const label = typeof date === "string" ? weekdayOf(date) : undefined;
    const code = finite(codes[index]);
    const maximum = temperature(maximums[index]);
    const minimum = temperature(minimums[index]);
    if (
      typeof date !== "string" ||
      label === undefined ||
      code === undefined ||
      maximum === undefined ||
      minimum === undefined
    ) {
      continue;
    }
    steps.push({
      date,
      label,
      high: wholeDegrees(Math.max(maximum, minimum)),
      low: wholeDegrees(Math.min(maximum, minimum)),
      condition: conditionFromWmo(code, true),
      summary: summaryFromWmo(code),
    });
  }
  return steps;
}

export async function fetchWeather(
  settings: Settings,
  get: FetchText,
  timezone?: string,
): Promise<WeatherFace> {
  const location = text(settings, "location");
  if (location === "") {
    throw new ConfigurationError("the location is empty");
  }
  const geocoding = parseJson(
    await get(
      withQuery(GEOCODING_ENDPOINT, [
        ["name", location],
        ["count", "1"],
        ["language", "en"],
        ["format", "json"],
      ]),
    ),
    "geocoding",
  );
  const place = Array.isArray(geocoding.results) ? geocoding.results[0] : undefined;
  if (!isObject(place)) {
    throw new ConfigurationError(`no place called "${location}" was found`);
  }
  const latitude = finite(place.latitude);
  const longitude = finite(place.longitude);
  if (
    latitude === undefined ||
    longitude === undefined ||
    Math.abs(latitude) > 90 ||
    Math.abs(longitude) > 180
  ) {
    throw new TransientError("the geocoder returned invalid coordinates");
  }
  const name = typeof place.name === "string" ? place.name : location;
  const display = typeof place.country === "string" ? `${name}, ${place.country}` : name;

  const query: [string, string][] = [
    ["latitude", String(latitude)],
    ["longitude", String(longitude)],
    ["current", "temperature_2m,weather_code,is_day"],
    ["daily", "weather_code,temperature_2m_max,temperature_2m_min"],
    ["hourly", "temperature_2m,weather_code,is_day"],
    ["forecast_days", "6"],
    ["timezone", timezone || "auto"],
  ];
  if (text(settings, "units") === "imperial") {
    query.push(["temperature_unit", "fahrenheit"]);
  }
  return parseForecast(await get(withQuery(FORECAST_ENDPOINT, query)), display);
}

// ---------------------------------------------------------------------------
// Drawing.
// ---------------------------------------------------------------------------

const HERO_TOP = MARGIN;
const HERO_HEIGHT = 200;
const HERO_BOTTOM = HERO_TOP + HERO_HEIGHT;
const STRIP_TOP = HERO_BOTTOM + 2 * GRID;
const STRIP_BOTTOM = CANVAS_HEIGHT - MARGIN;
const STRIP_HEIGHT = STRIP_BOTTOM - STRIP_TOP;
const HERO_GLYPH_SCALE = 52;
const STRIP_GLYPH_SCALE = 15;

const f2 = (value: number): string => fixed(value, 2);

export function renderWeather(face: WeatherFace): string {
  const canvas = new Canvas(CANVAS_WIDTH, CANVAS_HEIGHT);
  canvas.rect(0, 0, CANVAS_WIDTH, CANVAS_HEIGHT, GROUND);
  drawHero(canvas, nowContent(face), PALETTES[face.condition]);
  drawStrip(canvas, face);
  return canvas.finish();
}

const DAYS_GLYPH_SCALE = 18;
/** Tomorrow takes the hero, so the strip carries the four days after it. */
const DAYS_IN_STRIP = 4;
/** Between a day's high and the low that trails it. */
const READING_GAP = 7;

/**
 * The second view: tomorrow in the hero the current conditions usually hold, and
 * the days after it in the strip the hours usually hold.
 *
 * It is the same two modules in the same places, because it is the same face
 * saying the same kind of thing about a later moment. An earlier draft spent the
 * hero on the words "Coming days" beside an icon, which told the owner what they
 * were looking at instead of telling them the weather -- a label above a heading,
 * which `DESIGN.md` rules out.
 */
export function renderWeatherDays(face: WeatherFace): string {
  const canvas = new Canvas(CANVAS_WIDTH, CANVAS_HEIGHT);
  canvas.rect(0, 0, CANVAS_WIDTH, CANVAS_HEIGHT, GROUND);

  // `daily[0]` is today, which the resting view already covers.
  const lead = face.daily[1] ?? face.daily[0];
  if (lead === undefined) {
    drawHero(canvas, nowContent(face), PALETTES[face.condition]);
    canvas.roundedRect(MARGIN, STRIP_TOP, CONTENT_WIDTH, STRIP_HEIGHT, RADIUS_MODULE, SURFACE);
    canvas.text({
      x: CANVAS_WIDTH / 2,
      baseline: baselineFromCenter(STRIP_TOP + STRIP_HEIGHT / 2, SIZE_CAPTION),
      content: "Daily forecast unavailable",
      size: SIZE_CAPTION,
      fill: INK_3,
      anchor: "middle",
    });
    return canvas.finish();
  }

  const eyebrow = face.daily[1] === undefined ? lead.label : "Tomorrow";
  drawHero(canvas, dayContent(lead, eyebrow), PALETTES[lead.condition]);

  canvas.roundedRect(MARGIN, STRIP_TOP, CONTENT_WIDTH, STRIP_HEIGHT, RADIUS_MODULE, SURFACE);
  const rest = face.daily.slice(face.daily[1] === undefined ? 1 : 2, DAYS_IN_STRIP + 2);
  if (rest.length === 0) {
    canvas.text({
      x: CANVAS_WIDTH / 2,
      baseline: baselineFromCenter(STRIP_TOP + STRIP_HEIGHT / 2, SIZE_CAPTION),
      content: "No further days forecast",
      size: SIZE_CAPTION,
      fill: INK_3,
      anchor: "middle",
    });
    return canvas.finish();
  }

  const columnWidth = CONTENT_WIDTH / rest.length;
  for (let index = 1; index < rest.length; index += 1) {
    canvas.rect(
      MARGIN + index * columnWidth,
      STRIP_TOP + 2 * GRID,
      1,
      STRIP_HEIGHT - 4 * GRID,
      HAIRLINE,
    );
  }
  rest.forEach((step, index) => {
    const centerX = MARGIN + (index + 0.5) * columnWidth;
    canvas.text({
      x: centerX,
      baseline: baselineFromCapTop(STRIP_TOP + 2 * GRID, SIZE_EYEBROW),
      content: fitTracked(
        step.label,
        SIZE_EYEBROW,
        WEIGHT_SEMIBOLD,
        TRACKING_EYEBROW,
        columnWidth - 2 * GRID,
      ),
      size: SIZE_EYEBROW,
      fill: INK_3,
      weight: WEIGHT_SEMIBOLD,
      anchor: "middle",
      tracking: TRACKING_EYEBROW,
    });
    drawGlyph(
      canvas,
      centerX,
      STRIP_TOP + STRIP_HEIGHT * 0.5,
      DAYS_GLYPH_SCALE,
      step.condition,
      PALETTES[step.condition],
    );
    // The strip is the hourly strip's height, which holds three rows, so the two
    // readings share the last one: the high in full ink, the low behind it.
    const high = `${step.high}°`;
    const low = `${step.low}°`;
    const highWidth = textWidth(high, SIZE_BODY, WEIGHT_SEMIBOLD);
    const lowWidth = textWidth(low, SIZE_CAPTION, WEIGHT_SEMIBOLD);
    const readingsLeft = centerX - (highWidth + READING_GAP + lowWidth) / 2;
    const readingsBaseline = baselineFromCapTop(
      STRIP_BOTTOM - 2 * GRID - SIZE_BODY * CAP_HEIGHT,
      SIZE_BODY,
    );
    canvas.text({
      x: readingsLeft,
      baseline: readingsBaseline,
      content: high,
      size: SIZE_BODY,
      fill: INK,
      weight: WEIGHT_SEMIBOLD,
    });
    canvas.text({
      x: readingsLeft + highWidth + READING_GAP,
      baseline: readingsBaseline,
      content: low,
      size: SIZE_CAPTION,
      fill: INK_3,
      weight: WEIGHT_SEMIBOLD,
    });
  });
  return canvas.finish();
}

const TEMPORARY_VIEW_MS = 10 * 60 * 1_000;

function weatherState(value: unknown): WeatherState {
  if (typeof value !== "object" || value === null) {
    return { view: "now", tappedAt: null };
  }
  const candidate = value as {
    view?: unknown;
    tappedAt?: unknown;
    forecast?: unknown;
    fetchedAt?: unknown;
  };
  // The cached forecast is only carried forward when it still looks like one.
  // State is read back from a file the server wrote, so a shape that drifted
  // between deploys must fall back to fetching rather than draw from rubble.
  const forecast = candidate.forecast as WeatherFace | undefined;
  const usable =
    typeof forecast === "object" &&
    forecast !== null &&
    Array.isArray(forecast.hourly) &&
    Array.isArray(forecast.daily);
  return {
    view: candidate.view === "days" ? "days" : "now",
    tappedAt: typeof candidate.tappedAt === "string" ? candidate.tappedAt : null,
    ...(usable ? { forecast } : {}),
    ...(typeof candidate.fetchedAt === "string" ? { fetchedAt: candidate.fetchedAt } : {}),
  };
}

/** Chooses a view and returns the state the next scheduled refresh or tap will receive. */
export function renderWeatherResult(
  face: WeatherFace,
  now: Date,
  context?: RenderContext,
): RenderResult {
  let state = weatherState(context?.state);
  if (context?.event === undefined && state.view === "days") {
    const tappedAt = state.tappedAt === null ? Number.NaN : Date.parse(state.tappedAt);
    if (!Number.isFinite(tappedAt) || now.getTime() - tappedAt > TEMPORARY_VIEW_MS) {
      state = { view: "now", tappedAt: null };
    }
  }
  return {
    svg: context?.view === "days" ? renderWeatherDays(face) : renderWeather(face),
    state,
  };
}

/**
 * What the hero module says, whichever view is drawing it.
 *
 * Both views use this one component deliberately: the current conditions and
 * tomorrow are the same thing said about a different moment, and giving the second
 * view its own layout would mean two designs to keep in step. The eyebrow is the
 * only part that differs -- the place when the reading is now, the day when it is
 * not.
 */
interface HeroContent {
  eyebrow: string;
  reading: string;
  summary: string;
  condition: Condition;
  high: number;
  low: number;
}

function drawHero(canvas: Canvas, content: HeroContent, palette: Palette): void {
  canvas.roundedRect(MARGIN, HERO_TOP, CONTENT_WIDTH, HERO_HEIGHT, RADIUS_MODULE, palette.ground);

  const inset = MARGIN + 2.5 * GRID;
  const glyphCenterX = MARGIN + CONTENT_WIDTH - HERO_GLYPH_SCALE - 2.5 * GRID;
  const typeRoom = glyphCenterX - HERO_GLYPH_SCALE - inset - GRID;

  const range = `H ${content.high}°   L ${content.low}°`;
  const rangeWidth = textWidth(range, SIZE_CAPTION, WEIGHT_SEMIBOLD);
  const eyebrowBaseline = baselineFromCapTop(HERO_TOP + 2.5 * GRID, SIZE_EYEBROW);
  canvas.text({
    x: MARGIN + CONTENT_WIDTH - 2.5 * GRID,
    baseline: eyebrowBaseline,
    content: range,
    size: SIZE_CAPTION,
    fill: INK,
    weight: WEIGHT_SEMIBOLD,
    anchor: "end",
    opacity: 0.82,
  });

  // The place is tracked, so it is fitted with tracking: measured without it, a
  // long "city, country" drew straight through the range beside it.
  const placeRoom = CONTENT_WIDTH - 5 * GRID - rangeWidth - 2 * GRID;
  const place = fitTracked(
    normalizeWhitespace(content.eyebrow).toUpperCase(),
    SIZE_EYEBROW,
    WEIGHT_SEMIBOLD,
    TRACKING_EYEBROW,
    placeRoom,
  );
  if (place !== "") {
    canvas.text({
      x: inset,
      baseline: eyebrowBaseline,
      content: place,
      size: SIZE_EYEBROW,
      fill: palette.muted,
      weight: WEIGHT_SEMIBOLD,
      tracking: TRACKING_EYEBROW,
    });
  }

  const reading = content.reading;
  const size = fitSize(reading, WEIGHT_SEMIBOLD, typeRoom, HERO_STEPS);
  const readingCapTop = HERO_TOP + 6.5 * GRID;
  canvas.text({
    x: inset,
    baseline: baselineFromCapTop(readingCapTop, size),
    content: reading,
    size,
    fill: INK,
    weight: WEIGHT_SEMIBOLD,
  });

  const summaryTop = readingCapTop + size * CAP_HEIGHT + 1.5 * GRID;
  const summary = fit(
    normalizeWhitespace(content.summary),
    SIZE_SUBHEAD,
    WEIGHT_SEMIBOLD,
    typeRoom,
  );
  if (summary !== "") {
    canvas.text({
      x: inset,
      baseline: baselineFromCapTop(summaryTop, SIZE_SUBHEAD),
      content: summary,
      size: SIZE_SUBHEAD,
      fill: INK,
      weight: WEIGHT_SEMIBOLD,
      opacity: 0.9,
    });
  }

  drawGlyph(
    canvas,
    glyphCenterX,
    readingCapTop + (size * CAP_HEIGHT) / 2,
    HERO_GLYPH_SCALE,
    content.condition,
    palette,
  );
}

/** The current conditions as hero content. */
function nowContent(face: WeatherFace): HeroContent {
  return {
    eyebrow: face.place,
    reading: `${face.temperature}°`,
    summary: face.summary,
    condition: face.condition,
    high: face.high,
    low: face.low,
  };
}

/** A forecast day as hero content, read as that day's high. */
function dayContent(step: DailyStep, eyebrow: string): HeroContent {
  return {
    eyebrow,
    reading: `${step.high}°`,
    summary: step.summary,
    condition: step.condition,
    high: step.high,
    low: step.low,
  };
}

function drawStrip(canvas: Canvas, face: WeatherFace): void {
  canvas.roundedRect(MARGIN, STRIP_TOP, CONTENT_WIDTH, STRIP_HEIGHT, RADIUS_MODULE, SURFACE);

  if (face.hourly.length === 0) {
    canvas.text({
      x: CANVAS_WIDTH / 2,
      baseline: baselineFromCenter(STRIP_TOP + STRIP_HEIGHT / 2, SIZE_CAPTION),
      content: "Hourly forecast unavailable",
      size: SIZE_CAPTION,
      fill: INK_3,
      anchor: "middle",
    });
    return;
  }

  const columns = Math.min(face.hourly.length, STRIP_COLUMNS);
  const columnWidth = CONTENT_WIDTH / columns;

  face.hourly.slice(0, columns).forEach((step, index) => {
    const centerX = MARGIN + (index + 0.5) * columnWidth;
    canvas.text({
      x: centerX,
      baseline: baselineFromCapTop(STRIP_TOP + 2 * GRID, SIZE_EYEBROW),
      content: step.label,
      size: SIZE_EYEBROW,
      fill: INK_3,
      weight: WEIGHT_SEMIBOLD,
      anchor: "middle",
    });
    drawGlyph(
      canvas,
      centerX,
      STRIP_TOP + STRIP_HEIGHT * 0.5,
      STRIP_GLYPH_SCALE,
      step.condition,
      PALETTES[step.condition],
    );
    canvas.text({
      x: centerX,
      baseline: baselineFromCapTop(STRIP_BOTTOM - 2 * GRID - SIZE_BODY * CAP_HEIGHT, SIZE_BODY),
      content: `${step.temperature}°`,
      size: SIZE_BODY,
      fill: INK,
      weight: WEIGHT_SEMIBOLD,
      anchor: "middle",
    });
  });
}

function drawGlyph(
  canvas: Canvas,
  cx: number,
  cy: number,
  scale: number,
  condition: Condition,
  palette: Palette,
): void {
  const { accent, ground } = palette;
  switch (condition) {
    case "clear-day":
      drawSun(canvas, cx, cy, scale, accent);
      break;
    case "clear-night":
      drawMoon(canvas, cx, cy, scale, accent, ground);
      break;
    case "partly-cloudy-day":
      drawSun(canvas, cx + scale * 0.34, cy - scale * 0.42, scale * 0.62, accent);
      drawCloud(canvas, cx - scale * 0.1, cy + scale * 0.22, scale * 0.92, accent);
      break;
    case "partly-cloudy-night":
      drawMoon(canvas, cx + scale * 0.36, cy - scale * 0.44, scale * 0.56, accent, ground);
      drawCloud(canvas, cx - scale * 0.1, cy + scale * 0.22, scale * 0.92, accent);
      break;
    case "cloudy":
      drawCloud(canvas, cx, cy, scale, accent);
      break;
    case "fog":
      drawFog(canvas, cx, cy, scale, accent);
      break;
    case "drizzle":
      drawCloud(canvas, cx, cy - scale * 0.26, scale * 0.9, accent);
      drawPrecipitation(canvas, cx, cy + scale * 0.62, scale, accent, 0.36);
      break;
    case "rain":
      drawCloud(canvas, cx, cy - scale * 0.26, scale * 0.9, accent);
      drawPrecipitation(canvas, cx, cy + scale * 0.62, scale, accent, 0.58);
      break;
    case "sleet":
      drawCloud(canvas, cx, cy - scale * 0.26, scale * 0.9, accent);
      drawPrecipitation(canvas, cx, cy + scale * 0.6, scale, accent, 0.42);
      drawFlakes(canvas, cx, cy + scale * 0.66, scale, accent, [0.44]);
      break;
    case "snow":
      drawCloud(canvas, cx, cy - scale * 0.26, scale * 0.9, accent);
      drawFlakes(canvas, cx, cy + scale * 0.64, scale, accent, [-0.46, 0, 0.46]);
      break;
    case "thunderstorm":
      drawCloud(canvas, cx, cy - scale * 0.28, scale * 0.9, accent);
      drawBolt(canvas, cx, cy + scale * 0.62, scale, accent);
      break;
  }
}

function drawSun(canvas: Canvas, cx: number, cy: number, scale: number, color: string): void {
  const core = scale * 0.46;
  canvas.circle(cx, cy, core, color);
  const inner = core + scale * 0.2;
  const outer = scale * 0.98;
  const stroke = Math.max(scale * 0.12, 1.5);
  for (let index = 0; index < 8; index += 1) {
    const angle = index * (Math.PI / 4);
    const sin = Math.sin(angle);
    const cos = Math.cos(angle);
    canvas.line(
      cx + cos * inner,
      cy + sin * inner,
      cx + cos * outer,
      cy + sin * outer,
      color,
      stroke,
    );
  }
}

/** A crescent: a disc with a ground-coloured disc bitten out of it. */
function drawMoon(
  canvas: Canvas,
  cx: number,
  cy: number,
  scale: number,
  color: string,
  ground: string,
): void {
  canvas.circle(cx, cy, scale * 0.78, color);
  canvas.circle(cx + scale * 0.36, cy - scale * 0.26, scale * 0.7, ground);
}

function drawCloud(canvas: Canvas, cx: number, cy: number, scale: number, color: string): void {
  canvas.circle(cx - scale * 0.42, cy + scale * 0.04, scale * 0.36, color);
  canvas.circle(cx - scale * 0.04, cy - scale * 0.24, scale * 0.46, color);
  canvas.circle(cx + scale * 0.44, cy + scale * 0.02, scale * 0.34, color);
  canvas.roundedRect(
    cx - scale * 0.78,
    cy + scale * 0.02,
    scale * 1.56,
    scale * 0.4,
    scale * 0.2,
    color,
  );
}

function drawPrecipitation(
  canvas: Canvas,
  cx: number,
  cy: number,
  scale: number,
  color: string,
  length: number,
): void {
  const stroke = Math.max(scale * 0.13, 1.5);
  for (const offset of [-0.44, 0, 0.44]) {
    const x = cx + scale * offset;
    canvas.line(
      x + scale * 0.06,
      cy - (scale * length) / 2,
      x - scale * 0.06,
      cy + (scale * length) / 2,
      color,
      stroke,
    );
  }
}

function drawFlakes(
  canvas: Canvas,
  cx: number,
  cy: number,
  scale: number,
  color: string,
  offsets: number[],
): void {
  const radius = Math.max(scale * 0.11, 1.2);
  for (const offset of offsets) {
    canvas.circle(cx + scale * offset, cy, radius, color);
  }
}

function drawBolt(canvas: Canvas, cx: number, cy: number, scale: number, color: string): void {
  const width = scale * 0.36;
  const height = scale * 0.62;
  const points: [number, number][] = [
    [cx + width * 0.5, cy - height / 2],
    [cx - width * 0.55, cy + height * 0.1],
    [cx - width * 0.05, cy + height * 0.1],
    [cx - width * 0.45, cy + height / 2],
    [cx + width * 0.62, cy - height * 0.08],
    [cx + width * 0.05, cy - height * 0.08],
  ];
  const definition = points
    .map(([x, y], index) => `${index === 0 ? "M" : "L"}${f2(x)} ${f2(y)}`)
    .join("");
  canvas.path(`${definition}Z`, color);
}

function drawFog(canvas: Canvas, cx: number, cy: number, scale: number, color: string): void {
  const barHeight = Math.max(scale * 0.16, 2);
  const gap = scale * 0.34;
  [1.5, 1.2, 1.6, 1.1].forEach((width, index) => {
    const y = cy - gap * 1.5 + gap * index;
    const barWidth = scale * width;
    canvas.roundedRect(
      cx - barWidth / 2,
      y - barHeight / 2,
      barWidth,
      barHeight,
      barHeight / 2,
      color,
    );
  });
}

export const weather: FaceDefinition = {
  kind: "weather",
  label: "Weather",
  tap: "Tap the panel for the coming days.",
  fields: [
    { type: "text", key: "location", label: "Location", placeholder: "Dubai" },
    {
      type: "enum",
      key: "units",
      label: "Units",
      default: "metric",
      options: [
        { value: "metric", label: "Metric" },
        { value: "imperial", label: "Imperial" },
      ],
    },
  ],
  views() {
    return ["", "days"];
  },
  onTap(_settings, value, event, now) {
    let state = weatherState(value);
    if (event.taps % 2 !== 0) {
      state = { ...state, view: state.view === "now" ? "days" : "now" };
    }
    state = { ...state, tappedAt: now.toISOString() };
    return { view: state.view === "days" ? "days" : "", state };
  },
  async render(settings, now, context) {
    return renderWeatherRequest(settings, now, context);
  },
};
