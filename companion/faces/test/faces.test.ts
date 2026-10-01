import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import {
  ConfigurationError,
  type FaceDefinition,
  type RenderResult,
  type Settings,
  TransientError,
} from "../src/face";
import {
  fetchHackerNews,
  hackernews,
  renderHackerNews,
  renderHackerNewsRequest,
  type Story,
  storyFromItem,
} from "../src/faces/hackernews";
import { fetchRss, parseFeed, renderRss, renderRssRequest, rss } from "../src/faces/rss";
import {
  fetchToken,
  parseCandles,
  parseMarkets,
  renderToken,
  renderTokenRequest,
  splitPrice,
  token,
} from "../src/faces/token";
import {
  conditionFromWmo,
  fetchWeather,
  parseForecast,
  renderWeather,
  renderWeatherRequest,
  renderWeatherResult,
  weather,
} from "../src/faces/weather";
import type { FetchText } from "../src/kit/http";
import { pngFromSvg, textInk } from "../src/kit/raster";
import { BAD, GOOD } from "../src/kit/theme";

const NOW = new Date("2026-09-12T14:00:00Z");

interface CapturedHackerNewsItem {
  id: number;
  title: string;
  score: number;
  descendants: number;
  url: string;
  time: number;
  type: string;
}

const CAPTURED_HACKER_NEWS = (await Bun.file(
  new URL("./hn-front-page.captured.json", import.meta.url),
).json()) as CapturedHackerNewsItem[];

/** A fetcher that answers from a table and records what it was asked for. */
function fake(routes: Record<string, string | Error>): FetchText & { asked: string[] } {
  const asked: string[] = [];
  const get = async (url: string): Promise<string> => {
    asked.push(url);
    const key = Object.keys(routes).find((fragment) => url.includes(fragment));
    const answer = key === undefined ? new TransientError(`no route for ${url}`) : routes[key];
    if (answer instanceof Error || answer === undefined) {
      throw answer ?? new TransientError("unrouted");
    }
    return answer;
  };
  return Object.assign(get, { asked });
}

const rendersToAFrame = (svg: string): boolean => pngFromSvg(svg).length > 1_000;

const visibleSvgText = (svg: string): string =>
  [...svg.matchAll(/<text\b[^>]*>([^<]*)<\/text>/g)]
    .map((match) => match[1] ?? "")
    .join(" ")
    .replaceAll("&apos;", "'")
    .replaceAll("&quot;", '"')
    .replaceAll("&gt;", ">")
    .replaceAll("&lt;", "<")
    .replaceAll("&amp;", "&");

/**
 * A tap as the server performs it: `onTap` alone, with no fetch and no draw.
 *
 * `onTap` is optional on the interface because a face may ignore taps, but a
 * face under test here declares one -- so the assertion is part of the helper
 * rather than a `!` at every call site.
 */
function tapOn(
  face: FaceDefinition,
  settings: Settings,
  state: unknown,
  taps: number,
  now: Date,
): { view: string; state?: unknown } {
  if (face.onTap === undefined) {
    throw new Error(`${face.kind} declares no onTap`);
  }
  return face.onTap(settings, state, { taps, point: null }, now);
}

/** The page a paging face's stored state is on. */
function pageOf(state: unknown): number {
  return (state as { page: number }).page;
}

describe("token", () => {
  const MARKETS = JSON.stringify([
    {
      id: "solana",
      symbol: "sol",
      name: "Solana",
      current_price: 142.37,
      high_24h: 144.12,
      low_24h: 136.9,
      price_change_percentage_24h: 2.41,
    },
  ]);
  const CHART = JSON.stringify({
    prices: [
      [1, 100],
      [2, 101],
      [3, 102],
      [4, 103.5],
    ],
  });
  const quoted = (body: string, currency: string) => {
    const face = parseMarkets(body, currency);
    if (face === undefined) {
      throw new Error("the fixture lists a coin");
    }
    return face;
  };

  test("a tap moves between the line and the candles, and reverts when left alone", async () => {
    const CANDLES = JSON.stringify([
      [1, 108.06, 108.2, 107.95, 108.03],
      [2, 108.01, 108.24, 107.93, 108.11],
    ]);
    const get = fake({ "/coins/markets": MARKETS, "/market_chart": CHART, "/ohlc": CANDLES });
    const settings = { coin_id: "SOL", currency: "usd", chart: "line" };
    const NOW = new Date("2026-09-27T09:00:00Z");

    const first = await renderTokenRequest(settings, NOW, {}, get);
    expect(first.state?.chart).toBe("line");

    const toCandles = tapOn(token, settings, first.state, 1, NOW);
    expect(toCandles.view).toBe("candles");
    const tapped = await renderTokenRequest(
      settings,
      NOW,
      { state: toCandles.state, view: toCandles.view },
      get,
    );
    expect(tapped.state?.chart).toBe("candles");
    expect(tapped.svg).not.toBe(first.svg);

    // A scheduled refresh soon after keeps what the finger chose...
    const soon = await renderTokenRequest(
      settings,
      new Date(NOW.getTime() + 60_000),
      {
        state: tapped.state,
      },
      get,
    );
    expect(soon.state?.chart).toBe("candles");

    // ...and a later one hands the card back to its own setting.
    const later = await renderTokenRequest(
      settings,
      new Date(NOW.getTime() + 20 * 60_000),
      {
        state: tapped.state,
      },
      get,
    );
    expect(later.state?.chart).toBe("line");
  });

  test("a card configured for no chart has nothing to tap to", async () => {
    // "none" never requests the series, so offering a switch would draw an empty
    // chart. The state is cleared so a stale override cannot reappear later.
    const get = fake({ "/coins/markets": MARKETS });
    const result = await renderTokenRequest(
      { coin_id: "SOL", currency: "usd", chart: "none" },
      new Date("2026-09-27T09:00:00Z"),
      { event: { taps: 1, point: null } },
      get,
    );
    expect(result.state).toBeNull();
  });

  test("a quote and its chart are read into one face", async () => {
    const get = fake({ "/coins/markets": MARKETS, "/market_chart": CHART });
    const face = await fetchToken({ coin_id: "SOL", currency: "usd" }, get);
    expect(face).toMatchObject({
      symbol: "sol",
      name: "Solana",
      currency: "USD",
      currencyMark: "$",
    });
    expect(face.series).toEqual([100, 101, 102, 103.5]);
    expect(get.asked[0]).toContain("vs_currency=usd&symbols=sol&price_change_percentage=24h");
    expect(get.asked[1]).toContain("/coins/solana/market_chart?vs_currency=usd&days=1");
  });

  test("a chart failure still publishes the quote, with no sparkline", async () => {
    // A rate-limited decoration must not replace a correct price with a stale one.
    const get = fake({ "/coins/markets": MARKETS, "/market_chart": new TransientError("429") });
    const face = await fetchToken({ coin_id: "SOL" }, get);
    expect(face.price).toBe(142.37);
    expect(face.series).toEqual([]);
    expect(rendersToAFrame(renderToken(face))).toBe(true);
  });

  test("the ticker is looked up in the same request that returns the price", async () => {
    // Case is the owner's business: sol and SOL are one ticker.
    for (const typed of ["SOL", "sol", "  Sol  "]) {
      const get = fake({ "/coins/markets": MARKETS, "/market_chart": CHART });
      const face = await fetchToken({ coin_id: typed }, get);
      expect(face.name).toBe("Solana");
      expect(get.asked[0]).toContain("symbols=sol&");
      expect(get.asked[0]).not.toContain("ids=");
      expect(get.asked.at(-1)).toContain("/coins/solana/market_chart");
      expect(get.asked).toHaveLength(2);
    }
  });

  test("a name is not a ticker, and the refusal says what to type instead", async () => {
    // "solana" is a well-formed ticker that nothing trades under: the API answers [].
    const get = fake({ "/coins/markets": "[]" });
    const failure = await fetchToken({ coin_id: "solana" }, get).catch((error: unknown) => error);
    expect(failure).toBeInstanceOf(ConfigurationError);
    expect((failure as Error).message).toBe(
      "no coin trades as SOLANA; enter the ticker, e.g. SOL for Solana",
    );
  });

  test("an answer for a different ticker than the one asked for is not accepted", async () => {
    const get = fake({ "/coins/markets": MARKETS });
    expect(fetchToken({ coin_id: "ETH" }, get)).rejects.toBeInstanceOf(ConfigurationError);
  });

  test("anything that is not a ticker's shape is refused before any request", async () => {
    for (const coin_id of [
      "",
      "   ",
      "Shiba Inu",
      "wrapped-solana",
      "../etc",
      "<b>",
      "x".repeat(13),
    ]) {
      const get = fake({});
      await expect(fetchToken({ coin_id }, get)).rejects.toBeInstanceOf(ConfigurationError);
      expect(get.asked).toEqual([]);
    }
  });

  test("the chart-free face asks for a price and nothing else, and draws no chart", async () => {
    const get = fake({ "/coins/markets": MARKETS });
    const face = await fetchToken({ coin_id: "SOL", chart: "none" }, get);
    expect(get.asked).toHaveLength(1);
    const svg = renderToken(face);
    expect(svg).toContain(">SOL<");
    expect(svg).not.toContain(">24H<");
    expect(svg).not.toContain("%<");
    // The ground is black and the figure carries the day's colour; the arrow beside the
    // ticker is there so that colour is not the only carrier of the direction.
    expect(svg).not.toMatch(/<rect[^>]*rx=/);
    expect(svg).toMatch(new RegExp(`<text[^>]*fill="${GOOD}"[^>]*>142<`));
    expect(svg).toMatch(new RegExp(`<path[^>]*fill="${GOOD}"`));
    expect(renderToken({ ...face, changePercent: -1 })).toMatch(
      new RegExp(`<text[^>]*fill="${BAD}"[^>]*>142<`),
    );
    expect(rendersToAFrame(svg)).toBe(true);
  });

  test("a hand-edited api key rides on both requests and an absent one adds nothing", async () => {
    const keyed = fake({ "/coins/markets": MARKETS, "/market_chart": CHART });
    await fetchToken({ coin_id: "SOL", api_key: "CG-secret" }, keyed);
    expect(keyed.asked.every((url) => url.includes("x_cg_demo_api_key=CG-secret"))).toBe(true);
    const bare = fake({ "/coins/markets": MARKETS, "/market_chart": CHART });
    await fetchToken({ coin_id: "SOL", api_key: null }, bare);
    expect(bare.asked.some((url) => url.includes("x_cg_demo_api_key"))).toBe(false);
  });

  test("an inverted or missing range falls back rather than drawing nonsense", () => {
    const face = quoted(JSON.stringify([{ current_price: 5, high_24h: 4, low_24h: 6 }]), "eur");
    expect([face.low, face.high]).toEqual([4, 6]);
    expect(face.currencyMark).toBe("€");
    const bare = quoted(JSON.stringify([{ current_price: 5 }]), "xyz");
    expect([bare.low, bare.high, bare.changePercent, bare.currencyMark]).toEqual([5, 5, 0, ""]);
  });

  test("a fall prints its sign as well as its colour", () => {
    const face = quoted(MARKETS, "usd");
    const falling = renderToken({ ...face, changePercent: -3.08 });
    expect(falling).toContain("\u22123.08%");
    expect(falling).toContain(BAD);
    expect(renderToken(face)).toContain("+2.41%");
    expect(renderToken(face)).toContain(GOOD);
  });

  test("candles ask the ohlc endpoint instead of the line's, never both", async () => {
    const OHLC = JSON.stringify([
      [1, 108.06, 108.2, 107.95, 108.03],
      [2, 108.01, 108.24, 107.93, 108.11],
    ]);
    const get = fake({ "/coins/markets": MARKETS, "/ohlc": OHLC });
    const face = await fetchToken({ coin_id: "SOL", chart: "candles" }, get);
    expect(face.chart).toBe("candles");
    expect(face.candles).toEqual([
      { open: 108.06, high: 108.2, low: 107.95, close: 108.03 },
      { open: 108.01, high: 108.24, low: 107.93, close: 108.11 },
    ]);
    expect(get.asked.some((url) => url.includes("market_chart"))).toBe(false);
    expect(get.asked.at(-1)).toContain("/coins/solana/ohlc?vs_currency=usd&days=1");
    // An unknown choice is the default, not an error: the field is an enum upstream.
    expect(
      (await fetchToken({ coin_id: "SOL", chart: "renko" }, fake({ "/coins/markets": MARKETS })))
        .chart,
    ).toBe("line");
  });

  test("a refused candle request leaves a true price over an empty chart", async () => {
    const get = fake({ "/coins/markets": MARKETS, "/ohlc": new TransientError("429") });
    const face = await fetchToken({ coin_id: "SOL", chart: "candles" }, get);
    expect(face.price).toBe(142.37);
    expect(face.candles).toEqual([]);
    expect(rendersToAFrame(renderToken(face))).toBe(true);
  });

  test("a malformed candle is dropped and an inverted one cannot draw a wick inside out", () => {
    const candles = parseCandles(
      JSON.stringify([[1, 10, 9, 12, 11], [2, 10, "x", 9, 11], [3, 10], "junk", [4, 5, 6, 4, 5]]),
    );
    expect(candles).toEqual([
      { open: 10, high: 12, low: 9, close: 11 },
      { open: 5, high: 6, low: 4, close: 5 },
    ]);
  });

  test("each candle is coloured by its own half hour, whatever the day did", () => {
    const svg = renderToken({
      ...quoted(MARKETS, "usd"),
      chart: "candles",
      candles: [
        { open: 10, high: 12, low: 9, close: 11 },
        { open: 11, high: 11.5, low: 8, close: 9 },
      ],
    });
    expect(svg).toContain(GOOD);
    expect(svg).toContain(BAD);
  });

  test("the price starts at the module's margin by its ink, whatever digit leads it", () => {
    // An advance-based layout put the first glyph a bearing inside the margin, so the
    // price sat right of the symbol above it. The mark's INK must start at the margin
    // at every size the price is fitted to.
    for (const price of [118.04, 842.5, 104235.5, 0.004182]) {
      const svg = renderToken({ ...quoted(MARKETS, "usd"), price });
      const mark = svg.match(/<text x="([\d.]+)"[^>]*font-size="([\d.]+)"[^>]*>\$<\/text>/);
      const inkLeft = Number(mark?.[1]) + textInk("$", Number(mark?.[2]), 600).left;
      expect(inkLeft).toBeCloseTo(24 + 20, 1);
    }
  });

  test("decimals follow the magnitude, and only high precision is trimmed", () => {
    expect(splitPrice(104235.5)).toEqual(["104,235", ".50"]);
    expect(splitPrice(142.5)).toEqual(["142", ".50"]);
    expect(splitPrice(0.0371)).toEqual(["0", ".0371"]);
    expect(splitPrice(0.0000371)).toEqual(["0", ".0000371"]);
    expect(splitPrice(0)).toEqual(["0", ".00"]);
  });
});

describe("rss", () => {
  const RSS = `<?xml version="1.0"?><rss version="2.0"><channel><title>Feed</title>
    <item><title>First &amp; foremost</title><pubDate>Sat, 12 Sep 2026 13:46:00 GMT</pubDate></item>
    <item><title><![CDATA[Second <b>story</b>]]></title><pubDate>not a date</pubDate></item>
    <item><title>Third</title></item><item><title>Fourth</title></item><item><title>Fifth</title></item>
  </channel></rss>`;
  const ATOM = `<feed xmlns="http://www.w3.org/2005/Atom"><title>Releases</title>
    <entry><title type="html">v1.4 &lt;em&gt;out&lt;/em&gt;</title><updated>2026-09-12T11:00:00Z</updated></entry>
  </feed>`;

  test("RSS 2.0 parses, entities and CDATA included, keeping more than one page", () => {
    // The parse keeps four pages so a tap has somewhere to go; a page of four is
    // what the face draws at once. It used to cap here at the page size, which
    // left nothing to turn to.
    const entries = parseFeed(RSS, NOW);
    expect(entries.map((entry) => entry.title)).toEqual([
      "First & foremost",
      "Second story",
      "Third",
      "Fourth",
      "Fifth",
    ]);
    expect(entries.map((entry) => entry.age)).toEqual(["14m", "", "", "", ""]);
  });

  test("a tap turns the page without going back to the feed", async () => {
    // The whole point of storing entries: answering a tap must cost a draw and
    // nothing else. A face that refetched would put a network round trip between
    // the owner's finger and the new picture.
    let fetches = 0;
    const get = fake({ "example.com": RSS });
    const counting = async (url: string) => {
      fetches += 1;
      return get(url);
    };
    const settings = { url: "https://example.com/feed.xml", title: "News" };

    const first = await renderRssRequest(settings, NOW, {}, counting);
    expect(fetches).toBe(1);
    expect(first.state.page).toBe(0);

    const toSecond = tapOn(rss, settings, first.state, 1, NOW);
    const second = await renderRssRequest(
      settings,
      NOW,
      { state: toSecond.state, view: toSecond.view },
      counting,
    );
    expect(fetches).toBe(1);
    expect(pageOf(toSecond.state)).toBe(1);
    expect(second.svg).not.toBe(first.svg);

    // Five entries is two pages, so a second tap wraps rather than stopping.
    const toThird = tapOn(rss, settings, toSecond.state, 1, NOW);
    const third = await renderRssRequest(
      settings,
      NOW,
      { state: toThird.state, view: toThird.view },
      counting,
    );
    expect(fetches).toBe(1);
    expect(pageOf(toThird.state)).toBe(0);
    expect(third.svg).toBe(first.svg);
  });

  test("Atom parses, with an html title reduced to its text", () => {
    expect(parseFeed(ATOM, NOW)).toEqual([{ title: "v1.4 out", age: "3h" }]);
  });

  test("a page that is not a feed sends the owner back to the URL", async () => {
    const get = fake({ "example.com": "<html><body>hello</body></html>" });
    const settings = { url: "https://example.com/", title: "News" };
    expect(fetchRss(settings, NOW, get)).rejects.toBeInstanceOf(ConfigurationError);
  });

  test("a hostile headline cannot escape its text node, and still renders", () => {
    const svg = renderRss({
      feedTitle: "News",
      entries: [{ title: "</text><script>fetch('http://evil.test')</script><text>", age: "1h" }],
    });
    expect(svg).not.toContain("<script>");
    expect(svg).toContain("&lt;script&gt;");
    expect(rendersToAFrame(svg)).toBe(true);
  });

  test("an item without a date omits its age instead of inventing one", () => {
    const svg = renderRss({ feedTitle: "Undated", entries: [{ title: "A story", age: "" }] });
    expect(svg).not.toMatch(/just now|ago/);
  });
});

describe("weather", () => {
  const GEOCODING = JSON.stringify({
    results: [
      { name: "Dubai", country: "United Arab Emirates", latitude: 25.07, longitude: 55.17 },
    ],
  });
  const forecast = (current: object, hourly?: object): string =>
    JSON.stringify({
      current: {
        time: "2026-09-12T14:15",
        temperature_2m: 33.5,
        weather_code: 1,
        is_day: 1,
        ...current,
      },
      daily: {
        time: ["2026-09-12", "2026-09-13", "2026-09-14", "2026-09-15", "2026-09-16"],
        weather_code: [1, 2, 61, 3, 0],
        temperature_2m_max: [38.4, 37.2, 34.8, 35.1, 36.6],
        temperature_2m_min: [27.2, 28.1, 25.6, 24.9, 25.2],
      },
      hourly: hourly ?? {
        time: ["2026-09-12T13:00", "2026-09-12T14:00", "2026-09-12T15:00", "2026-09-12T16:00"],
        temperature_2m: [33, 34.4, 35.5, null],
        weather_code: [0, 1, 3, 61],
        is_day: [1, 1, 0, 0],
      },
    });

  test("the face says what the forecast is actually for, in the geocoder's words", async () => {
    const get = fake({ "geocoding-api": GEOCODING, "/v1/forecast": forecast({}) });
    const face = await fetchWeather({ location: "dubai", units: "metric" }, get);
    expect(face.place).toBe("Dubai, United Arab Emirates");
    expect(get.asked[1]).toContain("latitude=25.07&longitude=55.17");
    expect(get.asked[1]).toContain("timezone=auto");
    expect(get.asked[1]).toContain("daily=weather_code%2Ctemperature_2m_max%2Ctemperature_2m_min");
    // Six: today, tomorrow for the flipped view's hero, and the four days its
    // strip carries after it.
    expect(get.asked[1]).toContain("forecast_days=6");
    expect(get.asked[1]).not.toContain("temperature_unit");
  });

  test.each(["Asia/Tokyo", "America/Los_Angeles"])(
    "weather forwards owner timezone %s",
    async (timezone) => {
      const get = fake({ "geocoding-api": GEOCODING, "/v1/forecast": forecast({}) });
      await renderWeatherRequest(
        { location: "Dubai" },
        new Date("2028-01-01T00:00:00Z"),
        { timezone },
        get,
      );
      expect(new URL(get.asked[1] ?? "").searchParams.get("timezone")).toBe(timezone);
    },
  );

  test("a tap redraws from the forecast already in hand", async () => {
    // Both views say the same reading about a different moment, so switching
    // between them has no reason to visit Open-Meteo. The round trip it saves is
    // the largest remaining term in a tap once flash is out of the way.
    const get = fake({ "geocoding-api": GEOCODING, "/v1/forecast": forecast({}) });
    const settings = { location: "Dubai", units: "metric" };
    const NOW = new Date("2026-09-27T09:00:00Z");

    const first = await renderWeatherRequest(settings, NOW, {}, get);
    const asked = get.asked.length;
    const firstState = typeof first === "string" ? undefined : first.state;

    const later = new Date(NOW.getTime() + 30_000);
    const selection = tapOn(weather, settings, firstState, 1, later);
    const tapped = await renderWeatherRequest(
      settings,
      later,
      { state: selection.state, view: selection.view },
      get,
    );
    expect(get.asked.length).toBe(asked);
    expect(typeof tapped === "string" ? "" : tapped.svg).not.toBe(
      typeof first === "string" ? "" : first.svg,
    );

    // A cache older than the card's own refresh is not reused.
    await renderWeatherRequest(
      settings,
      new Date(NOW.getTime() + 20 * 60_000),
      { state: firstState, event: { taps: 1, point: null } },
      get,
    );
    expect(get.asked.length).toBeGreaterThan(asked);
  });

  test("imperial asks the API for fahrenheit rather than converting", async () => {
    const get = fake({ "geocoding-api": GEOCODING, "/v1/forecast": forecast({}) });
    await fetchWeather({ location: "Dubai", units: "imperial" }, get);
    expect(get.asked[1]).toContain("temperature_unit=fahrenheit");
  });

  test("a place nobody has heard of is a setting to fix", async () => {
    const get = fake({ "geocoding-api": "{}" });
    expect(fetchWeather({ location: "Qwertyuiop" }, get)).rejects.toBeInstanceOf(
      ConfigurationError,
    );
  });

  test("the strip starts at the current local hour and skips an unusable slot", () => {
    const face = parseForecast(forecast({}), "Dubai");
    expect(face.hourly).toEqual([
      { label: "14", temperature: 34, condition: "clear-day" },
      { label: "15", temperature: 36, condition: "cloudy" },
    ]);
    expect([face.temperature, face.high, face.low]).toEqual([34, 38, 27]);
  });

  test("the coming days are parsed with weekdays from the forecast's own dates", () => {
    expect(parseForecast(forecast({}), "Dubai").daily).toEqual([
      {
        date: "2026-09-12",
        label: "SAT",
        high: 38,
        low: 27,
        condition: "clear-day",
        // WMO code 1 is "mainly clear": the glyph and the words come from two
        // existing mappings that have always disagreed here, and this pins what
        // they actually do rather than changing the approved view.
        summary: "Partly cloudy",
      },
      {
        date: "2026-09-13",
        label: "SUN",
        high: 37,
        low: 28,
        condition: "partly-cloudy-day",
        summary: "Partly cloudy",
      },
      { date: "2026-09-14", label: "MON", high: 35, low: 26, condition: "rain", summary: "Rain" },
      {
        date: "2026-09-15",
        label: "TUE",
        high: 35,
        low: 25,
        condition: "cloudy",
        summary: "Overcast",
      },
      {
        date: "2026-09-16",
        label: "WED",
        high: 37,
        low: 25,
        condition: "clear-day",
        summary: "Clear",
      },
    ]);
  });

  test("halves round away from zero in both directions", () => {
    expect(parseForecast(forecast({ temperature_2m: 18.5 }), "x").temperature).toBe(19);
    expect(parseForecast(forecast({ temperature_2m: -18.5 }), "x").temperature).toBe(-19);
    expect(parseForecast(forecast({ temperature_2m: -0.4 }), "x").temperature).toBe(0);
  });

  test("a forecast with no usable hourly block still draws, with an empty strip", () => {
    expect(parseForecast(forecast({}, {}), "Dubai").hourly).toEqual([]);
    expect(parseForecast(forecast({ time: "garbage" }), "Dubai").hourly).toEqual([]);
  });

  test("conditions are coarser than the WMO list, and night is a different face", () => {
    expect(conditionFromWmo(0, true)).toBe("clear-day");
    expect(conditionFromWmo(1, false)).toBe("clear-night");
    expect(conditionFromWmo(2, false)).toBe("partly-cloudy-night");
    expect(conditionFromWmo(3, true)).toBe("cloudy");
    expect(conditionFromWmo(67, true)).toBe("sleet");
    expect(conditionFromWmo(82, true)).toBe("rain");
    expect(conditionFromWmo(99, false)).toBe("thunderstorm");
    expect(conditionFromWmo(12345, true)).toBe("cloudy");
  });

  const approvedNow = {
    place: "Dubai",
    temperature: 34,
    summary: "Mostly clear",
    condition: "clear-day" as const,
    high: 38,
    low: 27,
    hourly: [
      { label: "14", temperature: 34, condition: "clear-day" as const },
      { label: "15", temperature: 35, condition: "clear-day" as const },
      { label: "16", temperature: 34, condition: "partly-cloudy-day" as const },
      { label: "17", temperature: 32, condition: "partly-cloudy-day" as const },
      { label: "18", temperature: 30, condition: "cloudy" as const },
      { label: "19", temperature: 29, condition: "clear-night" as const },
    ],
    daily: [
      {
        date: "2026-09-12",
        label: "SAT",
        high: 38,
        low: 27,
        condition: "clear-day" as const,
        summary: "Clear",
      },
      {
        date: "2026-09-13",
        label: "SUN",
        high: 37,
        low: 28,
        condition: "partly-cloudy-day" as const,
        summary: "Partly cloudy",
      },
      {
        date: "2026-09-14",
        label: "MON",
        high: 35,
        low: 26,
        condition: "rain" as const,
        summary: "Rain",
      },
      {
        date: "2026-09-15",
        label: "TUE",
        high: 35,
        low: 25,
        condition: "cloudy" as const,
        summary: "Overcast",
      },
      {
        date: "2026-09-16",
        label: "WED",
        high: 37,
        low: 25,
        condition: "clear-day" as const,
        summary: "Clear",
      },
    ],
  };
  const svgOf = (result: RenderResult): string =>
    typeof result === "string" ? result : result.svg;
  const viewOf = (result: RenderResult): unknown =>
    typeof result === "string" || typeof result.state !== "object" || result.state === null
      ? undefined
      : (result.state as { view?: unknown }).view;
  const minutesBefore = (instant: Date, minutes: number): string =>
    new Date(instant.getTime() - minutes * 60_000).toISOString();

  test("a tap flips to the coming days and another flips back", () => {
    expect(weather.tap).toBe("Tap the panel for the coming days.");
    const toDays = tapOn(weather, approvedNow, { view: "now", tappedAt: null }, 1, NOW);
    expect(toDays.view).toBe("days");
    expect(toDays.state).toEqual({ view: "days", tappedAt: NOW.toISOString() });
    const days = renderWeatherResult(approvedNow, NOW, {
      state: toDays.state,
      view: toDays.view,
    });
    expect(viewOf(days)).toBe("days");

    const back = tapOn(weather, approvedNow, { view: "days", tappedAt: NOW.toISOString() }, 1, NOW);
    expect(back.view).toBe("");
    expect(
      viewOf(renderWeatherResult(approvedNow, NOW, { state: back.state, view: back.view })),
    ).toBe("now");
  });

  test("an even number of coalesced taps lands where it started", () => {
    for (const view of ["now", "days"] as const) {
      const result = renderWeatherResult(approvedNow, NOW, {
        state: { view, tappedAt: null },
        event: { taps: 2, point: null },
      });
      expect(viewOf(result)).toBe(view);
    }
  });

  test("a scheduled refresh more than ten minutes after a tap returns to now", () => {
    const result = renderWeatherResult(approvedNow, NOW, {
      state: { view: "days", tappedAt: minutesBefore(NOW, 11) },
    });
    expect(viewOf(result)).toBe("now");
  });

  test("a scheduled refresh exactly ten minutes after a tap keeps the days view", () => {
    const result = renderWeatherResult(approvedNow, NOW, {
      state: { view: "days", tappedAt: minutesBefore(NOW, 10) },
    });
    expect(viewOf(result)).toBe("days");
  });

  test("the current-conditions view is byte-identical to the approved design", () => {
    const result = renderWeatherResult(approvedNow, NOW, {
      state: { view: "now", tappedAt: null },
    });
    expect(svgOf(result)).toBe(
      readFileSync(`${import.meta.dir}/golden/weather--clear-day.svg`, "utf8"),
    );
    expect(svgOf(result)).toBe(renderWeather(approvedNow));
  });
});

describe("hacker news", () => {
  const CAPTURED_AT = new Date("2026-09-23T05:00:00Z");
  const FIXTURE_FRONT_PAGE = CAPTURED_HACKER_NEWS;

  const capturedStories = (count = 20): Story[] =>
    FIXTURE_FRONT_PAGE.slice(0, count).flatMap((item, index) => {
      const story = storyFromItem(item, index + 1, CAPTURED_AT);
      return story === undefined ? [] : [story];
    });

  const capturedFrontPage = (): FetchText & { asked: string[] } =>
    fake({
      "topstories.json": JSON.stringify(FIXTURE_FRONT_PAGE.map(({ id }) => id)),
      ...Object.fromEntries(
        FIXTURE_FRONT_PAGE.map((item) => [`item/${item.id}.json`, JSON.stringify(item)]),
      ),
    });

  const capturedTitle = (index: number): string => {
    const item = FIXTURE_FRONT_PAGE[index];
    if (item === undefined) {
      throw new Error(`captured Hacker News fixture has no story at index ${index}`);
    }
    return item.title;
  };

  const minutesBefore = (date: Date, minutes: number): string =>
    new Date(date.getTime() - minutes * 60_000).toISOString();

  const item = (id: number, extra: object = {}): string =>
    JSON.stringify({
      id,
      type: "story",
      title: `Story ${id}`,
      score: id * 10,
      descendants: id,
      url: `https://www.example${id}.com/post`,
      time: NOW.getTime() / 1000 - 3 * 3600,
      ...extra,
    });

  test("stories arrive in list order with the www stripped and the age rendered", async () => {
    const get = fake({
      "topstories.json": "[1, 2, 3, 4, 5, 6, 7, 8]",
      "item/1.json": item(1),
      "item/2.json": item(2),
      "item/3.json": item(3),
      "item/4.json": item(4),
      "item/5.json": item(5),
      "item/6.json": item(6),
    });
    const face = await fetchHackerNews({}, NOW, get);
    expect(face.stories.map((story) => story.rank)).toEqual([1, 2, 3, 4, 5, 6]);
    expect(face.stories[0]).toEqual({
      rank: 1,
      title: "Story 1",
      points: 10,
      comments: 1,
      domain: "example1.com",
      age: "3h",
    });
    expect(get.asked.some((url) => url.includes("item/8.json"))).toBe(true);
  });

  test("a dead, deleted or unreadable story is skipped and the ranks stay contiguous", async () => {
    const get = fake({
      "beststories.json": "[1, 2, 3, 4, 5, 6]",
      "item/1.json": item(1, { dead: true }),
      "item/2.json": item(2),
      "item/3.json": new TransientError("timeout"),
      "item/4.json": item(4, { deleted: true }),
      "item/5.json": item(5),
      "item/6.json": "null",
    });
    const face = await fetchHackerNews({ list: "best" }, NOW, get);
    expect(face.stories.map((story) => [story.rank, story.title])).toEqual([
      [1, "Story 2"],
      [2, "Story 5"],
    ]);
  });

  test("when no story at all can be read the stored frame is kept", async () => {
    const get = fake({ "topstories.json": "[1, 2]" });
    expect(fetchHackerNews({}, NOW, get)).rejects.toBeInstanceOf(TransientError);
  });

  test("an Ask HN post has no domain, and entities in a title are decoded once", () => {
    const story = storyFromItem(
      { title: "Ask HN: Who&#x27;s hiring &amp; where?", score: 5, time: 0 },
      1,
      NOW,
    );
    expect(story).toMatchObject({
      title: "Ask HN: Who's hiring & where?",
      domain: "",
      comments: 0,
    });
  });

  const story = (rank: number, title: string) => ({
    rank,
    title,
    points: 4321,
    comments: 1234,
    domain: "example.com",
    age: "2h",
  });

  test("a long headline costs index rows before it costs a word", () => {
    const long = "Human brain is two separate organs, Stanford Medicine-led research finds";
    const svg = renderHackerNews({
      stories: [story(1, long), story(2, long), story(3, long), story(4, long)],
    });
    expect(svg).not.toContain("…");
    // Every word of the lead is on the face.
    for (const word of long.split(" ")) {
      expect(svg).toContain(word.replace("&", "&amp;"));
    }
  });

  test("a short front page keeps its index: the lone lead is a fallback, not a preference", () => {
    const svg = renderHackerNews({
      stories: [story(1, "OpenJev"), story(2, "Cloudflare Quick Tunnels"), story(3, "Bun 1.4")],
    });
    expect(svg).toContain("Cloudflare Quick Tunnels");
    expect(svg).toContain("Bun 1.4");
  });

  test("a pathological title is cut inside the module rather than overflowing it", () => {
    const svg = renderHackerNews({ stories: [story(1, "Pneumonoultramicroscopic".repeat(12))] });
    expect(svg).toContain("…");
    expect(rendersToAFrame(svg)).toBe(true);
    for (const match of svg.matchAll(/<text x="[\d.-]+" y="([\d.-]+)"/g)) {
      expect(Number(match[1])).toBeLessThanOrEqual(368 - 24);
    }
  });

  test("a tap moves to the next page without fetching", async () => {
    // The tap path as the server drives it: onTap decides, without fetching or
    // drawing, and render draws the view it decided on. Asserting both is what
    // proves the two halves agree -- a view onTap can return but render cannot
    // draw is the failure this seam invites.
    const get = fake({});
    const selection = tapOn(
      hackernews,
      {},
      { stories: capturedStories(), page: 0, tappedAt: null },
      1,
      CAPTURED_AT,
    );
    const result = await renderHackerNewsRequest(
      {},
      CAPTURED_AT,
      { state: selection.state, view: selection.view },
      get,
    );
    expect(get.asked).toEqual([]);
    expect(hackernews.tap).toBe("Tap the panel for the next stories.");
    expect(pageOf(selection.state)).toBe(1);
    expect(visibleSvgText(result.svg)).toContain(capturedTitle(4));
    expect(result.svg).toContain(">2 / 5<");
  });

  test("three coalesced taps move three pages", async () => {
    const selection = tapOn(
      hackernews,
      {},
      { stories: capturedStories(), page: 0, tappedAt: null },
      3,
      CAPTURED_AT,
    );
    const result = await renderHackerNewsRequest(
      {},
      CAPTURED_AT,
      { state: selection.state, view: selection.view },
      fake({}),
    );
    expect(pageOf(selection.state)).toBe(3);
    expect(visibleSvgText(result.svg)).toContain(capturedTitle(12));
  });

  test("paging wraps at the end", async () => {
    const selection = tapOn(
      hackernews,
      {},
      { stories: capturedStories(), page: 4, tappedAt: null },
      1,
      CAPTURED_AT,
    );
    const result = await renderHackerNewsRequest(
      {},
      CAPTURED_AT,
      { state: selection.state, view: selection.view },
      fake({}),
    );
    expect(pageOf(selection.state)).toBe(0);
    expect(visibleSvgText(result.svg)).toContain(capturedTitle(0));
    expect(visibleSvgText(result.svg)).toContain(capturedTitle(1));
    expect(visibleSvgText(result.svg)).toContain(capturedTitle(2));
  });

  test("a scheduled refresh within ten minutes of a tap keeps the page", async () => {
    const get = capturedFrontPage();
    const result = await renderHackerNewsRequest(
      {},
      CAPTURED_AT,
      { state: { stories: capturedStories(), page: 2, tappedAt: minutesBefore(CAPTURED_AT, 9) } },
      get,
    );
    expect(result.state.page).toBe(2);
    expect(result.state.stories).toHaveLength(20);
    expect(Buffer.byteLength(JSON.stringify(result.state))).toBeLessThanOrEqual(16 * 1024);
    expect(get.asked).toHaveLength(23);
    expect(visibleSvgText(result.svg)).toContain(capturedTitle(8));
  });

  test("a scheduled refresh more than ten minutes after a tap returns to the front", async () => {
    const result = await renderHackerNewsRequest(
      {},
      CAPTURED_AT,
      { state: { stories: capturedStories(), page: 2, tappedAt: minutesBefore(CAPTURED_AT, 11) } },
      capturedFrontPage(),
    );
    expect(result.state.page).toBe(0);
    expect(result.state.tappedAt).toBeNull();
    expect(visibleSvgText(result.svg)).toContain(capturedTitle(0));
  });

  test("a tap on a face with no state fetches, as a first render does", async () => {
    const get = capturedFrontPage();
    const result = await renderHackerNewsRequest(
      {},
      CAPTURED_AT,
      { state: undefined, event: { taps: 1, point: null } },
      get,
    );
    expect(get.asked).toHaveLength(23);
    expect(result.state.page).toBe(0);
    expect(visibleSvgText(result.svg)).toContain(capturedTitle(0));
  });

  test("a page with fewer than four stories still draws", async () => {
    const selection = tapOn(
      hackernews,
      {},
      { stories: capturedStories(17), page: 3, tappedAt: null },
      1,
      CAPTURED_AT,
    );
    const result = await renderHackerNewsRequest(
      {},
      CAPTURED_AT,
      { state: selection.state, view: selection.view },
      fake({}),
    );
    expect(pageOf(selection.state)).toBe(4);
    expect(visibleSvgText(result.svg)).toContain(capturedTitle(16));
    expect(result.svg).toContain(">5 / 5<");
  });
});

describe("the views seam", () => {
  // The saving a tap makes is that NOTHING fetches and NOTHING rasterises on the
  // tap path: the server asks which view, then pushes a scene naming a frame the
  // device already holds. A face that fetched inside views() or onTap() would put
  // a network round trip back in front of the owner's finger and nothing else
  // here would notice.
  const faces: FaceDefinition[] = [weather, hackernews, rss, token];

  test("views and onTap neither fetch nor draw", () => {
    const original = globalThis.fetch;
    globalThis.fetch = (() => {
      throw new Error("a pure entry point must not reach the network");
    }) as unknown as typeof fetch;
    try {
      for (const face of faces) {
        const settings = { location: "Dubai", url: "https://example.com/feed.xml", coin_id: "SOL" };
        expect(() => face.views?.(settings, undefined)).not.toThrow();
        expect(() =>
          face.onTap?.(settings, undefined, { taps: 1, point: null }, NOW),
        ).not.toThrow();
      }
    } finally {
      globalThis.fetch = original;
    }
  });

  test("every face that offers views can answer a tap, and every view it names is one it offers", () => {
    for (const face of faces) {
      // The two are a pair: a face with views must be able to select one, and a
      // face that selects must say what it can select between.
      expect(face.views === undefined).toBe(face.onTap === undefined);
      if (face.views === undefined || face.onTap === undefined) {
        continue;
      }
      const settings = { location: "Dubai", url: "https://example.com/feed.xml", coin_id: "SOL" };
      const offered = face.views(settings, undefined);
      expect(offered.length).toBeGreaterThan(0);
      expect(offered[0]).toBe("");
      const selected = face.onTap(settings, undefined, { taps: 1, point: null }, NOW);
      expect(offered).toContain(selected.view);
    }
  });

  test("a face that declares a tap sentence declares views, and the reverse", () => {
    for (const face of faces) {
      expect(typeof face.tap === "string").toBe(face.views !== undefined);
    }
  });
});
